# 06 — GUI

Drawn with crcbl: `crcbl-app` for the window and device, `crcbl-shell` for
windows on Wayland and X11, `crcbl-ui` for widgets and the draw pass. Tokyo
Night (folke's "Night" variant), as the rest of the suite.

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

## Identify overlay

`Identify` shows each output's connector and description in large text on that
output for a few seconds, as a crcbl panel per monitor.

## What crcbl must provide

Checked against crcbl's tree on 2026-11-04 at a source level only (nothing built
against it yet). Confirmed at M3, and anything missing is raised in the crcbl
repo rather than worked around in dsplr:

| #   | Need                                                         | Seen in crcbl                                                                                             |
| --- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| C1  | Toplevel window on Wayland and X11                           | `WindowRole::Toplevel`; Wayland and X11 shells                                                            |
| C2  | A panel on a chosen monitor, on Wayland and X11              | `WindowRole::Panel` (`wayland/layer.rs`, `x11/panel.rs`)                                                  |
| C3  | Monitor enumeration matching the backend's outputs by name   | `crcbl-shell` `MonitorInfo`; whether it carries the connector name is to check                            |
| C4  | Cross-thread wakeup for the backend thread                   | `crcbl-shell` `waker.rs`                                                                                  |
| C5  | Widgets: button, select/list, number entry, toggle, tooltip  | `crcbl-ui` `widgets/` (`button`, `select`, `list`, `number_entry`), `tooltip`                             |
| C6  | Free-form drag canvas: draw rects and text, raw pointer drag | `crcbl-ui` `DrawList`, `grid_drag`; whether `grid_drag` fits or the canvas is hand-built is decided at M3 |
| C7  | System font lookup                                           | `crcbl-ui` `system-fonts` feature (pikr precedent)                                                        |

## Not in the GUI

A preview of the actual screen contents in each rectangle (wdisplays does this
with screencopy). Considered and left for later: it costs a screencopy
dependency and frame copies for little help in arranging.
