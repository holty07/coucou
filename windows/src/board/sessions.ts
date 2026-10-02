// Every Claude Code session, one row each — the desktop board's model.
//
// Two sources: Claude Code's own session registry (polled through the app, it
// lists every running session, even those started before Coucou), and the hook
// events, which say what each session is doing right now. The island folds all
// sessions into one pill; here they are kept apart by session id.
//
// The board never acts on anything. A permission request is shown and handed
// straight back (another widget may still be showing it — the app only returns
// it to the terminal once no widget is).

import { Bridge, onEvent } from "../core/bridge";
import { Plasma } from "../core/plasma";
import type { BotStateName } from "../core/layout";

export interface Session {
  id: string;
  name: string;
  cwd: string;
  pane: string | null;
  state: BotStateName;
  /** What it is doing, or what it wants. */
  detail: string;
  /** When `state` last changed (ms). */
  since: number;
  /** When a hook last spoke for it (ms); 0 = never, the registry is all we know. */
  lastHook: number;
  startedAt: number;
}

interface RegistryEntry {
  sessionId: string;
  pid: number;
  cwd: string;
  name: string;
  status: string;
  startedAt: number;
  statusUpdatedAt: number;
  pane: string | null;
}

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  message?: string;
  prompt?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
  herdr_pane_id?: string;
}

export interface RateWindow {
  used_percentage?: number;
  resets_at?: number;
}

export interface Usage {
  rateLimits: { five_hour?: RateWindow; seven_day?: RateWindow };
  receivedAt: number | null;
}

/** Most urgent first: what the board sorts by and what the big Mochi shows. */
export const RANK: Record<BotStateName, number> = {
  approval: 0, question: 1, error: 2, ratelimit: 3,
  working: 4, thinking: 4, searching: 4, dizzy: 4,
  finished: 5, idle: 6, sleeping: 6,
};

const TOOL_LABELS: Record<string, string> = {
  Bash: "Running", Read: "Reading", Write: "Writing", Edit: "Editing", MultiEdit: "Editing",
  Glob: "Finding", Grep: "Searching", WebSearch: "Searching the web", WebFetch: "Fetching",
  TodoWrite: "Planning", Task: "Running an agent", Agent: "Running an agent", LS: "Listing",
  NotebookEdit: "Editing a notebook",
};

function baseName(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  return cleaned.slice(cleaned.lastIndexOf("/") + 1) || "~";
}

function str(input: Record<string, unknown>, ...keys: string[]): string | null {
  for (const k of keys) {
    const v = input[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return null;
}

function toolStep(tool: string, input: Record<string, unknown>): string {
  const label = TOOL_LABELS[tool] ?? tool;
  const file = str(input, "file_path", "path", "notebook_path");
  if (file) return `${label} ${baseName(file)}`;
  const what = str(input, "command", "query", "pattern", "url", "description");
  return what ? `${label} · ${what.split("\n")[0]}` : label;
}

/** A session quietly busy with no hook for this long has stopped (Esc fires no Stop). */
const HOOK_STALE_MS = 20_000;
/** A session only hooks have seen, gone from the registry for this long, is over. */
const UNREGISTERED_GRACE_MS = 30_000;

class Sessions {
  private map = new Map<string, Session>();
  usage: Usage | null = null;
  private listeners = new Set<() => void>();

  subscribe(fn: () => void) {
    this.listeners.add(fn);
  }

  private notify() {
    for (const fn of this.listeners) fn();
  }

  /** Most urgent first, then oldest first, so rows don't shuffle while you read. */
  get list(): Session[] {
    return [...this.map.values()].sort(
      (a, b) => RANK[a.state] - RANK[b.state] || a.startedAt - b.startedAt,
    );
  }

  start() {
    void onEvent<HookPayload>("hook", (p) => this.onHook(p));
    void this.poll();
    window.setInterval(() => void this.poll(), 3000);
  }

  private set(s: Session, state: BotStateName, detail?: string) {
    if (s.state !== state) s.since = Date.now();
    s.state = state;
    if (detail !== undefined) s.detail = detail;
  }

  private async poll() {
    const [registry, usage] = await Promise.all([
      Plasma.call<RegistryEntry[]>("claude_sessions").catch(() => null),
      Plasma.call<Usage | null>("claude_usage").catch(() => null),
    ]);
    this.usage = usage;
    if (!registry) return this.notify();

    const now = Date.now();
    const live = new Set<string>();
    for (const r of registry) {
      if (!r.sessionId) continue;
      live.add(r.sessionId);
      let s = this.map.get(r.sessionId);
      if (!s) {
        s = {
          id: r.sessionId, name: baseName(r.cwd), cwd: r.cwd, pane: r.pane,
          state: "idle", detail: "", since: r.statusUpdatedAt || now, lastHook: 0,
          startedAt: r.startedAt,
        };
        this.map.set(s.id, s);
      }
      s.pane = r.pane ?? s.pane;
      s.startedAt = r.startedAt || s.startedAt;
      const busy = r.status === "busy";
      if (s.lastHook === 0) {
        // Nothing from the hooks yet: busy/idle is all we know.
        if (busy && RANK[s.state] > RANK.working) this.set(s, "working", "Working…");
        if (!busy && s.state === "working") this.set(s, "idle", "");
      } else if (!busy && (s.state === "working" || s.state === "thinking")
          && now - s.lastHook > HOOK_STALE_MS) {
        this.set(s, "idle", "Stopped");
      }
    }
    for (const [id, s] of this.map) {
      if (!live.has(id) && now - s.lastHook > UNREGISTERED_GRACE_MS) this.map.delete(id);
    }
    this.notify();
  }

  private onHook(p: HookPayload) {
    const id = p.session_id;
    if (!id) return;
    const event = p.hook_event_name ?? "";
    const now = Date.now();

    if (event === "PermissionRequest" && p.request_id) {
      // Shown here, decided elsewhere.
      void Bridge.approvalDecline(p.request_id);
    }
    if (event === "SessionEnd") {
      this.map.delete(id);
      return this.notify();
    }

    let s = this.map.get(id);
    if (!s) {
      const cwd = p.cwd ?? "";
      s = {
        id, name: baseName(cwd), cwd, pane: null, state: "idle", detail: "",
        since: now, lastHook: now, startedAt: now,
      };
      this.map.set(id, s);
    }
    s.lastHook = now;
    if (p.herdr_pane_id) s.pane = p.herdr_pane_id;

    switch (event) {
      case "SessionStart":
        this.set(s, "idle", "Started");
        break;
      case "UserPromptSubmit": {
        const asked = (p.prompt ?? p.message ?? "").split("\n")[0];
        this.set(s, "thinking", asked ? `“${asked}”` : "Thinking…");
        break;
      }
      case "PreToolUse":
        this.set(s, "working", toolStep(p.tool_name ?? "Tool", p.tool_input ?? {}));
        break;
      case "PostToolUse":
        // After an approval, back to work without losing the step.
        if (s.state !== "working") this.set(s, "working");
        break;
      case "PostToolUseFailure":
        this.set(s, "working", `${s.detail} — failed`);
        break;
      case "Notification": {
        const m = p.message ?? "";
        const lower = m.toLowerCase();
        if (lower.includes("rate limit") || lower.includes("usage limit")) this.set(s, "ratelimit", m);
        else if (lower.includes("permission")) {
          // Follows the PermissionRequest, whose detail says more.
          if (s.state !== "approval") this.set(s, "approval", m);
        }
        else if (lower.includes("waiting for your input")) {
          // Claude Code's idle nudge: the turn is over, nothing new to say.
          if (s.state !== "finished") this.set(s, "finished", "Waiting for you");
        } else if (m) this.set(s, "question", m);
        break;
      }
      case "PermissionRequest": {
        const tool = p.tool_name ?? "Tool";
        const target = str(p.tool_input ?? {}, "command", "file_path", "path", "url", "query", "pattern");
        this.set(s, "approval", target ? `${tool} · ${target.split("\n")[0]}` : tool);
        break;
      }
      case "Stop":
        this.set(s, "finished", p.message?.split("\n")[0] || "Done");
        break;
      case "StopFailure":
        this.set(s, "error", p.message?.split("\n")[0] || "Stopped on an error");
        break;
      case "SubagentStart":
        if (s.state !== "working") this.set(s, "working", "Running an agent");
        break;
    }
    this.notify();
  }
}

export const sessions = new Sessions();
