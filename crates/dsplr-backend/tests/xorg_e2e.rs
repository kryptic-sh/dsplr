//! The RandR backend against an X server with more than one output: the
//! harness's Xorg with the dummy driver, where Xvfb's one CRTC cannot
//! arrange, mirror or plug anything.
//!
//! ```text
//! DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh \
//!   cargo test -p dsplr-backend --test xorg_e2e -- --ignored
//! ```
//!
//! The dummy driver starts with `DUMMY0` on at 2048x1536 and the primary
//! output; the harness gives `DUMMY1` and `DUMMY2` CVT's 1280x720, and xrandr
//! turning one on plugs it in (it reads as connected from then on, off
//! too).
//! What each apply did is checked in xrandr's own `--query` too, not only in
//! dsplr's read-back.

#![cfg(target_os = "linux")]

use std::process::Command;

use dsplr_backend::Backend;
use dsplr_model::Layout;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/x11_support.rs"]
mod x11_support;

use support::{edited, entry, entry_mut};
use x11_support::xorg_runtime;

/// Runs xrandr on the harness's display with `args`; returns what it printed.
fn xrandr(args: &[&str]) -> String {
    let output = Command::new("xrandr")
        .args(args)
        .output()
        .expect("xrandr runs");
    assert!(
        output.status.success(),
        "xrandr {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// xrandr's line for `output` (`DUMMY1 connected primary 1280x720+0+0 …`).
fn line(output: &str) -> String {
    let prefix = format!("{output} ");
    xrandr(&["--query"])
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no {output} in xrandr --query"))
        .to_owned()
}

async fn read(backend: &Backend) -> Layout {
    backend.snapshot().await.expect("snapshot")
}

fn place(layout: &mut Layout, connector: &str, x: i32, y: i32) {
    let state = &mut entry_mut(layout, connector).state;
    (state.x, state.y) = (x, y);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Xorg harness: DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh"]
async fn two_outputs_are_arranged_made_primary_mirrored_and_turned_off() {
    xorg_runtime();
    let backend = dsplr_backend::detect().await.backend.expect("found");
    assert_eq!(backend.name(), "randr");
    let start = read(&backend).await;
    assert_eq!(start.entries.len(), 1, "{start:#?}");
    assert!(entry(&start, "DUMMY0").state.primary);

    // Plugged in, to the right.
    xrandr(&[
        "--output",
        "DUMMY1",
        "--mode",
        "1280x720_60.00",
        "--right-of",
        "DUMMY0",
    ]);
    let plugged = read(&backend).await;
    assert_eq!(plugged.entries.len(), 2, "{plugged:#?}");
    let dummy1 = &entry(&plugged, "DUMMY1").state;
    assert_eq!((dummy1.x, dummy1.y), (2048, 0));
    assert!(!dummy1.primary);

    // Moved to the left of DUMMY0, and made the primary output.
    let swapped = edited(&plugged, |layout| {
        place(layout, "DUMMY1", 0, 0);
        place(layout, "DUMMY0", 1280, 0);
        entry_mut(layout, "DUMMY1").state.primary = true;
        entry_mut(layout, "DUMMY0").state.primary = false;
    });
    backend.apply(&swapped).await.expect("arranged");
    assert!(
        line("DUMMY1").starts_with("DUMMY1 connected primary 1280x720+0+0"),
        "{}",
        line("DUMMY1")
    );
    assert!(
        line("DUMMY0").starts_with("DUMMY0 connected 2048x1536+1280+0"),
        "{}",
        line("DUMMY0")
    );
    let now = read(&backend).await;
    assert!(entry(&now, "DUMMY1").state.primary);
    assert_eq!(entry(&now, "DUMMY0").state.x, 1280);

    // DUMMY0 mirroring DUMMY1, both at DUMMY1's 1280x720 at the origin.
    // DUMMY0 does not list that mode; the planner adds it from the screen's.
    // X has no mirror direction: the read-back names the first of the two
    // the source, and the comparison takes them as one group.
    let dummy1 = entry(&now, "DUMMY1").clone();
    let mirrored = edited(&now, |layout| {
        let state = &mut entry_mut(layout, "DUMMY0").state;
        state.mode.clone_from(&dummy1.state.mode);
        state.mirror_of = Some(dummy1.id.clone());
        (state.x, state.y) = (0, 0);
    });
    backend.apply(&mirrored).await.expect("mirrored");
    assert!(
        line("DUMMY0").contains(" 1280x720+0+0"),
        "{}",
        line("DUMMY0")
    );
    assert!(
        line("DUMMY1").contains(" 1280x720+0+0"),
        "{}",
        line("DUMMY1")
    );
    let now = read(&backend).await;
    assert_eq!(
        mirrored.differences(&now, &backend.capabilities()),
        [],
        "{now:#?}"
    );
    assert!(!backend.capabilities().mirror_direction);

    // Back as plugged in, then DUMMY1 turned off by dsplr: still connected,
    // with no CRTC.
    backend.apply(&plugged).await.expect("back");
    assert!(line("DUMMY0").starts_with("DUMMY0 connected primary 2048x1536+0+0"));
    let off = edited(&plugged, |layout| {
        entry_mut(layout, "DUMMY1").state.enabled = false;
    });
    backend.apply(&off).await.expect("turned off");
    assert_eq!(line("DUMMY1"), "DUMMY1 connected");
    let now = read(&backend).await;
    assert_eq!(
        off.differences(&now, &backend.capabilities()),
        [],
        "{now:#?}"
    );
    assert!(!entry(&now, "DUMMY1").state.enabled);
}
