// Integration pollers — the Rust side of StripePoller / GithubPoller /
// VercelPoller / N8nPoller / ResendPoller / NotionPoller / CalcomPoller, plus
// Slack (this fork; no macOS counterpart).
//
// Same endpoints, same first-run delays and intervals as the Swift pollers. Each
// one emits an `integration` event; the island owns the badge, the sound and the
// 60 s auto-clear, exactly as the Swift handlers do.
//
// Nothing is polled until its key exists in the Credential Manager, and no
// request goes anywhere the user has not configured.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::log;
use crate::secrets;

const TIMEOUT: Duration = Duration::from_secs(10);

/// What the island receives. `event` is only set when something actually changed,
/// which is what drives the pill badge and the sound.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationUpdate {
    pub id: &'static str,
    pub data: Value,
    pub error: Option<String>,
    pub event: Option<IntegrationEvent>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEvent {
    pub success: bool,
    pub label: String,
    pub detail: Option<String>,
}

fn emit(app: &AppHandle, update: IntegrationUpdate) {
    crate::emit_island(app, "integration", update);
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// Set from the tray's Pause item. While it is on, nothing reaches the network:
/// pausing Coucou has to mean pausing Coucou, not just hiding the island.
pub static PAUSED: AtomicBool = AtomicBool::new(false);

pub fn set_paused(on: bool) {
    PAUSED.store(on, Ordering::Relaxed);
}

/// Spawns every poller with the macOS delays and intervals.
pub fn start(app: AppHandle) {
    spawn(app.clone(), "integration_n8n", 3, 15, poll_n8n);
    spawn(app.clone(), "integration_vercel", 5, 30, poll_vercel);
    spawn(app.clone(), "integration_stripe", 6, 30, poll_stripe);
    spawn(app.clone(), "integration_resend", 6, 60, poll_resend);
    // Reviews and CI are worth knowing about within a minute.
    spawn(app.clone(), "integration_github", 7, 60, poll_github);
    spawn(app.clone(), "integration_calcom", 8, 300, poll_calcom);
    spawn(app.clone(), "integration_notion", 9, 300, poll_notion);
    spawn(app, "integration_slack", 4, 60, poll_slack);
}

/// True when the user has this integration switched on in settings.
fn enabled(app: &AppHandle, id: &str) -> bool {
    app.try_state::<crate::Shared>()
        .map(|shared| {
            let settings = shared.settings.lock().unwrap();
            settings.active_integrations.iter().any(|x| x == id)
        })
        .unwrap_or(false)
}

fn spawn<F, Fut>(app: AppHandle, id: &'static str, delay_secs: u64, every_secs: u64, poll: F)
where
    F: Fn(AppHandle) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(every_secs));
        loop {
            ticker.tick().await;
            // The ticker keeps its cadence; we just decline to do the work. An
            // integration the user switched off, or a paused app, must make no
            // network calls at all — CLAUDE.md allows talking only to services
            // the user configured, and a disabled one is not configured.
            if PAUSED.load(Ordering::Relaxed) || !enabled(&app, id) {
                continue;
            }
            poll(app.clone()).await;
        }
    });
}

/// One-shot refresh from the Refresh buttons in the island.
pub async fn poll_once(app: AppHandle, id: &str) {
    match id {
        "integration_stripe" => poll_stripe(app).await,
        "integration_github" => poll_github(app).await,
        "integration_vercel" => poll_vercel(app).await,
        "integration_n8n" => poll_n8n(app).await,
        "integration_resend" => poll_resend(app).await,
        "integration_notion" => poll_notion(app).await,
        "integration_calcom" => poll_calcom(app).await,
        "integration_slack" => poll_slack(app).await,
        _ => {}
    }
}

/// Remembers the newest id per integration so an event fires once, not on every poll.
struct Seen(Mutex<std::collections::HashMap<&'static str, String>>);

static SEEN: std::sync::LazyLock<Seen> =
    std::sync::LazyLock::new(|| Seen(Mutex::new(std::collections::HashMap::new())));

/// Returns true the first time a given id is seen (and false on the very first
/// load, which only fills the card).
fn is_new(key: &'static str, id: &str) -> bool {
    let mut map = SEEN.0.lock().unwrap();
    match map.insert(key, id.to_string()) {
        Some(previous) => previous != id,
        None => false, // first poll: populate silently, like the Swift pollers
    }
}

fn status_error(code: u16, unauthorised_hint: &str) -> String {
    match code {
        401 => "Invalid API key (401)".into(),
        403 => unauthorised_hint.into(),
        _ => format!("API error {code}"),
    }
}

// ── Stripe ────────────────────────────────────────────────────────────────────

async fn poll_stripe(app: AppHandle) {
    let Some(key) = secrets::get("stripe-api-key") else { return };
    let auth = format!("Basic {}", crate::claude::base64_for(format!("{key}:").as_bytes()));
    let http = client();

    let balance = http
        .get("https://api.stripe.com/v1/balance")
        .header("Authorization", &auth)
        .send()
        .await;

    let (amount, currency) = match balance {
        Ok(r) if r.status().is_success() => {
            let json: Value = r.json().await.unwrap_or(json!({}));
            let mut buckets: Vec<Value> = Vec::new();
            for k in ["available", "pending"] {
                if let Some(arr) = json.get(k).and_then(Value::as_array) {
                    buckets.extend(arr.iter().cloned());
                }
            }
            let currency = buckets
                .first()
                .and_then(|b| b.get("currency"))
                .and_then(Value::as_str)
                .unwrap_or("eur")
                .to_string();
            let amount: i64 = buckets
                .iter()
                .filter_map(|b| b.get("amount").and_then(Value::as_i64))
                .sum();
            (amount, currency)
        }
        Ok(r) => {
            let code = r.status().as_u16();
            emit(&app, IntegrationUpdate {
                id: "integration_stripe",
                data: json!({}),
                error: Some(status_error(code, "Use a secret key (sk_live_… not pk_live_…)")),
                event: None,
            });
            return;
        }
        Err(e) => {
            emit(&app, IntegrationUpdate {
                id: "integration_stripe",
                data: json!({}),
                error: Some(format!("No connection: {e}")),
                event: None,
            });
            return;
        }
    };

    let charges = http
        .get("https://api.stripe.com/v1/charges?limit=3")
        .header("Authorization", &auth)
        .send()
        .await;
    let Ok(response) = charges else { return };
    if !response.status().is_success() {
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let payments: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let description = c
                        .get("description")
                        .and_then(Value::as_str)
                        .or_else(|| {
                            c.get("billing_details")
                                .and_then(|b| b.get("name"))
                                .and_then(Value::as_str)
                        })
                        .map(str::to_string);
                    Some(json!({
                        "id": c.get("id")?.as_str()?,
                        "amount": c.get("amount")?.as_i64()?,
                        "currency": c.get("currency")?.as_str()?,
                        "description": description,
                        "createdAt": c.get("created").and_then(Value::as_i64).unwrap_or(0) * 1000,
                        "status": c.get("status").and_then(Value::as_str).unwrap_or("succeeded"),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    let newest = payments
        .first()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let event = if !newest.is_empty() && is_new("stripe", &newest) {
        let label = payments[0]
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let cents = payments[0].get("amount").and_then(Value::as_i64).unwrap_or(0);
                format!("{:.2}", cents as f64 / 100.0)
            });
        Some(IntegrationEvent { success: true, label, detail: None })
    } else {
        None
    };

    emit(&app, IntegrationUpdate {
        id: "integration_stripe",
        data: json!({ "balance": amount, "currency": currency, "payments": payments }),
        error: None,
        event,
    });
}

// ── GitHub ────────────────────────────────────────────────────────────────────

// Pull requests that want you: reviews requested from you, and your own open
// PRs whose CI or reviews changed. One GraphQL query per poll — it works with a
// fine-grained token (Pull requests + Commit statuses, read), which can't read
// the notifications inbox. Only repositories the token was granted are seen.

#[derive(Default)]
struct GithubState {
    token_tag: String,
    login: String,
    /// False until the first full poll: that one only fills the card.
    primed: bool,
    reviews: std::collections::HashSet<String>,
    /// PR key → "passing" | "failing" | "pending" | "".
    checks: std::collections::HashMap<String, String>,
    /// PR key → GitHub's reviewDecision.
    decisions: std::collections::HashMap<String, String>,
}

static GITHUB: std::sync::LazyLock<tokio::sync::Mutex<GithubState>> =
    std::sync::LazyLock::new(Default::default);

const GITHUB_PR_FIELDS: &str = "number title url updatedAt isDraft repository { nameWithOwner } author { login }";

async fn github_graphql(token: &str, query: String) -> Result<Value, String> {
    let response = client()
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "Coucou")
        .json(&json!({ "query": query }))
        .send()
        .await
        .map_err(|_| String::new())?;
    if !response.status().is_success() {
        return Err(status_error(response.status().as_u16(), "Token lacks the needed access"));
    }
    let v: Value = response.json().await.map_err(|_| String::new())?;
    match v.get("data") {
        Some(data) if !data.is_null() => Ok(data.clone()),
        _ => Err(v
            .pointer("/errors/0/message")
            .and_then(Value::as_str)
            .unwrap_or("GitHub said no")
            .to_string()),
    }
}

async fn poll_github(app: AppHandle) {
    let Some(token) = secrets::get("github-token") else { return };
    let report = |app: &AppHandle, message: String| {
        log::line(format!("github: {message}"));
        emit(app, IntegrationUpdate { id: "integration_github", data: json!({}), error: Some(message), event: None });
    };

    let mut state = GITHUB.lock().await;
    let tag: String = token.chars().rev().take(8).collect();
    if state.token_tag != tag {
        *state = GithubState { token_tag: tag, ..GithubState::default() };
    }
    if state.login.is_empty() {
        match github_graphql(&token, "{ viewer { login } }".into()).await {
            Ok(v) => state.login = v.pointer("/viewer/login").and_then(Value::as_str).unwrap_or_default().into(),
            Err(m) if m.is_empty() => return, // network blip: next time
            Err(m) => return report(&app, m),
        }
        if state.login.is_empty() {
            return;
        }
    }

    let who = &state.login;
    let query = format!(
        r#"{{
  reviews: search(query: "is:open is:pr archived:false review-requested:{who}", type: ISSUE, first: 20) {{
    nodes {{ ... on PullRequest {{ {GITHUB_PR_FIELDS} }} }}
  }}
  mine: search(query: "is:open is:pr archived:false author:{who}", type: ISSUE, first: 20) {{
    nodes {{ ... on PullRequest {{ {GITHUB_PR_FIELDS} reviewDecision
      commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} }} }}
  }}
}}"#
    );
    let data = match github_graphql(&token, query).await {
        Ok(v) => v,
        Err(m) if m.is_empty() => return,
        Err(m) => return report(&app, m),
    };

    let key = |pr: &Value| {
        format!(
            "{}#{}",
            pr.pointer("/repository/nameWithOwner").and_then(Value::as_str).unwrap_or("?"),
            pr.get("number").and_then(Value::as_i64).unwrap_or(0)
        )
    };
    let summary = |pr: &Value, extra: Value| {
        let mut o = json!({
            "key": key(pr),
            "repo": pr.pointer("/repository/nameWithOwner").and_then(Value::as_str).unwrap_or(""),
            "number": pr.get("number").cloned().unwrap_or(Value::Null),
            "title": pr.get("title").and_then(Value::as_str).unwrap_or(""),
            "url": pr.get("url").and_then(Value::as_str).unwrap_or(""),
            "author": pr.pointer("/author/login").and_then(Value::as_str).unwrap_or(""),
            "updatedAt": pr.get("updatedAt").cloned().unwrap_or(Value::Null),
            "draft": pr.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        });
        if let (Some(o), Some(extra)) = (o.as_object_mut(), extra.as_object()) {
            o.extend(extra.clone());
        }
        o
    };
    let nodes = |name: &str| -> Vec<Value> {
        data.pointer(&format!("/{name}/nodes"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter(|n| n.get("number").is_some()).cloned().collect())
            .unwrap_or_default()
    };

    let reviews: Vec<Value> = nodes("reviews").iter().map(|pr| summary(pr, json!({}))).collect();
    let mine: Vec<Value> = nodes("mine")
        .iter()
        .map(|pr| {
            let checks = match pr
                .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
                .and_then(Value::as_str)
            {
                Some("SUCCESS") => "passing",
                Some("FAILURE" | "ERROR") => "failing",
                Some(_) => "pending",
                None => "",
            };
            let decision = pr.get("reviewDecision").and_then(Value::as_str).unwrap_or("");
            summary(pr, json!({ "checks": checks, "review": decision }))
        })
        .collect();

    // What changed since last time, most pressing first. One event per poll.
    let mut events: Vec<(u8, IntegrationEvent)> = Vec::new();
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let detail = |pr: &Value| Some(format!("{} · {}", short_key(&str_of(pr, "key")), str_of(pr, "title")));
    for pr in &reviews {
        if state.primed && !state.reviews.contains(&str_of(pr, "key")) {
            let who = str_of(pr, "author");
            let label = if who.is_empty() { "Review requested".into() } else { format!("{who} wants your review") };
            events.push((2, IntegrationEvent { success: true, label, detail: detail(pr) }));
        }
    }
    for pr in &mine {
        let k = str_of(pr, "key");
        let checks = str_of(pr, "checks");
        let review = str_of(pr, "review");
        if state.primed {
            let before_checks = state.checks.get(&k).cloned().unwrap_or_default();
            let before_review = state.decisions.get(&k).cloned().unwrap_or_default();
            if checks == "failing" && before_checks != "failing" {
                events.push((0, IntegrationEvent { success: false, label: "CI failed".into(), detail: detail(pr) }));
            } else if checks == "passing" && before_checks == "pending" {
                events.push((4, IntegrationEvent { success: true, label: "CI passed".into(), detail: detail(pr) }));
            }
            if review != before_review {
                match review.as_str() {
                    "CHANGES_REQUESTED" => events.push((1, IntegrationEvent {
                        success: false, label: "Changes requested".into(), detail: detail(pr),
                    })),
                    "APPROVED" => events.push((3, IntegrationEvent {
                        success: true, label: "PR approved".into(), detail: detail(pr),
                    })),
                    _ => {}
                }
            }
        }
    }
    state.reviews = reviews.iter().map(|pr| str_of(pr, "key")).collect();
    state.checks = mine.iter().map(|pr| (str_of(pr, "key"), str_of(pr, "checks"))).collect();
    state.decisions = mine.iter().map(|pr| (str_of(pr, "key"), str_of(pr, "review"))).collect();
    if !state.primed {
        log::line(format!(
            "github: watching as {} — {} review request(s), {} open PR(s) of yours",
            state.login, reviews.len(), mine.len()
        ));
    }
    state.primed = true;
    events.sort_by_key(|(rank, _)| *rank);

    emit(&app, IntegrationUpdate {
        id: "integration_github",
        data: json!({ "login": state.login, "reviews": reviews, "mine": mine }),
        error: None,
        event: events.into_iter().next().map(|(_, e)| e),
    });
}

/// `owner/repo#12` → `repo#12`: the owner is nearly always you or your org.
fn short_key(key: &str) -> &str {
    key.rsplit_once('/').map(|(_, k)| k).unwrap_or(key)
}

// ── Vercel ────────────────────────────────────────────────────────────────────

async fn poll_vercel(app: AppHandle) {
    let Some(token) = secrets::get("vercel-token") else { return };
    let response = client()
        .get("https://api.vercel.com/v6/deployments?limit=5")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_vercel",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Token lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let terminal = ["READY", "ERROR", "CANCELED"];
    let deployments: Vec<Value> = json
        .get("deployments")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    let state = d.get("state")?.as_str()?;
                    if !terminal.contains(&state) {
                        return None;
                    }
                    let meta = d.get("meta");
                    let pick = |keys: [&str; 3]| {
                        meta.and_then(|m| keys.iter().find_map(|k| m.get(*k).and_then(Value::as_str)))
                            .map(str::to_string)
                    };
                    Some(json!({
                        "id": d.get("uid")?.as_str()?,
                        "projectName": d.get("name")?.as_str()?,
                        "url": d.get("url").and_then(Value::as_str).unwrap_or(""),
                        "state": state,
                        "createdAt": d.get("createdAt").and_then(Value::as_f64).unwrap_or(0.0),
                        "commitMessage": pick(["githubCommitMessage", "gitlabCommitMessage", "bitbucketCommitMessage"]),
                        "branch": pick(["githubCommitRef", "gitlabCommitRef", "bitbucketBranch"]),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    let event = deployments.first().and_then(|latest| {
        let id = latest.get("id")?.as_str()?;
        if !is_new("vercel", id) {
            return None;
        }
        let success = latest.get("state")?.as_str()? == "READY";
        Some(IntegrationEvent {
            success,
            label: latest.get("projectName")?.as_str()?.to_string(),
            detail: None,
        })
    });

    emit(&app, IntegrationUpdate {
        id: "integration_vercel",
        data: json!({ "deployments": deployments }),
        error: None,
        event,
    });
}

// ── Resend ────────────────────────────────────────────────────────────────────

async fn poll_resend(app: AppHandle) {
    let Some(key) = secrets::get("resend-api-key") else { return };
    let response = client()
        .get("https://api.resend.com/emails?limit=100")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_resend",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Key lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let total = json
        .get("total")
        .or_else(|| json.get("count"))
        .and_then(Value::as_i64);
    let emails: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(5)
                .filter_map(|e| {
                    let to = match e.get("to") {
                        Some(Value::Array(a)) => a.clone(),
                        Some(Value::String(s)) => vec![Value::String(s.clone())],
                        _ => vec![],
                    };
                    Some(json!({
                        "id": e.get("id")?.as_str()?,
                        "to": to,
                        "subject": e.get("subject").and_then(Value::as_str).unwrap_or(""),
                        "createdAt": e.get("created_at").and_then(Value::as_str).unwrap_or(""),
                        "lastEvent": e.get("last_event").and_then(Value::as_str).unwrap_or(""),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_resend",
        data: json!({ "emails": emails, "total": total }),
        error: None,
        event: None,
    });
}

// ── Slack ─────────────────────────────────────────────────────────────────────
//
// Unread direct messages (1:1 and group DMs). Slack's public API has no "total
// unread" call and only reports unread counts for DMs, one conversation at a
// time (conversations.info, Tier 3 ≈ 50/min). So each poll spends a fixed
// budget: DMs that were unread last time first, then the next slice of the rest
// in rotation. A DM from a quiet contact is noticed within a few polls, never
// at the cost of a rate limit. User token scopes: im:read, mpim:read, users:read.

/// conversations.info calls per poll, under Slack's Tier 3 limit with room to spare.
const SLACK_BUDGET: usize = 40;
/// The DM list itself changes rarely; re-read it every this many polls.
const SLACK_LIST_EVERY: u32 = 10;

#[derive(Default)]
struct SlackState {
    /// Token the cache below belongs to; a new token starts over.
    token_tag: String,
    team: String,
    dms: Vec<SlackDm>,
    polls: u32,
    cursor: usize,
    unread: std::collections::HashMap<String, i64>,
    names: std::collections::HashMap<String, String>,
    /// Newest unread message seen per DM, so a new message fires once.
    seen_latest: std::collections::HashMap<String, String>,
    /// Every DM has been checked once. Until then unread messages are ones the
    /// user already had, not news: fill the card, make no sound.
    swept_once: bool,
    /// DMs Slack lists but won't let us read; never asked about again.
    unreadable: std::collections::HashSet<String>,
}

#[derive(Clone)]
struct SlackDm {
    id: String,
    /// The other person, for a 1:1 DM.
    user: Option<String>,
    /// Slack's `mpdm-a--b--c-1` name, for a group DM.
    group: Option<String>,
}

static SLACK: std::sync::LazyLock<tokio::sync::Mutex<SlackState>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(SlackState::default()));

enum SlackError {
    /// Show this on the card.
    Report(String),
    /// This one conversation can't be read (a Slack Connect DM, a deactivated
    /// user…). Skip it; it says nothing about the token.
    Conversation,
    /// Rate limited or a network blip: try again next poll, say nothing.
    Quiet,
}

async fn slack_call(token: &str, method: &str, query: &[(&str, &str)]) -> Result<Value, SlackError> {
    let response = client()
        .get(format!("https://slack.com/api/{method}"))
        .header("Authorization", format!("Bearer {token}"))
        .query(query)
        .send()
        .await
        .map_err(|e| {
            log::line(format!("slack: {method} failed: {e}"));
            SlackError::Quiet
        })?;
    if response.status().as_u16() == 429 {
        let wait = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        log::line(format!("slack: {method} rate limited, retry after {wait}s"));
        return Err(SlackError::Quiet);
    }
    let json: Value = response.json().await.map_err(|_| SlackError::Quiet)?;
    if json.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(json);
    }
    let err = json.get("error").and_then(Value::as_str).unwrap_or("unknown_error");
    Err(match err {
        "ratelimited" => SlackError::Quiet,
        "channel_not_found" | "not_in_channel" | "is_archived" | "method_not_supported_for_channel_type"
        | "user_not_found" | "user_not_visible" => SlackError::Conversation,
        "invalid_auth" | "not_authed" | "token_revoked" | "token_expired" | "account_inactive" => {
            SlackError::Report("Invalid token".into())
        }
        "missing_scope" => {
            let needed = json.get("needed").and_then(Value::as_str).unwrap_or("im:read, mpim:read, users:read");
            SlackError::Report(format!("Token lacks scope: {needed}"))
        }
        other => SlackError::Report(format!("Slack: {other}")),
    })
}

async fn slack_list_dms(token: &str) -> Result<Vec<SlackDm>, SlackError> {
    let mut out = Vec::new();
    let mut cursor = String::new();
    // A generous ceiling: 10 pages of 200.
    for _ in 0..10 {
        let mut query = vec![("types", "im,mpim"), ("exclude_archived", "true"), ("limit", "200")];
        if !cursor.is_empty() {
            query.push(("cursor", cursor.as_str()));
        }
        let page = slack_call(token, "users.conversations", &query).await?;
        for c in page.get("channels").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = c.get("id").and_then(Value::as_str) else { continue };
            if c.get("is_user_deleted").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            out.push(SlackDm {
                id: id.to_string(),
                user: c.get("user").and_then(Value::as_str).map(str::to_string),
                group: c.get("is_mpim").and_then(Value::as_bool).filter(|m| *m)
                    .and_then(|_| c.get("name").and_then(Value::as_str))
                    .map(str::to_string),
            });
        }
        cursor = page
            .get("response_metadata")
            .and_then(|m| m.get("next_cursor"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// `mpdm-alice--bob--carol-1` → `alice, bob, carol`
fn slack_group_name(raw: &str) -> String {
    let trimmed = raw.strip_prefix("mpdm-").unwrap_or(raw);
    let trimmed = trimmed.rsplit_once('-').map(|(a, _)| a).unwrap_or(trimmed);
    trimmed.split("--").collect::<Vec<_>>().join(", ")
}

async fn slack_user_name(token: &str, state: &mut SlackState, user: &str) -> String {
    if let Some(name) = state.names.get(user) {
        return name.clone();
    }
    let name = match slack_call(token, "users.info", &[("user", user)]).await {
        Ok(v) => {
            let u = v.get("user");
            let profile = u.and_then(|u| u.get("profile"));
            [
                profile.and_then(|p| p.get("display_name")),
                profile.and_then(|p| p.get("real_name")),
                u.and_then(|u| u.get("name")),
            ]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|s| !s.is_empty())
            .unwrap_or("Someone")
            .to_string()
        }
        Err(_) => return "Someone".into(),
    };
    state.names.insert(user.to_string(), name.clone());
    name
}

async fn poll_slack(app: AppHandle) {
    let Some(token) = secrets::get("slack-token") else { return };
    let report = |app: &AppHandle, message: String| {
        emit(app, IntegrationUpdate {
            id: "integration_slack",
            data: json!({}),
            error: Some(message),
            event: None,
        });
    };

    let mut state = SLACK.lock().await;
    let tag: String = token.chars().rev().take(8).collect();
    if state.token_tag != tag {
        *state = SlackState { token_tag: tag, ..SlackState::default() };
    }

    if state.team.is_empty() {
        match slack_call(&token, "auth.test", &[]).await {
            Ok(v) => state.team = v.get("team_id").and_then(Value::as_str).unwrap_or_default().to_string(),
            Err(SlackError::Report(m)) => return report(&app, m),
            Err(SlackError::Quiet | SlackError::Conversation) => return,
        }
    }
    if state.dms.is_empty() || state.polls % SLACK_LIST_EVERY == 0 {
        match slack_list_dms(&token).await {
            Ok(dms) => {
                // Conversations that turned out to be unreadable stay out.
                let skip = std::mem::take(&mut state.unreadable);
                state.dms = dms.into_iter().filter(|d| !skip.contains(&d.id)).collect();
                state.unreadable = skip;
            }
            Err(SlackError::Report(m)) => return report(&app, m),
            Err(SlackError::Quiet | SlackError::Conversation) => {}
        }
    }
    state.polls = state.polls.wrapping_add(1);

    // This poll's slice: everything unread last time, then the rotation.
    let total = state.dms.len();
    let mut batch: Vec<SlackDm> = state
        .dms
        .iter()
        .filter(|d| state.unread.get(&d.id).copied().unwrap_or(0) > 0)
        .take(SLACK_BUDGET)
        .cloned()
        .collect();
    let mut steps = 0;
    let mut wrapped = false;
    while batch.len() < SLACK_BUDGET && steps < total {
        let dm = state.dms[state.cursor % total].clone();
        state.cursor = (state.cursor + 1) % total.max(1);
        wrapped |= state.cursor == 0;
        steps += 1;
        if !batch.iter().any(|b| b.id == dm.id) {
            batch.push(dm);
        }
    }

    let mut newest: Option<(String, String, String)> = None; // (dm id, latest ts, who)
    for dm in &batch {
        let info = match slack_call(&token, "conversations.info", &[("channel", dm.id.as_str())]).await {
            Ok(v) => v,
            Err(SlackError::Report(m)) => return report(&app, m),
            Err(SlackError::Conversation) => {
                log::line(format!("slack: skipping unreadable conversation {}", dm.id));
                state.unreadable.insert(dm.id.clone());
                state.unread.remove(&dm.id);
                continue;
            }
            // Rate limited mid-poll: keep what we have and carry on next time.
            Err(SlackError::Quiet) => break,
        };
        let channel = info.get("channel");
        let count = channel
            .and_then(|c| c.get("unread_count_display").or_else(|| c.get("unread_count")))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        state.unread.insert(dm.id.clone(), count);
        if count > 0 {
            let latest = channel
                .and_then(|c| c.get("latest"))
                .and_then(|l| l.get("ts"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if state.seen_latest.get(&dm.id) != Some(&latest) {
                state.seen_latest.insert(dm.id.clone(), latest.clone());
                if state.swept_once {
                    let who = match (&dm.user, &dm.group) {
                        (Some(u), _) => slack_user_name(&token, &mut state, u).await,
                        (_, Some(g)) => slack_group_name(g),
                        _ => "Someone".into(),
                    };
                    newest = Some((dm.id.clone(), latest, who));
                }
            }
        } else {
            state.seen_latest.remove(&dm.id);
        }
    }

    if !state.unreadable.is_empty() {
        let skip = state.unreadable.clone();
        state.dms.retain(|d| !skip.contains(&d.id));
    }

    // The card: total and the top few senders.
    let mut unread: Vec<(SlackDm, i64)> = state
        .dms
        .iter()
        .filter_map(|d| state.unread.get(&d.id).copied().filter(|n| *n > 0).map(|n| (d.clone(), n)))
        .collect();
    unread.sort_by(|a, b| b.1.cmp(&a.1));
    let total_unread: i64 = unread.iter().map(|(_, n)| n).sum();
    let mut rows = Vec::new();
    for (dm, count) in unread.into_iter().take(4) {
        let who = match (&dm.user, &dm.group) {
            (Some(u), _) => slack_user_name(&token, &mut state, u).await,
            (_, Some(g)) => slack_group_name(g),
            _ => "Someone".into(),
        };
        rows.push(json!({
            "name": who,
            "count": count,
            "url": format!("https://app.slack.com/client/{}/{}", state.team, dm.id),
        }));
    }
    let checked = state.unread.len();
    if wrapped {
        state.swept_once = true;
    }

    emit(&app, IntegrationUpdate {
        id: "integration_slack",
        data: json!({
            "unread": total_unread,
            "conversations": rows,
            "checked": checked,
            "total": total,
            "teamUrl": format!("https://app.slack.com/client/{}", state.team),
        }),
        error: None,
        event: newest.map(|(_, _, who)| IntegrationEvent {
            success: true,
            label: format!("New message from {who}"),
            detail: None,
        }),
    });
}

// ── Notion ────────────────────────────────────────────────────────────────────

async fn poll_notion(app: AppHandle) {
    let Some(token) = secrets::get("notion-api-key") else { return };
    let response = client()
        .post("https://api.notion.com/v1/search")
        .header("Authorization", format!("Bearer {token}"))
        .header("Notion-Version", "2022-06-28")
        .header("Content-Type", "application/json")
        .json(&json!({
            "sort": { "direction": "descending", "timestamp": "last_edited_time" },
            "page_size": 3
        }))
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_notion",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Integration lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let pages: Vec<Value> = json
        .get("results")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(parse_notion_page).collect())
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_notion",
        data: json!({ "pages": pages }),
        error: None,
        event: None,
    });
}

fn parse_notion_page(obj: &Value) -> Option<Value> {
    let id = obj.get("id")?.as_str()?;
    let is_database = obj.get("object").and_then(Value::as_str) == Some("database");

    let mut title = "Untitled".to_string();
    if is_database {
        if let Some(text) = obj
            .get("title")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|t| t.get("plain_text"))
            .and_then(Value::as_str)
        {
            if !text.is_empty() {
                title = text.to_string();
            }
        }
    } else if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for prop in props.values() {
            if prop.get("type").and_then(Value::as_str) != Some("title") {
                continue;
            }
            if let Some(text) = prop
                .get("title")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|t| t.get("plain_text"))
                .and_then(Value::as_str)
            {
                if !text.is_empty() {
                    title = text.to_string();
                    break;
                }
            }
        }
    }

    let emoji = obj
        .get("icon")
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("emoji"))
        .and_then(|i| i.get("emoji"))
        .and_then(Value::as_str);

    Some(json!({
        "id": id,
        "title": title,
        "emoji": emoji,
        "lastEditedAt": obj.get("last_edited_time").and_then(Value::as_str)?,
        "url": obj.get("url").and_then(Value::as_str).unwrap_or("https://notion.so"),
    }))
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

async fn poll_calcom(app: AppHandle) {
    let Some(key) = secrets::get("calcom-api-key") else { return };
    let response = client()
        .get("https://api.cal.com/v2/bookings?status=upcoming")
        .header("Authorization", format!("Bearer {key}"))
        .header("cal-api-version", "2024-08-13")
        .send()
        .await;
    let Ok(response) = response else { return };
    if !response.status().is_success() {
        emit(&app, IntegrationUpdate {
            id: "integration_calcom",
            data: json!({}),
            error: Some(status_error(response.status().as_u16(), "Key lacks access")),
            event: None,
        });
        return;
    }
    let json: Value = response.json().await.unwrap_or(json!({}));
    let bookings: Vec<Value> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let start = b
                        .get("start")
                        .or_else(|| b.get("startTime"))
                        .and_then(Value::as_str)?;
                    let attendee = b.get("attendees").and_then(Value::as_array).and_then(|a| a.first());
                    let notes = b
                        .get("responses")
                        .and_then(|r| r.get("notes"))
                        .and_then(|n| n.get("value"))
                        .and_then(Value::as_str)
                        .or_else(|| b.get("description").and_then(Value::as_str))
                        .filter(|s| !s.is_empty());
                    Some(json!({
                        "id": b.get("id").map(|v| v.to_string()).unwrap_or_default(),
                        "title": b.get("title").and_then(Value::as_str).unwrap_or("Meeting"),
                        "start": start,
                        "status": b.get("status").and_then(Value::as_str).unwrap_or("accepted"),
                        "attendeeName": attendee.and_then(|a| a.get("name")).and_then(Value::as_str),
                        "attendeeEmail": attendee.and_then(|a| a.get("email")).and_then(Value::as_str),
                        "attendeeNotes": notes,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();

    emit(&app, IntegrationUpdate {
        id: "integration_calcom",
        data: json!({ "bookings": bookings }),
        error: None,
        event: None,
    });
}

// ── n8n ───────────────────────────────────────────────────────────────────────

async fn poll_n8n(app: AppHandle) {
    let (Some(key), Some(raw_base)) = (secrets::get("n8n-api-key"), secrets::get("n8n-url")) else {
        return;
    };
    let base = raw_base.trim_end_matches('/').to_string();
    let http = client();

    // Same two shapes as the Swift poller: the public API first, then /rest.
    let list_urls = [
        format!("{base}/api/v1/executions?limit=1&includeData=false"),
        format!("{base}/rest/executions?limit=1&includeData=false"),
    ];

    let mut items: Option<Vec<Value>> = None;
    for url in &list_urls {
        let Ok(response) = http.get(url).header("X-N8N-API-KEY", &key).header("Accept", "application/json").send().await
        else {
            continue;
        };
        if !response.status().is_success() {
            // Only the status: a self-hosted base URL can carry credentials.
            log::line(format!("n8n list HTTP {}", response.status()));
            continue;
        }
        let Ok(json) = response.json::<Value>().await else { continue };
        items = match &json {
            Value::Object(o) => o.get("data").and_then(Value::as_array).cloned(),
            Value::Array(a) => Some(a.clone()),
            _ => None,
        };
        if items.is_some() {
            break;
        }
    }

    let Some(first) = items.and_then(|list| list.into_iter().next()) else { return };
    let id = match first.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return,
    };

    let status = first.get("status").and_then(Value::as_str).unwrap_or("");
    if !["success", "error", "crashed", "canceled", "failed"].contains(&status) {
        return;
    }
    if !is_new("n8n", &id) {
        return;
    }
    let success = status == "success";

    let detail_urls = [
        format!("{base}/api/v1/executions/{id}?includeData=true"),
        format!("{base}/api/v1/executions/{id}"),
        format!("{base}/rest/executions/{id}?includeData=true"),
        format!("{base}/rest/executions/{id}"),
    ];
    let mut name = "Workflow".to_string();
    let mut detail = None;
    for url in &detail_urls {
        let Ok(response) = http.get(url).header("X-N8N-API-KEY", &key).header("Accept", "application/json").send().await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(json) = response.json::<Value>().await else { continue };
        name = json
            .get("workflowData")
            .and_then(|w| w.get("name"))
            .and_then(Value::as_str)
            .or_else(|| json.get("name").and_then(Value::as_str))
            .unwrap_or("Workflow")
            .to_string();
        detail = n8n_detail(&json, success);
        break;
    }

    log::line(format!("n8n execution {id} {status} · {name}"));
    emit(&app, IntegrationUpdate {
        id: "integration_n8n",
        data: json!({ "workflow": name, "status": status }),
        error: None,
        event: Some(IntegrationEvent { success, label: name, detail }),
    });
}

fn n8n_detail(json: &Value, success: bool) -> Option<String> {
    let result = json.get("data")?.get("resultData")?;
    if !success {
        if let Some(error) = result.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            if let Some(node) = error.get("node").and_then(|n| n.get("name")).and_then(Value::as_str) {
                if !node.is_empty() {
                    return Some(format!("{node}\n{message}"));
                }
            }
            return Some(message.to_string());
        }
        let runs = result.get("runData")?.as_object()?;
        for (node, value) in runs {
            if let Some(message) = value
                .as_array()
                .and_then(|a| a.first())
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                return Some(format!("{node}\n{message}"));
            }
        }
        return None;
    }

    let last_node = result.get("lastNodeExecuted")?.as_str()?;
    let items = result
        .get("runData")?
        .get(last_node)?
        .as_array()?
        .first()?
        .get("data")?
        .get("main")?
        .as_array()?
        .first()?
        .as_array()?;
    let count = items.len();
    let header = format!("→ {last_node} · {count} item{}", if count == 1 { "" } else { "s" });

    let fields = items
        .first()
        .and_then(|i| i.get("json"))
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .take(4)
                .map(|(k, v)| format!("{k}: {}", fmt_value(v)))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty());

    Some(match fields {
        Some(f) => format!("{header}\n{f}"),
        None => header,
    })
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.chars().take(50).collect(),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(_) => "{…}".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_dm_names_read_like_people() {
        assert_eq!(slack_group_name("mpdm-alice--bob--carol-1"), "alice, bob, carol");
        assert_eq!(slack_group_name("mpdm-matt--sam-1"), "matt, sam");
    }
}
