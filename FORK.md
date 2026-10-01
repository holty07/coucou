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
