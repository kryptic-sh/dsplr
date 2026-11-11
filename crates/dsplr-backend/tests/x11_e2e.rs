//! The RandR backend against a real X server, the harness's Xvfb: detection
//! finding it, its outputs read as a layout, and a layout applied and
//! applied back.
//!
//! Run through the harness, which starts that Xvfb on a display of its own:
//!
//! ```text
//! tests/e2e/headless-x11.sh cargo test -p dsplr-backend --test x11_e2e -- --ignored
//! ```
//!
//! The test refuses to run when the harness did not start it, so `--ignored`
//! on its own never reads or changes a real X server. Xvfb has one CRTC and
//! one output, `screen`, with no EDID and no physical size.

#![cfg(target_os = "linux")]

use dsplr_backend::{Backend, Error, Named};
use dsplr_model::{Mode, OutputId};

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/x11_support.rs"]
mod x11_support;

use support::{edited, entry, entry_mut};
use x11_support::harness_runtime;

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
async fn xvfb_is_found_and_read() {
    harness_runtime();

    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    assert_eq!(backend.name(), "randr");
    let x11 = detection.checks.last().expect("checks");
    assert_eq!(x11.what, "X11");
    assert!(x11.found.starts_with("RandR 1."), "{x11:?}");
    let named = dsplr_backend::detect_named(Named::Randr).await;
    assert_eq!(named.backend.map(|backend| backend.name()), Some("randr"));

    let layout = backend.snapshot().await.expect("snapshot");
    assert_eq!(layout.entries.len(), 1, "{layout:?}");
    let screen = entry(&layout, "screen");
    assert_eq!(screen.id, OutputId::Connector("screen".into()));
    assert!(screen.state.enabled);
    assert_eq!((screen.state.x, screen.state.y), (0, 0));
    let mode = screen.state.mode.as_ref().expect("a mode");
    assert_eq!((mode.width, mode.height), (1920, 1080));
    let output = screen.output.as_ref().expect("hardware facts");
    assert!(
        output
            .modes
            .iter()
            .any(|listed| listed.matches(&Mode::new(1920, 1080, 0))),
        "{output:?}"
    );

    // The layout read plans to nothing, and testing it says RandR has no
    // test, after planning it.
    match backend.test(&layout).await {
        Err(Error::Unsupported { what, .. }) => assert_eq!(what, "testing a layout"),
        other => panic!("{other:?}"),
    }
    let Backend::Randr(ref randr) = backend else {
        panic!("{backend:?}");
    };
    let snapshot = randr.read().await.expect("the server's state");
    assert_eq!(snapshot.crtcs.len(), 1);
    assert_eq!(snapshot.crtcs[0].panning, None, "Xvfb has no panning");
    assert_eq!(snapshot.crtcs[0].transform, None);
    assert_eq!(
        (snapshot.screen.width, snapshot.screen.height),
        (1920, 1080)
    );
    let plan = dsplr_backend::randr::plan(&snapshot, &layout).expect("plans");
    assert_eq!(
        plan.operations,
        [
            dsplr_backend::randr::Operation::GrabServer,
            dsplr_backend::randr::Operation::UngrabServer
        ]
    );

    // Applied: a CVT mode made for a size `screen` does not list, and the
    // screen shrunk to it; then back. Xvfb has no panning, so nothing is
    // noted.
    let small = edited(&layout, |layout| {
        entry_mut(layout, "screen").state.mode = Some(Mode::new(1280, 720, 60_000));
    });
    assert_eq!(
        backend.apply(&small).await.expect("applied"),
        Vec::<String>::new()
    );
    let now = backend.snapshot().await.expect("snapshot");
    let mode = entry(&now, "screen").state.mode.clone().expect("a mode");
    assert_eq!(
        (mode.width, mode.height, mode.refresh_mhz),
        (1280, 720, 59_855)
    );
    let after = randr.read().await.expect("the server's state");
    assert_eq!((after.screen.width, after.screen.height), (1280, 720));
    // The same layout again finds the mode made before.
    backend.apply(&small).await.expect("applied again");
    let again = randr.read().await.expect("the server's state");
    assert_eq!(after.resources.modes.len(), again.resources.modes.len());

    backend.apply(&layout).await.expect("applied back");
    let back = backend.snapshot().await.expect("snapshot");
    assert_eq!(
        entry(&back, "screen")
            .state
            .mode
            .as_ref()
            .map(|m| (m.width, m.height)),
        Some((1920, 1080))
    );
    assert_eq!(randr.read().await.expect("read").screen.width, 1920);
}

/// A request failing under the server grab still leaves the server
/// ungrabbed: a read on another connection, made while the failed one is
/// still open, is answered within a second rather than held up by a grab.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
async fn a_request_failing_under_the_grab_leaves_the_server_ungrabbed() {
    use dsplr_backend::randr::resources::{ModeXid, Rotation};
    use dsplr_backend::randr::{ModeRef, Operation, Plan};

    harness_runtime();
    let Some(Backend::Randr(randr)) = dsplr_backend::detect().await.backend else {
        panic!("no RandR backend");
    };
    let before = randr.read().await.expect("the server's state");
    let crtc = &before.crtcs[0];
    // A mode the server does not have: X refuses it (BadRRMode).
    let plan = Plan {
        operations: vec![
            Operation::GrabServer,
            Operation::SetCrtcConfig {
                crtc: crtc.id,
                mode: ModeRef::Listed(ModeXid(0x7fff_fff0)),
                x: 0,
                y: 0,
                rotation: Rotation::ROTATE_0,
                outputs: crtc.outputs.clone(),
            },
            Operation::UngrabServer,
        ],
        notes: Vec::new(),
    };
    let (sent, read) = randr
        .send_then_read_beside(plan, std::time::Duration::from_secs(1))
        .await
        .expect("ran");
    assert!(
        matches!(
            sent,
            Err(dsplr_backend::randr::Error::X {
                what: "SetCrtcConfig",
                ..
            })
        ),
        "{sent:?}"
    );
    let read = read.expect("read while the failed connection was still open");
    assert_eq!(read.crtcs[0].mode, crtc.mode, "nothing changed");
}
