# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the project stands (2026-11-09)

M0 (scaffold) and M1 (read Hyprland, `dsplr list`, `dsplr backend`) are done. On
the owner's machine (Hyprland 0.56.2) `dsplr list --json` was compared field by
field with `hyprctl -j monitors all` and matched. Detection knows Hyprland, KDE
Plasma (`kde`, over KWin's own protocols, which reads and applies; built at M5,
its e2e not yet run, below), GNOME (`gnome`, over Mutter's `DisplayConfig` on
the session bus, which reads, tests, applies and saves through Mutter; built at
M6, its e2e not yet run, below) and wlroots compositors (`wlr`, over
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
`dsplr_persist::hyprland::save`); its e2e has not run yet (below). `dsplr set`
edits one output of a fresh snapshot and hands the layout to the same flow
(`apply::run`); its steps run in `apps/dsplr/tests/sway_e2e.rs`, and a mirror
step and a `--no-mirror` step in `hyprland_e2e.rs` (not run yet, as below).
`dsplr save` on sway writes `outputs` beside sway's config
(`dsplr_persist::sway::save`); both sway e2e tests save, change the outputs at
runtime, reload and read the saved layout back, and start a second sway on the
saved file, green on headless sway 1.12 locally (2026-11-10), each alone and
both in one harness in either order. dsplr's profile store
(`dsplr_persist::profile`; `dsplr save --profile`, `dsplr restore [NAME]`,
`dsplr profile list|show|rm`) is written, and `apps/dsplr/tests/sway_e2e.rs`
saves, moves, restores (picked and by name) and removes profiles on headless
sway 1.12 (2026-11-10). Saving under a hyprlang config is declined (below).
`wlr` works on niri 26.04, and `dsplr save` on niri writes `outputs.kdl` beside
niri's config (`dsplr_persist::niri::save`); both niri e2e tests
(`tests/e2e/headless-niri.sh`, niri nested in a headless sway) are green locally
(2026-11-10), each alone and both in one harness in either order. The CI job is
added and has not run yet.

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

## Open work in the Linux-owned crates

- **The KDE e2e has not run anywhere** (`crates/dsplr-backend/tests/kwin_e2e.rs`
  and `apps/dsplr/tests/kwin_e2e.rs`, under `tests/e2e/headless-kwin.sh`): the
  owner's machine has no `kwin_wayland`, so their first run is CI's `kwin`
  entry. Everything the `kde` backend does against a live KWin is unchecked
  until then; what it rests on was read in KWin 6.8.2's source
  (`docs/plan/04-backends.md#kde-plasma-6-kde`), not observed. The assumptions
  most likely to fail first: that `--virtual --output-count 3` gives three
  outputs named `Virtual-*` with one 1920x1080@60 mode each and one of them
  primary; that KWin offers its globals soon after its socket appears (the tests
  retry detection for `KWIN_WAIT`); that a mirror of a virtual output reads back
  as one; that `failure_reason` is English with no translations installed
  ("negative"); that a priority change alone emits a new output order; and that
  `kwin_wayland --version` prints without a session (the harness gives it a time
  limit). The GUI e2e was not run either.
- **The GNOME e2e is green in CI, but not since the review fixes**
  (`crates/dsplr-backend/tests/mutter_e2e.rs` and
  `apps/dsplr/tests/mutter_e2e.rs`, under `tests/e2e/headless-mutter.sh`): its
  first run, CI's `mutter` entry, passed against Mutter 51 (the backend's five
  tests and the CLI's one). The owner's machine has no `mutter`, so it runs only
  there. Not run yet: what M6's review changed, which the next CI run is the
  first to check: detection requiring the Wayland socket's process to own
  `DisplayConfig` (the harness's Mutter serves both), the serial of the last
  snapshot making a later test or apply stale, the CLI moving an offset layout
  to the origin (`moved the layout by 0,-10`), and
  `dsplr set Meta-0 --disable --yes` turning off the primary output at the
  origin, the others moved up to it and the next taking the primary. The D-Bus
  side of each is unit-tested against a stand-in `DisplayConfig` on a private
  `dbus-daemon` (`gnome::tests`).
- **Moving a layout to the origin moves the outputs that are off too**
  (`Layout::normalise` moves disabled outputs and mirrors by the same amount,
  plan 03), so on GNOME an edit can change only where a disabled output sits,
  which shows nowhere and still makes an undo step and a pending change that
  applies as nothing. Leaving disabled outputs where they are would need a
  normalise that skips them; not done.
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
- **`dsplr --backend` is not written**: plan 04 names it for desktops offering
  more than one interface (GNOME on X11, KDE on X11); detection's order picks
  for now.
- **KDE custom modes**: KWin 6.8 takes them as a mode list per output
  (`create_mode_list`, `set_resolution`, `set_refresh_rate`, `add_mode`, then
  `set_custom_modes` on the configuration, from management version 18), which
  adds the modes rather than switching to one: the apply would have to send the
  list, then pick the new mode object in a second configuration once the device
  announces it. Until then `Capabilities::custom_modes` is off on KDE and a mode
  the output does not list is refused before sending. Virtual outputs have the
  `custom_modes` capability, so the e2e could cover it.
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
- **Hyprland's change stream misses rule changes; owner's call**: the GUI
  follows Hyprland through `.socket2.sock` (`hyprland::Events`: monitor added or
  removed, config reloaded), as asked for M3, which carries no event for a
  monitor rule changed with `hyprctl eval` / `keyword` by another program, so
  the window shows such a change only on Refresh or after its own next request.
  Plan 04 meant `wlr-output-management`'s `done` events instead, which see any
  change; that is a `wlr::Connection` held on Hyprland for watching only
  (`Watch::Wlr` already does it on wlroots), at the cost of a second connection
  and Hyprland's protocol support becoming a dependency of the GUI. Options:
  keep socket2, switch to wlr, or both. Only tested against a fake socket
  (`hyprland::events` tests); never run on a real or headless Hyprland.
- **Each `wlr` request still opens its own connection**: `Watch::Wlr` holds one
  `wlr::Connection` for following changes, but every read, test and apply opens
  its own (`Wlr::connect`) and closes it, so the serial guards only the instant
  between reading the heads and sending the configuration, and a change made
  after `dsplr list` is caught by the re-read (step 6), not by `cancelled`.
- **`wlr` on other compositors**: sway 1.12 (headless) and niri 26.04 (nested)
  have been run. river, labwc and Wayfire are expected to work through the same
  protocol and are not checked; COSMIC's own extension and whether it saves
  outputs itself are still M4's to check (`docs/plan/04-backends.md`).
- **The save e2e has not run.**
  `a_saved_layout_is_what_hyprland_reads_back_after_a_reload`
  (`crates/dsplr-backend/tests/hyprland_e2e.rs`) and
  `dsplr_save_writes_monitors_lua_and_asks_before_replacing_one`
  (`apps/dsplr/tests/hyprland_e2e.rs`) compile but have run nowhere (its
  terminal helpers now live in `tests/e2e/support.rs`, shared with the sway
  test, whose save prompt runs locally): the harness needs `kwin_wayland`, which
  the owner's machine lacks, so their first run is CI's; run them with
  `--test-threads=1`, since the save test's reload drops the other test's rules.
  The harness now starts Hyprland without `--config`, finding
  `hypr/hyprland.lua` in a private `XDG_CONFIG_HOME` the tests share, and gives
  KWin, Hyprland and the tests a private `HOME` (`CARGO_HOME` and `RUSTUP_HOME`
  pinned first; cargo under such a `HOME` was checked locally, KWin under one
  was not); that lookup, and `require("monitors")` resolving beside it, were
  checked with `Hyprland --verify-config` 0.57.1 (it reports a broken or missing
  `monitors.lua`), not in a running Hyprland.
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
- **The rest of the GUI (M3)**: reaching the panel's widgets from the keyboard
  (Tab selects outputs; the widgets take the pointer, and keys only once a field
  or list is engaged), and a place for the backend thread's messages beyond the
  last (the bar shows one line, so `dsplr save`'s include note can hide its
  "saved to", and a dropped-edits note can be replaced by the next message).
  Listed in `docs/plan/06-gui.md` ("Still to build in M3").
- **Caps Lock turns `h` into `H`, and `C-r` into `C-R`**: keys are matched by
  the keysym crcbl-shell reports, which is the one for the current xkb state
  (`Xkb::keysym`, `xkb_state_key_get_one_sym`), Caps Lock included; crcbl 0.18.0
  exposes no keysym for the level without Caps Lock. So with Caps Lock on, the
  nudge keys move to the neighbour and redo does nothing. Guessing by swapping a
  letter's case when `Modifiers::CAPS_LOCK` is set would be wrong for layouts
  whose Caps Lock is a shift lock or does not touch some keys. To ask of crcbl:
  the keysym without Caps Lock (xkbcommon's `xkb_state_key_get_one_sym` with the
  Lock modifier consumed, or the level-0/1 keysym with Shift alone), then match
  characters on it in `dsplr-gui`'s `keys::matches`.
- **The change stream is not reopened once it ends**: the backend thread
  (`apps/dsplr/src/gui.rs`) says "stopped following the displays' changes" and
  drops its `Watch`; Refresh and every request still read the displays. It ends
  only when the compositor's socket closes (the compositor gone) or on a
  protocol error a new connection would likely repeat; plan 02 says why that is
  acceptable. Reopening with a backoff is the change if a desktop turns out to
  close it in normal use. Not seen: no run ended a watch.
- **Key config limits**: `[keys]` takes Ctrl, Alt and Shift, not Super, and a
  key name from `dsplr_config::keys::NAMES` or F1 to F12 (no arrows' keypad
  forms, no Insert); the dialogs', quit prompt's and engaged panel field's keys
  are fixed. A key bound to two actions is refused rather than one winning.
  Character keys match the keysym's character, so they follow the keyboard
  layout; Shift is never compared for them (`H` is Shift+h).
- **Identify where there are no panels**: the window reports "identify is
  unsupported here" in the bar where crcbl's shell lacks `ShellCaps::PANEL`,
  GNOME included (GNOME Shell's `ShowMonitorLabels` refuses other callers, plan
  06). That path has no test: `MainWindow::identify` needs the window's GPU,
  which the window's unit tests do not have. A monitor that matches no output
  shows the window system's monitor name instead.
- **The GUI's Replace is `dsplr save --force`**: the CLI asks a yes for a file
  someone else wrote and needs `--force` for one edited by hand or holding what
  dsplr cannot keep; the window shows why and the diff in every case and takes
  Replace (Enter) for all of them, since the user has seen what is lost. Say if
  the hand-edited case should take a second, stronger answer.
- **The confirm dialogs and identify panels match monitors to outputs by logical
  position** (`dsplr_core::dialog::match_screens`, within a pixel), since crcbl
  0.18.0 reports no connector name (C3, below). Read in crcbl-shell 0.18.0's
  `wayland/mod.rs` (`info`): a monitor's position is its `xdg_output` logical
  position times its scale where the compositor has `xdg_output`, else
  `wl_output.geometry`'s logical position as it is, without saying which, so
  `panels::screen` offers both (the first divided by the scale), first first;
  its size is the current mode, untransformed, so a turned output's logical size
  is not it, and size is not compared. Only the `xdg_output` case has run
  (headless sway). Two outputs at one place (a mirror) name the first; a monitor
  matching none still gets its dialog, unnamed. Switching to the connector is
  `panels::screen` filling `Screen::connector` once released.
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
- **The panel shows and does not edit** the primary output, Hyprland's colour
  preset and bit depth, and KDE's overscan and RGB range.
- **Panel keys are mapped by hand** (`dsplr-gui`'s `keys::nav`: Enter, Escape,
  the arrows, Tab) where crcbl's docs pair `TextPump` with `crcbl-input`'s
  action map. `crcbl-input` is not on plan 02's dependency list, so adding it is
  the owner's call.
- **A gap is reported on the outputs not reached from the first one**, so after
  moving the first output away its neighbour gets the warning outline, and the
  moved one only its pending one (seen 2026-11-10 in headless sway).
  `Layout::validate`'s rule, not the canvas's; marking every output of a
  disconnected group on both sides would need it to say which group is which.
- **The GUI e2e test is not in CI** (`apps/dsplr/tests/gui_e2e.rs`, ignored): it
  needs Mesa's lavapipe (`vulkan-swrast`) and `wtype` in the sway VM's packages,
  and a CI entry running it with
  `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json`. Green locally under
  `tests/e2e/headless-sway.sh` (2026-11-10): kept with Enter, timed out, killed
  mid-confirm then reverted by the watchdog, and an output plugged in with
  `swaymsg create_output` while the window is open, then reached with Tab,
  edited and applied together with an edit made before it came, each read back
  with `dsplr list --json`. The hotplug step fails with the backend thread's
  watch switched off. One run with the watch switched off failed earlier, at the
  first step's "kept", which does not use the watch: the scale applied and then
  reverted, the Enter apparently lost before the dialog took the keyboard. Not
  reproduced since; if it recurs, it is the harness's timing. Seen in `grim`
  screenshots the same day: an output created and one disabled from outside
  appearing in the window, and `i` showing each output's name and description on
  it, closed after `IDENTIFY_TIME`. Run by hand with `grim` screenshots the same
  day: the dialog on both outputs, each named for its output; Save replacing a
  foreign `outputs` file after its question (Enter); and the editing keys of the
  last slice. Not driven by anything, since each needs a click (no pointer
  injector, `wlrctl` or `ydotool`, is installed): dragging, the panel's
  drop-downs, checkbox, number and text fields (custom mode, profile name), the
  profile picker and Restore, the Revert and Refresh buttons, the dialogs' own
  Keep and Revert buttons, the tooltips, and the pointer position from a
  pointer-enter. They are covered by `dsplr-core`'s editor, hit-region, flow and
  dialog tests, a build of the panel's tree, and the window's key and request
  tests against crcbl's headless shell; Revert and profile restore run the CLI's
  own `restore::revert` and `apply::bind`. Not covered by any test: a draft
  committed to the output it was typed for when a canvas click moves the
  selection (`follow_selection` is tested for the reset, `Drafts::commits` on
  its own), a panel click ignored behind the quit prompt or the dialogs, and two
  navigation keys in one batch each getting a frame. Not run on niri, Hyprland
  or X11, and the dialogs' X11 override-redirect panels never.
- **Connector names from crcbl (C3 in `docs/plan/06-gui.md`)**: on crcbl's main
  since 2026-11-10 (3b679fed), not in a release yet (the next one is the owner's
  call; it is not in 0.18.0, which dsplr now builds on).
  `MonitorInfo::connector: Option<String>` is `Some` from `wl_output.name`, else
  (below `wl_output` v4) `zxdg_output_v1.name`, and on X11 the RandR output
  name; `None` for crcbl's made-up names (`wl-output-N`, `output-N`, `CRTC-N`,
  `screen` without RandR) and on Win32, AppKit, web and headless. A late
  `wl_output.name` republishes with `MonitorsChanged`. Caveats for matching
  against wlr-output-management's `head.name`: Xvfb's real RandR output is named
  `screen`, and RandR 1.0 emulation reports `default`, which no head matches. It
  is a new public field, so `MonitorInfo { .. }` literals need it. Needed for
  the identify overlay and the per-output confirm dialogs.
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
- **e2e for X11**: M7 adds a `headless-x11.sh` harness and a CI matrix entry, as
  `tests/e2e/headless-mutter.sh` and the `mutter` entry did for `gnome`.

## Open work in the Windows-owned crates

- **`dsplr set --enable` does not place the output**: a desktop that reports a
  disabled output at the origin (wlroots, Hyprland) gets it back there, so
  validation refuses the overlap unless `--pos` is given. Placing it beside the
  others (the right edge of the layout, say) would guess where the user wants
  it; not done. The GUI's `space` makes that guess (right of the others, level
  with the topmost, only when it would overlap one: `Editor::set_enabled`);
  whether `set --enable` should share it is open. Turned on without a mode, an
  output takes its preferred mode, else its largest at the highest refresh
  (`set::default_mode`); headless sway lists no mode for a disabled output, so
  there `--mode` is needed too. `--no-mirror` does not place the output either:
  Hyprland moves a mirror onto its source (`CMonitor::setMirror` sets its
  position to the source's, v0.57.1 source, not observed), so ending one without
  `--pos` is refused as an overlap (exit `3`). The Hyprland e2e's `--no-mirror`
  step passes `--pos`.
- **`dsplr set` moves nothing else**: a new mode, scale or transform changes the
  output's size, and a neighbour it now overlaps or leaves a gap beside is
  refused or warned about by validation, not shifted. `Layout::normalise` and
  `snap` exist for the GUI; whether `set` should use them is open.
- **Adaptive sync is checked against `Capabilities` only by `dsplr set`**:
  `Layout::validate` does not look at `adaptive_sync`, so a layout file asking
  for a value the backend lacks reaches the backend: on `wlr` its `Plan` error
  refuses it at the test step, before the revert point (exit `3`, as `sway_e2e`
  checks), but the GUI's validation would not show it. Moving the check into
  validation would cover both.
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
  only headless sway has. The X11 autostart entry and the line shown to i3 and
  bspwm users are M7's. A profile file changed between the read and the rename
  of `dsplr save --profile` is overwritten without the question, as for
  `dsplr save`.
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
  not written; needs a vendor-name table with a source. Mutter 51.1 reports the
  PNP code (read in `set_output_details_from_edid`, not seen on a real monitor),
  so it is wlroots' and Hyprland's resolved names that need mapping.

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
- KDE's `kde_output_management_v2` has no test request, and the priority order
  is `kde_output_order_v1`. Rechecked at M5 against `plasma-wayland-protocols`
  v1.23.0 and KWin 6.8.2's source: its `position` request's "no gaps or
  overlaps" is not enforced by KWin, and KWin 6.8 lists devices only through
  `kde_output_device_registry_v2` (`docs/plan/04-backends.md`).
- Mutter's `ApplyMonitorsConfig`: `temporary` never reverts; `persistent` starts
  GNOME Shell's confirm and saves only when kept; the timeout is
  `DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`; stale serials and a policy switch
  refuse the call (`src/backends/meta-monitor-manager.c`, main).
- Mutter offers no `wlr-layer-shell` (`src/meson.build`'s protocol list).
- clap exits with `USAGE_CODE` 2 on usage errors (`clap_builder` 4.6.7).
- `wlr-output-management`'s `set_scale` takes a `wl_fixed` the compositor may
  round.

crcbl's tree was read at source level for the needs in `docs/plan/06-gui.md`;
the main window is built on crcbl 0.18.0 (C1 on Wayland, the canvas of C6), and
the table there says which needs are confirmed.

## Not verified (stated as gaps)

Reported correct by the plan review but not rechecked by hand: sway's
`adaptive_sync` taking effect (a headless output accepts it and stays off), and
the `hl.monitor` field names (Hyprland wiki). sway's `include` was checked on
headless sway 1.12 (2026-11-10): a relative path, `~`, `$HOME`,
`$XDG_CONFIG_HOME` and an absolute path load; a quoted absolute path with a
space in it does not.

Read in KWin 6.8.2's source but not yet observed: that KWin saves
`kwinoutputconfig.json` on every successful protocol apply
(`Workspace::applyOutputConfiguration` calls
`OutputConfigurationStore::storeConfig`); the KDE e2e checks it, and has not
run. Read in Mutter 51.1's source at M6 and not yet observed: the
`ApplyMonitorsConfig` method values, `layout-mode`, the vendor being the PNP
code, and the rest of `docs/plan/04-backends.md#gnome-gnome`; the GNOME e2e
checks most of it, and has not run. Read in GNOME Shell 51.1's source:
`ShowMonitorLabels` refuses callers that do not own one of its allowed names.
Not verified by anyone: COSMIC's protocol offer and whether it persists outputs
itself; the Xorg dummy driver giving several RandR outputs; whether an `eval` of
`hl.monitor` survives a protocol apply; KDE's confirm timeout; the RandR 1.5
details. Each is checked in its backend's milestone.

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
