# 05 — Persistence and profiles

"Save" means the layout comes back after a reboot or a compositor restart. How
that happens differs by desktop, and the GUI and CLI say which way it happened
(`Persisted::ByDesktop`, `Persisted::File(path)`, `Persisted::Profile(name)`).

## Rules for files dsplr writes

- **dsplr only writes files it owns.** A managed file starts with a header
  naming dsplr and a SHA-256 of the rest of the file (`sha2`, the owner's call,
  2026-11-04); no date, which would need a date library for a value the file's
  mtime already holds (`dsplr_persist::managed`). Before overwriting, dsplr
  checks the hash: a file edited by hand since dsplr wrote it is not overwritten
  without `--force` (CLI) or a confirm (GUI) that shows the diff.
- A managed file that does not exist yet is created, and dsplr tells the user
  the one line their main config needs to include it, rather than editing the
  main config.
- Writes are atomic: write to a temp file in the same directory, `fsync`,
  rename.
- The writers in `dsplr-persist` are pure (`Layout` in, text out) and tested
  against golden files; the readers are tested round-trip.

## Hyprland

- **Lua config** (Hyprland 0.55+, detected by a `hyprland.lua`): write
  `monitors.lua` as
  `hl.monitor({ output = …, mode = …, position = …, scale = …, transform = …, vrr = …, … })`
  calls, one per output, plus a disabled entry for outputs turned off. The
  owner's `hyprland.lua` already has `require("monitors")`.
- **hyprlang config**: write `monitors.conf` with `monitor = …` lines, sourced
  from `hyprland.conf`.
- Outputs are written by **description** (`desc:AOC Q27G4 18DQ2HA004850`) when
  the serial is non-empty, so a monitor keeps its settings on another port; by
  connector name otherwise. A GUI toggle names them all by connector instead, as
  nwg-displays does (the owner's answer to [12](12-open-questions.md)'s question
  4). Hyprland matches `desc:X` when the monitor's description _starts with_ X,
  so an empty or serial-less description is never written.
- Every call states every setting dsplr owns (`disabled = false`,
  `mirror = ""`): Hyprland merges a rule onto the existing one of the same name,
  so a setting left out keeps its old value. Positions are always explicit.
  `vrr` is written only when on or automatic, so a fullscreen-only rule that
  Hyprland reports as off is never overwritten.
- The catch-all `hl.monitor({ output = "", mode = "preferred", … })` from the
  template is preserved if present.
- Importing: an existing nwg-displays-generated `monitors.lua` is read as a
  starting point and replaced with a dsplr-managed one on first save, after a
  confirm.

## wlroots compositors

| Compositor                    | Native persistence                                                                                   |
| ----------------------------- | ---------------------------------------------------------------------------------------------------- |
| sway                          | `~/.config/sway/outputs` with `output … mode … pos … scale …` lines, `include`d from the sway config |
| niri                          | a managed `outputs.kdl`, if the niri version supports `include`; else dsplr's store (checked at M4)  |
| river, labwc, Wayfire, others | dsplr's store plus `dsplr restore` from the compositor's autostart                                   |

## KDE and GNOME

The desktop saves it. `persist` on KDE is a no-op returning
`Persisted::ByDesktop` (KWin stores applied configs). On GNOME it is the
`persistent` method of `ApplyMonitorsConfig`, which writes `monitors.xml`.

## X11

No native store. dsplr saves to its own profile store and installs (on confirm)
an XDG autostart entry `~/.config/autostart/dsplr-restore.desktop` running
`dsplr restore`. Window managers that do not run XDG autostart (i3, bspwm) get
the one line to add (`exec --no-startup-id dsplr restore`) shown instead.

## dsplr's profile store

`$XDG_CONFIG_HOME/dsplr/profiles/<name>.toml`: a `Layout` plus the set of
`OutputId`s it applies to. Used for X11 and the compositors without native
persistence, and by anyone on any desktop who wants named layouts ("desk",
"laptop only", "presentation").

`dsplr restore` picks the profile whose output set exactly matches what is
connected (kanshi's rule), else the most recently saved one that covers a
subset, else does nothing and says so.

## Hotplug profiles

`dsplr watch` (stretch, M8): stays running, listens to the backend's change
stream and runs `restore` when the connected set changes. Opt-in; it is not
started by the GUI. On desktops that persist on their own (KDE, GNOME) it is
refused unless `--force`, because two things restoring layouts will fight.

Importing kanshi and autorandr profiles into the store is a stretch item.
