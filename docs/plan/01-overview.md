# 01 — Overview

## What dsplr is

`dsplr` (read "displayer") is a display settings editor for Linux. It shows
every connected monitor as a rectangle on a canvas, lets the user drag them into
place and set each one's mode, refresh rate, scale, rotation, adaptive sync and
on/off state, applies the result live with a confirm-or-revert timer, and saves
it so it survives a restart.

It is one binary with two front ends over the same core:

- **GUI** — `dsplr` with no arguments opens a window drawn with crcbl (shell,
  HAL and UI tree), the same stack as pikr and lyr.
- **CLI** — `dsplr list`, `dsplr set`, `dsplr apply`, `dsplr restore`, … for
  scripts, keybinds and machines without a GUI session ([07](07-cli.md)).

## Why

nwg-displays is the tool in use today (it generated the owner's
`~/.config/hypr/monitors.lua`). It is Hyprland- and sway-only, Python and GTK,
and its output file is a whole-file overwrite marked "Do not edit manually".
dsplr wants:

- one tool on every desktop the owner and kryptic-sh users run, with the same
  keys and look;
- the kryptic-sh stack (Rust, crcbl, Tokyo Night, vim keys) instead of GTK;
- the safety net desktop settings panels have and nwg-displays lacks: a change
  that blanks the screen reverts on its own.

## Baseline

The feature baseline is **nwg-displays** for the editing surface and **GNOME /
KDE display settings** for the safety behaviour. Anything not spelled out below
takes its cue from those.

From nwg-displays: the drag canvas with edge snapping, per-output mode / refresh
/ scale / transform / adaptive sync / enable, custom modes, mirroring, and
writing the compositor's own config file.

From the desktop panels: apply now and keep-or-revert within a timeout; refuse a
layout with no enabled output; show which physical screen is which.

## MVP scope

The MVP is the daily-driver replacement for nwg-displays on the owner's Hyprland
machine, **plus** working backends for the other desktops. In scope:

1. **Read the current state** on every supported desktop: outputs, their make /
   model / serial, available modes, current mode, position, scale, transform,
   adaptive sync and enabled state ([03](03-display-model.md),
   [04](04-backends.md)).
2. **Edit and apply live** on every supported desktop, through the desktop's own
   interface.
3. **Confirm or revert.** Every apply that can blank or misplace a screen opens
   a confirm prompt; no answer within the timeout restores the previous state
   ([02](02-architecture.md#apply-confirm-revert)).
4. **Persist** on every supported desktop, natively where the desktop has a way
   and through dsplr's own store plus `dsplr restore` where it does not
   ([05](05-persistence-and-profiles.md)).
5. **GUI** on crcbl: canvas, property panel, identify overlay, confirm dialog,
   hjkl keys as well as the mouse ([06](06-gui.md)).
6. **CLI** with JSON output ([07](07-cli.md)).
7. **Desktops:** Hyprland, the wlroots family through `wlr-output-management`
   (sway, niri, river, labwc, Wayfire), KDE Plasma 6, GNOME, and X11 window
   managers through XRandR (i3, bspwm, awesome, Openbox, Xfce's xfwm4)
   ([08](08-platforms.md)). The owner put X11 in the MVP (2026-11-04).

### Stretch (MVP if it fits)

- **Hyprland workspace rules**: assign workspaces to monitors, as nwg-displays'
  second window does, written to `workspaces.lua`.
- **Mirroring on Hyprland.** GNOME and X11 mirroring are in the MVP (M6, M7),
  and KDE's is built (M5, from KWin's management version 13); Hyprland's depends
  on the M2 check of `eval` against protocol applies ([04](04-backends.md)).

## Non-goals (for now)

- **Windows and macOS.** Both ship display settings that cover this. Declined,
  not deferred: recorded in the backlog.
- **Colour calibration / ICC profiles, brightness and gamma.** Different tools
  (`colord`, `wl-gammarelay`, `ddcutil`). HDR and colour-management presets are
  shown read-only where the backend reports them and are an open question for
  later ([12](12-open-questions.md)).
- **Mutating the layout of a desktop dsplr cannot persist on** without saying
  so. A backend that can apply but not save says that in the GUI and CLI rather
  than appearing to save.
- **Being a session daemon by default.** Hotplug profiles are opt-in
  (`dsplr watch`, [05](05-persistence-and-profiles.md#hotplug-profiles)).

## Decisions taken in this plan

- **Linux only**, Wayland and X11. crcbl's shell covers both, so one GUI serves
  every desktop in scope; wayr was not used because it is Wayland-only by
  construction (owner's call, 2026-11-04: "crcbl for the UI, just like pikr and
  lyr").
- **Each desktop through its own interface**, never by shelling out to another
  tool's CLI (`wlr-randr`, `kscreen-doctor`, `xrandr`, `gnome-randr`). Those
  tools are useful as a cross-check in tests ([09](09-testing.md)) and in the
  prior-art table ([02](02-architecture.md#prior-art)).
- **Hyprland is driven through `wlr-output-management` for live changes and its
  IPC (`hyprctl eval` over the socket) for what that protocol cannot express**,
  with persistence written to `monitors.lua`. See [04](04-backends.md#hyprland).
- **Managed files are dsplr's alone.** dsplr writes a file the user's config
  includes (`monitors.lua`, a sway `include`, …) and never edits a
  hand-maintained config in place ([05](05-persistence-and-profiles.md)).
- **No polling.** State changes are pushed by every backend that can push
  (Wayland events, D-Bus signals, RandR notify events).

## Naming

`dsplr` follows the house pattern (`pikr`, `tmxr`, `gpur`, `lyr`): the word with
its vowels dropped and an `-r` ending. Binary `dsplr`, crates `dsplr-*`, config
under `$XDG_CONFIG_HOME/dsplr/`, state under `$XDG_STATE_HOME/dsplr/`. Checked
free on crates.io and in the `kryptic-sh` org on 2026-11-04.
