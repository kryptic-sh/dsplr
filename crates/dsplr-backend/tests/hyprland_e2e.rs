//! The Hyprland backend against a real, headless Hyprland.
//!
//! Run through the harness, which starts that Hyprland where it cannot reach
//! the desktop session:
//!
//! ```text
//! tests/e2e/headless-hyprland.sh cargo test -p dsplr-backend --test hyprland_e2e -- --ignored --test-threads=1
//! ```
//!
//! The test refuses to run when the harness did not start it, so `--ignored`
//! on its own never changes a real desktop's monitors.
//!
//! One Hyprland serves every step, so they run in turn, in one test, in a
//! fixed order. The last one applies through `wlr-output-management`, whose
//! state Hyprland keeps per monitor beside its rules: a later `eval` wins
//! over it (0.57.2), and whether a reload does was not checked, so nothing
//! runs after it.

#![cfg(target_os = "linux")]

use std::time::Duration;

use dsplr_backend::hyprland::Hyprland;
use dsplr_backend::wlr::Wlr;
use dsplr_backend::{Backend, Persisted, Watch};
use dsplr_model::{Layout, Mode, OutputId, Transform};
use dsplr_persist::managed::{Comment, Ownership, ownership};

#[path = "../../../tests/e2e/hyprland_support.rs"]
mod hyprland_support;
#[path = "../../../tests/e2e/support.rs"]
mod support;

use hyprland_support::{
    SETTLE, config_home, harness_runtime, hyprctl, hyprland_wayland_display, snapshot_until,
};
use support::{edited, entry, entry_mut};

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs the headless Hyprland harness: tests/e2e/headless-hyprland.sh"]
async fn a_headless_hyprland_applies_saves_watches_and_meets_the_protocol() {
    a_headless_hyprland_shows_each_applied_layout().await;
    a_saved_layout_is_what_hyprland_reads_back_after_a_reload().await;
    let backend = hyprland_backend().await;
    a_watch_follows_reloads_and_hotplug(&backend).await;
    colour_and_bit_depth_as_hyprland_reports_them().await;
    // Last: Hyprland keeps what the protocol applies per monitor, and a
    // reload after it was not checked.
    an_eval_mirror_meets_a_protocol_apply(&backend).await;
}

async fn hyprland_backend() -> Backend {
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));
    assert_eq!(backend.name(), "hyprland");
    backend
}

async fn a_headless_hyprland_shows_each_applied_layout() {
    harness_runtime();
    for name in ["DSPLR-1", "DSPLR-2"] {
        let reply = hyprctl(&format!("/output create headless {name}"));
        assert_eq!(reply.trim(), "ok", "creating {name}: {reply}");
    }

    let detection = dsplr_backend::detect().await;
    let backend = detection.backend.unwrap_or_else(|| {
        panic!("no backend: {:?}", detection.checks);
    });
    assert_eq!(backend.name(), "hyprland");
    let apply = |layout: Layout| {
        let backend = backend.clone();
        async move { backend.apply(&layout).await.expect("Hyprland accepted it") }
    };

    // Lay the outputs out in a row with a gap, from what Hyprland chose for
    // the new ones. The revert point is this layout.
    // Other outputs may be there too (the CI VM's own display): only these
    // three are laid out, and the rest are applied as they are.
    let first = snapshot_until(&backend, "both new outputs", |layout| {
        ["DSPLR-1", "DSPLR-2"].iter().all(|name| {
            layout
                .entries
                .iter()
                .any(|e| e.output.as_ref().is_some_and(|o| o.connector == *name))
        })
    })
    .await;
    let start = edited(&first, |layout| {
        for (connector, x) in [("WAYLAND-1", 0), ("DSPLR-1", 1280), ("DSPLR-2", 3200)] {
            let state = &mut entry_mut(layout, connector).state;
            (state.x, state.y) = (x, 0);
        }
        entry_mut(layout, "WAYLAND-1").state.mode = Some(Mode::new(1280, 720, 60_000));
        entry_mut(layout, "DSPLR-1").state.mode = Some(Mode::new(1920, 1080, 60_000));
        entry_mut(layout, "DSPLR-2").state.mode = Some(Mode::new(1920, 1080, 60_000));
    });
    apply(start.clone()).await;
    let start = snapshot_until(&backend, "the row", |layout| {
        entry(layout, "DSPLR-2").state.x == 3200
    })
    .await;
    let two = entry(&start, "DSPLR-2");
    assert_eq!(two.id, OutputId::Connector("DSPLR-2".into()), "no serial");
    assert_eq!((two.state.x, two.state.y), (3200, 0), "the gap is kept");
    assert_eq!(two.state.mode, Some(Mode::new(1920, 1080, 60_000)));

    // A fractional scale and a rotation.
    apply(edited(&start, |layout| {
        let state = &mut entry_mut(layout, "DSPLR-1").state;
        state.scale = 1.5;
        state.transform = Transform::Rotate90;
    }))
    .await;
    let layout = snapshot_until(&backend, "scale and transform", |layout| {
        entry(layout, "DSPLR-1").state.transform == Transform::Rotate90
    })
    .await;
    let one = &entry(&layout, "DSPLR-1").state;
    assert!(one.enabled);
    assert_eq!((one.x, one.y), (1280, 0));
    assert!(
        (one.scale - 1.5).abs() < f64::EPSILON,
        "scale {}",
        one.scale
    );
    assert_eq!(one.mode, Some(Mode::new(1920, 1080, 60_000)));

    // Applying the revert point puts every setting back, not just the ones
    // the last apply named.
    apply(start.clone()).await;
    snapshot_until(&backend, "the revert point", |layout| *layout == start).await;

    // Disabled outputs stay in the snapshot, disabled, without a mode.
    apply(edited(&start, |layout| {
        entry_mut(layout, "DSPLR-2").state.enabled = false;
    }))
    .await;
    let layout = snapshot_until(&backend, "DSPLR-2 disabled", |layout| {
        !entry(layout, "DSPLR-2").state.enabled
    })
    .await;
    assert_eq!(entry(&layout, "DSPLR-2").state.mode, None);

    // And come back as they were.
    apply(start.clone()).await;
    snapshot_until(&backend, "DSPLR-2 back", |layout| *layout == start).await;

    // A mirror is reported by the source monitor's id, and resolves to the
    // source's entry.
    let wayland_1 = entry(&start, "WAYLAND-1").id.clone();
    apply(edited(&start, |layout| {
        entry_mut(layout, "DSPLR-1").state.mirror_of = Some(wayland_1.clone());
    }))
    .await;
    let layout = snapshot_until(&backend, "DSPLR-1 mirroring", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.is_some()
    })
    .await;
    assert_eq!(
        entry(&layout, "DSPLR-1").state.mirror_of.as_ref(),
        Some(&wayland_1)
    );

    // A layout Hyprland rejects is an error, not a silent no-op.
    let hyprland = Hyprland::from_env().expect("the harness's Hyprland");
    assert!(
        hyprland
            .eval(r#"hl.monitor({ output = "DSPLR-1", mode = "nonsense" })"#)
            .await
            .is_err()
    );

    // And the revert point ends the mirror.
    apply(start.clone()).await;
    snapshot_until(&backend, "DSPLR-1 not mirroring", |layout| *layout == start).await;
}

/// A saved layout is what Hyprland shows after a reload, which drops every
/// rule `eval` set: so it came from the file.
async fn a_saved_layout_is_what_hyprland_reads_back_after_a_reload() {
    harness_runtime();
    let reply = hyprctl("/output create headless DSPLR-SAVE");
    assert_eq!(reply.trim(), "ok", "creating DSPLR-SAVE: {reply}");
    let detection = dsplr_backend::detect().await;
    let backend = detection
        .backend
        .unwrap_or_else(|| panic!("no backend: {:?}", detection.checks));

    // A state the config's catch-all would not give DSPLR-SAVE: placed,
    // scaled and turned by eval rules.
    let first = snapshot_until(&backend, "DSPLR-SAVE", |layout| {
        layout.entries.iter().any(|e| {
            e.output
                .as_ref()
                .is_some_and(|o| o.connector == "DSPLR-SAVE")
        })
    })
    .await;
    let placed = edited(&first, |layout| {
        let state = &mut entry_mut(layout, "DSPLR-SAVE").state;
        state.mode = Some(Mode::new(1920, 1080, 60_000));
        (state.x, state.y) = (9000, 0);
        state.scale = 1.5;
        state.transform = Transform::Rotate90;
    });
    backend.apply(&placed).await.expect("applied");
    let saved = snapshot_until(&backend, "DSPLR-SAVE placed", |layout| {
        entry(layout, "DSPLR-SAVE").state.x == 9000
    })
    .await;

    let hypr = config_home().join("hypr");
    let monitors = hypr.join("monitors.lua");
    assert!(!monitors.exists(), "the harness starts without one");
    let persisted = backend.persist(&saved, None).await.expect("saved");
    let Persisted::File(file) = persisted else {
        panic!("not saved to a file: {persisted:?}");
    };
    assert_eq!(file.path, monitors);
    assert_eq!(file.main_config, hypr.join("hyprland.lua"));
    assert!(file.created);
    assert!(
        !file.included,
        "the harness's config does not require it yet"
    );
    let text = std::fs::read_to_string(&monitors).expect("written");
    assert_eq!(ownership(&text, Comment::Lua), Ownership::Managed);

    // Move it at runtime only, then load the file as the user would: add the
    // line dsplr named, and reload.
    backend
        .apply(&edited(&saved, |layout| {
            entry_mut(layout, "DSPLR-SAVE").state.x = 12000;
        }))
        .await
        .expect("applied");
    snapshot_until(&backend, "DSPLR-SAVE moved", |layout| {
        entry(layout, "DSPLR-SAVE").state.x == 12000
    })
    .await;
    let main = std::fs::read_to_string(&file.main_config).expect("hyprland.lua");
    std::fs::write(&file.main_config, format!("{main}{}\n", file.include_line))
        .expect("hyprland.lua written");
    assert_eq!(hyprctl("reload").trim(), "ok");
    snapshot_until(&backend, "the saved layout", |layout| *layout == saved).await;
    let errors = hyprctl("configerrors");
    assert!(errors.trim().is_empty(), "config errors: {errors}");

    // Saved again, it is dsplr's own file and is overwritten, and the config
    // is seen to require it.
    let persisted = backend.persist(&saved, None).await.expect("saved again");
    let Persisted::File(again) = persisted else {
        panic!("not saved to a file again: {persisted:?}");
    };
    assert!(!again.created);
    assert!(again.included);
    assert_eq!(std::fs::read_to_string(&monitors).expect("written"), text);
}

/// A probe, not a check: what `hyprctl -j monitors all` reports about colour
/// management and bit depth on DSPLR-1 after each `cm` preset and each
/// `bitdepth` is set through `eval`, so dsplr reads and writes them on facts
/// (`docs/backlog.md`, Hyprland's colour preset and bit depth). Prints
/// `OBSERVED` lines and asserts nothing; puts `srgb` and 8 bits back last,
/// since Hyprland merges a rule onto the one before it.
async fn colour_and_bit_depth_as_hyprland_reports_them() {
    let hyprland = Hyprland::from_env().expect("the harness's Hyprland");
    let version = hyprland.version().await.expect("its version");
    let monitor = || {
        let reply = hyprctl("j/monitors all");
        let monitors: serde_json::Value = serde_json::from_str(&reply).expect("JSON");
        monitors
            .as_array()
            .expect("an array")
            .iter()
            .find(|monitor| monitor["name"] == "DSPLR-1")
            .cloned()
            .expect("DSPLR-1 listed")
    };
    // Every key, once, so the fields' names are on record.
    let keys: Vec<String> = monitor()
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    observed(&format!(
        "OBSERVED (Hyprland {version}): monitor keys: {}",
        keys.join(", ")
    ));
    let colour_fields = || {
        let monitor = monitor();
        let fields: Vec<String> = monitor
            .as_object()
            .expect("an object")
            .iter()
            .filter(|(key, _)| {
                let key = key.to_ascii_lowercase();
                [
                    "format", "color", "colour", "bit", "sdr", "hdr", "cm", "icc", "wide",
                ]
                .iter()
                .any(|part| key.contains(part))
            })
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        fields.join(" ")
    };
    observed(&format!(
        "OBSERVED (Hyprland {version}): before: {}",
        colour_fields()
    ));
    // A rule takes effect on Hyprland's next frame.
    let settle = || tokio::time::sleep(Duration::from_secs(1));
    let try_rule = async |rule: &str| {
        let call = format!(r#"hl.monitor({{ output = "DSPLR-1", {rule} }})"#);
        let answer = match hyprland.eval(&call).await {
            Ok(()) => "accepted".to_owned(),
            Err(error) => format!("refused ({error})"),
        };
        settle().await;
        observed(&format!(
            "OBSERVED (Hyprland {version}): eval {rule}: {answer}; then {}",
            colour_fields()
        ));
    };
    for preset in [
        "srgb", "wide", "edid", "hdr", "hdredid", "auto", "dcip3", "dp3", "adobe",
    ] {
        try_rule(&format!(r#"cm = "{preset}""#)).await;
    }
    try_rule(r#"cm = "srgb""#).await;
    try_rule("bitdepth = 10").await;
    // A rule leaving bit depth out, which Hyprland merges onto the last.
    try_rule(r#"mirror = """#).await;
    try_rule("bitdepth = 8").await;
    try_rule("bitdepth = 10").await;
    try_rule("bitdepth = 16").await;
    try_rule("bitdepth = 8").await;
}

/// Writes `line` to the test's stderr whether or not the test passes:
/// libtest captures `println!` and `eprintln!`, not the stderr handle, and
/// these lines are the record the CI log keeps.
fn observed(line: &str) {
    use std::io::Write;
    writeln!(std::io::stderr(), "{line}").expect("stderr");
}

/// How long a watch gets to report a change, and how long an event that is
/// not expected is waited for.
const WATCH_WAIT: Duration = Duration::from_secs(5);

/// Reads the events the watch has queued, so the next `changed` is about
/// the next change: Hyprland sends `monitoradded` and `monitoraddedv2` for
/// one output, and each would end a wait.
async fn drain(watch: &mut Watch) {
    loop {
        match tokio::time::timeout(Duration::from_millis(300), watch.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("the watch failed while draining: {error}"),
            Err(_) => return,
        }
    }
}

async fn changed(watch: &mut Watch, what: &str) {
    tokio::time::timeout(WATCH_WAIT, watch.changed())
        .await
        .unwrap_or_else(|_| panic!("the watch reported nothing after {what}"))
        .unwrap_or_else(|error| panic!("the watch failed after {what}: {error}"));
}

/// `.socket2.sock` reports a reload and an output plugged in and removed,
/// and a snapshot then shows each. A monitor rule set through `eval` is
/// said to send no event (`hyprland::Events`); what this Hyprland does is
/// printed, not asserted, as a record for the backlog's entry on it.
async fn a_watch_follows_reloads_and_hotplug(backend: &Backend) {
    let version = Hyprland::from_env()
        .expect("the harness's Hyprland")
        .version()
        .await
        .expect("its version");
    let mut watch = backend.watch().await.expect("a watch");

    let now = backend.snapshot().await.expect("snapshot");
    let x = entry(&now, "DSPLR-SAVE").state.x + 640;
    backend
        .apply(&edited(&now, |layout| {
            entry_mut(layout, "DSPLR-SAVE").state.x = x;
        }))
        .await
        .expect("applied");
    snapshot_until(backend, "DSPLR-SAVE moved by eval", |layout| {
        entry(layout, "DSPLR-SAVE").state.x == x
    })
    .await;
    let reported = match tokio::time::timeout(WATCH_WAIT, watch.changed()).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => panic!("the watch failed after an eval: {error}"),
        Err(_) => false,
    };
    observed(&format!(
        "OBSERVED (Hyprland {version}): a monitor rule set through eval {} on .socket2.sock",
        if reported {
            "sent an event"
        } else {
            "sent no event"
        }
    ));
    // Seen on Hyprland 0.57.2 (CI, 2026-11-10): no event, so the watch
    // follows neither dsplr's own eval applies nor another program's.
    assert!(
        !reported,
        "an eval monitor rule now sends a .socket2.sock event: the watch could follow it"
    );
    drain(&mut watch).await;

    assert_eq!(hyprctl("reload").trim(), "ok");
    changed(&mut watch, "a reload").await;
    drain(&mut watch).await;

    let reply = hyprctl("/output create headless DSPLR-WATCH");
    assert_eq!(reply.trim(), "ok", "creating DSPLR-WATCH: {reply}");
    changed(&mut watch, "an output plugged in").await;
    snapshot_until(backend, "DSPLR-WATCH plugged in", |layout| {
        layout
            .entries
            .iter()
            .any(|e| e.connector() == Some("DSPLR-WATCH"))
    })
    .await;
    drain(&mut watch).await;

    let reply = hyprctl("/output remove DSPLR-WATCH");
    assert_eq!(reply.trim(), "ok", "removing DSPLR-WATCH: {reply}");
    changed(&mut watch, "an output removed").await;
    snapshot_until(backend, "DSPLR-WATCH removed", |layout| {
        layout
            .entries
            .iter()
            .all(|e| e.connector() != Some("DSPLR-WATCH"))
    })
    .await;
}

/// The M2 exit check (`docs/plan/04-backends.md#hyprland`): whether a
/// `wlr-output-management` apply undoes a mirror `eval` set, and whether
/// state the protocol applied survives a later `eval` rule. The protocol is
/// spoken by dsplr's own `wlr` backend, on Hyprland's own Wayland socket.
///
/// Both are asserted as Hyprland 0.57.2 behaved in CI (2026-11-10): the
/// mirror is kept, and a later `eval` wins over the protocol's position.
/// The protocol requires every head in a configuration, so any client
/// re-sends the mirroring head too; keeping the mirror is Hyprland's doing,
/// not dsplr's. The `OBSERVED` lines say what this Hyprland did either way.
async fn an_eval_mirror_meets_a_protocol_apply(backend: &Backend) {
    let version = Hyprland::from_env()
        .expect("the harness's Hyprland")
        .version()
        .await
        .expect("its version");
    let runtime = harness_runtime();
    let display = hyprland_wayland_display();
    let wlr = Wlr::from_vars(|name| match name {
        "WAYLAND_DISPLAY" => Some(display.clone().into()),
        "XDG_RUNTIME_DIR" => Some(runtime.clone().into_os_string()),
        _ => None,
    })
    .await
    .expect("Hyprland's wlr-output-management");
    observed(&format!(
        "Hyprland {version} offers zwlr_output_manager_v1 version {}",
        wlr.version()
    ));

    // A mirror, through eval.
    let now = backend.snapshot().await.expect("snapshot");
    let source = entry(&now, "DSPLR-2").id.clone();
    backend
        .apply(&edited(&now, |layout| {
            entry_mut(layout, "DSPLR-1").state.mirror_of = Some(source.clone());
        }))
        .await
        .expect("applied");
    snapshot_until(backend, "DSPLR-1 mirroring DSPLR-2", |layout| {
        entry(layout, "DSPLR-1").state.mirror_of.as_ref() == Some(&source)
    })
    .await;

    // A protocol apply of every head as the protocol reads it, DSPLR-2
    // moved.
    let heads = wlr.snapshot().await.expect("the heads");
    observed(&format!(
        "the protocol reads the mirroring DSPLR-1 as {:?}",
        entry(&heads, "DSPLR-1").state
    ));
    let moved = 20_000;
    wlr.apply(&edited(&heads, |layout| {
        entry_mut(layout, "DSPLR-2").state.x = moved;
    }))
    .await
    .expect("Hyprland applied the configuration");
    snapshot_until(backend, "DSPLR-2 moved by the protocol", |layout| {
        entry(layout, "DSPLR-2").state.x == moved
    })
    .await;
    // Hyprland reloads its monitor rules after a protocol apply: give that
    // its time before reading the mirror.
    tokio::time::sleep(SETTLE).await;
    let shown = backend.snapshot().await.expect("snapshot");
    let mirror = entry(&shown, "DSPLR-1").state.mirror_of.clone();
    observed(&format!(
        "OBSERVED (Hyprland {version}): a wlr-output-management apply {} a mirror set through eval (DSPLR-1 mirrors {mirror:?})",
        if mirror.as_ref() == Some(&source) {
            "keeps"
        } else {
            "undoes"
        }
    ));
    assert_eq!(
        mirror.as_ref(),
        Some(&source),
        "a protocol apply now undoes a mirror set through eval"
    );

    // A later eval moves DSPLR-2 and an output the protocol never set. The
    // latter shows the eval was applied; the former, whether the eval won
    // over the protocol's position.
    let reply = hyprctl("/output create headless DSPLR-LATE");
    assert_eq!(reply.trim(), "ok", "creating DSPLR-LATE: {reply}");
    let now = snapshot_until(backend, "DSPLR-LATE", |layout| {
        layout
            .entries
            .iter()
            .any(|e| e.connector() == Some("DSPLR-LATE"))
    })
    .await;
    let late = 30_000;
    let asked = moved + 4_000;
    backend
        .apply(&edited(&now, |layout| {
            entry_mut(layout, "DSPLR-2").state.x = asked;
            let state = &mut entry_mut(layout, "DSPLR-LATE").state;
            state.mode = Some(Mode::new(1920, 1080, 60_000));
            state.x = late;
        }))
        .await
        .expect("applied");
    let shown = snapshot_until(backend, "DSPLR-LATE moved by eval", |layout| {
        entry(layout, "DSPLR-LATE").state.x == late
    })
    .await;
    // Both are in one eval, but give DSPLR-2 the same time to follow.
    let deadline = std::time::Instant::now() + SETTLE;
    let mut x = entry(&shown, "DSPLR-2").state.x;
    while x != asked && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        x = entry(&backend.snapshot().await.expect("snapshot"), "DSPLR-2")
            .state
            .x;
    }
    observed(&format!(
        "OBSERVED (Hyprland {version}): after the protocol set DSPLR-2's x to {moved}, an eval asking {asked} left it at {x}"
    ));
    assert_eq!(
        x, asked,
        "the protocol's position now wins over a later eval rule"
    );
}
