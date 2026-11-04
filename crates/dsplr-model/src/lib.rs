//! The one `Layout` struct describing displays and their settings, which the
//! GUI renders and every backend and config format converts to and from
//! (`docs/plan/03-display-model.md`). No I/O.
//!
//! What is here: the types. Validation, snapping, normalisation and the JSON
//! form are not implemented yet.

mod capabilities;
mod id;
mod layout;
mod mode;
mod transform;

pub use capabilities::Capabilities;
pub use id::OutputId;
pub use layout::{AdaptiveSync, Layout, Output, OutputEntry, OutputState, RgbRange};
pub use mode::{Mode, ModeHandle, hz_to_mhz};
pub use transform::Transform;
