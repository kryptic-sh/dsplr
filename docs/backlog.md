# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the project stands (2026-11-09)

M0 (scaffold) and M1 (read Hyprland, `dsplr list`, `dsplr backend`) are done. On
the owner's machine (Hyprland 0.56.2) `dsplr list --json` was compared field by
field with `hyprctl -j monitors all` and matched. Detection knows Hyprland, KDE
Plasma (`kde`, over KWin's own protocols, which reads and applies; built at M5,
its e2e green in CI, below), GNOME (`gnome`, over Mutter's `DisplayConfig` on
the session bus, which reads, tests, applies and saves through Mutter; built at
M6, its e2e green in CI, below) and wlroots compositors (`wlr`, over
`zwlr_output_manager_v1`, which reads, tests and applies; its e2e runs on
headless sway). Build order is `docs/plan/11-milestones.md`.

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
`dsplr_persist::hyprland::save`); its e2e runs in CI's VM, green in
run 42776828806. `dsplr set` edits one output of a fresh snapshot and hands the
layout to the same flow (`apply::run`); its steps run in
`apps/dsplr/tests/sway_e2e.rs`, and a mirror step and a `--no-mirror` step in
`hyprland_e2e.rs` (in CI's VM). `dsplr save` on sway writes `outputs` beside
sway's config (`dsplr_persist::sway::save`); both sway e2e tests save, change
the outputs at runtime, reload and read the saved layout back, and start a
second sway on the saved file, green on headless sway 1.12 locally (2026-11-10),
each alone and both in one harness in either order. dsplr's profile store
(`dsplr_persist::profile`; `dsplr save --profile`, `dsplr restore [NAME]`,
`dsplr profile list|show|rm`) is written, and `apps/dsplr/tests/sway_e2e.rs`
saves, moves, restores (picked and by name) and removes profiles on headless
sway 1.12 (2026-11-10). Saving under a hyprlang config is declined (below).
`wlr` works on niri 26.04, and `dsplr save` on niri writes `outputs.kdl` beside
niri's config (`dsplr_persist::niri::save`); both niri e2e tests
(`tests/e2e/headless-niri.sh`, niri nested in a headless sway) are green locally
(2026-11-10), each alone and both in one harness in either order, and in CI's
`niri` entry (green in run 42776828806).

M3 (the GUI) is built but for its exit check, the owner switching from
nwg-displays. `dsplr` with no command reads the displays on a backend thread and
opens a crcbl window showing the canvas (outputs at their logical sizes, the
tray of disabled ones, pending edits as outlines, validation problems in red and
yellow) and an editable property panel (crcbl-ui widgets). Edits are pending,
each undoable: dragging with snapping, the key table's moves, turns, scale steps
and on/off, and the panel's fields; quitting with pending edits asks.
`dsplr-core` holds the editor and builds a platform-free draw list with snapshot
tests. It ran in `tests/e2e/headless-sway.sh` on Mesa's lavapipe (2026-11-10).
Apply, save, revert and profiles run the CLI's confirm flow from the window,
with a confirm dialog per monitor; `i` shows each output's name on it; the
backend thread follows the desktop's changes (`Backend::watch`); keys are
rebound in `config.toml`'s `[keys]`; text is the system's sans-serif. What M3
still lists is in `docs/plan/06-gui.md` ("Still to build in M3").

M8 is under way: `dsplr watch` (`apps/dsplr/src/watch.rs`) restores the picked
profile, kept without asking, each time the connected outputs change, with the
rules in `docs/plan/05-persistence-and-profiles.md` ("Hotplug profiles").
`apps/dsplr/tests/sway_e2e.rs` plugs outputs in and unplugs them under it on a
second headless sway 1.12 of its own (2026-11-10). The GUI's profile picker was
built with the window in M3 (`dsplr-gui`'s `panel.rs`: pick a stored profile,
Restore). `dsplr profile import kanshi` and `dsplr profile import autorandr`
import kanshi's and autorandr's profiles.

Since 2026-11-09 one session (the Linux one) builds every crate, by the owner's
call; the "Linux-owned" and "Windows-owned" headings below keep the old split
only as a way to group the work. The Hyprland e2e needs `kwin_wayland` to nest
Hyprland in; the owner's machine has none installed (2026-11-09), so there it
runs only in CI's VM.

## Decisions waiting on the owner

Listed with defaults in `docs/plan/12-open-questions.md`. The ones that block a
milestone:

- **kanshi import freezes unset settings at import time** (M8, built that way;
  the options and trade-off are in the kanshi entry under "Open work in the
  Linux-owned crates"). Blocks nothing.

- **Where the monitor vendor names come from** (the "Make normalisation to the
  PNP vendor code" entry below; needed for profiles to match across X11/GNOME
  and Hyprland/wlroots). Hyprland and wlroots report the company name
  libdisplay-info looks up in hwdata's `pnp.ids` (`/usr/share/hwdata/pnp.ids`,
  hwdata 0.412, GPL-2.0-or-later); the EDID, RandR and Mutter give the
  three-letter code. A code maps to one name, but a name can have several codes
  (`DEL` and `DLL` are both Dell), so the match should turn codes into names,
  not the other way round. Options: (a) read the system's `pnp.ids` at runtime,
  which embeds nothing and falls back to the code when the file is missing, but
  makes hwdata a soft runtime dependency for packagers; (b) embed a generated
  table, which works everywhere but ships GPL-licensed data in an MIT crate; (c)
  compare only model and serial when the makes differ in form. Default if not
  answered: (a).

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
  - Exit codes: `3` when the backend cannot save at all (`Unsupported`: a
    hyprlang config, a wlroots compositor other than sway and niri, a layout
    with a mirror on sway), as dsplr declining before writing (changed from `4`
    on 2026-11-09, when `4` became the desktop's refusal only); `1` when nothing
    was saved because a file needs `--force`, the user said no, there was no
    terminal to ask on, or a file could not be read or written.
  - Saving prints its messages, the diff and the `require` line on stderr, as
    `dsplr apply` does.

- **`dsplr save` on sway, decided while building it (2026-11-10); confirm or
  change**:
  - `outputs` goes beside the config sway reads, not always in
    `$XDG_CONFIG_HOME/sway/` (the plan's `~/.config/sway/outputs`, which is the
    same file when the config is in sway's suggested place). So the line to add
    is always `include outputs`, which sway resolves from the config's own
    directory; an absolute path would not do for a config directory with a space
    in it (sway 1.12 loaded neither a single- nor a double-quoted one). The
    cost: with `~/.sway/config` or `~/.i3/config` the file lands in `~/.sway/`
    or `~/.i3/`.
  - It is sway when the socket `SWAYSOCK` names answers sway's IPC `GET_VERSION`
    (`Wlr::compositor`, asked by `persist` and `dsplr backend` only, not at
    detection; `wlr::sway::answers`), so a stale `SWAYSOCK` that a session
    manager imported does not count. `Capabilities::native_persistence` on `wlr`
    is only "`SWAYSOCK` is set", since every command reads capabilities;
    `Backend::native_persistence` is the checked answer, and the GUI will want
    that one. Since 2026-11-10 the socket also has to be served by the process
    serving the Wayland socket (`Wlr::compositor`, `SO_PEERCRED` pids through
    tokio's `UnixStream::peer_cred`), so a compositor nested in sway, or one
    whose environment holds a running sway's `SWAYSOCK`, is not taken for sway.
    A compositor behind a Wayland proxy (waypipe, a sandbox's security context)
    shows the proxy's pid, so there saving is refused, not misdirected. Not seen
    behind a proxy.
  - Two connected monitors with the same identifier, and a monitor whose
    identifier holds a quote, a backslash or a `$`, are written by connector.
  - A monitor without a serial is named by connector, as `OutputId` keeps it,
    not by its sway identifier (`Make Model Unknown`), though at startup an
    identifier's command would win over the config's own command for that port.
    The other choice would also let the saved block beat a name command in the
    main config at startup, but not on a reload, and would move a serial-less
    monitor's settings with its model rather than its port.
  - `NoConfig` (no sway or Hyprland config of the user's own) exits `1` on both
    desktops; plan 07 could read it as a save the backend cannot do there (`3`).
    Left as it is; the owner's call.
  - Old commands are carried over only from a file in dsplr's own form, and then
    verbatim, for outputs not connected now; any other setting makes the file
    need `--force`. nwg-displays writes a sway `outputs` too, and if it writes
    settings dsplr does not (`scale_filter`, `power`; not checked against
    nwg-displays itself), replacing its file takes `--force`.

- **`dsplr save` on niri, decided while building it (2026-11-10); confirm or
  change**:
  - `outputs.kdl` goes beside the config niri reads, and the line to add is
    `include "outputs.kdl"`, **before** the config's own `output` nodes, since
    niri uses the first node naming an output (the opposite of sway and
    Hyprland; `Saved::first_wins` tells the CLI which). The plain form is the
    one printed, not `include optional=true "outputs.kdl"`: after a save the
    file exists, and an older niri may not take `optional`. Either is recognised
    as including it.
  - It is niri when `NIRI_SOCKET`'s socket answers `Version` and is served by
    the process serving the Wayland socket, as for sway above.
  - On niri, `Backend::persist` refuses a layout other than the one on screen
    (exit `3`), since writing the file applies it; it became `async` to read the
    outputs again (`Backend::native_persistence` and `Wlr::compositor` too, for
    the pid check).
  - The main config's own `output` nodes after the include, for a connected
    output, that set what dsplr does not write make the save a `Conflict`
    (`Conflict::shadowed`, with `Ownership::Absent` when there is no file yet)
    that takes a yes, not `--force`: nothing in a file is lost, the settings
    stop applying. It asks on every save while those nodes are there; moving
    those settings into a node before the include (and the rest into dsplr's
    file) is the way out, and dsplr does not say that yet.
  - `AdaptiveSync::Automatic` is written as
    `variable-refresh-rate on-demand=true` (niri turns VRR on for windows that
    ask for it), the nearest niri has to "the desktop decides". No snapshot
    reads it (wlr-output-management has only on and off), so today only a layout
    file reaches it, and `dsplr save` writes snapshots. The other choice was
    refusing it, as sway does.
  - `wlr` still claims `test_before_apply` on niri, whose `test` checks only
    that one output stays on (`docs/plan/04-backends.md`); a layout niri cannot
    take is then caught by the apply and the re-read, not the test. Treating
    niri's test as Hyprland's (`Unsupported`, step skipped) would say so
    honestly, at the cost of the protocol-level checks it does make (an unlisted
    mode, a scale of 0) moving to the apply.
  - A `NoConfig` on niri exits `1`, as on sway and Hyprland.

- **The profile store, decided while building it (2026-11-10); confirm or
  change**:
  - **JSON, not TOML** (`<name>.json`, plan 05 said `.toml`). No dependency
    dsplr has writes TOML: `hjkl-config` 0.45 reads it (`load_from`, into any
    `AppConfig`) and writes only one key at a time (`write_key_at`, taking
    `toml_edit` values it does not re-export, under a lock file), so a whole
    `Layout` cannot be written through it. Adding `toml` was left to the owner.
    For TOML: hand-editable, and the format plan 05 and `dsplr apply`'s TOML
    files (plan 07) name. For JSON: no new dependency, one format for
    `dsplr list --json`, `dsplr apply` and the store (a profile file is a layout
    file plus `saved_unix_ms`; `dsplr list --json > …/desk.json` is a profile),
    and TOML has no `null`, so the `Option` fields would be written by leaving
    them out. Switching later means reading both extensions for a while.
  - **Exit `7`** when `dsplr restore` without a name finds no profile for the
    connected outputs (nothing changed). The other choices were `0` (an
    autostart run away from the desk is not a failure, but a script cannot tell
    "nothing to do" from "restored") and `1` (indistinguishable from a real
    failure). A name that names no profile exits `2`, as an unknown output name
    does.
  - **Unattended restore takes `--yes`**: `dsplr restore` is `dsplr apply`'s
    flow, so with no terminal a change that can hide or move a screen is not
    applied without `--yes` (exit `1`); autostart lines are
    `dsplr restore --yes`. The other choice, restore implying `--yes` when there
    is no terminal, would make it the one command where no terminal means keep;
    the profile was on screen when saved, so `--yes` is the expected case and
    one flag in the autostart line is cheap.
  - Replacing a profile takes `--yes`, `--force` or a yes on the terminal, after
    a diff of the two layouts as `dsplr list` text; a profile file that cannot
    be read takes `--force`. Profile files carry no managed-file header (JSON
    has no comments), so a hand-edited profile is replaced like any other.
  - Names: ASCII letters and digits, spaces, `-`, `_`, `.`; starting with a
    letter, digit or `_`; not ending in a space or `.`; no `..`; at most 200
    bytes. A `.json` file in the store whose name fails this is reported and
    passed over (`dsplr profile list` still exits `0` for it; a profile that
    cannot be read makes it exit `1`).
  - Pairing a profile's or layout file's outputs with the connected ones is a
    maximum bipartite matching (`dsplr_model::assign`, Kuhn's augmenting paths),
    shared by `dsplr restore`'s pick and `dsplr apply`. Where several pairings
    exist it places the closest matches first (an EDID identity on the connector
    it was saved on, then an EDID identity or a connector, then a whole
    description, then a description's first words), each entry taking its
    closest free output and moving one already placed only when it must; ties go
    in the order listed. So twins saved on ports still connected keep them, and
    twins saved elsewhere pair in the order listed, which may swap their
    settings.
  - Recency is `saved_unix_ms` in the file, falling back to the file's mtime
    when absent, so a copied profile keeps its save time; ties go to the earlier
    name.
  - `Persisted::Profile(name)` (plan 05's intro) was not added: profiles are
    saved by the CLI through `dsplr_persist::profile`, not through
    `Backend::persist`; the GUI may want it at M8.

- **`dsplr watch`, decided while building it (2026-11-10); confirm or change**:
  - A hotplug restore is kept without asking, as `dsplr restore --yes` and
    kanshi keep theirs: there is no terminal, and the revert point and its
    watchdog still guard the apply. The alternative is a confirm window on the
    desktop (the GUI's confirm dialog, opened by the daemon), which would make a
    hotplug restore that hides a screen revert itself unless kept, at the cost
    of a window popping up at every dock and undock, and the daemon depending on
    the GUI stack. Nothing is built for it yet.
  - The outputs connected when it starts count as a change, so it restores at
    once and `exec dsplr watch` replaces `dsplr restore --yes` in an autostart.
    After a reconnect it checks again the same way.
  - Its messages go to stderr, not plan 02's `tracing` logs in
    `$XDG_STATE_HOME/dsplr/logs/`: the compositor already puts its children's
    stderr in its log or the journal, and no other command logs to a file yet.
    Adding `tracing` and `tracing-subscriber` (on plan 02's list) is the way to
    a log file, for every command at once.
  - Giving up: after `RECONNECT_TRIES` failed tries in a row to reach the
    desktop again, it exits `1` rather than polling forever after the session
    ended. A watch that opens and ends at once does not start the tries afresh;
    only one that reported a change or stayed up for `STAYED_UP` does.

## Open work in the Linux-owned crates

- **The fallback rule is not a `Layout` field** (plan 12, question 11, for the
  owner): `dsplr save` keeps every Hyprland `output = ""` call and sway
  `output *` command of a file it can read, says so (`Saved::kept_catch_all`),
  and never writes one; `dsplr list`, the GUI and profiles cannot show or set
  one. Building `Layout::fallback` is the alternative there. Verified by unit
  tests only (`dsplr-persist`'s Hyprland reader and saver, sway's saver, the
  CLI's message); no e2e saves over a file with a catch-all. niri's lack of a
  catch-all is from plan 05's account of how niri 26.04 matches names, not a
  test of an `output "*"` node. KDE and GNOME keep their own configs, which
  dsplr does not write.
- **The KDE and GNOME e2e run only in CI** (`kwin_e2e.rs` and `mutter_e2e.rs` in
  both crates, under `tests/e2e/headless-kwin.sh` and `headless-mutter.sh`): the
  owner's machine has neither `kwin_wayland` nor `mutter`. Green in CI run
  42776828806 (main, `--backend`): KWin's four backend tests and the CLI's one,
  Mutter's five and the CLI's one, the latter with M6's review fixes (the moved
  layout said as `moved the layout by 0,-10`, `dsplr set … --disable --yes` on
  the primary at the origin, the stale serial). The GUI has not run on either.
- **GNOME's session check refuses some real sessions**: behind a Wayland proxy
  (waypipe, a Flatpak sandbox's) the socket's process is the proxy, so GNOME is
  not found there rather than misjudged; and on GNOME on X11 the check is only
  `XDG_CURRENT_DESKTOP`, which a nested X11 window manager inherits. Neither was
  seen.
- **A GNOME snapshot's serial is shared by clones of the backend**: the GUI's
  backend thread reads the displays after each change Mutter reports, so an
  apply racing a change it has not read yet is stale (exit `4` in the CLI, the
  displays shown again in the window) rather than applied over it. A request
  sent without any snapshot first (the watchdog's `dsplr restore --revert`) is
  planned on what it reads, as before.
- **GNOME adaptive sync**: Mutter 51.1 lists a variable-refresh mode as a mode
  of its own (`refresh-rate-mode` `variable` on the mode), not a setting, so
  dsplr shows it as a second mode of the same size and refresh and refuses
  `adaptive_sync` in a layout before sending (as `wlr` does without version 4).
  Mapping it means reading `refresh-rate-mode` into `Mode` or into
  `AdaptiveSync` by which mode is current; not designed.
- **GNOME physical layout mode not observed**: Mutter 51.1's native backend
  defaults to the logical mode; the physical one comes from a `monitors.xml`
  saying so or a client setting it, and the CLI e2e only checks that Mutter
  refuses a fractional scale there. Reading a physical layout, the canvas at
  mode sizes, and applying in it are unit-tested only. The editor's scale steps
  in the physical mode still offer the mode's listed scales, which Mutter lists
  for the current mode (integers only in the physical one).
- **GNOME RGB range, underscanning and colour mode are not observed**: dsplr
  reads `rgb-range` into `OutputState::rgb_range` and sends it back (or the
  layout's), and sends `underscanning` and `color-mode` back as read, since
  Mutter resets them when an apply leaves them out. Virtual monitors have none
  of them to show, so only the unit tests check them.
- **GNOME: other things not checked**: Cinnamon's Muffin
  (`org.cinnamon.Muffin.DisplayConfig`, same interface by its name); GNOME on
  X11 (Mutter 51.1 has none; which release dropped it was not checked); Mutter
  refusing D-Bus configuration by policy (`<policy><dbus>no</dbus>` in
  `monitors.xml`), classified as a refusal from its source; GNOME Shell's dialog
  and its timeout on a real session; the manual checklist on GNOME (M6's exit
  check).
- **`--backend wlr` on Hyprland is allowed** (decision for the owner): a backend
  named with `--backend` is checked alone, so the rule that stops detection at
  an unreachable Hyprland (plan 04) does not apply, and `--backend wlr` drives
  Hyprland through `wlr-output-management`. What that gives there: the test step
  (`--dry-run`, and the test before every apply) is Hyprland's stub, which
  always succeeds, so it proves nothing about the layout; what an apply sets is
  kept by Hyprland per monitor beside its monitor rules (plan 04's Hyprland
  section); and saving is refused, since without `SWAYSOCK` or `NIRI_SOCKET`
  answering the compositor counts as `wlr::Compositor::Other`
  (`Backend::persist` returns `Unsupported`; read in the source, not run on
  Hyprland). Chosen because the flag exists for testing and for desktops with
  more than one interface, and the user asked for that one by name. The other
  option is refusing `wlr` (and `kde`) while `HYPRLAND_INSTANCE_SIGNATURE` is
  set, with why, exit 6. Not run on a Hyprland session.
- **`--backend` takes the backends' own names only** (`hyprland`, `kde`,
  `gnome`, `wlr`, `randr` or `x11`), as `dsplr backend` spells them; `sway`,
  `niri` and the other wlroots compositors' names are usage errors listing
  those. An alias would read as checking that the compositor is sway, which
  `wlr` does not. Say if aliases are wanted anyway.
- **`--backend`'s coverage**: unit tests for the names, the parse before and
  after the command, unknown names (exit 2), each named backend being the only
  one checked (Hyprland's stop included), and the watchdog's arguments carrying
  it. The sway e2e checks `gnome`, `kde`, `hyprland` and `x11` exit `6` naming
  the backend and its check without checking wlroots, `dsplr backend` likewise,
  `--backend wlr` running `backend`, `set --yes` and `restore --revert`, and a
  `--backend wlr set` killed at its prompt (on a `script` terminal), whose
  watchdog, seen in `/proc` with `--backend wlr` on its command line, reverts it
  and removes the revert point. Not driven end to end: the GUI, `watch` and
  `identify` with the flag (each finds its backend through `find_backend`, which
  reads it). The non-Linux arm of `detect_named` is checked only by
  `cargo clippy -p dsplr-backend --target x86_64-pc-windows-gnu`. Not run on
  Hyprland, KDE or GNOME with it.
- **KDE custom modes run only in CI's KWin, and not yet**: the two-step apply
  (`kde::Connection::apply`: the mode list, then the layout at the mode KWin
  made) is unit-tested on its plan (`kde::plan`'s lists, kept custom modes,
  `add_cvt` from version 22, the mode object chosen) and on picking the new mode
  (`connection::new_mode`), not against KWin. The KDE e2e now applies two custom
  modes to a virtual output and checks both stay listed, and the CLI's sets one
  with `dsplr set <last> --mode 1280x720@60 --yes`; they run only in CI.
  Unchecked until then: that virtual outputs take custom modes, how far KWin's
  refresh lands from the one asked (the e2e allows 1 Hz), whether it re-creates
  kept custom modes as new objects, and whether a configuration holding only
  mode lists is applied. The refresh KWin makes is reported as a read-back
  difference rather than taken as asked, and a custom mode of a change reverted
  after it was applied stays in KWin's list, unused: dsplr cannot remove one
  (one whose layout was not applied is taken out again). Reduced blanking is
  always off for a new mode; CVT timings are not offered for one. On management
  versions 18 to 21 (no `add_cvt`), the custom modes KWin already lists go back
  in each list sent by their size, the refresh KWin computed and reduced
  blanking, so KWin makes their timings again from that refresh, which can move
  it a little each time a list is sent; lists are sent only for an apply that
  adds a mode, so the drift is per such apply, not per apply. Not observed.
- **KWin before the device registry**: KWin offers devices only through
  `kde_output_device_registry_v2` (version 21 or later) in 6.8; older Plasma 6
  releases published each `kde_output_device_v2` as a global instead, which
  dsplr does not bind, so on those `dsplr backend` reports the registry missing
  or too old. Supporting them means binding each device global and following
  `wl_registry.global_remove`, tested against a KWin that old. Which KWin
  release first shipped the registry was not checked.
- **KDE hotplug is not tested**: `kwin_wayland --virtual` makes its outputs at
  start, and nothing outside KWin adds one, so the KDE watch test covers changes
  made by another client and a new output order, not an output plugged in or
  removed (`removed` and a new registry `output`).
- **On KDE the GUI says KWin saved the change only once it is applied**: the
  bar's confirm line adds it while the change waits to be kept
  (`dsplr_core::dialog::confirm_prompt`, from `Capabilities::saves_on_apply`),
  not before the apply as plan 02's step 7 first put it, and the dialogs on each
  monitor do not repeat it. The CLI's terminal prompt does not say it. Neither
  was seen on screen: the bar's text is checked as a string, not drawn, and a
  long bar line may be cut off on a narrow window.
- **KWin's other output fields are not read**: EDID, EISA id, HDR, wide colour
  gamut, ICC profile, brightness, max bits per colour and the rest. The EISA id
  is the PNP vendor code plan 03 wants `make` normalised to, so it is the first
  candidate once that normalisation is built.
- **An output KWin added or removed while a configuration was made** fails it
  with "One of the relevant outputs is no longer available", reported as a
  refusal (`Error::is_refusal`), not as stale: the reason is translated
  (`i18n`), so dsplr cannot tell it from another refusal by its text.
- **`crcbl-wl-scanner` refuses CDATA**, which every KDE protocol file uses for
  its copyright, so `crates/dsplr-backend/build.rs` turns each CDATA section
  into escaped text before parsing (`without_cdata`) and the vendored files stay
  unmodified. Teaching the scanner's reader CDATA would remove that step; it is
  crcbl's call.
- **Detection opens a Wayland connection for the KDE check** on every desktop
  that is not Hyprland, before the wlroots check opens its own: one more
  connection and roundtrip per command on wlroots compositors. Not measured.
  Sharing it is not simple: a `wayland::client::Client` is typed by the protocol
  whose events it follows, so a shared probe of the globals would be a
  connection of its own and save nothing on wlroots, unless the wlr check could
  take that connection over, which means changing a live client's protocol (or
  one protocol type for both).
- **Hyprland on `wlr-output-management`**: the `wlr` backend
  (`dsplr_backend::wlr`) is written and tested on sway, but Hyprland still
  applies through `eval` and reads `j/monitors`; it needs the protocol only for
  a real change stream. On Hyprland 0.57.2 a protocol apply keeps an `eval`
  mirror and a later `eval` wins over its position
  (`docs/plan/04-backends.md#hyprland`), so moving a field onto it is a choice
  of mechanism, not a conflict between them.
- **Hyprland's change stream misses rule changes; owner's call**: the GUI
  follows Hyprland through `.socket2.sock` (`hyprland::Events`: monitor added or
  removed, config reloaded), as asked for M3, which carries no event for a
  monitor rule changed with `hyprctl eval` / `keyword` by another program, so
  the window shows such a change only on Refresh or after its own next request.
  Plan 04 meant `wlr-output-management`'s `done` events instead, which see any
  change; that is a `wlr::Connection` held on Hyprland for watching only
  (`Watch::Wlr` already does it on wlroots), at the cost of a second connection
  and Hyprland's protocol support becoming a dependency of the GUI. Options:
  keep socket2, switch to wlr, or both. Checked on Hyprland 0.57.2 (CI,
  2026-11-10, `a_watch_follows_reloads_and_hotplug`): a reload and an output
  created and removed with `hyprctl output` are reported, and a monitor rule set
  through `eval` sends no event, by dsplr or anyone, so the watch cannot see
  one; the e2e asserts that, and goes red if Hyprland starts sending one.
- **`wlr` on other compositors**: sway 1.12 (headless) and niri 26.04 (nested)
  have been run. river, labwc and Wayfire are expected to work through the same
  protocol and are not checked; COSMIC's own extension and whether it saves
  outputs itself are still M4's to check (`docs/plan/04-backends.md`).
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
- **A Hyprland save drops comments outside the calls it keeps**: a kept rule or
  catch-all is its source text from `hl.monitor` to the closing `)`
  (`lua::Call::text`), so comments inside a call survive and a comment on the
  lines above it does not. A catch-all written other than with an `output`
  field, or with Lua's call-without-parentheses form, makes the file unreadable
  (`read_monitors_lua`), so replacing it takes `--force` rather than dropping it
  silently.
- **What `dsplr save` cannot see on sway**: `sway --config`, and
  `/etc/sway/config` (with no config of the user's own, saving fails and says to
  copy it). It reads its own environment, not sway's. Whether the config loads
  `outputs` is a search for an `include` whose path, with a leading `~`,
  `$VAR`/`${VAR}` and a relative path read from the config's directory, is the
  file: a glob (`include ~/.config/sway/config.d/*`), a `set $var` variable, an
  include inside another included file, or a variable sway's environment has and
  dsplr's has not, is misjudged, and only the message after the save is wrong.
  Whether commands override it follows the precedence in
  `docs/plan/05-persistence-and-profiles.md` ("sway"): an `output` command
  setting one of dsplr's settings for a connected output after that include
  (`Saved::overridden`), or one anywhere naming a connected output by identifier
  when the file names it by connector (`Saved::beaten_at_startup`), with blocks
  expanded (a `{` on a line of its own opens one only right after an `output`
  command).
- **The sway harness runs the test command in its own process group** (`setsid`)
  and kills that group on exit, so a second sway a killed test started
  (`sway_support::FreshSway`) does not outlive the run; checked by hand with a
  command that starts a child and SIGKILLs itself. The Hyprland harness was not
  changed: its tests start no compositor of their own.
- **A kept identifier command can beat a saved connector block, later**: a
  command kept for a monitor not connected now, named by its identifier, wins
  over the saved block for a port at startup the day that monitor is plugged
  into that port, while on a reload the saved block, written after it, wins.
  Same family as Hyprland's kept-versus-live ordering below; needs a monitor
  with a serial saved earlier and a serial-less one saved on its port now.
- **sway behaviour not seen on real hardware**: headless outputs report no
  serial, so the e2e names them by connector, and naming by identifier was
  checked only by hand on headless sway 1.12: a config for
  `"Unknown Unknown Unknown"` applied to them, and a partial one did not;
  `mode --custom` for an unlisted mode on a DRM output, `adaptive_sync` taking
  effect, and a quoted identifier holding punctuation such as a comma
  (`"Dell, Inc. …"`) are unchecked.
- **niri not seen on real hardware**: the nested `winit` output takes only a
  scale and a position, so a saved `mode` (niri picks a DRM mode by exact
  millihertz after rounding the written refresh, by a reading of v26.04's
  source, not observed), `mode custom=true`, `transform` (names taken from
  niri's source, not read back), `variable-refresh-rate` and `off` have been
  checked only by `niri validate` and golden tests. A monitor with a serial
  named by its identifier, and nodes kept for outputs not connected, ran in unit
  tests only: the nested niri has one output and no serial.
- **What `dsplr save` cannot see on niri**: `niri --config`, and
  `/etc/niri/config.kdl`. It reads its own environment, not niri's (a
  `NIRI_CONFIG` niri was started with and dsplr's shell lacks). Whether the
  config includes `outputs.kdl`, and whether its own `output` nodes come first,
  is read line by line (`niri::save::top_level`): a `/* */` comment, a string
  spanning lines, a node continued with `\`, or an include inside an included
  file is misjudged: the message after the save, and which of the config's
  `output` nodes the save asks about hiding.
- **niri's on-demand VRR is kept only while it reads as on**:
  `variable-refresh-rate on-demand=true` reads through wlr-output-management as
  whatever VRR is doing at that moment, and niri's IPC shows only `vrr_enabled`.
  A save keeps `on-demand=true` from the old saved node when the output reads as
  on; when it reads as off (no window asking for VRR), it is written off and
  lost, since dsplr cannot tell that from VRR turned off with
  `dsplr set --vrr off`. And one turned always-on with `dsplr set --vrr on`
  after an on-demand save is written on demand again. Which is what niri's
  wlr-output-management reports for an on-demand output was not seen: the nested
  `winit` output has no VRR. The main config's own on-demand nodes are asked
  about instead (above).
- **niri facts are niri 26.04's**: the KDL form (KDL 1, quoted names), the first
  node winning whole, writing the file applying it, and `succeeded` followed by
  `cancelled` were all seen on niri 26.04 only. CI installs whatever Arch ships,
  so a niri update can break them; the harness prints `niri --version` first, so
  a CI failure after an update shows which niri it was.
- **A saved niri scale is the 256th the protocol carried**: the scale arrives as
  a `wl_fixed` (steps of 1/256), so one niri holds as `1.333333` reads back
  rounded to a 256th, and `dsplr save` writes that, which niri then applies on
  its reload (the same rounding as the `wlr` apply entry above). Reading the
  scale from niri's IPC (`logical.scale`, a float) would keep it exact; not
  done, and the rounding was not observed on niri.
- **niri rounds a scale further than the wire**: `dsplr set winit --scale 1.1`
  on the nested niri 26.04 (2026-11-10) reads back 1.09765625 (281/256), not the
  282/256 the `wl_fixed` carried, so it is reported as not taken as asked. That
  is niri's own choice, which `Layout::differences` is meant to show;
  `Capabilities::scale_steps` covers only the wire's roundings. Why niri picks
  it was not looked into.
- **Hyprland has no fallback for two monitors with one description**: the sway
  writer names them by connector, but `monitors.lua` would get two `desc:` rules
  with the same text, and Hyprland would apply the last to both. Not fixed; the
  same check as `sway::output_name` would do.
- **A symbolic link at the saved file is refused** (`FileError::Symlink`, for
  `monitors.lua` and sway's `outputs`), even with `--force`: the atomic rename
  would replace the link with a file, so the change would never reach its target
  (a dotfiles checkout) and the next deploy would undo it. The message names the
  link and its target; the user changes the file at its source. Writing through
  the link to its target was considered and not done: the target is a deployed
  copy or a checkout, and dsplr editing either is a change the user did not ask
  for.
- **Testing on the owner's desktop**: only after the headless suite passes, and
  only with the owner's go-ahead (owner's rule, 2026-11-04). Not done yet.
- **The panel from the keyboard, how far it is checked**: `p` / `F6` takes the
  keys into the panel (crcbl-ui's focus traversal), unit-tested against the
  headless shell (landing, space on the Enabled box, Tab moving focus, Escape
  back to the canvas) and driven in `gui_e2e` for one path (`p`, Enter on the
  Enabled box, Escape, apply). Not driven in a window: the arrows inside an open
  list, typing into a field reached by Tab, and the drag-values. The focus ring
  (`:focus` in the panel's stylesheet) was not looked at on screen.
- **The message log, how far it is checked**: `m` shows the last
  `MESSAGE_HISTORY` lines of the backend thread's messages and the window's
  dropped-edit notes over the canvas (`draw_messages`, a draw-list snapshot);
  not looked at on screen. A click on it does nothing (`messages_bounds`); a
  click elsewhere on the canvas works as usual and leaves it shown.
- **The GUI's watch reopening is unit-tested in its decision only**:
  `gui::next_try` (the wait and the line, from `watch::Backoff`) has a test; the
  loop that acts on it in `serve` (`Following`, `next_followed`) has none, and
  no run has ended a watch, since nothing in the harnesses closes the
  compositor's socket while it stays up. A watch that cannot be opened at the
  start is not retried (said once, as before).
- **Key config limits**: `[keys]` takes Ctrl, Alt and Shift, not Super, and a
  key name from `dsplr_config::keys::NAMES` or F1 to F12 (no arrows' keypad
  forms, no Insert); the dialogs', quit prompt's and engaged panel field's keys
  are fixed. A key the file binds to two actions is refused rather than one
  winning; one the file binds that another action has by default is taken from
  it, with a warning. Character keys match the keysym's character, so they
  follow the keyboard layout; Shift is never compared for them (`H` is Shift+h).
- **Identify where there are no panels**: the window reports "identify is
  unsupported here" in the bar where crcbl's shell lacks `ShellCaps::PANEL`,
  GNOME included (GNOME Shell's `ShowMonitorLabels` refuses other callers, plan
  06), and `dsplr identify` exits `3` saying the same. The window's path has no
  test: `MainWindow::identify` needs the window's GPU, which the window's unit
  tests do not have; the standalone one does (`identify::show` against crcbl's
  headless shell without `ShellCaps::PANEL`). A monitor that matches no output
  shows the window system's monitor name instead.
- **`dsplr identify`'s coverage**: the GUI e2e runs it in headless sway and sees
  its panel in a `grim` capture (pixels of the theme's surface colour, none
  before or after) within `IDENTIFY_TIME` of the stderr line that names every
  enabled output, and checks it exits `0` no sooner than `IDENTIFY_TIME` after
  it started and within `STEP` after that line. Without a GUI session it checks:
  `CRCBL_SHELL=x11` with no `DISPLAY` (the backend still reachable over Wayland)
  exits `1` saying it needs one; no `WAYLAND_DISPLAY` or `DISPLAY` exits `6`,
  since detection finds no backend before a window is tried; and
  `CRCBL_SHELL=headless` exits `1`, as does the GUI, which would otherwise run
  there unseen (`dsplr_gui::seen`). CI's `gui` entry installs `grim`, and runs
  dsplr-gui's ignored library tests (its `lib` key), the two that need a Vulkan
  device: `identify::tests::a_click_on_a_panel_closes_it_early` and
  `shown_alone_it_stays_its_time_and_names_what_it_shows`. The click is not
  driven end to end, since headless sway has no pointer
  (`swaymsg seat - cursor press` delivered nothing to the panel, tried
  2026-11-10) and no injector is installed. Not tested: the NotShown path (every
  panel refused). Not run on X11.
- **The GUI's Replace is `dsplr save --force`**: the CLI asks a yes for a file
  someone else wrote and needs `--force` for one edited by hand or holding what
  dsplr cannot keep; the window shows why and the diff in every case and takes
  Replace (Enter) for all of them, since the user has seen what is lost. Say if
  the hand-edited case should take a second, stronger answer.
- **The confirm dialogs and identify panels fall back to matching by logical
  position** (`dsplr_core::dialog::match_screens`, within a pixel) where crcbl
  reports no connector for a monitor (Win32, AppKit, the web, headless, its
  made-up names) or one no output has (Xvfb's one RandR output, `screen`; RandR
  1.0 emulation's `default`). Read in crcbl-shell's `wayland/mod.rs` (`info`): a
  monitor's position is its `xdg_output` logical position times its scale where
  the compositor has `xdg_output`, else `wl_output.geometry`'s logical position
  as it is, without saying which, so `panels::screen` offers both (the first
  divided by the scale), first first; its size is the current mode,
  untransformed, so a turned output's logical size is not it, and size is not
  compared. Two outputs at one place (a mirror) name the first; a monitor
  matching none still gets its dialog, unnamed. That the connector wins over a
  position naming another output, and that a connector naming an output the
  layout turns off names none, is unit-tested (`match_screens`); which of the
  two matches named each panel in the headless sway GUI e2e was not observed;
  there both would name the same output.
- **Key moves can leave negative positions**: `h j k l` and `H J K L` move one
  output and nothing else, so a nudge left of x 0 leaves it at -10, which
  wlroots and Hyprland take; only a drag's release lifts the layout off negative
  positions (`Layout::make_non_negative`, plan 03). Lifting after key moves too
  would make them shift every other output on that axis, as a drag does. And
  `Layout::normalise`, the origin at 0,0 that XRandR requires, is to apply in
  the RandR backend (M7), not in the editor.
- **The GUI's choices that are guesses, not rules from a desktop**: the offered
  scales where a mode lists none (quarter steps 0.5 to 3, or 1 to 3 whole;
  Hyprland's own "clean" scales are not consulted), `NUDGE_STEP` (10 layout
  pixels), `SNAP_DISTANCE` (12 window pixels), and `H J K L` moving only to an
  output sharing some of the span across (a diagonal neighbour is not reached),
  and stopping at a neighbour it already touches rather than jumping past it.
  Any typed scale is still taken.
- **Hyprland's colour preset and bit depth are read as in force, not as the
  rule**: the snapshot reads `colorManagementPreset` and `currentFormat`, which
  say what Hyprland drives, so a rule the monitor cannot do reads as the
  fallback (`hdr`, `edid`, `hdredid`, `auto` as `srgb` on an output without an
  EDID; `bitdepth = 10` as 8 where 10 is not driven). Consequences: a save
  writes the fallback, so such a rule in `monitors.lua` is dropped (dsplr cannot
  tell it from the user having turned it off); an apply states a preset or depth
  only where the change makes it differ from what Hyprland reads
  (`hyprland::colour_as_changed`), so a move leaves such a rule alone, and a
  fallback is reported only when the user asked for that preset. Keeping the old
  file's rule where the live value is a fallback was considered and not built:
  it would also keep a rule the user changed on purpose. Not seen on a monitor
  that can do HDR or 10 bits (CI's output is headless, without an EDID). The
  comparison is against a fresh read at apply time, not the snapshot the edit
  started from (`Backend::apply` is not given the flow's `current`), so a preset
  another program changed in between is put back, as any other setting is, and a
  `cm = "auto"` that resolves differently between the two is pinned.
- **A profile asking a desktop-specific setting another desktop lacks is
  refused**: neutral values are dropped (`Layout::drop_neutral_unsupported`),
  others refused (exit `3`), so a Hyprland profile with `wide` or 10 bits, or a
  KDE one with overscan or a limited RGB range, does not restore on sway. A
  primary output named in a GNOME or KDE profile is refused on wlroots and
  Hyprland the same way, and was not covered by this: `primary = true` is a
  request, not a neutral value. Taking the live value instead, saying so, is the
  alternative.
- **What a save sees of the main config's monitor rules**: a save states
  `cm = "srgb"` and `bitdepth = 8` for an output whenever `hyprland.lua` has a
  `hl.monitor` call that could name it (one whose output it cannot read counts
  as naming every output), since the saved rule merges onto it. Read line by
  line (`save::main_rules`): a call spelled `hl.monitor (`, one in a multi-line
  comment, one made in a loop or a function, and one in another file the config
  requires are not seen, and there a value left out keeps that rule's. A call is
  cut at its first `)`, even inside a string (`desc:Dell (U2720Q)`), which makes
  it name every output (safe), and the first `output` text is taken, so a string
  earlier in the call containing that word can pick the wrong name.
- **The panel's overscan steps are a guess**: 0 to 10 % by one, then 15 and 20 %
  (`dsplr-core`'s `OVERSCANS`), plus the output's own value; KDE's own settings
  offer every percent. No `dsplr set` flag sets the primary output, overscan or
  RGB range: plan 07 lists none.
- **Panel keys are mapped by hand** (`dsplr-gui`'s `keys::nav`: Enter, Escape,
  the arrows, Tab) where crcbl's docs pair `TextPump` with `crcbl-input`'s
  action map. `crcbl-input` is not on plan 02's dependency list, so adding it is
  the owner's call.
- **What the GUI e2e covers** (`apps/dsplr/tests/gui_e2e.rs`, ignored; CI's
  `gui` entry, under `tests/e2e/headless-sway.sh` with lavapipe, green in run
  42776828806): kept with Enter, timed out, killed mid-confirm then reverted by
  the watchdog, an output plugged in with `swaymsg create_output` while the
  window is open, then reached with Tab, edited and applied together with an
  edit made before it came, the panel by keys, and `dsplr identify`; each read
  back with `dsplr list --json`. One fixed wait remains: the 2 s for the window
  to take in an output plugged in, which nothing outside it shows. Not driven by
  anything, since each needs a click (no pointer injector, `wlrctl` or
  `ydotool`, is installed, and headless sway has no pointer): dragging, the
  panel's drop-downs, checkbox, number and text fields (custom mode, profile
  name), the profile picker and Restore, the Revert and Refresh buttons, the
  dialogs' own Keep and Revert buttons, the tooltips, and the pointer position
  from a pointer-enter. They are covered by `dsplr-core`'s editor, hit-region,
  flow and dialog tests, a build of the panel's tree, and the window's key and
  request tests against crcbl's headless shell; Revert and profile restore run
  the CLI's own `restore::revert` and `apply::bind`. Not covered by any test: a
  draft committed to the output it was typed for when a canvas click moves the
  selection (`follow_selection` is tested for the reset, `Drafts::commits` on
  its own), a panel click ignored behind the quit prompt or the dialogs, and two
  navigation keys in one batch each getting a frame. Not run on niri, Hyprland,
  KWin, Mutter or X11, and the dialogs' X11 override-redirect panels never.
- **The GUI e2e hung twice in CI, cause not found** (2026-11-10, runs
  42793558495 and 42798488907, at a9db874 and after): the last output was
  `swaymsg create_output`'s reply in the hotplug step, then nothing until the
  VM's 30 min limit. Not reproduced locally (passed three times, once pinned to
  one CPU), and the next run (42804379976) passed. The likeliest wait was the
  window's exit after `q`, which a pending edit turns into the quit prompt;
  which edit could be left pending only there is unknown. Every wait in
  `gui_e2e.rs` is now bounded and a timeout prints sway's windows, the layout
  and the window's stderr (`gui.log`), so a recurrence says which wait it was.
- **The confirm's fallback toplevel has not run anywhere**: where the window
  system has no panels, the confirm is one toplevel, and closing it reverts
  (`dialog_answer`, unit-tested on the decision only). Every compositor the e2e
  tests use has panels, so the toplevel, its close and the loop's reply to the
  close request were not run. A panel's own close request (sent when its monitor
  goes) is left unanswered, as before: it answers nothing, and the panel goes
  when the dialogs close.
- **After an output is plugged in while the window is open, the confirm dialog
  does not take the keyboard**: in headless sway 1.12 (2026-11-10) the main
  window stayed focused through the confirm of an edit to the new output, where
  without a hotplug the dialog takes the keys about 0.2 s after the apply shows.
  Enter and Esc still answer it in the main window. Whether a dialog opened at
  all, and on which output, was not looked into.
- **No Miri in dsplr's CI**: the `wlr` dispatcher's decoding
  (`wlr::connection::tests`, hand-built argument arrays decoded through
  `crcbl_wayland::ffi::with_dispatcher_args`) passed under Miri locally
  (`cargo +nightly miri test -p dsplr-backend --lib wlr::connection`,
  2026-11-10, nightly miri of 2026-09-09), and Miri reported a Stacked Borrows
  violation when the slice was built from a reference to the first slot, the old
  bug's shape. `dispatch` itself needs a loaded libwayland, which Miri cannot
  `dlopen`, so it is covered by the e2e tests only. crcbl runs the same kind of
  check weekly (`cron.yml`'s `miri` job); a dsplr job for these four tests would
  take seconds plus a nightly toolchain. Not added: a CI job is the owner's
  call.
- **autorandr profiles named by connector use X's names**: an output
  `dsplr profile import autorandr` names by its connector (its EDID has no
  serial, or `setup` holds autorandr's no-EDID marker, a `*` pattern or an older
  MD5) keeps the X driver's name for the port: `HDMI-1` from modesetting,
  `HDMI1` from intel, `HDMI-A-0` from amdgpu. Wayland desktops name it from the
  kernel's connector (`HDMI-A-1`), so such a profile restores there only where
  the names happen to agree. Not mapped: the mapping depends on the driver the
  profile was saved under, which autorandr does not record. Needs a decision
  (map by driver conventions, ask at import, or leave it to the RandR backend),
  and is said in the CHANGELOG and plan 05.
- **A CRTC that reflects only in y refuses a flipped transform**: the RandR
  planner sets a flipped transform as `RR_Reflect_X` with the same rotation
  (`randr::plan::rotation`), and `can_use_rotation` refuses it on a CRTC whose
  rotations offer `RR_Reflect_Y` but not `RR_Reflect_X`. The same picture is
  `RR_Reflect_Y` turned half a turn further (`RRTransformCompute`), which xrandr
  would be given by a user's `--reflect y` and succeed with. Trying that
  equivalent before refusing would cover it; no such CRTC has been seen.
- **What the Xorg dummy harness cannot show**
  (`DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh`, `xorg_e2e.rs`):
  xf86-video-dummy 0.4.1 has no rotation or reflection, no EDID (every output is
  named by its connector), no panning, and an output turned on stays connected
  when turned off, so unplugging, and `dsplr watch` after an unplug, cannot be
  run on it. The harness runs it only with `DSPLR_E2E_XORG_MAY_TAKE_SESSION=1`:
  Xorg is kept off the system bus (`DBUS_SYSTEM_BUS_ADDRESS` at no socket,
  checked first with `dbus-send`) and on a seat no device is on (`-seat`), and
  its log must say it failed to reach the bus and mention neither logind nor a
  DRM device; read from xorg-server 21.1.26's `systemd-logind.c`, `dbus-core.c`,
  `config/udev.c` and `xf86Config.c`. Before that guard, one local run did reach
  logind, whose `TakeControl` failed only because Hyprland holds the session.
  Run locally with xorg-server 21.1.26 and the dummy driver unpacked from Arch's
  packages into a scratch directory (`DSPLR_E2E_XORG`,
  `DSPLR_E2E_XORG_MODULES`), not installed; the CI entry installs them in the VM
  and had not run when this was written.
- **X11 saving: choices made without the owner** (`dsplr_persist::login`, plan
  05's X11 section). Each is a call to confirm or change:
  - **Profile named by its outputs.** `dsplr save` on X11 names the profile from
    the connected outputs' identities, so each set of monitors keeps its own and
    a save replaces only the one it kept for the same monitors (marked
    `saved_for_outputs`); a profile of that name the user saved, or one kept for
    other outputs, makes it use the hash name instead, and say so. The other
    option was refusing a plain `dsplr save` there and requiring
    `--profile NAME`, as before.
  - **No separate confirm for the autostart entry.** Plan 05 said "installs (on
    confirm)". It is written as every other file a save writes is: created or
    overwritten when it is dsplr's, asked about when it is someone else's, and
    named in the save's output.
  - **The desktops that restore their own layout** (`login::OWN_RESTORE`: KDE,
    XFCE, MATE, X-Cinnamon, LXQt, Budgie, Deepin, Unity, as
    `XDG_CURRENT_DESKTOP` names them) get no entry, and `dsplr restore --login`
    declines on them. The list is from what each desktop is known to ship
    (KScreen, xfsettingsd, the settings daemons' xrandr plugins), not checked on
    any of them; a desktop missing from it gets an entry that fights its own
    restore, one wrongly on it gets none. Plain `dsplr restore` and
    `dsplr watch` are not limited by it.
  - **bspwm's line** is `dsplr restore --login --yes &` in `bspwmrc` (a shell
    script), not the i3 `exec --no-startup-id` line plan 05 first gave both.
    Both lines use `dsplr` from the user's `PATH`, unlike the entry's absolute
    path.
  - **i3 and bspwm are found only by `XDG_CURRENT_DESKTOP`**, which display
    managers set from the session file's `DesktopNames`; started with `startx`
    it is usually unset, and then the entry is written and both lines shown. Not
    run under a real i3 or bspwm.
- **Mirrors without a direction are compared as groups** where the backend sets
  no `Capabilities::mirror_direction`: X11 and GNOME. On X11 that was seen on
  Xorg's dummy driver (DUMMY0 asked to mirror DUMMY1 reads back as DUMMY1
  mirroring DUMMY0). On GNOME it is from reading `gnome::state` (the first
  connector of a logical monitor is read as the source, in Mutter's order), not
  seen: whether Mutter keeps the order dsplr sends is unverified. GNOME's
  primary output is read only on that first connector, so a mirror group read
  back in another order would still show a primary difference; not handled.
- **`dsplr watch` and plain `dsplr restore` do not know a desktop that restores
  its own layout on X11**: on Plasma on X11 detection picks `Randr`, whose
  `restores_on_hotplug` is false, so `dsplr watch` runs there without `--force`
  and fights KScreen, as it would Xfce's, MATE's or Cinnamon's daemons; only
  `dsplr restore --login` and the X11 save check `login::OWN_RESTORE`. Plan 04
  said `--backend randr` warns under KDE and the watch refuses it; neither was
  ever built. Options: have the watch check `OWN_RESTORE` too on `Randr`, or
  make `restores_on_hotplug` take the desktop into account. Needs a decision.
- **The sway CLI e2e leaves two revert watchdogs behind on every run**
  (`apps/dsplr/tests/sway_e2e.rs`): two
  `dsplr restore --revert --after 60 --token …` processes in the harness's
  private runtime outlive the run, its compositor gone, so they can only fail
  when their time comes. Seen after every sway run in M7's slices and ended by
  hand (`kill` by PID, after checking their `XDG_RUNTIME_DIR` is the harness's).
  Which test leaves them, and whether it is a kept change whose watchdog is not
  ended or one the test meant to leave, is not checked. The X11 e2e collects its
  own (`x11_support::watchdogs`); sway's does not.
- **X11's watch after the server goes away is not tested**: `randr::Watch` ends
  with an error when `poll_event` finds the connection dead, and `dsplr watch`
  and the GUI connect again with their backoff, as on every backend; the X11 e2e
  has one Xvfb, which the harness owns, so no test stops a server under a watch.
  Nor has the GUI run on X11 (its e2e is sway's): its following of X11 changes
  is the same `Backend::watch`, read, not run.
- **X11 reads without probing**: `randr::x11` reads `GetScreenResourcesCurrent`,
  which returns the server's last known outputs without asking the driver. On a
  driver without hotplug detection a newly plugged monitor shows only after
  something probes (`xrandr` does, with `GetScreenResources`, which has the
  driver probe its outputs; how long that takes, and what it does to a screen,
  depends on the driver and was not measured). An option: probe only on an
  explicit refresh (the GUI's, a `dsplr list --probe`), never on every read. Not
  decided.
- **A scaled CRTC's size is wrong in `dsplr list`**: a CRTC with a transform
  (`xrandr --scale 2x2`) shows its mode's size, not the area it covers, and
  neither the CLI nor the GUI says so; the planner refuses to plan over it
  (`PlanError::Transformed`), so only applying says why. A `Layout` read from a
  desktop has no place for a note or problem about an output, so there is
  nothing to show it with; giving the snapshot one (or a read-only scale) is the
  owner's call.
- **X11 on FreeBSD is not compiled**: the RandR backend is built where crcbl-x11
  is (the `x11` cfg from `build.rs`: Linux and FreeBSD), and only Linux is built
  here or in CI. Detection off Linux tries X11 alone.
- **Adaptive sync on X11 reads as unknown**: the driver's `vrr_capable` output
  property says whether a monitor could run variable refresh, not what is set
  (that is a driver option), and `OutputState::adaptive_sync` has no read-only
  form, so the snapshot gives `None` and the planner refuses any value. Showing
  `vrr_capable` needs a place in the model for a read-only fact (the owner's
  call); not read.
- **What the RandR apply planner's tests do not reach** (`randr::plan`): Xorg's
  dummy driver runs two outputs arranged side by side, swapped, made primary,
  mirrored and turned off (`xorg_e2e.rs`); the other multi-CRTC cases (a CRTC
  search that moves an output, CRTCs running out) are checked against hand-built
  snapshots and the planner's own reading of `xrandr.c`, not against xrandr on a
  real server. The Xvfb cases were checked against `xrandr --dryrun --verbose`
  there, and the custom-mode one by sending it. The flipped-transform mapping
  (`RR_Reflect_X`) is read from the X server's `RRTransformCompute` and checked
  against wlroots' `wlr_box_transform`, not seen on a monitor. Panning is
  checked against the X server's `xf86RandR13VerifyPanningArea` as read, not on
  a server. Xvfb resets when its last client disconnects unless started with
  `-noreset`, which drops created modes; the harness needs it.

- **Where `dsplr watch` has run**: on sway (`sway_e2e.rs`: hotplug both ways, no
  loop on its own applies, a change waiting to be kept, a reconnect), on niri
  (`niri_e2e.rs`: the profile for the outputs at the start, then a clean stop; a
  nested niri has one output and no way to add another, so no hotplug), and on
  Hyprland (`hyprland_e2e.rs`, CI only, not yet run: an output created and
  removed with `hyprctl output`, each time the matching profile restored). Not
  run on KDE or GNOME, where it refuses to start without `--force` (checked on
  GNOME); each backend's `Watch` is tested in its own e2e, and the daemon uses
  nothing else from the backend that `dsplr restore` does not.
- **`dsplr watch` after a Hyprland restart**: Hyprland gives a new instance its
  own `HYPRLAND_INSTANCE_SIGNATURE`, and the watch keeps the old one from its
  environment, so detection finds no Hyprland and the watch gives up after its
  tries. Not observed; usually the session (and the watch with it) ends with
  Hyprland anyway. Finding the newest instance under `$XDG_RUNTIME_DIR/hypr/`
  would fix it.
- **The reconnect was checked by hand only**: a second headless sway killed and
  started again in the same runtime directory under a running watch, which said
  it was following the displays again (2026-11-10). No e2e covers it.
- **`dsplr watch`'s KDE and GNOME refusal**: checked once in
  `apps/dsplr/tests/mutter_e2e.rs` (`dsplr watch` exits `3` without `--force`),
  green in CI run 42776828806. KDE's refusal has no e2e (`kwin_e2e.rs` does not
  run `dsplr watch`), and neither has the same check after a reconnect (a
  restart into another desktop).
- **Only `dsplr watch` waits for a change in flight**: the revert point's
  `guarded_until_unix_ms` makes the watch hold back while a `dsplr apply`, a
  `dsplr set` or the GUI waits to be kept. The other way round is not
  coordinated: a `dsplr apply` started during a watch's restore replaces the
  watch's revert point, and the watch's keep then reports the change replaced. A
  lock every flow takes would cover both; not built, as the window is a
  restore's few seconds.
- **kanshi import: unset settings are frozen at import; owner's call**: kanshi
  keeps an output's own value for whatever a profile leaves unset (a setting not
  given, a mode without a refresh, `mode preferred`); a dsplr profile holds
  every setting, so `dsplr profile import kanshi` takes those from the output as
  it is at import time (dsplr's defaults when it is not connected, and a skipped
  profile when that leaves an enabled output without a mode), and says so per
  output. Built that way because it needs no change to the model. The
  alternative: let a profile hold "unset" and "preferred" (an `Option` per
  setting, a mode that names a size without a refresh or "preferred"), resolved
  against the live output at restore time, as kanshi resolves it at apply time.
  That follows kanshi exactly, and imports profiles for outputs not connected
  now, but changes `OutputState` and the JSON form every writer, reader and the
  apply flow use. Trade-off: values frozen at import (a monitor re-set by hand
  afterwards is restored to its old value) against a model that every backend
  and file format must then handle.
- **kanshi import, what else it does not carry over**: a profile is skipped when
  it uses `output *`, `...output`, a description glob other than a trailing `*`,
  an all-`Unknown` description, or an enabled output whose mode is left to it
  while it is not connected. `exec` commands are not run. A description
  criterion for a connected monitor with a serial becomes its EDID identity;
  otherwise it is matched against the output's reported description by whole
  leading words, which on KDE (no description, and a monitor without a serial
  named by connector) may match nothing. `include` paths are expanded for `~`,
  `$VAR` and `*`/`?` in the last component only (kanshi uses `wordexp(3)`);
  other shell syntax is refused. A relative `include` is taken against `$HOME`,
  with a note: kanshi resolves it against its own working directory, which it
  never changes (no `chdir` in kanshi 1.9.0), so whatever started it, usually
  the compositor from `$HOME`; a kanshi started elsewhere reads other files than
  the import does. Blocks may nest `kanshi::scfg::MAX_NESTING` deep (kanshi's
  config needs two), against a file that would exhaust the stack. The parser was
  checked against kanshi(5) and kanshi 1.9.0's and libscfg's source, not against
  a running kanshi.

## Open work in the Windows-owned crates

- **`dsplr set --enable` needs `--mode` on headless sway**: turned on without a
  mode, an output takes its preferred mode, else its largest at the highest
  refresh (`set::default_mode`); headless sway lists no mode for an output that
  is off, so there `--mode` is needed too. Placement is shared with the GUI
  (`Layout::place_beside_others`); the Hyprland e2e's `--no-mirror` step still
  passes `--pos`, and Hyprland putting a mirror on its source
  (`CMonitor::setMirror`, v0.57.1 source) was not observed.
- **`dsplr set` moves nothing else**: a new mode, scale or transform changes the
  output's size, and a neighbour it now overlaps or leaves a gap beside is
  refused or warned about by validation, not shifted. `Layout::normalise` and
  `snap` exist for the GUI; whether `set` should use them is open.
- **`dsplr restore --revert` by hand applies without a confirm prompt**, on
  purpose: it is the way back from a broken screen, so asking first would defeat
  it. The revert point is removed once it has been applied.
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
- **A change to the saved file (`monitors.lua`, `outputs`) made while
  `dsplr save` runs is lost**: it reads and classifies the file, then renames
  the new one over whatever is there then, without looking again. A file
  created, edited by hand or replaced by another tool in that window is
  overwritten without the check or the question it would otherwise get. When it
  asks, the second call re-reads and compares the text the user approved, so a
  change while the question waits is caught; one made during a call's own
  read-to-rename (a read, a write and an `fsync`) is not. No lock: nothing else
  that writes the file would honour one.
- **A saved rule can still win over a longer description's, across the kept and
  live groups**: `save`'s `merged` writes the kept rules (monitors not
  connected) before the live ones, and orders each group with
  `in_hyprland_order` so a `desc:` that starts another comes first. Between the
  groups only kept-before-live holds, so with `AOC Q27G4 SER12` unplugged (kept)
  and `AOC Q27G4 SER1` connected (live), SER1's rule is written last and
  Hyprland, matching `desc:` by prefix, applies it to the SER12 when it is next
  plugged in. Analysed 2026-11-09: a single order over both groups works for
  `desc:` rules (kept connector rules first, then all `desc:` rules in prefix
  order, kept before live otherwise), but a live monitor without a serial is
  written by connector (`eDP-1`) while Hyprland can still match its description
  with a `desc:` rule cut mid-word (`desc:BOE 0x0A`), so its rule has no safe
  fixed place: first lets a later mid-word rule beat it now, last lets it beat a
  monitor later plugged into that port. Proposed, not built: kept connector
  rules first; then `desc:` rules in prefix order; each live connector rule just
  after the last `desc:` rule that prefixes its monitor's description (with
  none, at the front as today). In general, every live monitor's own rule comes
  after every other rule Hyprland would match to it; whether that order can ever
  contradict itself is unchecked. Needs a rule cut mid-word or a mid-word prefix
  between monitors; dsplr and nwg-displays write full descriptions.
- **`dsplr apply` reads JSON only**: `docs/plan/07-cli.md` also names TOML
  layout files; the profile store went with JSON, and TOML for both waits on the
  owner (the profile store's decisions above).
- **Profile store gaps**: `dsplr restore` with a profile on Hyprland, and run
  from a real compositor's autostart (river, labwc, niri), have not been run;
  only headless sway has. A profile file changed between the read and the rename
  of `dsplr save --profile` is overwritten without the question, as for
  `dsplr save`.
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
  not written; needs a vendor-name table with a source (the source is a decision
  for the owner, under "Decisions waiting on the owner"). Mutter 51.1 reports
  the PNP code (read in `set_output_details_from_edid`, not seen on a real
  monitor), so it is wlroots' and Hyprland's resolved names that need mapping:
  both get make, model and serial from libdisplay-info (wlroots 0.20 and
  aquamarine 0.16 link it), whose make is a company name (`ASUSTek COMPUTER INC`
  for `AUS`, `The Linux Foundation` for `LNX`, libdisplay-info 0.3.0 run here).
  `dsplr_model::edid::Edid::identity` (for the coming `Randr` backend, and used
  by the autorandr import) gives the model and serial as libdisplay-info does,
  checked against it on built EDIDs and one real monitor, and the PNP code as
  the make; until the table exists, a profile saved on Wayland names a monitor's
  make differently from one an EDID read on X11 gives, so the two do not match.
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
- Read in v0.56.2's `applyWlrOutputConfig`: state applied through
  `wlr-output-management` is stored per monitor and overrides the matching
  rule's enabled, mode, position, transform, scale and vrr. Not so on 0.57.2 for
  a later `eval`, which moved a position the protocol had set (CI, 2026-11-10,
  `docs/plan/04-backends.md#hyprland`); a reload after a protocol apply was not
  checked.
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

- **A test X server can take the desktop's display**: an X server, or a
  compositor's Xwayland, takes the first display whose lock is missing or names
  a pid it cannot see (one in another PID namespace, or really dead), and
  unlinks that display's socket path before it binds its own: Xvfb with
  `-displayfd` and no number (Xtrans), and wlroots 0.20.2's
  `open_display_sockets` (`xwayland/sockets.c`, read in its release tarball),
  which binds them at startup even when Xwayland starts lazily; Hyprland's own
  XWayland code is reported to do the same (not read here). Run from a sandboxed
  session, the harnesses were in another PID namespace than the desktop, so
  Hyprland's live lock on `:0` looked stale, and a harness's Xvfb took `:0` and
  deleted the desktop's `/tmp/.X11-unix/X0` on exit (2026-11-12). Any headless
  sway or Hyprland with Xwayland on could have done the same. Since then:
  `headless-x11.sh` passes an explicit number from `:90` up, counts every lock
  as taken and refuses a number below `:90`; the sway, niri and Hyprland
  harnesses turn Xwayland off (`xwayland disable`, `xwayland-satellite { off }`,
  `xwayland = { enabled = false }`), KWin starts none without `--xwayland` and
  Mutter runs with `--no-x11`, and niri 26.04 skips any display with a lock
  anyway (`pick_x11_display`); and every harness sources `tests/e2e/x-guard.sh`,
  which records the sockets and locks below `:90` and fails with `!!` when any
  changed by the time the servers are up or by exit (another session's server
  there starting or stopping counts too).
- **The X guard fails a local run when another session's X server comes or
  goes**: `tests/e2e/x-guard.sh` cannot tell a server the harness disturbed from
  one another session started or stopped below `:90` while it ran (a test Xvfb
  of theirs on `:2`, say), so it fails the run with `!!` either way. By design:
  a false alarm costs a rerun, a missed hijack costs the desktop's display. CI's
  VM runs one harness at a time, so it does not see this.
- **sway numbers each new headless output afresh**: after
  `swaymsg create_output` and `swaymsg output HEADLESS-3 unplug` (sway 1.12
  supports `unplug` for headless, Wayland and X11 outputs), the next output is
  `HEADLESS-4`, not `HEADLESS-3` again, and its description's number
  (`Headless output N`) need not match its connector's. A profile for a
  hotplugged headless output names it by the description's first words
  (`Headless output`), and `sway_e2e.rs` runs `dsplr watch` on a sway of its own
  so the harness's outputs keep the names other tests use.

- **Headless sway needs one keyboard held for the whole session, or keys are
  lost** (a harness requirement, cause found by the crcbl session, 2026-11-10).
  Headless sway has no keyboard of its own, so each `wtype` call's virtual
  keyboard is the seat's only one, and `wtype` presses about 0.5 ms after the
  device appears. When sway handles that press before the client's
  `get_keyboard`, the client's `wl_keyboard.enter` lists the key as already
  held, and crcbl deliberately ignores keys held at enter; each `wtype` exit
  removes the keyboard, so every call races again. A race in `wtype`, not a
  crcbl bug. Holding one keyboard first (`wtype -s 3600000 -m shift &`, as
  pikr's e2e does) avoids it: with it, every key this slice sent arrived. Even
  then the focus flips with each `wtype` call, and a release can arrive without
  its press, so a test must not count on matched press and release. And `wtype`
  types a space between its positional arguments (`wtype j j j` sends j, space,
  j, space, j), which in dsplr turns the selected output on or off: send keys as
  one argument (`wtype jjj`) or by name (`wtype -k Tab`).
- **sway 1.12 picks an output's settings differently at startup and on a
  reload** (headless, 2026-11-10): on a reload the last setting given wins, by
  connector, identifier or `*` alike; at startup and on hotplug a `*` still
  counts in file order, but an identifier's setting beats a name's wherever each
  is. A reload also clears a setting from an earlier command for the same
  connected output, so after `output "Unknown Unknown Unknown" scale 1.5` then
  `output HEADLESS-1 scale 2` and a reload, a newly created output (same
  identifier) came up at scale 1, not 1.5. The sway e2e checks a saved file both
  by reloading and by starting a second sway on it (`sway_support::FreshSway`).

- **`Transform`'s text uses wl_output's quarter turns, not sway's**: `Rotate90`
  (wl_output 1) writes and parses as `90`, while sway calls that turn `270`. All
  eight checked on headless sway 1.12 (2026-11-10), each set by
  `transform <name>` in an included file and read back through
  wlr-output-management: sway's `90`, `270`, `flipped-90` and `flipped-270` are
  wl_output's `270`, `90`, `flipped-270` and `flipped-90`; `normal`, `180`,
  `flipped` and `flipped-180` agree. sway's IPC reports its own names.
  `Display`'s doc and `dsplr set --help` say so; the `outputs` writer writes
  sway's names (`sway::TRANSFORMS`), and the backend `sway_e2e` saves each of
  the eight and reads it back after a reload.
- **sway applies a config reload after `swaymsg reload` answers**: a read right
  after it still shows the old outputs, so the e2e polls
  (`sway_support::reload_until`). A setting the config leaves out keeps its
  runtime value across the reload, and a runtime change the config does name is
  undone by it.

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
- **Identify on GNOME by taking a name GNOME Shell trusts** (M6): Shell answers
  `ShowMonitorLabels` only for callers owning `org.gnome.Settings` and two other
  names (`DBusSenderChecker`, GNOME Shell 51.1). dsplr owning one of them would
  pass itself off as GNOME Settings and break the real one while it runs, so
  identify is unsupported on GNOME instead.
- **One `wlr` connection per backend, so the snapshot's serial guards the
  apply** (2026-11-10). Every `wlr` read, test and apply opens a connection of
  its own (`Wlr::connect`), so a configuration's serial guards the time between
  that connection reading the heads and sending it, and `cancelled` comes only
  from a change in that time. Keeping one connection, so the serial of the
  snapshot a change was made from guards its apply, would guard nothing more,
  because every apply already starts from a snapshot read just before it: the
  CLI's `apply`, `set` and `restore` read the displays and go straight on to
  test and apply (`apply::file`, `apply::onto`), and the GUI's backend thread
  reads them again for each Apply or Restore (`BackendThread::apply`), the
  user's edits having been carried onto each read the desktop reported meanwhile
  (`Editor::rebase`). That is the same window a connection per request has,
  which is how GNOME's remembered serial works out too: it is the serial of the
  read just before the apply. A change in it is caught by `cancelled` (exit
  `4`), and one after it by the read-back (step 6). Holding a connection open
  would add a reconnect path and lifetime rules for no case that differs, and a
  sway or niri e2e step could not land a change in a window of milliseconds on
  purpose.
- **Shelling out to `wlr-randr`, `kscreen-doctor`, `xrandr`, `gnome-randr`.**
  Each desktop is driven through its own interface; those tools are test
  cross-checks only.
- **Faking per-output scale on X11 with RandR CRTC transforms**
  (`xrandr --scale`): blurs and costs performance; the RandR backend reports
  scale as read-only instead.
- **A writer for Hyprland's legacy hyprlang `monitors.conf`** (`monitor = …`
  lines sourced from `hyprland.conf`; the owner's call, 2026-11-09). Hyprland
  0.57.1 reads no hyprlang config at all (its `getMainConfigPath` looks for
  `hyprland.lua` only, and its one config manager reads every path as Lua), so
  the writer would serve only 0.56 and older. `dsplr save` under a hyprlang
  config stays `Unsupported` (exit `3`), saying dsplr writes only the Lua
  config's `monitors.lua`.
- **Live screen previews in the canvas** (wdisplays-style screencopy). Cost of a
  screencopy path for little help in arranging; revisit if asked.
