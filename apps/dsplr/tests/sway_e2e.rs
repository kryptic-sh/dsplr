//! `dsplr backend`, `dsplr apply` (with and without `--dry-run`),
//! `dsplr set`, `dsplr save`, the profile store (`dsplr save --profile`,
//! `dsplr restore`, `dsplr profile`), `dsplr watch` and
//! `dsplr restore --revert` against a real, headless sway, whose backend has
//! a real test.
//!
//! Run through the harness, which starts that sway where it cannot reach the
//! desktop session and gives dsplr a private state and config directory:
//!
//! ```text
//! tests/e2e/headless-sway.sh cargo test -p dsplr --test sway_e2e -- --ignored
//! ```
//!
//! The confirm prompt is the same on every backend and is covered by
//! `hyprland_e2e`; here every change is kept with `--yes`. `dsplr save`'s own
//! question is asked on a terminal `script` (util-linux) gives it.
//! `DSPLR_BIN` names the dsplr to run when it is not where Cargo built it (in
//! CI's VM).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use dsplr_backend::{Backend, Named};
use std::time::SystemTime;

use dsplr_model::{
    AdaptiveSync, Layout, Mode, OutputEntry, OutputId, OutputState, RgbRange, Transform,
};
use dsplr_persist::managed::{Comment, Ownership, ownership};
use dsplr_persist::profile::{self, Profile, unix_ms};
use dsplr_persist::revert;

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/sway_support.rs"]
mod sway_support;

use support::{answered_on_a_terminal, edited, entry, entry_mut, on_a_terminal, plain};
use sway_support::{
    FreshSway, config_home, harness_runtime, no_outputs_file, outputs_up_to, reload_until,
    sway_config, swaymsg,
};

/// The exit code of a usage error (`Exit::Usage`, clap's own).
const USAGE: i32 = 2;

/// The exit code of a layout validation refused (`Exit::Refused`).
const REFUSED: i32 = 3;

/// The exit code of a layout the desktop refused (`Exit::BackendRefused`).
const BACKEND_REFUSED: i32 = 4;

/// The exit code of `dsplr restore` finding no profile (`Exit::NoProfile`).
const NO_PROFILE: i32 = 7;

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

/// `layout` written as a layout file in the harness's runtime.
fn layout_file(runtime: &Path, name: &str, layout: &Layout) -> String {
    let path = runtime.join(name);
    std::fs::write(&path, layout.to_json().expect("JSON")).expect("layout file written");
    path.to_str().expect("a UTF-8 path").to_owned()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless sway harness: tests/e2e/headless-sway.sh"]
async fn dsplr_applies_sets_and_reverts_on_sway() {
    let runtime = harness_runtime();
    // The harness starts HEADLESS-1; this test also needs HEADLESS-2.
    outputs_up_to(2).await;
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    // Another test binary in the same harness may have left the outputs
    // otherwise: start from the harness's own layout, any output past these
    // two off.
    let known = edited(&backend.snapshot().await.expect("snapshot"), |layout| {
        for entry in &mut layout.entries {
            entry.state = match entry.connector() {
                Some("HEADLESS-1") => OutputState {
                    mode: Some(Mode::new(1280, 720, 0)),
                    ..OutputState::default()
                },
                Some("HEADLESS-2") => OutputState {
                    mode: Some(Mode::new(1920, 1080, 0)),
                    x: 1280,
                    ..OutputState::default()
                },
                _ => OutputState {
                    enabled: false,
                    ..OutputState::default()
                },
            };
        }
    });
    backend.apply(&known).await.expect("sway applies it");
    // One sway serves them all, so they run in turn rather than as tests
    // racing over its outputs. Each leaves the layout as it found it; saving
    // goes last, as a reload puts back whatever the saved file says.
    apply_tests_on_a_dry_run_and_applies_and_reverts(&backend, &runtime).await;
    set_edits_one_output(&backend).await;
    profiles_are_saved_picked_and_restored(&backend).await;
    kanshi_profiles_are_imported_and_restored(&backend).await;
    watch_restores_profiles_as_outputs_come_and_go(&runtime).await;
    backend_flag_uses_the_one_named_or_says_why_not(&backend).await;
    save_writes_outputs_and_asks_before_replacing_one(&backend).await;
}

async fn apply_tests_on_a_dry_run_and_applies_and_reverts(backend: &Backend, runtime: &Path) {
    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("backend: wlr\n"), "{text}");
    assert!(text.contains("test before apply"), "{text}");
    assert!(text.contains("native persistence"), "{text}");
    assert!(!text.contains("mirroring"), "{text}");

    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };

    // A dry run has sway test the layout, and changes nothing.
    let moved = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.y = 200;
    });
    let file = layout_file(runtime, "moved.json", &moved);
    let (code, text) = dsplr(&["apply", "--dry-run", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("wlr tested the layout"), "{text}");
    assert_eq!(snapshot().await, start);

    // One sway refuses fails its test, on a dry run and before a real apply,
    // and changes nothing. It is the rightmost output, so the wide mode
    // overlaps nothing and passes validation.
    let unallocatable = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mode = Some(Mode::new(2_000_000_000, 1, 0));
    });
    let file_refused = layout_file(runtime, "refused.json", &unallocatable);
    for args in [
        ["apply", "--dry-run", &file_refused],
        ["apply", "--yes", &file_refused],
    ] {
        let (code, text) = dsplr(&args);
        assert_eq!(code, Some(BACKEND_REFUSED), "{args:?}: {text}");
        // The test refused it, not an apply that was then reverted: the
        // wording is the test step's.
        assert!(
            text.contains("wlr refused the layout in its test")
                && text.contains("refused the configuration"),
            "{args:?}: {text}"
        );
    }
    assert_eq!(snapshot().await, start);
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    assert!(
        revert_file.starts_with(runtime),
        "{}",
        revert_file.display()
    );
    assert!(!revert_file.exists(), "a failed test starts no change");

    // Mirroring is refused by validation, before sway is asked.
    let one = entry(&start, "HEADLESS-1").id.clone();
    let mirrored = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.mirror_of = Some(one);
    });
    let file_mirrored = layout_file(runtime, "mirrored.json", &mirrored);
    let (code, text) = dsplr(&["apply", "--dry-run", &file_mirrored]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("cannot mirror"), "{text}");

    // What validation passes but the protocol cannot express is refused by
    // dsplr at the test step, before sway is asked or a change starts: a
    // refusal of dsplr's, not the desktop's.
    let automatic = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.adaptive_sync = Some(AdaptiveSync::Automatic);
    });
    let file_automatic = layout_file(runtime, "automatic.json", &automatic);
    for args in [
        ["apply", "--dry-run", &file_automatic],
        ["apply", "--yes", &file_automatic],
    ] {
        let (code, text) = dsplr(&args);
        assert_eq!(code, Some(REFUSED), "{args:?}: {text}");
        assert!(
            text.contains("wlr cannot take the layout")
                && text.contains("automatic adaptive sync cannot be set"),
            "{args:?}: {text}"
        );
    }
    assert_eq!(snapshot().await, start);
    assert!(
        !revert_file.exists(),
        "a refusal before sending starts no change"
    );

    // --yes applies and keeps it; a revert by hand puts the old layout back.
    let (code, text) = dsplr(&["apply", "--yes", &file]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(entry(&snapshot().await, "HEADLESS-2").state.y, 200);
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a revert removes the revert point");
}

/// Puts the layout before the last change back by hand, and checks it is
/// `start` again.
async fn revert_to(backend: &Backend, start: &Layout) {
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(backend.snapshot().await.expect("snapshot"), *start);
}

async fn set_edits_one_output(backend: &Backend) {
    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };
    let two = || entry(&start, "HEADLESS-2");
    let (x, y) = (two().state.x, two().state.y);
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");

    // A name for no output, or for both, is a usage error listing them.
    let (code, text) = dsplr(&["set", "HEADLESS-9", "--pos", "0,0"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("names no output") && text.contains("  HEADLESS-1: "),
        "{text}"
    );
    let one_description = entry(&start, "HEADLESS-1").description().expect("one");
    let two_description = two().description().expect("one");
    let shared: String = one_description
        .chars()
        .zip(two_description.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect();
    assert!(
        !shared.trim().is_empty(),
        "{one_description} {two_description}"
    );
    let (code, text) = dsplr(&["set", &shared, "--pos", "0,0"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(text.contains("names more than one output"), "{text}");

    // Named by its description, on a dry run: sway tests it, nothing changes.
    let moved = (x, y + 200);
    let pos = format!("{},{}", moved.0, moved.1);
    let (code, text) = dsplr(&["set", &two_description, "--pos", &pos, "--dry-run"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("wlr tested the layout"), "{text}");
    // It says which output a description found.
    let setting_two = format!("dsplr: setting HEADLESS-2: {two_description}\n");
    assert!(text.contains(&setting_two), "{text}");
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a dry run starts no change");

    // Moved, and only that.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--pos", &pos, "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains(&setting_two), "{text}");
    let expected = edited(&start, |layout| {
        let state = &mut entry_mut(layout, "HEADLESS-2").state;
        (state.x, state.y) = moved;
    });
    assert_eq!(snapshot().await, expected);
    revert_to(backend, &start).await;

    // A custom mode (headless sway lists only the current one) needs its
    // refresh; with one it is applied.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mode", "1600x900", "--dry-run"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("give the refresh"), "{text}");
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mode", "1600x900@60", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let mode = entry(&snapshot().await, "HEADLESS-2").state.mode.clone();
    assert!(
        mode.as_ref()
            .is_some_and(|mode| mode.matches(&Mode::new(1600, 900, 60_000))),
        "{mode:?}"
    );
    revert_to(backend, &start).await;

    let (code, text) = dsplr(&["set", "HEADLESS-2", "--scale", "2", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let scale = entry(&snapshot().await, "HEADLESS-2").state.scale;
    assert!((scale - 2.0).abs() < f64::EPSILON, "{scale}");
    revert_to(backend, &start).await;

    // 1.1 is no 256th, so it travels as 282/256 and reads back as that: what
    // the wire holds, so not reported as not taken.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--scale", "1.1", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("scale asked"), "{text}");
    let scale = entry(&snapshot().await, "HEADLESS-2").state.scale;
    assert!((scale - 282.0 / 256.0).abs() < f64::EPSILON, "{scale}");
    revert_to(backend, &start).await;

    // dsplr's 90 is wl_output's, which sway reports as its 270, as --help
    // says.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--transform", "90", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(
        entry(&snapshot().await, "HEADLESS-2").state.transform,
        Transform::Rotate90
    );
    let outputs: String = swaymsg(&["-t", "get_outputs"])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(outputs.contains("\"transform\":\"270\""), "{outputs}");
    revert_to(backend, &start).await;

    // What the desktop cannot do is refused before it is asked.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--vrr", "automatic", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(
        text.contains("cannot set adaptive sync automatic"),
        "{text}"
    );
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--mirror", "HEADLESS-1", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("cannot mirror"), "{text}");
    for mirror in ["HEADLESS-9", "HEADLESS-2"] {
        let (code, text) = dsplr(&["set", "HEADLESS-2", "--mirror", mirror, "--yes"]);
        assert_eq!(code, Some(USAGE), "{mirror}: {text}");
    }
    assert_eq!(snapshot().await, start);
    assert!(!revert_file.exists(), "a refusal starts no change");

    // sway mirrors nothing, so --no-mirror asks for what is already so: like
    // any setting at its current value, it goes through and changes nothing.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--no-mirror", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
    revert_to(backend, &start).await;

    // Turned off, and on again where it was. Headless sway lists no mode for
    // an output that is off, so turning it on needs one.
    let (code, text) = dsplr(&["set", "HEADLESS-1", "--disable", "--scale", "2", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("--scale cannot be changed together with --disable"),
        "{text}"
    );
    assert_eq!(snapshot().await, start);
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--disable", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let off = snapshot().await;
    assert!(!entry(&off, "HEADLESS-2").state.enabled);
    // A setting for an output that stays off would be dropped unseen.
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--pos", "0,0", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("it is off; add --enable to change --pos"),
        "{text}"
    );
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--no-mirror", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(
        text.contains("it is off; add --enable to change --no-mirror"),
        "{text}"
    );
    assert_eq!(snapshot().await, off);
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--enable", "--dry-run"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("enabled without a mode"), "{text}");
    let start_mode = two().state.mode.clone().expect("enabled");
    let mode = format!("{}x{}@0", start_mode.width, start_mode.height);
    let pos = format!("{x},{y}");
    let (code, text) = dsplr(&[
        "set",
        "HEADLESS-2",
        "--enable",
        "--mode",
        &mode,
        "--pos",
        &pos,
        "--yes",
    ]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
}

async fn profiles_are_saved_picked_and_restored(backend: &Backend) {
    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };
    let store = profile::default_dir().expect("XDG_CONFIG_HOME is set by the harness");
    assert!(store.starts_with(harness_runtime()), "{}", store.display());
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    let revert_point = || revert::load(&revert_file).expect("a revert point or none");
    let last_change = revert_point();

    let (code, text) = dsplr(&["profile", "list"]);
    assert_eq!((code, &*text), (Some(0), "no profiles\n"));
    let (code, text) = dsplr(&["restore", "--yes"]);
    assert_eq!(code, Some(NO_PROFILE), "{text}");
    assert!(text.contains("there are no profiles"), "{text}");

    // A layout file put in the store by hand is a profile too, here one for
    // an output that is not connected: never picked, and refused by name.
    let elsewhere = Layout {
        entries: vec![OutputEntry {
            id: OutputId::Connector("HEADLESS-9".into()),
            output: None,
            state: OutputState {
                mode: Some(Mode::new(1280, 720, 0)),
                ..OutputState::default()
            },
        }],
        ..Default::default()
    };
    std::fs::create_dir_all(&store).expect("store made");
    std::fs::write(
        store.join("elsewhere.json"),
        elsewhere.to_json().expect("JSON"),
    )
    .expect("profile written");
    let (code, text) = dsplr(&["restore", "--yes"]);
    assert_eq!(code, Some(NO_PROFILE), "{text}");
    // The outputs in the order sway lists them.
    let line = text
        .lines()
        .find(|line| line.contains("no profile is for the outputs connected now ("))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.contains("HEADLESS-1")
            && line.contains("HEADLESS-2")
            && line.contains("nothing changed"),
        "{text}"
    );
    let (code, text) = dsplr(&["restore", "elsewhere", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("HEADLESS-9: not connected"), "{text}");
    let (code, text) = dsplr(&["restore", "nowhere", "--yes"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(text.contains("no profile named \"nowhere\""), "{text}");
    assert_eq!(snapshot().await, start);
    assert_eq!(
        revert_point(),
        last_change,
        "a refused restore starts no change"
    );

    // Profiles saved on Hyprland and KDE carry settings sway has not: at
    // their neutral values (srgb, 8 bits, no overscan, an automatic RGB
    // range) they apply here, and a real request (a wide colour preset) is
    // refused. Both removed after, so they are never picked.
    let neutral = edited(&start, |layout| {
        for entry in &mut layout.entries {
            let state = &mut entry.state;
            if state.enabled {
                state.colour_preset = Some("srgb".into());
                state.bit_depth = Some(8);
                state.overscan = Some(0);
                state.rgb_range = Some(RgbRange::Automatic);
            }
        }
    });
    let wide = edited(&neutral, |layout| {
        entry_mut(layout, "HEADLESS-1").state.colour_preset = Some("wide".into());
    });
    for (name, layout) in [("from-elsewhere", &neutral), ("wide", &wide)] {
        std::fs::write(
            store.join(format!("{name}.json")),
            layout.to_json().expect("JSON"),
        )
        .expect("profile written");
    }
    let (code, text) = dsplr(&["restore", "from-elsewhere", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, start);
    let (code, text) = dsplr(&["restore", "wide", "--yes"]);
    assert_eq!(code, Some(REFUSED), "{text}");
    assert!(text.contains("colour preset"), "{text}");
    assert_eq!(snapshot().await, start);
    for name in ["from-elsewhere", "wide"] {
        let (code, text) = dsplr(&["profile", "rm", name]);
        assert_eq!(code, Some(0), "{text}");
    }

    // Saved on any wlroots compositor, not only sway: here with no SWAYSOCK.
    let (code, text) = exit_and_output(
        &Command::new(dsplr_bin())
            .args(["save", "--profile", "desk"])
            .env_remove("SWAYSOCK")
            .stdin(Stdio::null())
            .output()
            .expect("dsplr runs"),
    );
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("saved profile \"desk\" to "), "{text}");
    let (code, text) = dsplr(&["profile", "list"]);
    assert_eq!(code, Some(0), "{text}");
    let desk = text.lines().next().unwrap_or_default();
    assert!(
        desk.starts_with("desk: ")
            && desk.contains("HEADLESS-1")
            && desk.contains("HEADLESS-2")
            && desk.ends_with(" (saved just now)")
            && text.contains("\nelsewhere: HEADLESS-9 (saved "),
        "{text}"
    );
    // Shown as `dsplr list` shows the displays it was saved from, and as
    // the JSON `dsplr list --json` and `dsplr apply` use.
    let (code, listed) = dsplr(&["list"]);
    assert_eq!(code, Some(0), "{listed}");
    let (code, shown) = dsplr(&["profile", "show", "desk"]);
    assert_eq!((code, &shown), (Some(0), &listed));
    let (_, listed) = dsplr(&["list", "--json"]);
    let (code, shown) = dsplr(&["profile", "show", "desk", "--json"]);
    assert_eq!(code, Some(0), "{shown}");
    assert_eq!(
        Layout::from_json(&shown).expect("a layout"),
        Layout::from_json(&listed).expect("a layout")
    );

    // A profile of that name is replaced only when asked to.
    let (code, text) = dsplr(&["save", "--profile", "desk"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("there is a profile named \"desk\" already") && text.contains("no terminal"),
        "{text}"
    );
    let (code, text) = dsplr(&["save", "--profile", "desk", "--yes"]);
    assert_eq!(code, Some(0), "{text}");

    // A link in the store is read, but saving over it is refused before
    // anything is shown or asked.
    let target = harness_runtime().join("linked.json");
    std::fs::write(&target, start.to_json().expect("JSON")).expect("written");
    let link = store.join("linked.json");
    std::os::unix::fs::symlink(&target, &link).expect("linked");
    let (code, text) = dsplr(&["save", "--profile", "linked"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("is a link to")
            && !text.contains("what saving changes")
            && !text.contains("no terminal"),
        "{text}"
    );
    std::fs::remove_file(&link).expect("unlinked");

    // A file in the store whose name no profile can have is said, and
    // passed over.
    let stray = store.join(".stray.json");
    std::fs::write(&stray, "{}").expect("written");
    let (code, text) = dsplr(&["profile", "list"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains(".stray.json is passed over: its name is not a profile's"),
        "{text}"
    );
    std::fs::remove_file(&stray).expect("removed");

    // Moved, then put back by the profile picked for these outputs.
    let two = entry(&start, "HEADLESS-2");
    let pos = format!("{},{}", two.state.x, two.state.y + 300);
    let (code, text) = dsplr(&["set", "HEADLESS-2", "--pos", &pos, "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let moved = snapshot().await;
    assert_ne!(moved, start);
    // Unattended, as from an autostart, it takes --yes to move a screen.
    let (code, text) = dsplr(&["restore"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("no terminal to confirm it on; pass --yes"),
        "{text}"
    );
    assert_eq!(snapshot().await, moved);
    let (code, text) = dsplr(&["restore", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("restoring profile \"desk\", saved for the outputs connected now"),
        "{text}"
    );
    assert_eq!(snapshot().await, start);
    // A restore is a change like any other, with a revert point.
    let (code, text) = dsplr(&["restore", "--revert"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(snapshot().await, moved);
    let (code, text) = dsplr(&["restore", "desk", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("restoring profile \"desk\"\n"), "{text}");
    assert_eq!(snapshot().await, start);
    // Restoring what is on screen moves no screen, so even unattended it
    // takes no --yes.
    let (code, text) = dsplr(&["restore"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("restoring profile \"desk\""), "{text}");
    assert_eq!(snapshot().await, start);

    // A newer profile for only some of the outputs loses to the exact one,
    // and is picked once that is gone, leaving the others as they are.
    let one = Profile {
        name: "one".parse().expect("a name"),
        layout: edited(&start, |layout| {
            layout
                .entries
                .retain(|entry| entry.connector() == Some("HEADLESS-1"));
            entry_mut(layout, "HEADLESS-1").state.transform = Transform::Rotate180;
        }),
        saved_unix_ms: unix_ms(SystemTime::now()) + 60_000,
    };
    profile::save(&store, &one).expect("profile saved");
    let (code, text) = dsplr(&["restore", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("restoring profile \"desk\""), "{text}");
    let (code, text) = dsplr(&["profile", "rm", "desk"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["profile", "rm", "desk"]);
    assert_eq!(code, Some(USAGE), "{text}");
    let (code, text) = dsplr(&["restore", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("restoring profile \"one\", saved for HEADLESS-1 of the outputs"),
        "{text}"
    );
    let expected = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-1").state.transform = Transform::Rotate180;
    });
    assert_eq!(snapshot().await, expected);
    revert_to(backend, &start).await;
    for name in ["one", "elsewhere"] {
        let (code, text) = dsplr(&["profile", "rm", name]);
        assert_eq!(code, Some(0), "{name}: {text}");
    }

    // A description naming both outputs, listed before a connector naming
    // one of them, gets the other, picked or named, wherever sway lists
    // them. Any other output is named by its connector, as it is.
    let words = |name: &str| {
        entry(&start, name)
            .description()
            .expect("a description")
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let shared: Vec<String> = words("HEADLESS-1")
        .into_iter()
        .zip(words("HEADLESS-2"))
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect();
    let described = OutputId::Description(shared.join(" "));
    for name in ["HEADLESS-1", "HEADLESS-2"] {
        assert!(described.matches(entry(&start, name)), "{described} {name}");
    }
    let mut entries = vec![OutputEntry {
        id: described,
        output: None,
        state: OutputState {
            transform: Transform::Rotate180,
            ..entry(&start, "HEADLESS-1").state.clone()
        },
    }];
    entries.extend(
        start
            .entries
            .iter()
            .filter(|live| live.connector() != Some("HEADLESS-1"))
            .map(|live| OutputEntry {
                id: OutputId::Connector(live.connector().expect("a connector").to_owned()),
                output: None,
                state: live.state.clone(),
            }),
    );
    let both = Profile {
        name: "both".parse().expect("a name"),
        layout: Layout {
            entries,
            ..Default::default()
        },
        saved_unix_ms: unix_ms(SystemTime::now()),
    };
    profile::save(&store, &both).expect("profile saved");
    let rotated = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-1").state.transform = Transform::Rotate180;
    });
    for args in [&["restore", "--yes"][..], &["restore", "both", "--yes"]] {
        let (code, text) = dsplr(args);
        assert_eq!(code, Some(0), "{args:?}: {text}");
        assert!(text.contains("restoring profile \"both\""), "{text}");
        assert_eq!(snapshot().await, rotated, "{args:?}");
        revert_to(backend, &start).await;
    }
    let (code, text) = dsplr(&["profile", "rm", "both"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["profile", "list"]);
    assert_eq!((code, &*text), (Some(0), "no profiles\n"));
}

/// How long `dsplr watch` gets to restore a profile after a hotplug: its
/// quiet time, the apply and the apply's settle, with room to spare.
const WATCH_RESTORE: Duration = Duration::from_secs(20);

/// How long `dsplr watch` is given to do something it must not: several
/// times its quiet time.
const WATCH_IDLE: Duration = Duration::from_secs(4);

/// `dsplr` with `args`, to be run against `sway` rather than the harness's.
fn dsplr_on(sway: &FreshSway, args: &[&str]) -> Command {
    let mut command = Command::new(dsplr_bin());
    command
        .args(args)
        .env("WAYLAND_DISPLAY", &sway.wayland)
        .env("SWAYSOCK", &sway.swaysock)
        .stdin(Stdio::null());
    command
}

/// A running `dsplr watch`, its stderr in a log file; killed if the test
/// ends before it stops it.
struct Watcher {
    child: Child,
    log: PathBuf,
}

impl Watcher {
    fn start(sway: &FreshSway, log: PathBuf) -> Self {
        let file = std::fs::File::create(&log).expect("its log");
        let child = dsplr_on(sway, &["watch"])
            .stdout(Stdio::null())
            .stderr(file)
            .spawn()
            .expect("dsplr watch runs");
        Self { child, log }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).expect("its log")
    }

    /// How many lines of its log say a profile is being restored: each
    /// one an apply.
    fn restores(&self) -> usize {
        self.log()
            .lines()
            .filter(|line| line.starts_with("dsplr: restoring profile"))
            .count()
    }

    /// Waits until the log has `count` restores, the last of `name`, and
    /// `dsplr list --json` shows a layout `shown` accepts; returns it.
    async fn restored(
        &self,
        sway: &FreshSway,
        count: usize,
        name: &str,
        shown: impl Fn(&Layout) -> bool,
    ) -> Layout {
        let deadline = Instant::now() + WATCH_RESTORE;
        loop {
            let log = self.log();
            let last = log
                .lines()
                .rfind(|line| line.starts_with("dsplr: restoring profile"));
            let layout = listed(sway);
            if self.restores() == count
                && last.is_some_and(|line| line.contains(&format!("\"{name}\"")))
                && shown(&layout)
            {
                return layout;
            }
            assert!(
                Instant::now() < deadline,
                "restore {count} of \"{name}\" not seen: {layout:#?}\n{log}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Waits until the log has a line containing `text`.
    async fn said(&self, text: &str) {
        let deadline = Instant::now() + WATCH_RESTORE;
        while !self.log().contains(text) {
            assert!(
                Instant::now() < deadline,
                "{text:?} not said:\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Sends it SIGTERM.
    fn terminate(&self) {
        let pid = self.child.id().to_string();
        let status = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("kill runs");
        assert!(status.success());
    }

    /// Whether it is still running.
    fn running(&mut self) -> bool {
        self.child.try_wait().expect("its status").is_none()
    }

    /// Sends SIGTERM and checks it stops cleanly; returns its log.
    fn stop(self) -> String {
        self.terminate();
        self.ended()
    }

    /// Waits for it to end, and checks it ended cleanly on SIGTERM; returns
    /// its log.
    fn ended(mut self) -> String {
        let status = self.child.wait().expect("dsplr watch ends");
        let log = self.log();
        assert_eq!(status.code(), Some(0), "{log}");
        assert!(log.ends_with("dsplr: SIGTERM; stopped\n"), "{log}");
        log
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Stopped already, or a failed test's: either way it is reaped.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The revert point, which must be there.
fn revert_point() -> revert::RevertPoint {
    any_revert_point().expect("a revert point")
}

/// The revert point, if there is one.
fn any_revert_point() -> Option<revert::RevertPoint> {
    let path = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    revert::load(&path).expect("a readable revert point")
}

/// Waits until the revert point is a kept change's, made since it held
/// `token` (or since there was none): that change's watchdog stood down.
/// Returns it.
async fn kept_since(token: Option<&str>) -> revert::RevertPoint {
    let deadline = Instant::now() + WATCH_RESTORE;
    loop {
        let point = any_revert_point();
        if let Some(point) = &point
            && Some(point.token.as_str()) != token
            && point.guarded_until_unix_ms.is_none()
        {
            return point.clone();
        }
        assert!(Instant::now() < deadline, "not kept: {point:#?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The layout `dsplr list --json` prints for `sway`.
fn listed(sway: &FreshSway) -> Layout {
    let output = dsplr_on(sway, &["list", "--json"])
        .output()
        .expect("dsplr runs");
    assert!(output.status.success(), "{output:?}");
    Layout::from_json(&String::from_utf8_lossy(&output.stdout)).expect("its JSON")
}

/// The connector sway gave the output plugged in last: in `now` and not in
/// `before`.
fn plugged(before: &Layout, now: &Layout) -> String {
    support::connectors(now)
        .into_iter()
        .find(|name| {
            before
                .entries
                .iter()
                .all(|old| old.connector() != Some(name))
        })
        .unwrap_or_else(|| panic!("no new output: {now:#?}"))
}

/// `dsplr watch` with two profiles, one for two outputs and one for those
/// and one more: each output plugged in and unplugged
/// (`swaymsg create_output`, `swaymsg output NAME unplug`) restores the
/// profile picked for the outputs then connected, once, and nothing else
/// restores one: not its own applies, not a change made by hand, not a set
/// of outputs no profile is for. Each restore is kept, so its watchdog
/// stands down; while another change waits to be kept, it waits; SIGTERM
/// during a restore lets it finish. On a sway of its own, since sway numbers
/// each new output afresh and other tests name the harness's.
async fn watch_restores_profiles_as_outputs_come_and_go(runtime: &Path) {
    let sway = FreshSway::start("watch", 2).await;
    let start = sway.wlr.snapshot().await.expect("snapshot");
    let store = profile::default_dir().expect("XDG_CONFIG_HOME is set by the harness");
    let y2 = entry(&start, "HEADLESS-2").state.y;
    let saved = |name: &str, layout: Layout| {
        let profile = Profile {
            name: name.parse().expect("a name"),
            layout,
            saved_unix_ms: unix_ms(SystemTime::now()),
        };
        profile::save(&store, &profile).expect("profile saved");
    };
    // For the two outputs: HEADLESS-2 lower than it is.
    let undocked = edited(&start, |layout| {
        entry_mut(layout, "HEADLESS-2").state.y = y2 + 200;
    });
    saved("undocked", undocked);
    // For those and one more, which a headless output's description names
    // whatever its connector. Off to the right, apart from the others.
    let docked_x = 10_000;
    let docked = edited(&start, |layout| {
        layout.entries.push(OutputEntry {
            id: OutputId::Description("Headless output".into()),
            output: None,
            state: OutputState {
                mode: Some(Mode::new(1920, 1080, 0)),
                x: docked_x,
                ..OutputState::default()
            },
        });
    });
    saved("docked", docked);
    let count = start.entries.len();
    let y = |layout: &Layout| entry(layout, "HEADLESS-2").state.y;

    // The outputs connected at the start count as a change.
    let earlier = any_revert_point().map(|point| point.token);
    let watcher = Watcher::start(&sway, runtime.join("watch.log"));
    let undocked_shown = |layout: &Layout| layout.entries.len() == count && y(layout) == y2 + 200;
    let before = watcher.restored(&sway, 1, "undocked", undocked_shown).await;
    watcher.said("saved for the outputs connected now").await;
    // Kept, from the layout before it: its watchdog stands down.
    assert_eq!(
        kept_since(earlier.as_deref())
            .await
            .layout
            .to_json()
            .expect("JSON"),
        start.to_json().expect("JSON"),
        "from the layout before it"
    );

    // Plugged in: the profile for all three, the new output where it says.
    sway.swaymsg(&["create_output"]);
    let docked_shown = |layout: &Layout| {
        layout.entries.len() == count + 1
            && y(layout) == y2
            && layout
                .entries
                .iter()
                .any(|entry| entry.state.x == docked_x && entry.state.enabled)
    };
    let now = watcher.restored(&sway, 2, "docked", docked_shown).await;
    let new = plugged(&before, &now);

    // Unplugged: back to the profile for the outputs left.
    sway.swaymsg(&["output", &new, "unplug"]);
    watcher.restored(&sway, 3, "undocked", undocked_shown).await;

    // Its own applies, and a change made by hand, leave the connected
    // outputs as they were: no restore, so no loop, and the change stays.
    let x2 = entry(&start, "HEADLESS-2").state.x.to_string();
    sway.swaymsg(&[
        "output",
        "HEADLESS-2",
        "position",
        &x2,
        &(y2 + 400).to_string(),
    ]);
    tokio::time::sleep(WATCH_IDLE).await;
    assert_eq!(watcher.restores(), 3, "{}", watcher.log());
    assert_eq!(y(&listed(&sway)), y2 + 400);

    // Plugged in while another change waits to be kept (a `dsplr apply` at
    // its prompt, here its revert point written by hand): it does not
    // replace that change's revert point, and restores once it is kept.
    let revert_file = revert::default_path().expect("XDG_STATE_HOME is set by the harness");
    let moved = listed(&sway);
    let waiting =
        revert::RevertPoint::guarding(moved.clone(), unix_ms(SystemTime::now()) + 600_000);
    revert::save(&revert_file, &waiting).expect("revert point written");
    sway.swaymsg(&["create_output"]);
    watcher.said("while another change waits to be kept").await;
    tokio::time::sleep(WATCH_IDLE).await;
    assert_eq!(watcher.restores(), 3, "{}", watcher.log());
    assert_eq!(revert_point(), waiting, "left to its own change");
    assert!(revert::keep(&revert_file, &waiting.token).expect("kept"));
    let kept = revert_point();
    let now = watcher.restored(&sway, 4, "docked", docked_shown).await;
    let new = plugged(&before, &now);
    let restored = kept_since(Some(&kept.token)).await;
    assert_eq!(
        y(&restored.layout),
        y2 + 400,
        "from the layout moved by hand"
    );

    // With no profile for the outputs connected, it says so and changes
    // nothing.
    let (code, text) = dsplr(&["profile", "rm", "undocked"]);
    assert_eq!(code, Some(0), "{text}");
    sway.swaymsg(&["output", &new, "unplug"]);
    watcher
        .said("no profile is for the outputs connected now (")
        .await;
    tokio::time::sleep(WATCH_IDLE).await;
    assert_eq!(watcher.restores(), 4, "{}", watcher.log());
    let left = listed(&sway);
    assert_eq!(left.entries.len(), count);
    assert_eq!(y(&left), y2, "as the last restore left it");
    watcher.stop();

    // Started with its profile on screen, it applies nothing.
    let (code, text) = dsplr(&["profile", "rm", "docked"]);
    assert_eq!(code, Some(0), "{text}");
    saved("start", start.clone());
    sway.wlr.apply(&start).await.expect("sway applies it");
    let watcher = Watcher::start(&sway, runtime.join("watch-again.log"));
    watcher.said("profile \"start\" is on screen already").await;
    let log = watcher.stop();
    assert!(!log.contains("dsplr: restoring profile"), "{log}");
    let (code, text) = dsplr(&["profile", "rm", "start"]);
    assert_eq!(code, Some(0), "{text}");

    // SIGTERM during a restore: it finishes the restore, keeps it, and then
    // stops. Adaptive sync, which a headless output takes and stays off
    // with, never reads back as asked, so the restore's settle runs to its
    // end, which leaves time to send the signal during it.
    saved(
        "slow",
        edited(&start, |layout| {
            entry_mut(layout, "HEADLESS-1").state.adaptive_sync = Some(AdaptiveSync::On);
        }),
    );
    let earlier = any_revert_point().map(|point| point.token);
    let mut watcher = Watcher::start(&sway, runtime.join("watch-stopped.log"));
    watcher.said("dsplr: restoring profile \"slow\"").await;
    watcher.terminate();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(watcher.running(), "stopped mid-restore:\n{}", watcher.log());
    let log = watcher.ended();
    let restoring = log.find("restoring profile \"slow\"").expect("restoring");
    assert!(
        log[restoring..].lines().count() > 2,
        "the restore's own lines come before it stops:\n{log}"
    );
    assert_eq!(
        kept_since(earlier.as_deref())
            .await
            .layout
            .to_json()
            .expect("JSON"),
        start.to_json().expect("JSON"),
        "from the layout before it"
    );
    let (code, text) = dsplr(&["profile", "rm", "slow"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["profile", "list"]);
    assert_eq!((code, &*text), (Some(0), "no profiles\n"));
}

/// `--backend`: the one named is the only one checked, never another in its
/// place, and a change made through it is reverted through it too.
async fn backend_flag_uses_the_one_named_or_says_why_not(backend: &Backend) {
    // Not a backend's name: a usage error.
    let (code, text) = dsplr(&["--backend", "sway", "list"]);
    assert_eq!(code, Some(USAGE), "{text}");
    assert!(text.contains("wlr"), "{text}");

    // Each other backend: not here, and wlroots, which is, never checked.
    for (name, check) in [
        ("gnome", "GNOME:"),
        ("kde", "KDE:"),
        ("hyprland", "Hyprland:"),
        ("x11", "X11: not written yet"),
    ] {
        let (code, text) = dsplr(&["--backend", name, "list"]);
        assert_eq!(code, Some(6), "{name}: {text}");
        let named = Named::from_name(name).expect("a backend's name").name();
        assert!(
            text.contains(&format!("the {named} backend is not usable here")),
            "{name}: {text}"
        );
        assert!(text.contains(check), "{name}: {text}");
        assert!(!text.contains("wlroots"), "{name}: {text}");
    }
    let (code, text) = dsplr(&["backend", "--backend", "gnome"]);
    assert_eq!(code, Some(6), "{text}");
    assert!(!text.contains("wlroots"), "{text}");

    // wlr, the one here: everything works through it.
    let (code, text) = dsplr(&["--backend", "wlr", "backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("backend: wlr\n"), "{text}");
    let start = backend.snapshot().await.expect("snapshot");
    let (x, y) = {
        let two = entry(&start, "HEADLESS-2");
        (two.state.x, two.state.y + 100)
    };
    let pos = format!("{x},{y}");
    let (code, text) = dsplr(&[
        "--backend",
        "wlr",
        "set",
        "HEADLESS-2",
        "--pos",
        &pos,
        "--yes",
    ]);
    assert_eq!(code, Some(0), "{text}");
    let moved = backend.snapshot().await.expect("snapshot");
    assert_eq!(entry(&moved, "HEADLESS-2").state.y, y);
    let (code, text) = dsplr(&["restore", "--revert", "--backend", "wlr"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(backend.snapshot().await.expect("snapshot"), start);

    // A change under `--backend wlr` whose dsplr is killed at the prompt:
    // its watchdog was started with the flag, and reverts it.
    let pid_file = harness_runtime().join("dsplr-set.pid");
    let mut terminal = on_a_terminal(&format!(
        "echo $$ > {pid}; exec {dsplr} --backend wlr set HEADLESS-2 --pos {pos} \
         --confirm-timeout {secs}",
        pid = plain(&pid_file),
        dsplr = plain(&dsplr_bin()),
        secs = KILLED_CONFIRM.as_secs(),
    ));
    let started = Instant::now();
    while entry(&backend.snapshot().await.expect("snapshot"), "HEADLESS-2")
        .state
        .y
        != y
    {
        assert!(
            started.elapsed() < KILLED_CONFIRM,
            "the change never applied"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let token = revert_point().token;
    let watchdogs = watchdog_command_lines(&token);
    assert_eq!(watchdogs.len(), 1, "{watchdogs:?}");
    assert!(watchdogs[0].ends_with(" --backend wlr"), "{watchdogs:?}");
    let pid = std::fs::read_to_string(&pid_file).expect("dsplr wrote its pid");
    let status = Command::new("kill")
        .args(["-KILL", pid.trim()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "dsplr {} killed", pid.trim());
    // Killed before its own countdown could revert it.
    assert!(started.elapsed() < KILLED_CONFIRM, "killed too late");
    terminal.wait().expect("script ended");
    let deadline = Instant::now() + WATCHDOG;
    while backend.snapshot().await.expect("snapshot") != start {
        assert!(Instant::now() < deadline, "the watchdog did not revert it");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    while any_revert_point().is_some() {
        assert!(
            Instant::now() < deadline,
            "the watchdog removed the revert point"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The confirm timeout of the change killed at its prompt: long enough to
/// see it applied and kill it first, short since the watchdog's delay counts
/// it.
const KILLED_CONFIRM: Duration = Duration::from_secs(15);
/// How long that change's watchdog may take: `restore::watchdog_delay` for
/// wlr's request timeout and [`KILLED_CONFIRM`], with room.
const WATCHDOG: Duration = Duration::from_secs(120);

/// The command lines of the running watchdogs for the change `token` names,
/// read from `/proc`, their arguments joined with spaces.
fn watchdog_command_lines(token: &str) -> Vec<String> {
    let mut found = Vec::new();
    for process in std::fs::read_dir("/proc").expect("/proc").flatten() {
        // Processes come and go while it is read.
        let Ok(line) = std::fs::read(process.path().join("cmdline")) else {
            continue;
        };
        let args: Vec<String> = line
            .split(|&byte| byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect();
        if args.iter().any(|arg| arg == "--after") && args.iter().any(|arg| arg == token) {
            found.push(args.join(" "));
        }
    }
    found
}

async fn save_writes_outputs_and_asks_before_replacing_one(backend: &Backend) {
    let outputs = no_outputs_file();
    let read = || std::fs::read_to_string(&outputs).expect("outputs");

    // On a wlroots compositor that is neither sway nor niri, dsplr declines before
    // writing anything: with no SWAYSOCK, and with one left over from a sway
    // that is gone.
    let stale = harness_runtime().join("sway-ipc.gone.sock");
    for swaysock in [None, Some(stale.as_os_str())] {
        let run = |subcommand: &str| {
            let mut command = Command::new(dsplr_bin());
            command.arg(subcommand).stdin(Stdio::null());
            match swaysock {
                None => command.env_remove("SWAYSOCK"),
                Some(path) => command.env("SWAYSOCK", path),
            };
            exit_and_output(&command.output().expect("dsplr runs"))
        };
        let (code, text) = run("save");
        assert_eq!(code, Some(REFUSED), "{swaysock:?}: {text}");
        assert!(
            text.contains("cannot save on wlr")
                && text.contains("answers as sway's or niri's")
                && text.contains("`dsplr save --profile NAME`")
                && text.contains("`dsplr restore --yes` from the compositor's autostart"),
            "{swaysock:?}: {text}"
        );
        assert!(!outputs.exists());
        // dsplr backend does not claim it either.
        let (code, text) = run("backend");
        assert_eq!(code, Some(0), "{swaysock:?}: {text}");
        assert!(text.contains("backend: wlr\n"), "{text}");
        assert!(!text.contains("native persistence"), "{swaysock:?}: {text}");
    }

    // Created; the harness's config includes it already.
    let start = backend.snapshot().await.expect("snapshot");
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains(&format!("saved to {}", outputs.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "{} already has include outputs",
            sway_config().display()
        )),
        "{text}"
    );
    assert_eq!(ownership(&read(), Comment::Hash), Ownership::Managed);

    // Moved at runtime only; a reload puts the saved layout back.
    swaymsg(&["output", "HEADLESS-2", "position", "0", "3000"]);
    assert_ne!(backend.snapshot().await.expect("snapshot"), start);
    reload_until(backend, &start).await;

    // Edited by hand: refused, with the diff, until --force.
    let edited = format!("{}# mine\n", read());
    std::fs::write(&outputs, &edited).expect("edited");
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("edited by hand"), "{text}");
    assert!(
        text.contains("\n-# mine\n"),
        "the diff shows it going: {text}"
    );
    assert!(text.contains("--force"), "{text}");
    assert_eq!(read(), edited, "nothing was written");
    let (code, text) = dsplr(&["save", "--force"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(ownership(&read(), Comment::Hash), Ownership::Managed);

    // Another tool's file, naming an output not connected: replaced only
    // after a yes, and that output's command is kept.
    let foreign = "output HEADLESS-9 {\n    disable\n}\n";
    std::fs::write(&outputs, foreign).expect("written");
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("no terminal"), "{text}");
    assert_eq!(read(), foreign);
    let on_a_terminal = |answer: &str| {
        exit_and_output(&answered_on_a_terminal(
            &format!("exec {} save", plain(&dsplr_bin())),
            answer,
        ))
    };
    let (code, text) = on_a_terminal("n\n");
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("Replace "), "it was asked: {text}");
    assert_eq!(read(), foreign);
    let (code, text) = on_a_terminal("y\n");
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("kept the settings for HEADLESS-9"), "{text}");
    let saved = read();
    assert_eq!(ownership(&saved, Comment::Hash), Ownership::Managed);
    assert!(saved.contains(foreign), "{saved}");
    // sway reads the kept command for an output it does not have without
    // changing anything else.
    reload_until(backend, &start).await;

    // A file with a setting dsplr does not write takes --force: --yes would
    // lose it.
    let wallpaper = "output HEADLESS-1 bg /nowhere.png fill\n";
    std::fs::write(&outputs, wallpaper).expect("written");
    let (code, text) = dsplr(&["save", "--yes"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("`bg`") && text.contains("--force"), "{text}");
    assert_eq!(read(), wallpaper);
    let (code, text) = dsplr(&["save", "--force"]);
    assert_eq!(code, Some(0), "{text}");
    reload_until(backend, &start).await;

    // A readable file naming the connected outputs by sway's identifier
    // (`Unknown Unknown Unknown`: headless outputs report no make, model or
    // serial). dsplr names them by connector, and at startup an identifier's
    // setting would win over that wherever it is, so the old command goes.
    let by_identifier = "output \"Unknown Unknown Unknown\" scale 2\n";
    std::fs::write(&outputs, by_identifier).expect("written");
    let (code, text) = dsplr(&["save", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("kept the settings"), "{text}");
    assert!(!read().contains("Unknown"), "{}", read());
    reload_until(backend, &start).await;
    FreshSway::start("fresh-cli", start.entries.len())
        .await
        .shows(&start)
        .await;
}

/// kanshi's config, where kanshi finds it in the harness's private
/// `XDG_CONFIG_HOME`, imported into the store: the profile dsplr can hold
/// restores as kanshi would apply it, and the wildcard one is skipped.
async fn kanshi_profiles_are_imported_and_restored(backend: &Backend) {
    let start = backend.snapshot().await.expect("snapshot");
    let snapshot = || async { backend.snapshot().await.expect("snapshot") };
    let kanshi = config_home().join("kanshi");
    std::fs::create_dir_all(&kanshi).expect("kanshi's directory made");
    let config = kanshi.join("config");
    std::fs::write(
        &config,
        "# as kanshi(5) writes it\n\
         profile kdesk {\n\
         \toutput HEADLESS-1 mode 1280x720 position 0,0\n\
         \toutput HEADLESS-2 {\n\
         \t\tenable\n\
         \t\tposition 1280,300\n\
         \t\ttransform 90\n\
         \t}\n\
         \texec notify-send docked\n\
         }\n\
         profile {\n\
         \toutput * enable\n\
         }\n",
    )
    .expect("kanshi's config written");

    // Found as kanshi finds it; the wildcard profile makes the exit 1.
    let (code, text) = dsplr(&["profile", "import", "kanshi"]);
    assert_eq!(code, Some(1), "{text}");
    for said in [
        "kanshi's \"kdesk\"",
        "saved profile \"kdesk\"",
        "the highest it lists, as kanshi picks",
        "HEADLESS-2: kanshi leaves its mode, scale as it is; taken from it as it is now",
        "1 `exec` command is not run",
        "kanshi's <anonymous profile 1>",
        "skipped: `output *` matches any output",
    ] {
        assert!(text.contains(said), "{said:?} not in:\n{text}");
    }

    let (code, text) = dsplr(&["restore", "kdesk", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let expected = edited(&start, |layout| {
        let state = &mut entry_mut(layout, "HEADLESS-2").state;
        (state.x, state.y) = (1280, 300);
        state.transform = Transform::Rotate90;
    });
    assert_eq!(snapshot().await, expected);
    revert_to(backend, &start).await;

    // Imported again, the store's profile is kept unless --force.
    let path = config.to_str().expect("a UTF-8 path");
    let (code, text) = dsplr(&["profile", "import", "kanshi", path]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("--force replaces it"), "{text}");
    assert!(!text.contains("saved profile"), "{text}");
    let (code, text) = dsplr(&["profile", "import", "kanshi", path, "--force"]);
    assert_eq!(code, Some(1), "the wildcard profile still: {text}");
    assert!(text.contains("saved profile \"kdesk\""), "{text}");
    assert!(
        text.contains("replaces the existing profile \"kdesk\""),
        "{text}"
    );

    // Imported fresh, nothing is said to be replaced.
    let (code, text) = dsplr(&["profile", "rm", "kdesk"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["profile", "import", "kanshi", path, "--force"]);
    assert_eq!(code, Some(1), "the wildcard profile still: {text}");
    assert!(text.contains("saved profile \"kdesk\""), "{text}");
    assert!(!text.contains("replaces the existing"), "{text}");

    let (code, text) = dsplr(&["profile", "rm", "kdesk"]);
    assert_eq!(code, Some(0), "{text}");
}
