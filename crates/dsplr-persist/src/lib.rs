//! Writers and readers for each desktop's saved display config, and dsplr's
//! own profile store (`docs/plan/05-persistence-and-profiles.md`).
//!
//! What is here: Hyprland's Lua `monitors.lua` and sway's `outputs`, each
//! written and read as text and saved beside the desktop's own config, the
//! managed-file header, conflict check and atomic write every saved file
//! goes through, a line diff to show what a save would change, and the
//! revert point.
//! The other desktops and the profile store are not written yet; Hyprland's
//! legacy hyprlang `monitors.conf` is not written at all.

pub mod diff;
pub mod hyprland;
pub mod managed;
pub mod revert;
pub mod sway;
