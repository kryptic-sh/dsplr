# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the project stands (2026-11-04)

M0 (scaffold) and M1 (read Hyprland, `dsplr list`, `dsplr backend`) are done. On
the owner's machine (Hyprland 0.56.2) `dsplr list --json` was compared field by
field with `hyprctl -j monitors all` and matched. Detection knows only Hyprland.
Build order is `docs/plan/11-milestones.md`.

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
prompt so its watchdog reverts. Not written yet: `dsplr set` and saving. The
Windows session owns the CLI, model and persist crates; the Linux session the
backend and their e2e coverage.

Two sessions work on it, each owning crates: the Linux session owns
`dsplr-backend`, `dsplr-gui` and the Linux e2e CI; the Windows session owns
`dsplr-model`, `dsplr-persist`, `dsplr-core`, `dsplr-config` and the CLI wiring
in `apps/dsplr`. Changes to shared files (workspace manifest, lockfile, CI, root
config, `docs/`) are agreed between them first.

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

## Open work in the Linux-owned crates

- **Hyprland adaptive sync**: the model now says "not known" (`None`) and the
  writer writes `vrr = 0` for off, so `Backend::capabilities` can list
  `[Off, On, Automatic]` for Hyprland. Not done: it needs an e2e step, and on a
  headless output that can only check Hyprland accepts the rule (see the
  Hyprland facts below), not that VRR changed.
- **`wlr-output-management` apply and test** (shared with M4's `Wlr`): no longer
  blocked. `crcbl-wayland` 0.17.2, crcbl-shell's libwayland-client binding with
  a raw connection API so `crcbl-wl-scanner` output works outside crcbl, is on
  crates.io (seen 2026-11-04); pin it with `cargo add`.
  `crates/crcbl-wayland-probe/src/outputs.rs` in crcbl is a worked example.
  `zwlr_output_configuration_v1` apply and test have not been exercised from
  outside crcbl. Hyprland needs it only for a real change stream; applying
  already works through `eval`. Whether an `eval` mirror survives a protocol
  apply is unchecked.
- **`Backend::persist`**: not written. Agree with the Windows session where the
  `monitors.lua` save is composed (their entry "Saving `monitors.lua`").
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
- **First outside use of `crcbl-wayland`'s configuration apply**: crcbl has
  never run `zwlr_output_configuration_v1` `apply`/`test` or their replies;
  dsplr's M2 is the first user. Report anything that breaks to the crcbl session
  (crcbl's backlog keeps a matching entry).
- **e2e for the other desktops**: each backend (M4 to M7) adds a
  `headless-<desktop>.sh` harness and a CI matrix entry. niri has no headless
  mode of its own; nest it (winit) in a headless sway, which was verified for
  the lyr project.

## Open work in the Windows-owned crates

- **`dsplr set <output> [--mode …] [--pos …] …`** (`docs/plan/07-cli.md`): not
  written. It edits one output of a fresh snapshot and hands the result to the
  same flow as `dsplr apply` (`apply::run` in `apps/dsplr/src/apply.rs`), so
  only argument parsing (`WxH@R`, `X,Y`, transform and vrr names) and finding
  the output by connector or description are new.
- **`dsplr restore --revert` by hand applies without a confirm prompt**, on
  purpose: it is the way back from a broken screen, so asking first would defeat
  it. The revert point is removed once it has been applied.
- **Saving `monitors.lua`**: the pieces exist (`dsplr_persist::hyprland`'s
  writer and reader, `dsplr_persist::managed`'s header, hand-edit check and
  atomic write) but nothing composes them into a save yet: read the existing
  file, refuse an `EditedByHand` one without `--force` or a confirm showing the
  diff, offer to replace a `Foreign` one (nwg-displays') after a confirm, write.
  It belongs with `Backend::persist` (the Linux session's) or a helper in
  `dsplr-persist` it calls; agree which before writing it.
- **Legacy `monitors.conf` writer** (hyprlang `monitor = …` lines): not written.
  The Linux session offered to run a generated file through headless Hyprland
  0.57.1 to check it.
- **The reader refuses Hyprland forms a layout cannot hold**: `preferred` and
  `highres` modes, a mode without a refresh, `auto` positions and scales, and
  the SDR, luminance, `icc` and `reserved` keys. A hand-written file using them
  cannot be imported; a lossless pass-through of unknown keys would fix the
  latter.
- **`vrr = 3` reads back as automatic**, the same as 2, since a layout does not
  tell fullscreen-only from fullscreen-video-and-games.
- **`dsplr apply` reads JSON only**: `docs/plan/07-cli.md` also names TOML
  layout files, which wait for the profile store's format.
- **`dsplr apply --dry-run` validates locally only** on every backend, since no
  backend has a test request wired up; Hyprland's is a stub anyway.
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
- On a GitHub runner, neither vgem (with or without Mesa's `kms_swrast`) nor a
  nested Hyprland in a VM can allocate GBM buffers; Hyprland running on the VM's
  own seat (seatd) with a virtio GPU can, which is what CI does.

## Behaviour that surprised us but is not a bug

- **Hyprland lists some modes twice.** Its `j/monitors all` reply on the owner's
  AOC Q27G4 has `1920x1080@60.00Hz`, `720x576@50.00Hz`,
  `720x480@60.00Hz`/`59.94Hz` and `640x480@59.94Hz` each twice, likely
  interlaced and progressive variants, and says nothing to tell them apart.
  dsplr shows both rather than drop a real mode. It matters for M2's mode
  matching: applying through `wlr-output-management` gives each its own mode
  object, so the two must be told apart there (by the protocol's own mode
  objects), not by size and refresh.
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
