//! The GUI's Apply against a real, headless sway: a change kept with Enter in
//! the confirm dialogs, one left to time out, and one whose dsplr is killed
//! mid-confirm, which its watchdog reverts.
//!
//! Run through the harness, with Mesa's software Vulkan (lavapipe) and
//! `wtype` installed:
//!
//! ```text
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
//!   tests/e2e/headless-sway.sh cargo test -p dsplr --test gui_e2e -- --ignored
//! ```
//!
//! Keys go in through `wtype`, which needs one keyboard held for the whole
//! session (the backlog's harness entry), and are sent as one argument each,
//! since `wtype` types a space between its arguments. Not in CI's matrix:
//! its VM has neither lavapipe nor `wtype` yet.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use dsplr_model::Layout;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::entry;
use sway_support::{config_home, harness_runtime};

/// The confirm timeout the test configures: short, so the timed-out run and
/// the watchdog's wait stay short.
const CONFIRM_SECS: u64 = 5;
/// How long a step may take to show: a GUI starting on lavapipe, an apply
/// settling.
const STEP: Duration = Duration::from_secs(20);
/// How long the watchdog may take, killed dsplr and all:
/// `restore::watchdog_delay` for wlr's request timeout, with room.
const WATCHDOG: Duration = Duration::from_secs(90);

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// A process ended when the test ends, passing or not.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        // Already gone is fine: the test may have ended it.
        if self.0.kill().is_ok() {
            self.0.wait().expect("waiting for a killed process");
        }
    }
}

/// Holds a virtual keyboard for the session, so each `wtype` call's keys
/// reach the window (the backlog's harness entry).
fn hold_keyboard() -> Running {
    Running(
        Command::new("wtype")
            .args(["-s", "3600000", "-m", "shift"])
            .spawn()
            .expect("wtype is installed"),
    )
}

/// Types `keys`, one `wtype` argument: a character string, or `-k NAME`.
fn wtype(keys: &[&str]) {
    let status = Command::new("wtype")
        .args(keys)
        .status()
        .expect("wtype runs");
    assert!(status.success(), "wtype {keys:?}");
}

fn start_gui() -> Running {
    Running(
        Command::new(dsplr_bin())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("dsplr starts"),
    )
}

/// HEADLESS-1's scale, as `dsplr list --json` reads it.
fn scale() -> f64 {
    let output = Command::new(dsplr_bin())
        .args(["list", "--json"])
        .output()
        .expect("dsplr list runs");
    assert!(output.status.success(), "{output:?}");
    let layout = Layout::from_json(&String::from_utf8_lossy(&output.stdout)).expect("a layout");
    entry(&layout, "HEADLESS-1").state.scale
}

/// Waits up to `limit` for HEADLESS-1's scale to be `want`.
fn until_scale(want: f64, limit: Duration, what: &str) {
    let deadline = Instant::now() + limit;
    loop {
        let now = scale();
        if (now - want).abs() < 1e-6 {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: scale {now}, not {want}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// How long the GUI takes to open on lavapipe and take the keys: keys
/// sent before that are lost.
const STARTUP: Duration = Duration::from_secs(4);

/// Starts the GUI, steps HEADLESS-1's scale up with `+` and applies it with
/// `a`, and waits for the desktop to show it: the dialogs are up then.
fn apply_a_scale_step(from: f64, to: f64) -> Running {
    until_scale(from, STEP, "before");
    let gui = start_gui();
    std::thread::sleep(STARTUP);
    wtype(&["+"]);
    wtype(&["a"]);
    until_scale(to, STEP, "the apply");
    gui
}

#[test]
#[ignore = "needs the headless sway harness, lavapipe and wtype: tests/e2e/headless-sway.sh"]
fn the_gui_keeps_on_enter_and_reverts_on_timeout_and_after_a_kill() {
    harness_runtime();
    let config = config_home().join("dsplr");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.toml"),
        format!("[confirm]\ntimeout_secs = {CONFIRM_SECS}\n"),
    )
    .unwrap();
    let _keyboard = hold_keyboard();

    // Kept: Enter in the dialog.
    let mut gui = apply_a_scale_step(1.0, 1.25);
    wtype(&["-k", "Return"]);
    std::thread::sleep(Duration::from_secs(CONFIRM_SECS + 2));
    until_scale(1.25, STEP, "kept");
    wtype(&["q"]);
    let status = gui.0.wait().unwrap();
    assert!(status.success(), "quit after keeping: {status}");

    // Not answered: the dialog's countdown reverts it.
    let gui = apply_a_scale_step(1.25, 1.5);
    until_scale(1.25, Duration::from_secs(CONFIRM_SECS) + STEP, "timed out");
    drop(gui);

    // dsplr killed mid-confirm: the detached watchdog reverts it.
    let gui = apply_a_scale_step(1.25, 1.5);
    drop(gui);
    until_scale(1.25, WATCHDOG, "the watchdog");
    let revert_point =
        PathBuf::from(std::env::var("XDG_STATE_HOME").unwrap()).join("dsplr/revert.json");
    assert!(
        !revert_point.exists(),
        "the watchdog removes the revert point"
    );
}
