//! Following an X server's output changes: RandR's events on one
//! long-lived connection, for `dsplr watch` and the GUI.
//!
//! The connection selects `ScreenChangeNotify` and the CRTC, output and
//! output property `Notify`s on the root window, and its socket is watched
//! with tokio's `AsyncFd`. [`Watch::changed`] returns at a RandR event that
//! can change the layout (`is_change`), once per burst: the events queued
//! with it are taken too. An output property counts only when it is the
//! `EDID`: a backlight or another driver property set many times a second
//! would otherwise have the GUI read the server again each time. The caller
//! reads the server again, and every
//! read is made on a fresh connection, which has the screen's size as it is
//! now (crcbl-x11 0.19.1 offers no `GetGeometry`); so a
//! `ScreenChangeNotify`'s own size is not used.
//!
//! `poll_event` (`xcb_poll_for_event`) never waits for the server: it reads
//! what the socket holds, without blocking, and returns `None` when that is
//! no whole event. It takes libxcb's lock on the connection, which a thread
//! waiting for a reply holds; nothing else uses this connection, so that
//! never happens, and polling it on the runtime's thread is safe. Only the
//! connecting and the event selection, which wait for replies, run on a
//! detached thread under [`super::OPERATION_TIMEOUT`].
//!
//! A server that goes away ends the watch with an error: the connection is
//! dead. The caller connects again, with `dsplr watch`'s backoff.

use crcbl_x11::{Atom, Connection, Event, NotifyMask};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use super::x11::{Error, connect_and};

/// The RandR events [`Watch`] selects.
const EVENTS: NotifyMask = NotifyMask(
    NotifyMask::SCREEN_CHANGE.0
        | NotifyMask::CRTC_CHANGE.0
        | NotifyMask::OUTPUT_CHANGE.0
        | NotifyMask::OUTPUT_PROPERTY.0,
);

/// An X server's RandR events, on a connection of its own.
#[derive(Debug)]
pub struct Watch {
    /// The connection, owned by the `AsyncFd` that watches its socket, so
    /// the socket stays open for as long as it is watched.
    connection: AsyncFd<Connection>,
    /// The `EDID` atom, whose changes are the only output property changes
    /// counted.
    edid: Atom,
}

impl Watch {
    /// Connects to the server `display` names and selects its RandR events.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached, its RandR is missing or too old,
    /// or selecting the events fails.
    pub async fn new(display: &str) -> Result<Self, Error> {
        let (connection, edid) = connect_and(display, "selecting RandR's events", |connection| {
            // Interned, not looked up: an output given an EDID later makes
            // the atom, and its change must still be seen.
            let edid = connection
                .intern_atom("EDID", false)
                .map_err(|error| Error::X {
                    what: "InternAtom EDID",
                    error,
                })?;
            connection
                .select_input(connection.screen().root, EVENTS)
                .and_then(|()| connection.flush())
                .map_err(|error| Error::X {
                    what: "RRSelectInput",
                    error,
                })?;
            Ok(edid)
        })
        .await?;
        // SAFETY: the descriptor is the connection's socket
        // (`Connection::as_raw_fd`, the same one each call), and the
        // `AsyncFd` owns the connection, which closes it only when dropped,
        // so it stays open and the same for the `AsyncFd`'s whole life.
        let connection = unsafe { AsyncFd::register_with_interest(connection, Interest::READABLE) }
            .map_err(|error| {
                Error::Thread(format!(
                    "cannot watch the X connection: {}",
                    error.into_parts().1
                ))
            })?;
        Ok(Self { connection, edid })
    }

    /// Waits until the server says its screen, a CRTC, an output or an
    /// output's EDID changed, then takes the rest of the events already
    /// queued, so one change is one return. Cancel-safe: the only await is
    /// the wait for the socket, and an event read off it stays in libxcb's
    /// queue until a call takes it; the taking after a change does not
    /// await.
    ///
    /// # Errors
    ///
    /// When the connection is gone.
    pub async fn changed(&mut self) -> Result<(), Error> {
        loop {
            // What libxcb already holds first: a reply wait may have read
            // events off the socket, which would not wake the runtime.
            while let Some(event) =
                self.connection
                    .get_ref()
                    .poll_event()
                    .map_err(|error| Error::X {
                        what: "following RandR's events",
                        error,
                    })?
            {
                if is_change(&event, self.edid) {
                    self.drain();
                    return Ok(());
                }
            }
            let mut ready = self.connection.readable().await.map_err(|error| {
                Error::Thread(format!("cannot wait on the X connection: {error}"))
            })?;
            ready.clear_ready();
        }
    }

    /// Takes the events libxcb already holds, without waiting: the rest of
    /// a change's burst. A dead connection stops it, and the next
    /// [`Watch::changed`] says so (the connection's error stays set).
    fn drain(&self) {
        while let Ok(Some(_)) = self.connection.get_ref().poll_event() {}
    }
}

/// Whether `event` says the outputs may have changed: a screen, CRTC or
/// output change, a change to the property `edid` (the `EDID` atom), and
/// RandR's other `Notify`s (providers, resources, leases). Another output
/// property, an X error or another event is not one.
const fn is_change(event: &Event, edid: Atom) -> bool {
    match *event {
        Event::ScreenChange(_)
        | Event::CrtcChange(_)
        | Event::OutputChange(_)
        | Event::OtherRandrNotify { .. } => true,
        Event::OutputProperty(property) => property.atom == edid,
        Event::Error(_) | Event::Other { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use crcbl_x11::{Event, OutputPropertyNotify, PropertyState, XError};

    use super::is_change;

    const EDID: u32 = 90;

    fn property(atom: u32) -> Event {
        Event::OutputProperty(OutputPropertyNotify {
            window: 1,
            output: 2,
            atom,
            timestamp: 3,
            state: PropertyState::NewValue,
        })
    }

    #[test]
    fn an_edid_is_a_change_and_another_property_is_not() {
        assert!(is_change(&property(EDID), EDID));
        assert!(!is_change(&property(EDID + 1), EDID));
    }

    #[test]
    fn randr_notifies_are_changes_and_errors_are_not() {
        assert!(is_change(&Event::OtherRandrNotify { sub_code: 3 }, EDID));
        assert!(!is_change(&Event::Other { response_type: 12 }, EDID));
        assert!(!is_change(
            &Event::Error(XError {
                error_code: 3,
                major_opcode: 140,
                minor_opcode: 7,
                resource_id: 0,
                sequence: 1,
            }),
            EDID
        ));
    }
}
