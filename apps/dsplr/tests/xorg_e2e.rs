//! `dsplr apply` and `dsplr watch` against an X server with more than one
//! output: the harness's Xorg with the dummy driver.
//!
//! ```text
//! DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh \
//!   cargo test -p dsplr --test xorg_e2e -- --ignored --test-threads=1
//! ```
//!
//! The tests share one server, and a dummy output turned on stays connected,
//! so each plugs in its own: `DUMMY1` for the mirror, `DUMMY2` for the
//! hotplug. `DSPLR_BIN` names the dsplr to run when it is not where Cargo
//! built it (in CI's VM).

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::SystemTime;

use dsplr_model::{Layout, Mode, OutputEntry, OutputId, OutputState};
use dsplr_persist::profile::{self, Profile, unix_ms};

#[path = "../../../tests/e2e/support.rs"]
mod support;
#[path = "../../../tests/e2e/x11_support.rs"]
mod x11_support;

use support::{Watcher, entry, entry_mut};
use x11_support::xorg_runtime;

fn dsplr_bin() -> PathBuf {
    std::env::var_os("DSPLR_BIN").map_or_else(|| env!("CARGO_BIN_EXE_dsplr").into(), PathBuf::from)
}

/// Runs dsplr with `args` and no terminal; its exit code and everything it
/// printed.
fn dsplr(args: &[&str]) -> (Option<i32>, String) {
    let output: Output = Command::new(dsplr_bin())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("dsplr runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), text)
}

fn listed() -> Layout {
    let (code, json) = dsplr(&["list", "--json"]);
    assert_eq!(code, Some(0), "{json}");
    Layout::from_json(&json).expect("dsplr list --json reads back")
}

/// Runs xrandr on the harness's display with `args`; returns what it printed.
fn xrandr(args: &[&str]) -> String {
    let output = Command::new("xrandr")
        .args(args)
        .output()
        .expect("xrandr runs");
    assert!(
        output.status.success(),
        "xrandr {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// xrandr's line for `output`.
fn line(output: &str) -> String {
    let prefix = format!("{output} ");
    xrandr(&["--query"])
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no {output} in xrandr --query"))
        .to_owned()
}

/// Plugs `output` in, at the right of DUMMY0, at the harness's 1280x720.
fn plug(output: &str) {
    xrandr(&[
        "--output",
        output,
        "--mode",
        "1280x720_60.00",
        "--right-of",
        "DUMMY0",
    ]);
}

/// `dsplr apply --yes` of `layout`, through a file in the harness's runtime.
fn apply(runtime: &std::path::Path, layout: &Layout) -> (Option<i32>, String) {
    let file = runtime.join("layout.json");
    std::fs::write(&file, layout.to_json().expect("JSON")).expect("written");
    dsplr(&["apply", "--yes", file.to_str().expect("UTF-8")])
}

/// `dsplr apply` of DUMMY0 mirroring DUMMY1: X keeps no mirror direction and
/// reads DUMMY1 back as mirroring DUMMY0, which is the same mirror, so the
/// apply reads back as asked.
#[test]
#[ignore = "needs the Xorg harness: DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh"]
fn a_mirror_applied_reads_back_as_asked_whichever_output_is_the_source() {
    let runtime = xorg_runtime();
    plug("DUMMY1");
    let before = listed();
    let mut mirrored = before.clone();
    let dummy1 = entry(&before, "DUMMY1").clone();
    for name in ["DUMMY0", "DUMMY1"] {
        let state = &mut entry_mut(&mut mirrored, name).state;
        state.mode.clone_from(&dummy1.state.mode);
        (state.x, state.y) = (0, 0);
    }
    entry_mut(&mut mirrored, "DUMMY0").state.mirror_of = Some(dummy1.id.clone());

    let (code, text) = apply(&runtime, &mirrored);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("other than was asked"), "{text}");
    assert!(
        line("DUMMY0").contains(" 1280x720+0+0"),
        "{}",
        line("DUMMY0")
    );
    assert!(
        line("DUMMY1").contains(" 1280x720+0+0"),
        "{}",
        line("DUMMY1")
    );

    let (code, text) = apply(&runtime, &before);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("other than was asked"), "{text}");
}

/// `dsplr watch` restoring the profile saved for a monitor when it is
/// plugged in: DUMMY2, at the left of the others.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the Xorg harness: DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh"]
async fn watch_restores_the_profile_for_a_monitor_plugged_in() {
    let runtime = xorg_runtime();
    // The profile for the outputs connected now and DUMMY2, written before
    // DUMMY2 can be listed: DUMMY2 at the origin, the others moved right.
    let now = listed();
    assert!(
        now.entries
            .iter()
            .all(|entry| entry.id != OutputId::Connector("DUMMY2".into()))
    );
    let mut docked = now.clone();
    for entry in &mut docked.entries {
        if entry.state.enabled {
            entry.state.x += 1280;
        }
    }
    docked.entries.push(OutputEntry {
        id: OutputId::Connector("DUMMY2".into()),
        output: None,
        state: OutputState {
            enabled: true,
            mode: Some(Mode::new(1280, 720, 59_855)),
            ..OutputState::default()
        },
    });
    let store = profile::default_dir().expect("XDG_CONFIG_HOME is set by the harness");
    assert!(store.starts_with(&runtime), "{}", store.display());
    let name = "docked".parse().expect("a name");
    profile::save(
        &store,
        &Profile {
            name,
            layout: docked,
            saved_unix_ms: unix_ms(SystemTime::now()),
        },
    )
    .expect("profile saved");

    let mut command = Command::new(dsplr_bin());
    command.arg("watch").stdin(Stdio::null());
    let watcher = Watcher::start(command, runtime.join("watch.log"));
    watcher
        .said("no profile is for the outputs connected now")
        .await;
    assert_eq!(watcher.restores(), 0, "{}", watcher.log());

    plug("DUMMY2");
    let shown = watcher
        .restored(listed, 1, "docked", |layout| {
            let at = |name: &str| {
                layout
                    .entries
                    .iter()
                    .find(|entry| entry.output.as_ref().is_some_and(|o| o.connector == name))
                    .map(|entry| (entry.state.x, entry.state.y))
            };
            at("DUMMY2") == Some((0, 0)) && at("DUMMY0") == Some((1280, 0))
        })
        .await;
    assert!(
        line("DUMMY2").contains(" 1280x720+0+0"),
        "{}",
        line("DUMMY2")
    );
    assert!(entry(&shown, "DUMMY0").state.enabled);
    let log = watcher.stop();
    assert!(!log.contains("not restored"), "{log}");
}
