//! The Hyprland backend against a real, headless Hyprland.
//!
//! Run through the harness, which starts that Hyprland where it cannot reach
//! the desktop session:
//!
//! ```text
//! tests/e2e/headless-hyprland.sh cargo test -p dsplr-backend --test hyprland_e2e -- --ignored
//! ```
//!
//! The test refuses to run when the harness did not start it, so `--ignored`
//! on its own never changes a real desktop's monitors.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_backend::hyprland::Hyprland;
use dsplr_model::{Layout, Mode, OutputEntry, OutputId, Transform};

/// How long Hyprland gets to show a change it accepted.
const SETTLE: Duration = Duration::from_secs(5);

/// The harness's runtime directory, after checking that this process really
/// runs under the harness and not against a session's Hyprland.
fn harness_runtime() -> PathBuf {
    let runtime = std::env::var("DSPLR_E2E_HYPRLAND").expect(
        "run through tests/e2e/headless-hyprland.sh, which starts a Hyprland that cannot \
         reach the desktop; refusing to touch whatever Hyprland this is",
    );
    assert_eq!(
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        Some(runtime.as_str()),
        "XDG_RUNTIME_DIR is not the harness's private runtime directory"
    );
    assert!(
        !runtime.starts_with("/run/user/"),
        "the harness runtime must be private, not a session's {runtime}"
    );
    PathBuf::from(runtime)
}

/// Sends one request to the harness's Hyprland, as `hyprctl` does, and
/// returns the reply.
fn hyprctl(request: &str) -> String {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").expect("set by the harness");
    let path = harness_runtime()
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    let mut stream = UnixStream::connect(&path).expect("Hyprland's request socket");
    stream.write_all(request.as_bytes()).expect("request sent");
    let mut reply = String::new();
    stream.read_to_string(&mut reply).expect("reply read");
    reply
}

fn entry<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the snapshot: {layout:#?}"))
}

fn entry_mut<'a>(layout: &'a mut Layout, connector: &str) -> &'a mut OutputEntry {
    layout
        .entries
        .iter_mut()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the layout"))
}

/// Takes snapshots until `shown` holds or [`SETTLE`] passes: Hyprland
/// applies monitor rules on its next frame, not when `eval` returns.
async fn snapshot_until(backend: &Backend, what: &str, shown: impl Fn(&Layout) -> bool) -> Layout {
    let deadline = Instant::now() + SETTLE;
    loop {
        let layout = backend.snapshot().await.expect("snapshot");
        if shown(&layout) {
            return layout;
        }
        assert!(
            Instant::now() < deadline,
            "{what} not shown within {SETTLE:?}: {layout:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `layout` with `edit` made to it: what a user does to a snapshot before
/// applying it.
fn edited(layout: &Layout, edit: impl FnOnce(&mut Layout)) -> Layout {
    let mut layout = layout.clone();
    edit(&mut layout);
    layout
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless Hyprland harness: tests/e2e/headless-hyprland.sh"]
async fn a_headless_hyprland_shows_each_applied_layout() {
    harness_runtime();
    for name in ["DSPLR-1", "DSPLR-2"] {
        let reply = hyprctl(&format!("/output create headless {name}"));
        assert_eq!(reply.trim(), "ok", "creating {name}: {reply}");
    }

    let detection = dsplr_backend::detect().await;
    let backend = detection.backend.unwrap_or_else(|| {
        panic!("no backend: {:?}", detection.checks);
    });
    assert_eq!(backend.name(), "hyprland");
    let apply = |layout: Layout| {
        let backend = backend.clone();
        async move { backend.apply(&layout).await.expect("Hyprland accepted it") }
    };

    // Lay the outputs out in a row with a gap, from what Hyprland chose for
    // the new ones. The revert point is this layout.
    // Other outputs may be there too (the CI VM's own display): only these
    // three are laid out, and the rest are applied as they are.
    let first = snapshot_until(&backend, "both new outputs", |layout| {
        ["DSPLR-1", "DSPLR-2"].iter().all(|name| {
            layout
                .entries
                .iter()
                .any(|e| e.output.as_ref().is_some_and(|o| o.connector == *name))
        })
    })
    .await;
    let start = edited(&first, |layout| {
        for (connector, x) in [("WAYLAND-1", 0), ("DSPLR-1", 1280), ("DSPLR-2", 3200)] {
            let state = &mut entry_mut(layout, connector).state;
            (state.x, state.y) = (x, 0);
        }
        entry_mut(layout, "WAYLAND-1").state.mode = Some(Mode::new(1280, 720, 60_000));
        entry_mut(layout, "DSPLR-1").state.mode = Some(Mode::new(1920, 1080, 60_000));
        entry_mut(layout, "DSPLR-2").state.mode = Some(Mode::new(1920, 1080, 60_000));
    });
    apply(start.clone()).await;
    let start = snapshot_until(&backend, "the row", |layout| {
        entry(layout, "DSPLR-2").state.x == 3200
    })
    .await;
    let two = entry(&start, "DSPLR-2");
    assert_eq!(two.id, OutputId::Connector("DSPLR-2".into()), "no serial");
    assert_eq!((two.state.x, two.state.y), (3200, 0), "the gap is kept");
    assert_eq!(two.state.mode, Some(Mode::new(1920, 1080, 60_000)));

    // A fractional scale and a rotation.
    apply(edited(&start, |layout| {
        let state = &mut entry_mut(layout, "DSPLR-1").state;
        state.scale = 1.5;
        state.transform = Transform::Rotate90;
    }))
    .await;
    let layout = snapshot_until(&backend, "scale and transform", |layout| {
        entry(layout, "DSPLR-1").state.transform == Transform::Rotate90
    })
    .await;
    let one = &entry(&layout, "DSPLR-1").state;
    assert!(one.enabled);
    assert_eq!((one.x, one.y), (1280, 0));
    assert!(
        (one.scale - 1.5).abs() < f64::EPSILON,
        "scale {}",
        one.scale
    );
    assert_eq!(one.mode, Some(Mode::new(1920, 1080, 60_000)));

    // Applying the revert point puts every setting back, not just the ones
    // the last apply named.
    apply(start.clone()).await;
    snapshot_until(&backend, "the revert point", |layout| *layout == start).await;

    // Disabled outputs stay in the snapshot, disabled, without a mode.
    apply(edited(&start, |layout| {
        entry_mut(layout, "DSPLR-2").state.enabled = false;
    }))
    .await;
    let layout = snapshot_until(&backend, "DSPLR-2 disabled", |layout| {
        !entry(layout, "DSPLR-2").state.enabled
    })
    .await;
    assert_eq!(entry(&layout, "DSPLR-2").state.mode, None);

    // And come back as they were.
    apply(start.clone()).await;
    snapshot_until(&backend, "DSPLR-2 back", |layout| *layout == start).await;

    // A mirror is reported by the source monitor's id, and resolves to the
    // source's entry.
    let wayland_1 = entry(&start, "WAYLAND-1").id.clone();
    apply(edited(&start, |layout| {
        entry_mut(layout, "DSPLR-1").state.mirror_of = Some(wayland_1.clone());
    }))
    .await;
    let layout = snapshot_until(&backend, "DSPLR-1 mirroring", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.is_some()
    })
    .await;
    assert_eq!(
        entry(&layout, "DSPLR-1").state.mirror_of.as_ref(),
        Some(&wayland_1)
    );

    // A layout Hyprland rejects is an error, not a silent no-op.
    let hyprland = Hyprland::from_env().expect("the harness's Hyprland");
    assert!(
        hyprland
            .eval(r#"hl.monitor({ output = "DSPLR-1", mode = "nonsense" })"#)
            .await
            .is_err()
    );

    // And the revert point ends the mirror.
    apply(start.clone()).await;
    snapshot_until(&backend, "DSPLR-1 not mirroring", |layout| *layout == start).await;
}
