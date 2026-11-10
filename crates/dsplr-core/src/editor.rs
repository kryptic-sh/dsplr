//! The editor's state: the live layout the backend reported, the pending
//! layout the user is editing over it, undo and redo over the pending edits,
//! and which output is selected. The edits themselves are in `edits`,
//! dragging on the canvas in `drag`, and taking a new read of the desktop
//! under the pending edits in `rebase`.

mod drag;
mod edits;
mod rebase;

use dsplr_model::{Capabilities, Layout, OutputState};

pub use drag::SNAP_DISTANCE;
pub use edits::{Direction, NUDGE_STEP, scale_choices};
pub use rebase::Rebased;

/// The desktop's state as the backend thread reports it
/// (`docs/plan/02-architecture.md`): the layout, and what the backend that
/// read it is called and can do.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The backend's name, as `dsplr backend` prints it.
    pub backend: String,
    pub capabilities: Capabilities,
    pub layout: Layout,
}

/// The live layout and the pending edits over it.
///
/// The pending layout starts as a copy of the live one and has the same
/// entries in the same order, so an output is named by its index in either.
/// Every change to it goes through [`Editor::edit`], which records an undo
/// step, or through a drag, which records one when it ends.
#[derive(Debug, Clone)]
pub struct Editor {
    snapshot: Snapshot,
    pending: Layout,
    /// Pending layouts before each edit, the latest last.
    undo: Vec<Layout>,
    /// Pending layouts undone, the latest undone last.
    redo: Vec<Layout>,
    selected: Option<usize>,
    /// The drag under way, if any.
    drag: Option<drag::Drag>,
}

impl Editor {
    /// An editor with no pending edits over `snapshot`, its first output in
    /// [`Editor::order`] selected.
    pub fn new(snapshot: Snapshot) -> Self {
        let mut editor = Self {
            pending: snapshot.layout.clone(),
            snapshot,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            drag: None,
        };
        editor.selected = editor.order().first().copied();
        editor
    }

    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The layout as the desktop has it.
    pub const fn live(&self) -> &Layout {
        &self.snapshot.layout
    }

    /// The layout with the pending edits.
    pub const fn pending(&self) -> &Layout {
        &self.pending
    }

    /// Whether the pending layout differs from the live one. Edits undone
    /// back to the live state leave nothing pending.
    pub fn has_pending(&self) -> bool {
        self.pending != self.snapshot.layout
    }

    /// Changes output `index`'s pending settings with `change`, recording an
    /// undo step and dropping the redo history when it changed anything.
    /// Returns whether it did; an index with no output, or an edit during a
    /// drag, changes nothing.
    pub fn edit(&mut self, index: usize, change: impl FnOnce(&mut OutputState)) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let Some(entry) = self.pending.entries.get(index) else {
            return false;
        };
        let mut state = entry.state.clone();
        change(&mut state);
        if state == entry.state {
            return false;
        }
        self.undo.push(self.pending.clone());
        self.redo.clear();
        self.pending.entries[index].state = state;
        true
    }

    /// Goes back one edit. Returns whether there was one; during a drag
    /// there is none.
    pub fn undo(&mut self) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.pending, previous));
        true
    }

    /// Goes forward one undone edit. Returns whether there was one; during a
    /// drag there is none.
    pub fn redo(&mut self) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.pending, next));
        true
    }

    /// The selected output's index.
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects output `index`, or nothing; an index with no output selects
    /// nothing.
    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&index| index < self.pending.entries.len());
    }

    /// Selects the output after the selected one in [`Editor::order`],
    /// wrapping round; the first when none is selected.
    pub fn select_next(&mut self) {
        self.step_selection(true);
    }

    /// Selects the output before the selected one in [`Editor::order`],
    /// wrapping round; the last when none is selected.
    pub fn select_previous(&mut self) {
        self.step_selection(false);
    }

    fn step_selection(&mut self, forward: bool) {
        let order = self.order();
        let Some(&first) = order.first() else {
            return;
        };
        let at = self
            .selected
            .and_then(|selected| order.iter().position(|&index| index == selected));
        let len = order.len();
        self.selected = Some(match (at, forward) {
            (Some(at), true) => order[(at + 1) % len],
            (Some(at), false) => order[(at + len - 1) % len],
            (None, true) => first,
            (None, false) => order[len - 1],
        });
    }

    /// The outputs in the order Tab steps through them: those on the canvas
    /// left to right (top to bottom where they share a left edge), then those
    /// in the tray, each group in the backend's order otherwise.
    pub fn order(&self) -> Vec<usize> {
        let mut placed = Vec::new();
        let mut tray = Vec::new();
        for (index, entry) in self.live().entries.iter().enumerate() {
            if on_canvas(&entry.state) {
                placed.push((entry.state.x, entry.state.y, index));
            } else {
                tray.push(index);
            }
        }
        placed.sort_unstable();
        placed
            .into_iter()
            .map(|(_, _, index)| index)
            .chain(tray)
            .collect()
    }
}

/// Whether an output with these settings has a rectangle on the canvas: it
/// is enabled and has a size. The others sit in the tray.
pub fn on_canvas(state: &OutputState) -> bool {
    state.enabled && state.logical_size().is_some()
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Capabilities, Layout, Mode, OutputEntry, OutputId, OutputState};

    use super::{Editor, Snapshot};

    fn entry(connector: &str, enabled: bool, x: i32) -> OutputEntry {
        OutputEntry {
            id: OutputId::Connector(connector.into()),
            output: None,
            state: OutputState {
                enabled,
                mode: Some(Mode::new(1920, 1080, 60_000)),
                x,
                ..OutputState::default()
            },
        }
    }

    /// Entries in the backend's order: a disabled one first, then two
    /// enabled ones right to left.
    fn editor() -> Editor {
        Editor::new(Snapshot {
            backend: "test".into(),
            capabilities: Capabilities::default(),
            layout: Layout {
                entries: vec![
                    entry("eDP-1", false, 0),
                    entry("DP-1", true, 1920),
                    entry("DP-2", true, 0),
                ],
            },
        })
    }

    #[test]
    fn tab_order_is_the_canvas_left_to_right_then_the_tray() {
        let editor = editor();
        assert_eq!(editor.order(), [2, 1, 0]);
        assert_eq!(editor.selected(), Some(2), "the first in order");
    }

    #[test]
    fn selection_steps_and_wraps_both_ways() {
        let mut editor = editor();
        let mut seen = Vec::new();
        for _ in 0..4 {
            editor.select_next();
            seen.push(editor.selected().unwrap());
        }
        assert_eq!(seen, [1, 0, 2, 1]);
        editor.select(Some(2));
        editor.select_previous();
        assert_eq!(editor.selected(), Some(0), "wraps to the tray's last");
        editor.select(None);
        editor.select_previous();
        assert_eq!(editor.selected(), Some(0));
        editor.select(None);
        editor.select_next();
        assert_eq!(editor.selected(), Some(2));
    }

    #[test]
    fn selecting_an_index_with_no_output_selects_nothing() {
        let mut editor = editor();
        editor.select(Some(3));
        assert_eq!(editor.selected(), None);
    }

    #[test]
    fn an_edit_is_pending_until_undone_and_comes_back_on_redo() {
        let mut editor = editor();
        assert!(!editor.has_pending());
        assert!(editor.edit(1, |state| state.x = 2000));
        assert!(editor.has_pending());
        assert_eq!(editor.pending().entries[1].state.x, 2000);
        assert_eq!(editor.live().entries[1].state.x, 1920, "live is untouched");

        assert!(editor.undo());
        assert!(!editor.has_pending());
        assert_eq!(editor.pending().entries[1].state.x, 1920);
        assert!(!editor.undo(), "nothing left to undo");

        assert!(editor.redo());
        assert_eq!(editor.pending().entries[1].state.x, 2000);
        assert!(!editor.redo(), "nothing left to redo");
    }

    #[test]
    fn edits_undo_one_at_a_time_in_reverse() {
        let mut editor = editor();
        editor.edit(1, |state| state.x = 2000);
        editor.edit(2, |state| state.scale = 2.0);
        editor.edit(1, |state| state.enabled = false);
        editor.undo();
        assert!(editor.pending().entries[1].state.enabled);
        assert_eq!(editor.pending().entries[2].state.scale, 2.0);
        editor.undo();
        assert_eq!(editor.pending().entries[2].state.scale, 1.0);
        assert_eq!(editor.pending().entries[1].state.x, 2000);
        editor.undo();
        assert!(!editor.has_pending());
    }

    #[test]
    fn a_new_edit_drops_what_was_undone() {
        let mut editor = editor();
        editor.edit(1, |state| state.x = 2000);
        editor.undo();
        editor.edit(1, |state| state.y = 100);
        assert!(!editor.redo());
        assert_eq!(editor.pending().entries[1].state.x, 1920);
        assert_eq!(editor.pending().entries[1].state.y, 100);
    }

    #[test]
    fn an_edit_that_changes_nothing_records_nothing() {
        let mut editor = editor();
        assert!(!editor.edit(1, |state| state.x = 1920));
        assert!(!editor.edit(7, |state| state.x = 0), "no such output");
        assert!(!editor.undo());
    }
}
