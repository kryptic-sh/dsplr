//! The main window's frame as a draw list: the canvas with every output on
//! it, the tray of disabled outputs below, and the bar along the bottom
//! (`docs/plan/06-gui.md`). The property panel is the GUI's, built from
//! [`crate::panel`] with crcbl-ui's widgets.

use dsplr_model::{OutputId, OutputState, Problem, ProblemKind, Severity};

use crate::draw::{Bounds, Color, DrawList, Hit, Point};
use crate::editor::on_canvas;
use crate::fit::{MARGIN, TILE_HEIGHT, fit, rects, regions};
use crate::panel::name;
use crate::{Editor, Theme};

/// A tray tile's width, and the space between two.
const TILE_WIDTH: f32 = 168.0;
const TILE_GAP: f32 = 12.0;
/// Space between a rectangle's edge and its text.
const PAD: f32 = 8.0;
/// Text sizes in pixels per em, and the distance between two lines.
const TITLE_SIZE: f32 = 15.0;
const TEXT_SIZE: f32 = 13.0;
const LINE: f32 = 18.0;
/// Border widths: an output's, the selected output's, a pending or problem
/// outline's.
const BORDER: f32 = 1.0;
const SELECTED_BORDER: f32 = 2.0;
const PENDING_BORDER: f32 = 2.0;
/// What the bar says about the keys, after the backend's name.
const KEYS_HINT: &str = "drag or h j k l move  ·  r rotate  ·  + - scale  ·  space on/off  ·  u / C-r undo / redo  ·  q quit";

/// A window's size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// The frame for `editor` in a window of `viewport`:
///
/// - each output enabled live and with a size as a rectangle at its logical
///   size, all scaled to fit the canvas together (frozen while a drag is
///   under way) and labelled with its connector, description and mode;
/// - each other output as a greyed tile in the tray strip below them, the
///   tiles narrowed to keep them all inside the canvas;
/// - each output whose pending settings differ from its live ones outlined
///   where the pending settings put it, or around its own rectangle or tile
///   when they leave it off the canvas; in the error or warning colour when
///   validating the pending layout found a problem with it;
/// - the bar: `message` when there is one, else the first problem, else the
///   backend and the keys.
///
/// Every rectangle and tile is a hit region for its output, and so is the
/// inside of each pending place on the canvas, below them all: where a
/// pending outline lies over another output's rectangle, the rectangle takes
/// the click.
pub fn draw(editor: &Editor, viewport: Size, theme: &Theme, message: Option<&str>) -> DrawList {
    let mut list = DrawList::default();
    let regions = regions(editor, viewport);
    let problems = editor.pending().validate(&editor.snapshot().capabilities);
    draw_canvas(&mut list, editor, &problems, &regions, theme);
    draw_bar(&mut list, editor, &problems, message, regions.bar, theme);
    list
}

/// The worst problem validation found with each output, by index. An
/// overlap counts against both outputs in it.
fn severities(editor: &Editor, problems: &[Problem]) -> Vec<Option<Severity>> {
    let entries = &editor.pending().entries;
    let index_of = |id: &OutputId| entries.iter().position(|entry| entry.id == *id);
    let mut worst = vec![None; entries.len()];
    for problem in problems {
        let with = match &problem.kind {
            ProblemKind::Overlap { with } => Some(with),
            _ => None,
        };
        for index in problem.output.iter().chain(with).filter_map(index_of) {
            if worst[index] != Some(Severity::Error) {
                worst[index] = Some(problem.severity);
            }
        }
    }
    worst
}

fn draw_canvas(
    list: &mut DrawList,
    editor: &Editor,
    problems: &[Problem],
    regions: &crate::fit::Regions,
    theme: &Theme,
) {
    let (canvas, area) = (regions.canvas, regions.area);
    list.fill(canvas, theme.background);
    let (live, pending) = (editor.live(), editor.pending());
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
    let fit = editor
        .drag_fit()
        .copied()
        .unwrap_or_else(|| fit(editor, area));
    let placed: Vec<_> = rects(pending)
        .map(|(index, rect)| (index, fit.place(rect)))
        .collect();
    let pending_place = |index: usize| {
        placed
            .iter()
            .find(|(placed, _)| *placed == index)
            .map(|&(_, bounds)| bounds)
    };
    let changed = |index: usize| live.entries[index].state != pending.entries[index].state;

    // Lowest first: a pending place, under every rectangle and tile.
    for &(index, bounds) in &placed {
        if changed(index) {
            list.hits.push(Hit {
                bounds,
                output: index,
            });
        }
    }

    // Where each output is drawn, for its outline.
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

    let tray: Vec<usize> = editor
        .order()
        .into_iter()
        .filter(|&index| !on_canvas(&live.entries[index].state))
        .collect();
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

    let severities = severities(editor, problems);
    for index in 0..live.entries.len() {
        let color = match severities[index] {
            Some(Severity::Error) => theme.error,
            Some(Severity::Warning) => theme.warning,
            None if changed(index) => theme.pending,
            None => continue,
        };
        if let Some(bounds) = pending_place(index).or(drawn[index]) {
            list.outline(bounds, PENDING_BORDER, color);
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

/// The bar: `message`, else the first of `problems` and how many more there
/// are, else the backend, whether edits are pending, and the keys.
fn draw_bar(
    list: &mut DrawList,
    editor: &Editor,
    problems: &[Problem],
    message: Option<&str>,
    bar: Bounds,
    theme: &Theme,
) {
    list.fill(bar, theme.surface);
    list.fill(Bounds::new(bar.x, bar.y, bar.width, 1.0), theme.divider);
    let backend = &editor.snapshot().backend;
    let (text, color): (String, Color) = if let Some(message) = message {
        (message.to_owned(), theme.pending)
    } else if let Some(first) = problems.first() {
        let more = match problems.len() - 1 {
            0 => String::new(),
            1 => " (and 1 more problem)".to_owned(),
            more => format!(" (and {more} more problems)"),
        };
        let color = if problems.iter().any(Problem::is_error) {
            theme.error
        } else {
            theme.warning
        };
        (format!("{backend}  ·  {first}{more}"), color)
    } else if editor.has_pending() {
        (
            format!("{backend}  ·  pending edits  ·  {KEYS_HINT}"),
            theme.muted,
        )
    } else {
        (format!("{backend}  ·  {KEYS_HINT}"), theme.muted)
    };
    list.text(
        Point::new(bar.x + 2.0 * PAD, bar.y + (bar.height - LINE) / 2.0),
        text,
        TEXT_SIZE,
        color,
        bar,
    );
}

/// Lines of text down from `bounds`' top-left corner, cut off at its edges.
fn label(list: &mut DrawList, bounds: Bounds, lines: Vec<(String, f32, Color)>) {
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

#[cfg(test)]
mod tests {
    use dsplr_model::{Capabilities, Layout, OutputId};

    use super::{Size, draw};
    use crate::fixtures::{assert_snapshot, owners_machine, three_monitors};
    use crate::{Editor, Point, Snapshot, Theme};

    const WINDOW: Size = Size {
        width: 1280.0,
        height: 800.0,
    };

    #[test]
    fn the_owners_machine() {
        let editor = Editor::new(owners_machine());
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
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
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
        assert_snapshot("three-monitors-pending", &list.to_string());

        // Undone, the outlines go and the frame is the untouched one.
        editor.undo();
        editor.undo();
        let untouched = {
            let mut fresh = Editor::new(three_monitors());
            fresh.select(Some(1));
            draw(&fresh, WINDOW, &Theme::tokyo_night(), None)
        };
        assert_eq!(
            draw(&editor, WINDOW, &Theme::tokyo_night(), None),
            untouched
        );
    }

    #[test]
    fn no_outputs() {
        let editor = Editor::new(Snapshot {
            backend: "sway".into(),
            capabilities: Capabilities::default(),
            layout: Layout::default(),
        });
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
        assert_snapshot("no-outputs", &list.to_string());
    }

    #[test]
    fn a_click_selects_the_output_under_it() {
        let mut editor = Editor::new(three_monitors());
        editor.edit(0, |state| state.y = 0);
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
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
            let list = draw(&editor, window, &Theme::tokyo_night(), None);
            let dp1 = list.hits[0].bounds;
            let dp2 = list.hits[1].bounds;
            // DP-2 is 1440x2560 at 1.25: 1152x2048, beside DP-1's 1920x1080.
            let ratio = |b: crate::Bounds| b.width / b.height;
            assert!((ratio(dp1) - 1920.0 / 1080.0).abs() < 0.01, "{window:?}");
            assert!((ratio(dp2) - 1152.0 / 2048.0).abs() < 0.01, "{window:?}");
            assert!((dp1.right() - dp2.x).abs() < 0.01, "they touch");
            let canvas_right = window.width - crate::PANEL_WIDTH;
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
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
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
        let canvas_right = WINDOW.width - crate::PANEL_WIDTH;
        for disabled in [1, 5, 12, 400] {
            let mut snapshot = three_monitors();
            let template = snapshot.layout.entries[2].clone();
            snapshot.layout.entries.truncate(2);
            for n in 0..disabled {
                let mut entry = template.clone();
                entry.id = OutputId::Connector(format!("DP-{}", n + 10));
                snapshot.layout.entries.push(entry);
            }
            let list = draw(&Editor::new(snapshot), WINDOW, &Theme::tokyo_night(), None);
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

    /// Mid-drag, DP-1 pulled onto DP-2: the canvas keeps its fit, DP-1's
    /// pending place is outlined in the error colour, and the bar says why.
    #[test]
    fn a_drag_onto_another_output() {
        let mut editor = Editor::new(three_monitors());
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
        let dp1 = list.hits.iter().find(|hit| hit.output == 0).unwrap().bounds;
        let at = Point::new(dp1.x + dp1.width / 2.0, dp1.y + dp1.height / 2.0);
        assert!(editor.press(WINDOW, at));
        let scale = editor.drag_fit().unwrap().scale() as f32;
        editor.drag_to(Point::new(at.x + 600.0 * scale, at.y));
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), None);
        let text = list.to_string();
        assert!(text.contains("#f7768e"), "an error outline:\n{text}");
        assert!(text.contains("overlaps DP-1"), "the bar says so:\n{text}");
        assert_snapshot("dragged-overlap", &text);
    }

    /// DP-1 moved clear of DP-2: a gap, which `wlr` allows, so a warning.
    #[test]
    fn a_gap_is_a_warning_where_the_backend_allows_one() {
        let mut editor = Editor::new(three_monitors());
        editor.edit(0, |state| state.x = -500);
        let text = draw(&editor, WINDOW, &Theme::tokyo_night(), None).to_string();
        assert!(text.contains("#e0af68"), "a warning outline:\n{text}");
        assert_snapshot("gap-warning", &text);
    }

    /// A message (a prompt, or why an edit was refused) takes the bar.
    #[test]
    fn a_message_takes_the_bar() {
        let editor = Editor::new(three_monitors());
        let list = draw(&editor, WINDOW, &Theme::tokyo_night(), Some("Quit? y / n"));
        let bar = list.to_string();
        assert!(
            bar.contains(r##"#ff9e64 clip 0.0,768.0 1280.0x32.0 "Quit? y / n""##),
            "{bar}"
        );
    }
}
