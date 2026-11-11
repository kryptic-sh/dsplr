//! X11 through RandR (`docs/plan/04-backends.md`, "X11 (`Randr`)").
//!
//! [`resources`] is dsplr's own form of what the server reports;
//! [`snapshot`] reads it as a layout, and [`plan`] turns a layout into the
//! requests that set it, by xrandr's algorithm. These need no server. The
//! server is reached through crcbl-x11 ([`x11`], on Linux and FreeBSD
//! only): its replies are read into a [`Snapshot`]. Sending the planned
//! [`Operation`]s is not written yet, so applying on X11 is unsupported.

mod cvt;
pub mod plan;
pub mod resources;
pub mod snapshot;
#[cfg(test)]
mod tests;
#[cfg(x11)]
pub mod watch;
#[cfg(x11)]
pub mod x11;

pub use plan::{ModeRef, Note, Operation, Plan, PlanError, plan};
pub use resources::Snapshot;
#[cfg(x11)]
pub use watch::Watch;
#[cfg(x11)]
pub use x11::{APPLY_TIMEOUT, Error, OPERATION_TIMEOUT, Randr};
