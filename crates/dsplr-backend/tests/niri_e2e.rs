//! The wlroots backend against a real niri, nested in a headless sway:
//! recognising niri, testing and applying a layout, and saving one to the
//! `outputs.kdl` niri reloads, and starts on.
//!
//! Run through the harness, which starts that niri where it cannot reach
//! the desktop session:
//!
//! ```text
//! tests/e2e/headless-niri.sh cargo test -p dsplr-backend --test niri_e2e -- --ignored
//! ```
//!
//! The test refuses to run when the harness did not start it, so `--ignored`
//! on its own never changes a real desktop's outputs. A nested niri has one
//! output, `winit`, and takes only its scale and position, so those are what
//! the test changes.

#![cfg(target_os = "linux")]

use std::ffi::OsString;

use dsplr_backend::wlr::{Compositor, Wlr};
use dsplr_backend::{Backend, Error, Persisted};
use dsplr_model::Layout;
use dsplr_persist::managed::{Comment, Ownership, ownership};

#[path = "../../../tests/e2e/niri_support.rs"]
mod niri_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use niri_support::{
    FreshNiri, harness_runtime, niri_config, no_outputs_file, placed, until, validate,
};
use support::{edited, entry, entry_mut};

/// The one output of a nested niri.
const WINIT: &str = "winit";

/// `layout` with `winit` at `scale`, at `x`, `y`.
fn placed_at(layout: &Layout, scale: f64, x: i32, y: i32) -> Layout {
    edited(layout, |layout| {
        let state = &mut entry_mut(layout, WINIT).state;
        state.scale = scale;
        (state.x, state.y) = (x, y);
    })
}

/// Whether `layout` shows `winit` at `scale`, at `x`, `y`.
fn shows(layout: &Layout, scale: f64, x: i32, y: i32) -> bool {
    placed(&entry(layout, WINIT).state) == (scale, x, y)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the nested niri harness: tests/e2e/headless-niri.sh"]
async fn a_nested_niri_applies_and_reloads_what_dsplr_saves() {
    harness_runtime();
    no_outputs_file();

    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    let Backend::Wlr(ref wlr) = backend else {
        panic!("not wlr: {}", backend.name());
    };
    assert_eq!(
        wlr.compositor().await,
        Compositor::Niri,
        "NIRI_SOCKET answers"
    );
    assert!(
        backend.capabilities().native_persistence,
        "NIRI_SOCKET is set"
    );
    assert!(
        backend.native_persistence().await,
        "and it answers as niri's"
    );

    // Back to niri's defaults, at runtime: an earlier test in this harness
    // may have moved the output.
    let first = wlr.snapshot().await.expect("snapshot");
    backend
        .apply(&placed_at(&first, 1.0, 0, 0))
        .await
        .expect("niri applies it");
    let start = until(wlr, "niri's defaults", |layout| shows(layout, 1.0, 0, 0)).await;

    // Tested, then applied. niri answers an apply that changes the outputs
    // with `succeeded` and then `cancelled`, the first being its answer.
    let moved = placed_at(&start, 1.5, 100, 50);
    backend.test(&moved).await.expect("niri tests it");
    backend.apply(&moved).await.expect("niri applies it");
    until(wlr, "the applied layout", |layout| {
        shows(layout, 1.5, 100, 50)
    })
    .await;

    a_saved_layout_is_applied_and_read_at_startup(&backend, wlr, &start).await;
    no_outputs_file();
}

/// Saves the layout on screen after applying it: niri reloads
/// `outputs.kdl` as soon as it is written, and a niri started on it starts
/// there. Saving a layout other than the one on screen is refused, since
/// writing the file would apply it past the apply flow. Then a file edited
/// by hand.
async fn a_saved_layout_is_applied_and_read_at_startup(
    backend: &Backend,
    wlr: &Wlr,
    start: &Layout,
) {
    let outputs = no_outputs_file();
    let saved = placed_at(start, 1.25, 30, 40);
    let result = backend.persist(&saved, None).await;
    assert!(
        matches!(
            result,
            Err(Error::Unsupported {
                what: "saving a layout other than the one on screen",
                ..
            })
        ),
        "{result:?}"
    );
    assert!(!outputs.exists(), "nothing was written");

    backend.apply(&saved).await.expect("niri applies it");
    until(wlr, "the applied layout", |layout| {
        shows(layout, 1.25, 30, 40)
    })
    .await;
    let Persisted::File(file) = backend.persist(&saved, None).await.expect("saved");
    assert_eq!(file.path, outputs);
    assert_eq!(file.main_config, niri_config());
    assert!(file.created);
    assert!(
        file.included,
        "the harness's config includes it, optionally"
    );
    assert!(!file.overridden);
    assert!(file.first_wins);
    let text = std::fs::read_to_string(&outputs).expect("written");
    assert_eq!(ownership(&text, Comment::Slashes), Ownership::Managed);
    assert!(text.contains("output \"winit\" {\n"), "{text}");
    let (valid, said) = validate(&niri_config());
    assert!(valid, "niri reads it: {said}\n{text}");

    // A change at runtime leaves the file as it was, and a niri started now
    // starts at the saved layout.
    backend
        .apply(&placed_at(start, 1.5, 0, 0))
        .await
        .expect("niri applies it");
    until(wlr, "the runtime change", |layout| shows(layout, 1.5, 0, 0)).await;
    let fresh = FreshNiri::start("fresh-backend").await;
    until(&fresh.wlr, "the saved layout at startup", |layout| {
        shows(layout, 1.25, 30, 40)
    })
    .await;
    drop(fresh);

    // Edited by hand: niri shows the edit at once, and dsplr replaces the
    // file only once its text is approved.
    let edited_text = text.replace("scale 1.25", "scale 2");
    std::fs::write(&outputs, &edited_text).expect("edited");
    until(wlr, "the edit", |layout| shows(layout, 2.0, 30, 40)).await;
    backend.apply(&saved).await.expect("niri applies it");
    until(wlr, "the applied layout", |layout| {
        shows(layout, 1.25, 30, 40)
    })
    .await;
    let conflict = match backend.persist(&saved, None).await {
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
        .await
        .expect("saved once approved");
    assert_eq!(std::fs::read_to_string(&outputs).expect("written"), text);
}

/// A compositor nested in sway or niri inherits `SWAYSOCK` or
/// `NIRI_SOCKET`, which answers, from the outer compositor's process. The
/// harness has both kinds: niri inside a headless sway.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the nested niri harness: tests/e2e/headless-niri.sh"]
async fn an_inherited_ipc_socket_does_not_count() {
    let runtime = harness_runtime();
    let var = |name: &str| std::env::var_os(name).unwrap_or_else(|| panic!("{name} is set"));
    let (niri_display, niri_socket) = (var("WAYLAND_DISPLAY"), var("NIRI_SOCKET"));
    let (sway_display, swaysock) = (
        var("DSPLR_E2E_NIRI_PARENT"),
        var("DSPLR_E2E_NIRI_PARENT_SWAYSOCK"),
    );
    let compositor =
        async |display: &OsString, swaysock: Option<&OsString>, niri: Option<&OsString>| {
            Wlr::from_vars(|name| match name {
                "WAYLAND_DISPLAY" => Some(display.clone()),
                "XDG_RUNTIME_DIR" => Some(runtime.clone().into_os_string()),
                "SWAYSOCK" => swaysock.cloned(),
                "NIRI_SOCKET" => niri.cloned(),
                _ => None,
            })
            .await
            .expect("its output manager")
            .compositor()
            .await
        };
    // niri, with the sway it runs in named too, as a niri started from a
    // sway session would have it: sway's socket answers, from sway.
    assert_eq!(
        compositor(&niri_display, Some(&swaysock), Some(&niri_socket)).await,
        Compositor::Niri
    );
    // The sway, with niri's socket named, as a compositor nested in niri
    // would have it: niri's socket answers, from niri.
    assert_eq!(
        compositor(&sway_display, None, Some(&niri_socket)).await,
        Compositor::Other
    );
    assert_eq!(
        compositor(&sway_display, Some(&swaysock), Some(&niri_socket)).await,
        Compositor::Sway
    );
    assert_eq!(
        compositor(&niri_display, Some(&swaysock), None).await,
        Compositor::Other
    );
}
