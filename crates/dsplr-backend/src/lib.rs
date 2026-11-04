//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! [`detect`] finds the backend for the running desktop and reports what it
//! looked at. So far only Hyprland exists; it reads and applies, while
//! testing, persisting and the other desktops are not written yet. Every
//! backend is Linux-only; on other targets [`Backend`] has no variants and
//! detection says why.

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

    /// What dsplr can do through this backend now.
    ///
    /// Hyprland's are the ones its e2e test applies and reads back. Adaptive
    /// sync is left out because turning it off cannot be applied yet, and
    /// colour presets and bit depth because the snapshot does not read them.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => Capabilities {
                mirroring: true,
                fractional_scale: true,
                per_output_scale: true,
                gaps_allowed: true,
                ..Capabilities::default()
            },
        }
    }

    /// The desktop's current state.
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.snapshot().await?),
        }
    }

    /// Sets the desktop to `layout`. Returning does not mean the desktop
    /// shows it yet, or shows it as asked: take a snapshot to see.
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.apply(layout).await?),
        }
    }
}
