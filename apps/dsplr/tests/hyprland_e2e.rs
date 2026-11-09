//! `dsplr apply` and `dsplr restore --revert` against a real, headless
//! Hyprland: keeping a change, declining it, letting the prompt time out, and
//! the watchdog reverting for a dsplr that died; `dsplr set` mirroring one
//! output and ending the mirror, through the same flow. And `dsplr save`:
//! creating monitors.lua, and asking before it replaces one it may not
//! simply overwrite.
//!
//! Run through the harness, which starts that Hyprland where it cannot reach
//! the desktop session and gives dsplr a private state directory:
//!
//! ```text
//! tests/e2e/headless-hyprland.sh cargo test -p dsplr --test hyprland_e2e -- --ignored --test-threads=1
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
use dsplr_persist::managed::{Comment, Ownership, ownership};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/hyprland_support.rs"]
mod hyprland_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use hyprland_support::{config_home, harness_runtime, hyprctl, snapshot_until};
use support::{answered_on_a_terminal, edited, entry, entry_mut, on_a_terminal, plain};

/// The confirm timeout the declined and timed-out runs use.
const CONFIRM_SECS: u32 = 2;

/// How long the watchdog may take to revert after dsplr is killed: its whole
/// delay (`restore::watchdog_delay` for this confirm timeout and Hyprland's
/// request timeout: two changes, each a request and a settle, the prompt and
/// a grace) and the revert's own settle, with room to spare.
const WATCHDOG_WAIT: Duration = Duration::from_secs(60);

/// How long after the kill the change must still show. The watchdog's delay
/// is longer: `restore::watchdog_delay` adds the requests, the settles and a
/// grace to the confirm timeout.
const STILL_CHANGED: Duration = Duration::from_secs(3);

/// What `dsplr` prints when it asks.
const PROMPT: &str = "Keep this layout?";

/// The exit code of a change that was reverted (`Exit::Reverted`).
const REVERTED: i32 = 5;

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// `dsplr apply <file> --confirm-timeout <CONFIRM_SECS>` on a terminal, as
/// `script` gives one, with the pid of dsplr itself written to `pid_file`.
/// Its stdin is the terminal's input, left open until the child is waited on.
fn apply_on_a_terminal(file: &Path, pid_file: &Path) -> Child {
    on_a_terminal(&format!(
        "echo $$ > {pid}; exec {dsplr} apply {file} --confirm-timeout {CONFIRM_SECS}",
        pid = plain(pid_file),
        dsplr = plain(&dsplr_bin()),
        file = plain(file),
    ))
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

    // Hyprland's test is a stub, so a dry run says it validated locally
    // only, and changes nothing.
    let output = Command::new(dsplr_bin())
        .args(["apply", "--dry-run"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("validated locally only"), "{text}");
    assert_eq!(backend.snapshot().await.expect("snapshot"), start);
    assert!(!revert_file.exists(), "a dry run starts no change");

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
    back_to(
        &backend,
        &start,
        "restore --revert",
        hyprland_support::SETTLE,
    )
    .await;
    assert!(!revert_file.exists(), "a revert removes the revert point");

    // dsplr set edits one output through the same flow: a mirror, which
    // Hyprland has and wlroots has not.
    let wayland_1 = entry(&start, "WAYLAND-1").id.clone();
    let output = Command::new(dsplr_bin())
        .args(["set", "DSPLR-1", "--mirror", "WAYLAND-1", "--yes"])
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(0), "{text}");
    snapshot_until(&backend, "DSPLR-1 mirroring", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.as_ref() == Some(&wayland_1)
    })
    .await;
    // And --no-mirror ends it. Hyprland moves a mirror onto its source
    // (`CMonitor::setMirror`), so --pos puts it back where it was. The
    // revert point left behind holds the mirror; the next change replaces it.
    let one = &entry(&start, "DSPLR-1").state;
    let pos = format!("{},{}", one.x, one.y);
    let output = Command::new(dsplr_bin())
        .args(["set", "DSPLR-1", "--no-mirror", "--pos", &pos, "--yes"])
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(0), "{text}");
    back_to(
        &backend,
        &start,
        "set --no-mirror",
        hyprland_support::SETTLE,
    )
    .await;

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
        back_to(
            &backend,
            &start,
            "after the prompt",
            hyprland_support::SETTLE,
        )
        .await;
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
    let deadline = Instant::now() + hyprland_support::SETTLE;
    while revert_file.exists() {
        assert!(
            Instant::now() < deadline,
            "the watchdog removed the revert point"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `dsplr save` with `args`, on no terminal.
fn save(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(dsplr_bin())
        .arg("save")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    exit_and_output(&output)
}

/// `dsplr save` on a terminal, answering `answer`.
fn save_on_a_terminal(answer: &str) -> (Option<i32>, String) {
    exit_and_output(&answered_on_a_terminal(
        &format!("exec {} save", plain(&dsplr_bin())),
        answer,
    ))
}

#[test]
#[ignore = "needs the headless Hyprland harness: tests/e2e/headless-hyprland.sh"]
fn dsplr_save_writes_monitors_lua_and_asks_before_replacing_one() {
    harness_runtime();
    let monitors = config_home().join("hypr/monitors.lua");
    assert!(!monitors.exists(), "the harness starts without one");
    let read = || std::fs::read_to_string(&monitors).expect("monitors.lua");

    // Created, with the line hyprland.lua needs named.
    let (code, text) = save(&[]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains(&monitors.display().to_string()), "{text}");
    assert!(text.contains("  require(\"monitors\")"), "{text}");
    assert_eq!(ownership(&read(), Comment::Lua), Ownership::Managed);

    // Edited by hand: refused, with the diff, until --force.
    let edited = format!("{}-- mine\n", read());
    std::fs::write(&monitors, &edited).expect("edited");
    let (code, text) = save(&[]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("edited by hand"), "{text}");
    assert!(
        text.contains("\n--- mine\n"),
        "the diff shows the line going: {text}"
    );
    assert!(text.contains("--force"), "{text}");
    assert_eq!(read(), edited, "nothing was written");
    let (code, text) = save(&["--force"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(ownership(&read(), Comment::Lua), Ownership::Managed);

    // Another tool's file, naming a monitor not connected: replaced only
    // after a yes, and its rule is kept.
    let foreign = "hl.monitor({ output = \"DSPLR-9\", disabled = true })\n";
    std::fs::write(&monitors, foreign).expect("written");
    let (code, text) = save(&[]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("no terminal"), "{text}");
    assert_eq!(read(), foreign);
    let (code, text) = save_on_a_terminal("n\n");
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("Replace "), "it was asked: {text}");
    assert_eq!(read(), foreign);
    let (code, text) = save_on_a_terminal("y\n");
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("kept the settings for DSPLR-9"), "{text}");
    let saved = read();
    assert_eq!(ownership(&saved, Comment::Lua), Ownership::Managed);
    assert!(saved.contains("output = \"DSPLR-9\""), "{saved}");

    // --yes replaces another tool's file without asking.
    std::fs::write(&monitors, foreign).expect("written");
    let (code, text) = save(&["--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(read(), saved);
}
