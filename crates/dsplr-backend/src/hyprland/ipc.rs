//! Talking to a running Hyprland over its request socket.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use dsplr_model::Layout;
use dsplr_persist::hyprland::{MonitorNaming, MonitorsLua, WriteError, monitor_calls};

use super::{Monitor, SnapshotError, layout};

/// How long a request may take before dsplr gives up on it, connecting
/// included. `hyprctl` uses the same receive timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The request for every monitor, disabled ones included.
const MONITORS_REQUEST: &str = "j/monitors all";

/// The request for Hyprland's version.
const VERSION_REQUEST: &str = "j/version";

/// What Hyprland answers a request it carried out with.
const OK_REPLY: &str = "ok";

/// What went wrong talking to Hyprland.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Hyprland is not running here: HYPRLAND_INSTANCE_SIGNATURE is not set")]
    NotRunning,
    #[error("XDG_RUNTIME_DIR is not set, so Hyprland's sockets cannot be found")]
    NoRuntimeDir,
    #[error("{what} {path}: {source}")]
    Io {
        what: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("Hyprland did not answer `{request}` within {REQUEST_TIMEOUT:?}")]
    Timeout { request: String },
    #[error("Hyprland's reply to `{request}` is not the JSON dsplr expects: {source}")]
    Json {
        request: String,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("Hyprland refused `{request}`: {reply}")]
    Refused { request: String, reply: String },
    #[error("the layout cannot be put into monitor rules: {0}")]
    Write(#[from] WriteError),
}

/// A connection point to one Hyprland instance.
#[derive(Clone, Debug)]
pub struct Hyprland {
    socket_dir: PathBuf,
}

impl Hyprland {
    /// The Hyprland this process runs under, found from its environment.
    pub fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// [`Hyprland::from_env`], reading the variables through `var`.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        Ok(Self {
            socket_dir: socket_dir(var)?,
        })
    }

    /// Hyprland's version, such as `0.56.2`. Also how detection checks that
    /// the socket answers.
    pub async fn version(&self) -> Result<String, Error> {
        #[derive(serde::Deserialize)]
        struct Version {
            version: String,
        }
        let reply = self.request(VERSION_REQUEST).await?;
        serde_json::from_str::<Version>(&reply)
            .map(|v| v.version)
            .map_err(|source| Error::Json {
                request: VERSION_REQUEST.to_owned(),
                source,
            })
    }

    /// Runs `lua` through Hyprland's `eval`, as `hyprctl eval` does. Under a
    /// Lua config this is how settings change at runtime (`hyprctl keyword` is
    /// refused there). The changes last until the next config reload.
    ///
    /// `Ok` means Hyprland accepted the chunk; a monitor rule it accepted
    /// takes effect on Hyprland's next frame, not before this returns.
    pub async fn eval(&self, lua: &str) -> Result<(), Error> {
        let request = format!("/eval {lua}");
        let reply = self.request(&request).await?;
        if reply.trim() == OK_REPLY {
            Ok(())
        } else {
            Err(Error::Refused {
                request,
                reply: reply.trim().to_owned(),
            })
        }
    }

    /// Sets every output to `layout`, in one `eval` of a monitor rule per
    /// output with every setting stated, so nothing is left over from an
    /// earlier rule. Rules name outputs by connector: unlike `desc:`, which
    /// Hyprland matches by prefix, a connector names exactly one monitor.
    ///
    /// The rules are runtime-only, and a config reload drops them. As with
    /// [`Hyprland::eval`], the change shows on Hyprland's next frame, so read
    /// the state back to see what it made of it. Adaptive sync turned off is
    /// not written (see [`dsplr_persist::hyprland::monitor_call`]).
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        let rules = monitor_calls(
            &MonitorsLua {
                layout: layout.clone(),
                catch_all: None,
            },
            MonitorNaming::Connector,
        )?;
        self.eval(&rules).await
    }

    /// The current state as a [`Layout`].
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        Ok(layout(&self.monitors().await?)?)
    }

    /// Every monitor Hyprland knows, disabled ones included.
    pub async fn monitors(&self) -> Result<Vec<Monitor>, Error> {
        let reply = self.request(MONITORS_REQUEST).await?;
        Monitor::parse_all(&reply).map_err(|source| Error::Json {
            request: MONITORS_REQUEST.to_owned(),
            source,
        })
    }

    /// Sends one request and returns the whole reply. Hyprland takes one
    /// request per connection and closes it after replying.
    async fn request(&self, request: &str) -> Result<String, Error> {
        let path = self.socket_dir.join(".socket.sock");
        let exchange = async {
            let mut stream = UnixStream::connect(&path).await?;
            stream.write_all(request.as_bytes()).await?;
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await?;
            Ok::<_, io::Error>(reply)
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, exchange).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(source)) => Err(Error::Io {
                what: "requesting over",
                path,
                source,
            }),
            Err(_) => Err(Error::Timeout {
                request: request.to_owned(),
            }),
        }
    }
}

/// Where Hyprland's sockets are: `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`.
fn socket_dir(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error> {
    let signature = var("HYPRLAND_INSTANCE_SIGNATURE").ok_or(Error::NotRunning)?;
    let runtime = var("XDG_RUNTIME_DIR").ok_or(Error::NoRuntimeDir)?;
    Ok(Path::new(&runtime).join("hypr").join(signature))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    use super::{Error, Hyprland, socket_dir};

    fn vars<'a>(pairs: &'a [(&'a str, &'a Path)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_os_str().to_owned())
        }
    }

    #[test]
    fn the_environment_names_the_sockets() {
        let runtime = Path::new("/run/user/1000");
        let signature = Path::new("abc_123");
        let env = [
            ("HYPRLAND_INSTANCE_SIGNATURE", signature),
            ("XDG_RUNTIME_DIR", runtime),
        ];
        assert_eq!(
            socket_dir(vars(&env)).expect("found"),
            Path::new("/run/user/1000/hypr/abc_123")
        );
        assert!(matches!(
            socket_dir(vars(&env[1..])),
            Err(Error::NotRunning)
        ));
        assert!(matches!(
            socket_dir(vars(&env[..1])),
            Err(Error::NoRuntimeDir)
        ));
    }

    /// A stand-in Hyprland on `.socket.sock` in `dir`: answers each request
    /// with `reply`, and reports what it was asked.
    fn fake_hyprland(
        dir: &Path,
        reply: fn(&str) -> &'static str,
    ) -> tokio::sync::mpsc::UnboundedReceiver<String> {
        let listener = UnixListener::bind(dir.join(".socket.sock")).expect("bind");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0; 4096];
                let n = stream.read(&mut buf).await.expect("read");
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                stream
                    .write_all(reply(&request).as_bytes())
                    .await
                    .expect("write");
                tx.send(request).expect("send");
            }
        });
        rx
    }

    /// The owner's Hyprland, as recorded.
    fn owner(request: &str) -> &'static str {
        match request {
            "j/monitors all" => include_str!("../../tests/fixtures/hyprland/monitors-all.json"),
            "j/version" => r#"{"branch": "v0.56.2", "version": "0.56.2", "tag": "v0.56.2"}"#,
            _ => "unknown request",
        }
    }

    #[tokio::test]
    async fn eval_sends_the_chunk_and_checks_the_reply() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut requests = fake_hyprland(dir.path(), |request| {
            if request.contains("nonsense") {
                "error: hl.monitor: error applying field 'mode'"
            } else {
                "ok"
            }
        });
        let hyprland = at(dir.path());
        let good = r#"hl.monitor({ output = "DP-1", disabled = true })"#;
        hyprland.eval(good).await.expect("accepted");
        assert_eq!(
            requests.recv().await.as_deref(),
            Some(format!("/eval {good}").as_str())
        );
        assert!(matches!(
            hyprland.eval(r#"hl.monitor({ output = "DP-1", mode = "nonsense" })"#).await,
            Err(Error::Refused { reply, .. }) if reply == "error: hl.monitor: error applying field 'mode'"
        ));
    }

    #[tokio::test]
    async fn the_version_and_snapshot_come_from_their_requests() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut requests = fake_hyprland(dir.path(), owner);
        let hyprland = at(dir.path());
        assert_eq!(hyprland.version().await.expect("version"), "0.56.2");
        assert_eq!(requests.recv().await.as_deref(), Some("j/version"));
        let layout = hyprland.snapshot().await.expect("snapshot");
        assert_eq!(requests.recv().await.as_deref(), Some("j/monitors all"));
        assert_eq!(layout.entries.len(), 2);
    }

    #[tokio::test]
    async fn apply_evals_a_rule_per_output_named_by_connector() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut requests = fake_hyprland(dir.path(), |request| {
            if request.starts_with("/eval ") {
                "ok"
            } else {
                owner(request)
            }
        });
        let hyprland = at(dir.path());
        let layout = hyprland.snapshot().await.expect("snapshot");
        requests.recv().await.expect("the snapshot's request");

        hyprland.apply(&layout).await.expect("applied");
        let request = requests.recv().await.expect("the apply's request");
        let rules = request.strip_prefix("/eval ").expect("an eval");
        assert_eq!(rules.matches("hl.monitor(").count(), 2, "{rules}");
        // eDP-1 is disabled; HDMI-A-1 carries every setting.
        assert!(rules.contains(r#"output = "eDP-1""#), "{rules}");
        assert!(rules.contains("disabled = true"), "{rules}");
        for field in [
            r#"output = "HDMI-A-1""#,
            "disabled = false",
            r#"mode = "2560x1440@120""#,
            r#"position = "0x0""#,
            r#"mirror = """#,
        ] {
            assert!(rules.contains(field), "{field} in {rules}");
        }
        assert!(!rules.contains("desc:"), "{rules}");
    }

    #[tokio::test]
    async fn a_layout_that_cannot_be_written_is_not_sent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut requests = fake_hyprland(dir.path(), owner);
        let hyprland = at(dir.path());
        let mut layout = hyprland.snapshot().await.expect("snapshot");
        requests.recv().await.expect("the snapshot's request");

        layout.entries[1].state.mode = None;
        assert!(matches!(
            hyprland.apply(&layout).await,
            Err(Error::Write(_))
        ));
        assert!(requests.try_recv().is_err(), "nothing was sent");
    }

    fn at(dir: &Path) -> Hyprland {
        Hyprland {
            socket_dir: dir.to_owned(),
        }
    }

    #[tokio::test]
    async fn monitors_asks_for_every_monitor_and_parses_the_reply() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut requests = fake_hyprland(dir.path(), owner);

        let monitors = at(dir.path()).monitors().await.expect("monitors");
        assert_eq!(requests.recv().await.as_deref(), Some("j/monitors all"));
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[1].name, "HDMI-A-1");
    }

    #[tokio::test]
    async fn a_reply_that_is_not_monitors_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _requests = fake_hyprland(dir.path(), |_| "unknown request");
        assert!(matches!(
            at(dir.path()).monitors().await,
            Err(Error::Json { request, .. }) if request == "j/monitors all"
        ));
    }

    #[tokio::test]
    async fn no_socket_is_an_io_error_naming_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            at(dir.path()).monitors().await,
            Err(Error::Io { path, .. }) if path == dir.path().join(".socket.sock")
        ));
    }

    /// Read-only, so it runs against the session's own Hyprland.
    #[tokio::test]
    #[ignore = "needs a running Hyprland in this session"]
    async fn a_real_hyprland_lists_its_monitors() {
        let monitors = Hyprland::from_env()
            .expect("HYPRLAND_INSTANCE_SIGNATURE and XDG_RUNTIME_DIR are set")
            .monitors()
            .await
            .expect("Hyprland answered");
        assert!(!monitors.is_empty(), "Hyprland always has an output");
        for monitor in &monitors {
            monitor.modes().expect("every mode Hyprland lists parses");
        }
        let hyprland = Hyprland::from_env().expect("found");
        hyprland
            .version()
            .await
            .expect("Hyprland reports its version");
        let layout = hyprland.snapshot().await.expect("the state converts");
        assert_eq!(layout.entries.len(), monitors.len());
    }
}
