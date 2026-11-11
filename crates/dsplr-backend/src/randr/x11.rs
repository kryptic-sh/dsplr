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

/// How long an apply may take: its read and plan, and every request it
/// sends. Longer than a read's [`OPERATION_TIMEOUT`], since a real modeset
/// takes the driver seconds (DisplayLink, nvidia, DisplayPort link training
/// across several monitors, one `SetCrtcConfig` after another). The revert
/// watchdog's delay is counted from it ([`crate::Backend::request_timeout`]),
/// so it stays above an apply and its revert.
pub const APPLY_TIMEOUT: Duration = Duration::from_secs(20);

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
    #[error("the X server did not answer {what} within {seconds} s")]
    Timeout { what: &'static str, seconds: u64 },
    /// The server is a Wayland compositor's Xwayland: its outputs are named
    /// `XWAYLAND0` and on. Its RandR does not set the compositor's outputs.
    #[error(
        "this X server is a Wayland compositor's Xwayland (its outputs are {0}), whose RandR does not set the compositor's outputs; use the compositor's own backend"
    )]
    Xwayland(String),
    #[error("the thread reading the X server ended: {0}")]
    Thread(String),
    /// The server answered a configuration request with a status other
    /// than success.
    #[error("the X server refused {request}: {status:?}")]
    Refused {
        request: &'static str,
        status: crcbl_x11::SetConfigStatus,
    },
    /// The server said its configuration changed since it was read
    /// (`InvalidConfigTime`, `InvalidTime`; X.Org never says so for
    /// `SetCrtcConfig`, which checks neither timestamp).
    #[error("the X server's configuration changed while {request} was sent: {status:?}")]
    Stale {
        request: &'static str,
        status: crcbl_x11::SetConfigStatus,
    },
}

/// The X server `DISPLAY` names, driven through RandR.
#[derive(Clone, Debug)]
pub struct Randr {
    /// The display name, as `DISPLAY` gave it.
    display: String,
    /// The RandR version the server settled on, when it was found.
    version: crcbl_x11::RandrVersion,
    /// Whether `WAYLAND_DISPLAY` was set: `DISPLAY` is then a Wayland
    /// compositor's Xwayland, whose RandR sets nothing on its outputs.
    xwayland: bool,
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
        let version = blocking("RandR's version", move || on_server(&name, version)).await?;
        let xwayland = var("WAYLAND_DISPLAY").is_some_and(|wayland| !wayland.is_empty());
        Ok(Some(Self {
            display,
            version,
            xwayland,
        }))
    }

    /// Whether this X server is a Wayland compositor's Xwayland
    /// (`WAYLAND_DISPLAY` was set when it was found).
    #[must_use]
    pub const fn xwayland(&self) -> bool {
        self.xwayland
    }

    /// Sets the server to `layout`: reads its state on a connection of its
    /// own, plans the requests on it ([`super::plan`]) and sends them there
    /// ([`send`]), all within [`APPLY_TIMEOUT`]. Returns what the plan did
    /// that the layout did not ask for ([`super::Note`]), one line each. A
    /// server whose outputs are Xwayland's is refused, nothing sent.
    ///
    /// On a timeout the thread is left behind with its connection, and any
    /// server grab it holds ends when the server answers it or the
    /// connection closes; a `SetCrtcConfig` it had sent may still take
    /// effect then. Every read and apply of a server waits for the one
    /// before it ([`on_server`]), so the read the apply flow makes next, to
    /// revert or report, sees what that late apply did, or times out too.
    ///
    /// # Errors
    ///
    /// As [`Randr::read`]; why the layout cannot be planned; a request the
    /// server failed or refused.
    pub async fn apply(&self, layout: &Layout) -> Result<Vec<String>, Error> {
        let name = self.display.clone();
        let layout = layout.clone();
        blocking_for(APPLY_TIMEOUT, "applying the layout", move || {
            on_server(&name, |connection| {
                version(connection)?;
                let snapshot = read(connection)?;
                if let Some(names) = xwayland_outputs(&snapshot) {
                    return Err(Error::Xwayland(names));
                }
                let plan = super::plan(&snapshot, &layout)?;
                send(connection, &snapshot, &plan)?;
                Ok(plan.notes.iter().map(ToString::to_string).collect())
            })
        })
        .await
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
            on_server(&name, |connection| {
                version(connection)?;
                read(connection)
            })
        })
        .await
    }

    /// Sends `plan` as it is, without planning, on a connection kept open
    /// while the server is read on another within `limit`: for the e2e test
    /// showing that a request failing under the grab leaves the server
    /// ungrabbed, with a plan the planner would not make. The first result
    /// is the send's, the second the read's.
    #[doc(hidden)]
    pub async fn send_then_read_beside(
        &self,
        plan: super::Plan,
        limit: Duration,
    ) -> Result<(Result<(), Error>, Result<Snapshot, Error>), Error> {
        let name = self.display.clone();
        blocking_for(APPLY_TIMEOUT, "the test's plan", move || {
            on_server(&name, |connection| {
                version(connection)?;
                let snapshot = read(connection)?;
                let sent = send(connection, &snapshot, &plan);
                // Read on a second connection while this one stays open:
                // a grab it still held would hold the read up.
                let (done, result) = std::sync::mpsc::channel();
                let beside = name.clone();
                std::thread::spawn(move || {
                    let _ = done.send(
                        Connection::connect(Some(&beside))
                            .map_err(Error::from)
                            .and_then(|other| read(&other)),
                    );
                });
                let read = result.recv_timeout(limit).unwrap_or(Err(Error::Timeout {
                    what: "the read beside the test's plan",
                    seconds: limit.as_secs(),
                }));
                Ok((sent, read))
            })
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

/// The names of `snapshot`'s outputs when they are a Wayland compositor's
/// Xwayland's (`XWAYLAND` and a number, as Xwayland names each output after
/// the compositor's), `None` when any is not. crcbl-x11 0.19.1 offers no
/// `QueryExtension`, which would find the `XWAYLAND` extension instead.
fn xwayland_outputs(snapshot: &Snapshot) -> Option<String> {
    let xwayland = |name: &str| {
        name.strip_prefix("XWAYLAND")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
    };
    let names: Vec<&str> = snapshot
        .outputs
        .iter()
        .map(|output| output.name.as_str())
        .collect();
    (!names.is_empty() && names.iter().all(|name| xwayland(name))).then(|| names.join(", "))
}

/// One lock per X server, by display name: what [`on_server`] holds.
static SERVERS: std::sync::Mutex<Vec<(String, std::sync::Arc<std::sync::Mutex<()>>)>> =
    std::sync::Mutex::new(Vec::new());

/// The lock for the server `display` names.
fn server_lock(display: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
    let mut servers = SERVERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, lock)) = servers.iter().find(|(name, _)| name == display) {
        return lock.clone();
    }
    let lock = std::sync::Arc::new(std::sync::Mutex::new(()));
    servers.push((display.to_owned(), lock.clone()));
    lock
}

/// Runs `work` on a new connection to the server `display` names, holding
/// that server's lock throughout: every read and apply of one server, in
/// this process, runs after the one before it has ended. So when an apply
/// times out and is left behind, the read that follows waits for it rather
/// than racing it, and sees what it did. Taken on the worker thread, so the
/// wait counts in the caller's limit.
fn on_server<T>(
    display: &str,
    work: impl FnOnce(&Connection) -> Result<T, Error>,
) -> Result<T, Error> {
    let lock = server_lock(display);
    // A worker that panicked holding it left nothing half-done that the
    // lock guards: the server's own state is read afresh each time.
    let _held = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let connection = Connection::connect(Some(display))?;
    work(&connection)
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
        Err(_) => Err(Error::Timeout {
            what,
            seconds: limit.as_secs(),
        }),
    }
}

/// Sends `plan`'s requests on `connection`, in order: the modes it makes
/// first, mapping each [`ModeRef::Created`] to the XID the server gave its
/// `CreateMode`; then, under a server grab, the CRTCs turned off, the
/// screen's size, each CRTC set (each `SetPanning` where the plan puts it),
/// the primary output, and the ungrab. The grab is released on every path,
/// an error included: when releasing it fails, the connection is gone, and
/// the grab with it.
///
/// [`ModeRef::Created`]: super::ModeRef::Created
fn send(connection: &Connection, snapshot: &Snapshot, plan: &super::Plan) -> Result<(), Error> {
    let mut grabbed = false;
    let result = send_operations(connection, snapshot, plan, &mut grabbed);
    if grabbed {
        let ungrabbed = connection
            .ungrab_server()
            .and_then(|()| connection.flush())
            .map_err(|error| Error::X {
                what: "UngrabServer",
                error,
            });
        // An error before it says more than the ungrab's.
        result?;
        ungrabbed?;
        return Ok(());
    }
    result
}

/// [`send`]'s requests, `grabbed` saying whether the server is grabbed.
fn send_operations(
    connection: &Connection,
    snapshot: &Snapshot,
    plan: &super::Plan,
    grabbed: &mut bool,
) -> Result<(), Error> {
    use super::{ModeRef, Operation};

    let x = |what| move |error| Error::X { what, error };
    let root = connection.screen().root;
    let config_timestamp = snapshot.resources.config_timestamp;
    let mut created: Vec<u32> = Vec::new();
    let mode = |mode: ModeRef, created: &[u32]| match mode {
        ModeRef::Listed(xid) => Ok(xid.0),
        ModeRef::Created(index) => created.get(index).copied().ok_or_else(|| {
            Error::Plan(super::PlanError::Inconsistent(format!(
                "the plan names created mode {index}, which it does not create"
            )))
        }),
    };
    // Whether a request that changes the configuration has been sent.
    let mut configured = false;
    for operation in &plan.operations {
        match operation {
            Operation::CreateMode(info) => {
                let xid = connection
                    .create_mode(root, &crcbl_mode(info))
                    .map_err(x("CreateMode"))?;
                created.push(xid);
            }
            Operation::AddOutputMode {
                output,
                mode: added,
            } => connection
                .add_output_mode(output.0, mode(*added, &created)?)
                .map_err(x("AddOutputMode"))?,
            Operation::GrabServer => {
                connection.grab_server().map_err(x("GrabServer"))?;
                *grabbed = true;
            }
            Operation::DisableCrtc(crtc) => {
                let reply = connection
                    .set_crtc_config(
                        crtc.0,
                        0,
                        config_timestamp,
                        &crcbl_x11::CrtcConfig::default(),
                    )
                    .map_err(x("SetCrtcConfig"))?;
                status("SetCrtcConfig", reply.status, configured)?;
                configured = true;
            }
            Operation::SetScreenSize {
                width,
                height,
                mm_width,
                mm_height,
            } => {
                connection
                    .set_screen_size(root, *width, *height, *mm_width, *mm_height)
                    .map_err(x("SetScreenSize"))?;
                configured = true;
            }
            Operation::SetCrtcConfig {
                crtc,
                mode: set,
                x: left,
                y: top,
                rotation,
                outputs,
            } => {
                let outputs: Vec<u32> = outputs.iter().map(|output| output.0).collect();
                let config = crcbl_x11::CrtcConfig {
                    x: *left,
                    y: *top,
                    mode: mode(*set, &created)?,
                    rotation: crcbl_x11::Rotation(rotation.0),
                    outputs: &outputs,
                };
                let reply = connection
                    .set_crtc_config(crtc.0, 0, config_timestamp, &config)
                    .map_err(x("SetCrtcConfig"))?;
                status("SetCrtcConfig", reply.status, configured)?;
                configured = true;
            }
            Operation::SetPanning { crtc, panning } => {
                let reply = connection
                    .set_panning(crtc.0, &crcbl_panning(panning))
                    .map_err(x("SetPanning"))?;
                status("SetPanning", reply.status, configured)?;
                configured = true;
            }
            Operation::SetOutputPrimary(output) => {
                connection
                    .set_output_primary(root, output.0)
                    .map_err(x("SetOutputPrimary"))?;
                configured = true;
            }
            Operation::UngrabServer => {
                connection.ungrab_server().map_err(x("UngrabServer"))?;
                *grabbed = false;
            }
        }
    }
    connection.flush().map_err(x("flush"))
}

/// A configuration request's `status`: success, or why not. A stale one
/// (`InvalidConfigTime`, `InvalidTime`: the server's configuration changed
/// since it was read) is [`Error::Stale`], which nothing is reverted for,
/// only while nothing has been configured yet; once something has
/// (`configured`), it is [`Error::Refused`], which the apply flow reverts.
/// X.Org 21.1.26 checks neither timestamp on `SetCrtcConfig`, so never
/// answers either there.
fn status(
    request: &'static str,
    status: crcbl_x11::SetConfigStatus,
    configured: bool,
) -> Result<(), Error> {
    use crcbl_x11::SetConfigStatus::{InvalidConfigTime, InvalidTime, Success};
    match status {
        Success => Ok(()),
        InvalidConfigTime | InvalidTime if !configured => Err(Error::Stale { request, status }),
        status => Err(Error::Refused { request, status }),
    }
}

/// crcbl-x11's form of `mode`, its id unset (`CreateMode` ignores it).
fn crcbl_mode(mode: &ModeInfo) -> crcbl_x11::ModeInfo {
    crcbl_x11::ModeInfo {
        id: mode.id.0,
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
        name: mode.name.clone(),
    }
}

/// crcbl-x11's form of `panning`, at the server's current time.
const fn crcbl_panning(panning: &Panning) -> crcbl_x11::Panning {
    crcbl_x11::Panning {
        timestamp: 0,
        left: panning.left,
        top: panning.top,
        width: panning.width,
        height: panning.height,
        track_left: panning.track_left,
        track_top: panning.track_top,
        track_width: panning.track_width,
        track_height: panning.track_height,
        border_left: panning.border_left,
        border_top: panning.border_top,
        border_right: panning.border_right,
        border_bottom: panning.border_bottom,
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
                // The pending one too: set but not yet in force, it takes
                // effect at the CRTC's next SetCrtcConfig, so it is no
                // less in the way of a plan.
                transform: [transform.current, transform.pending]
                    .into_iter()
                    .find(|matrix| *matrix != crcbl_x11::Transform::IDENTITY)
                    .map(|matrix| matrix.matrix),
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

// Abstract-namespace sockets and /proc/net/unix are Linux's.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::time::{Duration, Instant};

    use crcbl_x11::Connection;

    use super::{Error, blocking_for, on_server, server_lock, status, xwayland_outputs};
    use crate::randr::resources::{OutputInfo, Snapshot};

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
            matches!(
                result,
                Err(Error::Timeout {
                    what: "the test",
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_stale_status_is_stale_only_before_anything_was_configured() {
        use crcbl_x11::SetConfigStatus::{Failed, InvalidConfigTime, InvalidTime, Success};
        assert!(status("SetCrtcConfig", Success, true).is_ok());
        for stale in [InvalidConfigTime, InvalidTime] {
            assert!(matches!(
                status("SetCrtcConfig", stale, false),
                Err(Error::Stale { .. })
            ));
            // Something was set already: going back is the way out.
            assert!(matches!(
                status("SetCrtcConfig", stale, true),
                Err(Error::Refused { .. })
            ));
        }
        assert!(matches!(
            status("SetPanning", Failed, false),
            Err(Error::Refused { .. })
        ));
    }

    #[test]
    fn an_xwayland_server_is_known_by_its_output_names() {
        let named = |names: &[&str]| Snapshot {
            outputs: names
                .iter()
                .map(|name| OutputInfo {
                    name: (*name).to_owned(),
                    ..OutputInfo::default()
                })
                .collect(),
            ..Snapshot::default()
        };
        assert_eq!(
            xwayland_outputs(&named(&["XWAYLAND0", "XWAYLAND1"])),
            Some("XWAYLAND0, XWAYLAND1".to_owned())
        );
        for real in [
            &["DP-1"][..],
            &["XWAYLAND0", "DP-1"],
            &["XWAYLAND"],
            &["XWAYLANDx"],
            &[],
        ] {
            assert_eq!(xwayland_outputs(&named(real)), None, "{real:?}");
        }
    }

    /// A server's work waits for the work on it before, held by a worker
    /// left running (an apply that timed out, say), and runs once it ends;
    /// another server's does not wait.
    #[test]
    fn work_on_one_server_waits_for_the_work_before_it() {
        let display = "unix/:test-serial";
        let held = server_lock(display);
        let guard = held.lock().expect("lock");
        let (ran, after) = std::sync::mpsc::channel();
        let waiting = std::thread::spawn(move || {
            // Fails to connect once it has the lock (libxcb cannot parse
            // the name, so no server is reached): the order is what this
            // test reads.
            let _ = on_server(display, |_| Ok(()));
            let _ = ran.send(Instant::now());
        });
        assert!(
            after.recv_timeout(Duration::from_millis(300)).is_err(),
            "it ran while the lock was held"
        );
        let released = Instant::now();
        drop(guard);
        let ran_at = after
            .recv_timeout(Duration::from_secs(5))
            .expect("it ran once released");
        assert!(ran_at >= released);
        waiting.join().expect("joined");

        // Another server's lock is its own.
        let other = server_lock("unix/:test-other");
        let _other = other.lock().expect("lock");
        assert!(server_lock(display).try_lock().is_ok());
    }
}
