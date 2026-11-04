//! Hyprland, read over its IPC socket (`docs/plan/04-backends.md#hyprland`).
//!
//! [`Hyprland`] finds the running compositor and asks it for its monitors;
//! [`Monitor`] is its reply as Hyprland writes it. Converting that into
//! dsplr's display model, and applying changes, come later.

mod ipc;
mod monitors;

pub use ipc::{Error, Hyprland};
pub use monitors::{Mode, ModeError, Monitor};
