//! Shared by every niri e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-niri.sh`, where niri's config and `outputs.kdl` are,
//! waiting for niri to show a layout, and a second niri started fresh on
//! that config.

#![allow(
    dead_code,
    reason = "included into each niri e2e test binary, and each uses a subset"
)]

use std::fs::{DirBuilder, File};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use dsplr_backend::wlr::{Compositor, Wlr};
use dsplr_model::{Layout, OutputState};

/// The harness's runtime directory, after checking that this process really
/// runs under the niri harness (see [`crate::support::harness_runtime`]),
/// and that the niri config `dsplr save` writes beside is the harness's.
pub fn harness_runtime() -> PathBuf {
    let runtime = crate::support::harness_runtime("DSPLR_E2E_NIRI", "tests/e2e/headless-niri.sh");
    assert_eq!(
        config_home(),
        runtime.join("config"),
        "XDG_CONFIG_HOME is not the harness's"
    );
    assert_eq!(
        std::env::var_os("HOME").map(PathBuf::from),
        Some(runtime.join("home")),
        "HOME is not the harness's"
    );
    assert!(
        std::env::var_os("NIRI_CONFIG").is_none(),
        "NIRI_CONFIG would point dsplr past the harness's config"
    );
    let socket = PathBuf::from(std::env::var_os("NIRI_SOCKET").expect("set by the harness"));
    assert!(
        socket.starts_with(&runtime),
        "NIRI_SOCKET is not the harness's"
    );
    let config = niri_config();
    assert!(
        config.is_file(),
        "the harness's {} is missing",
        config.display()
    );
    runtime
}

/// `XDG_CONFIG_HOME`, where the harness's niri reads `niri/config.kdl`.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .expect("XDG_CONFIG_HOME is set by the harness")
        .into()
}

/// The config the harness's niri reads, which includes `outputs.kdl`
/// beside it.
pub fn niri_config() -> PathBuf {
    config_home().join("niri/config.kdl")
}

/// The `outputs.kdl` beside [`niri_config`], which `dsplr save` writes.
pub fn outputs_file() -> PathBuf {
    config_home().join("niri/outputs.kdl")
}

/// [`outputs_file`], removed if a test before this one in the same harness
/// saved one, so each save test starts without it. niri reloads when it
/// goes, so its output goes back to niri's defaults.
pub fn no_outputs_file() -> PathBuf {
    let outputs = outputs_file();
    match std::fs::remove_file(&outputs) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("removing {}: {error}", outputs.display()),
    }
    outputs
}

/// How long niri gets to show a config file it reloads. It watches the
/// files and reloads within a second or so (seen on niri 26.04).
pub const SETTLE: Duration = Duration::from_secs(5);

/// The settings of a nested niri's output that niri takes from a layout or
/// a config: a winit output ignores a mode, a transform and adaptive sync.
pub fn placed(state: &OutputState) -> (f64, i32, i32) {
    (state.scale, state.x, state.y)
}

/// Takes snapshots of `wlr` until `check` holds for the layout, failing
/// the test with `what` if it does not within [`SETTLE`].
pub async fn until(wlr: &Wlr, what: &str, check: impl Fn(&Layout) -> bool) -> Layout {
    let deadline = Instant::now() + SETTLE;
    loop {
        let layout = wlr.snapshot().await.expect("snapshot");
        if check(&layout) {
            return layout;
        }
        assert!(
            Instant::now() < deadline,
            "{what} not shown within {SETTLE:?}: {layout:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A niri process, killed when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        // It may have exited already; either way it is reaped.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A second niri, started on the harness's config (and so on whatever
/// `outputs.kdl` beside it holds) in the harness's headless sway, with a
/// directory of its own inside the harness's runtime for its log: what a
/// niri starting on the saved file shows.
pub struct FreshNiri {
    pub wlr: Wlr,
    _niri: Running,
}

impl FreshNiri {
    /// Starts it; `name` names its directory, which must not exist yet.
    pub async fn start(name: &str) -> Self {
        let runtime = harness_runtime();
        let own = runtime.join(name);
        DirBuilder::new()
            .mode(0o700)
            .create(&own)
            .expect("a directory of its own");
        let log = File::create(own.join("niri.log")).expect("its log");
        let env_file = own.join("niri.env");
        let niri = Running(
            Command::new("niri")
                .arg("--")
                .arg("sh")
                .arg("-c")
                .arg(r#"printf "%s\n%s\n" "$WAYLAND_DISPLAY" "$NIRI_SOCKET" > "$1.tmp" && mv "$1.tmp" "$1""#)
                .arg("sh")
                .arg(&env_file)
                .env_clear()
                .env(
                    "HOME",
                    std::env::var_os("HOME").expect("checked by harness_runtime"),
                )
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("XDG_CONFIG_HOME", config_home())
                .env(
                    "WAYLAND_DISPLAY",
                    std::env::var_os("DSPLR_E2E_NIRI_PARENT").expect("set by the harness"),
                )
                .env("LIBGL_ALWAYS_SOFTWARE", "1")
                .stdin(Stdio::null())
                .stdout(log.try_clone().expect("its log"))
                .stderr(log)
                .spawn()
                .expect("niri runs"),
        );
        let deadline = Instant::now() + SETTLE;
        let (display, socket) = loop {
            if let Ok(text) = std::fs::read_to_string(&env_file)
                && let Some((display, socket)) = text.trim_end().split_once('\n')
            {
                break (display.to_owned(), PathBuf::from(socket));
            }
            assert!(
                Instant::now() < deadline,
                "the fresh niri did not start; see {}",
                own.join("niri.log").display()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert!(socket.starts_with(&runtime), "{}", socket.display());
        let wlr = Wlr::from_vars(|name| match name {
            "WAYLAND_DISPLAY" => Some(display.clone().into()),
            "XDG_RUNTIME_DIR" => Some(runtime.clone().into_os_string()),
            "NIRI_SOCKET" => Some(socket.clone().into_os_string()),
            _ => None,
        })
        .await
        .expect("the fresh niri's output manager");
        assert_eq!(wlr.compositor().await, Compositor::Niri);
        Self { wlr, _niri: niri }
    }
}

/// `niri validate` on `config`: whether niri parses it, and what it says.
pub fn validate(config: &Path) -> (bool, String) {
    let output = Command::new("niri")
        .arg("validate")
        .arg("--config")
        .arg(config)
        .env_remove("NIRI_CONFIG")
        .output()
        .expect("niri validate runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
