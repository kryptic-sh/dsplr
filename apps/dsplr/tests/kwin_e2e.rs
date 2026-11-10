//! `dsplr backend`, `dsplr list`, `dsplr apply` (with and without
//! `--dry-run`), `dsplr restore --revert` and `dsplr save` against a real
//! KWin on its virtual backend, which has no test request and saves every
//! layout it applies itself.
//!
//! Run through the harness, which starts that KWin where it cannot reach the
//! desktop session and gives dsplr a private state and config directory:
//!
//! ```text
//! tests/e2e/headless-kwin.sh cargo test -p dsplr --test kwin_e2e -- --ignored --test-threads=1
//! ```
//!
//! The confirm prompt is the same on every backend and is covered by
//! `hyprland_e2e`; here every change is kept with `--yes`. `DSPLR_BIN` names
//! the dsplr to run when it is not where Cargo built it (in CI's VM).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dsplr_model::Layout;
use dsplr_persist::revert;

#[path = "../../../tests/e2e/kwin_support.rs"]
mod kwin_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use kwin_support::{harness_outputs, harness_runtime, kwin_backend, shows};
use support::{connectors, edited, entry, entry_mut, row};

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
#[ignore = "needs the KWin harness: tests/e2e/headless-kwin.sh"]
async fn dsplr_lists_applies_reverts_and_saves_on_kwin() {
    let runtime = harness_runtime();
    let backend = kwin_backend().await;
    let start = row(&backend.snapshot().await.expect("snapshot"));
    backend.apply(&start).await.expect("KWin applies the row");
    shows(&backend, &start).await;
    let names = connectors(&start);
    let last = &names[names.len() - 1];

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("backend: kde\n"), "{text}");
    assert!(text.contains("saves on every apply"), "{text}");
    assert!(text.contains("primary output"), "{text}");
    assert!(text.contains("custom modes"), "{text}");
    assert!(!text.contains("test before apply"), "{text}");
    assert!(!text.contains("gaps allowed"), "{text}");

    let (code, text) = dsplr(&["list", "--json"]);
    assert_eq!(code, Some(0), "{text}");
    let listed = Layout::from_json(&text).expect("dsplr list --json is a layout");
    assert_eq!(listed.entries.len(), harness_outputs());
    assert!(
        start
            .differences(&listed, &backend.capabilities())
            .is_empty()
    );

    // A dry run validates the layout locally, says KWin cannot test it, and
    // changes nothing.
    let moved = edited(&start, |layout| {
        entry_mut(layout, last).state.y = 200;
    });
    let file = layout_file(&runtime, "moved.json", &moved);
    let (code, text) = dsplr(&["apply", "--dry-run", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("validated locally only") && text.contains("no test request"),
        "{text}"
    );
    shows(&backend, &start).await;

    // KWin's protocol allows no gaps, so validation refuses one before KWin
    // is asked.
    let apart = edited(&start, |layout| {
        entry_mut(layout, last).state.x += 100;
    });
    let file_apart = layout_file(&runtime, "apart.json", &apart);
    let (code, text) = dsplr(&["apply", "--dry-run", &file_apart]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("does not touch the other outputs"), "{text}");

    // KWin refuses an enabled output at a negative position; with no test,
    // that is found at the apply, which leaves the layout as it was.
    let negative = edited(&start, |layout| {
        let state = &mut entry_mut(layout, &names[0]).state;
        state.y = -100;
    });
    let file_negative = layout_file(&runtime, "negative.json", &negative);
    let (code, text) = dsplr(&["apply", "--yes", &file_negative]);
    assert_eq!(code, Some(BACKEND_REFUSED), "{text}");
    assert!(
        text.contains("kde refused the layout") && text.contains("negative"),
        "{text}"
    );
    shows(&backend, &start).await;

    // A mode the output does not list, set from the command line: KWin is
    // sent it as a custom mode, then set to the mode it made of it, whose
    // refresh may differ a little from the one asked (KWin makes the
    // timings).
    let (code, text) = dsplr(&["set", last, "--mode", "1280x720@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let now = backend.snapshot().await.expect("snapshot");
    let mode = entry(&now, last).state.mode.clone().expect("on");
    assert_eq!((mode.width, mode.height), (1280, 720), "{text}");
    backend.apply(&start).await.expect("KWin applies it");
    shows(&backend, &start).await;

    // --yes applies and keeps it; a revert by hand puts the old layout back.
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    assert!(
        revert_file.starts_with(&runtime),
        "{}",
        revert_file.display()
    );
    let (code, text) = dsplr(&["apply", "--yes", &file]);
    assert_eq!(code, Some(0), "{text}");
    shows(&backend, &moved).await;
    assert_eq!(
        entry(&backend.snapshot().await.expect("snapshot"), last)
            .state
            .y,
        200
    );
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    shows(&backend, &start).await;
    assert!(!revert_file.exists(), "a revert removes the revert point");

    // Saving is KWin's: it saved the layout as it applied it.
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("already saved, by KWin"), "{text}");
}
