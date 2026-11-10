//! The GUI's Apply against a real, headless sway: a change kept with Enter in
//! the confirm dialogs, one left to time out, one whose dsplr is killed
//! mid-confirm, which its watchdog reverts, and an output plugged in while
//! the window is open, which it follows without losing the pending edit,
//! and then turned off from the panel by keys alone. And `dsplr identify`,
//! its panel seen in a `grim` capture while it is up, gone after, and the
//! exits where there is no GUI session (no X display for the X11 shell, no
//! display at all, crcbl's headless shell).
//!
//! Run through the harness, with Mesa's software Vulkan (lavapipe), `wtype`
//! and `grim` installed:
//!
//! ```text
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json \
//!   tests/e2e/headless-sway.sh cargo test -p dsplr --test gui_e2e -- --ignored
//! ```
//!
//! Keys go in through `wtype`, which needs one keyboard held for the whole
//! session (the backlog's harness entry), and are sent as one argument each,
//! since `wtype` types a space between its arguments. CI's `gui` entry runs
//! it so, in its VM.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dsplr_core::Theme;
use dsplr_core::dialog::{IDENTIFY_SIZE, IDENTIFY_TIME};
use dsplr_gui::{CONFIRM_TITLE, TITLE};
use dsplr_model::Layout;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::entry;
use sway_support::{config_home, harness_runtime};

/// The confirm timeout the test configures: short, so the timed-out run and
/// the watchdog's wait stay short.
const CONFIRM_SECS: u64 = 5;
/// How long a step may take to show: a GUI starting on lavapipe, an apply
/// settling.
const STEP: Duration = Duration::from_secs(20);
/// How long the watchdog may take, killed dsplr and all:
/// `restore::watchdog_delay` for wlr's request timeout, with room.
const WATCHDOG: Duration = Duration::from_secs(90);

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// A process ended when the test ends, passing or not.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        // Already gone is fine: the test may have ended it.
        if self.0.kill().is_ok() {
            self.0.wait().expect("waiting for a killed process");
        }
    }
}

/// Holds a virtual keyboard for the session, so each `wtype` call's keys
/// reach the window (the backlog's harness entry).
fn hold_keyboard() -> Running {
    Running(
        Command::new("wtype")
            .args(["-s", "3600000", "-m", "shift"])
            .spawn()
            .expect("wtype is installed"),
    )
}

/// Types `keys`, one `wtype` argument: a character string, or `-k NAME`.
fn wtype(keys: &[&str]) {
    let status = Command::new("wtype")
        .args(keys)
        .status()
        .expect("wtype runs");
    assert!(status.success(), "wtype {keys:?}");
}

fn start_gui() -> Running {
    Running(
        Command::new(dsplr_bin())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("dsplr starts"),
    )
}

/// The layout, as `dsplr list --json` reads it.
fn layout() -> Layout {
    let output = Command::new(dsplr_bin())
        .args(["list", "--json"])
        .output()
        .expect("dsplr list runs");
    assert!(output.status.success(), "{output:?}");
    Layout::from_json(&String::from_utf8_lossy(&output.stdout)).expect("a layout")
}

/// Waits up to `limit` for HEADLESS-1's scale to be `want`.
fn until_scale(want: f64, limit: Duration, what: &str) {
    until_scale_of("HEADLESS-1", want, limit, what);
}

/// Waits up to `limit` for `output`'s scale to be `want`.
fn until_scale_of(output: &str, want: f64, limit: Duration, what: &str) {
    let deadline = Instant::now() + limit;
    loop {
        let now = entry(&layout(), output).state.scale;
        if (now - want).abs() < 1e-6 {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: scale {now}, not {want}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// How long the window may take to open and take the keys, or to show a
/// change waiting on its answer: generous, for a GUI starting on lavapipe in
/// CI's VM.
const READY: Duration = Duration::from_secs(60);

/// dsplr's windows in sway's tree: each one's title, and whether it has the
/// keyboard. The confirm dialogs are layer surfaces, which the tree does not
/// list; while a change waits on its answer the main window's title is
/// [`CONFIRM_TITLE`] (a toplevel dialog, where there are no layer surfaces,
/// has it too).
fn dsplr_windows() -> Vec<(String, bool)> {
    let output = Command::new("swaymsg")
        .args(["-t", "get_tree"])
        .output()
        .expect("swaymsg runs");
    assert!(output.status.success(), "swaymsg -t get_tree: {output:?}");
    let tree: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("sway's tree is JSON");
    let mut windows = Vec::new();
    let mut nodes = vec![&tree];
    while let Some(node) = nodes.pop() {
        if node["app_id"] == TITLE {
            windows.push((
                node["name"].as_str().unwrap_or_default().to_owned(),
                node["focused"].as_bool() == Some(true),
            ));
        }
        for children in ["nodes", "floating_nodes"] {
            nodes.extend(node[children].as_array().into_iter().flatten());
        }
    }
    windows
}

/// Waits up to [`READY`] for `ready` to hold of dsplr's windows; fails
/// saying what sway showed last.
fn until_windows(what: &str, ready: impl Fn(&[(String, bool)]) -> bool) {
    let deadline = Instant::now() + READY;
    loop {
        let windows = dsplr_windows();
        if ready(&windows) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: not seen within {READY:?}; sway shows dsplr's windows as {windows:?} (title, focused)"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Starts the GUI and waits until its window has the keys: those sent
/// before then are lost.
fn start_gui_ready() -> Running {
    let gui = start_gui();
    until_windows("the window opening with the keys", |windows| {
        windows.iter().any(|(_, focused)| *focused)
    });
    gui
}

/// Waits until a change waits on its answer (`true`), or no longer does
/// (`false`), by the window's title. The desktop shows an apply before
/// dsplr has read it back and asks, and a key sent between the two is the
/// canvas's, not the confirm's.
fn until_confirming(want: bool, what: &str) {
    until_windows(what, |windows| {
        windows.iter().any(|(title, _)| title == CONFIRM_TITLE) == want
    });
}

/// Starts the GUI, steps HEADLESS-1's scale up with `+` and applies it with
/// `a`, and waits for the desktop to show it and for the change to wait on
/// its answer.
fn apply_a_scale_step(from: f64, to: f64) -> Running {
    until_scale(from, STEP, "before");
    let gui = start_gui_ready();
    wtype(&["+"]);
    wtype(&["a"]);
    until_scale(to, STEP, "the apply");
    until_confirming(true, &format!("the confirm of the step to {to}"));
    gui
}

#[test]
#[ignore = "needs the headless sway harness, lavapipe and wtype: tests/e2e/headless-sway.sh"]
fn the_gui_keeps_on_enter_and_reverts_on_timeout_and_after_a_kill() {
    harness_runtime();
    let config = config_home().join("dsplr");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.toml"),
        format!("[confirm]\ntimeout_secs = {CONFIRM_SECS}\n"),
    )
    .unwrap();
    let _keyboard = hold_keyboard();

    // Kept: Enter in the dialog.
    let mut gui = apply_a_scale_step(1.0, 1.25);
    wtype(&["-k", "Return"]);
    until_confirming(false, "Enter answering the confirm");
    std::thread::sleep(Duration::from_secs(CONFIRM_SECS + 2));
    until_scale(1.25, STEP, "kept");
    wtype(&["q"]);
    let status = gui.0.wait().unwrap();
    assert!(status.success(), "quit after keeping: {status}");

    // Not answered: the dialog's countdown reverts it.
    let gui = apply_a_scale_step(1.25, 1.5);
    until_scale(1.25, Duration::from_secs(CONFIRM_SECS) + STEP, "timed out");
    drop(gui);

    // dsplr killed mid-confirm: the detached watchdog reverts it.
    let gui = apply_a_scale_step(1.25, 1.5);
    drop(gui);
    until_scale(1.25, WATCHDOG, "the watchdog");
    let revert_point =
        PathBuf::from(std::env::var("XDG_STATE_HOME").unwrap()).join("dsplr/revert.json");
    assert!(
        !revert_point.exists(),
        "the watchdog removes the revert point"
    );

    // Plugged in while the window is open: it shows up, Tab reaches it, and
    // the edit made before it came is still pending.
    let gui = start_gui_ready();
    wtype(&["+"]);
    let before = layout().entries.len();
    let status = Command::new("swaymsg")
        .arg("create_output")
        .status()
        .unwrap();
    assert!(status.success(), "swaymsg create_output");
    let plugged = layout();
    assert_eq!(plugged.entries.len(), before + 1, "a new output");
    let new = plugged
        .entries
        .iter()
        .filter_map(|e| e.output.as_ref().map(|o| o.connector.clone()))
        .find(|name| name != "HEADLESS-1")
        .expect("the new output");
    std::thread::sleep(Duration::from_secs(2));
    wtype(&["-k", "Tab"]);
    wtype(&["+"]);
    wtype(&["a"]);
    until_scale_of(&new, 1.25, STEP, "the plugged output's edit");
    until_scale(1.5, STEP, "the edit made before it came");
    until_confirming(true, "the confirm of the hotplug's edit");
    wtype(&["-k", "Return"]);
    until_confirming(false, "Enter answering the confirm");
    std::thread::sleep(Duration::from_secs(CONFIRM_SECS + 2));
    until_scale_of(&new, 1.25, STEP, "kept");

    // The panel from the keyboard: `p` takes the keys into it, landing on
    // the selected output's Enabled box, Enter turns it off (Enter is no
    // canvas key, so only the panel can have done it), Escape gives the
    // keys back to the canvas, and `a` applies.
    wtype(&["p"]);
    wtype(&["-k", "Return"]);
    wtype(&["-k", "Escape"]);
    wtype(&["a"]);
    until_confirming(true, "the confirm of turning it off from the panel");
    wtype(&["-k", "Return"]);
    until_confirming(false, "Enter answering the confirm");
    let deadline = Instant::now() + STEP;
    while entry(&layout(), &new).state.enabled {
        assert!(
            Instant::now() < deadline,
            "{new} still on after the panel's apply"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    wtype(&["q"]);
    let mut gui = gui;
    let status = gui.0.wait().unwrap();
    assert!(status.success(), "quit after the hotplug: {status}");
}

/// How many of `output`'s pixels are `color`, in a `grim` capture of it.
/// grim writes PPM (`P6`, then width, height and the maximum value, then the
/// pixels as RGB bytes), which needs no decoder.
fn pixels_of(output: &str, color: dsplr_core::Color) -> usize {
    let capture = Command::new("grim")
        .args(["-t", "ppm", "-o", output, "-"])
        .output()
        .expect("grim runs (the gui entry installs it)");
    assert!(capture.status.success(), "{capture:?}");
    let ppm = capture.stdout;
    // The header is four whitespace-separated fields, then one whitespace
    // byte before the pixels.
    let mut fields = 0;
    let mut at = 0;
    while fields < 4 {
        while ppm[at].is_ascii_whitespace() {
            at += 1;
        }
        while !ppm[at].is_ascii_whitespace() {
            at += 1;
        }
        fields += 1;
    }
    assert!(ppm.starts_with(b"P6"), "not a PPM capture");
    ppm[at + 1..]
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|rgb| **rgb == [color.r, color.g, color.b])
        .count()
}

#[test]
#[ignore = "needs the headless sway harness, lavapipe and grim: tests/e2e/headless-sway.sh"]
fn identify_shows_each_output_s_panel_for_its_time() {
    harness_runtime();
    let outputs: Vec<String> = layout()
        .entries
        .iter()
        .filter(|e| e.state.enabled)
        .filter_map(|e| e.output.as_ref().map(|o| o.connector.clone()))
        .collect();
    assert!(!outputs.is_empty());
    let surface = Theme::tokyo_night().surface;
    for output in &outputs {
        assert_eq!(pixels_of(output, surface), 0, "{output} before identify");
    }

    let started = Instant::now();
    let mut identify = Running(
        Command::new(dsplr_bin())
            .arg("identify")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("dsplr identify starts"),
    );
    // Its lines, each with when it came, read on a thread of their own so
    // waiting for one has a limit.
    let stderr = identify.0.stderr.take().unwrap();
    let (lines, said) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if lines.send((Instant::now(), line)).is_err() {
                return;
            }
        }
    });
    // Said once the panels are open: their time runs from just before it.
    // Detection, the device and the first frame all come before, so READY
    // allows for them.
    let (opened_at, opened) = loop {
        let (at, line) = said
            .recv_timeout(READY)
            .expect("identify says what it shows");
        if line.starts_with("dsplr: identifying ") {
            break (at, line);
        }
    };
    for output in &outputs {
        assert!(opened.contains(output.as_str()), "{output} in {opened:?}");
    }

    // Most of each panel is its surface colour, once it has drawn, before
    // its time is up.
    let panel = (IDENTIFY_SIZE.width * IDENTIFY_SIZE.height) as usize;
    for output in &outputs {
        while pixels_of(output, surface) < panel / 2 {
            assert!(
                opened_at.elapsed() < IDENTIFY_TIME,
                "no identify panel seen on {output} while it was up"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let status = identify.0.wait().unwrap();
    let rest: Vec<String> = said.try_iter().map(|(_, line)| line).collect();
    assert!(status.success(), "{status}: {rest:?}");
    let took = started.elapsed();
    assert!(took >= IDENTIFY_TIME, "closed after {took:?}");
    let after = opened_at.elapsed();
    assert!(
        after < IDENTIFY_TIME + STEP,
        "closed {after:?} after opening"
    );
    for output in &outputs {
        assert_eq!(pixels_of(output, surface), 0, "{output} after identify");
    }

    // No GUI session: the X11 shell asked for with no X display, while the
    // backend is still reachable over Wayland.
    let (code, stderr) = identify_with(&[("CRCBL_SHELL", Some("x11")), ("DISPLAY", None)]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("needs a GUI session"), "{stderr}");
    // No display at all: detection finds no backend before a window is
    // tried, so it is the no-backend exit.
    let (code, stderr) = identify_with(&[
        ("CRCBL_SHELL", Some("wayland")),
        ("WAYLAND_DISPLAY", None),
        ("DISPLAY", None),
    ]);
    assert_eq!(code, Some(6), "{stderr}");
    assert!(stderr.contains("no backend"), "{stderr}");
    // crcbl's headless shell would show the panels to no one and report
    // them shown: refused as no GUI session.
    let (code, stderr) = identify_with(&[("CRCBL_SHELL", Some("headless"))]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("needs a GUI session"), "{stderr}");
    assert!(stderr.contains("headless"), "{stderr}");
    // And the GUI, which would otherwise run there unseen until killed.
    let mut gui = Running(
        Command::new(dsplr_bin())
            .env("CRCBL_SHELL", "headless")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + STEP;
    let status = loop {
        if let Some(status) = gui.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the GUI ran on the headless shell"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(1), "the GUI on the headless shell");
}

/// `dsplr identify`'s exit code and stderr, run with each variable set
/// (`Some`) or removed (`None`).
fn identify_with(vars: &[(&str, Option<&str>)]) -> (Option<i32>, String) {
    let mut command = Command::new(dsplr_bin());
    command.arg("identify");
    for (name, value) in vars {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let output = command.output().unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
