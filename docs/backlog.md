# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the project stands (2026-11-09)

M0 (scaffold) and M1 (read Hyprland, `dsplr list`, `dsplr backend`) are done. On
the owner's machine (Hyprland 0.56.2) `dsplr list --json` was compared field by
field with `hyprctl -j monitors all` and matched. Detection knows Hyprland and
wlroots compositors (`wlr`, over `zwlr_output_manager_v1`, which reads, tests
and applies; its e2e runs on headless sway). Build order is
`docs/plan/11-milestones.md`.

M2 is under way. `Backend::apply` sets a whole layout on Hyprland in one `eval`
(`Hyprland::apply`, built on `dsplr_persist::hyprland::monitor_calls`), and the
headless e2e test applies edited snapshots and reads them back.
`dsplr apply <FILE>` runs the confirm flow (`docs/plan/02-architecture.md`,
steps 1 to 6): validate, revert point in `$XDG_STATE_HOME/dsplr/revert.json`, a
detached watchdog (`dsplr restore --revert --after <secs> --token <t>`), apply,
re-read until the layout settles, ask, then keep or revert;
`dsplr restore --revert` goes back by hand. `apps/dsplr/tests/hyprland_e2e.rs`
drives `dsplr apply` in the headless harness, locally and in CI's VM: kept with
`--yes` then reverted by hand, declined, timed out, and dsplr killed at the
prompt so its watchdog reverts. `dsplr save` writes the `monitors.lua` beside a
Hyprland Lua config (`Backend::persist`, built on
`dsplr_persist::hyprland::save`); its e2e has not run yet (below). Not written
yet: `dsplr set`, and saving under a hyprlang config or on wlroots.

Since 2026-11-09 one session (the Linux one) builds every crate, by the owner's
call; the "Linux-owned" and "Windows-owned" headings below keep the old split
only as a way to group the work. The Hyprland e2e needs `kwin_wayland` to nest
Hyprland in; the owner's machine has none installed (2026-11-09), so there it
runs only in CI's VM.

## Decisions waiting on the owner

Listed with defaults in `docs/plan/12-open-questions.md`. The ones that block a
milestone:

- **X11 bindings**: `x11rb` or crcbl's own X11 FFI (blocks M7). The crcbl
  session's view (2026-11-09): use `x11rb`. crcbl-shell's RandR is read-only
  enumeration for games (fullscreen monitor and scale); a set path
  (`SetCrtcConfig`, `SetScreenSize`, transforms, primary, rollback) would be
  tested only by dsplr and put a screen-breaking path in every game's
  dependencies. If the owner wants one stack anyway, crcbl suggests a separate
  X11 crate beside `crcbl-wayland`, not RandR inside crcbl-shell.

- **`dsplr save`, decided while building it (2026-11-09); confirm or change**:
  - The seam is `Backend::persist(&self, layout, approved: Option<&str>)`, not
    the plan's `persist(layout)`: a file needing a yes comes back as
    `Error::Conflict` with its text and the new text, nothing written, and the
    caller persists again with that text approved, which writes only if the file
    still holds it. The backend cannot ask; the CLI and the GUI can.
  - What the old `monitors.lua` says is carried over where the snapshot has
    nothing: its catch-all, its rules for monitors not connected now (so a save
    at home keeps the office monitor's rule), and its `vrr`, `bitdepth` and `cm`
    for a connected monitor, which Hyprland does not report as rules. The cost:
    a rule for a monitor gone for good stays until the file is edited (then
    `--force`).
  - `--force` also replaces another tool's file without asking, as it overwrites
    anything; `--yes` replaces only a readable file another tool wrote.
  - Exit codes: `4` when the backend cannot save at all (`Unsupported`: a
    hyprlang config, wlroots), as the backend refusing the request; `1` when
    nothing was saved because a file needs `--force`, the user said no, there
    was no terminal to ask on, or a file could not be read or written.
  - Saving prints its messages, the diff and the `require` line on stderr, as
    `dsplr apply` does.

## Open work in the Linux-owned crates

- **Hyprland adaptive sync**: the model now says "not known" (`None`) and the
  writer writes `vrr = 0` for off, so `Backend::capabilities` can list
  `[Off, On, Automatic]` for Hyprland. Not done: it needs an e2e step, and on a
  headless output that can only check Hyprland accepts the rule (see the
  Hyprland facts below), not that VRR changed.
- **Hyprland on `wlr-output-management`**: the `wlr` backend
  (`dsplr_backend::wlr`) is written and tested on sway, but Hyprland still
  applies through `eval` and reads `j/monitors`; it needs the protocol only for
  a real change stream. Moving any field onto it is a decision, since
  wlr-applied state overrides Hyprland's rules until restart (Hyprland facts
  below). Whether an `eval` mirror survives a protocol apply is unchecked.
- **No change stream yet**: every `wlr` read, test and apply opens its own
  connection (`Wlr::connect`) and closes it, so the serial guards only the
  instant between reading the heads and sending the configuration, and a change
  made after `dsplr list` is caught by the re-read (step 6), not by `cancelled`.
  The backend thread of `docs/plan/02-architecture.md`, holding one
  `wlr::Connection` and following its `done` events, is M3's. `Connection` reads
  nothing between calls today; it would need a dispatch loop for that.
- **A scale that is not a 256th is reported as changed after every `wlr`
  apply**: `set_scale` takes a `wl_fixed`, so 1.3 arrives as 1.30078125, and
  `Layout::differences` compares within `SCALE_EPSILON`. `dsplr apply` then
  waits out the whole `SETTLE` and lists the scale as not taken as asked. Either
  round the asked scale to what the backend can hold before applying, or let the
  backend give the comparison its own tolerance; the model is the Windows
  session's, so agree which.
- **`wlr` on other compositors**: only sway 1.12 (headless) has been run. niri,
  river, labwc and Wayfire are expected to work through the same protocol and
  are not checked; COSMIC's own extension and whether it saves outputs itself
  are still M4's to check (`docs/plan/04-backends.md`).
- **The save e2e has not run.**
  `a_saved_layout_is_what_hyprland_reads_back_after_a_reload`
  (`crates/dsplr-backend/tests/hyprland_e2e.rs`) and
  `dsplr_save_writes_monitors_lua_and_asks_before_replacing_one`
  (`apps/dsplr/tests/hyprland_e2e.rs`) compile but have run nowhere: the harness
  needs `kwin_wayland`, which the owner's machine lacks, so their first run is
  CI's; run them with `--test-threads=1`, since the save test's reload drops the
  other test's rules. The harness now starts Hyprland without `--config`,
  finding `hypr/hyprland.lua` in a private `XDG_CONFIG_HOME` the tests share,
  and gives KWin, Hyprland and the tests a private `HOME` (`CARGO_HOME` and
  `RUSTUP_HOME` pinned first; cargo under such a `HOME` was checked locally,
  KWin under one was not); that lookup, and `require("monitors")` resolving
  beside it, were checked with `Hyprland --verify-config` 0.57.1 (it reports a
  broken or missing `monitors.lua`), not in a running Hyprland.
- **What `dsplr save` cannot see**: a Hyprland started with `--config` (nothing
  outside Hyprland can read that path), and the system configs in
  `$XDG_CONFIG_DIRS` and `/etc/xdg` (not the user's to write beside), so with
  either it writes beside the user's own config or finds none. It also reads its
  own environment, not Hyprland's: run over ssh, or with another
  `XDG_CONFIG_HOME` or `HYPRLAND_CONFIG` than the compositor was started with,
  it finds a different config than Hyprland reads. Whether `hyprland.lua` loads
  the file is a line-by-line text search for `require("monitors")` outside `--`
  comments (`require_line`), and whether rules override it a search for
  `hl.monitor(` on a later line (`monitor_calls_after`): a
  `pcall(require, "monitors")`, a require inside another required file, a call
  written `hl.monitor (`, or text inside a multi-line `--[[ ]]` comment is
  misjudged, and only the message printed after the save is wrong.
- **A symbolic link at `monitors.lua` is refused** (`SaveError::Symlink`), even
  with `--force`: the atomic rename would replace the link with a file, so the
  change would never reach its target (a dotfiles checkout) and the next deploy
  would undo it. The message names the link and its target; the user changes the
  file at its source. Writing through the link to its target was considered and
  not done: the target is a deployed copy or a checkout, and dsplr editing
  either is a change the user did not ask for.
- **Testing on the owner's desktop**: only after the headless suite passes, and
  only with the owner's go-ahead (owner's rule, 2026-11-04). Not done yet.
- **The GUI (M3)**: `dsplr-gui` is still a stub; nothing is built against crcbl.
- **Connector names from crcbl (C3 in `docs/plan/06-gui.md`)**: crcbl agreed
  (2026-11-09) to add `connector: Option<String>` to crcbl-shell's
  `MonitorInfo`, `Some` only when the platform reported a name
  (`wl_output.name`, the RandR output name) and `None` for its fallbacks
  (`output-N`, `wl-output-N`, `screen`) and on Win32, AppKit, web and headless.
  Send crcbl the request when M3 starts; it builds it as one slice with e2e
  tests. On Wayland the name can arrive after the output first appears, so match
  on hotplug and change events too. On wlroots and Hyprland it equals
  wlr-output-management's `head.name`.
- **`crcbl-wl-scanner`'s `decode_event` reads past its borrow; crcbl is fixing
  it.** The generated `decode_event(opcode, args: &WlArgument)` turns the
  reference to one slot into a pointer and reads `args.add(1..)` for events with
  more than one argument, which is outside that reference's provenance under
  Stacked Borrows (undefined behaviour by Rust's rules, though it works in
  practice). Reported 2026-11-09; crcbl confirmed it and is changing the
  generated signature to a slice `&[WlArgument]` sized from the message
  signature, with a Miri test. That is a breaking change to the generated API:
  when the crcbl release lands, bump every crcbl crate and have dsplr's
  dispatcher in `wlr::connection` build the slice (crcbl will send the helper).
- **e2e for the other desktops**: each backend (M5 to M7) adds a
  `headless-<desktop>.sh` harness and a CI matrix entry, as
  `tests/e2e/headless-sway.sh` and the `sway` entry did for `wlr`. niri has no
  headless mode of its own; nest it (winit) in a headless sway, which was
  verified for the lyr project.

## Open work in the Windows-owned crates

- **`dsplr set <output> [--mode …] [--pos …] …`** (`docs/plan/07-cli.md`): not
  written. It edits one output of a fresh snapshot and hands the result to the
  same flow as `dsplr apply` (`apply::run` in `apps/dsplr/src/apply.rs`), so
  only argument parsing (`WxH@R`, `X,Y`, transform and vrr names) and finding
  the output by connector or description are new.
- **`dsplr restore --revert` by hand applies without a confirm prompt**, on
  purpose: it is the way back from a broken screen, so asking first would defeat
  it. The revert point is removed once it has been applied.
- **Legacy `monitors.conf` writer** (hyprlang `monitor = …` lines): not written,
  so `dsplr save` under a `hyprland.conf` is `Unsupported` (exit 4). Hyprland
  0.57.1 no longer reads hyprlang configs at all (its `getMainConfigPath` looks
  for `hyprland.lua` only), so the writer would serve 0.56 and older; whether it
  is still wanted is the owner's call. The Linux session offered to run a
  generated file through headless Hyprland 0.57.1 to check it.
- **The reader refuses Hyprland forms a layout cannot hold**: `preferred` and
  `highres` modes, a mode without a refresh, `auto` positions and scales,
  `vrr = 3` (fullscreen video and games, which a layout cannot tell from 2), any
  setting beside `disabled = true` (including one merged from an earlier call
  for the same output), and the SDR, luminance, `icc` and `reserved` keys. A
  hand-written file using them cannot be imported, and `dsplr save` replaces it
  only with `--force`, losing them and everything else in it (its catch-all and
  its rules for monitors not connected now); a lossless pass-through of unknown
  keys would fix the latter.
- **`dsplr save`'s diff is hand-written**: `dsplr_persist::diff::line_diff`, a
  textbook longest-common-subsequence table, tested against CLRS's example; it
  shows every line, and past `MAX_CELLS` it shows the changed middle as removed
  then added. The `similar` crate would replace it, but adding a dependency is
  the owner's call.
- **A change to `monitors.lua` made while `dsplr save` runs is lost**: it reads
  and classifies the file, then renames the new one over whatever is there then,
  without looking again. A file created, edited by hand or replaced by another
  tool in that window is overwritten without the check or the question it would
  otherwise get. When it asks, the second call re-reads and compares the text
  the user approved, so a change while the question waits is caught; one made
  during a call's own read-to-rename (a read, a write and an `fsync`) is not. No
  lock: nothing else that writes the file would honour one.
- **`dsplr apply` reads JSON only**: `docs/plan/07-cli.md` also names TOML
  layout files, which wait for the profile store's format.
- **`Transform`'s text uses wl_output's quarter turns, not sway's**: its
  `Display` says it writes sway's names, but `Rotate90` (wl_output 1) writes
  `90`, while sway calls that turn `270`: `swaymsg output X transform 90` sets
  wl_output transform 3, and sway's IPC reports wl_output 1 as `"270"` (sway
  1.12, 2026-11-09). The flipped quarter turns presumably swap the same way; not
  checked. It decides what M4's sway `outputs` writer writes.
- **`dsplr list` prints `@ 0 Hz` for a mode with no refresh**: headless sway's
  outputs report refresh 0, which the protocol means as "not said".
- **Some error paths of `dsplr apply` have no test**: a test or apply the
  desktop dropped because the displays changed (`Error::is_stale`; dsplr shows
  the current state, and after an apply removes the revert point instead of
  going back, which would undo whatever changed them), and a test or apply that
  could not reach the desktop (a timeout or lost connection, exit 1). The
  backend's `cancelled` is tested in `sway_e2e`; provoking it through the CLI
  means winning a race inside one `dsplr apply`, and the transport failures need
  a compositor that stalls.
- **The watchdog's output goes nowhere**: its stdin, stdout and stderr are
  closed, since the terminal may be gone when it fires, so a watchdog that fails
  to revert says so to no one. A log file in the state directory would fix that.
- **Make normalisation to the PNP vendor code** (`OutputId`, `docs/plan/03`):
  not written; needs a vendor-name table with a source, and waits for M6 to see
  what Mutter reports.

## Verified while planning

On the owner's machine, 2026-11-04, Hyprland 0.56.2 with the Lua config:

- `zwlr_output_manager_v1` is offered at version 4 (from `wayland-info`).
- `hyprctl keyword` is refused under the Lua config ("keyword can't work with
  non-legacy parsers. Use eval.").
- `hyprctl monitors all -j` field names, as listed in
  `docs/plan/04-backends.md`.
- `~/.config/hypr/monitors.lua` is generated by nwg-displays as
  `hl.monitor({…})` calls and `require`d from `hyprland.lua`.

From the plan review (2026-11-04), each checked against its source:

- Hyprland's `wlr-output-management` `test` is a stub that always succeeds
  (`src/protocols/OutputManagement.cpp`, main).
- KDE's `kde_output_management_v2` (v12) has no test request, requires no gaps
  or overlaps, and the priority order is `kde_output_order_v1`
  (`plasma-wayland-protocols` in `wayland-protocols-plasma` 0.3.12).
- KWin 6.8.2 (`libkwin.so.6.8.2`) contains `set_custom_modes`,
  `set_replication_source` and `applyMirroring`, which the crate's protocol
  files lack.
- Mutter's `ApplyMonitorsConfig`: `temporary` never reverts; `persistent` starts
  GNOME Shell's confirm and saves only when kept; the timeout is
  `DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`; stale serials and a policy switch
  refuse the call (`src/backends/meta-monitor-manager.c`, main).
- Mutter offers no `wlr-layer-shell` (`src/meson.build`'s protocol list).
- clap exits with `USAGE_CODE` 2 on usage errors (`clap_builder` 4.6.7).
- `wlr-output-management`'s `set_scale` takes a `wl_fixed` the compositor may
  round.

crcbl's tree was read at source level only for the needs in
`docs/plan/06-gui.md`; nothing has been built against it.

## Not verified (stated as gaps)

Reported correct by the plan review but not rechecked by hand: the
`ApplyMonitorsConfig` method values and `layout-mode` semantics (Mutter's
DisplayConfig XML), niri `include` support and its `zwlr_output_manager_v1`
offer (niri 26.04 binary strings), sway's `include` and `adaptive_sync`, and the
`hl.monitor` field names (Hyprland wiki).

Not verified by anyone: that KWin saves on every protocol apply; that Mutter
reports `vendor` as a PNP code (why `OutputId` normalises make from the EDID);
COSMIC's protocol offer and whether it persists outputs itself; GNOME Shell
accepting `ShowMonitorLabels` from another client; the Xorg dummy driver giving
several RandR outputs; whether an `eval` of `hl.monitor` survives a protocol
apply; KDE's confirm timeout; the RandR 1.5 details. Each is checked in its
backend's milestone.

- **One unexplained failure of `hyprland_e2e`**: its first local run after the
  switch to `Backend::apply` (2026-11-04) failed, and its output was cut off
  before the panic message. The next 27 runs passed, including one that
  recompiled inside the harness. If it recurs, keep the whole log.
- **Input injection in the headless Hyprland harness**: office-pikr reported
  that keys sent with `wtype` do not trigger Hyprland binds under
  `tests/e2e/headless-hyprland.sh`. Not reproduced or investigated here; dsplr's
  own tests send no input.

## Hyprland facts found while building M2 (2026-11-04)

From a headless Hyprland 0.57.1 and the v0.56.2 source; they shape the eval path
and the `monitors.lua` writer:

- `hl.monitor` merges onto the existing rule with the same `output` name
  (`hlMonitor`), so an omitted field keeps its old value: re-enabling needs
  `disabled = false`, un-mirroring `mirror = ""`. Send every field.
- The last matching rule wins (`CMonitorRuleManager::get` walks in reverse); the
  `output = ""` rule is only a fallback. `desc:X` is a prefix match on "make
  model serial", so an empty `desc:` matches every monitor.
- `position = "auto"` outputs are re-placed when another monitor changes: send
  the whole layout, with explicit positions, in one `eval`.
- `eval` rules are runtime-only; `hyprctl reload` drops them.
- State applied through `wlr-output-management` overrides the matching rule's
  enabled, mode, position, transform, scale and vrr until restart
  (`applyWlrOutputConfig`), so a later `eval` or reload does not change them.
  The live path must pick one mechanism per field.
- `mirrorOf` in `hyprctl -j monitors` is the source's numeric id.
- A headless output never reports adaptive sync active: after `vrr = 1` or
  `vrr = 2` through `eval` (both answered `ok`), `j/monitors` still says
  `vrr: false` (Hyprland 0.57.1, 2026-11-04). An e2e test can check that a vrr
  rule is accepted, not that it took effect.
- `vrr` in `hyprctl -j monitors` is whether adaptive sync is active now, not the
  rule: a fullscreen-only rule reads as false outside fullscreen, and as true
  inside it. So the snapshot reports `adaptive_sync` as `None` (not known), and
  the writer writes `vrr` only when a layout sets it. Hyprland's `Capabilities`
  still leave `adaptive_sync` empty.
- `dsplr` applies with rules named by connector, not `desc:`: since `desc:` is a
  prefix match, a description ending in serial `SER1` would also catch a monitor
  with serial `SER12`. Inferred from the prefix rule above, not seen.
- Hyprland's wlr `test` is a stub that always succeeds.
- A nested Hyprland tries libseat's logind backend before falling back; on a
  desktop that asks to take control of the user's session.
  `LIBSEAT_BACKEND=seatd` in the harness stops it.
- Hyprland finds its config (v0.56.2 and v0.57.1, `Jeremy::getMainConfigPath`
  with hyprutils 0.14.3's `Path::findConfig`): `--config`, then
  `HYPRLAND_CONFIG`, then `hypr/hyprland.lua` in `$XDG_CONFIG_HOME`,
  `$HOME/.config`, `$XDG_CONFIG_DIRS` and `/etc/xdg` (each directory only when
  absolute); v0.56.2 then tries `hyprland.conf` in the same places, v0.57.1 does
  not. Which language a path is read as: v0.56.2 uses the Lua manager for a path
  ending in `.lua` and hyprlang for any other (`Config::initConfigManager`);
  v0.57.1 has only the Lua manager and reads every path as Lua. dsplr takes a
  path not ending in `.lua` as hyprlang either way and refuses to save. The
  config's directory is put first on Lua's `package.path`, so
  `require("monitors")` loads the `monitors.lua` beside it, and a reload re-runs
  required modules. A missing `monitors.lua` that is required makes the config
  fail to load ("module 'monitors' not found"; `--verify-config`, 0.57.1,
  2026-11-09).
- From the v0.57.1 source, not observed: Hyprland watches its config and every
  file it `require`d (`ConfigWatcher`) and reloads when one changes, unless
  `disable_autoreload` is set. So a `dsplr save` into a required `monitors.lua`
  likely reloads the whole config, which drops every `eval` rule; the saved file
  holds the current state, so the screens should not change.
- On a GitHub runner, neither vgem (with or without Mesa's `kms_swrast`) nor a
  nested Hyprland in a VM can allocate GBM buffers; Hyprland running on the VM's
  own seat (seatd) with a virtio GPU can, which is what CI does.

## Behaviour that surprised us but is not a bug

- **Headless sway's `test` passes nearly anything**: a 100000x100000 custom
  mode, overlapping outputs and every output disabled all pass; what it refuses
  is a mode whose buffer it cannot allocate (2000000000x1: `mmap failed`), which
  is what the e2e uses. Asking for adaptive sync on a headless output succeeds
  and it stays off, so, as on Hyprland, an e2e test can show the request is
  accepted, not that VRR changed.
- **Headless sway lists one made-up mode per output**: its current size, refresh
  0, not preferred; any other size is a custom mode.
- **Hyprland lists some modes twice.** Its `j/monitors all` reply on the owner's
  AOC Q27G4 has `1920x1080@60.00Hz`, `720x576@50.00Hz`,
  `720x480@60.00Hz`/`59.94Hz` and `640x480@59.94Hz` each twice, likely
  interlaced and progressive variants, and says nothing to tell them apart.
  dsplr shows both rather than drop a real mode. It matters for M2's mode
  matching: applying through `wlr-output-management` gives each its own mode
  object, so the two must be told apart there (by the protocol's own mode
  objects), not by size and refresh. The `wlr` backend does it by ordinal: a
  mode it read carries its index among its head's modes (`Mode::handle`), and
  each apply, on a connection of its own, re-resolves it on the head with the
  same connector at the same index, if that object still has the same size and
  refresh; otherwise it matches by size and refresh alone. A compositor that
  reorders a head's modes between the read and the apply can still swap two
  identical ones.
- **CI runs on `main` cancel each other.** The concurrency group cancels a run
  when a newer push lands, so with two sessions pushing, most commits get no
  finished run of their own; the run on the newest commit covers them all. Hold
  a push until the current run finishes when a green run per slice matters.

## Considered and declined

- **wayr for the GUI.** Wayland-only by construction, so it cannot serve the X11
  window managers in the MVP. crcbl covers both (owner's call, 2026-11-04).
- **Windows and macOS.** Their own display settings cover this.
- **Shelling out to `wlr-randr`, `kscreen-doctor`, `xrandr`, `gnome-randr`.**
  Each desktop is driven through its own interface; those tools are test
  cross-checks only.
- **Faking per-output scale on X11 with RandR CRTC transforms**
  (`xrandr --scale`): blurs and costs performance; the RandR backend reports
  scale as read-only instead.
- **Live screen previews in the canvas** (wdisplays-style screencopy). Cost of a
  screencopy path for little help in arranging; revisit if asked.
