//! Shared by every sway e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-sway.sh`, and `swaymsg` to that harness's sway.

use std::path::PathBuf;
use std::process::Command;

/// The harness's runtime directory, after checking that this process really
/// runs under the sway harness (see [`crate::support::harness_runtime`]).
pub fn harness_runtime() -> PathBuf {
    crate::support::harness_runtime("DSPLR_E2E_SWAY", "tests/e2e/headless-sway.sh")
}

/// Runs `swaymsg` with `args` against the harness's sway, and returns what it
/// printed. A command sway refuses fails the test.
pub fn swaymsg(args: &[&str]) -> String {
    let swaysock = std::env::var_os("SWAYSOCK").expect("set by the harness");
    assert!(
        PathBuf::from(&swaysock).starts_with(harness_runtime()),
        "SWAYSOCK is not the harness's"
    );
    let output = Command::new("swaymsg")
        .arg("--socket")
        .arg(&swaysock)
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
