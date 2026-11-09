//! Whether the compositor is sway: its IPC socket, which sway names in
//! `SWAYSOCK` for what it starts, answers sway's own request. `SWAYSOCK`
//! alone is not enough: a session manager that imported it
//! (`systemctl --user import-environment`) can hand a stale one to a later
//! session under another compositor.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

/// The start of every message on sway's IPC socket, i3's `i3-ipc` header
/// (sway-ipc(7)).
const MAGIC: &[u8; 6] = b"i3-ipc";

/// sway-ipc(7)'s `GET_VERSION` message type.
const GET_VERSION: u32 = 7;

/// Whether `socket` answers sway's `GET_VERSION` with a reply of that type,
/// sending and reading each within `timeout`. A socket that is gone,
/// refuses, stalls or answers anything else is not sway's. It blocks the
/// calling thread, so it is asked only where the answer is needed (saving,
/// `dsplr backend`), not on every command.
pub fn answers(socket: &Path, timeout: Duration) -> bool {
    get_version(socket, timeout).unwrap_or(false)
}

/// Sends `GET_VERSION` and reads the reply's header; `true` when it is
/// sway's and of that type. Lengths and types are in the host's byte order
/// (sway-ipc(7)).
fn get_version(socket: &Path, timeout: Duration) -> std::io::Result<bool> {
    let mut request = MAGIC.to_vec();
    request.extend(0_u32.to_ne_bytes());
    request.extend(GET_VERSION.to_ne_bytes());
    let mut stream = super::ipc::send(socket, timeout, &request)?;
    let mut header = [0_u8; 14];
    stream.read_exact(&mut header)?;
    let (magic, rest) = header.split_at(MAGIC.len());
    Ok(magic == MAGIC && rest[4..] == GET_VERSION.to_ne_bytes())
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::{GET_VERSION, MAGIC, answers};

    /// Long enough for a socket on this machine to answer, short enough
    /// that a stalled one does not slow the suite.
    const TIMEOUT: Duration = Duration::from_millis(200);

    /// A socket at `path` that answers `reply` to the first connection
    /// sending a request, passing over any that hang up first (as one
    /// asking only which process serves the socket does).
    pub(in crate::wlr) fn serve(path: &Path, reply: Vec<u8>) {
        let listener = UnixListener::bind(path).expect("bind");
        std::thread::spawn(move || {
            loop {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut request = [0_u8; 14];
                if stream.read_exact(&mut request).is_err() {
                    continue;
                }
                assert_eq!(&request[..6], MAGIC);
                assert_eq!(request[10..], GET_VERSION.to_ne_bytes());
                stream.write_all(&reply).expect("reply");
                break;
            }
        });
    }

    pub(in crate::wlr) fn message(magic: &[u8], kind: u32, payload: &[u8]) -> Vec<u8> {
        let mut message = magic.to_vec();
        message.extend(u32::try_from(payload.len()).unwrap().to_ne_bytes());
        message.extend(kind.to_ne_bytes());
        message.extend(payload);
        message
    }

    #[test]
    fn a_socket_answering_get_version_is_sway_s() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sway-ipc.sock");
        serve(&path, message(MAGIC, GET_VERSION, br#"{"major": 1}"#));
        assert!(answers(&path, TIMEOUT));
    }

    #[test]
    fn anything_else_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(
            !answers(&dir.path().join("gone.sock"), TIMEOUT),
            "no socket"
        );
        let other_type = dir.path().join("other.sock");
        serve(&other_type, message(MAGIC, 0, b"[]"));
        assert!(!answers(&other_type, TIMEOUT), "another reply type");
        let other_magic = dir.path().join("magic.sock");
        serve(&other_magic, message(b"xx-ipc", GET_VERSION, b"{}"));
        assert!(!answers(&other_magic, TIMEOUT), "not i3-ipc");
        let short = dir.path().join("short.sock");
        serve(&short, b"i3-ip".to_vec());
        assert!(!answers(&short, TIMEOUT), "a cut reply");
    }

    /// A socket that takes the request and never answers is given up on
    /// after the timeout, not waited on.
    #[test]
    fn a_socket_that_never_answers_is_not_sway_s_after_the_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("stalled.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let held = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            // Reads until dsplr hangs up, answering nothing.
            std::io::copy(&mut stream, &mut std::io::sink()).expect("read until dsplr hangs up");
        });
        let started = Instant::now();
        assert!(!answers(&path, TIMEOUT));
        let took = started.elapsed();
        assert!(took >= TIMEOUT, "it waited for an answer: {took:?}");
        assert!(took < TIMEOUT * 10, "it gave up at the timeout: {took:?}");
        held.join().expect("the stalled socket");
    }
}
