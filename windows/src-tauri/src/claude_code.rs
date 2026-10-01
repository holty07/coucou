// Chat through the user's own Claude Code install — `claude -p`, its headless
// mode — so chat runs on their Claude plan instead of a separately billed API
// key. Used whenever no Anthropic API key is stored.
//
// Claude Code is started lean on purpose. A bare `claude -p` loads the user's
// whole environment (MCP servers, skills, plugins, CLAUDE.md, hooks), which on
// a well-equipped machine is ~180k tokens of context for a one-line reply. With
// the flags below the same turn is ~4k:
//   * --setting-sources ""   no user/project settings: no hooks, no plugins
//   * --strict-mcp-config     no MCP servers
//   * --disable-slash-commands  no skills
//   * --tools …               only web search, web fetch and Read
//   * --permission-mode dontAsk  never stops to ask; anything else is denied
//
// Multi-turn: the first reply carries a session id; later turns `--resume` it.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::claude::{ChatContext, ChatReply};

/// A chat turn with web search can take a while; beyond this we give up.
const TURN_TIMEOUT: Duration = Duration::from_secs(240);

/// Set on the child so coucou-hook ignores it even if hooks were somehow
/// loaded: Mochi's own chat must never show up as a Claude Code session.
pub const CHAT_ENV: &str = "COUCOU_CHAT";

const TOOLS: &[&str] = &["WebSearch", "WebFetch", "Read"];

/// The `claude` executable: $PATH first, then the usual install locations
/// (a session started from the desktop may not have ~/.local/bin on $PATH).
pub fn executable() -> Option<PathBuf> {
    if let Some(p) = crate::find_on_path("claude") {
        return Some(p);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let home = PathBuf::from(home);
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    [
        home.join(".local/bin").join(exe),
        home.join(".claude/local").join(exe),
        home.join(".npm-global/bin").join(exe),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub fn available() -> bool {
    executable().is_some()
}

/// One turn. `session` is the id of the conversation so far, if any; the new
/// id comes back with the reply.
pub async fn send(
    session: Option<String>,
    system_prompt: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<(ChatReply, String), String> {
    let exe = executable().ok_or_else(|| {
        "Chat needs Claude Code (or an API key in Settings). Install it from claude.com/claude-code.".to_string()
    })?;

    // Its own folder, so these sessions don't land among the user's projects.
    let cwd = crate::settings::local_dir().join("chat");
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;

    let mut prompt = String::new();
    let mut extra_dir: Option<PathBuf> = None;
    // Context rides along with the first message only, as with the API path.
    if session.is_none() {
        match &context {
            Some(ChatContext::File { name, path }) => {
                prompt.push_str(&format!(
                    "The user dropped a file on you: \"{name}\" at {path}. \
                     Read it with the Read tool before answering.\n\n"
                ));
                extra_dir = std::path::Path::new(path).parent().map(PathBuf::from);
            }
            Some(ChatContext::Window { app_name, title, url }) => {
                prompt.push_str(&format!("Context — App: {app_name}, Window: {title}"));
                if let Some(url) = url {
                    prompt.push_str(&format!(", URL: {url}"));
                }
                prompt.push_str("\n\n");
            }
            None => {}
        }
    }
    prompt.push_str(&query);

    let mut cmd = tokio::process::Command::new(exe);
    cmd.current_dir(&cwd)
        .arg("-p")
        .args(["--output-format", "json"])
        .args(["--setting-sources", ""])
        .arg("--strict-mcp-config")
        .arg("--disable-slash-commands")
        .arg("--exclude-dynamic-system-prompt-sections")
        .args(["--permission-mode", "dontAsk"])
        .arg("--tools")
        .args(TOOLS)
        .arg("--allowedTools")
        .args(TOOLS)
        .args(["--append-system-prompt", system_prompt]);
    if let Some(dir) = &extra_dir {
        cmd.arg("--add-dir").arg(dir);
    }
    if let Some(id) = &session {
        cmd.args(["--resume", id]);
    }
    cmd.env(CHAT_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start Claude Code: {e}"))?;
    // The prompt goes in on stdin, never on the command line.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await.map_err(|e| e.to_string())?;
    }

    let output = match tokio::time::timeout(TURN_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Claude Code failed: {e}")),
        Err(_) => return Err("Claude Code took too long to answer.".into()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Ok(result) = serde_json::from_str::<Value>(stdout.trim()) else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output");
        crate::log::line(format!("chat: claude exited {} — {detail}", output.status));
        return Err(format!("Claude Code: {detail}"));
    };

    let text = result.get("result").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if result.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        return Err(if text.is_empty() { "Claude Code returned an error.".into() } else { text });
    }
    if text.is_empty() {
        return Err("No response text.".into());
    }
    let session = result
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((ChatReply { text }, session))
}
