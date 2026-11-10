//! Shared by every KWin e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-kwin.sh`, waiting for that harness's KWin, and the
//! `kwinoutputconfig.json` KWin saves.

#![allow(
    dead_code,
    reason = "included into each KWin e2e test binary, and each uses a subset"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::Layout;

/// How long KWin gets to offer its output management after its socket
/// appears, and to report a change it was asked for.
pub const KWIN_WAIT: Duration = Duration::from_secs(10);

/// The harness's runtime directory, after checking that this process really
/// runs under the KWin harness (see [`crate::support::harness_runtime`]),
/// and that the config KWin saves to is the harness's.
pub fn harness_runtime() -> PathBuf {
    let runtime = crate::support::harness_runtime("DSPLR_E2E_KWIN", "tests/e2e/headless-kwin.sh");
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
    runtime
}

/// `XDG_CONFIG_HOME`, KWin's config location in the harness.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .expect("XDG_CONFIG_HOME is set by the harness")
        .into()
}

/// Where KWin saves the configurations it applies
/// (`OutputConfigurationStore::save` in KWin 6.8's
/// `src/outputconfigurationstore.cpp`).
pub fn kwin_output_config() -> PathBuf {
    config_home().join("kwinoutputconfig.json")
}

/// How many outputs the harness gave KWin.
pub fn harness_outputs() -> usize {
    std::env::var("DSPLR_E2E_KWIN_OUTPUTS")
        .expect("set by the harness")
        .parse()
        .expect("a number")
}

/// The backend detection finds once KWin offers its output management and
/// describes every output the harness asked for: its socket appears before
/// KWin has finished starting.
pub async fn kwin_backend() -> Backend {
    harness_runtime();
    let started = Instant::now();
    loop {
        let detection = dsplr_backend::detect().await;
        if let Some(backend) = detection.backend {
            assert_eq!(backend.name(), "kde", "{:?}", detection.checks);
            if let Ok(layout) = backend.snapshot().await
                && layout.entries.len() == harness_outputs()
            {
                return backend;
            }
        }
        assert!(
            started.elapsed() < KWIN_WAIT,
            "KWin did not offer its {} outputs: {:?}",
            harness_outputs(),
            detection.checks
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until KWin shows `layout` ([`crate::support::shows`]). KWin sends
/// a device's `done` on its next event-loop turn, which can come after it
/// has answered the apply.
pub async fn shows(backend: &Backend, layout: &Layout) {
    crate::support::shows(backend, layout, "KWin", KWIN_WAIT).await;
}
