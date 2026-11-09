//! `dsplr backend`, `dsplr apply` (with and without `--dry-run`) and
//! `dsplr restore --revert` against a real, headless sway, whose backend has
//! a real test.
//!
//! Run through the harness, which starts that sway where it cannot reach the
//! desktop session and gives dsplr a private state directory:
//!
//! ```text
//! tests/e2e/headless-sway.sh cargo test -p dsplr --test sway_e2e -- --ignored
//! ```
//!
//! The confirm prompt is the same on every backend and is covered by
//! `hyprland_e2e`; here every change is kept with `--yes`. `DSPLR_BIN` names
//! the dsplr to run when it is not where Cargo built it (in CI's VM).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dsplr_model::{Layout, Mode};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::{edited, entry, entry_mut};
use sway_support::{harness_runtime, swaymsg};

/// The exit code of a layout validation refused (`Exit::Refused`).
const REFUSED: i32 = 3;

/// The exit code of a layout the desktop refused (`Exit::BackendRefused`).
const BACKEND_REFUSED: i32 = 4;

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// Runs dsplr with `args` and no terminal, and returns its exit code and
/// everything it printed.
fn dsplr(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(dsplr_bin())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

/// `layout` written as a layout file in the harness's runtime.
fn layout_file(runtime: &Path, name: &str, layout: &Layout) -> String {
    let path = runtime.join(name);
    std::fs::write(&path, layout.to_json().expect("JSON")).expect("layout file written");
    path.to_str().expect("a UTF-8 path").to_owned()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless sway harness: tests/e2e/headless-sway.sh"]
async fn dsplr_tests_on_a_dry_run_and_applies_and_reverts_on_sway() {
    let runtime = harness_runtime();
    swaymsg(&["create_output"]);
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("backend: wlr\n"), "{text}");
    assert!(text.contains("test before apply"), "{text}");
    assert!(!text.contains("mirroring"), "{text}");

    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };

    // A dry run has sway test the layout, and changes nothing.
    let moved = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.y = 200;
    });
    let file = layout_file(&runtime, "moved.json", &moved);
    let (code, text) = dsplr(&["apply", "--dry-run", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("wlr tested the layout"), "{text}");
    assert_eq!(snapshot().await, start);

    // One sway refuses fails its test, on a dry run and before a real apply,
    // and changes nothing. It is the rightmost output, so the wide mode
    // overlaps nothing and passes validation.
    let unallocatable = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mode = Some(Mode::new(2_000_000_000, 1, 0));
    });
    let file_refused = layout_file(&runtime, "refused.json", &unallocatable);
    for args in [
        ["apply", "--dry-run", &file_refused],
        ["apply", "--yes", &file_refused],
    ] {
        let (code, text) = dsplr(&args);
        assert_eq!(code, Some(BACKEND_REFUSED), "{args:?}: {text}");
        // The test refused it, not an apply that was then reverted: the
        // wording is the test step's.
        assert!(
            text.contains("wlr refused the layout in its test")
                && text.contains("refused the configuration"),
            "{args:?}: {text}"
        );
    }
    assert_eq!(snapshot().await, start);
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    assert!(
        revert_file.starts_with(&runtime),
        "{}",
        revert_file.display()
    );
    assert!(!revert_file.exists(), "a failed test starts no change");

    // Mirroring is refused by validation, before sway is asked.
    let one = entry(&start, "HEADLESS-1").id.clone();
    let mirrored = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mirror_of = Some(one);
    });
    let file_mirrored = layout_file(&runtime, "mirrored.json", &mirrored);
    let (code, text) = dsplr(&["apply", "--dry-run", &file_mirrored]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("cannot mirror"), "{text}");

    // --yes applies and keeps it; a revert by hand puts the old layout back.
    let (code, text) = dsplr(&["apply", "--yes", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(entry(&snapshot().await, "HEADLESS-2").state.y, 200);
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a revert removes the revert point");
}
