//! What asking a compositor's IPC socket whether it is that compositor
//! shares, whichever compositor it is: connecting, and a timeout on every
//! send and read, so a socket that stalls is given up on. Which process
//! serves a socket, to tell the compositor's own IPC socket from one it
//! inherited, is [`crate::wayland::peer_pid`].

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
