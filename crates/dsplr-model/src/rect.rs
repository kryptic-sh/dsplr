//! An output's rectangle on the canvas, shared by validation and arranging.

use crate::{LayoutMode, OutputEntry, OutputState};

/// An output's place on the canvas, in whole layout pixels.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Rect {
    /// Where `entry`'s settings put it in `layout_mode`: [`Rect::of_state`].
    pub fn of(entry: &OutputEntry, layout_mode: LayoutMode) -> Option<Self> {
        Self::of_state(&entry.state, layout_mode)
    }

    /// Where these settings put an output in `layout_mode`: their position
    /// and their size there ([`OutputState::size`]). `None` when there is no
    /// size; whether the output is enabled is the caller's question.
    pub fn of_state(state: &OutputState, layout_mode: LayoutMode) -> Option<Self> {
        let (width, height) = state.size(layout_mode)?;
        Some(Self {
            x: state.x.into(),
            y: state.y.into(),
            width: width.into(),
            height: height.into(),
        })
    }

    pub const fn right(self) -> i64 {
        self.x + self.width
    }

    pub const fn bottom(self) -> i64 {
        self.y + self.height
    }

    /// Whether the two share any area.
    pub const fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether the two share a stretch of edge the pointer can cross; a
    /// shared corner alone is not one.
    pub const fn touches(self, other: Self) -> bool {
        let side_by_side = (self.right() == other.x || other.right() == self.x)
            && self.y < other.bottom()
            && other.y < self.bottom();
        let stacked = (self.bottom() == other.y || other.bottom() == self.y)
            && self.x < other.right()
            && other.x < self.right();
        side_by_side || stacked
    }
}
