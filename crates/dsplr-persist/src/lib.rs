//! Writers and readers for each desktop's saved display config, and dsplr's
//! own profile store (`docs/plan/05-persistence-and-profiles.md`).
//!
//! What is here: Hyprland's Lua `monitors.lua`, written and read as text and
//! saved beside Hyprland's config, the managed-file header and atomic write
//! every saved file goes through, a line diff to show what a save would
//! change, and the revert point.
//! The legacy `monitors.conf`, the other desktops and the profile store are
//! not written yet.

pub mod diff;
pub mod hyprland;
pub mod managed;
pub mod revert;
