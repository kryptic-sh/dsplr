//! Shared by every Mutter e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-mutter.sh` and on its bus, waiting for that harness's
//! Mutter, and the `monitors.xml` Mutter saves.

#![allow(
    dead_code,
    reason = "included into each Mutter e2e test binary, and each uses a subset"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::Layout;

/// How long Mutter gets to own its bus name after its Wayland socket
/// appears, to report a change it was asked for, and to write
/// `monitors.xml`, which it does asynchronously.
pub const MUTTER_WAIT: Duration = Duration::from_secs(10);

/// The harness's runtime directory, after checking that this process really
/// runs under the Mutter harness (see [`crate::support::harness_runtime`]),
/// that the config Mutter saves to is the harness's, and that the session
/// bus is the harness's own, not the desktop session's.
pub fn harness_runtime() -> PathBuf {
    let runtime =
        crate::support::harness_runtime("DSPLR_E2E_MUTTER", "tests/e2e/headless-mutter.sh");
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
    let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("the harness starts a bus");
    assert_eq!(
        std::env::var("DSPLR_E2E_MUTTER_BUS").ok().as_deref(),
        Some(bus.as_str()),
        "DBUS_SESSION_BUS_ADDRESS is not the bus the harness started"
    );
    assert!(
        !bus.contains("/run/user/"),
        "the session bus must be the harness's, not a session's: {bus}"
    );
    runtime
}

/// `XDG_CONFIG_HOME`, where Mutter writes `monitors.xml` in the harness.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .expect("XDG_CONFIG_HOME is set by the harness")
        .into()
}

/// Where Mutter saves the configurations a `persistent` apply keeps
/// (`g_get_user_config_dir()/monitors.xml`, Mutter 51.1's
/// `src/backends/meta-monitor-config-store.c`).
pub fn monitors_xml() -> PathBuf {
    config_home().join("monitors.xml")
}

/// How many monitors the harness gave Mutter.
pub fn harness_outputs() -> usize {
    std::env::var("DSPLR_E2E_MUTTER_OUTPUTS")
        .expect("set by the harness")
        .parse()
        .expect("a number")
}

/// The backend detection finds once Mutter owns its name on the bus and
/// describes every monitor the harness asked for: its Wayland socket
/// appears before it has finished starting.
pub async fn mutter_backend() -> Backend {
    harness_runtime();
    let started = Instant::now();
    loop {
        let detection = dsplr_backend::detect().await;
        if let Some(backend) = detection.backend {
            assert_eq!(backend.name(), "gnome", "{:?}", detection.checks);
            if let Ok(layout) = backend.snapshot().await
                && layout.entries.len() == harness_outputs()
                && layout.entries.iter().all(|entry| entry.state.enabled)
            {
                return backend;
            }
        }
        assert!(
            started.elapsed() < MUTTER_WAIT,
            "Mutter did not describe its {} monitors: {:?}",
            harness_outputs(),
            detection.checks
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until Mutter shows `layout` ([`crate::support::shows`]).
pub async fn shows(backend: &Backend, layout: &Layout) {
    crate::support::shows(backend, layout, "Mutter", MUTTER_WAIT).await;
}

/// Waits until `monitors.xml` has a logical monitor at `x`, `y` showing
/// `connector`, and returns the file's text; fails after [`MUTTER_WAIT`].
/// Mutter writes the file asynchronously, after the apply has answered.
pub async fn saved(connector: &str, x: i32, y: i32) -> String {
    let started = Instant::now();
    loop {
        let text = std::fs::read_to_string(monitors_xml()).unwrap_or_default();
        if has_logical_monitor(&text, connector, x, y) {
            return text;
        }
        assert!(
            started.elapsed() < MUTTER_WAIT,
            "{} has no logical monitor at {x},{y} showing {connector}:\n{text}",
            monitors_xml().display()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Whether `monitors.xml`'s text has a logical monitor at `x`, `y` that
/// shows `connector`: Mutter writes each as a `<logicalmonitor>` holding
/// `<x>`, `<y>` and a `<connector>` per monitor, one element to a line
/// (`append_logical_monitor_xml`).
pub fn has_logical_monitor(text: &str, connector: &str, x: i32, y: i32) -> bool {
    text.split("<logicalmonitor>").skip(1).any(|block| {
        let block = block.split("</logicalmonitor>").next().unwrap_or_default();
        block.contains(&format!("<x>{x}</x>"))
            && block.contains(&format!("<y>{y}</y>"))
            && block.contains(&format!("<connector>{connector}</connector>"))
    })
}
