// The page inside the KDE Plasma widget (plasma/ at the repository root).
//
// There the island is rendered by QtWebEngine, not by a Tauri webview, so:
//   * commands and events go over the app's WebSocket (src-tauri/src/plasma.rs);
//   * the widget's QML talks to the page with runJavaScript → `window.coucouHost`,
//     and the page answers with console messages prefixed `coucou-host:`, which
//     the QML side picks up in onJavaScriptConsoleMessage.
//
// The widget loads `index.html?host=panel|desktop#ws=…&token=…`. The fragment never
// reaches any server, and it is wiped from the address as soon as it is read.

import type { DragDropPayload } from "./bridge";

export type PlasmaHostMode = "panel" | "desktop";

const query = new URLSearchParams(location.search);
const fragment = new URLSearchParams(location.hash.slice(1));

const WS_URL = fragment.get("ws");
const TOKEN = fragment.get("token");

export const IS_PLASMA =
  typeof window !== "undefined" &&
  !("__TAURI_INTERNALS__" in window) &&
  query.has("host") &&
  !!WS_URL &&
  !!TOKEN;

/** Panel: a small Mochi in the panel and the island in its popup. Desktop: the island itself. */
export const HOST_MODE: PlasmaHostMode = query.get("host") === "panel" ? "panel" : "desktop";

if (IS_PLASMA) history.replaceState(null, "", location.pathname + location.search);

type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void };
type HostHandler = (payload: unknown) => void;

class PlasmaConnection {
  private ws: WebSocket | null = null;
  private open = false;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private queue: string[] = [];
  private listeners = new Map<string, Set<(payload: unknown) => void>>();
  private hostHandlers = new Map<string, Set<HostHandler>>();

  constructor() {
    if (!IS_PLASMA) return;
    (window as unknown as { coucouHost: unknown }).coucouHost = {
      receive: (msg: { type: string; payload?: unknown }) => this.fromHost(msg),
    };
    this.connect();
  }

  private connect() {
    const ws = new WebSocket(`${WS_URL}?token=${encodeURIComponent(TOKEN!)}`);
    this.ws = ws;
    ws.onopen = () => {
      this.open = true;
      for (const m of this.queue.splice(0)) ws.send(m);
      this.post({ type: "connected" });
    };
    ws.onmessage = (e) => this.onMessage(String(e.data));
    ws.onclose = () => {
      const wasOpen = this.open;
      this.open = false;
      for (const p of this.pending.values()) p.reject(new Error("Coucou is not running"));
      this.pending.clear();
      // The widget re-reads the connection file and reloads us: the app may have
      // restarted on a new port with a new token.
      this.post({ type: "disconnected", payload: { wasOpen } });
    };
  }

  private onMessage(text: string) {
    let msg: { id?: number; ok?: boolean; result?: unknown; error?: string; event?: string; payload?: unknown };
    try {
      msg = JSON.parse(text);
    } catch {
      return;
    }
    if (typeof msg.event === "string") {
      for (const fn of this.listeners.get(msg.event) ?? []) fn(msg.payload);
      return;
    }
    if (typeof msg.id !== "number") return;
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if (msg.ok) p.resolve(msg.result);
    else p.reject(new Error(msg.error ?? "failed"));
  }

  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const id = this.nextId++;
    const text = JSON.stringify({ id, cmd, args: args ?? {} });
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      if (this.open) this.ws?.send(text);
      else this.queue.push(text);
    });
  }

  listen<T>(event: string, handler: (payload: T) => void): () => void {
    const set = this.listeners.get(event) ?? new Set();
    const fn = handler as (payload: unknown) => void;
    set.add(fn);
    this.listeners.set(event, set);
    return () => set.delete(fn);
  }

  // ── Widget (QML) side ──────────────────────────────────────────────────────

  /** Tell the widget something: it reads these from the JavaScript console. */
  post(msg: { type: string; payload?: unknown }) {
    if (!IS_PLASMA) return;
    console.log(`coucou-host:${JSON.stringify(msg)}`);
  }

  /** Something the widget tells us: `open`, `close`, `drag`. */
  onHost(type: string, handler: HostHandler): () => void {
    const set = this.hostHandlers.get(type) ?? new Set();
    set.add(handler);
    this.hostHandlers.set(type, set);
    return () => set.delete(handler);
  }

  onDragDrop(handler: (e: DragDropPayload) => void): () => void {
    return this.onHost("drag", (p) => handler(p as DragDropPayload));
  }

  private fromHost(msg: { type: string; payload?: unknown }) {
    for (const fn of this.hostHandlers.get(msg.type) ?? []) fn(msg.payload);
  }
}

export const Plasma = new PlasmaConnection();
