//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! [`detect`] finds the backend for the running desktop and reports what it
//! looked at. So far Hyprland and wlroots compositors exist; both read and
//! apply, and wlroots tests, while persisting and the other desktops are not
//! written yet. Every backend is Linux-only; on other targets [`Backend`]
//! has no variants and detection says why.

use std::time::Duration;

use dsplr_model::{Capabilities, Layout};

mod detect;
#[cfg(target_os = "linux")]
pub mod hyprland;
#[cfg(target_os = "linux")]
pub mod wlr;

pub use detect::{Check, Detection, detect};

/// The backend for the running desktop, found by [`detect`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Backend {
    #[cfg(target_os = "linux")]
    Hyprland(hyprland::Hyprland),
    #[cfg(target_os = "linux")]
    Wlr(wlr::Wlr),
}

/// What went wrong talking to the desktop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Hyprland(#[from] hyprland::Error),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Wlr(#[from] wlr::Error),
    /// The backend cannot do this at all; nothing was sent.
    #[error("{what} is not supported here: {reason}")]
    Unsupported {
        what: &'static str,
        reason: &'static str,
    },
}

impl Error {
    /// Whether the layout itself was refused, and nothing changed: by the
    /// desktop (wlroots' `failed`, a rule Hyprland rejected), or by the
    /// backend because the desktop's interface cannot express it. Not a
    /// connection that failed or timed out, and not [`Error::is_stale`].
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Failed | wlr::Error::Plan(_))
            | Self::Hyprland(hyprland::Error::Refused { .. } | hyprland::Error::Write(_)) => true,
            _ => false,
        }
    }

    /// Whether the desktop's state changed between reading it and the
    /// request, so the request was dropped unapplied. Read the state again
    /// before deciding anything; never retry the same request blindly.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Cancelled) => true,
            _ => false,
        }
    }
}

impl Backend {
    /// The backend's name, as `dsplr backend` and `--backend` spell it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => "hyprland",
            #[cfg(target_os = "linux")]
            Self::Wlr(_) => "wlr",
        }
    }

    /// The longest one [`snapshot`](Self::snapshot), [`test`](Self::test) or
    /// [`apply`](Self::apply) takes before it returns, an error included:
    /// each is cut off at this. A deadline that must outlast a request
    /// (the revert watchdog's) counts it in.
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => hyprland::REQUEST_TIMEOUT,
            #[cfg(target_os = "linux")]
            Self::Wlr(_) => wlr::OPERATION_TIMEOUT,
        }
    }

    /// What dsplr can do through this backend now.
    ///
    /// Hyprland's are the ones its e2e test applies and reads back. Adaptive
    /// sync is left out because turning it off cannot be applied yet, and
    /// colour presets and bit depth because the snapshot does not read them.
    ///
    /// wlroots compositors take custom modes, since the protocol has them and
    /// `test` lets the compositor refuse one first; adaptive sync needs the
    /// manager's version 4; mirroring is not in the protocol.
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
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Capabilities {
                custom_modes: true,
                adaptive_sync: if wlr.version() >= wlr::ADAPTIVE_SYNC_SINCE {
                    vec![
                        dsplr_model::AdaptiveSync::Off,
                        dsplr_model::AdaptiveSync::On,
                    ]
                } else {
                    Vec::new()
                },
                fractional_scale: true,
                per_output_scale: true,
                test_before_apply: true,
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
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(wlr.snapshot().await?),
        }
    }

    /// Asks the desktop whether it would apply `layout`, changing nothing.
    ///
    /// [`Error::Unsupported`] on a backend with no real test (Hyprland's is a
    /// stub that always succeeds); the apply flow then skips the step rather
    /// than pretend it ran (`docs/plan/02-architecture.md`, step 2).
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub async fn test(&self, layout: &Layout) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => Err(Error::Unsupported {
                what: "testing a layout",
                reason: "Hyprland's wlr-output-management test is a stub that always succeeds",
            }),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(wlr.test(layout).await?),
        }
    }

    /// Sets the desktop to `layout`. Returning does not mean the desktop
    /// shows it yet, or shows it as asked: take a snapshot to see.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.apply(layout).await?),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(wlr.apply(layout).await?),
        }
    }
}
