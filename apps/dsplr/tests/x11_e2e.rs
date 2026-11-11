//! `dsplr backend`, `dsplr list` and a dry run of `dsplr set` against a
//! real X server, the harness's Xvfb.
//!
//! Run through the harness, which starts that Xvfb on a display of its own
//! and gives dsplr a private state and config directory:
//!
//! ```text
//! tests/e2e/headless-x11.sh cargo test -p dsplr --test x11_e2e -- --ignored
//! ```
//!
//! `DSPLR_BIN` names the dsplr to run when it is not where Cargo built it
//! (in CI's VM). The RandR backend reads and plans and does not apply yet,
//! so nothing here changes the screen: an apply is refused with nothing
//! sent, and leaves no revert point or watchdog behind.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use dsplr_model::{Layout, OutputId};

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/x11_support.rs"]
mod x11_support;

use support::entry;
use x11_support::{harness_runtime, watchdogs};

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// Runs dsplr with `args` and no terminal, and returns its exit code and
/// everything it printed.
fn dsplr(args: &[&str]) -> (Option<i32>, String) {
    exit_and_output(
        &Command::new(dsplr_bin())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("dsplr runs"),
    )
}

/// The exit code of a process that ran, and everything it printed.
fn exit_and_output(output: &Output) -> (Option<i32>, String) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

#[test]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
fn the_cli_finds_and_lists_xvfb() {
    let runtime = harness_runtime();

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    for said in [
        "X11: RandR 1.",
        "backend: randr",
        "custom modes, mirroring",
        "starts at the origin",
    ] {
        assert!(text.contains(said), "{said:?} not in:\n{text}");
    }
    // Asked for by either name.
    for name in ["randr", "x11"] {
        let (code, text) = dsplr(&["--backend", name, "backend"]);
        assert_eq!(code, Some(0), "{name}: {text}");
        assert!(text.contains("backend: randr"), "{name}: {text}");
    }

    let (code, text) = dsplr(&["list"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.starts_with("screen: screen\n  1920x1080"), "{text}");

    let (code, json) = dsplr(&["list", "--json"]);
    assert_eq!(code, Some(0), "{json}");
    let layout = Layout::from_json(&json).expect("dsplr list --json reads back");
    let screen = entry(&layout, "screen");
    assert_eq!(screen.id, OutputId::Connector("screen".into()));
    assert_eq!(
        screen
            .state
            .mode
            .as_ref()
            .map(|mode| (mode.width, mode.height)),
        Some((1920, 1080))
    );

    // Planned and validated, not tested: RandR has no test request.
    let (code, text) = dsplr(&["set", "screen", "--pos", "0,0", "--dry-run"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("randr cannot test a layout: RandR has no test request"),
        "{text}"
    );

    // A change that plans (a CVT mode made for it) but cannot be applied
    // yet: refused (exit 3), nothing sent, so neither its revert point nor
    // its watchdog is left behind.
    let revert = dsplr_persist::revert::default_path().expect("XDG_STATE_HOME is set");
    assert!(revert.starts_with(&runtime), "{}", revert.display());
    let (code, text) = dsplr(&["set", "screen", "--mode", "1280x720@60", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(
        text.contains("randr cannot apply a layout, so nothing was applied"),
        "{text}"
    );
    assert!(!text.contains("revert point"), "{text}");
    assert!(!revert.exists(), "{} left behind", revert.display());
    assert_eq!(watchdogs(&runtime), Vec::<String>::new());

    // A revert point left from before (another desktop's, say) cannot be
    // gone back to either: refused, with no hint that trying again would
    // help, and kept.
    let point = dsplr_persist::revert::RevertPoint::new(layout);
    dsplr_persist::revert::save(&revert, &point).expect("revert point written");
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(
        text.contains("cannot go back to the revert point") && !text.contains("tries again"),
        "{text}"
    );
    std::fs::remove_file(&revert).expect("revert point removed");
}

/// The exit code of a layout refused with nothing sent (`Exit::Refused`).
const REFUSED: i32 = 3;
