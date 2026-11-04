//! Writers and readers for each desktop's saved display config, and dsplr's
//! own profile store (`docs/plan/05-persistence-and-profiles.md`).
//!
//! What is here: Hyprland's Lua `monitors.lua`, written and read as text. The
//! managed-file header, the atomic write to disk, the legacy `monitors.conf`,
//! the other desktops and the profile store are not written yet.

pub mod hyprland;
