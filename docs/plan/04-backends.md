# 04 — Backends

## Detection

At start `dsplr-backend` picks one backend, first match wins:

| Order | Condition                                                                | Backend    |
| ----- | ------------------------------------------------------------------------ | ---------- |
| 1     | `HYPRLAND_INSTANCE_SIGNATURE` set and its socket answers                 | `Hyprland` |
| 2     | `WAYLAND_DISPLAY` set and the registry offers `kde_output_management_v2` | `Kde`      |
| 3     | `org.gnome.Mutter.DisplayConfig` owned on the session bus                | `Gnome`    |
| 4     | `WAYLAND_DISPLAY` set and the registry offers `zwlr_output_manager_v1`   | `Wlr`      |
| 5     | `DISPLAY` set and the server has RandR 1.5 or later                      | `Randr`    |

When `HYPRLAND_INSTANCE_SIGNATURE` is set but its socket cannot be found or does
not answer, detection stops at row 1 with no backend rather than going on: the
session is Hyprland, which offers `zwlr_output_manager_v1` too, and driving it
as `Wlr` would test with its stub and leave protocol-applied state overriding
its monitor rules until restart.

Detection never trusts `XDG_CURRENT_DESKTOP` alone: a variable says what the
session claims to be, the registry and the bus say what is there.
`dsplr --backend <name>` overrides it, for testing and for desktops that offer
more than one interface (KDE on X11 is `Randr`; GNOME on X11 offers both
`DisplayConfig` and RandR, and `Gnome` wins because it persists).

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
  after it.

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
  `eval` or `keyword` is not followed; the backlog has the choice.
- **Apply** through `wlr-output-management` (shared code with `Wlr`): atomic
  apply and adaptive sync, but no real test. Fields the protocol cannot express
  (mirroring, bit depth, colour-management preset) go through
  `eval hl.monitor({ … })` on the socket. Whether `eval` of a `hl.monitor` table
  applies immediately, and whether it composes with a protocol apply, is an **M2
  exit check**: the protocol requires every head in a configuration and Hyprland
  reports no mirrored state through it, so a protocol apply may undo an `eval`
  mirror. Until that is checked those fields are read-only.
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
  the rest in KWin's order now. Refused before sending: a custom mode (KWin's
  take a mode list per output, not written yet), colour preset, bit depth, a
  setting the output's capabilities lack, a second primary output, one that is
  off, and turning off an output that says it cannot be (device version 25 on).
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
  primary output when KWin offers `kde_output_order_v1`; native persistence and
  saves on apply; no test, no gaps, no custom modes.
- **KDE on X11** keeps KScreen's own daemon, which re-applies its saved config
  and would fight dsplr's `Randr` backend. `--backend randr` under a KDE session
  warns about this, and `dsplr watch` refuses it there too.

## GNOME (`Gnome`)

`org.gnome.Mutter.DisplayConfig` on the session bus:

- **Read:** `GetCurrentState()` → serial, physical monitors (connector, vendor,
  product, serial, modes with their supported scales), logical monitors (x, y,
  scale, transform, primary, which monitors), properties (`layout-mode`,
  `supports-changing-layout-mode`, `global-scale-required`).
- **Change stream:** the `MonitorsChanged` signal.
- **Apply:** `ApplyMonitorsConfig(serial, method, logical_monitors, props)`
  where method is `verify` (dsplr's `test`), `temporary` (apply, not saved,
  never reverted by Mutter) or `persistent` (apply, then Mutter's own confirm,
  then `~/.config/monitors.xml`).
- GNOME's model is logical monitors containing physical ones; mirroring is two
  monitors in one logical monitor, so `Gnome` supports mirroring.
- `layout-mode` decides whether positions are logical or physical pixels; the
  backend converts both ways.
- A `persistent` apply from **any** client starts Mutter's own confirmation:
  GNOME Shell shows its "Keep changes?" dialog, Mutter writes `monitors.xml`
  only if the user keeps it and restores the previous config on timeout
  (`DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT` in
  `src/backends/meta-monitor-manager.c`). How dsplr uses that is in
  [02](02-architecture.md#apply-confirm-revert).
- Mutter offers no layer-shell (its Wayland protocol list in `src/meson.build`
  has no `wlr-layer-shell`), so dsplr's per-monitor dialogs and identify overlay
  are drawn differently on GNOME ([06](06-gui.md)).

Cinnamon's Muffin exposes the same interface under
`org.cinnamon.Muffin.DisplayConfig`; whether one implementation serves both is
checked at M6 and recorded as a stretch item if it does.

## X11 (`Randr`)

RandR 1.5 on the X server, for i3, bspwm, awesome, Openbox, Xfce, and KDE or
GNOME sessions on X11 when chosen with `--backend randr`.

- **Read:** screen resources, outputs, CRTCs, modes, the `EDID` output property
  (parsed for make, model, serial), monitors (RandR 1.5).
- **Change stream:** `RRSelectInput` for screen, CRTC and output change
  notifications.
- **Apply:** compute CRTC assignments, grab the server, disable CRTCs that
  shrink or move, `SetScreenSize` to the new bounding box, set each CRTC's mode,
  position and rotation, ungrab. This is xrandr's own algorithm; it is
  transcribed with its source cited and tested against recorded screen
  resources.
- **No test primitive** (`Unsupported`), **no per-output scale**, **no native
  persistence**: persistence is dsplr's store plus `dsplr restore` at login
  ([05](05-persistence-and-profiles.md#x11)).
- Mirroring is two outputs on one CRTC, or same-position CRTCs; supported.
- Adaptive sync on X11 is a driver property (`vrr_capable` / per-driver options)
  and is shown read-only.
