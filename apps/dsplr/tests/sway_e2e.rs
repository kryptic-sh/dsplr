//! `dsplr backend`, `dsplr apply` (with and without `--dry-run`),
//! `dsplr set` and `dsplr restore --revert` against a real, headless sway,
//! whose backend has a real test.
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

use dsplr_backend::Backend;
use dsplr_model::{Layout, Mode, Transform};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::{edited, entry, entry_mut};
use sway_support::{harness_runtime, swaymsg};

/// The exit code of a usage error (`Exit::Usage`, clap's own).
const USAGE: i32 = 2;

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
async fn dsplr_applies_sets_and_reverts_on_sway() {
    let runtime = harness_runtime();
    swaymsg(&["create_output"]);
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    // One sway serves both, so they run in turn rather than as two tests
    // racing over its outputs. Each leaves the layout as it found it.
    apply_tests_on_a_dry_run_and_applies_and_reverts(&backend, &runtime).await;
    set_edits_one_output(&backend).await;
}

async fn apply_tests_on_a_dry_run_and_applies_and_reverts(backend: &Backend, runtime: &Path) {
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
    let file = layout_file(runtime, "moved.json", &moved);
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
    let file_refused = layout_file(runtime, "refused.json", &unallocatable);
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
        revert_file.starts_with(runtime),
        "{}",
        revert_file.display()
    );
    assert!(!revert_file.exists(), "a failed test starts no change");

    // Mirroring is refused by validation, before sway is asked.
    let one = entry(&start, "HEADLESS-1").id.clone();
    let mirrored = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mirror_of = Some(one);
    });
    let file_mirrored = layout_file(runtime, "mirrored.json", &mirrored);
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

/// Puts the layout before the last change back by hand, and checks it is
/// `start` again.
async fn revert_to(backend: &Backend, start: &Layout) {
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(backend.snapshot().await.expect("snapshot"), *start);
}

async fn set_edits_one_output(backend: &Backend) {
    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };
    let two = || entry(&start, "HEADLESS-2");
    let (x, y) = (two().state.x, two().state.y);
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");

    // A name for no output, or for both, is a usage error listing them.
    let (code, text) = dsplr(&["set", "HEADLESS-9", "--pos", "0,0"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("names no output") && text.contains("  HEADLESS-1: "),
        "{text}"
    );
    let one_description = entry(&start, "HEADLESS-1").description().expect("one");
    let two_description = two().description().expect("one");
    let shared: String = one_description
        .chars()
        .zip(two_description.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect();
    assert!(
        !shared.trim().is_empty(),
        "{one_description} {two_description}"
    );
    let (code, text) = dsplr(&["set", &shared, "--pos", "0,0"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(text.contains("names more than one output"), "{text}");

    // Named by its description, on a dry run: sway tests it, nothing changes.
    let moved = (x, y + 200);
    let pos = format!("{},{}", moved.0, moved.1);
    let (code, text) = dsplr(&["set", &two_description, "--pos", &pos, "--dry-run"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("wlr tested the layout"), "{text}");
    // It says which output a description found.
    let setting_two = format!("dsplr: setting HEADLESS-2: {two_description}\n");
    assert!(text.contains(&setting_two), "{text}");
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a dry run starts no change");

    // Moved, and only that.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--pos", &pos, "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains(&setting_two), "{text}");
    let expected = edited(&start, |layout| {
        let state = &mut entry_mut(layout, "HEADLESS-2").state;
        (state.x, state.y) = moved;
    });
    assert_eq!(snapshot().await, expected);
    revert_to(backend, &start).await;

    // A custom mode (headless sway lists only the current one) needs its
    // refresh; with one it is applied.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mode", "1600x900", "--dry-run"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("give the refresh"), "{text}");
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mode", "1600x900@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let mode = entry(&snapshot().await, "HEADLESS-2").state.mode.clone();
    assert!(
        mode.as_ref()
            .is_some_and(|mode| mode.matches(&Mode::new(1600, 900, 60_000))),
        "{mode:?}"
    );
    revert_to(backend, &start).await;

    let (code, text) = dsplr(&["set", "HEADLESS-2", "--scale", "2", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let scale = entry(&snapshot().await, "HEADLESS-2").state.scale;
    assert!((scale - 2.0).abs() < f64::EPSILON, "{scale}");
    revert_to(backend, &start).await;

    // dsplr's 90 is wl_output's, which sway reports as its 270, as --help
    // says.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--transform", "90", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(
        entry(&snapshot().await, "HEADLESS-2").state.transform,
        Transform::Rotate90
    );
    let outputs: String = swaymsg(&["-t", "get_outputs"])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(outputs.contains("\"transform\":\"270\""), "{outputs}");
    revert_to(backend, &start).await;

    // What the desktop cannot do is refused before it is asked.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--vrr", "automatic", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(
        text.contains("cannot set adaptive sync automatic"),
        "{text}"
    );
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mirror", "HEADLESS-1", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("cannot mirror"), "{text}");
    for mirror in ["HEADLESS-9", "HEADLESS-2"] {
        let (code, text) = dsplr(&["set", "HEADLESS-2", "--mirror", mirror, "--yes"]);
        assert_eq!(code, Some(USAGE), "{mirror}: {text}");
    }
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a refusal starts no change");

    // Turned off, and on again where it was. Headless sway lists no mode for
    // an output that is off, so turning it on needs one.
    let (code, text) = dsplr(&["set", "HEADLESS-1", "--disable", "--scale", "2", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("--scale cannot be changed together with --disable"),
        "{text}"
    );
    assert_eq!(snapshot().await, start);
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--disable", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let off = snapshot().await;
    assert!(!entry(&off, "HEADLESS-2").state.enabled);
    // A setting for an output that stays off would be dropped unseen.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--pos", "0,0", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("it is off; add --enable to change --pos"),
        "{text}"
    );
    assert_eq!(snapshot().await, off);
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--enable", "--dry-run"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("enabled without a mode"), "{text}");
    let start_mode = two().state.mode.clone().expect("enabled");
    let mode = format!("{}x{}@0", start_mode.width, start_mode.height);
    let pos = format!("{x},{y}");
    let (code, text) = dsplr(&[
        "set",
        "HEADLESS-2",
        "--enable",
        "--mode",
        &mode,
        "--pos",
        &pos,
        "--yes",
    ]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
}
