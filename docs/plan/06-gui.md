# 06 — GUI

Drawn with crcbl: `crcbl-app` for the window and device, `crcbl-shell` for
windows on Wayland and X11, `crcbl-ui` for widgets and the draw pass. Tokyo
Night (folke's "Night" variant), as the rest of the suite.

## Built so far (M3, parts 1 and 2, 2026-11-10)

- `dsplr` with no command reads the displays on a backend thread (a
  current-thread tokio runtime, [02](02-architecture.md#process)), then opens
  the main window. No backend exits `6` before any window opens; no window
  system or no usable GPU exits `1` with the reason.
- `dsplr-core` holds the editor (`Editor`: the live `Snapshot`, the pending
  layout, undo and redo, the selection and its Tab order), every edit
  (`editor/edits.rs`: nudge, move to a neighbour, rotate, step the scale, turn
  on or off, and the panel's setters, each one undo step through
  `Editor::edit`), dragging (`editor/drag.rs`), the panel as data (`panel`), and
  builds the canvas and bar as its own `DrawList` of fills, outlines and clipped
  text plus hit regions (`draw`). No crcbl types: `dsplr-gui` translates the
  list into crcbl's (`paint`).
- The canvas: each output enabled live and with a size, at its logical size,
  scaled to fit together and centred, labelled with connector, description and
  mode; disabled (or mode-less) outputs as tiles in a tray strip below, narrowed
  so all of them stay inside the canvas. Each output whose pending settings
  differ is outlined where they put it, or around its own rectangle or tile when
  they leave it off the canvas. The pending layout is validated
  (`Layout::validate`) every frame: an output in an error (both outputs of an
  overlap) is outlined in the error colour, one with a warning (a gap, where the
  backend allows gaps) in the warning colour, and the bar shows the first
  problem and how many more there are.
- Hit regions: rectangles and tiles first, then the inside of each pending place
  below them, so a moved output can be grabbed where it now is, and an outline
  over another output's rectangle never takes that output's clicks.
- Dragging: a press on an output on the canvas selects and grabs it; the
  canvas's fit is frozen for the drag; each move snaps through `Layout::snap`
  with `SNAP_DISTANCE` (window pixels) divided by the zoom; the release lifts
  the layout off negative positions (`Layout::make_non_negative`, so no other
  output moves unless the drag left one negative; plan 03) and records the drag
  as one undo step; `Esc` during a drag puts the output back. A layout that
  cannot be lifted without a position leaving `i32` refuses the drag.
- The property panel, crcbl-ui widgets (C5) for the selected output: enabled
  (checkbox), mode (size) and refresh (drop-downs, refresh filtered by the
  size), custom mode (a text field, `WxH@R`, made on Enter or a click
  elsewhere), position (read-only: drag or keys), scale (a drop-down of the
  offered scales and a number to drag or type, made when the drag or typing
  ends), transform, adaptive sync, mirror source (drop-downs). A click on the
  canvas takes the keyboard out of the panel (`Ui::clear_focus`); what was being
  typed or dragged there is made on the output it was for, even when the click
  selects another, and the panel's drafts start afresh whenever the selection
  moves. While the quit prompt is up the panel makes no change. A field the
  backend cannot set, and every field but enabled on an output that is off, is
  disabled with why as its tooltip. A field a pending edit changed has its label
  in the pending colour. Hyprland's colour preset and bit depth and KDE's
  overscan and RGB range are shown, not edited.
- The rules are `dsplr set`'s, shared: `dsplr_model::default_mode` (an output
  turned on without a mode) and `dsplr_model::pick_mode` (a size keeps its
  refresh; a custom mode needs its refresh and a backend that takes them),
  `is_valid_scale`, adaptive sync only as the backend lists it, and no setting
  changed on an output that is off. Offered scales (`scale_choices`): the mode's
  own list (GNOME), else quarter steps from 0.5 to 3 on a backend with
  fractional scaling, else 1, 2 and 3.
- Keys: the whole table below except `i`, `a`, `s`. `h j k l` move by
  `NUDGE_STEP` (10 layout pixels); `H J K L` move until touching the nearest
  output that way that shares some of the span across, stopping at one it
  already touches; `space` turns the output on (its default mode, moved right of
  the others when it would overlap one) or off. `q` / `Esc` with pending edits
  asks in the bar (`y` / Enter quits, `n` / `Esc` does not); so does closing the
  window. While a field takes typing or a drop-down is open, keys go to the
  panel: Enter accepts, `Esc` backs out, the arrows move in a list, Tab moves
  between fields.
- Text is crcbl-ui's committed sans-serif (`Font::sans`, the `parsed-font`
  feature); no system font lookup yet.

## Still to build in M3

- The bottom bar's `Identify`, `Revert`, `Apply`, `Save` and how the backend
  will save; `a` / `s` / `i`.
- `Request`s to the backend thread (test, apply, revert, persist) and the
  confirm dialog per output (C2).
- The identify overlay (C2, C3).
- The backend thread following the desktop's changes, so the window updates on a
  hotplug (C4); today the thread reads one snapshot and ends.
- Keys configurable in `config.toml`.
- System fonts with fallbacks (C7), for descriptions outside the committed
  font's coverage.
- Reaching the panel's widgets from the keyboard: Tab selects outputs, so today
  the widgets take the pointer only, and the keys only once a field or list is
  engaged.

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
0.18.0 by building on it where the "M3" column says so. Anything missing is
raised in the crcbl repo rather than worked around in dsplr:

| #   | Need                                                         | Seen in crcbl                                                                                             | M3                                                                                                                                                                                                                                                                         |
| --- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C1  | Toplevel window on Wayland and X11                           | `WindowRole::Toplevel`; Wayland and X11 shells                                                            | Confirmed on Wayland: the main window runs in headless sway (2026-11-10). X11 not run                                                                                                                                                                                      |
| C2  | A panel on a chosen monitor, on Wayland (not GNOME) and X11  | `WindowRole::Panel` (`wayland/layer.rs`, `x11/panel.rs`)                                                  | Not built on yet                                                                                                                                                                                                                                                           |
| C3  | Monitor enumeration matching the backend's outputs by name   | `crcbl-shell` `MonitorInfo`; whether it carries the connector name is to check                            | Not built on yet; the `connector` field is requested of crcbl (backlog)                                                                                                                                                                                                    |
| C4  | Cross-thread wakeup for the backend thread                   | `crcbl-shell` `waker.rs`                                                                                  | Not used yet: the one snapshot arrives before the window opens                                                                                                                                                                                                             |
| C5  | Widgets: button, select/list, number entry, toggle, tooltip  | `crcbl-ui` `widgets/` (`button`, `select`, `list`, `number_entry`), `tooltip`                             | Built on 0.17.2, still on 0.18.0: `select`, `checkbox`, `drag_value` (typing into it is the number entry), `text_input`, `tooltip`, and `Ui::enabled` for disabled fields; no plain list or toggle needed. Run in headless sway; never clicked there (no pointer injector) |
| C6  | Free-form drag canvas: draw rects and text, raw pointer drag | `crcbl-ui` `DrawList`, `grid_drag`; whether `grid_drag` fits or the canvas is hand-built is decided at M3 | Hand-built, not `grid_drag`: dsplr-core draws the canvas, its hit regions and the drag (fit frozen, `Layout::snap`); crcbl gives only the raw pointer                                                                                                                      |
| C7  | System font lookup                                           | `crcbl-ui` `system-fonts` feature (pikr precedent)                                                        | Not used yet: text is `Font::sans`                                                                                                                                                                                                                                         |

## Not in the GUI

A preview of the actual screen contents in each rectangle (wdisplays does this
with screencopy). Considered and left for later: it costs a screencopy
dependency and frame copies for little help in arranging.
