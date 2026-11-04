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
    pub fn persist(&mut self, layout: &Layout) -> Result<Persisted, BackendError>;
    // plus a change stream feeding the backend thread's loop
}
```

`test` on a backend with no test primitive returns `Unsupported`, and the apply
flow skips step 2 rather than pretending it ran
([02](02-architecture.md#apply-confirm-revert)).

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

- **Read** from `j/monitors all` on `.socket.sock` (the bytes `hyprctl -j`
  sends), and subscribe to `monitoraddedv2` / `monitorremovedv2` /
  `configreloaded` on `.socket2.sock` for changes, as lyr does for workspaces.
- **Apply** through `wlr-output-management` (shared code with `Wlr`), which has
  test, atomic apply and adaptive sync. Fields the protocol cannot express
  (mirroring, bit depth, colour-management preset) go through
  `eval hl.monitor({ … })` on the socket. Whether `eval` of a `hl.monitor` table
  applies immediately, and whether it composes with a protocol apply, is the
  first thing M2 checks; until it is checked those fields are read-only.
- **Persist** by writing `~/.config/hypr/monitors.lua`
  ([05](05-persistence-and-profiles.md#hyprland)), which the owner's
  `hyprland.lua` already `require`s. Legacy hyprlang configs (`hyprland.conf` +
  `monitor=` lines) are detected and written in that syntax.

## wlroots compositors (`Wlr`)

`zwlr_output_manager_v1`: heads with their modes; a configuration object with
`enable_head` / `disable_head`, `set_mode` / `set_custom_mode`, `set_position`,
`set_transform`, `set_scale`, `set_adaptive_sync` (v4), then `test()` or
`apply()`, answered by `succeeded` / `failed` / `cancelled`. Every change event
ends with `done` carrying a serial; a configuration created against a stale
serial is `cancelled`, which dsplr answers by re-reading and showing the new
state, never by retrying blindly.

Covers sway, niri, river, labwc, Wayfire, Hyprland (live path) and any other
compositor that offers the protocol, including COSMIC (to verify in M4).
Persistence differs per compositor ([05](05-persistence-and-profiles.md)).

Mirroring is not in the protocol; on `Wlr` it is `Unsupported` unless a
compositor-specific path is added later.

## KDE Plasma 6 (`Kde`)

KWin's own Wayland protocols: `kde_output_device_v2` (one per output: modes,
geometry, scale, transform, VRR policy, overscan, RGB range, HDR and the
priority order KDE uses for the primary output) and `kde_output_management_v2`
(a configuration object with per-device setters and `apply`, answered `applied`
/ `failed`).

- Plasma 6 KWin saves applied configurations itself (in
  `~/.config/kwinoutputconfig.json`, to verify at M5), so `persist` is a no-op
  that reports `Persisted::ByDesktop`.
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
  where method is `verify` (dsplr's `test`), `temporary` (apply) or `persistent`
  (apply and write `~/.config/monitors.xml`).
- GNOME's model is logical monitors containing physical ones; mirroring is two
  monitors in one logical monitor, so `Gnome` supports mirroring.
- `layout-mode` decides whether positions are logical or physical pixels; the
  backend converts both ways.
- Mutter shows its own "Keep changes?" dialog only for changes made in
  gnome-control-center; dsplr's dialog is the one shown here. To verify at M6.

Cinnamon's Muffin exposes the same interface under
`org.cinnamon.Muffin. DisplayConfig`; whether one implementation serves both is
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
