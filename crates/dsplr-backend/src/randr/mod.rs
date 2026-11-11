//! X11 through RandR 1.5 (`docs/plan/04-backends.md`, "X11 (`Randr`)").
//!
//! So far the part that needs no server: [`resources`], dsplr's own form of
//! what the server reports, and [`plan`], which turns a layout into the
//! requests that set it, by xrandr's algorithm. The RandR client comes from
//! crcbl; the backend will read its replies into a [`Snapshot`] and send
//! the planned [`Operation`]s.

mod cvt;
pub mod plan;
pub mod resources;
#[cfg(test)]
mod tests;

pub use plan::{ModeRef, Note, Operation, Plan, PlanError, plan};
pub use resources::Snapshot;
