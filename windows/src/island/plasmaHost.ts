// Glue between the island and the KDE Plasma widget that hosts it.
//
// Desktop: the widget *is* the island's window — Mochi sits on the desktop in
// the compact island and never retracts on its own.
// Panel: a small Mochi lives in the panel (drawn by the widget) and the island
// lives in its popup. The popup never opens by itself — only a click on the
// panel Mochi or the tray's Open does that. Everything that wants attention (a
// permission request, a finished session, an error, a Slack DM) shows on the
// panel Mochi instead, and stays there until the popup is opened. Permission
// requests that arrive while it is shut go straight to the terminal.

import { Bridge, onEvent } from "../core/bridge";
import { botGlowColor, islandSize, type BotStateName } from "../core/layout";
import { HOST_MODE, Plasma } from "../core/plasma";
import { State } from "../core/state";
import type { AgentTask } from "../core/state";
import type { Island } from "./island";

/**
 * Pills whose state is always live, so it is never kept "until seen": Slack is
 * curious exactly while DMs are unread, and stops the moment they're read.
 */
const LIVE_STATE: ReadonlySet<string> = new Set(["integration_slack"]);

/** States that mean "this Mochi wants you", most urgent first. */
const URGENCY: Partial<Record<BotStateName, number>> = { approval: 4, question: 3, error: 2, finished: 1 };

function urgencyOf(t: AgentTask): BotStateName | null {
  const badge = t.pillBadge;
  const candidates = [t.state, badge].filter((s): s is BotStateName => !!s && !!URGENCY[s as BotStateName]);
  candidates.sort((a, b) => (URGENCY[b] ?? 0) - (URGENCY[a] ?? 0));
  return candidates[0] ?? null;
}

export function installPlasmaHost(island: Island) {
  // No OS cursor poll here: the DOM drives the cursor, and leaving the view
  // must read as leaving the island.
  document.documentElement.addEventListener("mouseleave", () => island.onCursor(-10000, -10000));
  // During a file drag the page gets no mouse events at all; the widget feeds
  // the cursor instead, as the Win32 poll does on Windows.
  Plasma.onHost("cursor", (p) => {
    const { x, y } = p as { x: number; y: number };
    island.onCursor(x, y);
  });
  // The widget's context menu.
  Plasma.onHost("command", (cmd) => {
    if (cmd === "open_settings_window") void Bridge.openSettingsWindow();
  });

  let popupOpen = false;
  /** Attention raised while the popup was shut, kept until it is opened. */
  const unseen = new Map<string, BotStateName>();

  if (HOST_MODE === "desktop") {
    island.fsm.neverHide = true;
  } else {
    State.approvalsVisible = () => popupOpen;
    // Tray Open / Settings are explicit requests, so they may open the popup.
    let askedUntil = 0;
    void onEvent<string>("tray", () => {
      askedUntil = performance.now() + 1000;
    });
    const inner = island.fsm.onTransition;
    island.fsm.onTransition = (from, to) => {
      inner?.(from, to);
      if (to === "home") {
        // Checked once this event's other listeners have run (the tray flag
        // is set after the island has already reacted to it).
        queueMicrotask(() => {
          const asked = performance.now() < askedUntil;
          if (asked || popupOpen) Plasma.post({ type: "expand" });
        });
      } else if (to === "petit" || to === "hidden") {
        Plasma.post({ type: "collapse" });
      }
    };
    // Another panel's popup was opened (or this one's): it has all been seen.
    void onEvent<null>("attention-seen", () => {
      if (unseen.size === 0) return;
      unseen.clear();
      State.notify();
    });
    Plasma.onHost("open", () => {
      popupOpen = true;
      unseen.clear();
      void Bridge.attentionSeen();
      if (island.fsm.state !== "home") island.alert(State.defaultView());
      State.notify();
    });
    // A click on one of the panel Mochis opens the popup on that pill.
    Plasma.onHost("focus", (id) => {
      if (typeof id === "string" && State.tasks.some((t) => t.id === id)) State.setFocus(id);
    });
    Plasma.onHost("close", () => {
      popupOpen = false;
      // A card nobody can see any more must not keep Claude Code waiting.
      const pending = State.pendingApproval;
      if (pending) {
        if (pending.requestId) void Bridge.approvalDecline(pending.requestId);
        State.pendingApproval = null;
        State.isPinned = false;
        island.dropPin();
      }
      if (island.fsm.state === "home") island.fsm.forcePetit();
    });
  }

  // What the widget needs to draw the panel Mochi and size its popup.
  let last = "";
  State.subscribe(() => {
    // Every Mochi that wants you, live or raised while nobody was looking.
    const wanting = new Map<string, BotStateName>();
    for (const t of State.tasks) {
      const s = urgencyOf(t);
      if (!s) continue;
      wanting.set(t.id, s);
      if (!popupOpen && HOST_MODE === "panel" && !LIVE_STATE.has(t.id)) {
        const prev = unseen.get(t.id);
        if (!prev || (URGENCY[s] ?? 0) > (URGENCY[prev] ?? 0)) unseen.set(t.id, s);
      }
    }
    for (const [id, s] of unseen) {
      const live = wanting.get(id);
      if (!live || (URGENCY[s] ?? 0) > (URGENCY[live] ?? 0)) wanting.set(id, s);
    }
    const attention = [...wanting]
      .map(([id, s]) => {
        const t = State.tasks.find((x) => x.id === id);
        return { id, name: t?.name ?? id, color: t?.color ?? "#FFFFFF", state: s };
      })
      .filter((a) => State.tasks.some((t) => t.id === a.id))
      .sort((a, b) => (URGENCY[b.state] ?? 0) - (URGENCY[a.state] ?? 0));

    // The panel draws one Mochi per pill, each in its own mood — a mood raised
    // while nobody was looking included.
    const tasks = State.tasks.map((t) => ({
      id: t.id, name: t.name, color: t.color, state: wanting.get(t.id) ?? t.state,
    }));
    const state = attention[0]?.state ?? State.effectiveState;
    // The popup is sized for the expanded island it is about to show, whatever
    // the island happens to be while it is shut (compact is 32 px tall).
    const view = State.view === "greeting" ? "overview" : State.view;
    const size = islandSize("expanded", view, State.chatHistory.length);
    const key = `${state}|${State.mode}|${size.w}x${size.h}|${State.paused}|${JSON.stringify(attention)}|${JSON.stringify(tasks)}`;
    if (key === last) return;
    last = key;
    Plasma.post({
      type: "state",
      payload: {
        bot: state, color: botGlowColor(state), mode: State.mode, paused: State.paused, size, attention,
        tasks: tasks.map((t) => ({ ...t, glow: botGlowColor(t.state) })),
      },
    });
  });
}
