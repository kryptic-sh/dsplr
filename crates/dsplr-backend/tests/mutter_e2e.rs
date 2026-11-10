//! The GNOME backend against a real Mutter on its headless backend:
//! reading, testing and applying layouts, a refusal of Mutter's and two of
//! dsplr's, a stale serial, the primary output, mirroring, following
//! changes, and the `monitors.xml` a `persistent` apply writes.
//!
//! Run through the harness, which starts that Mutter, and a D-Bus session
//! bus for it, where neither can reach the desktop session:
//!
//! ```text
//! tests/e2e/headless-mutter.sh cargo test -p dsplr-backend --test mutter_e2e -- --ignored --test-threads=1
//! ```
//!
//! The tests refuse to run when the harness did not start them, so
//! `--ignored` on its own never changes a real desktop's outputs. One Mutter
//! serves them all, so each starts from the same layout ([`row`]) and they
//! run one at a time.

#![cfg(target_os = "linux")]

use std::collections::HashMap;

use dsplr_backend::gnome::{self, PlanError};
use dsplr_backend::{Backend, Error, Persisted};
use dsplr_model::{Layout, LayoutMode, Mode, OutputId, Transform};

#[path = "../../../tests/e2e/mutter_support.rs"]
mod mutter_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use mutter_support::{
    MUTTER_WAIT, harness_outputs, harness_runtime, has_logical_monitor, monitors_xml,
    mutter_backend, saved, shows,
};
use support::{connectors, edited, entry, entry_mut, row};

async fn snapshot(backend: &Backend) -> Layout {
    backend.snapshot().await.expect("snapshot")
}

/// The backend, with the monitors laid out as [`row`] lays them, and that
/// layout.
async fn from_the_row() -> (Backend, Layout, Vec<String>) {
    let backend = mutter_backend().await;
    let start = row(&snapshot(&backend).await);
    backend.apply(&start).await.expect("Mutter applies the row");
    shows(&backend, &start).await;
    let names = connectors(&start);
    (backend, start, names)
}

/// The mode `connector` is at in `layout`, as width and height.
fn size(layout: &Layout, connector: &str) -> (i32, i32) {
    let mode = entry(layout, connector)
        .state
        .mode
        .as_ref()
        .expect("a mode");
    (
        i32::try_from(mode.width).expect("fits"),
        i32::try_from(mode.height).expect("fits"),
    )
}

/// The connectors of the two monitors of one size the harness made, so one
/// can mirror the other, and the other monitor's.
fn twins(layout: &Layout) -> (String, String, String) {
    let names = connectors(layout);
    let mut by_width: Vec<&String> = names.iter().collect();
    by_width.sort_by_key(|name| size(layout, name).0);
    let [other, a, b] = by_width[..] else {
        panic!("three monitors: {names:?}")
    };
    assert_eq!(size(layout, a), size(layout, b), "{names:?}");
    assert_ne!(size(layout, a), size(layout, other), "{names:?}");
    (a.clone(), b.clone(), other.clone())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn mutter_tests_applies_and_shows_each_layout() {
    let backend = mutter_backend().await;
    let capabilities = backend.capabilities();
    assert!(capabilities.test_before_apply, "verify");
    assert!(!capabilities.gaps_allowed, "logical monitors must touch");
    assert!(capabilities.mirroring && capabilities.primary);
    assert!(capabilities.native_persistence && !capabilities.saves_on_apply);
    assert!(!capabilities.custom_modes);
    assert!(capabilities.adaptive_sync.is_empty());
    assert!(
        capabilities.per_output_scale,
        "Mutter's native backend has no global scale"
    );

    // What the harness asked Mutter for: that many virtual monitors, each on
    // at its one mode, named Meta-N, with a serial, one of them primary.
    let first = snapshot(&backend).await;
    assert_eq!(first.layout_mode, Some(LayoutMode::Logical), "{first:#?}");
    assert_eq!(first.entries.len(), harness_outputs(), "{first:#?}");
    for entry in &first.entries {
        let output = entry.output.as_ref().expect("live");
        assert!(
            output.connector.starts_with("Meta-"),
            "{}",
            output.connector
        );
        assert_eq!(
            (output.make.as_str(), output.model.as_str()),
            ("MetaVendor", "MetaVirtualMonitor"),
            "{output:?}"
        );
        assert!(output.serial.starts_with("0x"), "{output:?}");
        assert!(matches!(entry.id, OutputId::Edid { .. }), "{:?}", entry.id);
        assert_eq!(output.modes.len(), 1, "{output:?}");
        assert_eq!(output.modes[0].refresh_mhz, 60_000, "{output:?}");
        assert!(output.modes[0].scales.contains(&1.0), "{output:?}");
        assert!(entry.state.enabled);
    }
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|entry| entry.state.primary)
            .count(),
        1,
        "{first:#?}"
    );

    let (_, start, names) = from_the_row().await;
    let [one, two, three] = [&names[0], &names[1], &names[2]];

    // Mutter verifies a layout without applying it.
    backend.test(&start).await.expect("Mutter verifies the row");

    // A scale the last monitor's mode lists, other than 1: the last one, so
    // nothing has to move. It goes and comes back as the mode lists it.
    let scales = &entry(&start, three).output.as_ref().expect("live").modes[0].scales;
    let scale = scales
        .iter()
        .copied()
        .find(|&scale| (scale - 2.0).abs() < f64::EPSILON)
        .or_else(|| scales.iter().copied().find(|&scale| scale > 1.0))
        .unwrap_or_else(|| panic!("a scale above 1 in {scales:?}"));
    let scaled = edited(&start, |layout| {
        entry_mut(layout, three).state.scale = scale;
    });
    backend.test(&scaled).await.expect("Mutter verifies it");
    shows(&backend, &start).await;
    backend.apply(&scaled).await.expect("Mutter applies it");
    shows(&backend, &scaled).await;
    let shown = entry(&snapshot(&backend).await, three).state.scale;
    assert!((shown - scale).abs() < f64::EPSILON, "{shown} for {scale}");

    // A quarter turn of the middle monitor: it is as wide as it was high,
    // and the last one moves up to it.
    let (w1, _) = size(&start, one);
    let (_, h2) = size(&start, two);
    let turned = edited(&start, |layout| {
        entry_mut(layout, two).state.transform = Transform::Rotate90;
        entry_mut(layout, three).state.x = w1 + h2;
    });
    backend.apply(&turned).await.expect("Mutter applies it");
    shows(&backend, &turned).await;

    // The primary output.
    let primary = edited(&start, |layout| {
        entry_mut(layout, one).state.primary = false;
        entry_mut(layout, three).state.primary = true;
    });
    backend.apply(&primary).await.expect("Mutter applies it");
    shows(&backend, &primary).await;

    // A monitor turned off stays in the snapshot, off and without a mode.
    let off = edited(&start, |layout| {
        entry_mut(layout, three).state.enabled = false;
    });
    backend.apply(&off).await.expect("Mutter applies it");
    shows(&backend, &off).await;
    let shown = snapshot(&backend).await;
    let three_state = &entry(&shown, three).state;
    assert!(!three_state.enabled);
    assert_eq!(three_state.mode, None);

    // Applying the start puts every setting back, the monitor turned on
    // again included.
    backend.apply(&start).await.expect("Mutter applies it");
    shows(&backend, &start).await;

    // Mutter refuses logical monitors that do not touch, says why, and
    // changes nothing, in its test and its apply alike
    // (`meta_verify_logical_monitor_config_list`, Mutter 51.1).
    let apart = edited(&start, |layout| {
        entry_mut(layout, three).state.x += 100;
    });
    for result in [backend.test(&apart).await, backend.apply(&apart).await] {
        let error = result.expect_err("refused");
        assert!(error.is_refusal(), "{error:?}");
        assert!(!error.is_refused_before_sending(), "{error:?}");
        assert!(error.to_string().contains("adjacent"), "{error}");
    }
    shows(&backend, &start).await;

    // What Mutter would refuse and dsplr can see is refused before Mutter is
    // asked: a layout away from the origin, a mode the monitor does not
    // list.
    let offset = edited(&start, |layout| {
        for entry in &mut layout.entries {
            entry.state.y = 10;
        }
    });
    let unlisted = edited(&start, |layout| {
        entry_mut(layout, one).state.mode = Some(Mode::new(640, 480, 60_000));
    });
    for layout in [&offset, &unlisted] {
        for result in [backend.test(layout).await, backend.apply(layout).await] {
            assert!(
                matches!(result, Err(Error::Gnome(gnome::Error::Plan(_)))),
                "{result:?}"
            );
            assert!(
                result.as_ref().is_err_and(Error::is_refused_before_sending),
                "{result:?}"
            );
        }
    }
    shows(&backend, &start).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn mutter_mirrors_a_monitor_and_ends_the_mirror() {
    let (backend, start, _) = from_the_row().await;
    let (a, b, other) = twins(&start);
    let source = entry(&start, &a).id.clone();
    let (wa, _) = size(&start, &a);

    // The mirror shares its source's logical monitor, so it reads back at
    // the source's place, scale and turn; the other monitor sits beside it.
    let mirrored = edited(&start, |layout| {
        for name in [&a, &other] {
            entry_mut(layout, name).state.primary = false;
        }
        let a_state = &mut entry_mut(layout, &a).state;
        (a_state.x, a_state.y, a_state.primary) = (0, 0, true);
        let b_state = &mut entry_mut(layout, &b).state;
        (b_state.x, b_state.y, b_state.primary) = (0, 0, false);
        b_state.mirror_of = Some(source.clone());
        entry_mut(layout, &other).state.x = wa;
    });
    backend.test(&mirrored).await.expect("Mutter verifies it");
    backend.apply(&mirrored).await.expect("Mutter applies it");
    shows(&backend, &mirrored).await;
    assert_eq!(
        entry(&snapshot(&backend).await, &b).state.mirror_of,
        Some(source.clone())
    );

    // Mutter needs every monitor of a logical monitor at one mode size, and
    // dsplr sees that first.
    let unequal = edited(&mirrored, |layout| {
        entry_mut(layout, &other).state.mirror_of = Some(source.clone());
    });
    let result = backend.test(&unequal).await;
    assert!(
        matches!(
            result,
            Err(Error::Gnome(gnome::Error::Plan(
                PlanError::MirrorModeSize { .. }
            )))
        ),
        "{result:?}"
    );

    backend.apply(&start).await.expect("Mutter applies it");
    shows(&backend, &start).await;
    assert_eq!(entry(&snapshot(&backend).await, &b).state.mirror_of, None);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn a_stale_serial_is_dropped_as_stale() {
    harness_runtime();
    let backend = mutter_backend().await;
    let before = snapshot(&backend).await;

    // The request a change between dsplr's read and its apply makes: one
    // whose serial Mutter does not have. Sent by hand, since dsplr reads
    // and applies on one connection with nothing in between.
    let error = Error::from(gnome::Error::from_apply(stale_answer().await));
    assert!(error.is_stale(), "{error:?}");
    assert!(!error.is_refusal(), "{error:?}");
    shows(&backend, &before).await;
}

/// What Mutter answers an `ApplyMonitorsConfig` whose serial is one past its
/// current one, sent on the harness's bus with no logical monitors, which
/// Mutter does not look at once the serial is wrong
/// (`meta_monitor_manager_handle_apply_monitors_config`, Mutter 51.1).
async fn stale_answer() -> zbus::Error {
    harness_runtime();
    let connection = zbus::Connection::session()
        .await
        .expect("the harness's bus");
    let name = "org.gnome.Mutter.DisplayConfig";
    let path = "/org/gnome/Mutter/DisplayConfig";
    let reply = connection
        .call_method(Some(name), path, Some(name), "GetCurrentState", &())
        .await
        .expect("Mutter's state");
    let body = reply.body();
    let state: zbus::Structure<'_> = body.deserialize().expect("a reply");
    let serial = u32::try_from(&state.fields()[0]).expect("the serial comes first");
    type MonitorConfig = (String, String, HashMap<String, zbus::OwnedVariant>);
    type LogicalMonitorConfig = (i32, i32, f64, u32, bool, Vec<MonitorConfig>);
    let none: Vec<LogicalMonitorConfig> = Vec::new();
    let properties: HashMap<String, zbus::OwnedVariant> = HashMap::new();
    connection
        .call_method(
            Some(name),
            path,
            Some(name),
            "ApplyMonitorsConfig",
            &(serial.wrapping_add(1), 1_u32, none, properties),
        )
        .await
        .expect_err("a stale serial is refused")
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn a_watch_follows_changes_made_by_another_client() {
    async fn changed(watch: &mut dsplr_backend::Watch, what: &str) {
        tokio::time::timeout(MUTTER_WAIT, watch.changed())
            .await
            .unwrap_or_else(|_| panic!("no change reported after {what}"))
            .unwrap_or_else(|error| panic!("the watch failed after {what}: {error}"));
    }
    let (backend, start, names) = from_the_row().await;
    let mut watch = backend.watch().await.expect("a watch");

    // Another connection moves a monitor below the first: Mutter says its
    // monitors changed.
    let (_, h1) = size(&start, &names[0]);
    let moved = edited(&start, |layout| {
        let state = &mut entry_mut(layout, &names[2]).state;
        (state.x, state.y) = (0, h1);
    });
    backend.apply(&moved).await.expect("Mutter applies it");
    changed(&mut watch, "a move").await;
    shows(&backend, &moved).await;

    backend.apply(&start).await.expect("Mutter applies it");
    changed(&mut watch, "the row again").await;
    shows(&backend, &start).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Mutter harness: tests/e2e/headless-mutter.sh"]
async fn a_persistent_apply_writes_monitors_xml() {
    let (backend, start, names) = from_the_row().await;
    let (_, h1) = size(&start, &names[0]);
    let below = &names[2];

    // A layout no other test leaves, so a logical monitor of it in the file
    // can only come from this save.
    let kept = edited(&start, |layout| {
        let state = &mut entry_mut(layout, below).state;
        (state.x, state.y) = (0, h1);
    });
    let before = std::fs::read_to_string(monitors_xml()).unwrap_or_default();
    assert!(
        !has_logical_monitor(&before, below, 0, h1),
        "{} has the layout before it is saved:\n{before}",
        monitors_xml().display()
    );
    // An apply alone (`temporary`) saves nothing.
    backend.apply(&kept).await.expect("Mutter applies it");
    shows(&backend, &kept).await;
    let applied = std::fs::read_to_string(monitors_xml()).unwrap_or_default();
    assert!(
        !has_logical_monitor(&applied, below, 0, h1),
        "a temporary apply was saved:\n{applied}"
    );

    // Saving sends it again with `persistent`; Mutter without GNOME Shell
    // keeps it at once and writes it.
    let persisted = backend.persist(&kept, None).await.expect("handed over");
    assert!(
        matches!(persisted, Persisted::WhenKept { .. }),
        "{persisted:?}"
    );
    saved(below, 0, h1).await;
    shows(&backend, &kept).await;

    // Saving applies, so only the layout on screen is saved.
    let result = backend.persist(&start, None).await;
    assert!(
        matches!(result, Err(Error::Unsupported { .. })),
        "{result:?}"
    );

    backend.apply(&start).await.expect("Mutter applies it");
    shows(&backend, &start).await;
}
