//! The one `Layout` struct describing displays and their settings, which the
//! GUI renders and every backend and config format converts to and from
//! (`docs/plan/03-display-model.md`). No I/O.
//!
//! What is here: the types, their JSON form, validation, snapping and
//! normalisation.

mod arrange;
mod capabilities;
mod id;
mod json;
mod layout;
mod mode;
mod rect;
mod transform;
mod validate;

pub use arrange::PositionOutOfRange;
pub use capabilities::Capabilities;
pub use id::OutputId;
pub use json::{JSON_VERSION, JsonError};
pub use layout::{AdaptiveSync, Layout, Output, OutputEntry, OutputState, RgbRange};
pub use mode::{Mode, ModeHandle, REFRESH_TOLERANCE_MHZ, hz_to_mhz};
pub use transform::Transform;
pub use validate::{Field, Problem, ProblemKind, Severity};
