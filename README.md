# dsplr

A display settings editor for Linux: an nwg-displays replacement. Rust binary.

[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Drag monitors into place, set their mode, refresh rate, scale, rotation and
adaptive sync, apply with a keep-or-revert timer, and save. One tool on
Hyprland, sway, niri, river, labwc, Wayfire, KDE Plasma, GNOME and X11 window
managers. Drawn on the GPU through [crcbl](https://github.com/kryptic-sh/crcbl).
Part of the [kryptic.sh](https://kryptic.sh) suite.

## Status

**Works, not yet released.** Running today:

- **Backends**: Hyprland, KDE Plasma 6 on Wayland, GNOME, wlroots compositors
  (sway and niri tested; river, labwc and Wayfire expected to work through the
  same protocol), and X11 through RandR (tested on Xvfb, and with several
  outputs on Xorg's dummy driver). Each reads, applies and follows its outputs,
  and saves where the desktop reads them: Hyprland's `monitors.lua`, sway's
  `outputs`, niri's `outputs.kdl`; KDE keeps each layout it applies, GNOME saves
  through its own keep-or-revert dialog, and on X11 the layout is saved as a
  profile that dsplr puts back at login.
- **GUI** (`dsplr`): a canvas to drag outputs on, a panel for each output's
  settings, keys for everything, undo, a confirm dialog on every monitor that
  reverts on its own, identify, profiles, and live updates when the desktop
  changes.
- **CLI**: `list`, `backend`, `set`, `apply`, `save`, `restore` (and
  `--revert`), `profile list|show|rm`, `profile import kanshi`, `identify`, and
  `watch`, which restores the matching profile whenever monitors are plugged in.

Open: the owner's check on an X11 window manager (i3) and the first release
(M9), with packages and install instructions. The plan is in
[`docs/plan/`](docs/plan/00-index.md); what is open is in
[`docs/backlog.md`](docs/backlog.md).

## License

MIT
