// Glue between the island and the KDE Plasma widget that hosts it.
//
// Desktop: the widget *is* the island's window — Mochi sits on the desktop in
// the compact island and never retracts on its own.
// Panel: a small Mochi lives in the panel (drawn by the widget) and the island
// lives in its popup. The popup opens whenever the island would expand, and
// closing it puts the island back to compact.

import { Bridge } from "../core/bridge";
import { botGlowColor, islandSize } from "../core/layout";
import { HOST_MODE, Plasma } from "../core/plasma";
import { State } from "../core/state";
import type { Island } from "./island";

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

  if (HOST_MODE === "desktop") {
    island.fsm.neverHide = true;
  } else {
    const inner = island.fsm.onTransition;
    island.fsm.onTransition = (from, to) => {
      inner?.(from, to);
      if (to === "home") Plasma.post({ type: "expand" });
      else if (to === "petit" || to === "hidden") Plasma.post({ type: "collapse" });
    };
    Plasma.onHost("open", () => {
      if (island.fsm.state !== "home") island.alert(State.defaultView());
    });
    Plasma.onHost("close", () => {
      if (island.fsm.state === "home") island.fsm.forcePetit();
    });
  }

  // What the widget needs to draw the panel Mochi and size its popup.
  let last = "";
  State.subscribe(() => {
    const state = State.effectiveState;
    const size = islandSize(State.mode, State.view, State.chatHistory.length);
    const key = `${state}|${State.mode}|${size.w}x${size.h}|${State.paused}`;
    if (key === last) return;
    last = key;
    Plasma.post({
      type: "state",
      payload: { bot: state, color: botGlowColor(state), mode: State.mode, paused: State.paused, size },
    });
  });
}
