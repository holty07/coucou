// Bridge to the KDE Plasma widget.
//
// On Linux the island is not a Tauri window: it lives inside a Plasma widget
// (`plasma/` at the repository root) that renders the very same front end in a
// QtWebEngine view. The widget cannot speak Tauri IPC, so the app serves it here:
//
//   * GET  /…   the built front end (index.html, assets, sounds), from the same
//               embedded assets the Tauri windows use;
//   * GET  /ws  a WebSocket carrying `{id, cmd, args}` calls and their replies,
//               plus every event the island window would have received.
//
// Only 127.0.0.1, and only with the token written to
// `$XDG_RUNTIME_DIR/coucou/plasma.json` (0600, in a 0700 directory), so neither
// another account on the machine nor a web page in a browser can drive it. The
// command list is deliberately the island's, not the settings window's:
// installing hooks and writing keys stay behind the real Settings window.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, mpsc};

use crate::log;

/// Every event the island would get, already serialised as `{event, payload}`.
static EVENTS: OnceLock<broadcast::Sender<String>> = OnceLock::new();

/// Who has said what about each permission request. With several widgets (a
/// panel one and a desktop one), one that can't show the card must not cancel
/// it for another that can: the request only goes back to the terminal once no
/// widget is showing it and every widget has said so.
#[derive(Default)]
struct Verdicts {
    showing: std::collections::HashSet<u64>,
    declined: std::collections::HashSet<u64>,
}

static APPROVALS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Verdicts>>> =
    std::sync::LazyLock::new(Default::default);
static NEXT_CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn widgets_connected() -> usize {
    EVENTS.get().map(|tx| tx.receiver_count()).unwrap_or(0)
}

fn approval_ack(app: &AppHandle, client: u64, request_id: String) {
    {
        let mut map = APPROVALS.lock().unwrap();
        // Requests that ran out of time without an answer leave entries behind;
        // there is only ever one live card, so a handful is plenty.
        if map.len() > 32 && !map.contains_key(&request_id) {
            map.clear();
        }
        let v = map.entry(request_id.clone()).or_default();
        v.declined.remove(&client);
        v.showing.insert(client);
    }
    crate::approval_ack(app.clone(), request_id);
}

fn approval_decline(app: &AppHandle, client: u64, request_id: String) {
    let release = {
        let mut map = APPROVALS.lock().unwrap();
        let v = map.entry(request_id.clone()).or_default();
        v.showing.remove(&client);
        v.declined.insert(client);
        let release = v.showing.is_empty() && v.declined.len() >= widgets_connected();
        if release {
            map.remove(&request_id);
        }
        release
    };
    if release {
        crate::approval_decline(app.clone(), request_id);
    }
}

fn approval_decision(app: &AppHandle, request_id: String, decision: String) {
    APPROVALS.lock().unwrap().remove(&request_id);
    crate::approval_decision(app.clone(), request_id, decision);
}

/// Largest message a widget may send: a chat turn, never a file.
const MAX_MESSAGE: usize = 1 << 20;

struct Ctx {
    app: AppHandle,
    token: String,
    /// Origins a browser-based client may connect from: our own server, and the
    /// Vite dev server in a dev build.
    origins: Vec<String>,
    csp: String,
}

/// The latest update per integration, handed to a widget as it connects so a
/// panel that loads late doesn't sit idle until the next poll (GitHub's is 5 min).
static LATEST: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(Default::default);

/// Forwards an island event to every connected widget.
pub fn broadcast(event: &str, payload: &Value) {
    if event == "integration" {
        if let Some(id) = payload.get("id").and_then(Value::as_str) {
            // Without the one-off event: a late joiner gets the state, not the news.
            let mut replay = payload.clone();
            replay["event"] = Value::Null;
            let text = json!({ "event": event, "payload": replay }).to_string();
            LATEST.lock().unwrap().insert(id.to_string(), text);
        }
    }
    if let Some(tx) = EVENTS.get() {
        if tx.receiver_count() > 0 {
            let _ = tx.send(json!({ "event": event, "payload": payload }).to_string());
        }
    }
}

pub fn start(app: AppHandle) {
    let (tx, _) = broadcast::channel(256);
    let _ = EVENTS.set(tx);

    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(err) => {
                log::line(format!("plasma: cannot listen: {err}"));
                return;
            }
        };
        let Ok(addr) = listener.local_addr() else { return };
        let own = format!("http://127.0.0.1:{}", addr.port());
        let Some(token) = random_token() else {
            log::line("plasma: no randomness for the token — widget bridge disabled");
            return;
        };

        // In a dev build the pages come from Vite; a bundled build serves its own.
        #[allow(unused_mut)]
        let mut origins = vec![own.clone()];
        #[allow(unused_mut)]
        let mut page_base = own.clone();
        #[cfg(dev)]
        if let Some(dev) = app.config().build.dev_url.clone() {
            let dev = dev.as_str().trim_end_matches('/').to_string();
            origins.push(dev.clone());
            page_base = dev;
        }

        let ws_url = format!("ws://127.0.0.1:{}/ws", addr.port());
        if let Err(err) = write_connection_file(&page_base, &ws_url, &token) {
            log::line(format!("plasma: cannot write the connection file: {err}"));
            return;
        }
        log::line(format!("plasma: widget bridge on {own}"));

        let csp = format!(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self' {ws_url}"
        );
        let ctx = Arc::new(Ctx { app, token, origins, csp });
        let router = Router::new()
            .route("/ws", get(upgrade))
            .fallback(get(asset))
            .with_state(ctx);
        if let Err(err) = axum::serve(listener, router).await {
            log::line(format!("plasma: server stopped: {err}"));
        }
    });
}

/// `$XDG_RUNTIME_DIR/coucou/plasma.json`, readable by us only.
pub fn connection_file() -> std::path::PathBuf {
    crate::runtime_dir().join("plasma.json")
}

fn write_connection_file(page: &str, ws: &str, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = connection_file();
    crate::ensure_private_dir(path.parent().unwrap())?;
    let body = json!({ "page": page, "ws": ws, "token": token, "pid": std::process::id() });
    // Written beside the target and renamed over it, so the widget never reads half a file.
    let temp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&temp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(body.to_string().as_bytes())?;
    drop(file);
    std::fs::rename(&temp, &path)
}

fn random_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── Static front end ─────────────────────────────────────────────────────────

async fn asset(State(ctx): State<Arc<Ctx>>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if path.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match ctx.app.asset_resolver().get(path.to_string()) {
        Some(asset) => (
            [
                (header::CONTENT_TYPE, asset.mime_type().to_string()),
                (header::CONTENT_SECURITY_POLICY, ctx.csp.clone()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            asset.bytes().to_vec(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── WebSocket ────────────────────────────────────────────────────────────────

async fn upgrade(
    State(ctx): State<Arc<Ctx>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let token_ok = query.get("token").map(|t| same(t, &ctx.token)).unwrap_or(false);
    // Browsers always send Origin; a page on some website must not get in even
    // if it guessed the port.
    let origin_ok = match headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        Some(origin) => ctx.origins.iter().any(|o| o == origin),
        None => true,
    };
    if !token_ok || !origin_ok {
        log::line("plasma: refused a connection (bad token or origin)");
        return StatusCode::FORBIDDEN.into_response();
    }
    let app = ctx.app.clone();
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| session(app, socket))
}

async fn session(app: AppHandle, mut socket: WebSocket) {
    let Some(tx) = EVENTS.get() else { return };
    let mut events = tx.subscribe();
    // Replies come back here so a slow command (a chat turn) never holds up
    // the events behind it.
    let (reply_tx, mut replies) = mpsc::channel::<String>(32);
    let client = NEXT_CLIENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    log::line("plasma: widget connected");
    let catch_up: Vec<String> = LATEST.lock().unwrap().values().cloned().collect();
    for text in catch_up {
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break };
                let Message::Text(text) = msg else { continue };
                let Ok(call) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                let id = call.get("id").cloned().unwrap_or(Value::Null);
                let cmd = call.get("cmd").and_then(Value::as_str).unwrap_or_default().to_string();
                let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                let app = app.clone();
                let reply_tx = reply_tx.clone();
                tauri::async_runtime::spawn(async move {
                    let reply = match dispatch(&app, client, &cmd, args).await {
                        Ok(result) => json!({ "id": id, "ok": true, "result": result }),
                        Err(error) => json!({ "id": id, "ok": false, "error": error }),
                    };
                    let _ = reply_tx.send(reply.to_string()).await;
                });
            }
            Some(reply) = replies.recv() => {
                if socket.send(Message::Text(reply.into())).await.is_err() { break }
            }
            event = events.recv() => match event {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() { break }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::line(format!("plasma: widget fell behind, {n} events dropped"));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    log::line("plasma: widget disconnected");
    // A widget that goes away can't be showing a card any more.
    let pending: Vec<String> = APPROVALS
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, v)| v.showing.contains(&client))
        .map(|(id, _)| id.clone())
        .collect();
    drop(events);
    for id in pending {
        approval_decline(&app, client, id);
    }
}

fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("bad argument {name}: {e}"))
}

fn ok<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// The island's commands, as in `tauri::generate_handler!` in lib.rs. Anything
/// that writes ~/.claude/settings.json or a key is absent on purpose.
async fn dispatch(app: &AppHandle, client: u64, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
        "boot" => ok(crate::boot(app.clone(), app.state())),
        "save_settings" => ok(crate::save_settings(app.clone(), app.state(), arg(&args, "settings")?)),
        // The widget owns its own geometry, focus and mouse.
        "set_collapsed" | "set_island_rect" | "focus_window" | "reposition" => ok(()),
        "open_url" => ok(crate::open_url(arg(&args, "url")?)),
        "open_in_vscode" => ok(crate::open_in_vscode(arg(&args, "path")?)),
        "open_session" => ok(crate::open_session(arg(&args, "path")?, arg(&args, "pane")?)),
        "open_settings_window" => ok(crate::show_settings_window(app)),
        "quit_app" => ok(app.exit(0)),
        "log_line" => ok(crate::log_line(arg(&args, "message")?)),
        "hooks_status" => ok(crate::hooks_status()),
        "approval_decision" => ok(approval_decision(app, arg(&args, "requestId")?, arg(&args, "decision")?)),
        "approval_ack" => ok(approval_ack(app, client, arg(&args, "requestId")?)),
        "approval_decline" => ok(approval_decline(app, client, arg(&args, "requestId")?)),
        "chat_send" => ok(crate::chat_send(
            app.state(),
            app.state(),
            arg(&args, "query")?,
            arg(&args, "context")?,
        )
        .await?),
        "chat_reset" => ok(crate::chat_reset(app.state())),
        "ingest_file" => ok(crate::ingest_file(arg(&args, "path")?)?),
        "secret_present" => ok(crate::secret_present(arg(&args, "key")?)),
        "refresh_integration" => {
            ok(crate::refresh_integration(app.clone(), arg(&args, "id")?).await)
        }
        "open_n8n" => ok(crate::open_n8n()),
        "set_paused" => ok(crate::set_paused(arg(&args, "paused")?)),
        // One widget's popup was opened: everything that wanted attention has
        // been seen, on every panel (one widget per monitor, say).
        "attention_seen" => {
            broadcast("attention-seen", &Value::Null);
            ok(())
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}
