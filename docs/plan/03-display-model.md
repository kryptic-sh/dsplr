# 03 — Display model

`dsplr-model` is the one vocabulary every backend translates into and out of. It
has no I/O.

## One struct, many formats

The design rests on one rule (the owner's, 2026-11-04): **a single data
structure describes the displays and their settings, and everything else is a
conversion of it.**

```
                      ┌──────────────── GUI renders and edits it
                      │                 CLI prints and edits it
                      ▼
 desktop state ──► Layout ──► live apply (wlr protocol, KDE protocol,
   (read in by                 Mutter D-Bus, RandR, Hyprland eval)
    each backend)        └──► saved formats (monitors.lua, monitors.conf,
                               sway outputs, niri KDL, dsplr profile TOML)
```

- The GUI holds a `Layout` (plus pending edits, which are a second `Layout`) and
  draws from it. It never reads a backend type or a config format.
- Each backend has one function in (`desktop state → Layout`) and one out
  (`Layout → the desktop's apply call`).
- Each saved format in `dsplr-persist` is a pair of pure functions,
  `write(&Layout) -> String` and `read(&str) -> Layout`, tested by round-trip.
- Adding a desktop or a format means adding a conversion, never a field in the
  GUI.

Settings only one desktop has (Hyprland's colour preset and bit depth, KDE's
overscan and RGB range, a primary output) are **fields of the same struct**,
optional, with `Capabilities` saying which the active backend reads and sets.
They are not kept in a side table per backend: the GUI shows a field when the
capability is present and the conversions for other desktops ignore it.

## Types

- **`OutputId`** — stable identity for matching across reconnects and between
  backends: `(make, model, serial)` when the serial is non-empty, else the
  connector name (`HDMI-A-1`). Hyprland, wlroots, KDE and GNOME all report make,
  model and serial; XRandR gets them by parsing the EDID property. Make is
  **normalised to the EDID's PNP vendor code** before comparing: the backends do
  not agree on it (wlroots and Hyprland report a resolved name such as "AOC",
  Mutter is expected to report the three-letter code, to verify at M6), so a raw
  `make` would make profiles fail to match across desktops. KDE exposes the EDID
  and EISA id directly; RandR has the EDID property.
- **`Layout`** — the one struct. A list of **`OutputEntry`**s, each holding the
  hardware facts (`Output`) and the settings (`OutputState`) for one monitor,
  plus layout-wide settings: the **fallback rule** for monitors not listed
  (Hyprland's `output = ""` catch-all, sway's `output *`), and GNOME's
  `layout_mode` where it applies. `dsplr list --json`, the GUI, the profile
  store and every converter use this same struct; `Snapshot` is a `Layout` read
  from the desktop plus its serial, not a separate shape. Apply, test, persist
  and profiles all take a whole `Layout`, never a single field, because every
  desktop applies a configuration atomically.
- **`Output`** (hardware facts) — connector name, description, `OutputId`,
  physical size in mm, available `Mode`s. **Optional**, because a `Layout` read
  back from a config file (`monitors.lua`, a sway outputs file) knows a
  monitor's name and settings but not its modes or EDID. Readers in
  `dsplr-persist` therefore return a `Layout` whose entries may lack hardware
  facts, and the GUI fills them from the live snapshot by `OutputId`.
- **`OutputState`** (settings) — enabled, current mode (or a custom mode),
  position in layout coordinates, scale, `Transform`, `AdaptiveSync`, mirror
  source, primary, and the optional desktop-specific settings above
  (`colour_preset`, `bit_depth`, `overscan`, `rgb_range`, …).
- **`AdaptiveSync`** — one enum for every desktop: `Off`, `On`, `Automatic`
  (KDE's VRR policy, Hyprland's fullscreen-only value). Backends map their own
  form (wlroots' bool, KDE's policy, Hyprland's integer `vrr`) onto it, and
  `Capabilities` says which values the active backend accepts.
- **`Mode`** — width and height in pixels, refresh in **millihertz** (the unit
  `wlr-output-management` and KDE use; Hyprland's `"1920x1080@60.00Hz"` strings
  and GNOME's doubles are converted at the edge, once), whether it is preferred,
  the **valid scales for this mode** (GNOME lists them per mode), and an opaque
  **backend handle** (GNOME's mode id string, the wlr mode object, the RandR
  mode XID) so the exact mode is passed back rather than matched again by
  number. Matching a mode from a config file uses a refresh tolerance
  (`REFRESH_TOLERANCE_MHZ`), because the formats round differently.
- **`Transform`** — the eight Wayland transforms (normal, 90, 180, 270 and the
  flipped four). XRandR's rotation + reflection pairs map onto the same eight.
- **`Capabilities`** — what the active backend can do: custom modes, mirroring,
  the `AdaptiveSync` values it accepts, fractional scale, per-output scale (no
  on X11, and not when GNOME reports `global-scale-required`), real
  test-before-apply, whether gaps are allowed, native persistence, whether it
  saves on every apply. The GUI greys out what is absent and says why.

## Coordinates and scale

Positions are in **layout (logical) coordinates**, the space Wayland compositors
and GNOME's "logical monitors" use: an output's extent on the canvas is its mode
size, rotated by its transform, divided by its scale.

- Scale is stored as given by the backend. Valid scales are backend-specific: on
  wlroots `set_scale` takes a `wl_fixed` and the protocol lets the compositor
  round it, so dsplr reads the applied scale back rather than assuming it;
  Hyprland has its own list of "clean" scales; GNOME lists supported scales per
  mode (on `Mode`); KDE on X11 is integer-only. The backend supplies the valid
  set; the model only rounds to it.
- **X11 has no per-output scale.** The RandR backend reports scale `1.0`, marks
  it read-only, and positions are in pixels. RandR's CRTC transforms
  (`xrandr --scale`) can fake one by rescaling the framebuffer; declined for the
  MVP because it blurs and costs performance. Xft DPI is out of scope.
- Rounding: positions are integers. A logical size that is not a whole number
  (2560 wide at scale 1.75) is rounded the way the target backend rounds it,
  recorded per backend with a test against a recorded reply. Until those are
  recorded, `OutputState::logical_size` rounds to the nearest pixel for every
  backend.

## Validation

`Layout::validate(&Capabilities) -> Vec<Problem>`. Each `Problem` names the
output (none for a layout-wide problem), its kind, the field the kind is about,
and a severity: the layout is refused when any problem is an error, and warnings
are shown without stopping it. A `Result<(), _>` would drop the warnings on
success, so it is not one. Checked:

- at least one output enabled, and no output listed twice;
- every enabled output has a mode it lists, or a custom mode on a backend that
  accepts them. A mode is matched to the list within `REFRESH_TOLERANCE_MHZ`; an
  entry without hardware facts (read from a file) is not checked until it is
  matched to a live output;
- scale a positive number, whole unless the backend has fractional scaling, in
  the mode's valid set where the mode lists one, and the same on every output
  when the backend has no per-output scale (X11, GNOME's
  `global-scale-required`);
- no overlap between enabled, non-mirrored outputs;
- no gap: the enabled, non-mirrored outputs are connected edge to edge (a shared
  corner is not a connection, and two islands are a gap even though each output
  in them touches another). An **error** where `Capabilities::gaps_allowed` is
  off, as on KDE, whose management protocol says "There may be no gaps or
  overlaps between outputs"; a warning elsewhere (Hyprland and sway allow gaps,
  but the pointer cannot cross one);
- mirroring supported, and mirror sources exist, are enabled and are not mirrors
  themselves.

## Snapping

Dragging on the canvas snaps an output's edges to the nearest edges of the
others within a threshold in screen pixels (`SNAP_DISTANCE`), preferring
edge-to-edge contact, then aligned top/left edges, then centres. On release the
layout is **normalised**: translated so the minimum x and y across enabled
outputs is zero, which is what XRandR requires and what every other backend
accepts.

`Layout::snap` takes its threshold in layout pixels, so `SNAP_DISTANCE` belongs
to the canvas (`dsplr-core`), which divides it by the canvas zoom before
calling. Each axis snaps on its own. Edge contact is only offered against an
output that overlaps, or nearly, along the other axis, since touching is
impossible otherwise. `Layout::normalise` moves disabled outputs and mirrors by
the same amount as the rest, and refuses (changing nothing) when a position
would leave `i32`.

## JSON form

`dsplr list --json` and the profile store use one serde form of `Layout` (field
names in snake_case, refresh in mHz; backend mode handles are not serialised, so
a profile's modes are matched again on load). It is versioned (`"version": 0`)
and may change before 1.0.
