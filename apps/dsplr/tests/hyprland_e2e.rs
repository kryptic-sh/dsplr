//! `dsplr apply` and `dsplr restore --revert` against a real, headless
//! Hyprland: keeping a change, declining it, letting the prompt time out, and
//! the watchdog reverting for a dsplr that died.
//!
//! Run through the harness, which starts that Hyprland where it cannot reach
//! the desktop session and gives dsplr a private state directory:
//!
//! ```text
//! tests/e2e/headless-hyprland.sh cargo test -p dsplr --test hyprland_e2e -- --ignored
//! ```
//!
//! The confirm prompt needs a terminal, which `script` (util-linux) provides.
//! `DSPLR_BIN` names the dsplr to run when it is not where Cargo built it (in
//! CI's VM).

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::{Layout, Transform};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/hyprland_support.rs"]
mod support;

use support::{edited, entry, entry_mut, harness_runtime, hyprctl, snapshot_until};

/// The confirm timeout the declined and timed-out runs use.
const CONFIRM_SECS: u32 = 2;

/// How long the watchdog may take to revert after dsplr is killed: its whole
/// delay (settle, prompt and grace, `restore::watchdog_delay`) and the
/// revert's own settle, with room to spare.
const WATCHDOG_WAIT: Duration = Duration::from_secs(30);

/// How long after the kill the change must still show. The watchdog's delay
/// is longer: `restore::watchdog_delay` adds the settle and a grace to the
/// confirm timeout.
const STILL_CHANGED: Duration = Duration::from_secs(3);

/// What `dsplr` prints when it asks.
const PROMPT: &str = "Keep this layout?";

/// The exit code of a change that was reverted (`Exit::Reverted`).
const REVERTED: i32 = 5;

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// `path` for a shell command line, refusing anything that would need
/// quoting: every path here is the harness's or Cargo's own.
fn plain(path: &Path) -> &str {
    let text = path.to_str().expect("a UTF-8 path");
    assert!(
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)),
        "{text:?} would need quoting"
    );
    text
}

/// `dsplr apply <file> --confirm-timeout <CONFIRM_SECS>` on a terminal, as
/// `script` gives one, with the pid of dsplr itself written to `pid_file`.
/// Its stdin is the terminal's input, left open until the child is waited on.
fn apply_on_a_terminal(file: &Path, pid_file: &Path) -> Child {
    let command = format!(
        "echo $$ > {pid}; exec {dsplr} apply {file} --confirm-timeout {CONFIRM_SECS}",
        pid = plain(pid_file),
        dsplr = plain(&dsplr_bin()),
        file = plain(file),
    );
    Command::new("script")
        .args(["--quiet", "--return", "--command", &command, "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("script (util-linux) runs")
}

fn exit_and_output(output: &Output) -> (Option<i32>, String) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

fn revert_path() -> PathBuf {
    let path = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    assert!(path.starts_with(harness_runtime()), "{}", path.display());
    path
}

/// Waits for the layout to be `expected` again, for at most `within`.
async fn back_to(backend: &Backend, expected: &Layout, what: &str, within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        let layout = backend.snapshot().await.expect("snapshot");
        if layout == *expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: not back within {within:?}: {layout:#?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless Hyprland harness: tests/e2e/headless-hyprland.sh"]
async fn dsplr_apply_keeps_declines_times_out_and_is_reverted_by_its_watchdog() {
    let runtime = harness_runtime();
    let reply = hyprctl("/output create headless DSPLR-1");
    assert_eq!(reply.trim(), "ok", "creating DSPLR-1: {reply}");
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));

    // Pin the layout Hyprland chose, so later reads compare equal to it.
    let first = snapshot_until(&backend, "DSPLR-1", |layout| {
        layout
            .entries
            .iter()
            .any(|e| e.output.as_ref().is_some_and(|o| o.connector == "DSPLR-1"))
    })
    .await;
    backend.apply(&first).await.expect("applied");
    let start = snapshot_until(&backend, "the pinned layout", |layout| *layout == first).await;

    let changed = edited(&start, |layout| {
        entry_mut(layout, "DSPLR-1").state.transform = Transform::Rotate90;
    });
    let file = runtime.join("layout.json");
    std::fs::write(&file, changed.to_json().expect("JSON")).expect("layout file written");
    let rotated = |layout: &Layout| entry(layout, "DSPLR-1").state.transform == Transform::Rotate90;
    let revert_file = revert_path();

    // --yes keeps it without asking, and leaves the revert point for a
    // revert by hand, which puts the old layout back and removes it.
    let output = Command::new(dsplr_bin())
        .args(["apply", "--yes"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(0), "{text}");
    snapshot_until(&backend, "--yes applied", rotated).await;
    let point = revert::load(&revert_file)
        .expect("readable")
        .expect("--yes leaves the revert point");
    assert_eq!(point.layout, start);

    let output = Command::new(dsplr_bin())
        .args(["restore", "--revert"])
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(0), "{text}");
    back_to(&backend, &start, "restore --revert", support::SETTLE).await;
    assert!(!revert_file.exists(), "a revert removes the revert point");

    // "n" on the prompt reverts, and so does no answer.
    let pid_file = runtime.join("dsplr.pid");
    for answer in [Some("n\n"), None] {
        let mut child = apply_on_a_terminal(&file, &pid_file);
        let mut input = child.stdin.take().expect("piped");
        if let Some(answer) = answer {
            input.write_all(answer.as_bytes()).expect("answered");
        }
        let output = child.wait_with_output().expect("dsplr ran");
        drop(input);
        let (code, text) = exit_and_output(&output);
        assert_eq!(code, Some(REVERTED), "answer {answer:?}: {text}");
        assert!(text.contains(PROMPT), "answer {answer:?} was asked: {text}");
        back_to(&backend, &start, "after the prompt", support::SETTLE).await;
        assert!(!revert_file.exists(), "a revert removes the revert point");
    }

    // A dsplr killed at the prompt is reverted by its watchdog.
    std::fs::remove_file(&pid_file).expect("the last run's pid file");
    let mut child = apply_on_a_terminal(&file, &pid_file);
    let mut terminal = child.stdout.take().expect("piped");
    let shown = std::thread::spawn(move || {
        // Reads the terminal until the prompt shows, then hands back what
        // it read.
        let mut seen = Vec::new();
        let mut chunk = [0; 4096];
        while !String::from_utf8_lossy(&seen).contains(PROMPT) {
            match terminal.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.extend_from_slice(&chunk[..n]),
            }
        }
        String::from_utf8_lossy(&seen).into_owned()
    })
    .join()
    .expect("the reader");
    assert!(
        shown.contains(PROMPT),
        "dsplr asked before the kill: {shown}"
    );
    let pid = std::fs::read_to_string(&pid_file).expect("dsplr wrote its pid");
    let status = Command::new("kill")
        .args(["-KILL", pid.trim()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "dsplr {} killed", pid.trim());
    child.wait().expect("script ended");
    // Nothing else reverts: the change stays until the watchdog's delay is up,
    // longer than any revert done at once would take.
    tokio::time::sleep(STILL_CHANGED).await;
    let layout = backend.snapshot().await.expect("snapshot");
    assert!(rotated(&layout), "nothing reverts before the watchdog does");
    back_to(&backend, &start, "the watchdog", WATCHDOG_WAIT).await;
    // The watchdog removes the revert point after its own settle.
    let deadline = Instant::now() + support::SETTLE;
    while revert_file.exists() {
        assert!(
            Instant::now() < deadline,
            "the watchdog removed the revert point"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
