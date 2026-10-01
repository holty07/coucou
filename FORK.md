# Fork notes

This is a personal fork of [louis-cfm/coucou](https://github.com/louis-cfm/coucou).

## Goal

Get the Tauri app (`windows/`) building and running on Linux (CachyOS / Arch, Wayland + X11).

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
