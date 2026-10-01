// Jumping to a Claude Code session that runs inside herdr (herdr.dev).
//
// herdr injects HERDR_PANE_ID into every pane, coucou-hook forwards it with each
// event, and "Open" here does two things:
//   1. `herdr agent focus <pane>` — herdr switches to that agent's pane;
//   2. brings the terminal window running herdr to the front, through a
//      one-shot KWin script (Wayland lets no app raise another's window itself).
// With no herdr client attached anywhere, a terminal is started with `herdr`,
// which attaches and lands on the focused pane.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::log;

/// Pane ids look like `w1:p3`. Anything else is not passed on.
fn valid_pane(pane: &str) -> bool {
    pane.len() <= 64
        && pane.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && pane.bytes().all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'-')
}

fn herdr_exe() -> Option<PathBuf> {
    crate::find_on_path("herdr").or_else(|| {
        let p = PathBuf::from(std::env::var_os("HOME")?).join(".local/bin/herdr");
        p.is_file().then_some(p)
    })
}

pub fn open(pane: &str) -> bool {
    if !valid_pane(pane) {
        return false;
    }
    let Some(herdr) = herdr_exe() else { return false };

    let focused = Command::new(&herdr)
        .args(["agent", "focus", pane])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !focused {
        log::line(format!("herdr: could not focus {pane}"));
        return false;
    }

    let hosts = client_window_pids();
    if hosts.is_empty() {
        return launch_client(&herdr);
    }
    if !raise_window_of(&hosts) {
        log::line("herdr: pane focused, but the terminal window could not be raised");
    }
    true
}

// ── Finding the terminal ─────────────────────────────────────────────────────

/// For every attached herdr client, its ancestors nearest first — one of them is
/// the terminal emulator that owns the window. The server is not a client.
fn client_window_pids() -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return out };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Some(args) = cmdline(pid) else { continue };
        let Some(argv0) = args.first() else { continue };
        let is_herdr = Path::new(argv0).file_name().map(|n| n == "herdr").unwrap_or(false);
        let is_server = args.get(1).map(|a| a == "server").unwrap_or(false);
        if !is_herdr || is_server {
            continue;
        }
        let mut p = parent(pid);
        while let Some(pp) = p.filter(|&pp| pp > 1) {
            if !out.contains(&pp) {
                out.push(pp);
            }
            p = parent(pp);
        }
    }
    out
}

fn cmdline(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect(),
    )
}

fn parent(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name can contain spaces and parentheses: read after the last ')'.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Activates the first normal window owned by any of `pids`, nearest first.
fn raise_window_of(pids: &[u32]) -> bool {
    let list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    let script = format!(
        "const pids = [{list}];\n\
         const wins = workspace.windowList();\n\
         outer: for (const pid of pids) {{\n\
           for (const w of wins) {{\n\
             if (w.pid === pid && w.normalWindow) {{\n\
               if (w.minimized) w.minimized = false;\n\
               workspace.activeWindow = w;\n\
               break outer;\n\
             }}\n\
           }}\n\
         }}\n"
    );
    let dir = crate::runtime_dir();
    if crate::ensure_private_dir(&dir).is_err() {
        return false;
    }
    let path = dir.join("raise-terminal.js");
    if std::fs::write(&path, script).is_err() {
        return false;
    }
    const NAME: &str = "coucou-raise-terminal";
    let busctl = |args: &[&str]| -> Option<String> {
        let out = Command::new("busctl").arg("--user").args(args).stdin(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
    };
    // A leftover from an interrupted run would make loadScript refuse the name.
    let _ = busctl(&["call", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "unloadScript", "s", NAME]);
    let Some(loaded) = busctl(&[
        "call", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "loadScript", "ss",
        &path.to_string_lossy(), NAME,
    ]) else {
        return false;
    };
    // Reply is `i <id>`.
    let Some(id) = loaded.split_whitespace().nth(1).filter(|id| id.parse::<i64>().map(|n| n >= 0).unwrap_or(false))
    else {
        return false;
    };
    let ran = busctl(&["call", "org.kde.KWin", &format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run"]).is_some();
    // The script runs synchronously inside KWin; unloading right after is safe.
    let _ = busctl(&["call", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "unloadScript", "s", NAME]);
    ran
}

/// No client attached: open a terminal running `herdr`, which attaches to the
/// running server and shows the pane we just focused.
fn launch_client(herdr: &Path) -> bool {
    let preferred = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
    let candidates = preferred
        .into_iter()
        .chain(["kitty", "konsole", "ghostty", "wezterm", "alacritty", "foot"].map(String::from));
    for term in candidates {
        let Some(exe) = crate::find_on_path(&term) else { continue };
        let mut cmd = Command::new(exe);
        // kitty and foot take the program directly; the others want -e.
        if !matches!(term.as_str(), "kitty" | "foot") {
            cmd.arg("-e");
        }
        if cmd.arg(herdr).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return true;
        }
    }
    log::line("herdr: no terminal found to attach to herdr");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pane_shaped_ids_pass() {
        assert!(valid_pane("w1:p3"));
        assert!(valid_pane("wC:p1"));
        assert!(!valid_pane(""));
        assert!(!valid_pane("w1:p1; rm -rf ~"));
        assert!(!valid_pane("--help"));
    }
}
