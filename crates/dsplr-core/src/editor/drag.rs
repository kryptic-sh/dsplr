//! Dragging an output on the canvas, with snapping
//! (`docs/plan/03-display-model.md#snapping`).
//!
//! A press on an output selects it, and on one placed on the canvas starts a
//! drag. The canvas's fit is frozen for the drag, so the outputs do not
//! rescale under the pointer as the moving one changes the layout's extent.
//! Each move snaps through `Layout::snap`; the release moves the layout off
//! negative positions (`Layout::make_non_negative`), which moves no other
//! output unless the drag left one negative, or, where the backend needs the
//! layout at the origin, to the origin (`Layout::normalise`), and records
//! the whole drag as one undo step.

use dsplr_model::{Layout, PositionOutOfRange};

use super::{Editor, on_canvas};
use crate::fit::{Fit, fit, regions};
use crate::{Point, Size, Theme, draw};

/// How close, in window pixels, a dragged output's edge comes to another
/// output's before it snaps there.
pub const SNAP_DISTANCE: f32 = 12.0;

/// A drag under way.
#[derive(Debug, Clone)]
pub struct Drag {
    output: usize,
    /// The pending layout when the drag began: what a cancel restores and
    /// the undo step records.
    before: Layout,
    /// The pointer's offset from the output's top-left corner, in layout
    /// pixels.
    grab: (f64, f64),
    fit: Fit,
}

impl Drag {
    /// The output being dragged.
    pub(super) const fn output(&self) -> usize {
        self.output
    }

    /// What a cancel restores.
    pub(super) const fn before(&self) -> &Layout {
        &self.before
    }

    /// The same drag, its grab and frozen fit, over a layout read again:
    /// of `output`, cancelling to `before`.
    pub(super) fn carried(self, output: usize, before: Layout) -> Self {
        Self {
            output,
            before,
            ..self
        }
    }
}

impl Editor {
    /// A press of the primary button at `at` in a window of `viewport`:
    /// selects the output there, and starts dragging it when it is placed on
    /// the canvas. Returns whether it hit an output.
    pub fn press(&mut self, viewport: Size, at: Point) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let Some(output) = draw(self, viewport, &Theme::default(), None, "").hit(at) else {
            return false;
        };
        self.select(Some(output));
        let state = &self.pending.entries[output].state;
        if on_canvas(state) {
            let fit = fit(self, regions(self, viewport).area);
            let (x, y) = fit.layout_point(at);
            self.drag = Some(Drag {
                output,
                before: self.pending.clone(),
                grab: (x - f64::from(state.x), y - f64::from(state.y)),
                fit,
            });
        }
        true
    }

    /// The pointer moved to `at` during a drag: moves the output with it,
    /// snapped to the others. Returns whether it moved.
    pub fn drag_to(&mut self, at: Point) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        let (x, y) = drag.fit.layout_point(at);
        // Clamped into i32's range first, so the cast is exact.
        let position = |layout: f64, grab: f64| {
            (layout - grab)
                .round()
                .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        };
        let (x, y) = (position(x, drag.grab.0), position(y, drag.grab.1));
        // A non-negative whole number, held inside u32's range.
        let threshold = (f64::from(SNAP_DISTANCE) / drag.fit.scale())
            .round()
            .min(f64::from(u32::MAX)) as u32;
        let output = drag.output;
        let (x, y) = self.pending.snap(output, x, y, threshold);
        let state = &mut self.pending.entries[output].state;
        if (state.x, state.y) == (x, y) {
            return false;
        }
        (state.x, state.y) = (x, y);
        true
    }

    /// The button came up: ends the drag, lifting the layout off negative
    /// positions, or moving it to the origin where the backend needs that,
    /// and recording the whole drag as one undo step when it moved anything.
    /// Returns whether it did; a layout that cannot be moved so without a
    /// position leaving `i32` is put back as it was.
    pub fn release(&mut self) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        let placed = if self.snapshot.capabilities.origin_required {
            self.pending.normalise()
        } else {
            self.pending.make_non_negative()
        };
        if let Err(PositionOutOfRange { .. }) = placed {
            // Moving the layout would put a position outside i32: the drag
            // is refused, as no desktop could take the layout.
            self.pending = drag.before;
            return false;
        }
        if self.pending == drag.before {
            return false;
        }
        self.undo.push(drag.before);
        self.redo.clear();
        true
    }

    /// Ends the drag with the output back where it was. Returns whether there
    /// was a drag.
    pub fn cancel_drag(&mut self) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        self.pending = drag.before;
        true
    }

    /// The output being dragged.
    pub fn dragging(&self) -> Option<usize> {
        self.drag.as_ref().map(|drag| drag.output)
    }

    /// The canvas's fit, frozen for the drag under way.
    pub fn drag_fit(&self) -> Option<&Fit> {
        self.drag.as_ref().map(|drag| &drag.fit)
    }
}

#[cfg(test)]
mod tests {
    use super::SNAP_DISTANCE;
    use crate::fixtures::three_monitors;
    use crate::{Editor, Point, Size, Theme, draw};

    const WINDOW: Size = Size {
        width: 1280.0,
        height: 800.0,
    };

    /// The centre of where `output` is drawn.
    fn centre(editor: &Editor, output: usize) -> Point {
        let list = draw(editor, WINDOW, &Theme::default(), None, "");
        let hit = list
            .hits
            .iter()
            .rev()
            .find(|hit| hit.output == output)
            .unwrap();
        let b = hit.bounds;
        Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0)
    }

    /// Presses on `output`'s centre and moves the pointer by `dx`, `dy`
    /// layout pixels.
    fn drag(editor: &mut Editor, output: usize, dx: f64, dy: f64) {
        let at = centre(editor, output);
        assert!(editor.press(WINDOW, at));
        assert_eq!(editor.dragging(), Some(output));
        let scale = editor.drag_fit().unwrap().scale();
        let to = Point::new(at.x + (dx * scale) as f32, at.y + (dy * scale) as f32);
        editor.drag_to(to);
    }

    fn position(editor: &Editor, output: usize) -> (i32, i32) {
        let state = &editor.pending().entries[output].state;
        (state.x, state.y)
    }

    #[test]
    fn a_press_selects_and_drags_only_an_output_on_the_canvas() {
        let mut editor = three_monitors_editor();
        assert!(editor.press(WINDOW, centre(&editor, 2)), "the tray tile");
        assert_eq!(editor.selected(), Some(2));
        assert_eq!(editor.dragging(), None, "a tile is not dragged");
        assert!(!editor.press(WINDOW, Point::new(2.0, 2.0)), "the margin");
        assert!(editor.press(WINDOW, centre(&editor, 0)));
        assert_eq!((editor.selected(), editor.dragging()), (Some(0), Some(0)));
    }

    fn three_monitors_editor() -> Editor {
        Editor::new(three_monitors())
    }

    #[test]
    fn a_drag_snaps_an_edge_within_the_snap_distance() {
        // DP-1 starts at y 360; DP-2's top is at 0.
        let mut editor = three_monitors_editor();
        drag(&mut editor, 0, 0.0, -330.0);
        let threshold = f64::from(SNAP_DISTANCE) / editor.drag_fit().unwrap().scale();
        assert!(
            threshold > 30.0,
            "the test's 30 pixels are inside it: {threshold}"
        );
        assert_eq!(
            position(&editor, 0),
            (0, 0),
            "30 pixels off: snapped to DP-2's top"
        );
        editor.cancel_drag();

        drag(&mut editor, 0, 0.0, -200.0);
        let (x, y) = position(&editor, 0);
        assert_eq!(x, 0);
        assert!(
            (159..=161).contains(&y),
            "far from any edge, not snapped: {y}"
        );
    }

    #[test]
    fn a_release_lifts_the_layout_off_negative_positions_in_one_undo_step() {
        let mut editor = three_monitors_editor();
        drag(&mut editor, 0, -500.0, 0.0);
        assert!(editor.release());
        assert_eq!(editor.dragging(), None);
        // DP-1 went 500 left, to -500: everything moves 500 right, and only
        // along x.
        assert_eq!(position(&editor, 0).0, 0);
        assert_eq!(position(&editor, 1), (2420, 0));
        assert_eq!(position(&editor, 0).1, 360);
        assert!(editor.undo());
        assert!(!editor.has_pending(), "the whole drag undone at once");
        assert!(!editor.undo());
    }

    /// headless sway's layout starts at x 1920: a drag that leaves nothing
    /// negative changes the dragged output and no other.
    #[test]
    fn a_drag_away_from_the_origin_changes_only_the_dragged_output() {
        let mut snapshot = three_monitors();
        for entry in &mut snapshot.layout.entries {
            entry.state.x += 1920;
        }
        let mut editor = Editor::new(snapshot);
        drag(&mut editor, 0, 0.0, 200.0);
        assert!(editor.release());
        let (live, pending) = (editor.live(), editor.pending());
        assert_eq!(position(&editor, 0), (1920, 560));
        for index in 1..live.entries.len() {
            assert_eq!(
                live.entries[index], pending.entries[index],
                "output {index} moved"
            );
        }
    }

    /// Where the backend needs the layout at the origin (GNOME), the release
    /// moves it there: the same drag as above ends with the layout at 0,0.
    #[test]
    fn a_release_moves_the_layout_to_the_origin_where_the_backend_needs_it() {
        let mut snapshot = three_monitors();
        snapshot.capabilities.origin_required = true;
        let mut editor = Editor::new(snapshot);
        // DP-1 (0,360) down 200: DP-2 keeps the top at 0, DP-1 x 0.
        drag(&mut editor, 0, 0.0, 200.0);
        assert!(editor.release());
        assert_eq!(position(&editor, 0), (0, 560));
        // DP-1 right past DP-2 (at 1920, 1152 wide): DP-2 becomes the
        // leftmost and moves to x 0, DP-1 by the same.
        drag(&mut editor, 0, 3072.0, 0.0);
        assert!(editor.release());
        assert_eq!(position(&editor, 1), (0, 0));
        assert!(position(&editor, 0).0 > 0, "{:?}", position(&editor, 0));
        assert!(editor.undo());
        assert_eq!(position(&editor, 0), (0, 560));
    }

    #[test]
    fn a_cancelled_drag_puts_it_back_and_records_nothing() {
        let mut editor = three_monitors_editor();
        drag(&mut editor, 0, -500.0, 0.0);
        assert!(editor.has_pending());
        assert!(
            !editor.edit(1, |state| state.x = 0),
            "no edit during a drag"
        );
        assert!(!editor.undo(), "no undo during a drag");
        assert!(editor.cancel_drag());
        assert!(!editor.has_pending());
        assert!(!editor.undo());
        assert!(!editor.release(), "no drag left to end");
    }

    #[test]
    fn a_drag_that_moved_nothing_records_nothing() {
        let mut editor = three_monitors_editor();
        assert!(editor.press(WINDOW, centre(&editor, 0)));
        assert!(!editor.release());
        assert!(!editor.undo());
    }

    #[test]
    fn a_drag_goes_on_over_a_new_read_of_the_desktop() {
        let mut editor = three_monitors_editor();
        drag(&mut editor, 0, -200.0, 0.0);
        let dragged = position(&editor, 0);
        assert_ne!(dragged, (0, 360));

        // Nothing changed (a reload, a stray event): the drag is untouched.
        let same = editor.snapshot().clone();
        editor.rebase(same, true);
        assert_eq!(editor.dragging(), Some(0));
        assert_eq!(position(&editor, 0), dragged);

        // DP-2 changed elsewhere and listed first now: the drag goes on, on
        // DP-1 where it now is in the list.
        let mut read = editor.snapshot().clone();
        read.layout.entries.swap(0, 1);
        read.layout.entries[0].state.scale = 1.5;
        editor.rebase(read, true);
        assert_eq!(editor.dragging(), Some(1));
        assert_eq!(position(&editor, 1), dragged, "where it was dragged");
        assert!(editor.cancel_drag());
        assert_eq!(position(&editor, 1), (0, 360), "a cancel puts it back");
        assert_eq!(
            editor.pending().entries[0].state.scale,
            1.5,
            "over the new read"
        );

        // Its output gone: the drag ends.
        let mut editor = three_monitors_editor();
        drag(&mut editor, 0, -200.0, 0.0);
        let mut read = editor.snapshot().clone();
        read.layout.entries.remove(0);
        editor.rebase(read, true);
        assert_eq!(editor.dragging(), None);
        assert!(!editor.has_pending(), "its edit went with it");
    }
}
