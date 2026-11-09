//! Shared by every sway e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-sway.sh`, `swaymsg` to that harness's sway, where its
//! config is, and a second sway started fresh on that config.

#![allow(
    dead_code,
    reason = "included into each sway e2e test binary, and each uses a subset"
)]

use std::fs::{DirBuilder, File};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_backend::wlr::Wlr;
use dsplr_model::Layout;

/// The harness's runtime directory, after checking that this process really
/// runs under the sway harness (see [`crate::support::harness_runtime`]),
/// and that the sway config `dsplr save` writes beside is the harness's.
pub fn harness_runtime() -> PathBuf {
    let runtime = crate::support::harness_runtime("DSPLR_E2E_SWAY", "tests/e2e/headless-sway.sh");
    assert_eq!(
        config_home(),
        runtime.join("config"),
        "XDG_CONFIG_HOME is not the harness's"
    );
    // Without it, sway's lookup (and dsplr's) would try ~/.sway first: so
    // HOME is the harness's too.
    assert_eq!(
        std::env::var_os("HOME").map(PathBuf::from),
        Some(runtime.join("home")),
        "HOME is not the harness's"
    );
    let config = sway_config();
    assert!(
        config.is_file(),
        "the harness's {} is missing",
        config.display()
    );
    runtime
}

/// `XDG_CONFIG_HOME`, where the harness's sway reads `sway/config`.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .expect("XDG_CONFIG_HOME is set by the harness")
        .into()
}

/// The config the harness's sway reads, which includes `outputs` beside it.
pub fn sway_config() -> PathBuf {
    config_home().join("sway/config")
}

/// The `outputs` file beside [`sway_config`], which `dsplr save` writes.
pub fn outputs_file() -> PathBuf {
    config_home().join("sway/outputs")
}

/// [`outputs_file`], removed if a test before this one in the same harness
/// saved one, so each save test starts without it.
pub fn no_outputs_file() -> PathBuf {
    let outputs = outputs_file();
    match std::fs::remove_file(&outputs) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("removing {}: {error}", outputs.display()),
    }
    outputs
}

/// Runs `swaymsg` with `args` against the harness's sway, and returns what it
/// printed. A command sway refuses fails the test.
pub fn swaymsg(args: &[&str]) -> String {
    let swaysock = std::env::var_os("SWAYSOCK").expect("set by the harness");
    swaymsg_to(Path::new(&swaysock), args)
}

/// Creates headless outputs until the harness's sway has `count`,
/// `HEADLESS-1` (the harness's own) included. Outputs another test binary
/// made in the same harness count too, so each test asks for what it needs
/// rather than adding to whatever is there.
pub async fn outputs_up_to(count: usize) {
    let wlr = Wlr::from_env().await.expect("the harness's sway");
    let have = wlr.snapshot().await.expect("snapshot").entries.len();
    for _ in have..count {
        swaymsg(&["create_output"]);
    }
}

/// [`swaymsg`] to the sway whose IPC socket is `swaysock`, which must be in
/// the harness's runtime directory.
fn swaymsg_to(swaysock: &Path, args: &[&str]) -> String {
    assert!(
        swaysock.starts_with(harness_runtime()),
        "{} is not the harness's",
        swaysock.display()
    );
    let output = Command::new("swaymsg")
        .arg("--socket")
        .arg(swaysock)
        .args(args)
        .output()
        .expect("swaymsg runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "swaymsg {args:?}: {text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    text
}

/// How long sway gets to show a reloaded config: a reload answers before
/// it has applied the outputs' config (on headless sway 1.12 a read right
/// after `swaymsg reload` still shows the config before it).
pub const SETTLE: Duration = Duration::from_secs(5);

/// Reloads sway's config, and takes snapshots until the layout is
/// `expected`, failing the test if it is not within [`SETTLE`].
pub async fn reload_until(backend: &Backend, expected: &Layout) {
    swaymsg(&["reload"]);
    let deadline = Instant::now() + SETTLE;
    loop {
        let layout = backend.snapshot().await.expect("snapshot");
        if layout == *expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "not the expected layout {SETTLE:?} after a reload: {layout:#?}\nexpected {expected:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A second headless sway, started on the harness's config (and so on
/// whatever `outputs` beside it holds) with a runtime directory of its own
/// inside the harness's: its first output there at startup, the others
/// hotplugged. sway applies a config at startup and on hotplug by other
/// rules than on a reload (an identifier's setting wins over a name's
/// wherever each is), so a saved file is checked here as well.
pub struct FreshSway {
    pub wlr: Wlr,
    _sway: Running,
}

/// A sway process, killed when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        // It may have exited already; either way it is reaped.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl FreshSway {
    /// Starts it in `<harness runtime>/<name>`, a directory that must not
    /// exist yet, with `outputs` outputs, named `HEADLESS-1` on as the
    /// harness's are.
    pub async fn start(name: &str, outputs: usize) -> Self {
        let runtime = harness_runtime().join(name);
        DirBuilder::new()
            .mode(0o700)
            .create(&runtime)
            .expect("a runtime directory of its own");
        let log = File::create(runtime.join("sway.log")).expect("its log");
        let sway = Running(
            Command::new("sway")
                .env_clear()
                .env(
                    "HOME",
                    std::env::var_os("HOME").expect("checked by harness_runtime"),
                )
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("XDG_CONFIG_HOME", config_home())
                .env("WLR_BACKENDS", "headless")
                .env("WLR_RENDERER", "pixman")
                .env("WLR_LIBINPUT_NO_DEVICES", "1")
                .stdin(Stdio::null())
                .stdout(log.try_clone().expect("its log"))
                .stderr(log)
                .spawn()
                .expect("sway runs"),
        );
        let deadline = Instant::now() + SETTLE;
        let (wayland, swaysock) = loop {
            let named = |prefix: &str| {
                std::fs::read_dir(&runtime)
                    .expect("its runtime directory")
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.starts_with(prefix) && !name.ends_with(".lock")
                            })
                    })
            };
            if let (Some(wayland), Some(swaysock)) = (named("wayland-"), named("sway-ipc.")) {
                break (wayland, swaysock);
            }
            assert!(
                Instant::now() < deadline,
                "the fresh sway did not start; see {}",
                runtime.join("sway.log").display()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        for _ in 1..outputs {
            swaymsg_to(&swaysock, &["create_output"]);
        }
        let wlr = Wlr::from_vars(|name| match name {
            "WAYLAND_DISPLAY" => Some(wayland.clone().into_os_string()),
            "SWAYSOCK" => Some(swaysock.clone().into_os_string()),
            _ => None,
        })
        .await
        .expect("the fresh sway's output manager");
        assert!(wlr.is_sway());
        Self { wlr, _sway: sway }
    }

    /// Takes snapshots until the layout is `expected`, failing the test if
    /// it is not within [`SETTLE`].
    pub async fn shows(&self, expected: &Layout) {
        let deadline = Instant::now() + SETTLE;
        loop {
            let layout = self.wlr.snapshot().await.expect("snapshot");
            if layout == *expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "a fresh sway does not start at the expected layout: {layout:#?}\nexpected {expected:#?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
