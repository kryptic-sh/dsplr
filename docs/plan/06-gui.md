# 06 — GUI

Drawn with crcbl: `crcbl-app` for the window and device, `crcbl-shell` for
windows on Wayland and X11, `crcbl-ui` for widgets and the draw pass. Tokyo
Night (folke's "Night" variant), as the rest of the suite.

## Built so far (M3, part 1, 2026-11-10)

- `dsplr` with no command reads the displays on a backend thread (a
  current-thread tokio runtime, [02](02-architecture.md#process)), then opens
  the main window. No backend exits `6` before any window opens; no window
  system or no usable GPU exits `1` with the reason.
- `dsplr-core` holds the editor (`Editor`: the live `Snapshot`, the pending
  layout, undo and redo over pending edits through `Editor::edit`, the selection
  and its Tab order) and builds the frame as its own `DrawList` of fills,
  outlines and clipped text plus hit regions (`draw`), from the editor, the
  window size and the `Theme`. No crcbl types: `dsplr-gui` translates the list
  into crcbl's (`paint`).
- The canvas: each output enabled live and with a size, at its logical size,
  scaled to fit together and centred, labelled with connector, description and
  mode; disabled (or mode-less) outputs as tiles in a tray strip below; each
  output whose pending settings differ outlined where they put it, or around its
  own rectangle or tile when they leave it off the canvas. Only rectangles and
  tiles are click targets: an outline is a border, and over another output it
  would take that output's clicks. Tray tiles narrow so that all of them stay
  inside the canvas.
- The property panel, read-only: the rows below for the selected output, a
  setting the backend cannot set reading "not supported by <backend>", a pending
  change in the pending colour. The backend-specific rows show only on a backend
  with the capability.
- The bottom bar: the backend's name and the keys.
- Keys: `Tab` / `S-Tab`, `u` / `C-r`, `q` / `Esc` (quitting does not ask yet:
  nothing in the window can make an edit). Click to select.
- Text is crcbl-ui's committed sans-serif (`Font::sans`, the `parsed-font`
  feature); no system font lookup yet.

## Still to build in M3

- Dragging with snapping ([03](03-display-model.md#snapping)), and the editing
  keys (`h j k l`, `H J K L`, `r` / `R`, `+` / `-`, `space`), all through
  `Editor::edit`. A drag should be one undo step, not one per pointer move.
- Editing in the property panel with crcbl-ui widgets (C5), tooltips for the
  fields the backend cannot set.
- The bottom bar's `Identify`, `Revert`, `Apply`, `Save` and how the backend
  will save; `a` / `s` / `i`.
- `Request`s to the backend thread (test, apply, revert, persist) and the
  confirm dialog per output (C2); quitting with pending edits asks first.
- The identify overlay (C2, C3).
- The backend thread following the desktop's changes, so the window updates on a
  hotplug (C4); today the thread reads one snapshot and ends.
- Keys configurable in `config.toml`.
- System fonts with fallbacks (C7), for descriptions outside the committed
  font's coverage.

## Main window

A toplevel window with two regions:

- **Canvas** (left, most of the window): every output as a rectangle at its
  logical size, scaled to fit, labelled with connector, description and mode.
  Disabled outputs sit in a tray strip below, greyed. Drag to move, with
  snapping ([03](03-display-model.md#snapping)); click to select.
- **Property panel** (right): for the selected output — enabled, mode (list),
  refresh (list filtered by mode), custom mode (if the backend allows), scale
  (list of the backend's valid scales plus a number entry), transform, adaptive
  sync, mirror source. A backend-specific section below (Hyprland colour preset
  and bit depth, KDE overscan and RGB range). Fields the backend cannot set are
  disabled with the reason as a tooltip.
- **Bottom bar:** backend name and how it will save, `Identify`, `Revert`,
  `Apply`, `Save`, and a profile picker when profiles exist.

Pending edits are drawn as an outline over the live state until applied. Undo
and redo over pending edits (`u`, `C-r`).

## Keys

Mouse-first for dragging, but every action has a key:

| Key             | Action                                          |
| --------------- | ----------------------------------------------- |
| `Tab` / `S-Tab` | select next / previous output                   |
| `h j k l`       | nudge the selected output by one snap step      |
| `H J K L`       | move the selected output to touch its neighbour |
| `r` / `R`       | rotate clockwise / counter-clockwise            |
| `+` / `-`       | next / previous scale                           |
| `space`         | enable / disable                                |
| `i`             | identify                                        |
| `a` / `s`       | apply / save                                    |
| `u` / `C-r`     | undo / redo                                     |
| `q` / `Esc`     | quit (asks if there are pending edits)          |

Keys are configurable in `config.toml`.

## Confirm dialog

After an apply that can hide or move a screen, a small dialog opens **on every
enabled output** (so it is on screen whichever one still shows): "Keep these
display settings?" with `Keep` and `Revert` and a countdown. `Enter` keeps,
`Esc` reverts, the timeout reverts. Implemented as crcbl panels (layer-shell on
Wayland, override-redirect on X11) placed per monitor.

**GNOME has no layer-shell**, so there the confirm is a normal toplevel dialog
that GNOME places, and the revert watchdog covers the case where it lands on a
screen that went dark ([02](02-architecture.md#apply-confirm-revert)). On Save,
GNOME Shell shows its own confirm dialog instead of dsplr's.

## Identify overlay

`Identify` shows each output's connector and description in large text on that
output for a few seconds, as a crcbl panel per monitor. On GNOME, where panels
are not available, it asks GNOME Shell to show its own monitor labels
(`org.gnome.Shell` `ShowMonitorLabels`, which gnome-control-center uses; whether
Shell accepts the call from another client is to verify at M6), else identify is
`Unsupported` there.

## What crcbl must provide

Checked against crcbl's tree on 2026-11-04 at a source level, and against crcbl
0.17.2 by building on it where the "M3" column says so. Anything missing is
raised in the crcbl repo rather than worked around in dsplr:

| #   | Need                                                         | Seen in crcbl                                                                                             | M3                                                                                      |
| --- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| C1  | Toplevel window on Wayland and X11                           | `WindowRole::Toplevel`; Wayland and X11 shells                                                            | Confirmed on Wayland: the main window runs in headless sway (2026-11-10). X11 not run   |
| C2  | A panel on a chosen monitor, on Wayland (not GNOME) and X11  | `WindowRole::Panel` (`wayland/layer.rs`, `x11/panel.rs`)                                                  | Not built on yet                                                                        |
| C3  | Monitor enumeration matching the backend's outputs by name   | `crcbl-shell` `MonitorInfo`; whether it carries the connector name is to check                            | Not built on yet; the `connector` field is requested of crcbl (backlog)                 |
| C4  | Cross-thread wakeup for the backend thread                   | `crcbl-shell` `waker.rs`                                                                                  | Not used yet: the one snapshot arrives before the window opens                          |
| C5  | Widgets: button, select/list, number entry, toggle, tooltip  | `crcbl-ui` `widgets/` (`button`, `select`, `list`, `number_entry`), `tooltip`                             | Not built on yet                                                                        |
| C6  | Free-form drag canvas: draw rects and text, raw pointer drag | `crcbl-ui` `DrawList`, `grid_drag`; whether `grid_drag` fits or the canvas is hand-built is decided at M3 | Hand-built: dsplr-core draws the canvas and its hit regions; the drag is still to build |
| C7  | System font lookup                                           | `crcbl-ui` `system-fonts` feature (pikr precedent)                                                        | Not used yet: text is `Font::sans`                                                      |

## Not in the GUI

A preview of the actual screen contents in each rectangle (wdisplays does this
with screencopy). Considered and left for later: it costs a screencopy
dependency and frame copies for little help in arranging.
