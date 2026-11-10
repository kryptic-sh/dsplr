//! The editor: the live layout and the pending edits over it, undo and redo,
//! the selection, the edits the keys, the panel and dragging make, and the
//! draw list and hit regions the GUI renders (`docs/plan/02-architecture.md`).
//! Plain data: no platform types, so the canvas is tested without a display.
//!
//! The apply, confirm and revert flow every front end drives is [`flow`].

pub mod dialog;
mod draw;
mod editor;
mod fit;
#[cfg(test)]
mod fixtures;
pub mod flow;
pub mod link;
mod messages;
pub mod panel;
mod scene;
mod theme;

pub use draw::{Bounds, Color, DrawList, Hit, Item, Point};
pub use editor::{Direction, Editor, NUDGE_STEP, Rebased, SNAP_DISTANCE, Snapshot, scale_choices};
pub use fit::{BAR_HEIGHT, PANEL_WIDTH, Regions, regions};
pub use messages::{MESSAGE_HISTORY, Messages};
pub use scene::{Size, draw, draw_messages, messages_bounds};
pub use theme::Theme;
