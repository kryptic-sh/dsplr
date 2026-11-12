# 04 — Backends

## Detection

At start `dsplr-backend` picks one backend, first match wins:

| Order | Condition                                                                | Backend    |
| ----- | ------------------------------------------------------------------------ | ---------- |
| 1     | `HYPRLAND_INSTANCE_SIGNATURE` set and its socket answers                 | `Hyprland` |
| 2     | `WAYLAND_DISPLAY` set and the registry offers `kde_output_management_v2` | `Kde`      |
| 3     | `org.gnome.Mutter.DisplayConfig` owned on the session bus                | `Gnome`    |
| 4     | `WAYLAND_DISPLAY` set and the registry offers `zwlr_output_manager_v1`   | `Wlr`      |
| 5     | `DISPLAY` set, `WAYLAND_DISPLAY` not, and the server has RandR 1.3+      | `Randr`    |

When `HYPRLAND_INSTANCE_SIGNATURE` is set but its socket cannot be found or does
not answer, detection stops at row 1 with no backend rather than going on: the
session is Hyprland, which offers `zwlr_output_manager_v1` too, and driving it
as `Wlr` would test with its stub and leave protocol-applied state, which
Hyprland keeps per monitor, beside its monitor rules (on 0.57.2 a later `eval`
rule wins over it, and a protocol apply keeps an `eval` mirror;
[Hyprland](#hyprland)).

Detection never trusts `XDG_CURRENT_DESKTOP` alone: a variable says what the
session claims to be, the registry and the bus say what is there.
`dsplr --backend <name>` overrides it, for testing and for desktops that offer
more than one interface (KDE on X11 is `Randr`; GNOME on X11 offers both
`DisplayConfig` and RandR, and `Gnome` wins because it persists, which the order
above gives: Mutter 51.1 has no X11 backend, so this is an older GNOME; which
release dropped it was not checked). Only the backend named is checked, never
another in its place ([07](07-cli.md)). Row 5 is not tried under a Wayland
session: `DISPLAY` there is the compositor's Xwayland, whose RandR shows its
outputs and sets nothing on them; `--backend randr` (or `x11`) tries it anyway.
RandR 1.3 is what dsplr's reads use (`GetScreenResourcesCurrent`,
`GetOutputPrimary`, `GetPanning`, `GetCrtcTransform`); nothing of 1.4 or 1.5 is
needed, so it asks no more.

No match is an error that lists what was checked and what was found, never an
empty canvas.

## The seam

```rust
pub enum Backend { Hyprland(..), Kde(..), Wlr(..), Gnome(..), Randr(..) }

impl Backend {
    pub fn capabilities(&self) -> Capabilities;
    pub fn snapshot(&mut self) -> Result<Snapshot, BackendError>;
    pub fn test(&mut self, layout: &Layout) -> Result<(), BackendError>;
    pub fn apply(&mut self, layout: &Layout) -> Result<(), BackendError>;
    pub fn persist(&self, layout: &Layout, approved: Option<&str>) -> Result<Persisted, BackendError>;
    pub async fn watch(&self) -> Result<Watch, Error>; // the change stream
}
```

`test` on a backend with no test primitive returns `Unsupported`, and the apply
flow skips step 2 rather than pretending it ran
([02](02-architecture.md#apply-confirm-revert)).

`persist` writes nothing a file there would lose without asking: a file dsplr
may not simply overwrite is a `Conflict` holding its text and the new one, and
the caller asks the user, then calls `persist` again with that text as
`approved` ([05](05-persistence-and-profiles.md#rules-for-files-dsplr-writes)).

## Hyprland

Verified on the owner's machine (2026-11-04, Hyprland 0.56.2, Lua config):

- The compositor offers `zwlr_output_manager_v1` at **version 4** (adaptive sync
  in the protocol).
- `hyprctl keyword monitor …` is refused: "keyword can't work with non-legacy
  parsers. Use eval." Under the Lua config, runtime changes go through `eval`.
- `hyprctl monitors all -j` reports, per output: name, description, make, model,
  serial, physical size, current mode and position, scale, transform, `vrr`,
  `disabled`, `mirrorOf`, `availableModes` (as `"WxH@R.RRHz"` strings),
  `currentFormat`, and colour-management fields (`colorManagementPreset`,
  `sdrBrightness`, `sdrSaturation`, …).

So:

Read in Hyprland's `src/protocols/OutputManagement.cpp` (main, 2026-11-04):

- **`test` is a stub** that logs "TODO: STUB: applyTestConfiguration for test
  not implemented, returning true." and always succeeds. dsplr treats Hyprland
  as having no test step ([02](02-architecture.md#apply-confirm-revert)).
- A protocol apply stores the new state per monitor name in memory
  (`m_monitorStates`), schedules a monitor-rule reload, and sends `succeeded`
  after it. On 0.57.2 a later `eval` rule still wins over that state (below).

So:

- **Read** from `j/monitors all` on `.socket.sock` (the bytes `hyprctl -j`
  sends) for the fields only Hyprland reports. **Changes** come from the
  `wlr-output-management` connection dsplr already holds: its `head` events and
  `done` fire for any mode, scale or position change, made by any tool, which
  `monitoraddedv2` / `monitorremovedv2` / `configreloaded` on `.socket2.sock`
  would miss. Each `done` triggers a fresh `j/monitors all`. As built (M3): the
  Hyprland backend holds no `wlr-output-management` connection (it applies
  through `eval`), so its change stream is `.socket2.sock`'s monitor and reload
  events (`hyprland::Events`), and a rule changed by another program through
  `eval` or `keyword` is not followed; the backlog has the choice. Hyprland
  0.57.2 sends no `.socket2.sock` event for a monitor rule set through `eval`
  (CI run 42763145431, 2026-11-10, asserted by the e2e's
  `a_watch_follows_reloads_and_hotplug`), so the watch sees neither dsplr's own
  applies nor another program's `eval`; dsplr's own are read back by the apply
  flow (step 6), another program's only on Refresh or after the window's next
  request. Reloads and outputs added or removed are reported.
- **Apply** through `wlr-output-management` (shared code with `Wlr`): atomic
  apply and adaptive sync, but no real test. Fields the protocol cannot express
  (mirroring, bit depth, colour-management preset) go through
  `eval hl.monitor({ … })` on the socket. Whether `eval` of a `hl.monitor` table
  applies immediately, and whether it composes with a protocol apply, is an **M2
  exit check**: the protocol requires every head in a configuration and Hyprland
  reports no mirrored state through it, so a protocol apply may undo an `eval`
  mirror. **Checked** on Hyprland 0.57.2 (CI run 42763145431, 2026-11-10) by the
  last step of `crates/dsplr-backend/tests/hyprland_e2e.rs`
  (`an_eval_mirror_meets_a_protocol_apply`), which speaks the protocol through
  the `wlr` backend on Hyprland's own Wayland socket and now asserts both:
  - A protocol apply **keeps** a mirror set through `eval`. The protocol makes
    every client send every head, the mirroring one included, so this is
    Hyprland's doing, not dsplr's: any protocol client (wlr-randr, kanshi, a
    `Wlr` apply) leaves an `eval` mirror in place.
  - A later `eval` **wins** over a position the protocol set: the protocol's
    state does not hold over Hyprland's rules until restart, which is what
    reading `applyWlrOutputConfig` (v0.56.2) had suggested.

  So `eval` and protocol applies compose, the later one deciding. dsplr's
  Hyprland backend applies everything through `eval` and needs no protocol
  apply; mirroring through `eval` stays writable (it is in `Capabilities`), and
  another tool's protocol apply does not undo it.

- **Adaptive sync** is set through each rule's `vrr` (`0` off, `1` on, `2`
  fullscreen only, which dsplr calls automatic), so `Capabilities` lists all
  three. The snapshot cannot read the rule back (`vrr` in `j/monitors all` is
  whether it is active now), so it reads as not known, which the read-back does
  not compare and the panel shows as `unknown` until set. The e2e checks
  Hyprland takes each rule on a headless output, not that VRR changed.
- **Colour preset and bit depth** (0.57.2, CI run 42784568478, a headless output
  without an EDID): `j/monitors all` reports `colorManagementPreset`, by the
  name a `cm` rule takes, and `currentFormat` (`XRGB8888`, `XRGB2101010`;
  `Invalid` while off). `eval` accepts `cm` = `srgb`, `wide`, `edid`, `hdr`,
  `hdredid`, `auto`, `dcip3`, `dp3` and `adobe` (`dsplr_model::COLOUR_PRESETS`);
  the JSON names `srgb`, `wide`, `dcip3`, `dp3` and `adobe` as given, and
  `edid`, `hdr`, `hdredid` and `auto` as `srgb` there, where the output cannot
  do them. `bitdepth = 10` gives a 2101010 format, a later rule without
  `bitdepth` keeps it (the merge), `bitdepth = 8` turns it off, and `16` reads
  as 8. So the snapshot reads the preset in force, and 10 bits for a 2101010 or
  1010102 format, 8 for an 8888 one, and nothing for any other (a half-float
  format), which is then neither compared nor written. An apply states the
  preset and depth only for an output whose change makes them differ from what
  Hyprland reads (`hyprland::colour_as_changed`, one read first), 8 and `srgb`
  included: the rest are left to the rule in force by the merge, so a move does
  not pin `cm = "auto"` to the preset it resolved to. A save leaves Hyprland's
  defaults (`srgb`, 8) out of `monitors.lua` except for an output a
  `hyprland.lua` rule could name
  ([05](05-persistence-and-profiles.md#hyprland)). A preset the user asks for
  that falls back reads back as `srgb`, which the read-back reports as a
  difference (asked `hdr`, got `srgb`): it is what Hyprland shows, and the
  differences are said, not a failure, so the user learns the monitor did not
  take it. Not seen on a monitor that can do HDR or 10 bits.
- **Persist** by writing the `monitors.lua` beside the `hyprland.lua` Hyprland
  reads, found as Hyprland finds it (`HYPRLAND_CONFIG`, then
  `$XDG_CONFIG_HOME/hypr`, then `~/.config/hypr`;
  [05](05-persistence-and-profiles.md#hyprland)); the owner's `hyprland.lua`
  already `require`s it. Legacy hyprlang configs (`hyprland.conf` + `monitor=`
  lines) are detected, and `persist` there is `Unsupported`: Hyprland 0.57 reads
  only Lua configs, so no hyprlang writer is planned.

## wlroots compositors (`Wlr`)

`zwlr_output_manager_v1`: heads with their modes; a configuration object with
`enable_head` / `disable_head`, `set_mode` / `set_custom_mode`, `set_position`,
`set_transform`, `set_scale`, `set_adaptive_sync` (v4), then `test()` or
`apply()`, answered by `succeeded` / `failed` / `cancelled`. Every change event
ends with `done` carrying a serial; a configuration created against a stale
serial is `cancelled`, which dsplr answers by re-reading and showing the new
state, never by retrying blindly.

`set_scale` takes a `wl_fixed`, and the protocol says the compositor may round
it, so the applied scale is read back from the next `done`.

Covers sway, niri, river, labwc, Wayfire, Hyprland (live path) and any other
compositor that offers the protocol. **COSMIC** is expected to offer it plus its
own extension (`cosmic-output-management-unstable-v1`, used by `cosmic-randr`)
and probably saves outputs itself, which would make it a desktop that persists
on its own rather than one on dsplr's store; to verify in M4. Persistence
differs per compositor ([05](05-persistence-and-profiles.md)): `Wlr` takes the
compositor for sway when the socket `SWAYSOCK` names answers sway's IPC, and
then writes sway's `outputs` ([05](05-persistence-and-profiles.md#sway)); for
niri when the socket `NIRI_SOCKET` names answers niri's, and then writes niri's
`outputs.kdl` ([05](05-persistence-and-profiles.md#niri)). Each IPC socket
counts only when the process listening on it also listens on the Wayland socket
(their `SO_PEERCRED` pids, through tokio's `UnixStream::peer_cred`), so a
compositor nested in sway or niri, which inherits `SWAYSOCK` or `NIRI_SOCKET`,
is not taken for it. On any other compositor it is `Unsupported`, pointing at
dsplr's profile store
([05](05-persistence-and-profiles.md#dsplrs-profile-store)).

**niri 26.04** (nested in a headless sway, 2026-11-10) reads, tests and applies
through the protocol (version 4), with two differences from sway:

- An `apply` that changes the outputs is answered `succeeded` and then
  `cancelled` on the same configuration (seen with `WAYLAND_DEBUG=1`): niri's
  change notice cancels every configuration the client still holds. The first
  answer is the configuration's, so dsplr takes the first and ignores the rest.
- Its `test` checks only that the serial is current and that some output stays
  on (`FIXME: actually test the configuration with TTY` in
  `src/protocols/output_management.rs`, v26.04 source); a listed mode, a
  positive scale and a custom mode's refresh are checked as each request
  arrives. A nested niri's test passed a 99999x99999 custom mode.

Mirroring is not in the protocol; on `Wlr` it is `Unsupported` unless a
compositor-specific path is added later.

## KDE Plasma 6 (`Kde`)

Built in M5 (`crates/dsplr-backend/src/kde/`). KWin's own Wayland protocols,
from KDE's `plasma-wayland-protocols` at tag `v1.23.0`, vendored in
`crates/dsplr-backend/protocols/` (origin and licence, MIT-CMU, in its
`build.rs`). KWin 6.8.2, Arch's package on 2026-11-10, builds against that
release (its `CMakeLists.txt` asks for 1.23.0). Read in KWin 6.8.2's source:

- `kde_output_device_v2` (v26) is no longer a global: since protocol version 21
  each output device arrives as an `output` event of the
  `kde_output_device_registry_v2` global, which KWin refuses to bind below
  version 21 (`unsupported_version`, in `src/wayland/outputdevice_v2.cpp`). The
  `wayland-protocols-plasma` 0.3.12 files the plan first read (device v11,
  management v12, devices as globals) cannot talk to it. So dsplr needs the
  registry at version 21 or later, and refuses an older KWin with that reason; a
  KWin from before the registry is not supported (backlog).
- Each device sends its modes (refresh in mHz), geometry (position, physical
  size, make, model, transform), scale, name (the connector), serial, uuid,
  capability flags, overscan, VRR policy, RGB range, replication source,
  priority and more, then `done`; later changes end with a `done` sent on KWin's
  next event-loop turn, which can come after it answered an apply.
- `kde_output_order_v1` (v1): the output names in priority order; the first is
  the primary output. Set with the configuration's `set_priority`.
- `kde_output_management_v2` (v22): a configuration object with per-device
  setters and `apply`, answered `applied` or `failure_reason` then `failed`.
  **No test request.** It has no serial: an output added or removed while a
  configuration is being made fails it ("One of the relevant outputs is no
  longer available"). Before applying, KWin refuses an enabled output at a
  negative position, a position past 1,000,000, a mirror of itself and an output
  turned off that lacks the `disable` capability
  (`src/wayland/outputmanagement_v2.cpp`, `src/workspace.cpp`). It does **not**
  check the "no gaps or overlaps" its `position` request describes; dsplr's
  validation still refuses gaps on KDE (`gaps_allowed` off).
- KWin keeps scales in 120ths, ignores an overscan above 100 and a VRR policy or
  RGB range it does not define, and accepts `set_primary_output` but ignores it.

As built:

- **Read**: every device and the output order, on a connection of its own.
  Adaptive sync (the VRR policy: never, always, automatic), overscan and RGB
  range are read only for an output whose capabilities include them, and a
  disabled output has none of them and no mode, as on wlroots. A mirror's source
  is the device whose uuid its replication source names, while that one is on
  (KWin's `applyMirroring` in `src/outputconfigurationstore.cpp`).
- **Test**: the layout is checked against the devices and refused if it cannot
  be sent; then `Unsupported`, and the apply flow skips step 2.
- **Apply**: one configuration naming every output the layout names (KWin
  changes only those): enabled, mode object, transform, position, scale, and
  overscan, VRR policy and RGB range where set; the replication source from
  management version 13 (empty for none); and, when the layout names a primary
  output, `set_priority` for every output that will be on: the primary first,
  the rest in KWin's order now. Refused before sending: colour preset, bit
  depth, a setting the output's capabilities lack, a second primary output, and
  turning off an output that says it cannot be (device version 25 on).
- **Custom modes** (management version 18 on, outputs whose capabilities include
  `custom_modes`): `set_custom_modes` gives an output a mode list
  (`create_mode_list`; `set_resolution`, `set_refresh_rate`,
  `set_reduced_blanking`, `add_mode` per mode), which adds the modes to the
  output, deleting custom ones not in the list, rather than switching to one. So
  a mode the output does not list takes two configurations: the first sends the
  list, the custom modes KWin lists for the output (by their mode `flags`; by
  their `cvt` timings through `add_cvt` from management version 22, else by
  size, refresh and reduced blanking) then the new one, so none is deleted; the
  second, once the output lists a mode it did not before at the new size (the
  one nearest the refresh asked, since KWin makes the timings and so the refresh
  itself), sets the layout at that mode object. Each output is followed by its
  uuid between the two, so one plugged in or out meanwhile shifts none. KWin not
  listing it within `REQUEST_TIMEOUT` is an error naming the output. When the
  layout is not applied (that, or KWin refusing it), the lists are sent back
  without the new mode, so a failed attempt leaves none behind; if that fails
  too, the error says the mode stays in KWin's list for the output. Every KDE
  apply is bounded by `kde::OPERATION_TIMEOUT` (five `REQUEST_TIMEOUT`s: the
  connection, the list, the new mode, the layout, the list put back), and the
  revert watchdog counts that bound in, since which apply adds a mode is known
  only once connected; so on KDE the watchdog waits longer than on wlroots for
  every change. A refresh that came out other than asked is a difference the
  read-back reports. A revert of a kept change sends the old layout, at its old
  mode, and the added mode stays listed, unused (KWin saves it with the rest).
  Read in the protocol XML of plasma-wayland-protocols v1.23.0; KWin's own
  handling is checked only by the KDE e2e, in CI.
- **Persist**: KWin saves on **every** successful apply, whoever sent it:
  `Workspace::applyOutputConfiguration` calls
  `OutputConfigurationStore::storeConfig`, which writes `kwinoutputconfig.json`
  in `QStandardPaths::ConfigLocation` (`$XDG_CONFIG_HOME`) before the protocol
  answers. So `persist` writes nothing and reports `Persisted::ByDesktop`, for
  the layout on screen only, `Capabilities` says the desktop saves on apply, and
  a change that is not kept is undone only by dsplr's revert
  ([02](02-architecture.md#apply-confirm-revert)).
- **Watch**: a connection of its own; each device's `done` and `removed` and
  each new output order is a change.
- **Capabilities**: mirroring from management version 13; adaptive sync off, on
  and automatic; fractional and per-output scale; overscan; RGB range; the
  primary output when KWin offers `kde_output_order_v1` (a primary flag on an
  output that is off is ignored, and KWin picks); custom modes from management
  version 18; native persistence and saves on apply; no test, no gaps.
- **KDE on X11** keeps KScreen's own daemon, which re-applies its saved config
  and can fight dsplr's `Randr` backend, which detection picks there. Nothing
  warns about it yet: `dsplr watch` refuses only backends that restore on
  hotplug themselves (`Backend::restores_on_hotplug`), and `Randr` does not know
  the desktop; only `dsplr restore --login` and the X11 save look at
  `XDG_CURRENT_DESKTOP` (`login::OWN_RESTORE`; backlog).

## GNOME (`Gnome`)

Built in M6 (`crates/dsplr-backend/src/gnome/`), over
`org.gnome.Mutter.DisplayConfig` on the session bus, through `zbus`. Read in
Mutter 51.1's source (Arch's package on 2026-11-10;
`data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml`,
`src/backends/meta-monitor-manager.c`, `meta-monitor-config-manager.c`,
`meta-monitor-config-utils.c`):

- `GetCurrentState()` → serial, monitors (connector, vendor, product, serial;
  modes with id, size, refresh as a double, `supported-scales` and `is-current`
  / `is-preferred`; properties such as `display-name`, `width-mm`,
  `is-underscanning`, `color-mode`, `rgb-range`), logical monitors (x, y, scale,
  transform, primary, their monitors) and properties (`layout-mode`, absent
  meaning logical; `supports-changing-layout-mode`; `global-scale-required`).
  There is no `supports-mirroring` property in 51.1.
- The vendor is the EDID's PNP code (`set_output_details_from_edid` in
  `src/backends/meta-output.c`), and Mutter sends `unknown` for a vendor,
  product or serial it does not know (`meta_monitor_generate_spec`).
- `ApplyMonitorsConfig(serial, method, logical_monitors, properties)`: method
  `0` verify, `1` temporary, `2` persistent. A wrong serial is `AccessDenied`
  "The requested configuration is based on stale information", and D-Bus
  configuration turned off by policy is `AccessDenied` too; a configuration
  Mutter cannot apply is `InvalidArgs` with its reason (none translated). Mutter
  refuses logical monitors that overlap, do not all touch ("Logical monitors not
  adjacent"), do not start at 0,0 ("positions are offset"), have no primary or
  two, a scale the mode does not list (compared within a float's epsilon), a
  fractional scale in the physical layout mode, and monitors of one logical
  monitor at different mode sizes. A monitor left out of every logical monitor
  is turned off. A monitor's `underscanning` and `color-mode` reset to their
  defaults when an apply leaves them out. `layout-mode` left out is Mutter's
  **default** mode, not the current one.
- `temporary` is not saved and never reverted by Mutter. `persistent` applies,
  emits `confirm-display-change` and starts a timeout
  (`DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`); kept, Mutter writes `monitors.xml`
  in `g_get_user_config_dir()` (asynchronously); not kept or timed out, it
  applies the configuration from before the persistent one, `temporary`. GNOME
  Shell answers the signal with its dialog; Mutter's default plugin has no
  handler, so plain `mutter` keeps it at once
  (`meta_plugin_manager_confirm_display_change`). Any `temporary` or
  `persistent` apply cancels a confirmation still waiting.
- `MonitorsChanged` is emitted after each change.
- Mutter 51.1 has no X11 backend (no `src/backends/x11/`), and its native
  backend's default layout mode is logical, with `supports-changing-layout-mode`
  set. The physical mode still comes from a `monitors.xml` that says so or a
  client that sets it.

As built:

- **Read**: one entry per monitor; a monitor in no logical monitor is off. The
  first monitor of a logical monitor is placed there; each other one mirrors it,
  with the logical monitor's position, scale and transform. The primary logical
  monitor makes its first monitor primary. Mutter's `unknown` is read as not
  reported, so a monitor without a serial is named by its connector. The
  description is `display-name`. A mode's handle is its ordinal among the
  monitor's modes, as on wlroots (a monitor can list one size and refresh twice:
  interlaced, or a variable-refresh mode). The layout carries Mutter's layout
  mode (`Layout::layout_mode`, below).
- **Test**: `verify`, a real test. **Apply**: `temporary`, so dsplr's own
  confirm and watchdog guard it as elsewhere. Both read the state and send on
  one connection, with that state's serial; and when the last snapshot read
  another serial, the displays changed since the layout was made from it, so the
  request is stale, nothing sent (exit `4` from the apply flow, which shows the
  displays as they are now), as on wlroots. An apply forgets that serial, since
  Mutter's moves on with it, and the next snapshot takes the new one. Each
  output on and mirroring nothing is a logical monitor; a mirror joins its
  source's. A monitor the layout does not name keeps its current settings. The
  mode is the one the monitor lists (no custom modes); the scale is sent as the
  mode lists it, so it reads back as sent (no `scale_steps`). With no primary
  named among the outputs that are on (an output turned off keeps its flag in a
  layout made from a snapshot), the one Mutter has stays primary while on, else
  the first logical monitor, and the read-back compares no primary
  (`Layout::differences`). A layout naming no layout mode is in Mutter's; one
  naming the other is sent with `layout-mode` wherever Mutter can change it
  (then its scales are left to Mutter, which lists them for its current mode
  only), and refused elsewhere. Underscanning and colour mode go back as read;
  RGB range as the layout says, else as read.
- **Refused before sending** (`gnome::PlanError`, exit `3`): a mode the monitor
  does not list, a scale its mode does not list, a mirror of another mode size
  or of an output that is off or a mirror, a second primary output, a layout not
  starting at 0,0 (which the editor and the apply flow move there first,
  `Capabilities::origin_required`, so only a caller of the backend skipping them
  meets it), a layout mode Mutter cannot switch to, and colour preset, bit
  depth, overscan or adaptive sync (Mutter offers variable refresh as modes of
  their own, not read yet). Gaps are refused by validation (`gaps_allowed` off).
- **Errors**: Mutter's `InvalidArgs`, and `AccessDenied` other than the stale
  one, are refusals (`Error::is_refusal`); the stale one, and a monitor gone
  since the read, are `Error::is_stale`.
- **Persist**: the layout must be the one on screen (as on niri and KDE), and is
  sent again as `persistent`; `Persisted::WhenKept` says Mutter saves it once
  GNOME Shell's dialog keeps it. How that meets dsplr's confirm flow is in
  [05](05-persistence-and-profiles.md#kde-and-gnome).
- **Watch**: `MonitorsChanged` on a bus connection of its own. Every request
  uses a connection of its own too, so a signal stream left unread never holds
  up a reply (zbus stops reading a connection while one of its streams is full).
- **Capabilities**: test before apply, mirroring, primary output, RGB range,
  fractional scale, per-output scale unless `global-scale-required`, native
  persistence, the layout at the origin; no gaps, no custom modes, no adaptive
  sync, not saved on every apply.
- **Layout modes**: a GNOME snapshot names its mode (`Layout::layout_mode`); in
  `physical` an output's extent on the canvas is its mode turned by its
  transform, whatever its scale (`OutputState::size`), which validation,
  snapping and the canvas use. The JSON form leaves out only a mode not named.
  `dsplr apply` keeps the desktop's mode unless the file names one, and a change
  of mode is a difference the confirm asks about
  ([03](03-display-model.md#types)).
- **Detection**: after KDE, before wlroots: the session bus the environment
  names (`DBUS_SESSION_BUS_ADDRESS`, else `$XDG_RUNTIME_DIR/bus`), asked whether
  something owns `org.gnome.Mutter.DisplayConfig` (`NameHasOwner`, so nothing is
  activated), then whether it is **this session's** Mutter, since systemd's user
  bus is shared by a user's sessions: under Wayland the process serving
  `WAYLAND_DISPLAY`'s socket (`SO_PEERCRED`) must be the one owning the name
  (`GetConnectionUnixProcessID`), as GNOME Shell is both, so a compositor nested
  in GNOME (which inherits its bus and `XDG_CURRENT_DESKTOP`) is not taken for
  it; with no Wayland socket (GNOME on X11) `XDG_CURRENT_DESKTOP` must name
  GNOME. Then `GetCurrentState`. Behind a Wayland proxy (waypipe, a sandbox) the
  socket's pid is the proxy's, so GNOME is not found there.
- Mutter offers no layer-shell (its Wayland protocol list in `src/meson.build`
  has no `wlr-layer-shell`), so dsplr's per-monitor dialogs fall back to one
  toplevel and identify is unsupported there ([06](06-gui.md)).

Cinnamon's Muffin exposes the same interface under
`org.cinnamon.Muffin.DisplayConfig`; whether one implementation serves both is
not checked (backlog).

## X11 (`Randr`)

RandR 1.3 or later on the X server ([detection](#detection)), for i3, bspwm,
awesome, Openbox, Xfce, and Plasma on X11, which detection finds as `Randr`;
GNOME on X11 only when chosen with `--backend randr`, as Mutter is found first.

- **Read (built, M7):** through crcbl-x11 0.20.0 (`randr::x11`): screen
  resources, the size range, the primary output, each output and its `EDID`
  property, each CRTC with its panning and its transform, the screen's size in
  pixels (`GetGeometry` on the root) and millimetres (from the connection's
  setup, a copy from connect time). Each read and apply has a connection of its
  own: the size in millimetres is only in the setup, a connection a timeout
  abandoned is never used again, and one connection's life is what the
  per-server lock covers. Each connection waits for a reply at most the
  operation's limit (`OPERATION_TIMEOUT`, `APPLY_TIMEOUT`; crcbl-x11's
  `set_reply_timeout`): a server that stops answering ends the work with
  crcbl-x11's `Error::Timeout`, naming the request, and the worker thread ends,
  releasing the lock (seen on Xvfb stopped with `SIGSTOP`, `x11_e2e.rs`). The
  work still runs on a detached thread whose result is waited for at most that
  limit too: crcbl-x11 has no deadline in connecting (its setup and RandR's
  version), in a flush the server reads nothing of, or for the rest of a reply
  cut short; a deadline per reply does not bound a whole apply, whose limit the
  revert watchdog's delay counts on; and a tokio blocking task stuck in one of
  those waits would hold up the runtime's shutdown. `randr::snapshot` reads that
  as a layout: connected outputs and any a CRTC still drives; the identity from
  the EDID (`Edid::output_id` with the RandR output name as connector), else the
  connector; each mode's refresh as xrandr's `mode_refresh` gives it; a CRTC's
  rotation and reflection as the transform the planner sets them back with;
  outputs sharing a CRTC, or CRTCs at one place, as mirrors of the first; the
  physical size; scale 1; adaptive sync none. A CRTC with a transform
  (`xrandr --scale`) has no place in the model, and the planner refuses to plan
  over it (`PlanError::Transformed`), saying how to take it off. The resources
  are `GetScreenResourcesCurrent`'s, which asks the driver nothing: a monitor
  plugged in on a driver without hotplug detection shows only once something has
  probed (`xrandr` does, with `GetScreenResources`). Run against Xvfb in
  `tests/e2e/headless-x11.sh`. The transform the planner refuses is the current
  one or, failing that, the pending one (set, in force from the next
  `SetCrtcConfig`).
- **Apply (built, M7):** `Randr::apply` reads, plans and sends on one connection
  of its own (`randr::x11::send`): the `CreateMode`s first, each
  `ModeRef::Created(n)` the XID the `n`th one answered; then, under a server
  grab, the CRTCs turned off, the screen size, each `SetCrtcConfig` (its status
  checked: `Failed` is the desktop refusing, the two time statuses stale) and
  each `SetPanning` where the plan puts it, the primary output, and the ungrab,
  which is released on every path, an error included. What the plan did unasked
  (a panning reset) comes back as lines the apply flow says. A timed-out apply
  leaves its thread and connection behind; the flow reads the server again on a
  new connection before it reverts or reports, since a `SetCrtcConfig` sent
  under the grab may still take effect; every read and apply of one server holds
  a lock for it, in this process, so that read waits for the late apply and sees
  what it did. An apply has its own limit, `APPLY_TIMEOUT` (a modeset can take
  the driver seconds), which is the backend's request timeout and so what the
  revert watchdog's delay is counted from. A stale status (the two time
  statuses, which X.Org 21.1.26 never sends for `SetCrtcConfig`) is stale only
  before anything was configured, and a refusal, which reverts, after. Created
  modes stay on their output for the server's life, as with xrandr, and a later
  apply reuses one by name and timings. Under `--backend randr` with
  `WAYLAND_DISPLAY` set (Xwayland) applying is `Error::Unsupported`, and a
  server with the `XWAYLAND` extension (`QueryExtension`), or else whose outputs
  are all `XWAYLAND<n>`, is refused before sending. An absent extension proves
  nothing (Xwayland before 23.1 has none, it is a build option, and
  `-extension XWAYLAND` turns it off), so the names stay as the fallback. A mode
  named without a handle takes the progressive one before an interlaced one at
  the same refresh (`Mode` has no interlace flag). Reverting is the apply flow's
  as on every backend: a change kept, declined, and reverted by its watchdog run
  against Xvfb (`apps/dsplr/tests/x11_e2e.rs`). Two outputs arranged, made
  primary, mirrored and turned off, and `dsplr watch` restoring a profile when a
  monitor is plugged in, run against Xorg's dummy driver (`xorg_e2e.rs`). X
  keeps no mirror direction (outputs share a CRTC, or CRTCs sit at one place),
  so the read-back names the first output the source and
  `Capabilities::mirror_direction` is off: the comparison takes mirrors as
  groups.
- **Change stream (built, M7):** `randr::Watch`, one long-lived connection of
  its own selecting `ScreenChangeNotify` and the CRTC, output and output
  property `Notify`s on the root (`RRSelectInput`), its socket watched with
  tokio's `AsyncFd`, which owns the connection. `changed` returns at a screen,
  CRTC or output change, or an output property change to the `EDID` atom
  (interned at setup; a backlight stepping is not a change), and takes the rest
  of the events already queued with it, so one change is one wake; the caller
  reads again, the screen's size included (`GetGeometry`, on the read's own
  connection), so a `ScreenChangeNotify`'s own size is not used. `poll_event`
  reads only what the socket holds and never waits for the server, and nothing
  else uses the connection, so it runs on the runtime's thread; connecting and
  selecting run on a detached thread under `OPERATION_TIMEOUT`. A server gone
  ends the watch with an error, and `dsplr watch` and the GUI connect again with
  their backoff. Run against Xvfb: the watch wakes once for each mode xrandr
  sets, and `dsplr watch` restores a profile when it starts, then reads again
  after an xrandr change and says the same outputs are connected.
- **Apply:** compute CRTC assignments, grab the server, disable CRTCs that
  shrink or move, `SetScreenSize` to the new bounding box, set each CRTC's mode,
  position and rotation, ungrab. This is xrandr's own algorithm; it is
  transcribed with its source cited and tested against recorded screen
  resources. The planner is written (`dsplr_backend::randr::plan`, xrandr
  1.5.4's `pick_crtcs`, `check_crtc_for_output`, `set_positions`,
  `set_screen_size`, `mark_changing_crtcs` and `apply`): pure, it turns a
  snapshot of the server (dsplr's own structs, which the crcbl RandR client's
  replies are copied into) and a layout into the ordered requests. Layout
  entries play xrandr's `--output` arguments, in the layout's order. Where it
  differs from xrandr: an output changes only when its mode, position, rotation
  or on/off state differ from the server's (xrandr sets every CRTC it is told
  about again). A CRTC's panning is carried over as xrandr carries it (read,
  then set again after the CRTC), unless it no longer fits the CRTC's new mode
  or position: Xorg's `xf86RandR12CrtcSet` checks the area
  (`xf86RandR13VerifyPanningArea`) and then pans the CRTC into
  `[x1, x2 - HDisplay]` by `[y1, y2 - VDisplay]` (`xf86RandR13Pan`) right after
  setting it, so the CRTC must lie inside the area. Panning that does not fit is
  reset before the CRTC is turned off, or before it is set when it is not, since
  a reset sent after would leave the CRTC where the old area clamped it; the
  plan notes it (`randr::Note::PanningReset`, "panning on HDMI-1 reset") for the
  user. Only Xorg pans (Xvfb has none), so this rests on its source alone. The
  primary output is set only when the layout makes one primary, as xrandr
  without `--primary` or `--noprimary`: `OutputState::primary` is a plain flag,
  so a profile from a desktop with no primary says false everywhere, and that
  must not clear X's. A mode the output does not list is added from the screen's
  modes (`--addmode`), or else made with VESA CVT timings transcribed from
  libxcvt 0.1.3 (`--newmode` with `cvt`'s line), and refused where CVT has no
  mode of that size. A flipped transform is `RR_Reflect_X` with the same
  rotation: `RRTransformCompute` reflects in the rotated frame, so the picture
  is flipped before it turns, as `wl_output.transform` defines the flipped
  values. xrandr's and libxcvt's notices, which both ask to be kept with copies,
  are in `crates/dsplr-backend/src/randr/LICENSES/`: xrandr's is an X11-style
  permission notice (HPND), libxcvt's is MIT.
- **No test primitive** (`Unsupported`), **no per-output scale**, **no native
  persistence**: persistence is dsplr's store plus `dsplr restore` at login
  ([05](05-persistence-and-profiles.md#x11)).
- Mirroring is two outputs on one CRTC, or same-position CRTCs; supported.
- Adaptive sync on X11 is a driver property (`vrr_capable` / per-driver
  options), not RandR's to set. The model has no place for a read-only value, so
  it reads as unknown, and the planner refuses one (backlog).
