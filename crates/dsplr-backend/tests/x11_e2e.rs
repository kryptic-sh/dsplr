//! The RandR backend against a real X server, the harness's Xvfb: detection
//! finding it, and its outputs read as a layout.
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

use support::entry;
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
    // Applying is not written yet, and says so.
    assert!(matches!(
        backend.apply(&layout).await,
        Err(Error::Unsupported { .. })
    ));
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
}
