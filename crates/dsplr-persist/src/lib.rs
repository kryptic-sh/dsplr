//! Writers and readers for each desktop's saved display config, and dsplr's
//! own profile store (`docs/plan/05-persistence-and-profiles.md`).
//!
//! What is here: Hyprland's Lua `monitors.lua`, sway's `outputs` and niri's
//! `outputs.kdl`, each written and read as text and saved beside the
//! desktop's own config, the managed-file header, conflict check and atomic
//! write every saved file goes through, a line diff to show what a save
//! would change, the revert point, the profile store, and kanshi's and
//! autorandr's profiles read for importing into it.
//! The other desktops are not written yet; Hyprland's legacy hyprlang
//! `monitors.conf` is not written at all.

pub mod autorandr;
pub mod diff;
pub mod hyprland;
mod identity;
mod import;
pub mod kanshi;
mod layout_json;
pub mod managed;
pub mod niri;
pub mod profile;
pub mod revert;
pub mod sway;
