# Fork notes — Coucou on Linux (KDE Plasma)

A personal fork of [louis-cfm/coucou](https://github.com/louis-cfm/coucou) that runs
on Linux, with Mochi living in a **KDE Plasma 6 widget** — in a panel (a small Mochi
that opens the island in a popup) or on the desktop (the island itself).

Tested on CachyOS, Plasma 6.7, Wayland.

## How it fits together

- **The app** (`windows/`, the Tauri app, now cross-platform) runs in the background:
  Claude Code hooks, chat, integrations, tray icon, Settings window. Keys live in the
  Secret Service (KWallet).
- **The widget** (`plasma/package`) renders the same island front end in QtWebEngine
  and talks to the app over a token-protected WebSocket on 127.0.0.1
  (`windows/src-tauri/src/plasma.rs`). The connection file is
  `$XDG_RUNTIME_DIR/coucou/plasma.json` (0600).
- **coucou-hook** relays Claude Code events over `$XDG_RUNTIME_DIR/coucou/hook.sock`
  and exits immediately when the app isn't running — Claude Code is never blocked.

## What's different from upstream

- **Chat on your Claude plan.** With Claude Code installed, chat runs through
  `claude -p` (lean: no MCP, skills, plugins or hooks; web search, fetch and Read
  only), so it uses your plan instead of a billed API key. Switch in
  Settings → Claude → "Chat runs on".
- **herdr.** Sessions running in [herdr](https://herdr.dev) are tracked by pane;
  "Open in herdr" focuses the session's pane and raises its terminal (via KWin).
- The Claude Code card is called Claude Code (not VS Code); step labels are English.

## Requirements

```bash
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libayatana-appindicator rustup nodejs npm base-devel
rustup default stable
```

Plasma 6 with QtWebEngine (`qt6-webengine`) for the widget.

## Install

```bash
plasma/install.sh        # builds, installs to ~/.local/lib/coucou + ~/.local/bin/coucou, installs the widget
coucou &                 # start the app
```

Then: right-click a panel or the desktop → **Add or Manage Widgets…** → **Coucou**, and
in the widget's right-click menu → **Coucou Settings…** → Claude Code → **Install hooks…**
(shows the diff to `~/.claude/settings.json` and backs it up first). Turn on
**Launch at login** there too.

## Working on it

Where things are:

| Piece | Files |
|---|---|
| Widget bridge (HTTP + WebSocket, approval verdicts across widgets, catch-up replay) | `windows/src-tauri/src/plasma.rs` |
| Linux stand-in for the Win32 island window | `windows/src-tauri/src/island_linux.rs` |
| Hook relay over a Unix socket | `windows/src-tauri/src/pipe.rs`, `windows/hook/src/unix.rs` |
| Chat through Claude Code | `windows/src-tauri/src/claude_code.rs` |
| herdr focus + KWin raise | `windows/src-tauri/src/herdr.rs` |
| Slack poller | `windows/src-tauri/src/integrations.rs` (`poll_slack`) |
| Page side of the widget (WebSocket transport, host messages) | `windows/src/core/plasma.ts`, `windows/src/island/plasmaHost.ts` |
| The widget itself (QML) | `plasma/package/contents/ui/main.qml`, `PanelMochi.qml` |

The page talks to the widget through `console.log("coucou-host:" + JSON)` (read in
`onJavaScriptConsoleMessage`), and the widget talks to the page with
`runJavaScript("window.coucouHost.receive(…)")`. The page is told whether it is in a
panel or on the desktop by `?host=panel|desktop`.

Dev loop:

```bash
pkill -x coucou; plasma/install.sh && (setsid -f coucou >/dev/null 2>&1)
# QML changes only take effect after Plasma reloads the widget:
systemctl --user restart plasma-plasmashell.service
tail -f ~/.local/share/coucou/coucou.log          # app log (hooks, widget connects, slack…)
journalctl --user -u plasma-plasmashell -f | grep coucou   # QML errors
```

Restarting the app gives it a new port and token; the widgets notice within a few
seconds and reload their page, so front-end and Rust changes need no Plasma restart.

Debugging the page inside the real widget:

```bash
systemctl --user set-environment QTWEBENGINE_REMOTE_DEBUGGING=127.0.0.1:9334
systemctl --user restart plasma-plasmashell.service
curl -s http://127.0.0.1:9334/json        # one page per widget: ?host=panel / ?host=desktop
# …inspect over CDP (Runtime.evaluate, Page.captureScreenshot), then turn it off again:
systemctl --user unset-environment QTWEBENGINE_REMOTE_DEBUGGING
systemctl --user restart plasma-plasmashell.service
```

Gotchas learned the hard way:

- `plasmawindowed io.github.holty07.coucou` runs the widget in a window (desktop form
  factor only), but KWin keeps it behind other windows and Chromium then stops
  rendering it (`requestAnimationFrame` never fires). Raise it with a KWin script
  (see `herdr.rs` for the busctl dance) or test in the real widget.
- The popup's page is preloaded hidden at a 1× pixel ratio; anything sized by
  `devicePixelRatio` must re-check it when drawing.
- Fake events for testing: pipe hook JSON into `~/.local/share/coucou/bin/coucou-hook <Event>`.
  They reach the real widgets too.

## Remotes

```bash
git remote -v
# origin    https://github.com/holty07/coucou.git
# upstream  https://github.com/louis-cfm/coucou.git

git fetch upstream && git merge upstream/main   # pull in upstream changes
```

## Licensing reminder

Code is MIT. The Coucou/Mochi name, character, icon and sounds are not (see
[LICENSE-ASSETS.md](LICENSE-ASSETS.md)). Building and running it for personal use is fine;
publishing builds needs a different name, icon, character and sounds.
