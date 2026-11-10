//! `dsplr backend`, `dsplr list`, `dsplr apply` (with and without
//! `--dry-run`), `dsplr restore --revert` and `dsplr save` against a real
//! Mutter on its headless backend, which tests a layout (`verify`) and
//! saves `monitors.xml` on a `persistent` apply.
//!
//! Run through the harness, which starts that Mutter and a session bus for
//! it where neither can reach the desktop session, and gives dsplr a private
//! state and config directory:
//!
//! ```text
//! tests/e2e/headless-mutter.sh cargo test -p dsplr --test mutter_e2e -- --ignored --test-threads=1
//! ```
//!
//! The confirm prompt is the same on every backend and is covered by
//! `hyprland_e2e`; here every change is kept with `--yes`. `DSPLR_BIN` names
//! the dsplr to run when it is not where Cargo built it (in CI's VM).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dsplr_model::{Layout, LayoutMode};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/mutter_support.rs"]
mod mutter_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use mutter_support::{harness_outputs, harness_runtime, mutter_backend, saved, shows};
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
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn dsplr_lists_tests_applies_reverts_and_saves_on_mutter() {
    let runtime = harness_runtime();
    let backend = mutter_backend().await;
    let start = row(&backend.snapshot().await.expect("snapshot"));
    backend.apply(&start).await.expect("Mutter applies the row");
    shows(&backend, &start).await;
    let names = connectors(&start);
    let last = &names[names.len() - 1];

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("backend: gnome\n"), "{text}");
    assert!(text.contains("test before apply"), "{text}");
    assert!(text.contains("mirroring"), "{text}");
    assert!(text.contains("primary output"), "{text}");
    assert!(text.contains("native persistence"), "{text}");
    assert!(!text.contains("saves on every apply"), "{text}");
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

    // A dry run has Mutter verify the layout, and changes nothing.
    let first_mode = entry(&start, &names[0])
        .state
        .mode
        .as_ref()
        .expect("a mode");
    let h1 = i32::try_from(first_mode.height).expect("fits");
    let moved = edited(&start, |layout| {
        let state = &mut entry_mut(layout, last).state;
        (state.x, state.y) = (0, h1);
    });
    let file = layout_file(&runtime, "moved.json", &moved);
    let (code, text) = dsplr(&["apply", "--dry-run", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("gnome tested the layout"), "{text}");
    shows(&backend, &start).await;

    // Mutter's logical monitors must touch, so validation refuses a gap
    // before Mutter is asked.
    let apart = edited(&start, |layout| {
        entry_mut(layout, last).state.x += 100;
    });
    let file_apart = layout_file(&runtime, "apart.json", &apart);
    let (code, text) = dsplr(&["apply", "--dry-run", &file_apart]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("does not touch the other outputs"), "{text}");

    // Mutter refuses a fractional scale in its physical layout mode, which
    // dsplr's validation cannot see (the mode's scales are listed for the
    // logical one): its test refuses it, on a dry run and before an apply,
    // and nothing changes.
    let scales = entry(&start, last).output.as_ref().expect("live").modes[0]
        .scales
        .clone();
    let fractional = scales
        .iter()
        .copied()
        .find(|scale| scale.fract() != 0.0)
        .unwrap_or_else(|| panic!("a fractional scale in {scales:?}"));
    let physical = edited(&start, |layout| {
        layout.layout_mode = Some(LayoutMode::Physical);
        entry_mut(layout, last).state.scale = fractional;
    });
    let file_physical = layout_file(&runtime, "physical.json", &physical);
    for args in [
        ["apply", "--dry-run", &file_physical],
        ["apply", "--yes", &file_physical],
    ] {
        let (code, text) = dsplr(&args);
        assert_eq!(code, Some(BACKEND_REFUSED), "{args:?}: {text}");
        assert!(
            text.contains("gnome refused the layout in its test"),
            "{args:?}: {text}"
        );
    }
    shows(&backend, &start).await;

    // Mutter needs the layout at the origin, so a layout away from it is
    // moved there first, saying so: here that is the row as it is.
    let offset = edited(&start, |layout| {
        for entry in &mut layout.entries {
            entry.state.y = 10;
        }
    });
    let file_offset = layout_file(&runtime, "offset.json", &offset);
    let (code, text) = dsplr(&["apply", "--yes", &file_offset]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("moved the layout by 0,-10 so it starts at 0,0"),
        "{text}"
    );
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
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    shows(&backend, &start).await;
    assert!(!revert_file.exists(), "a revert removes the revert point");

    // Saving hands the layout on screen to Mutter, which writes it to
    // monitors.xml (at once, without GNOME Shell to ask).
    let (code, text) = dsplr(&["apply", "--yes", &file]);
    assert_eq!(code, Some(0), "{text}");
    shows(&backend, &moved).await;
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("handed to Mutter"), "{text}");
    saved(last, 0, h1).await;
    backend.apply(&start).await.expect("Mutter applies it");
    shows(&backend, &start).await;

    // Turning off the output at the origin leaves the others away from it;
    // dsplr moves them there, and Mutter takes it.
    let (code, text) = dsplr(&["set", &names[0], "--disable", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("so it starts at 0,0"), "{text}");
    let first_width = i32::try_from(first_mode.width).expect("fits");
    let without_first = edited(&start, |layout| {
        entry_mut(layout, &names[0]).state.enabled = false;
        for name in &names[1..] {
            entry_mut(layout, name).state.x -= first_width;
        }
        // Mutter's primary goes with the output turned off; dsplr keeps
        // none named there, so the first left takes it.
        entry_mut(layout, &names[0]).state.primary = false;
        entry_mut(layout, &names[1]).state.primary = true;
    });
    shows(&backend, &without_first).await;

    backend.apply(&start).await.expect("Mutter applies it");
    shows(&backend, &start).await;
}
