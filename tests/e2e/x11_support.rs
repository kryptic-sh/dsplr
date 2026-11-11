//! Shared by every X11 e2e test (included with `#[path]`, beside
//! `support.rs`): the check that the test runs under
//! `tests/e2e/headless-x11.sh`, against its X server (Xvfb, or Xorg's dummy
//! driver) and nothing else.

#![allow(
    dead_code,
    reason = "included into each X11 e2e test binary, and each uses a subset"
)]

use std::path::PathBuf;

/// The harness's runtime directory, after checking that this process really
/// runs under the X11 harness (see [`crate::support::harness_runtime`]), and
/// that `DISPLAY` is the harness's Xvfb, with no Wayland session to find.
pub fn harness_runtime() -> PathBuf {
    on_server("xvfb")
}

/// [`harness_runtime`], for the harness's Xorg with the dummy driver
/// (`DSPLR_E2E_X11_SERVER=xorg`).
pub fn xorg_runtime() -> PathBuf {
    on_server("xorg")
}

/// The harness's runtime directory, after checking it runs `server`.
fn on_server(server: &str) -> PathBuf {
    let started = std::env::var("DSPLR_E2E_X11_SERVER").unwrap_or_default();
    assert_eq!(
        started, server,
        "the harness started {started:?}, and this test is for {server}: set DSPLR_E2E_X11_SERVER"
    );
    let runtime = crate::support::harness_runtime("DSPLR_E2E_X11", "tests/e2e/headless-x11.sh");
    let display = std::env::var("DSPLR_E2E_X11_DISPLAY").expect("set by the harness");
    assert_eq!(
        std::env::var("DISPLAY").ok().as_deref(),
        Some(display.as_str()),
        "DISPLAY is not the harness's X server"
    );
    assert!(
        std::env::var_os("WAYLAND_DISPLAY").is_none(),
        "WAYLAND_DISPLAY is set: this is a Wayland session, not the harness's"
    );
    assert_eq!(
        std::env::var_os("HOME").map(PathBuf::from),
        Some(runtime.join("home")),
        "HOME is not the harness's"
    );
    runtime
}

/// The command lines of the revert watchdogs (`dsplr restore --revert
/// --after …`) started under the harness: those whose `XDG_RUNTIME_DIR` is
/// `runtime`.
pub fn watchdogs(runtime: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    for process in std::fs::read_dir("/proc").expect("/proc").flatten() {
        // Processes come and go while it is read, and others' environments
        // cannot be read: either is not one of these.
        let (Ok(line), Ok(environ)) = (
            std::fs::read(process.path().join("cmdline")),
            std::fs::read(process.path().join("environ")),
        ) else {
            continue;
        };
        let words = |bytes: &[u8]| -> Vec<String> {
            bytes
                .split(|&byte| byte == 0)
                .filter(|word| !word.is_empty())
                .map(|word| String::from_utf8_lossy(word).into_owned())
                .collect()
        };
        let args = words(&line);
        let here = format!("XDG_RUNTIME_DIR={}", runtime.display());
        if args.iter().any(|arg| arg == "--after") && words(&environ).contains(&here) {
            found.push(args.join(" "));
        }
    }
    found
}
