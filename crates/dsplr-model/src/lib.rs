//! The one `Layout` struct describing displays and their settings, which the
//! GUI renders and every backend and config format converts to and from
//! (`docs/plan/03-display-model.md`). No I/O.
//!
//! What is here: the types, their JSON form, validation, snapping and
//! normalisation, and pairing a saved layout's entries with the connected
//! outputs.

mod arrange;
mod assign;
mod capabilities;
mod compare;
mod id;
mod json;
mod layout;
mod mode;
mod name;
mod rect;
mod transform;
mod validate;

pub use arrange::PositionOutOfRange;
pub use assign::assign;
pub use capabilities::Capabilities;
pub use compare::{Difference, Setting};
pub use id::OutputId;
pub use json::{JSON_VERSION, JsonError};
pub use layout::{
    AdaptiveSync, Layout, Output, OutputEntry, OutputState, RgbRange, is_valid_scale,
};
pub use mode::{
    Mode, ModeHandle, ModeSpec, ParseModeError, REFRESH_TOLERANCE_MHZ, hz_text_to_mhz, hz_to_mhz,
    mhz_to_hz_text,
};
pub use name::ParseNameError;
pub use rect::Rect;
pub use transform::Transform;
pub use validate::{Field, Problem, ProblemKind, Severity};
