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
pub enum Backend { Hyprland(..), Wlr(..), Kde(..), Gnome(..), Randr(..) }

impl Backend {
    pub fn capabilities(&self) -> Capabilities;
    pub fn snapshot(&mut self) -> Result<Snapshot, BackendError>;
    pub fn test(&mut self, layout: &Layout) -> Result<(), BackendError>;
    pub fn apply(&mut self, layout: &Layout) -> Result<(), BackendError>;
    pub fn persist(&self, layout: &Layout, approved: Option<&str>) -> Result<Persisted, BackendError>;
    // plus a change stream feeding the backend thread's loop
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
  would miss. Each `done` triggers a fresh `j/monitors all`.
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
then writes sway's `outputs` ([05](05-persistence-and-profiles.md#sway)); on any
other it is `Unsupported` for now.

Mirroring is not in the protocol; on `Wlr` it is `Unsupported` unless a
compositor-specific path is added later.

## KDE Plasma 6 (`Kde`)

KWin's own Wayland protocols, read in `plasma-wayland-protocols` as shipped in
the `wayland-protocols-plasma` crate (0.3.12):

- `kde_output_device_v2` (v11): one per output, with modes (refresh in mHz),
  geometry, scale, transform, VRR policy, overscan, RGB range, HDR, EDID and
  EISA id.
- `kde_output_order_v1`: the output **priority** order (KDE's primary output is
  the first), announced as output names. Set with the configuration's
  `set_priority` (since v3).
- `kde_output_management_v2` (v12): a configuration object with per-device
  setters and `apply`, answered `applied` / `failed`, with a `failure_reason`
  event since v12. **No test request.** Its `position` request requires "no gaps
  or overlaps between outputs", which validation enforces on KDE
  ([03](03-display-model.md#validation)).

The installed KWin is ahead of that crate: `libkwin.so.6.8.2` on the owner's
machine contains `set_custom_modes`, `set_replication_source` and
`applyMirroring`, so **custom modes and mirroring exist in KWin 6.8** but not in
the crate's protocol files. Using them needs newer bindings or bindings
generated from KWin's own XML (open question). The protocol files also warn that
backward-incompatible changes may land without a version bump, so the KDE
backend checks the versions it gets and refuses ones it was not tested against.

- Plasma 6 KWin saves output configurations itself: `kwinoutputconfig.json` is a
  string in `libkwin` (seen), and that it saves on **every** protocol apply is
  expected but not verified (M5). If it does, `persist` is a no-op that reports
  `Persisted::ByDesktop`, `Capabilities` says the desktop saves on apply, and a
  change that is not kept is undone only by dsplr's revert
  ([02](02-architecture.md#apply-confirm-revert)).
- **KDE on X11** keeps KScreen's own daemon, which re-applies its saved config
  and would fight dsplr's `Randr` backend. `--backend randr` under a KDE session
  warns about this, and `dsplr watch` refuses it there too.
- KDE's extra fields (overscan, RGB range, VRR policy "automatic", output
  priority) are optional fields of the shared `OutputState`
  ([03](03-display-model.md#one-struct-many-formats)), shown in the property
  panel when `Capabilities` says KDE is active.
- The protocol versions KWin offers are read from the registry at M5 and the
  supported range written into this section.

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
