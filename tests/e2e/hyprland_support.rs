//! Shared by every Hyprland e2e test (included with `#[path]`): the check
//! that the test runs under `tests/e2e/headless-hyprland.sh`, raw requests to
//! that harness's Hyprland, and reading its layout back.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::{Layout, OutputEntry};

/// The harness's runtime directory, after checking that this process really
/// runs under the harness and not against a session's Hyprland, and that
/// dsplr's state goes to the harness's directory, not the user's.
pub fn harness_runtime() -> PathBuf {
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
    let state = std::env::var("XDG_STATE_HOME").unwrap_or_default();
    assert!(
        state.starts_with(&runtime),
        "XDG_STATE_HOME ({state:?}) is not inside the harness runtime"
    );
    PathBuf::from(runtime)
}

/// Sends one request to the harness's Hyprland, as `hyprctl` does, and
/// returns the reply.
pub fn hyprctl(request: &str) -> String {
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

/// How long Hyprland gets to show a change it accepted.
pub const SETTLE: Duration = Duration::from_secs(5);

pub fn entry<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the snapshot: {layout:#?}"))
}

pub fn entry_mut<'a>(layout: &'a mut Layout, connector: &str) -> &'a mut OutputEntry {
    layout
        .entries
        .iter_mut()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the layout"))
}

/// Takes snapshots until `shown` holds or [`SETTLE`] passes: Hyprland
/// applies monitor rules on its next frame, not when `eval` returns.
pub async fn snapshot_until(
    backend: &Backend,
    what: &str,
    shown: impl Fn(&Layout) -> bool,
) -> Layout {
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
pub fn edited(layout: &Layout, edit: impl FnOnce(&mut Layout)) -> Layout {
    let mut layout = layout.clone();
    edit(&mut layout);
    layout
}
