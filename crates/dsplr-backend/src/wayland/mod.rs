//! What the backends that speak Wayland share: the generated protocol code,
//! finding the compositor's socket, a connection driven from tokio
//! ([`client`]), and the outputs an output-management protocol describes
//! ([`heads`], [`plan`]). The `wlr` and `kde` backends build on it.

pub(crate) mod client;
pub mod heads;
pub mod plan;

#[allow(
    dead_code,
    reason = "every interface in the XML is generated; dsplr uses a few"
)]
#[allow(
    non_snake_case,
    reason = "the scanner keeps the XML's argument names, and KDE's has `serialNumber` and `eisaId`"
)]
#[allow(
    clippy::too_many_arguments,
    reason = "a request takes the arguments the XML gives it (KDE's `add_cvt` has twelve)"
)]
pub(crate) mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
}

use std::ffi::OsString;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crcbl_wayland::{ConnectError, Disconnected};

/// How long the compositor gets to answer anything dsplr waits for. The
/// same as Hyprland's request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest one read, test or apply takes, connecting included: each
/// wait inside it has [`REQUEST_TIMEOUT`], and the whole of it this.
pub const OPERATION_TIMEOUT: Duration = REQUEST_TIMEOUT.saturating_mul(2);

/// What went wrong reaching the compositor or following its objects,
/// whichever protocol was spoken.
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
    /// The compositor withdrew the object dsplr follows the outputs through.
    #[error("the compositor withdrew its {0}")]
    Finished(&'static str),
    #[error("libwayland would not route a {0}'s events to dsplr")]
    Dispatcher(&'static str),
    #[error("the compositor did not {what} in time")]
    Timeout { what: &'static str },
}

/// `operation` cut off at [`OPERATION_TIMEOUT`].
pub(crate) async fn bounded<T, E: From<Error>>(
    what: &'static str,
    operation: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| Error::Timeout { what })?
}

/// The pid of the process listening on `socket`, as the kernel reports it
/// for a connection to it (`SO_PEERCRED`: the listener's at `listen`), to
/// tell which process serves a compositor's socket. `None` when it cannot
/// be connected to within `timeout` or the pid is not known.
pub(crate) async fn peer_pid(socket: &Path, timeout: Duration) -> Option<i32> {
    let stream = tokio::time::timeout(timeout, tokio::net::UnixStream::connect(socket))
        .await
        .ok()?
        .ok()?;
    stream.peer_cred().ok()?.pid()
}

/// Where the Wayland socket is, by libwayland's rule: `WAYLAND_DISPLAY`
/// itself when absolute, else under `XDG_RUNTIME_DIR`.
pub(crate) fn socket_path(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
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
pub(crate) mod tests {
    use core::ffi::CStr;
    use core::ptr;
    use std::ffi::OsString;
    use std::path::Path;

    use crcbl_wayland::ffi::{self, WlArgument, WlMessage};

    use super::{Error, socket_path};

    /// Runs `f` on `slots` as libwayland hands a dispatcher an event whose
    /// signature is `signature`.
    pub fn dispatched<R>(
        signature: &'static CStr,
        slots: &mut [WlArgument],
        f: impl for<'a> FnOnce(&'a [WlArgument]) -> R,
    ) -> R {
        let message = WlMessage::new(c"event", signature, ptr::null());
        // SAFETY: each caller's `signature` names no more slots than `slots`
        // holds, all initialised, and nothing touches them while `f` runs.
        unsafe { ffi::with_dispatcher_args(&raw const message, slots.as_mut_ptr(), f) }
    }

    /// The variables `pairs` names, as an environment to read.
    pub fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
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
}
