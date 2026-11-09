//! The main window's frame as a draw list: the canvas with every output on
//! it, the tray of disabled outputs below, the property panel on the right
//! and the bar along the bottom (`docs/plan/06-gui.md`).

use dsplr_model::{Layout, OutputState, Rect};

use crate::draw::{Bounds, DrawList, Hit, Point};
use crate::editor::on_canvas;
use crate::properties::{name, properties};
use crate::{Editor, Theme};

/// The property panel's width.
pub const PANEL_WIDTH: f32 = 320.0;
/// The bottom bar's height.
pub const BAR_HEIGHT: f32 = 32.0;
/// Space kept clear around the outputs, and around the tray.
const MARGIN: f32 = 24.0;
/// A tray tile's size, and the space between two.
const TILE_WIDTH: f32 = 168.0;
const TILE_HEIGHT: f32 = 52.0;
const TILE_GAP: f32 = 12.0;
/// Space between a rectangle's edge and its text.
const PAD: f32 = 8.0;
/// Text sizes in pixels per em, and the distance between two lines.
const TITLE_SIZE: f32 = 15.0;
const TEXT_SIZE: f32 = 13.0;
const LINE: f32 = 18.0;
/// Border widths: an output's, the selected output's, a pending outline's.
const BORDER: f32 = 1.0;
const SELECTED_BORDER: f32 = 2.0;
const PENDING_BORDER: f32 = 2.0;
/// The property panel's row pitch, and the width of its label column.
const ROW_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 112.0;
/// What the bar says about the keys, after the backend's name.
const KEYS_HINT: &str = "Tab next output  ·  click select  ·  u / C-r undo / redo  ·  q quit";

/// A window's size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// The frame for `editor` in a window of `viewport`:
///
/// - each output enabled live and with a size as a rectangle at its logical
///   size, all scaled to fit the canvas together and labelled with its
///   connector, description and mode;
/// - each other output as a greyed tile in the tray strip below them, the
///   tiles narrowed to keep them all inside the canvas;
/// - each output whose pending settings differ from its live ones outlined
///   where the pending settings put it, or around its own rectangle or tile
///   when they leave it off the canvas;
/// - the selected output's settings in the panel;
/// - the backend and the keys in the bar.
///
/// Every rectangle and tile is also a hit region for its output. A pending
/// outline is not: it is only a border, and over another output's rectangle
/// it would take the clicks meant for that output.
pub fn draw(editor: &Editor, viewport: Size, theme: &Theme) -> DrawList {
    let mut list = DrawList::default();
    let canvas = Bounds::new(
        0.0,
        0.0,
        (viewport.width - PANEL_WIDTH).max(0.0),
        (viewport.height - BAR_HEIGHT).max(0.0),
    );
    let panel = Bounds::new(
        canvas.right(),
        0.0,
        viewport.width - canvas.width,
        canvas.height,
    );
    let bar = Bounds::new(
        0.0,
        canvas.height,
        viewport.width,
        viewport.height - canvas.height,
    );
    draw_canvas(&mut list, editor, canvas, theme);
    draw_panel(&mut list, editor, panel, theme);
    draw_bar(&mut list, editor, bar, theme);
    list
}

fn draw_canvas(list: &mut DrawList, editor: &Editor, canvas: Bounds, theme: &Theme) {
    list.fill(canvas, theme.background);
    let (live, pending) = (editor.live(), editor.pending());
    let tray: Vec<usize> = editor
        .order()
        .into_iter()
        .filter(|&index| !on_canvas(&live.entries[index].state))
        .collect();
    let tray_band = if tray.is_empty() {
        0.0
    } else {
        TILE_HEIGHT + MARGIN
    };
    let area = Bounds::new(
        canvas.x + MARGIN,
        canvas.y + MARGIN,
        (canvas.width - 2.0 * MARGIN).max(0.0),
        (canvas.height - 2.0 * MARGIN - tray_band).max(0.0),
    );
    if live.entries.is_empty() {
        list.text(
            Point::new(area.x, area.y),
            "No outputs",
            TITLE_SIZE,
            theme.muted,
            canvas,
        );
        return;
    }
    let fit = Fit::new(
        rects(live).chain(rects(pending)).map(|(_, rect)| rect),
        area,
    );

    // Where each output is drawn, for its pending outline.
    let mut drawn = vec![None; live.entries.len()];
    for (index, rect) in rects(live) {
        let bounds = fit.place(rect);
        drawn[index] = Some(bounds);
        let selected = editor.selected() == Some(index);
        let (fill, border, thickness) = if selected {
            (theme.output_selected, theme.accent, SELECTED_BORDER)
        } else {
            (theme.output, theme.border, BORDER)
        };
        list.fill(bounds, fill);
        list.outline(bounds, thickness, border);
        let entry = &live.entries[index];
        let mut lines = vec![(name(entry), TITLE_SIZE, theme.text)];
        if let Some(description) = entry.description() {
            lines.push((description, TEXT_SIZE, theme.text_dim));
        }
        lines.push((mode_line(&entry.state), TEXT_SIZE, theme.text_dim));
        label(list, bounds, lines);
        list.hits.push(Hit {
            bounds,
            output: index,
        });
    }

    let tiles_top = canvas.bottom() - MARGIN - TILE_HEIGHT;
    let (pitch, tile_width) = tray_pitch(tray.len(), area.width);
    for (slot, &index) in tray.iter().enumerate() {
        let bounds = Bounds::new(
            area.x + slot as f32 * pitch,
            tiles_top,
            tile_width,
            TILE_HEIGHT,
        );
        drawn[index] = Some(bounds);
        let selected = editor.selected() == Some(index);
        let (border, thickness) = if selected {
            (theme.accent, SELECTED_BORDER)
        } else {
            (theme.tray_border, BORDER)
        };
        list.fill(bounds, theme.tray);
        list.outline(bounds, thickness, border);
        let entry = &live.entries[index];
        let state = if entry.state.enabled {
            "no mode"
        } else {
            "disabled"
        };
        label(
            list,
            bounds,
            vec![
                (name(entry), TITLE_SIZE, theme.text_dim),
                (state.into(), TEXT_SIZE, theme.muted),
            ],
        );
        list.hits.push(Hit {
            bounds,
            output: index,
        });
    }

    let placed: Vec<_> = rects(pending).collect();
    for (index, (was, now)) in live.entries.iter().zip(&pending.entries).enumerate() {
        if was.state == now.state {
            continue;
        }
        let bounds = placed
            .iter()
            .find(|(placed, _)| *placed == index)
            .map(|&(_, rect)| fit.place(rect))
            .or(drawn[index]);
        if let Some(bounds) = bounds {
            list.outline(bounds, PENDING_BORDER, theme.pending);
        }
    }
}

/// The distance from one tray tile to the next and a tile's width, for
/// `tiles` tiles in a row `width` wide: full-sized where they fit, else
/// narrowed (the gap first kept, then given up too) so the last one still
/// ends inside the row.
fn tray_pitch(tiles: usize, width: f32) -> (f32, f32) {
    let full = TILE_WIDTH + TILE_GAP;
    let pitch = if tiles == 0 {
        full
    } else {
        full.min((width + TILE_GAP) / tiles as f32)
    };
    (pitch, (pitch - TILE_GAP).max(0.0))
}

fn draw_panel(list: &mut DrawList, editor: &Editor, panel: Bounds, theme: &Theme) {
    list.fill(panel, theme.surface);
    list.fill(
        Bounds::new(panel.x, panel.y, 1.0, panel.height),
        theme.divider,
    );
    let clip = panel.inset(PAD);
    let left = panel.x + 2.0 * PAD;
    let rows = editor
        .selected()
        .and_then(|index| properties(editor, index));
    let Some(rows) = rows else {
        list.text(
            Point::new(left, panel.y + 2.0 * PAD),
            "No output selected",
            TEXT_SIZE,
            theme.muted,
            clip,
        );
        return;
    };
    for (row, property) in rows.iter().enumerate() {
        let y = panel.y + 2.0 * PAD + row as f32 * ROW_HEIGHT;
        list.text(
            Point::new(left, y),
            property.label,
            TEXT_SIZE,
            theme.muted,
            clip,
        );
        let color = if property.changed {
            theme.pending
        } else if property.supported {
            theme.text
        } else {
            theme.muted
        };
        list.text(
            Point::new(left + LABEL_WIDTH, y),
            property.value.clone(),
            TEXT_SIZE,
            color,
            clip,
        );
    }
}

fn draw_bar(list: &mut DrawList, editor: &Editor, bar: Bounds, theme: &Theme) {
    list.fill(bar, theme.surface);
    list.fill(Bounds::new(bar.x, bar.y, bar.width, 1.0), theme.divider);
    let backend = &editor.snapshot().backend;
    let text = if editor.has_pending() {
        format!("{backend}  ·  pending edits  ·  {KEYS_HINT}")
    } else {
        format!("{backend}  ·  {KEYS_HINT}")
    };
    list.text(
        Point::new(bar.x + 2.0 * PAD, bar.y + (bar.height - LINE) / 2.0),
        text,
        TEXT_SIZE,
        theme.muted,
        bar,
    );
}

/// Lines of text down from `bounds`' top-left corner, cut off at its edges.
fn label(list: &mut DrawList, bounds: Bounds, lines: Vec<(String, f32, crate::Color)>) {
    for (line, (text, size, color)) in lines.into_iter().enumerate() {
        let at = Point::new(bounds.x + PAD, bounds.y + PAD + line as f32 * LINE);
        list.text(at, text, size, color, bounds);
    }
}

/// The mode as [`Mode::describe`](dsplr_model::Mode::describe) writes it,
/// with the scale after it when it is not 1, or `no mode`.
fn mode_line(state: &OutputState) -> String {
    let Some(mode) = &state.mode else {
        return "no mode".into();
    };
    let line = mode.describe();
    if state.scale == 1.0 {
        line
    } else {
        format!("{line}, scale {}", state.scale)
    }
}

/// The index and rectangle of each of `layout`'s outputs on the canvas.
fn rects(layout: &Layout) -> impl Iterator<Item = (usize, Rect)> + '_ {
    layout
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| on_canvas(&entry.state))
        .filter_map(|(index, entry)| Some((index, Rect::of(entry)?)))
}

/// The scale and offset that fit a set of layout rectangles, as a whole,
/// inside an area of the window, centred in it.
struct Fit {
    /// Window pixels per layout pixel.
    scale: f64,
    /// The layout point drawn at `origin`.
    min: (i64, i64),
    origin: (f64, f64),
}

impl Fit {
    fn new(rects: impl Iterator<Item = Rect>, area: Bounds) -> Self {
        let mut extent: Option<(i64, i64, i64, i64)> = None;
        for rect in rects {
            let (left, top, right, bottom) =
                extent.unwrap_or((rect.x, rect.y, rect.x + rect.width, rect.y + rect.height));
            extent = Some((
                left.min(rect.x),
                top.min(rect.y),
                right.max(rect.x + rect.width),
                bottom.max(rect.y + rect.height),
            ));
        }
        let (left, top, right, bottom) = extent.unwrap_or_default();
        // An empty extent (nothing on the canvas, or only zero-sized
        // outputs) is taken as one pixel, so the scale stays finite.
        let width = (right - left).max(1) as f64;
        let height = (bottom - top).max(1) as f64;
        let scale = (f64::from(area.width) / width).min(f64::from(area.height) / height);
        Self {
            scale,
            min: (left, top),
            origin: (
                f64::from(area.x) + (f64::from(area.width) - width * scale) / 2.0,
                f64::from(area.y) + (f64::from(area.height) - height * scale) / 2.0,
            ),
        }
    }

    fn place(&self, rect: Rect) -> Bounds {
        let x = self.origin.0 + (rect.x - self.min.0) as f64 * self.scale;
        let y = self.origin.1 + (rect.y - self.min.1) as f64 * self.scale;
        Bounds::new(
            x as f32,
            y as f32,
            (rect.width as f64 * self.scale) as f32,
            (rect.height as f64 * self.scale) as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dsplr_model::{
        AdaptiveSync, Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState,
        Transform,
    };

    use super::{Size, draw};
    use crate::{Editor, Point, Snapshot, Theme};

    const WINDOW: Size = Size {
        width: 1280.0,
        height: 800.0,
    };

    /// Compares `list`'s text form with `src/snapshots/<name>.txt`. With
    /// `DSPLR_BLESS` set it writes the file instead, for a change to the
    /// canvas that is meant, and fails all the same: a run that rewrote its
    /// snapshots has checked nothing, so it must be run again without it.
    /// Read the diff before committing the files.
    fn assert_snapshot(name: &str, actual: &str) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/snapshots")
            .join(format!("{name}.txt"));
        if std::env::var_os("DSPLR_BLESS").is_some() {
            std::fs::write(&path, actual).unwrap();
            panic!(
                "{name}: snapshot rewritten at {}; re-run without DSPLR_BLESS",
                path.display()
            );
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            actual == expected,
            "{name}: the draw list differs from {}\n--- actual\n{actual}",
            path.display()
        );
    }

    fn output(connector: &str, description: &str, modes: Vec<Mode>) -> Output {
        Output {
            connector: connector.into(),
            description: description.into(),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            physical_size_mm: None,
            modes,
        }
    }

    /// The owner's machine: the laptop panel off, one monitor at 2560x1440
    /// and 120 Hz.
    fn owners_machine() -> Snapshot {
        Snapshot {
            backend: "hyprland".into(),
            capabilities: Capabilities {
                mirroring: true,
                adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
                fractional_scale: true,
                per_output_scale: true,
                gaps_allowed: true,
                colour_preset: true,
                bit_depth: true,
                ..Capabilities::default()
            },
            layout: Layout {
                entries: vec![
                    OutputEntry {
                        id: OutputId::Connector("eDP-1".into()),
                        output: Some(output(
                            "eDP-1",
                            "BOE 0x0BCA",
                            vec![Mode::new(1920, 1080, 60_000)],
                        )),
                        state: OutputState {
                            enabled: false,
                            ..OutputState::default()
                        },
                    },
                    OutputEntry {
                        id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                        output: Some(output(
                            "HDMI-A-1",
                            "AOC Q27G4 SERIAL0000001",
                            vec![Mode::new(2560, 1440, 120_000)],
                        )),
                        state: OutputState {
                            mode: Some(Mode::new(2560, 1440, 120_000)),
                            adaptive_sync: Some(AdaptiveSync::Off),
                            ..OutputState::default()
                        },
                    },
                ],
            },
        }
    }

    /// Three monitors side by side: one turned and scaled, one disabled, and
    /// no hardware facts for any (as a layout read from a file).
    fn three_monitors() -> Snapshot {
        let entry = |connector: &str, state| OutputEntry {
            id: OutputId::Connector(connector.into()),
            output: None,
            state,
        };
        Snapshot {
            backend: "sway".into(),
            capabilities: Capabilities::default(),
            layout: Layout {
                entries: vec![
                    entry(
                        "DP-1",
                        OutputState {
                            mode: Some(Mode::new(1920, 1080, 60_000)),
                            x: 0,
                            y: 360,
                            ..OutputState::default()
                        },
                    ),
                    entry(
                        "DP-2",
                        OutputState {
                            mode: Some(Mode::new(2560, 1440, 144_000)),
                            x: 1920,
                            scale: 1.25,
                            transform: Transform::Rotate90,
                            ..OutputState::default()
                        },
                    ),
                    entry(
                        "HDMI-A-1",
                        OutputState {
                            enabled: false,
                            mode: Some(Mode::new(1920, 1080, 60_000)),
                            ..OutputState::default()
                        },
                    ),
                ],
            },
        }
    }

    #[test]
    fn the_owners_machine() {
        let editor = Editor::new(owners_machine());
        let list = draw(&editor, WINDOW, &Theme::tokyo_night());
        assert_snapshot("owners-machine", &list.to_string());
    }

    #[test]
    fn three_monitors_with_pending_edits() {
        let mut editor = Editor::new(three_monitors());
        // DP-1 moved up, the disabled one turned on beside DP-2.
        editor.edit(0, |state| state.y = 0);
        editor.edit(2, |state| {
            state.enabled = true;
            state.x = 3072;
        });
        editor.select(Some(1));
        let list = draw(&editor, WINDOW, &Theme::tokyo_night());
        assert_snapshot("three-monitors-pending", &list.to_string());

        // Undone, the outlines go and the frame is the untouched one.
        editor.undo();
        editor.undo();
        let untouched = {
            let mut fresh = Editor::new(three_monitors());
            fresh.select(Some(1));
            draw(&fresh, WINDOW, &Theme::tokyo_night())
        };
        assert_eq!(draw(&editor, WINDOW, &Theme::tokyo_night()), untouched);
    }

    #[test]
    fn no_outputs() {
        let editor = Editor::new(Snapshot {
            backend: "sway".into(),
            capabilities: Capabilities::default(),
            layout: Layout::default(),
        });
        let list = draw(&editor, WINDOW, &Theme::tokyo_night());
        assert_snapshot("no-outputs", &list.to_string());
    }

    #[test]
    fn a_click_selects_the_output_under_it() {
        let mut editor = Editor::new(three_monitors());
        editor.edit(0, |state| state.y = 0);
        let list = draw(&editor, WINDOW, &Theme::tokyo_night());
        let centre = |output: usize| {
            let hit = list.hits.iter().rev().find(|hit| hit.output == output);
            let bounds = hit.unwrap().bounds;
            Point::new(
                bounds.x + bounds.width / 2.0,
                bounds.y + bounds.height / 2.0,
            )
        };
        for output in 0..3 {
            assert_eq!(list.hit(centre(output)), Some(output), "output {output}");
        }
        assert_eq!(list.hit(Point::new(1.0, 1.0)), None, "the canvas margin");
        assert_eq!(list.hit(Point::new(1270.0, 100.0)), None, "the panel");
    }

    /// Every output on the canvas keeps its proportions and stays inside it,
    /// in a window of any shape.
    #[test]
    fn outputs_fit_the_canvas_at_their_proportions() {
        let editor = Editor::new(three_monitors());
        for window in [
            WINDOW,
            Size {
                width: 700.0,
                height: 1200.0,
            },
            Size {
                width: 2000.0,
                height: 300.0,
            },
        ] {
            let list = draw(&editor, window, &Theme::tokyo_night());
            let dp1 = list.hits[0].bounds;
            let dp2 = list.hits[1].bounds;
            // DP-2 is 1440x2560 at 1.25: 1152x2048, beside DP-1's 1920x1080.
            let ratio = |b: crate::Bounds| b.width / b.height;
            assert!((ratio(dp1) - 1920.0 / 1080.0).abs() < 0.01, "{window:?}");
            assert!((ratio(dp2) - 1152.0 / 2048.0).abs() < 0.01, "{window:?}");
            assert!((dp1.right() - dp2.x).abs() < 0.01, "they touch");
            let canvas_right = window.width - super::PANEL_WIDTH;
            for bounds in [dp1, dp2] {
                assert!(
                    bounds.x >= 0.0 && bounds.right() <= canvas_right,
                    "{window:?}"
                );
                assert!(bounds.y >= 0.0, "{window:?}");
            }
        }
    }

    /// A pending outline is only a border: a click inside it, on another
    /// output's rectangle, still selects that output.
    #[test]
    fn a_pending_outline_does_not_take_another_output_s_clicks() {
        let mut editor = Editor::new(three_monitors());
        // DP-1 moved onto DP-2.
        editor.edit(0, |state| state.x = 1920);
        let list = draw(&editor, WINDOW, &Theme::tokyo_night());
        let dp2 = list.hits.iter().find(|hit| hit.output == 1).unwrap().bounds;
        // DP-2's centre (layout 2496,1024) is inside DP-1's pending place
        // (1920,360 to 3840,1440).
        let centre = Point::new(dp2.x + dp2.width / 2.0, dp2.y + dp2.height / 2.0);
        assert_eq!(list.hit(centre), Some(1));
    }

    /// However many outputs are disabled, their tiles stay inside the
    /// canvas, so none sits under the panel taking its clicks.
    #[test]
    fn tray_tiles_stay_inside_the_canvas() {
        let canvas_right = WINDOW.width - super::PANEL_WIDTH;
        for disabled in [1, 5, 12, 400] {
            let mut snapshot = three_monitors();
            let template = snapshot.layout.entries[2].clone();
            snapshot.layout.entries.truncate(2);
            for n in 0..disabled {
                let mut entry = template.clone();
                entry.id = OutputId::Connector(format!("DP-{}", n + 10));
                snapshot.layout.entries.push(entry);
            }
            let list = draw(&Editor::new(snapshot), WINDOW, &Theme::tokyo_night());
            let tiles: Vec<_> = list.hits.iter().filter(|hit| hit.output >= 2).collect();
            assert_eq!(tiles.len(), disabled, "a hit region per tile");
            for tile in tiles {
                assert!(
                    tile.bounds.x >= 0.0 && tile.bounds.right() <= canvas_right,
                    "{disabled} disabled: {:?}",
                    tile.bounds
                );
            }
            for y in [700.0, 720.0, 740.0] {
                assert_eq!(list.hit(Point::new(canvas_right + 5.0, y)), None);
            }
        }
    }
}
