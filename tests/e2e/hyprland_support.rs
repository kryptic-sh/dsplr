//! Shared by every Hyprland e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-hyprland.sh`, raw requests to that harness's Hyprland,
//! and reading its layout back.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::Layout;

/// The harness's runtime directory, after checking that this process really
/// runs under the Hyprland harness (see [`crate::support::harness_runtime`]),
/// and that the Hyprland config `dsplr save` writes beside is the harness's.
pub fn harness_runtime() -> PathBuf {
    let runtime =
        crate::support::harness_runtime("DSPLR_E2E_HYPRLAND", "tests/e2e/headless-hyprland.sh");
    assert_eq!(
        std::env::var_os("HYPRLAND_CONFIG"),
        None,
        "HYPRLAND_CONFIG would point dsplr save at another config"
    );
    assert_eq!(
        config_home(),
        runtime.join("config"),
        "XDG_CONFIG_HOME is not the harness's"
    );
    // Without it, Hyprland's lookup (and dsplr's) would go on to
    // $HOME/.config: so HOME is the harness's too.
    let config = config_home().join("hypr/hyprland.lua");
    assert!(
        config.is_file(),
        "the harness's {} is missing",
        config.display()
    );
    assert_eq!(
        std::env::var_os("HOME").map(PathBuf::from),
        Some(runtime.join("home")),
        "HOME is not the harness's"
    );
    runtime
}

/// `XDG_CONFIG_HOME`, where the harness's Hyprland reads `hypr/hyprland.lua`.
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .expect("XDG_CONFIG_HOME is set by the harness")
        .into()
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
