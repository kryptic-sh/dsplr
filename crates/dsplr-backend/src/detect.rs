//! Finding the backend for the running desktop (`docs/plan/04-backends.md#detection`).

use crate::Backend;

/// The outcome of [`detect`]: what was looked at, and the backend found.
#[derive(Debug)]
pub struct Detection {
    /// What was checked, in order, and what was found.
    pub checks: Vec<Check>,
    /// `None` when nothing usable was found; [`Detection::checks`] says why.
    pub backend: Option<Backend>,
}

/// One thing detection looked at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Check {
    pub what: &'static str,
    pub found: String,
}

impl Check {
    fn new(what: &'static str, found: impl Into<String>) -> Self {
        Self {
            what,
            found: found.into(),
        }
    }
}

/// What detection says about the desktops whose backends do not exist yet.
#[cfg(target_os = "linux")]
const NOT_WRITTEN: &str = "not written yet (planned for M4 to M7)";

/// Finds the backend for the running desktop. Hyprland is tried first; the
/// wlroots, KDE, GNOME and X11 backends are not written yet. On other
/// targets than Linux nothing is found.
pub async fn detect() -> Detection {
    #[cfg(target_os = "linux")]
    {
        detect_with(|name| std::env::var_os(name)).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        Detection {
            checks: vec![Check::new(
                "platform",
                "not Linux: dsplr supports Linux only",
            )],
            backend: None,
        }
    }
}

/// [`detect`] on Linux, reading the environment through `var`.
#[cfg(target_os = "linux")]
async fn detect_with(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Detection {
    use crate::hyprland::{Error, Hyprland};

    let mut checks = Vec::new();
    match Hyprland::from_vars(var) {
        Err(error @ (Error::NotRunning | Error::NoRuntimeDir)) => {
            checks.push(Check::new("Hyprland", error.to_string()));
        }
        Err(error) => checks.push(Check::new("Hyprland", error.to_string())),
        Ok(hyprland) => match hyprland.version().await {
            Ok(version) => {
                checks.push(Check::new(
                    "Hyprland",
                    format!("socket answered: Hyprland {version}"),
                ));
                return Detection {
                    checks,
                    backend: Some(Backend::Hyprland(hyprland)),
                };
            }
            Err(error) => {
                checks.push(Check::new(
                    "Hyprland",
                    format!("socket did not answer: {error}"),
                ));
            }
        },
    }
    checks.push(Check::new("wlroots, KDE, GNOME, X11", NOT_WRITTEN));
    Detection {
        checks,
        backend: None,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    use super::detect_with;

    fn env(runtime: &Path) -> impl Fn(&str) -> Option<OsString> + '_ {
        move |name| match name {
            "HYPRLAND_INSTANCE_SIGNATURE" => Some("sig".into()),
            "XDG_RUNTIME_DIR" => Some(runtime.as_os_str().to_owned()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_hyprland_whose_socket_answers_is_the_backend() {
        let runtime = tempfile::tempdir().expect("tempdir");
        let dir = runtime.path().join("hypr").join("sig");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let listener = UnixListener::bind(dir.join(".socket.sock")).expect("bind");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0; 64];
            let _ = stream.read(&mut buf).await.expect("read");
            stream
                .write_all(br#"{"version": "0.56.2"}"#)
                .await
                .expect("write");
        });

        let detection = detect_with(env(runtime.path())).await;
        let backend = detection.backend.expect("found");
        assert_eq!(backend.name(), "hyprland");
        assert_eq!(detection.checks.len(), 1);
        assert_eq!(
            detection.checks[0].found,
            "socket answered: Hyprland 0.56.2"
        );
    }

    #[tokio::test]
    async fn a_hyprland_socket_that_is_not_there_is_reported() {
        let runtime = tempfile::tempdir().expect("tempdir");
        let detection = detect_with(env(runtime.path())).await;
        assert!(detection.backend.is_none());
        assert!(
            detection.checks[0]
                .found
                .starts_with("socket did not answer")
        );
        assert_eq!(
            detection.checks.last().map(|c| c.what),
            Some("wlroots, KDE, GNOME, X11")
        );
    }

    #[tokio::test]
    async fn no_hyprland_in_the_environment_is_reported() {
        let detection = detect_with(|_| None).await;
        assert!(detection.backend.is_none());
        assert!(
            detection.checks[0]
                .found
                .contains("HYPRLAND_INSTANCE_SIGNATURE")
        );
    }
}
