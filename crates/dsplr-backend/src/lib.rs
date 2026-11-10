//! Desktop detection and one backend per desktop: Hyprland, wlroots
//! compositors, KDE Plasma, GNOME and X11 (`docs/plan/04-backends.md`).
//!
//! [`detect`] finds the backend for the running desktop and reports what it
//! looked at. So far Hyprland, KDE Plasma, GNOME and wlroots compositors
//! exist; all read and apply, wlroots and GNOME test, Hyprland persists to
//! its Lua config, sway to its `outputs` file and niri to its
//! `outputs.kdl`, KWin saves each layout it applies itself, and Mutter saves
//! `monitors.xml` once GNOME Shell's dialog keeps it; other wlroots
//! compositors have no native persistence and use dsplr's profile store
//! (`dsplr save --profile`), and X11 is not written yet.
//! Every backend is Linux-only; on other targets [`Backend`] has no variants
//! and detection says why.

use std::time::Duration;

use dsplr_model::{Capabilities, Layout};
use dsplr_persist::managed::{Conflict, Saved};

#[cfg(test)]
#[path = "../build/cdata.rs"]
mod cdata;
mod detect;
#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(target_os = "linux")]
pub mod hyprland;
#[cfg(target_os = "linux")]
pub mod kde;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub mod wlr;

pub use detect::{Check, Detection, Named, detect, detect_named};

/// The backend for the running desktop, found by [`detect`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Backend {
    #[cfg(target_os = "linux")]
    Gnome(gnome::Gnome),
    #[cfg(target_os = "linux")]
    Hyprland(hyprland::Hyprland),
    #[cfg(target_os = "linux")]
    Kde(kde::Kde),
    #[cfg(target_os = "linux")]
    Wlr(wlr::Wlr),
}

/// Following a desktop's changes ([`Backend::watch`]).
#[derive(Debug)]
#[non_exhaustive]
pub enum Watch {
    #[cfg(target_os = "linux")]
    Gnome(Box<gnome::Watch>),
    #[cfg(target_os = "linux")]
    Hyprland(hyprland::Events),
    #[cfg(target_os = "linux")]
    Kde(Box<kde::Connection>),
    #[cfg(target_os = "linux")]
    Wlr(Box<wlr::Connection>),
}

impl Watch {
    /// Waits, for as long as it takes, until the desktop says its outputs
    /// may have changed. Cancel-safe, so it can wait beside other work in a
    /// `tokio::select!`. An error ends the watch: the connection is gone.
    pub async fn changed(&mut self) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref mut watch) => Ok(watch.changed().await?),
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref mut events) => Ok(events.changed().await?),
            #[cfg(target_os = "linux")]
            Self::Kde(ref mut connection) => Ok(connection.changed().await?),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref mut connection) => Ok(connection.changed().await?),
        }
    }
}

/// What went wrong talking to the desktop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Gnome(#[from] gnome::Error),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Hyprland(#[from] hyprland::Error),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Kde(#[from] kde::Error),
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
    /// The desktop saved it itself when it applied it, so nothing was
    /// written.
    ByDesktop {
        /// Who saved it and where, for the user: "KWin, in its
        /// kwinoutputconfig.json".
        by: &'static str,
    },
    /// Handed to the desktop, which saves it once the user keeps it in a
    /// dialog of the desktop's own, and not otherwise; nothing was written.
    WhenKept {
        /// Who saves it, where, and what it asks, for the user.
        by: &'static str,
    },
}

impl Error {
    /// Whether the desktop itself refused the layout, and nothing changed:
    /// wlroots' `failed`, KWin's `failed`, a rule Hyprland rejected, Mutter's
    /// refusal (`InvalidArgs`, or D-Bus configuration turned off). Not a
    /// layout dsplr
    /// refused before sending it ([`Error::is_refused_before_sending`]), a
    /// connection that failed or timed out, or [`Error::is_stale`].
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Failed)
            | Self::Kde(kde::Error::Failed { .. })
            | Self::Gnome(gnome::Error::Refused(_))
            | Self::Hyprland(hyprland::Error::Refused { .. }) => true,
            _ => false,
        }
    }

    /// Whether the backend refused the layout before sending anything,
    /// because it cannot be put into the desktop's request. The desktop was
    /// not asked, so nothing changed. On wlroots, a [`wlr::PlanError`]: an
    /// output named twice, one enabled without a mode, a mode or scale out
    /// of the protocol's range, or a setting the protocol cannot express (a
    /// mirror, a primary output, colour settings, automatic adaptive sync).
    /// On KDE, a [`kde::PlanError`] too: the same, or a custom mode, colour
    /// settings, a setting the output says it has not, or more than one
    /// primary output. On Hyprland, a [`hyprland::Error::Write`]: an output
    /// with no name,
    /// enabled without a mode or with an unusable scale, or mirroring one
    /// with no name. On GNOME, a [`gnome::PlanError`]: a mode or scale the
    /// monitor does not list, a mirror of another size, a second primary
    /// output, a layout away from the origin, or a setting Mutter's
    /// interface cannot carry. An output that is no longer connected is
    /// [`Error::is_stale`] instead.
    #[must_use]
    pub const fn is_refused_before_sending(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Plan(wlr::PlanError::NotConnected(_)))
            | Self::Kde(kde::Error::Plan(kde::PlanError::NotConnected(_)))
            | Self::Gnome(gnome::Error::Plan(gnome::PlanError::NotConnected(_))) => false,
            #[cfg(target_os = "linux")]
            Self::Wlr(wlr::Error::Plan(_))
            | Self::Kde(kde::Error::Plan(_))
            | Self::Gnome(gnome::Error::Plan(_))
            | Self::Hyprland(hyprland::Error::Write(_)) => true,
            _ => false,
        }
    }

    /// Whether the desktop's state changed between reading it and the
    /// request, so the request was dropped unapplied: wlroots' `cancelled`,
    /// Mutter's stale serial, or an output in the layout that is no longer
    /// connected (unplugged
    /// between the snapshot and the request), which dsplr finds before
    /// sending. Read the state again before deciding anything; never retry
    /// the same request blindly. KWin's configurations carry no serial, so
    /// on KDE only the second is found here; an output KWin added or removed
    /// while the request was being made is its `failed`
    /// ([`Error::is_refusal`]).
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        match self {
            #[cfg(target_os = "linux")]
            Self::Wlr(
                wlr::Error::Cancelled | wlr::Error::Plan(wlr::PlanError::NotConnected(_)),
            )
            | Self::Kde(kde::Error::Plan(kde::PlanError::NotConnected(_)))
            | Self::Gnome(
                gnome::Error::Stale(_) | gnome::Error::Plan(gnome::PlanError::NotConnected(_)),
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
            Self::Gnome(_) => "gnome",
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => "hyprland",
            #[cfg(target_os = "linux")]
            Self::Kde(_) => "kde",
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
            Self::Gnome(_) => gnome::OPERATION_TIMEOUT,
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => hyprland::REQUEST_TIMEOUT,
            #[cfg(target_os = "linux")]
            Self::Kde(_) => kde::OPERATION_TIMEOUT,
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
    /// manager's version 4; mirroring is not in the protocol. Only sway and
    /// niri have native persistence, sway's `outputs` file and niri's
    /// `outputs.kdl`; here that is whether `SWAYSOCK` or `NIRI_SOCKET` names
    /// a socket, asking nothing, since every command reads capabilities.
    /// [`Backend::native_persistence`] asks the sockets.
    ///
    /// KWin takes every adaptive sync value (its VRR policy), overscan, RGB
    /// range and the primary output (when it offers `kde_output_order_v1`),
    /// mirroring from management version 13, and saves each layout it
    /// applies; it has no test request, and its management protocol says
    /// outputs may have no gaps between them. It takes custom modes from
    /// management version 18, on outputs whose capabilities say so.
    ///
    /// Mutter tests (`verify`), mirrors (monitors sharing a logical
    /// monitor), has a primary output and an RGB range, and takes the
    /// scales each mode lists, each output its own unless it says
    /// `global-scale-required`; the scale goes as the mode lists it, so it
    /// reads back as sent. It requires its logical monitors to touch and to
    /// start at the origin, takes no custom modes, and offers adaptive sync
    /// only as modes of their own, which dsplr does not read yet. It saves
    /// `monitors.xml` on a `persistent` apply once GNOME Shell's dialog
    /// keeps it, not on every apply.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref gnome) => Capabilities {
                mirroring: true,
                fractional_scale: true,
                per_output_scale: !gnome.global_scale_required(),
                test_before_apply: true,
                origin_required: true,
                native_persistence: true,
                primary: true,
                rgb_range: true,
                ..Capabilities::default()
            },
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => Capabilities {
                mirroring: true,
                fractional_scale: true,
                per_output_scale: true,
                gaps_allowed: true,
                native_persistence: true,
                colour_preset: true,
                bit_depth: true,
                ..Capabilities::default()
            },
            #[cfg(target_os = "linux")]
            Self::Kde(ref kde) => Capabilities {
                mirroring: kde.management_version() >= kde::REPLICATION_SINCE,
                adaptive_sync: vec![
                    dsplr_model::AdaptiveSync::Off,
                    dsplr_model::AdaptiveSync::On,
                    dsplr_model::AdaptiveSync::Automatic,
                ],
                fractional_scale: true,
                per_output_scale: true,
                // A wl_fixed to KWin, its 120ths, a wl_fixed back.
                scale_steps: vec![
                    wayland::heads::WL_FIXED_STEPS,
                    kde::KWIN_SCALE_STEPS,
                    wayland::heads::WL_FIXED_STEPS,
                ],
                custom_modes: kde.management_version() >= kde::CUSTOM_MODES_SINCE,
                native_persistence: true,
                saves_on_apply: true,
                primary: kde.has_order(),
                overscan: true,
                rgb_range: true,
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
                // A wl_fixed each way; the compositor may round it further,
                // which then shows.
                scale_steps: vec![wayland::heads::WL_FIXED_STEPS],
                test_before_apply: true,
                gaps_allowed: true,
                native_persistence: wlr.names_compositor(),
                ..Capabilities::default()
            },
        }
    }

    /// Whether [`persist`](Self::persist) can save natively here, checked:
    /// [`Capabilities::native_persistence`] as [`capabilities`](Self::capabilities)
    /// says, except that on a wlroots compositor sway's and niri's IPC
    /// sockets are asked which it is ([`wlr::Wlr::compositor`]), which can
    /// take up to their request timeout.
    pub async fn native_persistence(&self) -> bool {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(_) | Self::Hyprland(_) | Self::Kde(_) => {
                self.capabilities().native_persistence
            }
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => matches!(
                wlr.compositor().await,
                wlr::Compositor::Sway | wlr::Compositor::Niri
            ),
        }
    }

    /// Whether the desktop puts a layout it saved back by itself when the
    /// connected outputs change: KWin from its `kwinoutputconfig.json`,
    /// Mutter from its `monitors.xml`, each keeping a layout per set of
    /// outputs. Something else restoring layouts on hotplug there
    /// (`dsplr watch`) would fight it.
    #[must_use]
    pub const fn restores_on_hotplug(&self) -> bool {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(_) | Self::Kde(_) => true,
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) | Self::Wlr(_) => false,
        }
    }

    /// A way to follow the desktop's changes to its outputs: on wlroots a
    /// connection of its own that dispatches the output manager's `done`
    /// events, on KDE one that dispatches each output device's `done` and
    /// the output order, on GNOME Mutter's `MonitorsChanged` on a bus
    /// connection of its own, on Hyprland its event socket. Read the state
    /// again after each [`Watch::changed`].
    pub async fn watch(&self) -> Result<Watch, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref gnome) => Ok(Watch::Gnome(Box::new(gnome.watch().await?))),
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(Watch::Hyprland(hyprland.events().await?)),
            #[cfg(target_os = "linux")]
            Self::Kde(ref kde) => Ok(Watch::Kde(Box::new(kde.connect().await?))),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(Watch::Wlr(Box::new(wlr.connect().await?))),
        }
    }

    /// The desktop's current state.
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref gnome) => Ok(gnome.snapshot().await?),
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.snapshot().await?),
            #[cfg(target_os = "linux")]
            Self::Kde(ref kde) => Ok(kde.snapshot().await?),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(wlr.snapshot().await?),
        }
    }

    /// Asks the desktop whether it would apply `layout`, changing nothing.
    /// A layout the backend cannot send is refused here, as it would be by
    /// [`apply`](Self::apply) ([`Error::is_refused_before_sending`]).
    ///
    /// [`Error::Unsupported`] on a backend with no real test (Hyprland's is a
    /// stub that always succeeds, KDE's management protocol has none) once
    /// the layout is known to be sendable;
    /// the apply flow then skips the step rather than pretend it ran
    /// (`docs/plan/02-architecture.md`, step 2).
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub async fn test(&self, layout: &Layout) -> Result<(), Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref gnome) => Ok(gnome.test(layout).await?),
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => {
                hyprland::monitor_rules(layout)?;
                Err(Error::Unsupported {
                    what: "testing a layout",
                    reason: "Hyprland's wlr-output-management test is a stub that always succeeds",
                })
            }
            #[cfg(target_os = "linux")]
            Self::Kde(ref kde) => {
                kde.check(layout).await?;
                Err(Error::Unsupported {
                    what: "testing a layout",
                    reason: "KWin's kde-output-management-v2 has no test request",
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
            Self::Gnome(ref gnome) => Ok(gnome.apply(layout).await?),
            #[cfg(target_os = "linux")]
            Self::Hyprland(ref hyprland) => Ok(hyprland.apply(layout).await?),
            #[cfg(target_os = "linux")]
            Self::Kde(ref kde) => Ok(kde.apply(layout).await?),
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => Ok(wlr.apply(layout).await?),
        }
    }

    /// Makes `layout` last past a restart of the desktop, where the desktop
    /// itself reads it: on Hyprland, the `monitors.lua` beside its Lua
    /// config ([`dsplr_persist::hyprland::save`]); on sway, the `outputs`
    /// file beside its config ([`dsplr_persist::sway::save`]); on niri, the
    /// `outputs.kdl` beside its config ([`dsplr_persist::niri::save`]),
    /// which niri reloads, and so applies, as soon as it is written. On KDE
    /// nothing is written: KWin saved the layout in its
    /// `kwinoutputconfig.json` when it applied it, so this is
    /// [`Persisted::ByDesktop`]. On GNOME the layout is sent to Mutter again
    /// with `persistent`, which has GNOME Shell ask whether to keep it and
    /// writes `monitors.xml` only once it is kept, so this is
    /// [`Persisted::WhenKept`], returned before anyone has answered.
    ///
    /// Because writing it applies it, on niri `layout` must be the outputs as
    /// they are now: they are read again and compared
    /// ([`Layout::differences`], within the tolerance the apply flow reads
    /// back with), and any other layout is [`Error::Unsupported`], nothing
    /// written, so a save never changes the screen past the apply flow's
    /// test, revert point and confirm. Apply a layout first, then save it.
    /// The same holds on KDE, where KWin has saved only what is on screen,
    /// and on GNOME, where saving applies the layout again.
    ///
    /// A file there that dsplr may not overwrite unasked is
    /// [`Error::Conflict`], and nothing is written; once the user has seen it
    /// and agreed, persist again with `approved` set to the conflict's
    /// `existing` text. [`Error::Unsupported`], with nothing written, under a
    /// hyprlang config, on a wlroots compositor other than sway and niri (one
    /// nested in them included, [`wlr::Wlr::compositor`]), and on sway or
    /// niri for a layout its output config cannot hold (a mirror, say).
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_variables, reason = "no backend exists off Linux")
    )]
    pub async fn persist(
        &self,
        layout: &Layout,
        approved: Option<&str>,
    ) -> Result<Persisted, Error> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::Gnome(ref gnome) => {
                self.on_screen(layout, "Mutter saves a layout by applying it again, so dsplr saves only the outputs as they are now; apply the layout first (dsplr apply tests it and asks to keep it), then save").await?;
                gnome.persist(layout).await?;
                Ok(Persisted::WhenKept {
                    by: "Mutter, which writes monitors.xml once the layout is kept in GNOME Shell's dialog asking whether to keep the display settings, and puts the layout from before back if it is not (without GNOME Shell, Mutter keeps it at once)",
                })
            }
            #[cfg(target_os = "linux")]
            Self::Hyprland(_) => {
                use dsplr_persist::hyprland::{SaveError, save};
                use dsplr_persist::managed::FileError;
                match save(|name| std::env::var_os(name), layout, approved) {
                    Ok(saved) => Ok(Persisted::File(saved)),
                    Err(SaveError::File(FileError::Conflict(conflict))) => {
                        Err(Error::Conflict(conflict))
                    }
                    Err(SaveError::Hyprlang(_)) => Err(Error::Unsupported {
                        what: "saving the layout",
                        reason: "Hyprland reads a hyprlang hyprland.conf here, and dsplr writes only a Lua config's monitors.lua",
                    }),
                    Err(error) => Err(Error::Hyprland(hyprland::Error::Save(error))),
                }
            }
            #[cfg(target_os = "linux")]
            Self::Kde(_) => {
                self.on_screen(layout, "KWin keeps only the layout it last applied, so dsplr saves only the outputs as they are now; apply the layout first (dsplr apply asks to keep it), and KWin saves it then").await?;
                Ok(Persisted::ByDesktop {
                    by: "KWin, in its kwinoutputconfig.json, when it applied it",
                })
            }
            #[cfg(target_os = "linux")]
            Self::Wlr(ref wlr) => {
                use dsplr_persist::managed::FileError;
                use dsplr_persist::{niri, sway};
                let var = |name: &str| std::env::var_os(name);
                match wlr.compositor().await {
                    wlr::Compositor::Sway => match sway::save(var, layout, approved) {
                        Ok(saved) => Ok(Persisted::File(saved)),
                        Err(sway::SaveError::File(FileError::Conflict(conflict))) => {
                            Err(Error::Conflict(conflict))
                        }
                        Err(sway::SaveError::Write(sway::WriteError::Unsupported {
                            what, ..
                        })) => Err(Error::Unsupported {
                            what,
                            reason: "sway's output config cannot hold it, so the layout cannot be saved there",
                        }),
                        Err(error) => Err(Error::Wlr(wlr::Error::Save(error))),
                    },
                    wlr::Compositor::Niri => {
                        self.on_screen(layout, "niri applies its outputs.kdl as soon as it is written, so dsplr saves only the outputs as they are now; apply the layout first (dsplr apply tests it and asks to keep it), then save").await?;
                        match niri::save(var, layout, approved) {
                            Ok(saved) => Ok(Persisted::File(saved)),
                            Err(niri::SaveError::File(FileError::Conflict(conflict))) => {
                                Err(Error::Conflict(conflict))
                            }
                            Err(niri::SaveError::Write(niri::WriteError::Unsupported {
                                what,
                                ..
                            })) => Err(Error::Unsupported {
                                what,
                                reason: "niri's output config cannot hold it, so the layout cannot be saved there",
                            }),
                            Err(error) => Err(Error::Wlr(wlr::Error::NiriSave(error))),
                        }
                    }
                    wlr::Compositor::Other => Err(Error::Unsupported {
                        what: "saving the layout",
                        reason: "neither SWAYSOCK nor NIRI_SOCKET names a socket that answers as sway's or niri's from the process serving WAYLAND_DISPLAY, so this is neither sway nor niri (a compositor nested in one inherits its variable), the wlroots compositors whose config dsplr writes; on another, save a profile with `dsplr save --profile NAME` and run `dsplr restore --yes` from the compositor's autostart",
                    }),
                }
            }
        }
    }
}

impl Backend {
    /// [`Error::Unsupported`] for saving, with `reason`, unless `layout` is
    /// what the desktop shows now ([`Layout::differences`], within the
    /// tolerance the apply flow reads back with).
    #[cfg(target_os = "linux")]
    async fn on_screen(&self, layout: &Layout, reason: &'static str) -> Result<(), Error> {
        let now = self.snapshot().await?;
        if layout.differences(&now, &self.capabilities()).is_empty() {
            Ok(())
        } else {
            Err(Error::Unsupported {
                what: "saving a layout other than the one on screen",
                reason,
            })
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use dsplr_model::OutputId;
    use dsplr_persist::hyprland::WriteError;

    use super::{Error, gnome, hyprland, wlr};

    #[test]
    fn a_refusal_is_the_desktop_s_one_before_sending_dsplr_s_and_a_vanished_output_stale() {
        let by_desktop = [
            Error::Wlr(wlr::Error::Failed),
            Error::Hyprland(hyprland::Error::Refused {
                request: "/eval".into(),
                reply: "error".into(),
            }),
            Error::Gnome(gnome::Error::Refused(
                "Logical monitors not adjacent".into(),
            )),
        ];
        let before_sending = [
            Error::Wlr(wlr::Error::Plan(wlr::PlanError::Twice("DP-1".into()))),
            Error::Hyprland(hyprland::Error::Write(WriteError::NoMode(
                OutputId::Connector("DP-1".into()),
            ))),
            Error::Gnome(gnome::Error::Plan(gnome::PlanError::Offset { x: 0, y: 5 })),
        ];
        let stale = [
            Error::Wlr(wlr::Error::Cancelled),
            Error::Wlr(wlr::Error::Plan(wlr::PlanError::NotConnected(
                OutputId::Connector("DP-1".into()),
            ))),
            Error::Gnome(gnome::Error::Stale(
                "The requested configuration is based on stale information".into(),
            )),
            Error::Gnome(gnome::Error::Plan(gnome::PlanError::NotConnected(
                OutputId::Connector("DP-1".into()),
            ))),
        ];
        let neither = [
            Error::Wlr(wlr::Error::Wayland(crate::wayland::Error::Timeout {
                what: "apply",
            })),
            Error::Unsupported {
                what: "testing a layout",
                reason: "a stub",
            },
            Error::Gnome(gnome::Error::Timeout { what: "apply" }),
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
