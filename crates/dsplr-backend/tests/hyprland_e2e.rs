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

/// Applies `lua` through `eval` and checks Hyprland accepted it.
fn eval(lua: &str) {
    let reply = hyprctl(&format!("/eval {lua}"));
    assert_eq!(reply.trim(), "ok", "Hyprland refused {lua}: {reply}");
}

fn entry<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the snapshot: {layout:#?}"))
}

/// Takes snapshots until `shown` holds or [`SETTLE`] passes: Hyprland
/// applies monitor rules on its next frame, not when `eval` returns.
async fn snapshot_until(
    hyprland: &Hyprland,
    what: &str,
    shown: impl Fn(&Layout) -> bool,
) -> Layout {
    let deadline = Instant::now() + SETTLE;
    loop {
        let layout = hyprland.snapshot().await.expect("snapshot");
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

/// The whole layout in one `eval`, every field explicit: a rule for an
/// output merges onto that output's previous rule, so a field left out keeps
/// its old value (`hlMonitor`, Hyprland 0.56 and 0.57).
fn layout_lua(dsplr_1: &str, dsplr_2: &str) -> String {
    format!(
        r#"hl.monitor({{ output = "WAYLAND-1", mode = "1280x720@60", position = "0x0", scale = 1, transform = 0, disabled = false, mirror = "" }}); hl.monitor({{ output = "DSPLR-1", mode = "1920x1080@60", position = "1280x0", scale = 1, transform = 0, disabled = false, mirror = "" {dsplr_1} }}); hl.monitor({{ output = "DSPLR-2", mode = "1920x1080@60", position = "3200x0", scale = 1, transform = 0, disabled = false, mirror = "" {dsplr_2} }})"#
    )
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless Hyprland harness: tests/e2e/headless-hyprland.sh"]
async fn a_headless_hyprland_s_monitors_are_read_back_as_set() {
    harness_runtime();
    for name in ["DSPLR-1", "DSPLR-2"] {
        let reply = hyprctl(&format!("/output create headless {name}"));
        assert_eq!(reply.trim(), "ok", "creating {name}: {reply}");
    }
    let hyprland = Hyprland::from_env().expect("the harness's Hyprland");

    let detection = dsplr_backend::detect().await;
    assert_eq!(
        detection.backend.as_ref().map(dsplr_backend::Backend::name),
        Some("hyprland"),
        "{:?}",
        detection.checks
    );

    // Positions, a fractional scale and a rotation.
    eval(&layout_lua(", scale = 1.5, transform = 1", ""));
    let layout = snapshot_until(&hyprland, "scale and transform", |layout| {
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
    let two = entry(&layout, "DSPLR-2");
    assert_eq!(two.id, OutputId::Connector("DSPLR-2".into()), "no serial");
    assert_eq!((two.state.x, two.state.y), (3200, 0));

    // Disabled outputs stay in the snapshot, disabled, without a mode.
    eval(&layout_lua("", ", disabled = true"));
    let layout = snapshot_until(&hyprland, "DSPLR-2 disabled", |layout| {
        !entry(layout, "DSPLR-2").state.enabled
    })
    .await;
    assert_eq!(entry(&layout, "DSPLR-2").state.mode, None);

    // And come back.
    eval(&layout_lua("", ""));
    snapshot_until(&hyprland, "DSPLR-2 enabled again", |layout| {
        entry(layout, "DSPLR-2").state.enabled
    })
    .await;

    // A mirror is reported by the source monitor's id, and resolves to the
    // source's entry.
    eval(&layout_lua(r#", mirror = "WAYLAND-1""#, ""));
    let layout = snapshot_until(&hyprland, "DSPLR-1 mirroring", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.is_some()
    })
    .await;
    assert_eq!(
        entry(&layout, "DSPLR-1").state.mirror_of.as_ref(),
        Some(&entry(&layout, "WAYLAND-1").id)
    );

    // `mirror = ""` ends it.
    eval(&layout_lua("", ""));
    snapshot_until(&hyprland, "DSPLR-1 not mirroring", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.is_none()
    })
    .await;
}
