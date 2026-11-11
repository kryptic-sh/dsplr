//! The RandR backend's connection to the X server, through crcbl-x11: the
//! display found from `DISPLAY`, and the server's state read into a
//! [`Snapshot`].
//!
//! crcbl-x11 talks to libxcb, which it loads with `dlopen`, and is built on
//! Linux and FreeBSD only, so this module is too (the `x11` cfg,
//! `build.rs`). Only Linux is built and run here; FreeBSD is not compiled.
//!
//! Each read opens a connection of its own: the connection setup is where
//! crcbl-x11 has the root window's size in pixels and millimetres
//! ([`crcbl_x11::Screen`]), and it is a copy from connect time, so a new
//! connection reads the size the screen has now. libxcb's calls block with
//! no deadline, so they run on a detached thread of their own, given up on
//! at [`OPERATION_TIMEOUT`] (`blocking_for`).
//!
//! The reads are `GetScreenResourcesCurrent`'s, which asks the driver
//! nothing: a monitor plugged in on a driver without hotplug detection shows
//! only once something has probed.

use std::ffi::OsString;
use std::time::Duration;

use crcbl_x11::{ConnectError, Connection, OutputConnection, PropertyValue};

use super::resources::{
    Connection as Connected, CrtcInfo, CrtcXid, ModeInfo, ModeXid, OutputInfo, OutputXid, Panning,
    Rotation, ScreenResources, ScreenSize, SizeRange, Snapshot,
};
use super::snapshot::{SnapshotError, layout};
use dsplr_model::Layout;

/// How long one read of the server's state may take.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// The RandR version the reads need: `GetScreenResourcesCurrent`,
/// `GetOutputPrimary`, `GetPanning` and `GetCrtcTransform` are 1.3's.
const NEEDED: crcbl_x11::RandrVersion = crcbl_x11::RandrVersion { major: 1, minor: 3 };

/// How many 32-bit units of an `EDID` property to read: 1024 bytes, the
/// base block and up to seven extension blocks, which is all an EDID's
/// extension count can name past the base block in practice; only the base
/// block is parsed.
const EDID_LONGS: u32 = 256;

/// What went wrong talking to the X server.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot connect to the X server: {0}")]
    Connect(#[from] ConnectError),
    #[error("{what}: {error}")]
    X {
        what: &'static str,
        error: crcbl_x11::Error,
    },
    #[error(
        "the X server has RandR {have}; dsplr needs {} (GetScreenResourcesCurrent, GetOutputPrimary, GetPanning, GetCrtcTransform)",
        NEEDED
    )]
    TooOld { have: crcbl_x11::RandrVersion },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Plan(#[from] super::PlanError),
    #[error("the X server did not answer {what} within {seconds} s", seconds = OPERATION_TIMEOUT.as_secs())]
    Timeout { what: &'static str },
    #[error("the thread reading the X server ended: {0}")]
    Thread(String),
}

/// The X server `DISPLAY` names, driven through RandR.
#[derive(Clone, Debug)]
pub struct Randr {
    /// The display name, as `DISPLAY` gave it.
    display: String,
    /// The RandR version the server settled on, when it was found.
    version: crcbl_x11::RandrVersion,
}

impl Randr {
    /// The X server `DISPLAY` names, when it answers with RandR 1.3 or
    /// later. `None` in the error's place when `DISPLAY` is not set.
    ///
    /// # Errors
    ///
    /// When it cannot be connected to, or its RandR is missing or too old.
    pub async fn from_vars(var: &impl Fn(&str) -> Option<OsString>) -> Result<Option<Self>, Error> {
        let Some(display) = var("DISPLAY")
            .filter(|display| !display.is_empty())
            .map(|display| display.to_string_lossy().into_owned())
        else {
            return Ok(None);
        };
        let name = display.clone();
        let version = blocking("RandR's version", move || {
            let connection = Connection::connect(Some(&name))?;
            version(&connection)
        })
        .await?;
        Ok(Some(Self { display, version }))
    }

    /// The display name, as `DISPLAY` gave it.
    #[must_use]
    pub fn display(&self) -> &str {
        &self.display
    }

    /// The RandR version the server settled on.
    #[must_use]
    pub const fn version(&self) -> crcbl_x11::RandrVersion {
        self.version
    }

    /// The server's RandR state, read on a connection of its own.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached, a request fails, or it takes
    /// longer than [`OPERATION_TIMEOUT`].
    pub async fn read(&self) -> Result<Snapshot, Error> {
        let name = self.display.clone();
        blocking("the outputs", move || {
            let connection = Connection::connect(Some(&name))?;
            version(&connection)?;
            read(&connection)
        })
        .await
    }

    /// The server's state as a layout ([`super::snapshot::layout`]).
    ///
    /// # Errors
    ///
    /// As [`Randr::read`], or when the state cannot be read as a layout.
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        Ok(layout(&self.read().await?)?)
    }

    /// Plans `layout` on the server's state now, sending nothing: whether
    /// the planner can put it into requests ([`super::plan`]).
    ///
    /// # Errors
    ///
    /// As [`Randr::read`], or why the layout cannot be planned.
    pub async fn check(&self, layout: &Layout) -> Result<(), Error> {
        super::plan(&self.read().await?, layout)?;
        Ok(())
    }
}

/// Runs `work` on a thread of its own, cut off at [`OPERATION_TIMEOUT`].
async fn blocking<T: Send + 'static>(
    what: &'static str,
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    blocking_for(OPERATION_TIMEOUT, what, work).await
}

/// Runs `work` on a detached thread, its result sent back over a channel,
/// waiting at most `limit` for it.
///
/// libxcb waits for a reply with no deadline, so a server that stops
/// answering leaves the thread blocked in it. A tokio blocking task would
/// hold up the runtime's shutdown until it returned, which may be never; a
/// detached thread holds up nothing. On a timeout the thread and its
/// connection are left behind, to end when the server answers or closes,
/// or with the process.
async fn blocking_for<T: Send + 'static>(
    limit: Duration,
    what: &'static str,
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("dsplr-x11".to_owned())
        .spawn(move || {
            // The receiver is gone when the wait timed out: nobody to tell.
            let _ = send.send(work());
        })
        .map_err(|error| Error::Thread(error.to_string()))?;
    match tokio::time::timeout(limit, receive).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(Error::Thread("it ended without a result".to_owned())),
        Err(_) => Err(Error::Timeout { what }),
    }
}

/// The RandR version `connection` negotiated, when it is [`NEEDED`] or
/// later.
fn version(connection: &Connection) -> Result<crcbl_x11::RandrVersion, Error> {
    let version = connection
        .randr()
        .map_err(|error| Error::X {
            what: "RandR",
            error,
        })?
        .version;
    if version < NEEDED {
        return Err(Error::TooOld { have: version });
    }
    Ok(version)
}

/// The server's RandR state, read on `connection`.
fn read(connection: &Connection) -> Result<Snapshot, Error> {
    let x = |what| move |error| Error::X { what, error };
    let randr = connection;
    let screen = connection.screen();
    let root = screen.root;
    let resources = randr
        .screen_resources_current(root)
        .map_err(x("GetScreenResourcesCurrent"))?;
    let range = randr
        .screen_size_range(root)
        .map_err(x("GetScreenSizeRange"))?;
    let primary = randr.output_primary(root).map_err(x("GetOutputPrimary"))?;
    // `None` when no output has an EDID: the atom does not exist then.
    let edid_atom = Some(
        connection
            .intern_atom("EDID", true)
            .map_err(x("InternAtom"))?,
    )
    .filter(|&atom| atom != 0);
    let config = resources.config_timestamp;

    let outputs = resources
        .outputs
        .iter()
        .map(|&id| {
            let info = randr.output_info(id, config).map_err(x("GetOutputInfo"))?;
            let edid = edid_atom
                .map(|atom| randr.get_output_property(id, atom, 0, 0, EDID_LONGS, false, false))
                .transpose()
                .map_err(x("GetOutputProperty EDID"))?
                .and_then(|data| match data.value {
                    Some(PropertyValue::Format8(bytes)) if !bytes.is_empty() => Some(bytes),
                    _ => None,
                });
            Ok(OutputInfo {
                id: OutputXid(id),
                timestamp: info.timestamp,
                crtc: (info.crtc != 0).then_some(CrtcXid(info.crtc)),
                mm_width: info.mm_width,
                mm_height: info.mm_height,
                connection: match info.connection {
                    OutputConnection::Connected => Connected::Connected,
                    OutputConnection::Disconnected => Connected::Disconnected,
                    OutputConnection::Unknown | OutputConnection::Other(_) => Connected::Unknown,
                },
                subpixel_order: 0,
                crtcs: info.crtcs.iter().map(|&crtc| CrtcXid(crtc)).collect(),
                modes: info.modes.iter().map(|&mode| ModeXid(mode)).collect(),
                num_preferred: info.num_preferred,
                clones: info.clones.iter().map(|&clone| OutputXid(clone)).collect(),
                name: info.name,
                edid,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let crtcs = resources
        .crtcs
        .iter()
        .map(|&id| {
            let info = randr.crtc_info(id, config).map_err(x("GetCrtcInfo"))?;
            // All zero where the driver has no panning (Xvfb's): the server
            // answers so without asking the driver (`ProcRRGetPanning`, X
            // server 21.1.26).
            let pan = randr.get_panning(id).map_err(x("GetPanning"))?;
            let panning = Some(Panning {
                left: pan.left,
                top: pan.top,
                width: pan.width,
                height: pan.height,
                track_left: pan.track_left,
                track_top: pan.track_top,
                track_width: pan.track_width,
                track_height: pan.track_height,
                border_left: pan.border_left,
                border_top: pan.border_top,
                border_right: pan.border_right,
                border_bottom: pan.border_bottom,
            })
            .filter(|pan| *pan != Panning::default());
            let transform = randr.crtc_transform(id).map_err(x("GetCrtcTransform"))?;
            Ok(CrtcInfo {
                id: CrtcXid(id),
                timestamp: info.timestamp,
                x: info.x,
                y: info.y,
                width: info.width,
                height: info.height,
                mode: (info.mode != 0).then_some(ModeXid(info.mode)),
                rotation: Rotation(info.rotation.0),
                rotations: Rotation(info.rotations.0),
                outputs: info
                    .outputs
                    .iter()
                    .map(|&output| OutputXid(output))
                    .collect(),
                possible: info
                    .possible
                    .iter()
                    .map(|&output| OutputXid(output))
                    .collect(),
                panning,
                transform: (transform.current != crcbl_x11::Transform::IDENTITY)
                    .then_some(transform.current.matrix),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(Snapshot {
        resources: ScreenResources {
            timestamp: resources.timestamp,
            config_timestamp: resources.config_timestamp,
            crtcs: resources.crtcs.iter().map(|&crtc| CrtcXid(crtc)).collect(),
            outputs: resources
                .outputs
                .iter()
                .map(|&output| OutputXid(output))
                .collect(),
            modes: resources
                .modes
                .into_iter()
                .map(|mode| ModeInfo {
                    id: ModeXid(mode.id),
                    width: mode.width,
                    height: mode.height,
                    dot_clock: mode.dot_clock,
                    hsync_start: mode.hsync_start,
                    hsync_end: mode.hsync_end,
                    htotal: mode.htotal,
                    hskew: mode.hskew,
                    vsync_start: mode.vsync_start,
                    vsync_end: mode.vsync_end,
                    vtotal: mode.vtotal,
                    flags: mode.flags,
                    name: mode.name,
                })
                .collect(),
        },
        outputs,
        crtcs,
        size_range: SizeRange {
            min_width: range.min_width,
            min_height: range.min_height,
            max_width: range.max_width,
            max_height: range.max_height,
        },
        screen: ScreenSize {
            width: screen.width,
            height: screen.height,
            mm_width: u32::from(screen.width_mm),
            mm_height: u32::from(screen.height_mm),
        },
        primary: (primary != 0).then_some(OutputXid(primary)),
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crcbl_x11::Connection;

    use super::{Error, blocking_for};

    /// A server that takes the connection and never answers: libxcb waits
    /// for its setup reply forever. The call still returns at its limit,
    /// and so does the runtime it ran on, which a blocking task would hold
    /// up until libxcb returned.
    #[test]
    fn a_server_that_never_answers_is_given_up_on_at_the_limit() {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixListener};

        // A display number no X server has (nothing bound to its socket
        // name, as a file or in the abstract namespace), and a listener on
        // its abstract socket, which libxcb tries first for a local display:
        // no file is made, in /tmp/.X11-unix or anywhere. `unix/:N` names
        // the unix transport, so libxcb tries no TCP (libxcb 1.17 refuses
        // `unix:N` as a name it cannot parse).
        let bound = std::fs::read_to_string("/proc/net/unix").expect("/proc/net/unix");
        let number = (200..300)
            .find(|n| {
                let socket = format!("/tmp/.X11-unix/X{n}");
                !bound.lines().any(|line| line.ends_with(&socket))
                    && !std::path::Path::new(&socket).exists()
            })
            .expect("a free display number");
        let name = format!("/tmp/.X11-unix/X{number}");
        let address = SocketAddr::from_abstract_name(name.as_bytes()).expect("an abstract name");
        let _listener = UnixListener::bind_addr(&address).expect("bind");
        let display = format!("unix/:{number}");
        let limit = Duration::from_millis(300);

        let (done, finished) = std::sync::mpsc::channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("runtime");
            let result = runtime.block_on(blocking_for(limit, "the test", move || {
                Ok(Connection::connect(Some(&display))?.screen().root)
            }));
            // The runtime ends here: with a blocking task it would wait.
            drop(runtime);
            let _ = done.send(result.map(drop));
        });
        let result = finished
            .recv_timeout(Duration::from_secs(5))
            .expect("returned, and its runtime ended, within the deadline");
        assert!(
            matches!(result, Err(Error::Timeout { what: "the test" })),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
