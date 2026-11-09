//! What asking a compositor's IPC socket whether it is that compositor
//! shares, whichever compositor it is: connecting, and a timeout on every
//! send and read, so a socket that stalls is given up on; and which process
//! serves a socket, to tell the compositor's own IPC socket from one it
//! inherited.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Connects to `socket` and sends `request`, each send and later read on
/// the stream limited to `timeout`. Blocks the calling thread.
pub(super) fn send(
    socket: &Path,
    timeout: Duration,
    request: &[u8],
) -> std::io::Result<UnixStream> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(request)?;
    Ok(stream)
}

/// The pid of the process listening on `socket`, as the kernel reports it
/// for a connection to it (`SO_PEERCRED`: the listener's at `listen`).
/// `None` when it cannot be connected to within `timeout` or the pid is not
/// known.
pub(super) async fn peer_pid(socket: &Path, timeout: Duration) -> Option<i32> {
    let stream = tokio::time::timeout(timeout, tokio::net::UnixStream::connect(socket))
        .await
        .ok()?
        .ok()?;
    stream.peer_cred().ok()?.pid()
}
