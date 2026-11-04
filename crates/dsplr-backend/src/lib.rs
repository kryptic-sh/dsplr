//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! So far only Hyprland's state can be read ([`hyprland`]), and only on Linux:
//! the module is not compiled anywhere else, since Hyprland's sockets are Unix
//! sockets. Detection, applying and the other desktops are not written yet.

#[cfg(target_os = "linux")]
pub mod hyprland;
