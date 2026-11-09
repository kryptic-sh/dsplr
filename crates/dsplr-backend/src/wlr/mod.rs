//! wlroots compositors, over `zwlr_output_manager_v1`
//! (`docs/plan/04-backends.md#wlroots-compositors-wlr`).
//!
//! [`Wlr`] finds the compositor and does each read, test and apply on a
//! connection of its own; [`Connection`] is one such connection, and
//! [`Head`] what it reports about each output. Turning heads into dsplr's
//! display model, and a layout into a configuration, is plain data and
//! tested without a compositor.
//!
//! The protocol has no mirroring, no primary output and no colour settings:
//! a layout asking for any of them is refused, not applied without them.

mod connection;
mod heads;
mod ipc;
mod niri;
mod plan;
mod sway;

#[allow(
    dead_code,
    reason = "every interface in the XML is generated; dsplr uses a few"
)]
mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

use std::ffi::OsString;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crcbl_wayland::{ConnectError, Disconnected};
use dsplr_model::Layout;

pub use connection::{Connection, REQUEST_TIMEOUT};
pub use heads::{Head, HeadMode, SnapshotError};
pub use plan::{ADAPTIVE_SYNC_SINCE, PlanError};

/// What went wrong talking to the compositor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Wayland compositor here: WAYLAND_DISPLAY is not set")]
    NotRunning,
    #[error("XDG_RUNTIME_DIR is not set, so the Wayland socket {display:?} cannot be found")]
    NoRuntimeDir { display: OsString },
    #[error("{what} {path}: {source}")]
    Io {
        what: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("cannot speak Wayland: {0}")]
    Connect(ConnectError),
    #[error("the connection to the compositor failed: {0}")]
    Disconnected(Disconnected),
    #[error("the compositor offers no zwlr_output_manager_v1")]
    NoManager,
    #[error("the compositor stopped its output manager")]
    Finished,
    #[error("libwayland would not route a {0}'s events to dsplr")]
    Dispatcher(&'static str),
    #[error("the compositor did not {what} in time")]
    Timeout { what: &'static str },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("the layout cannot be sent: {0}")]
    Plan(#[from] PlanError),
    #[error("the compositor refused the configuration")]
    Failed,
    #[error(
        "the outputs changed after dsplr read them, so the compositor cancelled the configuration; read them again"
    )]
    Cancelled,
    #[error("cannot save the layout: {0}")]
    Save(#[source] dsplr_persist::sway::SaveError),
    #[error("cannot save the layout: {0}")]
    NiriSave(#[source] dsplr_persist::niri::SaveError),
}

/// The longest one read, test or apply takes, connecting included: each
/// wait inside it has [`REQUEST_TIMEOUT`], and the whole of it this.
pub const OPERATION_TIMEOUT: Duration = REQUEST_TIMEOUT.saturating_mul(2);

/// `operation` cut off at [`OPERATION_TIMEOUT`].
async fn bounded<T>(
    what: &'static str,
    operation: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| Error::Timeout { what })?
}

/// A wlroots compositor with an output manager: where its socket is, the
/// manager version it offers, and the sway and niri IPC sockets its
/// environment names.
#[derive(Clone, Debug)]
pub struct Wlr {
    socket: PathBuf,
    version: u32,
    swaysock: Option<PathBuf>,
    niri_socket: Option<PathBuf>,
}

/// Which compositor a [`Wlr`] is, as far as saving to its own config goes,
/// by the IPC sockets its environment names that answer as their
/// compositor's and belong to the process serving its Wayland socket
/// ([`Wlr::compositor`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compositor {
    /// `SWAYSOCK`'s socket answers sway's IPC.
    Sway,
    /// `NIRI_SOCKET`'s socket answers niri's IPC.
    Niri,
    /// Neither does, as the Wayland socket's own: another wlroots compositor,
    /// whose config dsplr does not write, or one nested in sway or niri,
    /// which inherited that compositor's variable.
    Other,
}

impl Wlr {
    /// The compositor this process runs under, found from its environment.
    pub async fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name)).await
    }

    /// [`Wlr::from_env`], reading the variables through `var`: connects to
    /// the socket `WAYLAND_DISPLAY` names and reads its heads, which is how
    /// detection checks that the output manager is there and answers. The
    /// sockets `SWAYSOCK` and `NIRI_SOCKET` name are kept, not asked:
    /// [`Wlr::compositor`] asks them.
    pub async fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        let named = |name| {
            var(name)
                .filter(|socket| !socket.is_empty())
                .map(PathBuf::from)
        };
        let (swaysock, niri_socket) = (named("SWAYSOCK"), named("NIRI_SOCKET"));
        let socket = socket_path(var)?;
        let version = bounded("describe its outputs", Connection::open(&socket))
            .await?
            .version();
        Ok(Self {
            socket,
            version,
            swaysock,
            niri_socket,
        })
    }

    /// The compositor's Wayland socket.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The `zwlr_output_manager_v1` version dsplr binds: the compositor's,
    /// up to the newest dsplr speaks.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Whether `SWAYSOCK` or `NIRI_SOCKET` named a socket, which sway and
    /// niri set for what they start (sway(1), niri's wiki): a claim that
    /// this is one of the compositors whose config dsplr writes, not yet
    /// checked.
    pub const fn names_compositor(&self) -> bool {
        self.swaysock.is_some() || self.niri_socket.is_some()
    }

    /// Which of the wlroots compositors whose config dsplr writes this is:
    /// the one whose IPC socket, as its environment names it, answers as
    /// its own within [`REQUEST_TIMEOUT`] and is served by the same process
    /// as the Wayland socket (their `SO_PEERCRED` pids). A compositor nested
    /// in sway or niri inherits `SWAYSOCK` or `NIRI_SOCKET`, which answers,
    /// but from the outer compositor's process, so it is not taken for it.
    /// It asks those sockets each time, blocking the calling thread for the
    /// IPC request, so it is asked only where it decides something: saving,
    /// and `dsplr backend`.
    pub async fn compositor(&self) -> Compositor {
        let Some(display) = ipc::peer_pid(&self.socket, REQUEST_TIMEOUT).await else {
            return Compositor::Other;
        };
        let serves = async |socket: &Option<PathBuf>, answers: fn(&Path, Duration) -> bool| {
            let Some(socket) = socket.as_deref() else {
                return false;
            };
            ipc::peer_pid(socket, REQUEST_TIMEOUT).await == Some(display)
                && answers(socket, REQUEST_TIMEOUT)
        };
        if serves(&self.swaysock, sway::answers).await {
            Compositor::Sway
        } else if serves(&self.niri_socket, niri::answers).await {
            Compositor::Niri
        } else {
            Compositor::Other
        }
    }

    /// A new connection, with its heads read.
    pub async fn connect(&self) -> Result<Connection, Error> {
        Connection::open(&self.socket).await
    }

    /// The current state as a [`Layout`].
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        bounded("describe its outputs", async {
            Ok(self.connect().await?.layout()?)
        })
        .await
    }

    /// Asks the compositor whether it would apply `layout`, changing
    /// nothing. `Ok` is the compositor's own check, not a promise: applying
    /// may still fail.
    pub async fn test(&self, layout: &Layout) -> Result<(), Error> {
        bounded("test the layout", async {
            self.connect().await?.test(layout).await
        })
        .await
    }

    /// Sets every head to `layout` in one configuration, read and sent on a
    /// fresh connection. The compositor may round a setting, so read the
    /// state back to see what it shows.
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        bounded("apply the layout", async {
            self.connect().await?.apply(layout).await
        })
        .await
    }
}

/// Where the Wayland socket is, by libwayland's rule: `WAYLAND_DISPLAY`
/// itself when absolute, else under `XDG_RUNTIME_DIR`.
fn socket_path(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let display = var("WAYLAND_DISPLAY")
        .filter(|display| !display.is_empty())
        .ok_or(Error::NotRunning)?;
    if Path::new(&display).is_absolute() {
        return Ok(display.into());
    }
    let runtime = var("XDG_RUNTIME_DIR").ok_or_else(|| Error::NoRuntimeDir {
        display: display.clone(),
    })?;
    Ok(Path::new(&runtime).join(display))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{Compositor, Error, Wlr, socket_path};

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.into())
        }
    }

    #[test]
    fn the_environment_names_the_socket() {
        let env = [
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ];
        assert_eq!(
            socket_path(vars(&env)).expect("found"),
            Path::new("/run/user/1000/wayland-1")
        );
        assert_eq!(
            socket_path(vars(&[("WAYLAND_DISPLAY", "/tmp/w/wayland-9")])).expect("absolute"),
            Path::new("/tmp/w/wayland-9")
        );
        assert!(matches!(
            socket_path(vars(&env[1..])),
            Err(Error::NotRunning)
        ));
        assert!(matches!(
            socket_path(vars(&[("WAYLAND_DISPLAY", ""), env[1]])),
            Err(Error::NotRunning)
        ));
        assert!(matches!(
            socket_path(vars(&env[..1])),
            Err(Error::NoRuntimeDir { .. })
        ));
    }

    /// Detection does not ask sway's socket: one that never answers costs
    /// it nothing (it used to wait out the request timeout on every
    /// command).
    #[tokio::test]
    async fn detection_does_not_wait_on_sway_s_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stalled = dir.path().join("sway-ipc.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&stalled).expect("bind");
        let runtime = dir.path().to_str().expect("UTF-8");
        let swaysock = stalled.to_str().expect("UTF-8");
        let env = [
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", runtime),
            ("SWAYSOCK", swaysock),
        ];
        let started = std::time::Instant::now();
        let result = Wlr::from_vars(vars(&env)).await;
        assert!(matches!(result, Err(Error::Io { .. })), "{result:?}");
        assert!(
            started.elapsed() < super::REQUEST_TIMEOUT / 2,
            "{:?}",
            started.elapsed()
        );
    }

    /// Each IPC socket is asked, and one answering as its own compositor's,
    /// from the process serving the Wayland socket (here this test's own),
    /// names that compositor. A socket served by another process (a nested
    /// compositor's inherited one) is checked by the niri e2e test, which
    /// has two compositors to ask.
    #[tokio::test]
    async fn the_compositor_is_the_one_whose_socket_answers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _wayland =
            std::os::unix::net::UnixListener::bind(dir.path().join("wayland-0")).expect("bind");
        let sway = |name: &str| {
            let path = dir.path().join(name);
            super::sway::tests::serve(
                &path,
                super::sway::tests::message(b"i3-ipc", 7, br#"{"major": 1}"#),
            );
            path
        };
        let niri = |name: &str| {
            let path = dir.path().join(name);
            super::niri::tests::serve(&path, b"{\"Ok\":{\"Version\":\"26.04\"}}\n");
            path
        };
        let gone = dir.path().join("gone.sock");
        let wlr = |swaysock: Option<PathBuf>, niri_socket: Option<PathBuf>| Wlr {
            socket: dir.path().join("wayland-0"),
            version: 4,
            swaysock,
            niri_socket,
        };
        for (each, expected) in [
            (wlr(Some(sway("s1")), None), Compositor::Sway),
            (wlr(None, Some(niri("n1"))), Compositor::Niri),
            (wlr(Some(gone.clone()), Some(niri("n2"))), Compositor::Niri),
            (wlr(Some(sway("s2")), Some(gone.clone())), Compositor::Sway),
            (wlr(Some(sway("s3")), Some(niri("n3"))), Compositor::Sway),
            (
                wlr(Some(gone.clone()), Some(gone.clone())),
                Compositor::Other,
            ),
            (wlr(None, None), Compositor::Other),
        ] {
            assert!(each.names_compositor() || expected == Compositor::Other);
            assert_eq!(each.compositor().await, expected, "{each:?}");
        }
        // With no Wayland socket to compare with, nothing is taken.
        let without = Wlr {
            socket: dir.path().join("wayland-9"),
            ..wlr(Some(sway("s5")), Some(niri("n5")))
        };
        assert_eq!(without.compositor().await, Compositor::Other);
    }

    #[tokio::test]
    async fn a_socket_that_is_not_there_is_an_io_error_naming_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = dir.path().to_str().expect("UTF-8");
        let env = [
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", runtime),
        ];
        assert!(matches!(
            Wlr::from_vars(vars(&env)).await,
            Err(Error::Io { path, .. }) if path == dir.path().join("wayland-0")
        ));
    }
}
