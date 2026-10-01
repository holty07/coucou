#!/usr/bin/env bash
# Builds Coucou for Linux and installs it for the current user only:
#   ~/.local/lib/coucou/{coucou,coucou-hook}   the app and the Claude Code relay
#   ~/.local/bin/coucou                        symlink, so `coucou` starts it
#   the Plasma widget "Coucou"                 add it to a panel or the desktop
#
# Usage: plasma/install.sh [--no-build]
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
lib="$HOME/.local/lib/coucou"
bin="$HOME/.local/bin"

if [[ "${1:-}" != "--no-build" ]]; then
    (cd "$repo/windows" && npm install --no-audit --no-fund && npx tauri build --no-bundle)
fi

mkdir -p "$lib" "$bin"
install -m 755 "$repo/windows/target/release/coucou" "$lib/coucou"
install -m 755 "$repo/windows/target/release/coucou-hook" "$lib/coucou-hook"
ln -sfn "$lib/coucou" "$bin/coucou"
echo "app      → $lib (and $bin/coucou)"

# The widget: upgrade in place when it is already installed.
pkg="$repo/plasma/package"
id="$(sed -n 's/.*"Id": *"\([^"]*\)".*/\1/p' "$pkg/metadata.json")"
if kpackagetool6 -t Plasma/Applet -l 2>/dev/null | grep -qx "$id"; then
    kpackagetool6 -t Plasma/Applet -u "$pkg" >/dev/null
    echo "widget   → $id upgraded (restart plasmashell to reload a widget already on screen)"
else
    kpackagetool6 -t Plasma/Applet -i "$pkg" >/dev/null
    echo "widget   → $id installed"
fi

cat <<EOF

Next:
  1. Start the app:            coucou &
  2. Add the widget:           right-click the desktop or a panel → Add or Manage Widgets… → Coucou
  3. Hook up Claude Code:      Coucou Settings… (widget right-click menu or tray) → Claude Code → Install hooks…
  4. Start at login (optional): Coucou Settings… → Launch at login
EOF
