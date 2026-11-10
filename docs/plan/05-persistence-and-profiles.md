# 05 — Persistence and profiles

"Save" means the layout comes back after a reboot or a compositor restart. How
that happens differs by desktop, and the GUI and CLI say which way it happened
(`Persisted::ByDesktop`, `Persisted::WhenKept`, `Persisted::File(path)`,
`Persisted::Profile(name)`).

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
  `vrr` is written only when the layout knows it (`0`, `1` or `2`): a snapshot
  reads it as not known, since Hyprland reports whether adaptive sync is active
  rather than its rule, so a fullscreen-only rule is never overwritten by a
  save. `cm` and `bitdepth` are written only when not Hyprland's default
  (`srgb`, 8): the file is read afresh at startup, where a rule naming neither
  gets the defaults. Except for an output a `hyprland.lua` `hl.monitor` call
  could name (read line by line; one whose output is not a plain string counts
  as naming every output): the saved rule merges onto that one, so the defaults
  are stated there. That is the preset and depth in force, as the snapshot reads
  them, so a rule the monitor cannot do (`cm = "hdr"` falling back to `srgb`,
  `bitdepth = 10` driven at 8) is not kept by a save (backlog).
- The catch-all `hl.monitor({ output = "", mode = "preferred", … })` from the
  template is preserved if present: every such call of a file dsplr can read
  (its own, nwg-displays', or one approved after a conflict), as its text and in
  its order, since Hyprland merges same-name rules field by field and an earlier
  call's fields count where a later one leaves them out. They go first in the
  saved file. dsplr never writes one of its own, and says when it kept one
  (`Saved::kept_catch_all`). A file dsplr cannot read carries nothing over,
  catch-alls included, after `--force`. nwg-displays' `monitors.lua` (the
  fixture in `dsplr-persist`) has none. This is the fallback rule plan 03 keeps
  out of `Layout` ([12](12-open-questions.md), question 11).
- Importing: an existing nwg-displays-generated `monitors.lua` is read as a
  starting point and replaced with a dsplr-managed one on first save, after a
  confirm.

## wlroots compositors

| Compositor                    | Native persistence                                                                                                                        |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| sway                          | `outputs` beside the sway config, with `output … { … }` blocks, `include outputs` from that config                                        |
| niri                          | `outputs.kdl` beside niri's config, with `output "…" { … }` nodes, `include "outputs.kdl"` from that config before its own `output` nodes |
| river, labwc, Wayfire, others | dsplr's store plus `dsplr restore --yes` from the compositor's autostart                                                                  |

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
  IPC (`GET_VERSION`) from the process serving the Wayland socket (their
  `SO_PEERCRED` pids), so a stale `SWAYSOCK` left in a session manager's
  environment, or one a compositor nested in sway inherited, does not count; on
  any wlroots compositor other than sway and niri ([below](#niri)) `persist` is
  `Unsupported` (exit `3`), and its message points at `dsplr save --profile` and
  `dsplr restore --yes` from the compositor's autostart. The socket is asked
  only by `persist` and `dsplr backend`, not at detection, so a socket that
  hangs delays only those. `Capabilities::native_persistence` says only that
  `SWAYSOCK` is set; `Backend::native_persistence` is the checked answer.
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
  reload. dsplr says when it kept `*` commands, and never writes one of its own
  (plan 03's fallback rule; [12](12-open-questions.md), question 11). A command
  naming a connected output by its connector **or** its identifier is dropped,
  whichever name the new block uses, since at startup an identifier's command
  would beat a saved block by connector. A file with anything else (`bg`,
  `power`, `scale_filter`, `set $var`) cannot be carried over and takes
  `--force`, as on Hyprland.
- After saving, dsplr says when the main config's own `output` commands win over
  the saved file, each case with its own fix: one after the `include` setting a
  connected output (by connector, identifier or `*`) wins on a reload, and
  moving the `include` below it fixes that; one anywhere naming a connected
  output by its identifier, when the file names it by connector, wins at startup
  wherever the `include` is, so the fix is to remove it or name the output by
  connector there. The second is looked for even when the config does not
  include the file yet.

### niri

Written (`dsplr_persist::niri`), and checked on niri 26.04 nested (winit) in a
headless sway 1.12 (2026-11-10, `tests/e2e/headless-niri.sh`): by saving and
watching niri apply the file, changing the output at runtime and starting a
second niri on the saved file, and by `niri validate` on what was written.

- **`include` works** in niri 26.04: `niri validate -c` accepts a config with
  `include "outputs.kdl"` and reads the included `output` nodes, rejects a
  missing file unless the include has `optional=true`, and rejects an invalid
  node in the included file as an error of the whole config. A relative path is
  read from the including file's directory. niri watches included files too: an
  `outputs.kdl` written after niri started, behind an `optional=true` include,
  was applied within two seconds.
- **How niri 26.04 picks an output's settings** (nested, 2026-11-10): the
  **first** `output` node naming an output wins, whole; a later node for it, in
  the same file or the main config, is not merged in (a later `position` did not
  apply on top of an earlier `scale`). So the line to add is
  `include "outputs.kdl"` **before** the config's own `output` nodes, and dsplr
  says when one of those for a connected output comes before the include
  (`Saved::overridden`, `Saved::first_wins`). A node after it is hidden whole,
  so its other settings stop applying for that output: when such a node, for a
  connected output, sets anything dsplr does not write (`focus-at-startup`,
  `backdrop-color`, `variable-refresh-rate on-demand=true`), the save is a
  `Conflict` listing them (`Conflict::shadowed`) that takes a yes, file or no
  file (`Ownership::Absent`). With no include yet every such node counts, since
  the include goes before them. Seen nested: with the saved node present, a
  scale written there showed over the config's own node's.
- A setting a node leaves out takes niri's default, not its runtime value: with
  no node an output went back to scale 1 at the origin. Every setting is stated:
  `off` alone, or `mode`, `scale`, `transform`, `position`, and
  `variable-refresh-rate` when on (`on-demand=true` when automatic); off is
  niri's default and writes nothing. wlr-output-management reads adaptive sync
  only as on or off, so an output whose old saved node had `on-demand=true`
  keeps it when it reads as on.
- **Writing the file applies it**: niri reloads its config whenever a file of it
  changes, a rename over it included. A runtime change made through
  wlr-output-management (`dsplr apply`, `dsplr set`) survives a reload that
  leaves its output's node as it was, and is replaced when the node changes. So
  `dsplr save`, which writes the layout on screen, moves nothing; its runtime
  changes become the config's. With the confirm flow: `dsplr save` run while
  another `dsplr apply` waits for its confirm saves the unconfirmed layout, and
  its revert then goes back at runtime only, the file keeping the unconfirmed
  layout until it is saved again, as on sway; on niri the file is also what any
  later change to that output's node reloads.
- niri has no rule for outputs a config does not name: a node names one output,
  whole (below), so there is no catch-all to keep or write.
- Names: niri matches a node by connector or by `"Make Model Serial"` (`Unknown`
  for a part not reported, none at all when all three are missing), each whole
  and ignoring ASCII case (`"smithay winit unknown"` named the nested `winit`
  output; `"Smithay Winit"` did not). A monitor with a serial is written by
  identifier, others, and monitors whose identifiers differ only in case, by
  connector. Names are KDL 1 strings, always quoted (`output winit {` is
  refused), with `"` and `\` escaped.
- Transforms keep wl_output's names, which are dsplr's: niri's `90` is
  `WL_OUTPUT_TRANSFORM_90` (`src/utils/mod.rs` maps each config name to the
  smithay transform of the same name, v26.04 source). The nested `winit` output
  ignores a transform, so this was not read back.
- Values niri rejects are never written, since an included file that does not
  parse makes niri drop the whole config: a nested niri started on a config
  including one with `scale 11` came up at its defaults, ignoring a valid
  include beside it. So: no scale above 10, no mode side over 65535, and no
  custom mode without its refresh (niri requires one).
- The config is found as niri finds it: `NIRI_CONFIG` when set and not empty
  (absolute only, else the save is refused), else `niri/config.kdl` in
  `$XDG_CONFIG_HOME` or `~/.config`. Not `niri --config`, nor
  `/etc/niri/config.kdl` (niri creates the user's config at startup when there
  is none, so a running niri normally has one).
- It is niri when the socket `NIRI_SOCKET` names answers niri's `Version`
  request (one JSON line each way), asked only by `persist` and `dsplr backend`.
  niri sets `NIRI_SOCKET` and `WAYLAND_DISPLAY` for what it starts (seen with a
  startup command, `niri -- sh -c …`, printing them; the harness learns niri's
  sockets that way). It counts only when the process listening on `NIRI_SOCKET`
  also listens on the Wayland socket (`SO_PEERCRED`), so a compositor nested in
  niri, which inherits `NIRI_SOCKET`, is not taken for it; checked in the nested
  harness both ways round (sway's socket named under niri, niri's under sway).
- **Saving applies, so only the layout on screen is saved**: `Backend::persist`
  on niri reads the outputs again and refuses (`Unsupported`, exit `3`) a layout
  that differs from them (`Layout::differences`, the apply flow's tolerance), so
  a save never changes the screen past `dsplr apply`'s test, revert point and
  confirm.
- The old file is read in dsplr's form (`output` nodes holding only those
  settings, each node starting and ending its line, `//` comments); its nodes
  for outputs not connected are kept, written first so a monitor saved by its
  identifier keeps its settings when later plugged into a port the file names,
  and its nodes naming a connected output by connector or identifier, in any
  case, are dropped. Anything else needs `--force`.
- A mirror, a primary output and colour settings are refused (exit `3`): niri's
  output config has none (`mirror` is refused by `niri validate`).

## KDE and GNOME

The desktop saves it. `persist` on KDE writes nothing and returns
`Persisted::ByDesktop`: KWin writes every configuration it applies to
`kwinoutputconfig.json` ([04](04-backends.md#kde-plasma-6-kde)). Since KWin has
saved only what it applied, a layout other than the one on screen is
`Unsupported`, as on niri. On GNOME it is the `persistent` method of
`ApplyMonitorsConfig`, which writes `monitors.xml`.

### GNOME

Built at M6 (`Backend::persist`, `gnome::Gnome::persist`), on what Mutter 51.1
does ([04](04-backends.md#gnome-gnome)):

- **Apply is `temporary`, Save is `persistent`.** `dsplr apply` and the GUI's
  Apply send `temporary`, which Mutter neither saves nor reverts, and run
  dsplr's own flow: test (`verify`), revert point and watchdog, apply, read
  back, confirm. So on GNOME, as elsewhere, an apply not kept is reverted by
  dsplr, and one kept is lost at the next login unless saved.
- **Save sends the layout on screen again as `persistent`**, so nothing on
  screen changes, and the confirm is GNOME Shell's "keep these display
  settings?" dialog, not a second one of dsplr's: Mutter writes `monitors.xml`
  only when it is kept, and on a no or its timeout applies the configuration
  from before the `persistent` one, which is the same layout dsplr applied and
  the user kept, so the screen still does not change; only the save is lost.
  `persist` returns as soon as Mutter took the request, before anyone has
  answered, so it reports `Persisted::WhenKept` ("handed to Mutter, which writes
  monitors.xml once … kept"), not that it was saved. Without GNOME Shell (plain
  `mutter`, the e2e harness) Mutter keeps it at once.
- As on niri and KDE, a layout other than the one on screen is `Unsupported`
  (exit `3`): saving applies, so a save never changes the screen past dsplr's
  test, revert point and confirm.
- Any `temporary` or `persistent` apply cancels a confirmation of Mutter's that
  is still waiting, so applying with dsplr while GNOME Shell's dialog is up
  drops that save (its dialog's answer then does nothing). Not seen, read in
  `meta_monitor_manager_handle_apply_monitors_config`.
- dsplr never writes `monitors.xml` itself.

## X11

No native store. dsplr saves to its own profile store and installs (on confirm)
an XDG autostart entry `~/.config/autostart/dsplr-restore.desktop` running
`dsplr restore --yes`. Window managers that do not run XDG autostart (i3, bspwm)
get the one line to add (`exec --no-startup-id dsplr restore --yes`) shown
instead. Not written yet (M7).

## dsplr's profile store

Written (`dsplr_persist::profile`), and checked on headless sway 1.12 by saving
a profile, moving an output and restoring it, by name and picked.

- `$XDG_CONFIG_HOME/dsplr/profiles/<name>.json`: the layout in the JSON form
  `dsplr list --json` prints, with `saved_unix_ms` (milliseconds since the Unix
  epoch) beside its fields, so `dsplr apply` reads a profile too. A layout file
  without it (`dsplr list --json > …/desk.json`, or one written by hand) is a
  profile saved at its file's modification time. JSON, not the TOML planned
  first: no dependency dsplr has writes TOML (`hjkl-config` only reads it), and
  adding `toml` is the owner's call (`docs/backlog.md`).
- The outputs a profile applies to are its entries' `OutputId`s: saved from the
  displays, it names every connected output, on or off.
- A profile saved on one desktop applies on another: a desktop-specific setting
  the backend lacks (`Capabilities`) is dropped where its value is the neutral
  one (`dsplr_model::NEUTRAL_COLOUR_PRESET` `srgb`, `NEUTRAL_BIT_DEPTH` 8,
  `NEUTRAL_OVERSCAN` 0, `NEUTRAL_RGB_RANGE` automatic, `NEUTRAL_ADAPTIVE_SYNC`
  off), which is what a Hyprland, KDE or wlroots snapshot says when nothing
  special is set (`Layout::drop_neutral_unsupported`, in `apply::prepared` for
  every apply, restore and the GUI). Any other value (a `wide` preset, 5 %
  overscan on sway) is a request the desktop cannot honour, and is refused (exit
  `3`). Checked on headless sway 1.12 with a profile holding all four.
- A name is ASCII letters and digits, spaces, `-`, `_` and `.`, starts with a
  letter, a digit or `_`, does not end in a space or `.`, has no `..`, and is at
  most 200 bytes, so it is a file name in the store and nothing else; anything
  else is a usage error (exit `2`), and a `.json` file in the store whose name
  is not one is reported and passed over. A name that names no profile is one
  too, as an output name that names no output is.
- `dsplr save --profile NAME` saves the current snapshot on any desktop,
  alongside native saving rather than in its place. Replacing a profile of that
  name shows what changes (as `dsplr list` text) and takes `--yes`, `--force` or
  a yes on the terminal; one that cannot be read takes `--force`. The write is
  atomic, and a symbolic link at the profile's path is refused, as for every
  file dsplr writes; reading through one is fine.
- `dsplr restore` picks the profile whose outputs are exactly the connected ones
  (kanshi's rule), else, as dsplr's own fallback (kanshi matches exactly or not
  at all), the most recently saved one whose outputs are all connected (others
  stay as they are), the earlier name winning a tie; with none it changes
  nothing, says so and exits `7`. An output matches as `dsplr apply` matches one
  (`OutputId::matches`): a monitor with a serial on any port, one without on its
  connector. Each profile output takes its own connected output, paired by a
  maximum bipartite matching shared with `dsplr apply` (`dsplr_model::assign`),
  so an identity naming several outputs (a description, twins with one EDID
  identity) takes whichever is left; where several pairings exist, the closest
  matches are placed first (EDID on its saved connector, then EDID or connector,
  then a whole description, then a description's first words), ties in the order
  listed. An empty profile is never picked. A profile file that cannot be read
  is reported and passed over.
- `dsplr restore NAME` applies that profile, refused (exit `3`) when one of its
  outputs is not connected or every output it names is another's. Either way the
  layout goes through `dsplr apply`'s flow: validate, test, revert point and
  watchdog, apply, confirm.
- **Unattended**: run from an autostart there is no terminal, and a change that
  can hide or move a screen is not applied without `--yes` (exit `1`, nothing
  changed), as for `dsplr apply`. So autostart lines run `dsplr restore --yes`,
  which keeps the change without asking and still writes the revert point for
  `dsplr restore --revert`. A profile is a layout that was on screen when it was
  saved, so keeping it unseen is the expected case; one that no longer suits the
  connected outputs is either not picked or refused before anything is sent, and
  a restore that changes nothing needs no `--yes` at all.
- `dsplr profile list [--json]` (name, outputs, how long ago it was saved),
  `dsplr profile show NAME [--json]` (the layout as `dsplr list` prints it, or
  its JSON), `dsplr profile rm NAME`.

## Hotplug profiles

`dsplr watch` (M8) is written, and checked on headless sway 1.12 (2026-11-10) by
plugging outputs in and unplugging them (`swaymsg create_output`,
`swaymsg output NAME unplug`) under it. Opt-in; it is not started by the GUI.

- **What it does**: stays running, follows the backend's change stream
  (`Backend::watch`), and each time the set of connected outputs changes,
  restores the profile `dsplr restore` would pick for them, by the same rules
  (`dsplr_persist::profile::pick`: an exact fit, else the most recent subset,
  through `profile::picked` in `apps/dsplr`). The outputs connected when it
  starts count as a change, so `exec dsplr watch` replaces `dsplr restore --yes`
  in an autostart. Profiles are read afresh each time, so one saved while it
  runs is picked next time. No profile for the outputs is said, and nothing
  changes.
- **Only the connected set**: a change is the set of `(connector, OutputId)`
  pairs (`watch::Connected`), so a mode, position, scale, on/off or mirror
  change does nothing, a monitor moved to another port is a change, and so is an
  unplug and a plug. That is what keeps it from answering its own apply, or
  undoing a change the user made by hand. A second guard: a picked profile
  already on screen (no differences once paired) is said and not applied, so a
  watch restarted, or a desktop that put the layout back itself, applies
  nothing.
- **Debounced**: the displays are read once the changes have been quiet for
  `watch::QUIET`, so a dock attaching several outputs one after another is one
  restore, not one per output; but no later than `watch::QUIET_MAX` after the
  first change not yet read, so a desktop that never stops reporting changes is
  still read.
- **Not over another change**: while another change waits to be kept (a
  `dsplr apply` or `dsplr set` at its prompt, the GUI's confirm dialog, or that
  change's watchdog reverting it), a restore would replace its revert point and
  disarm its watchdog. So the revert point says while its change waits: written
  with the time its watchdog reverts (`RevertPoint::guarded_until_unix_ms`, the
  `revert_guarded_until_unix_ms` field in `revert.json`), and rewritten without
  it once kept. The watch treats such a point as busy (`watch::busy`) until that
  time plus the longest the watchdog's own revert takes
  (`restore::change_limit`): it says so, leaves the new outputs unhandled, and
  looks again every `watch::BUSY_RETRY`, restoring once the change is kept or
  reverted. A revert point from before this field reads as kept. Only the watch
  waits: a `dsplr apply` started during a watch's restore replaces the watch's
  revert point, and the watch's keep then says the change was replaced, as for
  any two changes at once.
- **Unattended, so kept without asking**: a restore runs `dsplr apply`'s flow as
  `dsplr restore --yes` does: validate, test, the revert point and its watchdog,
  apply, re-read, keep. There is no one to ask, and kanshi keeps its profiles
  the same way; a profile is a layout that was on screen when it was saved, one
  that does not suit the outputs is not picked or is refused before anything is
  sent, and `dsplr restore --revert` goes back from the revert point. A restore
  that fails is said and not retried until the outputs change again. Whether a
  hotplug restore should instead ask, with a confirm window on the desktop, is
  the owner's call (`docs/backlog.md`).
- **KDE and GNOME**: refused (exit `3`) unless `--force`, because KWin and
  Mutter restore their own saved layout per set of outputs, and two things
  restoring layouts will fight (`Backend::restores_on_hotplug`).
- **A dropped watch** (the compositor restarted, or its socket closed) is
  reconnected: the backend is detected again and a new watch opened, waiting
  `watch::Backoff`'s doubling delays (`BACKOFF_FIRST` up to `BACKOFF_MAX`)
  between tries, and giving up (exit `1`) after `RECONNECT_TRIES` failed tries
  in a row. The tries start afresh only after a watch that worked: one that
  reported a change, or stayed up for `STAYED_UP`; a watch that opens and ends
  at once keeps counting, so it is given up on rather than reopened forever.
  Once reconnected the outputs are checked as at the start, and the KDE and
  GNOME refusal is checked again, as the desktop may be another one. Checked by
  hand on headless sway (killed and started again under a running watch,
  2026-11-10); a Hyprland restart gets a new instance signature, which the
  watch's environment does not have (`docs/backlog.md`). If the desktop cannot
  be reached when it starts, it exits at once (`6`, or `1` when no watch can be
  opened).
- **Stopping**: SIGINT or SIGTERM ends it with exit `0`. One that arrives during
  a restore is taken once the restore has finished, so a kept change never
  leaves its revert point to the watchdog. The sway e2e sends SIGTERM during a
  restore whose settle runs to its end (adaptive sync on a headless output never
  reads back as asked), and checks the watch is still running a second later,
  then exits `0` with the change kept.
- **Logs** go to stderr, one `dsplr:` line each, as every command's do: the
  compositor puts its children's stderr in its own log or the journal, and an
  autostart line can redirect it. Plan 02's `tracing` logs in
  `$XDG_STATE_HOME/dsplr/logs/` are not added yet (`docs/backlog.md`).

Starting it with the session, one line per compositor (no installer):

| Desktop               | Line                                                                                    |
| --------------------- | --------------------------------------------------------------------------------------- |
| Hyprland (Lua config) | `hl.on("hyprland.start", function () hl.exec_cmd("dsplr watch") end)` in `hyprland.lua` |
| Hyprland (hyprlang)   | `exec-once = dsplr watch` in `hyprland.conf`                                            |
| sway                  | `exec dsplr watch` in sway's config                                                     |
| niri                  | `spawn-at-startup "dsplr" "watch"` in `config.kdl`                                      |
| river                 | `dsplr watch &` in `init`                                                               |
| labwc                 | `dsplr watch &` in `autostart`                                                          |
| Wayfire               | `watch = dsplr watch` under `[autostart]` in `wayfire.ini`                              |
| An XDG autostart      | a `~/.config/autostart/dsplr-watch.desktop` with `Exec=dsplr watch`                     |
| KDE Plasma, GNOME     | not needed: the desktop restores its own layouts (`dsplr watch --force` if not)         |

The Hyprland Lua line follows the autostart example in Hyprland 0.57.2's default
config (read in its binary, 2026-11-10); the sway line is what the e2e harness's
sway runs commands with; the others are each compositor's usual autostart, not
run here.

## Importing kanshi profiles

`dsplr profile import kanshi [PATH] [--force]` (M8, `dsplr_persist::kanshi`)
reads kanshi's config as kanshi 1.9.0 does: its own lookup
(`$XDG_CONFIG_HOME/kanshi/config`, else `~/.config/kanshi/config`) when no path
is given, scfg as libscfg parses it, top-level output defaults and `$aliases`
resolved into the profiles, `include` followed. A config kanshi would refuse is
refused whole, with the file and line. A relative `include` is taken against
`$HOME`, kanshi's usual working directory (it never changes its own, so it is
whatever started it, usually the compositor), and said.

- Each profile becomes a dsplr profile of the same name; one whose name is not a
  profile name gets one made from it, an anonymous one `kanshi-N` (kanshi's own
  number for it), each said. A name taken by an earlier profile, or already in
  the store without `--force`, is not written.
- A connector criteria is a connector. `make model serial` is the EDID identity
  of the connected monitor whose own `make model serial` it is, when it has a
  serial, else a description matched by whole leading words, a trailing `*` or
  `Unknown` dropped (said).
- What kanshi leaves to the output (a setting not given, a mode without a
  refresh, `mode preferred`) is taken from the output as it is now, when it is
  connected: the highest refresh listed at that size, as kanshi picks, its
  preferred mode, its current settings. When it is not connected, dsplr's
  defaults stand in, except for an enabled output's mode, which skips the
  profile. Each choice is said.
- A profile dsplr cannot hold is skipped, with why: `output *`, `...output`, a
  description glob other than a trailing `*`, an all-`Unknown` description.
  `exec` is not run, and said.
- Exit `0` when every kanshi profile was imported, `1` when one was skipped or
  not written.

Importing autorandr's profiles waits for M7: they are X11 configs, outputs named
by RandR's names and monitors fingerprinted by raw EDID, which dsplr's
identities need the EDID parser and the `Randr` backend for (`docs/backlog.md`).
