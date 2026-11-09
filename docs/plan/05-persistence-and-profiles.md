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
- **hyprlang config** (`hyprland.conf`): not written. Hyprland 0.57 reads only
  the Lua config, so `persist` there is `Unsupported`, saying dsplr writes only
  the Lua config's `monitors.lua` (the owner's call, 2026-11-09).
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

| Compositor                    | Native persistence                                                                                  |
| ----------------------------- | --------------------------------------------------------------------------------------------------- |
| sway                          | `outputs` beside the sway config, with `output … { … }` blocks, `include outputs` from that config  |
| niri                          | a managed `outputs.kdl`, if the niri version supports `include`; else dsplr's store (checked at M4) |
| river, labwc, Wayfire, others | dsplr's store plus `dsplr restore` from the compositor's autostart                                  |

### sway

Written (`dsplr_persist::sway`), and checked on headless sway 1.12 by saving,
changing the outputs at runtime and reloading, and by starting a second sway on
the saved file.

- **How sway 1.12 picks an output's settings** (headless, 2026-11-10): on a
  reload the last setting given for an output wins, whether the command names it
  by connector, by identifier or as `*`. At startup and on hotplug a `*` setting
  still counts in file order, but an identifier's setting wins over a name's
  wherever each is (`output "Make Model Serial" scale 1.5` before
  `output DP-1 scale 2` starts at 1.5 and reloads to 2). Everything below is
  arranged so the saved block wins both ways.
- The `wlr` backend saves only when the socket `SWAYSOCK` names answers sway's
  IPC (`GET_VERSION`), so a stale `SWAYSOCK` left in a session manager's
  environment does not count; on any other wlroots compositor `persist` is
  `Unsupported` until dsplr's store exists. The socket is asked only by
  `persist` and `dsplr backend`, not at detection, so a socket that hangs delays
  only those. `Capabilities::native_persistence` says only that `SWAYSOCK` is
  set; `Backend::native_persistence` is the checked answer.
- The config is found as sway(1) says sway finds it: `~/.sway/config`,
  `$XDG_CONFIG_HOME/sway/config`, `~/.i3/config`, `$XDG_CONFIG_HOME/i3/config`
  (`XDG_CONFIG_HOME` defaulting to `~/.config`); not `sway --config` nor
  `/etc/sway/config`. `outputs` goes **beside the config found**, so in the
  suggested location it is `~/.config/sway/outputs`, and the line to add is
  always `include outputs`: sway reads a relative include from the including
  config's directory (the link's own directory when the config is a link), and a
  quoted absolute path with a space in it was not loaded at all.
- One block per output: `disable` alone for one that is off, else `enable`,
  `mode` (`--custom` for a mode its output is known not to list, the refresh
  only when known), `position`, `scale`, `transform` and `adaptive_sync` when
  known. sway keeps a setting the config leaves out as it was at runtime, so
  every setting is stated.
- `transform` is written in sway's names, which turn the quarter turns the other
  way from `wl_output`'s, flipped ones too: sway's `90` is wl_output's `270`,
  its `flipped-90` wl_output's `flipped-270`.
- A monitor with a serial is named by sway's identifier, `"Make Model Serial"`
  with `Unknown` for a part it does not report, compared whole; one without, one
  sharing its identifier with another connected monitor, or one whose identifier
  holds a quote, a backslash or a `$` (which sway reads its own way), by
  connector. A monitor without a serial still has an identifier to sway
  (`Make Model Unknown`), so the snapshot keeps every monitor's make, model and
  serial (`Output`), not only those `OutputId` keeps.
- A mirror, a primary output, colour settings and automatic adaptive sync are
  refused before anything is written (exit `3`): sway has no command for them.
- A file in dsplr's own form (only `output` commands setting what dsplr writes,
  as blocks or on one line, with comments) keeps its commands for outputs not
  connected now, `*` included, written first so the saved blocks win on a
  reload. A command naming a connected output by its connector **or** its
  identifier is dropped, whichever name the new block uses, since at startup an
  identifier's command would beat a saved block by connector. A file with
  anything else (`bg`, `power`, `scale_filter`, `set $var`) cannot be carried
  over and takes `--force`, as on Hyprland.
- After saving, dsplr says when the main config's own `output` commands win over
  the saved file, each case with its own fix: one after the `include` setting a
  connected output (by connector, identifier or `*`) wins on a reload, and
  moving the `include` below it fixes that; one anywhere naming a connected
  output by its identifier, when the file names it by connector, wins at startup
  wherever the `include` is, so the fix is to remove it or name the output by
  connector there. The second is looked for even when the config does not
  include the file yet.

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
