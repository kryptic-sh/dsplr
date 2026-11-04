//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! [`detect`] finds the backend for the running desktop and reports what it
//! looked at. So far only Hyprland exists, and it can only be read: applying
//! and the other desktops are not written yet. Every backend is Linux-only;
//! on other targets [`Backend`] has no variants and detection says why.

use dsplr_model::{Capabilities, Layout};

mod detect;
#[cfg(target_os = "linux")]
pub mod hyprland;

pub use detect::{Check, Detection, detect};

/// The backend for the running desktop, found by [`detect`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Backend {
    #[cfg(target_os = "linux")]
    Hyprland(hyprland::Hyprland),
}

/// What went wrong talking to the desktop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Hyprland(#[from] hyprland::Error),
}

impl Backend {
    /// The backend's name, as `dsplr backend` and `--backend` spell it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => "hyprland",
        }
    }

    /// What dsplr can do through this backend now. Every backend is
    /// read-only so far, so none can set anything optional yet.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => Capabilities::default(),
        }
    }

    /// The desktop's current state.
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.snapshot().await?),
        }
    }
}
