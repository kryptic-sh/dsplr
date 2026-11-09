//! Whether the compositor is niri: its IPC socket, which niri names in
//! `NIRI_SOCKET` for what it starts, answers niri's `Version` request. As
//! with sway, the variable alone is not enough: a stale one can outlive its
//! niri in a session manager's environment.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::Duration;

/// niri's `Version` request: a JSON string on one line, as `niri msg
/// version` sends it. niri 26.04 answers one JSON line,
/// `{"Ok":{"Version":"26.04 (8ed0da4)"}}`.
const VERSION: &[u8] = b"\"Version\"\n";

/// The most of a reply read: a version line is far shorter, so a socket
/// sending more is not niri's.
const MAX_REPLY: u64 = 4096;

/// Whether `socket` answers niri's `Version` request with a version,
/// sending and reading each within `timeout`. A socket that is gone,
/// refuses, stalls or answers anything else is not niri's. It blocks the
/// calling thread, so it is asked only where the answer is needed (saving,
/// `dsplr backend`), not on every command.
pub fn answers(socket: &Path, timeout: Duration) -> bool {
    version(socket, timeout).unwrap_or(false)
}

/// Sends `Version` and reads the reply's line; `true` when it is
/// `{"Ok":{"Version":"…"}}`.
fn version(socket: &Path, timeout: Duration) -> std::io::Result<bool> {
    let stream = super::ipc::send(socket, timeout, VERSION)?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REPLY)).read_line(&mut line)?;
    let reply: serde_json::Value = match serde_json::from_str(&line) {
        Ok(reply) => reply,
        Err(_) => return Ok(false),
    };
    Ok(reply
        .get("Ok")
        .and_then(|ok| ok.get("Version"))
        .is_some_and(serde_json::Value::is_string))
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::answers;

    /// Long enough for a socket on this machine to answer, short enough
    /// that a stalled one does not slow the suite.
    const TIMEOUT: Duration = Duration::from_millis(200);

    /// A socket at `path` that answers `reply` to the first connection
    /// sending a request line, passing over any that hang up first (as one
    /// asking only which process serves the socket does).
    pub(in crate::wlr) fn serve(path: &Path, reply: &'static [u8]) {
        let listener = UnixListener::bind(path).expect("bind");
        std::thread::spawn(move || {
            loop {
                let (stream, _) = listener.accept().expect("accept");
                let mut request = String::new();
                if BufReader::new(&stream)
                    .read_line(&mut request)
                    .expect("request")
                    == 0
                {
                    continue;
                }
                assert_eq!(request, "\"Version\"\n");
                (&stream).write_all(reply).expect("reply");
                break;
            }
        });
    }

    #[test]
    fn a_socket_answering_version_is_niri_s() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("niri.sock");
        serve(&path, b"{\"Ok\":{\"Version\":\"26.04 (8ed0da4)\"}}\n");
        assert!(answers(&path, TIMEOUT));
    }

    #[test]
    fn anything_else_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(
            !answers(&dir.path().join("gone.sock"), TIMEOUT),
            "no socket"
        );
        for (name, reply) in [
            ("error.sock", &b"{\"Err\":\"error parsing request\"}\n"[..]),
            ("other.sock", b"{\"Ok\":{\"Outputs\":{}}}\n"),
            ("number.sock", b"{\"Ok\":{\"Version\":26}}\n"),
            ("sway.sock", b"i3-ipc\0\0\0\0\x07\0\0\0"),
            ("cut.sock", b"{\"Ok\":{\"Vers"),
        ] {
            let path = dir.path().join(name);
            serve(&path, reply);
            assert!(!answers(&path, TIMEOUT), "{name}");
        }
    }

    /// A socket that takes the request and never answers is given up on
    /// after the timeout, not waited on.
    #[test]
    fn a_socket_that_never_answers_is_not_niri_s_after_the_timeout() {
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
