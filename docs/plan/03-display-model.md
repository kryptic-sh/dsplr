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
  model and serial; XRandR gets them by parsing the EDID property.
- **`Output`** — connector name, description, `OutputId`, physical size in mm,
  available `Mode`s, and the `OutputState`.
- **`OutputState`** — enabled, current mode (or a custom mode), position in
  layout coordinates, scale, `Transform`, adaptive sync, mirror source, primary,
  and the optional desktop-specific settings above (`colour_preset`,
  `bit_depth`, `overscan`, `rgb_range`, …).
- **`Mode`** — width and height in pixels, refresh in **millihertz** (the unit
  `wlr-output-management` and KDE use; Hyprland's floats and GNOME's doubles are
  converted at the edge, once), and whether it is preferred.
- **`Transform`** — the eight Wayland transforms (normal, 90, 180, 270 and the
  flipped four). XRandR's rotation + reflection pairs map onto the same eight.
- **`Layout`** — the full set of `OutputState`s, keyed by `OutputId`. Apply,
  test, persist and profiles all take a whole `Layout`, never a single field,
  because every desktop applies a configuration atomically.
- **`Capabilities`** — what the active backend can do: custom modes, mirroring,
  adaptive sync, fractional scale, per-output scale (no on X11), test-before-
  apply, native persistence. The GUI greys out what is absent and says why.

## Coordinates and scale

Positions are in **layout (logical) coordinates**, the space Wayland compositors
and GNOME's "logical monitors" use: an output's extent on the canvas is its mode
size, rotated by its transform, divided by its scale.

- Scale is stored as given by the backend. Valid scales are backend-specific:
  `wp_fractional_scale` steps of 1/120 on wlroots, Hyprland's own list of
  "clean" scales, GNOME's per-mode `supported_scales`, integer-only on KDE X11.
  The backend supplies the valid set; the model only rounds to it.
- **X11 has no per-output scale.** The RandR backend reports scale `1.0`, marks
  it read-only, and positions are in pixels. Xft DPI is out of scope.
- Rounding: positions are integers. A logical size that is not a whole number
  (2560 wide at scale 1.75) is rounded the way the target backend rounds it,
  recorded per backend with a test against a recorded reply.

## Validation

`Layout::validate(&Capabilities) -> Result<(), Vec<Problem>>`. Every problem
names the output and the field. Checked:

- at least one output enabled;
- every enabled output has a mode it lists, or a custom mode on a backend that
  accepts them;
- scale in the backend's valid set;
- no overlap between enabled, non-mirrored outputs;
- no gap: every enabled output touches another (a warning, not an error —
  Hyprland and sway allow gaps, but the pointer cannot cross one);
- mirror sources exist, are enabled and are not mirrors themselves.

## Snapping

Dragging on the canvas snaps an output's edges to the nearest edges of the
others within a threshold in screen pixels (`SNAP_DISTANCE`), preferring
edge-to-edge contact, then aligned top/left edges, then centres. On release the
layout is **normalised**: translated so the minimum x and y across enabled
outputs is zero, which is what XRandR requires and what every other backend
accepts.

## JSON form

`dsplr list --json` and the profile store use one serde form of `Layout` and
`Output` (field names in snake_case, refresh in mHz). It is versioned
(`"version": 0`) and may change before 1.0.
