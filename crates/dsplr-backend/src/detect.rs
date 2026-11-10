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

/// What detection says about the desktop whose backend does not exist yet.
#[cfg(target_os = "linux")]
const NOT_WRITTEN: &str = "not written yet (planned for M7)";

/// Finds the backend for the running desktop, in the plan's order: Hyprland,
/// then KWin's output management, then Mutter's `DisplayConfig` on the
/// session bus, then a wlroots compositor's output manager, then X11 (not
/// written yet). When
/// Hyprland's variable is set but
/// its socket does not answer, detection stops there: this is Hyprland, and
/// its own `wlr-output-management` would pass for a wlroots compositor. On
/// other targets than Linux nothing is found.
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
    let mut checks = Vec::new();
    match hyprland(&var, &mut checks).await {
        Hyprland::Found(backend) => {
            return Detection {
                checks,
                backend: Some(backend),
            };
        }
        Hyprland::Unreachable => {
            checks.push(Check::new("other backends", HYPRLAND_UNREACHABLE));
            return Detection {
                checks,
                backend: None,
            };
        }
        Hyprland::NotRunning => {}
    }
    if let Some(backend) = kde(&var, &mut checks).await {
        return Detection {
            checks,
            backend: Some(backend),
        };
    }
    if let Some(backend) = gnome(&var, &mut checks).await {
        return Detection {
            checks,
            backend: Some(backend),
        };
    }
    if let Some(backend) = wlr(&var, &mut checks).await {
        return Detection {
            checks,
            backend: Some(backend),
        };
    }
    checks.push(Check::new("X11", NOT_WRITTEN));
    Detection {
        checks,
        backend: None,
    }
}

/// Why detection stops at a Hyprland that does not answer.
#[cfg(target_os = "linux")]
const HYPRLAND_UNREACHABLE: &str = "not tried: HYPRLAND_INSTANCE_SIGNATURE is set, so this is Hyprland, \
     whose wlr-output-management has no real test and whose state applied through it \
     overrides its monitor rules until restart";

/// What the Hyprland check found.
#[cfg(target_os = "linux")]
enum Hyprland {
    Found(Backend),
    /// `HYPRLAND_INSTANCE_SIGNATURE` is not set.
    NotRunning,
    /// It is set, but the socket cannot be found or does not answer.
    Unreachable,
}

/// Hyprland, when its variable is set and its socket answers.
#[cfg(target_os = "linux")]
async fn hyprland(
    var: &impl Fn(&str) -> Option<std::ffi::OsString>,
    checks: &mut Vec<Check>,
) -> Hyprland {
    use crate::hyprland::{Error, Hyprland as Ipc};

    let hyprland = match Ipc::from_vars(var) {
        Ok(hyprland) => hyprland,
        Err(error) => {
            checks.push(Check::new("Hyprland", error.to_string()));
            return if matches!(error, Error::NotRunning) {
                Hyprland::NotRunning
            } else {
                Hyprland::Unreachable
            };
        }
    };
    match hyprland.version().await {
        Ok(version) => {
            checks.push(Check::new(
                "Hyprland",
                format!("socket answered: Hyprland {version}"),
            ));
            Hyprland::Found(Backend::Hyprland(hyprland))
        }
        Err(error) => {
            checks.push(Check::new(
                "Hyprland",
                format!("socket did not answer: {error}"),
            ));
            Hyprland::Unreachable
        }
    }
}

/// KWin, when `WAYLAND_DISPLAY` names a compositor that offers
/// `kde_output_management_v2` and a device registry dsplr can bind, and
/// describes its outputs.
#[cfg(target_os = "linux")]
async fn kde(
    var: &impl Fn(&str) -> Option<std::ffi::OsString>,
    checks: &mut Vec<Check>,
) -> Option<Backend> {
    match crate::kde::Kde::from_vars(var).await {
        Ok(kde) => {
            checks.push(Check::new(
                KDE_CHECK,
                format!(
                    "kde_output_management_v2 version {} at {}",
                    kde.management_version(),
                    kde.socket().display()
                ),
            ));
            Some(Backend::Kde(kde))
        }
        Err(error) => {
            checks.push(Check::new(KDE_CHECK, error.to_string()));
            None
        }
    }
}

/// What the KDE check is called in [`Detection::checks`].
#[cfg(target_os = "linux")]
const KDE_CHECK: &str = "KDE";

/// Mutter, when the session bus the environment names has
/// `org.gnome.Mutter.DisplayConfig` owned and it describes the displays.
/// Under X11 GNOME offers RandR too, and this comes first because Mutter
/// persists (`docs/plan/04-backends.md#detection`).
#[cfg(target_os = "linux")]
async fn gnome(
    var: &impl Fn(&str) -> Option<std::ffi::OsString>,
    checks: &mut Vec<Check>,
) -> Option<Backend> {
    match crate::gnome::Gnome::from_vars(var).await {
        Ok(gnome) => {
            checks.push(Check::new(
                GNOME_CHECK,
                format!("org.gnome.Mutter.DisplayConfig at {}", gnome.address()),
            ));
            Some(Backend::Gnome(gnome))
        }
        Err(error) => {
            checks.push(Check::new(GNOME_CHECK, error.to_string()));
            None
        }
    }
}

/// What the GNOME check is called in [`Detection::checks`].
#[cfg(target_os = "linux")]
const GNOME_CHECK: &str = "GNOME";

/// A wlroots compositor, when `WAYLAND_DISPLAY` names a compositor that
/// offers `zwlr_output_manager_v1` and describes its heads.
#[cfg(target_os = "linux")]
async fn wlr(
    var: &impl Fn(&str) -> Option<std::ffi::OsString>,
    checks: &mut Vec<Check>,
) -> Option<Backend> {
    match crate::wlr::Wlr::from_vars(var).await {
        Ok(wlr) => {
            checks.push(Check::new(
                WLR_CHECK,
                format!(
                    "zwlr_output_manager_v1 version {} at {}",
                    wlr.version(),
                    wlr.socket().display()
                ),
            ));
            Some(Backend::Wlr(wlr))
        }
        Err(error) => {
            checks.push(Check::new(WLR_CHECK, error.to_string()));
            None
        }
    }
}

/// What the wlroots check is called in [`Detection::checks`].
#[cfg(target_os = "linux")]
const WLR_CHECK: &str = "wlroots";

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
        let order: Vec<_> = detection.checks.iter().map(|c| c.what).collect();
        assert_eq!(order, ["Hyprland", "other backends"], "stops at Hyprland");
        assert!(detection.checks[1].found.starts_with("not tried"));
    }

    #[tokio::test]
    async fn a_hyprland_that_does_not_answer_is_not_driven_as_a_wlroots_compositor() {
        let runtime = tempfile::tempdir().expect("tempdir");
        // A Wayland socket that would answer as KWin or wlroots if it were tried:
        // anything listening counts, since trying it is what must not happen.
        let _wayland = UnixListener::bind(runtime.path().join("wayland-1")).expect("bind");
        let detection = detect_with(|name| match name {
            "HYPRLAND_INSTANCE_SIGNATURE" => Some("sig".into()),
            "WAYLAND_DISPLAY" => Some("wayland-1".into()),
            "XDG_RUNTIME_DIR" => Some(runtime.path().as_os_str().to_owned()),
            _ => None,
        })
        .await;
        assert!(detection.backend.is_none());
        assert!(
            detection
                .checks
                .iter()
                .all(|check| check.what != "wlroots" && check.what != "KDE"),
            "{:?}",
            detection.checks
        );
    }

    #[tokio::test]
    async fn a_hyprland_signature_without_a_runtime_dir_stops_detection_too() {
        let detection = detect_with(|name| match name {
            "HYPRLAND_INSTANCE_SIGNATURE" => Some("sig".into()),
            "WAYLAND_DISPLAY" => Some("/tmp/nowhere/wayland-1".into()),
            _ => None,
        })
        .await;
        assert!(detection.backend.is_none());
        let order: Vec<_> = detection.checks.iter().map(|c| c.what).collect();
        assert_eq!(order, ["Hyprland", "other backends"]);
    }

    #[tokio::test]
    async fn no_wayland_compositor_in_the_environment_is_reported() {
        let detection = detect_with(|_| None).await;
        assert!(detection.backend.is_none());
        let order: Vec<_> = detection.checks.iter().map(|c| c.what).collect();
        assert_eq!(order, ["Hyprland", "KDE", "GNOME", "wlroots", "X11"]);
        for wayland in [&detection.checks[1], &detection.checks[3]] {
            assert!(wayland.found.contains("WAYLAND_DISPLAY"), "{wayland:?}");
        }
        assert!(
            detection.checks[2]
                .found
                .contains("DBUS_SESSION_BUS_ADDRESS"),
            "{:?}",
            detection.checks[2]
        );
    }

    #[tokio::test]
    async fn a_wayland_socket_that_is_not_there_is_reported() {
        let runtime = tempfile::tempdir().expect("tempdir");
        let detection = detect_with(|name| match name {
            "WAYLAND_DISPLAY" => Some("wayland-7".into()),
            "XDG_RUNTIME_DIR" => Some(runtime.path().as_os_str().to_owned()),
            _ => None,
        })
        .await;
        assert!(detection.backend.is_none());
        for wayland in [&detection.checks[1], &detection.checks[3]] {
            assert!(wayland.found.contains("wayland-7"), "{wayland:?}");
        }
        // No DBUS_SESSION_BUS_ADDRESS: the bus in the runtime directory,
        // which is not there.
        assert!(
            detection.checks[2].found.starts_with("the session bus"),
            "{:?}",
            detection.checks[2]
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
