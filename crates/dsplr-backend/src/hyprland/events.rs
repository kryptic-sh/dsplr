//! Following Hyprland's changes over its event socket (`.socket2.sock`),
//! where it writes one `EVENT>>DATA` line per event.
//!
//! The events that say the outputs changed are a monitor added or removed,
//! and the config reloaded (which re-reads its monitor rules). Hyprland
//! sends none when a monitor rule changes through `eval` or `keyword`, so a
//! change made that way by another program is not followed; dsplr's own
//! applies are read back by the requests that make them.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::net::UnixStream;

use super::Error;

/// The events after which the outputs may differ.
const OUTPUT_EVENTS: [&str; 5] = [
    "monitoradded",
    "monitoraddedv2",
    "monitorremoved",
    "monitorremovedv2",
    "configreloaded",
];

/// An open event socket.
#[derive(Debug)]
pub struct Events {
    lines: Lines<BufReader<UnixStream>>,
    path: PathBuf,
}

impl Events {
    /// Connects to the event socket at `path`.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|source| Error::Io {
                what: "connecting to",
                path: path.to_owned(),
                source,
            })?;
        Ok(Self {
            lines: BufReader::new(stream).lines(),
            path: path.to_owned(),
        })
    }

    /// Waits, for as long as it takes, for an event after which the outputs
    /// may differ. Cancel-safe: a line partly read is kept for the next call
    /// (tokio's `Lines::next_line`).
    pub async fn changed(&mut self) -> Result<(), Error> {
        loop {
            let line = self.lines.next_line().await.map_err(|source| Error::Io {
                what: "reading",
                path: self.path.clone(),
                source,
            })?;
            let Some(line) = line else {
                return Err(Error::Io {
                    what: "reading",
                    path: self.path.clone(),
                    source: std::io::ErrorKind::UnexpectedEof.into(),
                });
            };
            if is_output_event(&line) {
                return Ok(());
            }
        }
    }
}

/// Whether `line`, one event as Hyprland writes it, says the outputs may
/// have changed.
fn is_output_event(line: &str) -> bool {
    let name = line.split_once(">>").map_or(line, |(name, _)| name);
    OUTPUT_EVENTS.contains(&name)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;

    use super::{Events, is_output_event};

    #[test]
    fn only_monitor_and_reload_events_are_changes() {
        for line in [
            "monitoradded>>DP-1",
            "monitoraddedv2>>3,DP-1,AOC Q27G4",
            "monitorremoved>>HDMI-A-1",
            "monitorremovedv2>>3,HDMI-A-1,AOC",
            "configreloaded>>",
        ] {
            assert!(is_output_event(line), "{line}");
        }
        for line in [
            "workspace>>2",
            "focusedmon>>DP-1,2",
            "activewindow>>kitty,~",
            "monitoradd>>DP-1",
            "",
        ] {
            assert!(!is_output_event(line), "{line}");
        }
    }

    #[tokio::test]
    async fn a_change_is_reported_past_other_events_and_the_end_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".socket2.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let hyprland = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream
                .write_all(b"workspace>>2\nfocusedmon>>DP-1,2\nmonitoradded>>DP-2\n")
                .await
                .unwrap();
        });
        let mut events = Events::open(&path).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), events.changed())
            .await
            .expect("a change in time")
            .expect("a change");
        hyprland.await.unwrap();
        assert!(events.changed().await.is_err(), "the socket closed");
    }
}
