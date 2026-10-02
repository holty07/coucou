// The desktop board: Coucou as a KDE Plasma desktop widget.
//
// The panel widget is the island; on the desktop Mochi gets a board instead —
// one row per Claude Code session, the plan's usage, and the integrations worth
// a glance. It shows, it never acts: no approvals, no sounds, and it never opens
// or raises anything by itself. A click on a row jumps to that session.
//
// This page only works out what the board says; the widget draws it in QML
// (BoardView.qml). Nothing is drawn here on purpose: a Chromium canvas animating
// all day inside plasmashell piles up native memory that only a forced
// collection gives back (every save()/clip() leaves some), and the kernel ended
// up OOM-killing plasmashell. Qt Quick draws Mochi for free.

import { Bridge, onEvent, type IntegrationUpdate } from "../core/bridge";
import { Plasma } from "../core/plasma";
import { colorForProject, type BotStateName } from "../core/layout";
import type { Settings } from "../core/state";
import { githubSummary, githubView, githubWaiting } from "../core/github";
import { RANK, sessions, type RateWindow, type Session } from "./sessions";

// ── Words ─────────────────────────────────────────────────────────────────────

function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** A finished Mochi celebrates for a while, then rests; a long-idle one sleeps. */
function moodOf(s: Session): BotStateName {
  const age = Date.now() - s.since;
  if (s.state === "finished" && age > 120_000) return "idle";
  if (s.state === "idle" && age > 30 * 60_000) return "sleeping";
  return s.state;
}

function statusOf(s: Session): { text: string; tone: string } {
  switch (s.state) {
    case "approval": return { text: "Needs approval", tone: "amber" };
    case "question": return { text: "Question", tone: "amber" };
    case "error": return { text: "Error", tone: "red" };
    case "ratelimit": return { text: "Rate limited", tone: "amber" };
    case "thinking": return { text: "Thinking", tone: "violet" };
    case "working":
    case "searching": return { text: "Working", tone: "blue" };
    case "finished": return { text: `Done · ${ago(s.since)}`, tone: "green" };
    default: return { text: `Idle · ${ago(s.since)}`, tone: "dim" };
  }
}

const WANTS = (s: Session) => RANK[s.state] <= RANK.ratelimit;
const BUSY = (s: Session) => s.state === "working" || s.state === "thinking" || s.state === "searching";

function heroMood(list: Session[]): BotStateName {
  if (list.length === 0) return "sleeping";
  const top = list[0];
  if (WANTS(top)) return top.state;
  const busy = list.find(BUSY);
  if (busy) return busy.state;
  if (list.some((s) => moodOf(s) === "finished")) return "finished";
  return list.every((s) => moodOf(s) === "sleeping") ? "sleeping" : "idle";
}

function summary(list: Session[]): string {
  if (list.length === 0) return "No sessions running";
  const parts: string[] = [];
  const wants = list.filter(WANTS).length;
  const busy = list.filter(BUSY).length;
  const done = list.filter((s) => s.state === "finished").length;
  if (wants) parts.push(`${wants} need${wants === 1 ? "s" : ""} you`);
  if (busy) parts.push(`${busy} working`);
  if (done) parts.push(`${done} done`);
  parts.push(`${list.length} session${list.length === 1 ? "" : "s"}`);
  return parts.join(" · ");
}

function meter(label: string, w: RateWindow | undefined, receivedAt: number | null) {
  if (!w || typeof w.used_percentage !== "number") return null;
  const resets = (w.resets_at ?? 0) * 1000;
  // The window has rolled over since the last status line: the figure is gone.
  const stale = resets > 0 && Date.now() > resets && (receivedAt ?? 0) * 1000 < resets;
  const pct = stale ? 0 : Math.max(0, Math.min(100, w.used_percentage));
  return {
    label,
    pct,
    text: stale ? "—" : `${Math.round(pct)}%`,
    tone: pct >= 90 ? "red" : pct >= 70 ? "amber" : "blue",
    reset: resets && !stale
      ? new Date(resets).toLocaleString(undefined, label === "5h"
        ? { hour: "numeric", minute: "2-digit" }
        : { weekday: "short", hour: "numeric" })
      : "",
  };
}

// ── Integrations ──────────────────────────────────────────────────────────────

const SHOWN = [
  { id: "integration_slack", name: "Slack", color: "#36C5F0", url: "https://app.slack.com/client" },
  { id: "integration_github", name: "GitHub", color: "#F4505E", url: "https://github.com/pulls/review-requested" },
];

const integ = new Map<string, { data: Record<string, unknown>; event: string | null; at: number; error: string | null }>();
let active: string[] = [];

function chipText(id: string): { text: string; hot: boolean } {
  const i = integ.get(id);
  if (!i) return { text: "Checking…", hot: false };
  if (i.error && Object.keys(i.data).length === 0) return { text: "Not connected", hot: false };
  if (id === "integration_slack") {
    const n = Number(i.data.unread ?? 0);
    return n > 0 ? { text: `${n} unread DM${n === 1 ? "" : "s"}`, hot: true } : { text: "No unread DMs", hot: false };
  }
  // GitHub: what just happened, for ten minutes; then what's waiting on you.
  const v = githubView(i.data);
  if (i.event && Date.now() - i.at < 600_000) return { text: i.event, hot: true };
  return { text: githubSummary(v), hot: githubWaiting(v) > 0 };
}

/** Where a chip goes: the PR that needs you when there's exactly one. */
function chipUrl(id: string, fallback: string): string {
  const i = integ.get(id);
  if (id !== "integration_github" || !i) return fallback;
  const v = githubView(i.data);
  const waiting = [...v.reviews, ...v.blocked];
  return waiting.length === 1 && waiting[0].url ? waiting[0].url : fallback;
}

// ── To the widget ─────────────────────────────────────────────────────────────

function board() {
  const list = sessions.list;
  // Two sessions in one folder: number them, oldest first, so they stay put.
  const dupes = new Map<string, number>();
  for (const s of list) dupes.set(s.name, (dupes.get(s.name) ?? 0) + 1);
  const nth = new Map<string, number>();
  const counted = new Map<string, number>();
  for (const s of [...list].sort((a, b) => a.startedAt - b.startedAt)) {
    if ((dupes.get(s.name) ?? 0) < 2) continue;
    const n = (counted.get(s.name) ?? 0) + 1;
    counted.set(s.name, n);
    nth.set(s.id, n);
  }

  const u = sessions.usage;
  return {
    hero: heroMood(list),
    summary: summary(list),
    usage: u
      ? [meter("5h", u.rateLimits.five_hour, u.receivedAt), meter("7d", u.rateLimits.seven_day, u.receivedAt)]
        .filter((m) => m !== null)
      : [],
    rows: list.map((s) => {
      const st = statusOf(s);
      const n = nth.get(s.id);
      return {
        key: s.id,
        name: n ? `${s.name} ${n}` : s.name,
        detail: s.detail,
        tip: [s.cwd, s.detail].filter(Boolean).join("\n"),
        status: st.text,
        tone: st.tone,
        mood: moodOf(s),
        tint: colorForProject(s.name),
        wants: WANTS(s),
        quiet: s.state === "idle",
      };
    }),
    chips: SHOWN.filter((c) => active.includes(c.id))
      .map((c) => ({ ...c, url: chipUrl(c.id, c.url), ...chipText(c.id) })),
  };
}

let last = "";
function publish() {
  const payload = board();
  const key = JSON.stringify(payload);
  if (key === last) return;
  last = key;
  Plasma.post({ type: "board", payload });
}

async function main() {
  Plasma.onHost("board-open", (id) => {
    const s = sessions.list.find((x) => x.id === id);
    if (s) void Bridge.openSession(s.cwd || null, s.pane);
  });
  Plasma.onHost("board-url", (url) => {
    // Only what the board itself offered: GitHub's own pages, Slack.
    const ok = typeof url === "string"
      && (SHOWN.some((c) => c.url === url) || url.startsWith("https://github.com/"));
    if (ok) void Bridge.openUrl(url);
  });
  // The widget was (re)created and wants the whole board again.
  Plasma.onHost("board-hello", () => {
    last = "";
    publish();
  });

  const boot = await Bridge.boot();
  active = (boot?.settings as Settings | undefined)?.activeIntegrations ?? [];
  void onEvent<Settings>("settings-changed", (s) => {
    active = s.activeIntegrations ?? active;
    publish();
  });
  void onEvent<IntegrationUpdate>("integration", (u) => {
    const prev = integ.get(u.id);
    integ.set(u.id, {
      data: u.error ? (prev?.data ?? {}) : u.data,
      error: u.error,
      event: u.event ? [u.event.label, u.event.detail].filter(Boolean).join(" · ") : (prev?.event ?? null),
      at: u.event ? Date.now() : (prev?.at ?? 0),
    });
    publish();
  });

  sessions.subscribe(publish);
  sessions.start();
  publish();
  // "2m ago" has to move on by itself.
  window.setInterval(publish, 15_000);
}

void main();
