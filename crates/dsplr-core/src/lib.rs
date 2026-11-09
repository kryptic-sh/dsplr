//! The editor: the live layout and the pending edits over it, undo and redo,
//! the selection, and the draw list and hit regions the GUI renders
//! (`docs/plan/02-architecture.md`). Plain data: no platform types, so the
//! canvas is tested without a display.
//!
//! Not here yet (`docs/plan/06-gui.md`): dragging and snapping, the editing
//! actions behind the keys, and the confirm timer.

mod draw;
mod editor;
mod properties;
mod scene;
mod theme;

pub use draw::{Bounds, Color, DrawList, Hit, Item, Point};
pub use editor::{Editor, Snapshot};
pub use properties::{Property, properties};
pub use scene::{BAR_HEIGHT, PANEL_WIDTH, Size, draw};
pub use theme::Theme;
