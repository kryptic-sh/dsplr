//! An output's rectangle on the canvas, shared by validation and arranging.

use crate::OutputEntry;

/// An output's place on the canvas, in whole layout pixels.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Rect {
    /// Where `entry`'s settings put it: its position and logical size.
    /// `None` when it has no logical size; whether it is enabled is the
    /// caller's question.
    pub fn of(entry: &OutputEntry) -> Option<Self> {
        let (width, height) = entry.state.logical_size()?;
        Some(Self {
            x: entry.state.x.into(),
            y: entry.state.y.into(),
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
