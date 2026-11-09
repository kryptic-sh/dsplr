//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! [`detect`] finds the backend for the running desktop and reports what it
//! looked at. So far Hyprland and wlroots compositors exist; both read and
//! apply, wlroots tests, and Hyprland persists to its Lua config, while
//! persisting on wlroots and the other desktops are not written yet. Every
//! backend is Linux-only; on other targets [`Backend`] has no variants and
//! detection says why.

use std::time::Duration;

use dsplr_model::{Capabilities, Layout};
use dsplr_persist::managed::{Conflict, Saved};

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
    /// A save found a file there that dsplr may not overwrite unasked, and
    /// wrote nothing: ask, and persist again with its text approved.
    #[error(transparent)]
    Conflict(Box<Conflict>),
}

/// How a layout was made to last (`docs/plan/05-persistence-and-profiles.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Persisted {
    /// Written to a file the desktop reads when it starts.
    File(Saved),
}

impl Error {
    /// Whether the desktop itself refused the layout, and nothing changed:
    /// wlroots' `failed`, a rule Hyprland rejected. Not a layout dsplr
    /// refused before sending it ([`Error::is_refused_before_sending`]), a
    /// connection that failed or timed out, or [`Error::is_stale`].
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Failed) | Self::Hyprland(hyprland::Error::Refused { .. }) => true,
            _ => false,
        }
    }

    /// Whether the backend refused the layout before sending anything,
    /// because it cannot be put into the desktop's request. The desktop was
    /// not asked, so nothing changed. On wlroots, a [`wlr::PlanError`]: an
    /// output named twice, one enabled without a mode, a mode or scale out
    /// of the protocol's range, or a setting the protocol cannot express (a
    /// mirror, a primary output, colour settings, automatic adaptive sync).
    /// On Hyprland, a [`hyprland::Error::Write`]: an output with no name,
    /// enabled without a mode or with an unusable scale, or mirroring one
    /// with no name. An output that is no longer connected is
    /// [`Error::is_stale`] instead.
    #[must_use]
    pub const fn is_refused_before_sending(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Plan(wlr::PlanError::NotConnected(_))) => false,
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Plan(_)) | Self::Hyprland(hyprland::Error::Write(_)) => true,
            _ => false,
        }
    }

    /// Whether the desktop's state changed between reading it and the
    /// request, so the request was dropped unapplied: wlroots' `cancelled`,
    /// or an output in the layout that is no longer connected (unplugged
    /// between the snapshot and the request), which dsplr finds before
    /// sending. Read the state again before deciding anything; never retry
    /// the same request blindly.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(
                wlr::Error::Cancelled | wlr::Error::Plan(wlr::PlanError::NotConnected(_)),
            ) => true,
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
    /// Its native persistence is the Lua config's `monitors.lua`; under a
    /// hyprlang config [`persist`](Self::persist) is unsupported.
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
                native_persistence: true,
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
    /// A layout the backend cannot send is refused here, as it would be by
    /// [`apply`](Self::apply) ([`Error::is_refused_before_sending`]).
    ///
    /// [`Error::Unsupported`] on a backend with no real test (Hyprland's is a
    /// stub that always succeeds) once the layout is known to be sendable;
    /// the apply flow then skips the step rather than pretend it ran
    /// (`docs/plan/02-architecture.md`, step 2).
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub async fn test(&self, layout: &Layout) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => {
                hyprland::monitor_rules(layout)?;
                Err(Error::Unsupported {
                    what: "testing a layout",
                    reason: "Hyprland's wlr-output-management test is a stub that always succeeds",
                })
            }
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

    /// Makes `layout` last past a restart of the desktop, where the desktop
    /// itself reads it: on Hyprland, the `monitors.lua` beside its Lua
    /// config ([`dsplr_persist::hyprland::save`]).
    ///
    /// A file there that dsplr may not overwrite unasked is
    /// [`Error::Conflict`], and nothing is written; once the user has seen it
    /// and agreed, persist again with `approved` set to the conflict's
    /// `existing` text. [`Error::Unsupported`] under a hyprlang config, and
    /// on wlroots compositors.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub fn persist(&self, layout: &Layout, approved: Option<&str>) -> Result<Persisted, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => {
                use dsplr_persist::hyprland::{SaveError, save};
                match save(|name| std::env::var_os(name), layout, approved) {
                    Ok(saved) => Ok(Persisted::File(saved)),
                    Err(SaveError::Conflict(conflict)) => Err(Error::Conflict(conflict)),
                    Err(SaveError::Hyprlang(_)) => Err(Error::Unsupported {
                        what: "saving the layout",
                        reason: "Hyprland reads a hyprlang hyprland.conf here, and dsplr writes only a Lua config's monitors.lua",
                    }),
                    Err(error) => Err(Error::Hyprland(hyprland::Error::Save(error))),
                }
            }
            #[cfg(target_os = "linux")]
            Self::Wlr(_) => Err(Error::Unsupported {
                what: "saving the layout",
                reason: "dsplr cannot write a wlroots compositor's config yet",
            }),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use dsplr_model::OutputId;
    use dsplr_persist::hyprland::WriteError;

    use super::{Error, hyprland, wlr};

    #[test]
    fn a_refusal_is_the_desktop_s_one_before_sending_dsplr_s_and_a_vanished_output_stale() {
        let by_desktop = [
            Error::Wlr(wlr::Error::Failed),
            Error::Hyprland(hyprland::Error::Refused {
                request: "/eval".into(),
                reply: "error".into(),
            }),
        ];
        let before_sending = [
            Error::Wlr(wlr::Error::Plan(wlr::PlanError::Twice("DP-1".into()))),
            Error::Hyprland(hyprland::Error::Write(WriteError::NoMode(
                OutputId::Connector("DP-1".into()),
            ))),
        ];
        let stale = [
            Error::Wlr(wlr::Error::Cancelled),
            Error::Wlr(wlr::Error::Plan(wlr::PlanError::NotConnected(
                OutputId::Connector("DP-1".into()),
            ))),
        ];
        let neither = [
            Error::Wlr(wlr::Error::Timeout { what: "apply" }),
            Error::Unsupported {
                what: "testing a layout",
                reason: "a stub",
            },
        ];
        for error in &by_desktop {
            assert!(error.is_refusal(), "{error:?}");
            assert!(!error.is_refused_before_sending(), "{error:?}");
        }
        for error in &before_sending {
            assert!(error.is_refused_before_sending(), "{error:?}");
            assert!(!error.is_refusal(), "{error:?}");
        }
        for error in &stale {
            assert!(error.is_stale(), "{error:?}");
            assert!(
                !error.is_refusal() && !error.is_refused_before_sending(),
                "{error:?}"
            );
        }
        for error in by_desktop.iter().chain(&before_sending) {
            assert!(!error.is_stale(), "{error:?}");
        }
        for error in &neither {
            assert!(
                !error.is_refusal() && !error.is_refused_before_sending() && !error.is_stale(),
                "{error:?}"
            );
        }
    }
}
