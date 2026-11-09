//! `dsplr backend`, `dsplr set` and `dsplr save` against a real niri,
//! nested in a headless sway.
//!
//! Run through the harness, which starts that niri where it cannot reach the
//! desktop session and gives dsplr a private state and config directory:
//!
//! ```text
//! tests/e2e/headless-niri.sh cargo test -p dsplr --test niri_e2e -- --ignored
//! ```
//!
//! Every change is kept with `--yes`; the confirm prompt is the same on every
//! backend and is covered by `hyprland_e2e`. `dsplr save`'s own question is
//! asked on a terminal `script` (util-linux) gives it. `DSPLR_BIN` names the
//! dsplr to run when it is not where Cargo built it (in CI's VM). A nested
//! niri has one output, `winit`, and takes only its scale and position.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use dsplr_backend::wlr::Wlr;
use dsplr_model::Layout;
use dsplr_persist::managed::{Comment, Ownership, ownership};

#[path = "../../../tests/e2e/niri_support.rs"]
mod niri_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use niri_support::{harness_runtime, niri_config, no_outputs_file, placed, until};
use support::{answered_on_a_terminal, entry, plain};

/// The exit code of a layout validation refused, or a save the backend
/// cannot make (`Exit::Refused`).
const REFUSED: i32 = 3;

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

/// Whether `layout` shows niri's one output at `scale`, at `x`, `y`.
fn shows(layout: &Layout, scale: f64, x: i32, y: i32) -> bool {
    placed(&entry(layout, "winit").state) == (scale, x, y)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the nested niri harness: tests/e2e/headless-niri.sh"]
async fn dsplr_sets_and_saves_on_niri() {
    let runtime = harness_runtime();
    let outputs = no_outputs_file();
    let read = || std::fs::read_to_string(&outputs).expect("outputs.kdl");
    let wlr = Wlr::from_env().await.expect("the harness's niri");

    let (code, text) = dsplr(&["backend"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("backend: wlr\n") && text.contains("native persistence"),
        "{text}"
    );

    // Each change is applied and kept: niri's `cancelled` after its
    // `succeeded` is not taken for its answer.
    for (scale, position, x, y) in [("1.5", "10,20", 10, 20), ("1", "0,0", 0, 0)] {
        let (code, text) = dsplr(&["set", "winit", "--scale", scale, "--pos", position, "--yes"]);
        assert_eq!(code, Some(0), "{text}");
        let scale: f64 = scale.parse().expect("a scale");
        until(&wlr, "the change", |layout| shows(layout, scale, x, y)).await;
    }
    let (code, text) = dsplr(&["set", "winit", "--scale", "1.5", "--pos", "10,20", "--yes"]);
    assert_eq!(code, Some(0), "{text}");

    // With NIRI_SOCKET unset, or naming a socket that is gone, dsplr declines
    // before writing anything, and `dsplr backend` does not claim it.
    let stale = runtime.join("niri.gone.sock");
    for niri_socket in [None, Some(stale.as_os_str())] {
        let run = |subcommand: &str| {
            let mut command = Command::new(dsplr_bin());
            command.arg(subcommand).stdin(Stdio::null());
            match niri_socket {
                None => command.env_remove("NIRI_SOCKET"),
                Some(path) => command.env("NIRI_SOCKET", path),
            };
            exit_and_output(&command.output().expect("dsplr runs"))
        };
        let (code, text) = run("save");
        assert_eq!(code, Some(REFUSED), "{niri_socket:?}: {text}");
        assert!(
            text.contains("answers as sway's or niri's")
                && text.contains("`dsplr save --profile NAME`"),
            "{niri_socket:?}: {text}"
        );
        assert!(!outputs.exists());
        let (code, text) = run("backend");
        assert_eq!(code, Some(0), "{niri_socket:?}: {text}");
        assert!(
            !text.contains("native persistence"),
            "{niri_socket:?}: {text}"
        );
    }

    // Created; the harness's config includes it already.
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains(&format!("saved to {}", outputs.display())),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "{} already has include \"outputs.kdl\"",
            niri_config().display()
        )),
        "{text}"
    );
    let saved = read();
    assert_eq!(ownership(&saved, Comment::Slashes), Ownership::Managed);
    assert!(
        saved.contains("    scale 1.5\n") && saved.contains("    position x=10 y=20\n"),
        "{saved}"
    );

    // niri reads the file: edited by hand, it shows the edit at once.
    let edited = saved.replace("scale 1.5", "scale 2");
    std::fs::write(&outputs, &edited).expect("edited");
    until(&wlr, "the edit", |layout| shows(layout, 2.0, 10, 20)).await;
    let (code, text) = dsplr(&["list"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("position 10,20, scale 2,"), "{text}");

    // Edited by hand: refused, with the diff, until --force. A change at
    // runtime first, so saving changes more than the header.
    let (code, text) = dsplr(&["set", "winit", "--scale", "1.5", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("edited by hand"), "{text}");
    assert!(
        text.contains("\n-    scale 2\n+    scale 1.5\n"),
        "the diff shows the edit going: {text}"
    );
    assert!(text.contains("--force"), "{text}");
    assert_eq!(read(), edited, "nothing was written");
    let (code, text) = dsplr(&["save", "--force"]);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(ownership(&read(), Comment::Slashes), Ownership::Managed);
    assert!(
        read().contains("    scale 1.5\n"),
        "the layout shown: {}",
        read()
    );

    // Another tool's file, naming an output not connected: replaced only
    // after a yes, and that output's node is kept.
    let foreign = "output \"DP-9\" {\n    off\n}\n";
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
    assert!(text.contains("kept the settings for DP-9"), "{text}");
    let saved = read();
    assert_eq!(ownership(&saved, Comment::Slashes), Ownership::Managed);
    assert!(saved.contains(foreign), "{saved}");

    // A file with a setting dsplr does not write takes --force: --yes would
    // lose it.
    let backdrop = "output \"winit\" {\n    backdrop-color \"#001100\"\n}\n";
    std::fs::write(&outputs, backdrop).expect("written");
    let (code, text) = dsplr(&["save", "--yes"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("`backdrop-color`") && text.contains("--force"),
        "{text}"
    );
    assert_eq!(read(), backdrop);

    // A readable file naming the output by niri's identifier, in another
    // case: it is that output's, so its node goes.
    let by_identifier = "output \"smithay winit unknown\" {\n    scale 3\n}\n";
    std::fs::write(&outputs, by_identifier).expect("written");
    until(&wlr, "the identifier's node", |layout| {
        shows(layout, 3.0, 0, 0)
    })
    .await;
    let (code, text) = dsplr(&["save", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("kept the settings"), "{text}");
    assert!(!read().contains("Unknown"), "{}", read());
    assert!(
        read().contains("output \"winit\" {\n    mode"),
        "{}",
        read()
    );
    until(&wlr, "the saved layout", |layout| shows(layout, 3.0, 0, 0)).await;

    // The config's own node for the output, after the include, setting what
    // dsplr does not write: once the file is written its node comes first
    // and hides that one whole, so saving asks first, file or no file.
    let config = niri_config();
    let original = std::fs::read_to_string(&config).expect("niri's config");
    no_outputs_file();
    std::fs::write(
        &config,
        format!(
            "{original}output \"winit\" {{\n    backdrop-color \"#001100\"\n    scale 1.75\n}}\n"
        ),
    )
    .expect("config written");
    until(&wlr, "the config's own node", |layout| {
        shows(layout, 1.75, 0, 0)
    })
    .await;
    let (code, text) = dsplr(&["save"]);
    assert_eq!(code, Some(1), "{text}");
    assert!(
        text.contains("stop applying")
            && text.contains(&format!(
                "{}: output \"winit\" sets backdrop-color",
                config.display()
            ))
            && text.contains("no terminal"),
        "{text}"
    );
    assert!(!outputs.exists(), "nothing was written");
    let (code, text) = dsplr(&["save", "--yes"]);
    assert_eq!(code, Some(0), "{text}");
    assert!(read().contains("    scale 1.75\n"), "{}", read());
    // niri takes the saved node now, not the config's: a scale written there
    // shows, the config's own 1.75 notwithstanding.
    std::fs::write(&outputs, read().replace("scale 1.75", "scale 1.25")).expect("edited");
    until(&wlr, "the saved node over the config's", |layout| {
        shows(layout, 1.25, 0, 0)
    })
    .await;
    std::fs::write(&config, original).expect("config restored");
    no_outputs_file();
}
