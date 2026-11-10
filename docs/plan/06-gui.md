# 06 — GUI

Drawn with crcbl: `crcbl-app` for the window and device, `crcbl-shell` for
windows on Wayland and X11, `crcbl-ui` for widgets and the draw pass. Tokyo
Night (folke's "Night" variant), as the rest of the suite.

## Built (M3, 2026-11-10)

The window, canvas, property panel, keys, undo, Apply / Save / Revert / profiles
through the shared confirm flow, the confirm dialogs and the identify overlay
are built; the code is the account of them (`dsplr-core`'s editor, scene, panel,
dialog and link modules, `dsplr-gui`, `apps/dsplr/src/gui.rs`). What still binds
later work is below.

- The backend thread follows the desktop's changes (`Backend::watch`): on
  wlroots a `wlr-output-management` connection of its own, on Hyprland its event
  socket ([04](04-backends.md#hyprland)). Each change reads the displays again,
  paired with the last read by `dsplr_model::pair` (twins keep their
  connectors), and merged setting by setting under the pending edits
  (`Editor::rebase`): a setting the user changed keeps the user's value, saying
  so in the bar when the desktop changed it too, and every other setting takes
  the desktop's. Which output is primary is merged as one setting of the whole
  layout (one primary per desktop): edited anywhere, the user's choice holds on
  every output. Edits to outputs that went away are dropped, also in the bar. A
  read that changed nothing leaves the edits, their history and a drag alone;
  otherwise a drag goes on over the new read while its output is there. A watch
  that ends is opened again with `dsplr watch`'s backoff, said in the bar
  ([02](02-architecture.md#process)).
- Text is the system's sans-serif through fontconfig, with its fallbacks
  (crcbl-ui's `system-fonts`, C7), else crcbl's committed `Font::sans`, with a
  note in the bar.
- The panel edits the settings only some desktops have where the backend sets
  them (`Capabilities`): the primary output (a checkbox; making one primary
  takes it from the other, one undo step; locked while checked, since KDE and
  GNOME always have one, and on a mirror, whose source is the one to make
  primary; turning an output off clears its flag, the desktop picking another),
  overscan (a percentage, offered in steps, `OVERSCAN_MAX` at most) and RGB
  range. A field the backend has but did not report for an output (a KWin
  virtual output, a monitor Mutter gives no RGB range) is locked, saying so.
  Hyprland's colour preset (`COLOUR_PRESETS`, plus one Hyprland names that dsplr
  does not) and bit depth (8 or 10) are edited the same way.
- The panel is reached from the keyboard with the `panel` key ([Keys](#keys)),
  driven by crcbl-ui's own focus traversal (tree order, accept, back, arrows);
  `Tab` keeps selecting outputs on the canvas.
- The bar shows the backend thread's last message; the `messages` key shows the
  last `MESSAGE_HISTORY` lines (`dsplr_core::Messages`) over the canvas.
- The confirm dialogs and the identify overlay are one per-monitor panel set
  (`dsplr-gui`'s `panels.rs`), each panel naming the output at its monitor's
  logical position until crcbl reports connectors (C3).

## Still to build in M3

- **The exit check: the owner switches from nwg-displays.** The owner's step.

## Keys

Mouse-first for dragging, but every action has a key. The defaults:

| Key             | Action                                                                                 | `[keys]` name                                  |
| --------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `Tab` / `S-Tab` | select next / previous output                                                          | `select_next` / `select_previous`              |
| `h j k l`       | nudge the selected output by one snap step                                             | `nudge_left` / `_down` / `_up` / `_right`      |
| `H J K L`       | move the selected output to touch its neighbour                                        | `touch_left` / `_down` / `_up` / `_right`      |
| `r` / `R`       | rotate clockwise / counter-clockwise                                                   | `rotate_clockwise` / `rotate_counterclockwise` |
| `+` / `-`       | next / previous scale (and the keypad's)                                               | `scale_up` / `scale_down`                      |
| `space`         | enable / disable (placed beside the others when its old spot overlaps or touches none) | `toggle_enabled`                               |
| `i`             | identify                                                                               | `identify`                                     |
| `p` / `F6`      | keyboard into the panel, and back out                                                  | `panel`                                        |
| `m`             | show / hide the recent messages                                                        | `messages`                                     |
| `a` / `s`       | apply / save                                                                           | `apply` / `save`                               |
| `u` / `C-r`     | undo / redo                                                                            | `undo` / `redo`                                |
| `q` / `Esc`     | quit (asks if there are pending edits)                                                 | `quit`                                         |

Each is rebound in `config.toml`'s `[keys]` table as a list
(`nudge_left = ["Left", "h"]`); an action left out keeps its defaults. A key is
a character, `space`, `F1` to `F12` or a key's name
(`dsplr_config::keys::NAMES`), with `C-`, `A-` or `S-` in front; Shift is part
of a character (`H`, not `S-h`). An unknown action, an unknown key or one key
the file binds to two actions is an error naming it; a key the file binds that
another action has by default goes to the file's action, and the other keeps its
other defaults (none left, it has no key), with a warning (`Keys::warnings`) the
CLI prints and the GUI shows in its bar and message log. The keys of the
dialogs, the quit prompt and the panel (Enter, `Esc`, `y`, `n`, the arrows, Tab)
are not rebound.

**In the panel** (after the `panel` key): focus lands on the selected output's
first widget, or where it was last; `Tab` / `S-Tab` move through the widgets in
order, the arrows move toward a widget that way and choose inside an open list,
Enter or space works the focused widget (a button fires, a checkbox flips, a
list opens, a field starts taking typing) and Enter commits an engaged one,
`Esc` cancels an engaged one or else gives the keyboard back to the canvas, as
the `panel` key does. The canvas's keys do nothing meanwhile. While the messages
are shown, `Esc` hides them before it quits.

## Confirm dialog

**GNOME has no layer-shell**, so there the confirm is a normal toplevel dialog
that GNOME places, and the revert watchdog covers the case where it lands on a
screen that went dark ([02](02-architecture.md#apply-confirm-revert)). This is
the panel set's own fallback (`dsplr-gui`'s `panels.rs`): crcbl-shell reports
`ShellCaps::PANEL` only where the compositor offers `zwlr_layer_shell_v1`
(crcbl-shell 0.18.0, `wayland/mod.rs`), and the confirm kind tries one toplevel
when no panel opens. Not run on GNOME. On Save, GNOME Shell shows its own
confirm dialog instead of dsplr's ([05](05-persistence-and-profiles.md#gnome)).

## Identify overlay

`i` in the window, or `dsplr identify` alone ([07](07-cli.md)): the same panels,
one per monitor, for `IDENTIFY_TIME`. They take no keys. Shown alone, a click on
one closes them early; in the window a click on one does nothing.

On GNOME identify is **unsupported**: the window says so in the bar, as on any
window system without panels. GNOME Shell's `ShowMonitorLabels` (which
gnome-control-center uses) answers only callers owning `org.gnome.Settings`,
`org.gnome.SettingsDaemon.MediaKeys` or
`org.freedesktop.impl.portal.desktop.gnome`, unless Shell runs in unsafe mode
(`DBusSenderChecker` in GNOME Shell 51.1's `js/ui/shellDBus.js` and
`js/misc/util.js`, read, not run). Taking one of those names to get past it was
declined (backlog).

## What crcbl must provide

Checked against crcbl's tree on 2026-11-04 at a source level, and against crcbl
0.18.0 by building on it where the "M3" column says so. Anything missing is
raised in the crcbl repo rather than worked around in dsplr:

| #   | Need                                                         | Seen in crcbl                                                                                             | M3                                                                                                                                                                                                                                                                         |
| --- | ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C1  | Toplevel window on Wayland and X11                           | `WindowRole::Toplevel`; Wayland and X11 shells                                                            | Confirmed on Wayland: the main window runs in headless sway (2026-11-10). X11 not run                                                                                                                                                                                      |
| C2  | A panel on a chosen monitor, on Wayland (not GNOME) and X11  | `WindowRole::Panel` (`wayland/layer.rs`, `x11/panel.rs`)                                                  | Built on 0.18.0: the confirm dialogs and the identify overlay, a layer-shell overlay per monitor, run in headless sway; X11 not run                                                                                                                                        |
| C3  | Monitor enumeration matching the backend's outputs by name   | `crcbl-shell` `MonitorInfo`; whether it carries the connector name is to check                            | Not built on yet; the `connector` field is requested of crcbl (backlog)                                                                                                                                                                                                    |
| C4  | Cross-thread wakeup for the backend thread                   | `crcbl-shell` `waker.rs`                                                                                  | Used: a relay thread wakes the window for each reply from the backend thread; without a waker the loop polls                                                                                                                                                               |
| C5  | Widgets: button, select/list, number entry, toggle, tooltip  | `crcbl-ui` `widgets/` (`button`, `select`, `list`, `number_entry`), `tooltip`                             | Built on 0.17.2, still on 0.18.0: `select`, `checkbox`, `drag_value` (typing into it is the number entry), `text_input`, `tooltip`, and `Ui::enabled` for disabled fields; no plain list or toggle needed. Run in headless sway; never clicked there (no pointer injector) |
| C6  | Free-form drag canvas: draw rects and text, raw pointer drag | `crcbl-ui` `DrawList`, `grid_drag`; whether `grid_drag` fits or the canvas is hand-built is decided at M3 | Hand-built, not `grid_drag`: dsplr-core draws the canvas, its hit regions and the drag (fit frozen, `Layout::snap`); crcbl gives only the raw pointer                                                                                                                      |
| C7  | System font lookup                                           | `crcbl-ui` `system-fonts` feature (pikr precedent)                                                        | Used: the system's sans-serif and its fallbacks, `Font::sans` where there is none                                                                                                                                                                                          |

## Not in the GUI

A preview of the actual screen contents in each rectangle (wdisplays does this
with screencopy). Considered and left for later: it costs a screencopy
dependency and frame copies for little help in arranging.
