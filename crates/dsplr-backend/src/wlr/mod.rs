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
/// manager version it offers, and the sway IPC socket its environment names.
#[derive(Clone, Debug)]
pub struct Wlr {
    socket: PathBuf,
    version: u32,
    swaysock: Option<PathBuf>,
}

impl Wlr {
    /// The compositor this process runs under, found from its environment.
    pub async fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name)).await
    }

    /// [`Wlr::from_env`], reading the variables through `var`: connects to
    /// the socket `WAYLAND_DISPLAY` names and reads its heads, which is how
    /// detection checks that the output manager is there and answers. The
    /// socket `SWAYSOCK` names is kept, not asked: [`Wlr::is_sway`] asks it.
    pub async fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        let swaysock = var("SWAYSOCK")
            .filter(|socket| !socket.is_empty())
            .map(PathBuf::from);
        let socket = socket_path(var)?;
        let version = bounded("describe its outputs", Connection::open(&socket))
            .await?
            .version();
        Ok(Self {
            socket,
            version,
            swaysock,
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

    /// Whether `SWAYSOCK` named a socket, which sway sets for what it starts
    /// (sway(1)): a claim that this is sway, not yet checked.
    pub const fn names_sway(&self) -> bool {
        self.swaysock.is_some()
    }

    /// Whether the compositor is sway, the one wlroots compositor whose
    /// config dsplr writes: the socket `SWAYSOCK` names answers sway's IPC
    /// within [`REQUEST_TIMEOUT`]. It asks that socket each time, blocking
    /// the calling thread meanwhile, so it is asked only where it decides
    /// something: saving, and `dsplr backend`.
    pub fn is_sway(&self) -> bool {
        self.swaysock
            .as_deref()
            .is_some_and(|socket| sway::answers(socket, REQUEST_TIMEOUT))
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
    use std::path::Path;

    use super::{Error, Wlr, socket_path};

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
