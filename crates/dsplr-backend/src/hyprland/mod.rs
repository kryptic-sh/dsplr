//! Hyprland, read over its IPC socket (`docs/plan/04-backends.md#hyprland`).
//!
//! [`Hyprland`] finds the running compositor and asks it for its monitors;
//! [`Monitor`] is its reply as Hyprland writes it, and [`layout`] turns that
//! into dsplr's display model. Applying changes comes later.

mod events;
mod ipc;
mod monitors;
mod snapshot;

pub use events::Events;
pub use ipc::{Error, Hyprland, REQUEST_TIMEOUT, monitor_rules};
pub use monitors::{ModeError, Monitor, parse_mode};
pub use snapshot::{SnapshotError, layout};
