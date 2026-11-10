//! The KDE backend against a real KWin on its virtual backend: reading and
//! applying layouts, a refusal of KWin's and one of dsplr's, the primary
//! output, mirroring, following changes, and the `kwinoutputconfig.json`
//! KWin saves on every apply.
//!
//! Run through the harness, which starts that KWin where it cannot reach the
//! desktop session:
//!
//! ```text
//! tests/e2e/headless-kwin.sh cargo test -p dsplr-backend --test kwin_e2e -- --ignored --test-threads=1
//! ```
//!
//! The tests refuse to run when the harness did not start them, so
//! `--ignored` on its own never changes a real desktop's outputs. One KWin
//! serves them all, so each starts from the same layout ([`row`]) and they
//! run one at a time.

#![cfg(target_os = "linux")]

use dsplr_backend::kde::{self, PlanError};
use dsplr_backend::{Backend, Error, Persisted};
use dsplr_model::{AdaptiveSync, Layout, Mode, Transform};

#[path = "../../../tests/e2e/kwin_support.rs"]
mod kwin_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use kwin_support::{
    KWIN_WAIT, connectors, harness_outputs, kwin_backend, kwin_output_config, row, shows,
};
use support::{edited, entry, entry_mut};

async fn snapshot(backend: &Backend) -> Layout {
    backend.snapshot().await.expect("snapshot")
}

/// The backend, with the outputs laid out as [`row`] lays them, and that
/// layout.
async fn from_the_row() -> (Backend, Layout, Vec<String>) {
    let backend = kwin_backend().await;
    let start = row(&snapshot(&backend).await);
    backend.apply(&start).await.expect("KWin applies the row");
    shows(&backend, &start).await;
    let names = connectors(&start);
    (backend, start, names)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the KWin harness: tests/e2e/headless-kwin.sh"]
async fn kwin_applies_and_shows_each_layout() {
    let backend = kwin_backend().await;
    let capabilities = backend.capabilities();
    assert!(!capabilities.test_before_apply, "no test request");
    assert!(!capabilities.gaps_allowed, "the protocol says so");
    assert!(capabilities.saves_on_apply && capabilities.native_persistence);
    assert!(capabilities.primary, "KWin offers kde_output_order_v1");
    assert!(capabilities.mirroring, "management version 13 or later");
    assert!(!capabilities.custom_modes, "not written for KDE yet");
    assert_eq!(
        capabilities.adaptive_sync,
        [AdaptiveSync::Off, AdaptiveSync::On, AdaptiveSync::Automatic]
    );

    // What the harness asked KWin for: that many outputs, each on, at one
    // 1920x1080 mode, one of them primary.
    let first = snapshot(&backend).await;
    assert_eq!(first.entries.len(), harness_outputs(), "{first:#?}");
    for entry in &first.entries {
        let output = entry.output.as_ref().expect("live");
        assert!(
            output.connector.starts_with("Virtual-"),
            "{}",
            output.connector
        );
        assert_eq!(output.modes.len(), 1, "{output:?}");
        assert!(output.modes[0].matches(&Mode::new(1920, 1080, 60_000)));
        assert!(entry.state.enabled);
        // A virtual output has no VRR, overscan or RGB range capability.
        assert_eq!(entry.state.adaptive_sync, None);
        assert_eq!(entry.state.overscan, None);
        assert_eq!(entry.state.rgb_range, None);
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

    // KWin has no test request: the layout is checked against the outputs
    // and then the test is reported as not supported, not as passed.
    let result = backend.test(&start).await;
    assert!(
        matches!(
            result,
            Err(Error::Unsupported {
                what: "testing a layout",
                ..
            })
        ),
        "{result:?}"
    );

    // A fractional scale, a rotation and a move, in one configuration: the
    // second output is 720 by 1280 then, and the third moves up to it.
    let turned = edited(&start, |layout| {
        let state = &mut entry_mut(layout, two).state;
        state.scale = 1.5;
        state.transform = Transform::Rotate90;
        entry_mut(layout, three).state.x = 1920 + 720;
    });
    backend.apply(&turned).await.expect("KWin applies it");
    shows(&backend, &turned).await;
    let shown = snapshot(&backend).await;
    let state = &entry(&shown, two).state;
    assert!((state.scale - 1.5).abs() < f64::EPSILON, "{}", state.scale);
    assert_eq!(state.transform, Transform::Rotate90);
    assert_eq!(
        (entry(&shown, three).state.x, entry(&shown, three).state.y),
        (2640, 0)
    );

    // 1.1 is neither a 256th nor kept as asked: it goes as 282/256, KWin
    // keeps the nearest 120th (1.1), and sends that back as 282/256. That is
    // what the wire holds, so it shows as no difference.
    let tenth = edited(&start, |layout| {
        entry_mut(layout, two).state.scale = 1.1;
    });
    backend.apply(&tenth).await.expect("KWin applies it");
    shows(&backend, &tenth).await;
    let scale = entry(&snapshot(&backend).await, two).state.scale;
    assert!((scale - 282.0 / 256.0).abs() < f64::EPSILON, "{scale}");

    // The primary output, through the output order.
    let primary = edited(&start, |layout| {
        entry_mut(layout, one).state.primary = false;
        entry_mut(layout, three).state.primary = true;
    });
    backend.apply(&primary).await.expect("KWin applies it");
    shows(&backend, &primary).await;
    let shown = snapshot(&backend).await;
    assert!(entry(&shown, three).state.primary);
    assert!(!entry(&shown, one).state.primary);

    // An output turned off stays in the snapshot, off and without a mode.
    let off = edited(&start, |layout| {
        entry_mut(layout, three).state.enabled = false;
    });
    backend.apply(&off).await.expect("KWin applies it");
    shows(&backend, &off).await;
    let shown = snapshot(&backend).await;
    let three_state = &entry(&shown, three).state;
    assert!(!three_state.enabled);
    assert_eq!(three_state.mode, None);

    // Applying the start puts every setting back, the output turned on again
    // included.
    backend.apply(&start).await.expect("KWin applies it");
    shows(&backend, &start).await;

    // KWin refuses an enabled output at a negative position, says why, and
    // changes nothing (`kde_output_configuration_v2_apply` in KWin 6.8's
    // `src/wayland/outputmanagement_v2.cpp`).
    let negative = edited(&start, |layout| {
        entry_mut(layout, one).state.x = -100;
    });
    let error = backend.apply(&negative).await.expect_err("refused");
    assert!(error.is_refusal(), "{error:?}");
    assert!(!error.is_refused_before_sending(), "{error:?}");
    match &error {
        Error::Kde(kde::Error::Failed {
            reason: Some(reason),
        }) => assert!(reason.contains("negative"), "{reason}"),
        other => panic!("not KWin's refusal with a reason: {other:?}"),
    }
    shows(&backend, &start).await;

    // A mode the output does not list is refused by dsplr before KWin is
    // asked, by the test and the apply alike.
    let custom = edited(&start, |layout| {
        entry_mut(layout, one).state.mode = Some(Mode::new(1280, 720, 60_000));
    });
    for result in [backend.test(&custom).await, backend.apply(&custom).await] {
        assert!(
            matches!(
                result,
                Err(Error::Kde(kde::Error::Plan(PlanError::Unsupported { .. })))
            ),
            "{result:?}"
        );
        assert!(
            result.as_ref().is_err_and(Error::is_refused_before_sending),
            "{result:?}"
        );
    }
    shows(&backend, &start).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the KWin harness: tests/e2e/headless-kwin.sh"]
async fn kwin_mirrors_an_output_and_ends_the_mirror() {
    let (backend, start, names) = from_the_row().await;
    let source = entry(&start, &names[0]).id.clone();
    let mirrored = edited(&start, |layout| {
        entry_mut(layout, &names[2]).state.mirror_of = Some(source.clone());
    });
    backend.apply(&mirrored).await.expect("KWin applies it");
    shows(&backend, &mirrored).await;
    assert_eq!(
        entry(&snapshot(&backend).await, &names[2]).state.mirror_of,
        Some(source)
    );

    backend.apply(&start).await.expect("KWin applies it");
    shows(&backend, &start).await;
    assert_eq!(
        entry(&snapshot(&backend).await, &names[2]).state.mirror_of,
        None
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the KWin harness: tests/e2e/headless-kwin.sh"]
async fn a_watch_follows_changes_made_by_another_client() {
    async fn changed(watch: &mut dsplr_backend::Watch, what: &str) {
        tokio::time::timeout(KWIN_WAIT, watch.changed())
            .await
            .unwrap_or_else(|_| panic!("no change reported after {what}"))
            .unwrap_or_else(|error| panic!("the watch failed after {what}: {error}"));
    }
    let (backend, start, names) = from_the_row().await;
    let mut watch = backend.watch().await.expect("a watch");

    // Another connection moves an output: the watch's devices report it.
    let moved = edited(&start, |layout| {
        entry_mut(layout, &names[2]).state.y = 300;
    });
    backend.apply(&moved).await.expect("KWin applies it");
    changed(&mut watch, "a move").await;
    shows(&backend, &moved).await;

    // The output order alone changing is a change too.
    let primary = edited(&moved, |layout| {
        entry_mut(layout, &names[0]).state.primary = false;
        entry_mut(layout, &names[1]).state.primary = true;
    });
    backend.apply(&primary).await.expect("KWin applies it");
    changed(&mut watch, "a new primary output").await;
    shows(&backend, &primary).await;

    backend.apply(&start).await.expect("KWin applies it");
    changed(&mut watch, "the row again").await;
    shows(&backend, &start).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the KWin harness: tests/e2e/headless-kwin.sh"]
async fn kwin_saves_each_layout_it_applies() {
    let (backend, start, names) = from_the_row().await;
    let config = kwin_output_config();

    // A layout no other test leaves, so the file's setup can only be it.
    let saved = edited(&start, |layout| {
        let state = &mut entry_mut(layout, &names[1]).state;
        state.y = 120;
        state.scale = 1.25;
    });
    backend.apply(&saved).await.expect("KWin applies it");
    shows(&backend, &saved).await;
    // KWin writes it before it answers the apply
    // (`Workspace::applyOutputConfiguration` in KWin 6.8's
    // `src/workspace.cpp` stores the setup, then the protocol says
    // `applied`), the whole store each time.
    let text = std::fs::read_to_string(&config)
        .unwrap_or_else(|error| panic!("{}: {error}", config.display()));
    let json: serde_json::Value = serde_json::from_str(&text).expect("KWin's JSON");
    let positions = |layout: &Layout| {
        let mut positions: Vec<(i64, i64)> = layout
            .entries
            .iter()
            .map(|entry| (i64::from(entry.state.x), i64::from(entry.state.y)))
            .collect();
        positions.sort_unstable();
        positions
    };
    let section = |name: &str| {
        json.as_array()
            .expect("an array of sections")
            .iter()
            .find(|section| section["name"] == name)
            .unwrap_or_else(|| panic!("no {name} section: {json:#}"))["data"]
            .as_array()
            .expect("its data")
            .clone()
    };
    let setups = section("setups");
    assert!(
        setups.iter().any(|setup| {
            let mut saved_positions: Vec<(i64, i64)> = setup["outputs"]
                .as_array()
                .expect("a setup's outputs")
                .iter()
                .map(|output| {
                    let position = &output["position"];
                    (
                        position["x"].as_i64().expect("x"),
                        position["y"].as_i64().expect("y"),
                    )
                })
                .collect();
            saved_positions.sort_unstable();
            saved_positions == positions(&saved)
        }),
        "no setup in {} has the layout's positions: {json:#}",
        config.display()
    );
    assert!(
        section("outputs")
            .iter()
            .any(|output| output["scale"].as_f64() == Some(1.25)),
        "no output in {} has scale 1.25: {json:#}",
        config.display()
    );

    // So saving is KWin's: dsplr writes nothing, and says so, for the layout
    // on screen; any other is refused, as KWin keeps only what it applied.
    let persisted = backend.persist(&saved, None).await.expect("saved");
    assert!(
        matches!(persisted, Persisted::ByDesktop { .. }),
        "{persisted:?}"
    );
    let result = backend.persist(&start, None).await;
    assert!(
        matches!(result, Err(Error::Unsupported { .. })),
        "{result:?}"
    );

    backend.apply(&start).await.expect("KWin applies it");
    shows(&backend, &start).await;
}
