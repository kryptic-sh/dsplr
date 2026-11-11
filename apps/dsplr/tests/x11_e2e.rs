//! `dsplr backend`, `dsplr list` and `dsplr set` against a real X server,
//! the harness's Xvfb: a change kept, one declined and one reverted by its
//! watchdog.
//!
//! Run through the harness, which starts that Xvfb on a display of its own
//! and gives dsplr a private state and config directory:
//!
//! ```text
//! tests/e2e/headless-x11.sh cargo test -p dsplr --test x11_e2e -- --ignored
//! ```
//!
//! `DSPLR_BIN` names the dsplr to run when it is not where Cargo built it
//! (in CI's VM). Changes are kept with `--yes`, declined on a terminal
//! `script` (util-linux) gives dsplr, or left to the revert watchdog; each
//! test leaves `screen` at 1920x1080, as Xvfb starts it.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use dsplr_model::{Layout, OutputId};

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/x11_support.rs"]
mod x11_support;

use support::{Watcher, answered_on_a_terminal, entry, on_a_terminal, plain};
use x11_support::{harness_runtime, watchdogs};

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// Runs dsplr with `args` and no terminal, and returns its exit code and
/// everything it printed.
fn dsplr(args: &[&str]) -> (Option<i32>, String) {
    exit_and_output(
        &Command::new(dsplr_bin())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("dsplr runs"),
    )
}

/// The exit code of a process that ran, and everything it printed.
fn exit_and_output(output: &Output) -> (Option<i32>, String) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

#[test]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
fn the_cli_finds_and_lists_xvfb() {
    harness_runtime();

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    for said in [
        "X11: RandR 1.",
        "backend: randr",
        "custom modes, mirroring",
        "starts at the origin",
    ] {
        assert!(text.contains(said), "{said:?} not in:\n{text}");
    }
    // Asked for by either name.
    for name in ["randr", "x11"] {
        let (code, text) = dsplr(&["--backend", name, "backend"]);
        assert_eq!(code, Some(0), "{name}: {text}");
        assert!(text.contains("backend: randr"), "{name}: {text}");
    }

    let (code, text) = dsplr(&["list"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.starts_with("screen: screen\n  1920x1080"), "{text}");

    let (code, json) = dsplr(&["list", "--json"]);
    assert_eq!(code, Some(0), "{json}");
    let layout = Layout::from_json(&json).expect("dsplr list --json reads back");
    let screen = entry(&layout, "screen");
    assert_eq!(screen.id, OutputId::Connector("screen".into()));
    assert_eq!(
        screen
            .state
            .mode
            .as_ref()
            .map(|mode| (mode.width, mode.height)),
        Some((1920, 1080))
    );

    // Planned and validated, not tested: RandR has no test request.
    let (code, text) = dsplr(&["set", "screen", "--pos", "0,0", "--dry-run"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("randr cannot test a layout: RandR has no test request"),
        "{text}"
    );
}

/// `dsplr list --json`'s layout of the harness's Xvfb.
fn listed() -> Layout {
    let (code, json) = dsplr(&["list", "--json"]);
    assert_eq!(code, Some(0), "{json}");
    Layout::from_json(&json).expect("dsplr list --json reads back")
}

/// The size `screen` is driven at.
fn size() -> (u32, u32) {
    let layout = listed();
    let mode = entry(&layout, "screen").state.mode.clone().expect("a mode");
    (mode.width, mode.height)
}

#[test]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
fn changes_are_kept_reverted_and_reverted_by_the_watchdog() {
    let runtime = harness_runtime();
    let revert = dsplr_persist::revert::default_path().expect("XDG_STATE_HOME is set");
    assert!(revert.starts_with(&runtime), "{}", revert.display());
    assert_eq!(size(), (1920, 1080));

    // Kept: a mode `screen` does not list, made with CVT's timings (59.855
    // Hz for 60, which the read-back says), at the screen size it needs.
    let (code, text) = dsplr(&["set", "screen", "--mode", "1280x720@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("got 1280x720@59.855"), "{text}");
    assert_eq!(size(), (1280, 720));
    let (code, text) = dsplr(&["set", "screen", "--mode", "1920x1080", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(size(), (1920, 1080));

    // Declined at the prompt: reverted (exit 5), and its revert point gone.
    let output = answered_on_a_terminal(
        &format!(
            "exec {} set screen --mode 1280x720@59.855",
            plain(&dsplr_bin())
        ),
        "n\n",
    );
    let (code, text) = exit_and_output(&output);
    assert_eq!(code, Some(REVERTED), "{text}");
    assert_eq!(size(), (1920, 1080));
    assert!(!revert.exists(), "{} left behind", revert.display());

    // Killed at its prompt: its watchdog reverts it.
    let pid_file = runtime.join("dsplr-set.pid");
    let mut terminal = on_a_terminal(&format!(
        "echo $$ > {pid}; exec {dsplr} set screen --mode 1280x720@59.855 \
         --confirm-timeout {secs}",
        pid = plain(&pid_file),
        dsplr = plain(&dsplr_bin()),
        secs = KILLED_CONFIRM.as_secs(),
    ));
    let started = Instant::now();
    while size() != (1280, 720) {
        assert!(
            started.elapsed() < KILLED_CONFIRM,
            "the change never applied"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // Its watchdog, among those of the changes kept before, which wait out
    // their delay before they find their change kept.
    let token = dsplr_persist::revert::load(&revert)
        .expect("its revert point reads")
        .expect("its revert point")
        .token;
    assert!(
        watchdogs(&runtime).iter().any(|line| line.contains(&token)),
        "no watchdog for {token}"
    );
    let pid = std::fs::read_to_string(&pid_file).expect("dsplr wrote its pid");
    let status = Command::new("kill")
        .args(["-KILL", pid.trim()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "dsplr {} killed", pid.trim());
    assert!(started.elapsed() < KILLED_CONFIRM, "killed too late");
    terminal.wait().expect("script ended");
    let deadline = Instant::now() + WATCHDOG;
    while size() != (1920, 1080) || revert.exists() {
        assert!(
            Instant::now() < deadline,
            "the watchdog did not revert it and remove its revert point"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The exit code of a change declined and reverted (`Exit::Reverted`).
const REVERTED: i32 = 5;
/// The confirm timeout of the change killed at its prompt: long enough to
/// see it applied and kill it first.
const KILLED_CONFIRM: Duration = Duration::from_secs(15);
/// How long that change's watchdog may take: its delay for RandR's request
/// timeout (an apply's, `randr::APPLY_TIMEOUT`) and [`KILLED_CONFIRM`],
/// about 110 s, then its revert, with room.
const WATCHDOG: Duration = Duration::from_secs(180);

/// `dsplr watch` on X11: it restores the profile picked for the outputs
/// when it starts, and goes on following the server through a change
/// another client makes with xrandr (the outputs connected are the same, so
/// it restores nothing), until it is stopped.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
async fn watch_restores_a_profile_and_follows_xrandr_s_changes() {
    let runtime = harness_runtime();
    // A profile at CVT's 1280x720, saved from the screen, then the screen
    // put back.
    let (code, text) = dsplr(&["set", "screen", "--mode", "1280x720@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["save", "--profile", "small"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["set", "screen", "--mode", "1920x1080", "--yes"]);
    assert_eq!(code, Some(0), "{text}");

    let mut command = Command::new(dsplr_bin());
    command.arg("watch").stdin(Stdio::null());
    let mut watcher = Watcher::start(command, runtime.join("watch.log"));
    watcher
        .restored(listed, 1, "small", |layout| {
            entry(layout, "screen")
                .state
                .mode
                .as_ref()
                .is_some_and(|mode| (mode.width, mode.height) == (1280, 720))
        })
        .await;

    // Another client's change: seen (the watch woke and read again), and
    // nothing restored for it.
    let status = Command::new("xrandr")
        .args(["--output", "screen", "--mode", "1920x1080"])
        .status()
        .expect("xrandr runs");
    assert!(status.success());
    watcher
        .said("dsplr: the displays changed; the same outputs are connected")
        .await;
    assert!(watcher.running(), "{}", watcher.log());
    assert_eq!(watcher.restores(), 1, "{}", watcher.log());
    assert_eq!(size(), (1920, 1080));
    let log = watcher.stop();
    assert!(!log.contains("stopped following"), "{log}");

    let (code, text) = dsplr(&["profile", "rm", "small"]);
    assert_eq!(code, Some(0), "{text}");
}

/// `dsplr` with each of `vars` set to its value, or unset with `None`, so
/// the session's own values do not reach it.
fn dsplr_with(vars: &[(&str, Option<&str>)], args: &[&str]) -> (Option<i32>, String) {
    let mut command = Command::new(dsplr_bin());
    command.args(args).stdin(Stdio::null());
    for &(name, value) in vars {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    exit_and_output(&command.output().expect("dsplr runs"))
}

/// `dsplr` with `XDG_CURRENT_DESKTOP` as `desktop`, unset with `None`.
fn dsplr_on(desktop: Option<&str>, args: &[&str]) -> (Option<i32>, String) {
    dsplr_with(&[("XDG_CURRENT_DESKTOP", desktop)], args)
}

/// Sets `screen` to `mode` with xrandr, as another client would.
fn screen_mode(mode: &str) {
    let status = Command::new("xrandr")
        .args(["--output", "screen", "--mode", mode])
        .status()
        .expect("xrandr runs");
    assert!(status.success(), "xrandr --mode {mode}");
}

/// `dsplr save` on X11: the profile for the outputs connected now, and the
/// autostart entry whose command, run as a session would at login, puts it
/// back, by dsplr's own path; `restore --login` changing nothing under a
/// desktop that restores its own layout or under Wayland, and restoring
/// otherwise; no entry under Xfce, the line for its config under i3; an
/// entry someone else wrote replaced only when allowed.
#[test]
#[ignore = "needs the Xvfb harness: tests/e2e/headless-x11.sh"]
fn save_writes_a_profile_that_the_autostart_entry_restores() {
    let runtime = harness_runtime();
    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("set"));
    assert!(config.starts_with(&runtime), "{}", config.display());
    let autostart = config.join("autostart/dsplr-restore.desktop");
    // Xvfb's `screen` has no EDID, so the profile is named by its connector.
    let profile = config.join("dsplr/profiles/screen.json");
    let exe = std::fs::canonicalize(dsplr_bin()).expect("dsplr's path");
    let exe = exe.to_str().expect("UTF-8");
    assert!(
        !exe.contains([' ', '"', '\\', '$', '%']),
        "quoted in Exec: {exe}"
    );

    let (code, text) = dsplr(&["set", "screen", "--mode", "1280x720@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr_on(None, &["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("saved profile \"screen\""), "{text}");
    assert!(text.contains("wrote "), "{text}");
    assert!(
        text.contains("exec --no-startup-id dsplr restore --login --yes"),
        "{text}"
    );
    assert!(profile.is_file(), "{text}");
    let entry = std::fs::read_to_string(&autostart).expect("the autostart entry");
    let value = |key: &str| {
        entry
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or_else(|| panic!("no {key} line:\n{entry}"))
    };
    let exec = value("Exec=");
    assert_eq!(exec, format!("{exe} restore --login --yes"));
    assert_eq!(value("TryExec="), exe);

    // Login: the entry's command, with no terminal, restores the profile.
    screen_mode("1920x1080");
    let words: Vec<&str> = exec.split_whitespace().collect();
    let login = |vars: &[(&str, Option<&str>)]| {
        let mut command = Command::new(words[0]);
        command.args(&words[1..]).stdin(Stdio::null());
        for &(name, value) in vars {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        exit_and_output(&command.output().expect("the entry's command runs"))
    };
    let (code, text) = login(&[("XDG_CURRENT_DESKTOP", None)]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("restoring profile \"screen\""), "{text}");
    assert_eq!(size(), (1280, 720));

    // Where the desktop restores its own layout, or the session is Wayland:
    // nothing, said to no one.
    screen_mode("1920x1080");
    for vars in [
        &[("XDG_CURRENT_DESKTOP", Some("XFCE"))][..],
        &[("XDG_CURRENT_DESKTOP", Some("X-Cinnamon"))],
        &[
            ("XDG_CURRENT_DESKTOP", None),
            ("WAYLAND_DISPLAY", Some("wayland-dsplr-none")),
        ],
    ] {
        let (code, text) = login(vars);
        assert_eq!((code, text.as_str()), (Some(0), ""), "{vars:?}");
        assert_eq!(size(), (1920, 1080), "{vars:?}");
    }
    // Under i3 it restores.
    let (code, text) = login(&[("XDG_CURRENT_DESKTOP", Some("i3"))]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(size(), (1280, 720));

    // Saved again on a desktop that runs XDG autostart: the entry is there
    // already, and is left as it is; the profile is this save's own, so it
    // is replaced, not renamed.
    let (code, text) = dsplr_on(Some("Openbox"), &["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("dsplr-restore.desktop runs `dsplr restore --login`"),
        "{text}"
    );
    assert!(text.contains("saved profile \"screen\""), "{text}");
    assert!(!text.contains("i3"), "{text}");
    assert_eq!(std::fs::read_to_string(&autostart).unwrap(), entry);

    // Under Xfce, which restores its own layout: no entry, and why.
    std::fs::remove_file(&autostart).expect("the entry removed");
    let (code, text) = dsplr_on(Some("XFCE"), &["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("XFCE puts its own saved display layout back at login"),
        "{text}"
    );
    assert!(!autostart.exists(), "{text}");

    // Under i3: the line for its config, and no entry.
    let (code, text) = dsplr_on(Some("i3"), &["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("add this line to i3's config")
            && text.contains("exec --no-startup-id dsplr restore --login --yes"),
        "{text}"
    );
    assert!(!autostart.exists(), "{text}");

    // Someone else's entry: shown, and replaced only with --yes.
    let theirs = "[Desktop Entry]\nType=Application\nExec=true\n";
    std::fs::create_dir_all(autostart.parent().unwrap()).unwrap();
    std::fs::write(&autostart, theirs).unwrap();
    let (code, text) = dsplr_on(None, &["save"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("was not written by dsplr;"), "{text}");
    assert_eq!(std::fs::read_to_string(&autostart).unwrap(), theirs);
    let (code, text) = dsplr_on(None, &["save", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(std::fs::read_to_string(&autostart).unwrap(), entry);

    screen_mode("1920x1080");
    for leftover in [&autostart, &profile] {
        std::fs::remove_file(leftover).expect("removed");
    }
}
