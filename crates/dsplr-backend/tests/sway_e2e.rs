//! The wlroots backend against a real, headless sway: reading, testing and
//! applying layouts, and saving one to the `outputs` file sway reads back.
//!
//! Run through the harness, which starts that sway where it cannot reach
//! the desktop session:
//!
//! ```text
//! tests/e2e/headless-sway.sh cargo test -p dsplr-backend --test sway_e2e -- --ignored
//! ```
//!
//! The test refuses to run when the harness did not start it, so `--ignored`
//! on its own never changes a real desktop's outputs.

#![cfg(target_os = "linux")]

use dsplr_backend::wlr::{self, PlanError, Wlr};
use dsplr_backend::{Backend, Error, Persisted};
use dsplr_model::{AdaptiveSync, Layout, Mode, OutputId, ProblemKind, Transform};
use dsplr_persist::managed::{Comment, Ownership, ownership};

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::{edited, entry, entry_mut};
use sway_support::{
    FreshSway, harness_runtime, no_outputs_file, outputs_up_to, reload_until, sway_config, swaymsg,
};

/// A mode sway's `test` refuses: it cannot allocate the buffer (its log says
/// `mmap failed`), while the protocol takes any positive size.
const UNALLOCATABLE: Mode = Mode::new(2_000_000_000, 1, 0);

async fn snapshot(backend: &Backend) -> Layout {
    backend.snapshot().await.expect("snapshot")
}

/// Sway's own view of one output, from `swaymsg -t get_outputs`: an
/// independent reader to check dsplr's against.
fn sway_output(name: &str) -> serde_json::Value {
    let outputs: Vec<serde_json::Value> =
        serde_json::from_str(&swaymsg(&["--raw", "-t", "get_outputs"])).expect("sway's JSON");
    outputs
        .into_iter()
        .find(|output| output["name"] == name)
        .unwrap_or_else(|| panic!("sway lists {name}"))
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless sway harness: tests/e2e/headless-sway.sh"]
async fn a_headless_sway_tests_applies_and_shows_each_layout() {
    harness_runtime();
    // The harness starts HEADLESS-1; this test also needs HEADLESS-2 and
    // HEADLESS-3.
    outputs_up_to(3).await;

    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    assert_eq!(backend.name(), "wlr");
    let capabilities = backend.capabilities();
    assert!(capabilities.test_before_apply);
    assert!(!capabilities.mirroring, "not in the protocol");
    assert_eq!(
        capabilities.adaptive_sync,
        [AdaptiveSync::Off, AdaptiveSync::On],
        "sway offers version 4"
    );

    // Lay the outputs out in a row with a gap. The revert point is this
    // layout, and each apply below is read back against it.
    let first = snapshot(&backend).await;
    assert_eq!(first.entries.len(), 3, "{first:#?}");
    let start = edited(&first, |layout| {
        for (connector, x) in [
            ("HEADLESS-1", 0),
            ("HEADLESS-2", 1280),
            ("HEADLESS-3", 3300),
        ] {
            let state = &mut entry_mut(layout, connector).state;
            (state.x, state.y) = (x, 0);
        }
    });
    backend.test(&start).await.expect("sway's test passes it");
    backend.apply(&start).await.expect("sway applies it");
    let shown = snapshot(&backend).await;
    assert_eq!(shown, start, "every setting, mode objects included");
    let three = entry(&shown, "HEADLESS-3");
    assert_eq!(
        three.id,
        OutputId::Connector("HEADLESS-3".into()),
        "no serial"
    );
    assert_eq!((three.state.x, three.state.y), (3300, 0), "the gap is kept");

    // A layout sway refuses: its test says so, and so does an apply, and
    // neither changes anything.
    let unallocatable = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-1").state.mode = Some(UNALLOCATABLE);
    });
    for result in [
        backend.test(&unallocatable).await,
        backend.apply(&unallocatable).await,
    ] {
        assert!(
            matches!(result, Err(Error::Wlr(wlr::Error::Failed))),
            "{result:?}"
        );
        assert_eq!(snapshot(&backend).await, start);
    }

    // Mirroring is not in the protocol: validation refuses it, and so does
    // the backend itself, before anything is sent.
    let one = entry(&start, "HEADLESS-1").id.clone();
    let mirrored = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mirror_of = Some(one);
    });
    assert!(
        mirrored
            .validate(&capabilities)
            .iter()
            .any(|problem| problem.kind == ProblemKind::MirrorUnsupported)
    );
    for result in [
        backend.test(&mirrored).await,
        backend.apply(&mirrored).await,
    ] {
        assert!(
            matches!(
                result,
                Err(Error::Wlr(wlr::Error::Plan(PlanError::Unsupported {
                    what: "mirroring",
                    ..
                })))
            ),
            "{result:?}"
        );
        assert!(
            result.as_ref().is_err_and(Error::is_refused_before_sending),
            "dsplr's refusal, not sway's: {result:?}"
        );
    }
    assert_eq!(snapshot(&backend).await, start);

    // A custom mode, a fractional scale, a rotation and a move, in one
    // configuration.
    backend
        .apply(&edited(&start, |layout| {
            let state = &mut entry_mut(layout, "HEADLESS-2").state;
            state.mode = Some(Mode::new(1600, 900, 0));
            state.scale = 1.5;
            state.transform = Transform::Rotate90;
            (state.x, state.y) = (1280, 100);
        }))
        .await
        .expect("sway applies it");
    let shown = snapshot(&backend).await;
    let two = &entry(&shown, "HEADLESS-2").state;
    assert!(two.enabled);
    let mode = two.mode.as_ref().expect("a mode");
    assert_eq!((mode.width, mode.height), (1600, 900));
    assert!(
        (two.scale - 1.5).abs() < f64::EPSILON,
        "scale {}",
        two.scale
    );
    assert_eq!(two.transform, Transform::Rotate90);
    assert_eq!((two.x, two.y), (1280, 100));
    assert_eq!(
        entry(&shown, "HEADLESS-1").state,
        entry(&start, "HEADLESS-1").state
    );
    let sway = sway_output("HEADLESS-2");
    assert_eq!(sway["current_mode"]["width"], 1600, "{sway}");
    assert_eq!(sway["current_mode"]["height"], 900, "{sway}");
    assert_eq!(sway["scale"], 1.5, "{sway}");
    // sway names quarter turns clockwise, wl_output.transform
    // counter-clockwise: wl_output's 90 is sway's "270".
    assert_eq!(sway["transform"], "270", "{sway}");
    assert_eq!(
        (&sway["rect"]["x"], &sway["rect"]["y"]),
        (&1280.into(), &100.into())
    );

    // A scale is sent as a wl_fixed, so 1.3 arrives as its nearest 256th,
    // and reads back as that.
    backend
        .apply(&edited(&start, |layout| {
            entry_mut(layout, "HEADLESS-1").state.scale = 1.3;
        }))
        .await
        .expect("sway applies it");
    let scale = entry(&snapshot(&backend).await, "HEADLESS-1").state.scale;
    assert!(
        (scale - 333.0 / 256.0).abs() < f64::EPSILON,
        "scale {scale}"
    );

    // Applying the revert point puts every setting back, not just the ones
    // the last apply named.
    backend.apply(&start).await.expect("sway applies it");
    assert_eq!(snapshot(&backend).await, start);

    // Disabled outputs stay in the snapshot, disabled, without a mode, and
    // come back as they were.
    backend
        .apply(&edited(&start, |layout| {
            entry_mut(layout, "HEADLESS-3").state.enabled = false;
        }))
        .await
        .expect("sway applies it");
    let shown = snapshot(&backend).await;
    let three = &entry(&shown, "HEADLESS-3").state;
    assert!(!three.enabled);
    assert_eq!(three.mode, None);
    assert_eq!(sway_output("HEADLESS-3")["active"], false);
    backend.apply(&start).await.expect("sway applies it");
    assert_eq!(snapshot(&backend).await, start);

    // A configuration made against heads read before another client moved
    // an output is cancelled, and the other change stands.
    let wlr = Wlr::from_env().await.expect("the harness's sway");
    let mut stale = wlr.connect().await.expect("connected");
    swaymsg(&["output", "HEADLESS-1", "position", "0", "500"]);
    let error = Error::from(stale.apply(&start).await.expect_err("cancelled"));
    assert!(error.is_stale(), "{error:?}");
    drop(stale);
    let shown = snapshot(&backend).await;
    assert_eq!(entry(&shown, "HEADLESS-1").state.y, 500, "not overwritten");

    backend.apply(&start).await.expect("sway applies it");
    assert_eq!(snapshot(&backend).await, start);

    // Last, as a reload puts back whatever the saved file says.
    a_saved_layout_is_what_sway_reads_back_after_a_reload(&backend, &start).await;
}

/// Saves a layout, changes the outputs at runtime only, and reloads: sway
/// shows the saved layout again, so it came from the file. Once for every
/// transform, since sway's names for the quarter turns are not wl_output's.
async fn a_saved_layout_is_what_sway_reads_back_after_a_reload(backend: &Backend, start: &Layout) {
    assert!(backend.capabilities().native_persistence, "SWAYSOCK is set");
    assert!(backend.native_persistence(), "and it answers as sway's");
    let outputs = no_outputs_file();
    let transforms = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];
    let mut saved = start.clone();
    for (round, transform) in transforms.into_iter().enumerate() {
        backend
            .apply(&edited(start, |layout| {
                let state = &mut entry_mut(layout, "HEADLESS-2").state;
                state.mode = Some(Mode::new(1600, 900, 0));
                state.scale = 1.5;
                state.transform = transform;
                (state.x, state.y) = (1280, 100);
                entry_mut(layout, "HEADLESS-3").state.enabled = false;
            }))
            .await
            .expect("sway applies it");
        saved = snapshot(backend).await;
        assert_eq!(entry(&saved, "HEADLESS-2").state.transform, transform);
        let Persisted::File(file) = backend.persist(&saved, None).expect("saved");
        assert_eq!(file.path, outputs);
        assert_eq!(file.main_config, sway_config());
        assert_eq!(file.created, round == 0, "{transform}");
        assert!(file.included, "the harness's config includes it");
        assert!(!file.overridden);
        assert_eq!(
            ownership(
                &std::fs::read_to_string(&outputs).expect("written"),
                Comment::Hash
            ),
            Ownership::Managed
        );

        backend.apply(start).await.expect("sway applies it");
        assert_eq!(snapshot(backend).await, *start, "changed at runtime");
        reload_until(backend, &saved).await;
    }

    // A sway started on the saved file shows it too: its first output at
    // startup, the others hotplugged, where an identifier's setting would
    // beat a name's.
    FreshSway::start("fresh-backend", saved.entries.len())
        .await
        .shows(&saved)
        .await;

    // A mirror is refused before anything is written.
    let text = std::fs::read_to_string(&outputs).expect("written");
    let one = entry(&saved, "HEADLESS-1").id.clone();
    let mirrored = edited(&saved, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mirror_of = Some(one);
    });
    let result = backend.persist(&mirrored, None);
    assert!(
        matches!(
            result,
            Err(Error::Unsupported {
                what: "a mirror",
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(&outputs).expect("kept"), text);

    // Edited by hand, it is replaced only once its text is approved.
    let edited_text = format!("{text}# mine\n");
    std::fs::write(&outputs, &edited_text).expect("edited");
    let conflict = match backend.persist(&saved, None) {
        Err(Error::Conflict(conflict)) => conflict,
        other => panic!("not a conflict: {other:?}"),
    };
    assert_eq!(conflict.ownership, Ownership::EditedByHand);
    assert_eq!(
        std::fs::read_to_string(&outputs).expect("kept"),
        edited_text
    );
    backend
        .persist(&saved, Some(&conflict.existing))
        .expect("saved once approved");
    assert_eq!(std::fs::read_to_string(&outputs).expect("written"), text);
}
