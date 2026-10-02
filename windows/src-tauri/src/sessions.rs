// What the desktop board lists: every Claude Code session that is running, and
// the plan's usage.
//
// Hooks only tell us about sessions that do something while Coucou is up. Claude
// Code itself keeps a registry of its interactive sessions in
// ~/.claude/sessions/<pid>.json (cwd, name, busy/idle), so the board can list
// all of them from the start — files of sessions that crashed are left behind,
// hence the check that the process is still the one that wrote the file.
//
// Usage comes from the statusLine JSON Claude Code hands its status line
// command, when that command saves it to ~/.claude/plasma-usage-status.json.
// Nothing is written here and nothing leaves the machine.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    session_id: String,
    pid: u32,
    cwd: String,
    name: String,
    /// "busy" or "idle", as Claude Code last wrote it.
    status: String,
    started_at: f64,
    status_updated_at: f64,
    /// The herdr pane the session runs in, so Open can jump to it.
    pane: Option<String>,
}

/// Field 22 of /proc/<pid>/stat: when the process started, in clock ticks.
fn proc_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may hold spaces and parentheses: count from
    // the last ')'.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19).map(str::to_string)
}

fn herdr_pane(pid: u32) -> Option<String> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    env.split(|b| *b == 0)
        .find_map(|kv| kv.strip_prefix(b"HERDR_PANE_ID="))
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .filter(|v| !v.is_empty())
}

pub fn list() -> Vec<Session> {
    let Some(dir) = claude_dir().map(|d| d.join("sessions")) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let str_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let num_of = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or_default();
        let Some(pid) = v.get("pid").and_then(Value::as_u64).map(|p| p as u32) else { continue };
        if v.get("kind").and_then(Value::as_str).is_some_and(|k| k != "interactive") {
            continue;
        }
        // Dead, or the pid now belongs to something else.
        match (proc_start(pid), v.get("procStart").and_then(Value::as_str)) {
            (Some(now), Some(then)) if now == then => {}
            (Some(_), None) => {}
            _ => continue,
        }
        out.push(Session {
            session_id: str_of("sessionId"),
            pid,
            cwd: str_of("cwd"),
            name: str_of("name"),
            status: str_of("status"),
            started_at: num_of("startedAt"),
            status_updated_at: num_of("statusUpdatedAt"),
            pane: herdr_pane(pid),
        });
    }
    out.sort_by(|a, b| a.started_at.total_cmp(&b.started_at));
    out
}

/// `rate_limits` from the last status line, and when it was received (seconds).
pub fn usage() -> Option<Value> {
    let path = claude_dir()?.join("plasma-usage-status.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let limits = v.get("rate_limits")?.clone();
    Some(serde_json::json!({
        "rateLimits": limits,
        "receivedAt": v.get("_received_at_ts").cloned().unwrap_or(Value::Null),
    }))
}
