//! [`import`] against profiles written as autorandr writes them, with EDIDs
//! built byte by byte here.

use std::path::Path;

use dsplr_model::{Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform};

use super::profiles::{AutorandrOutput, AutorandrProfile, Read};
use super::{Imported, Skipped, import, read, transform};

/// An EDID 1.4 base block: vendor `DEL`, product 0x12AB, and `serial` in a
/// serial number descriptor when given; its checksum set.
fn edid(serial: Option<&str>) -> String {
    let mut b = [0u8; 128];
    b[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
    // D=4, E=5, L=12: 00100 00101 01100.
    let vendor: u16 = (4 << 10) | (5 << 5) | 12;
    b[8..10].copy_from_slice(&vendor.to_be_bytes());
    b[10..12].copy_from_slice(&0x12abu16.to_le_bytes());
    b[16] = 12;
    b[17] = 30;
    b[18] = 1;
    b[19] = 4;
    if let Some(serial) = serial {
        b[72..77].copy_from_slice(&[0, 0, 0, 0xff, 0]);
        let mut text = [b' '; 13];
        text[..serial.len()].copy_from_slice(serial.as_bytes());
        text[serial.len()] = b'\n';
        b[77..90].copy_from_slice(&text);
    }
    let sum = b[..127]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    b[127] = 0u8.wrapping_sub(sum);
    b.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A profile directory `name` under `dir` with `config` and `setup`.
fn profile(dir: &Path, name: &str, config: &str, setup: &str) {
    let at = dir.join(name);
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join("config"), config).unwrap();
    std::fs::write(at.join("setup"), setup).unwrap();
}

fn one(dir: &Path) -> (Vec<Imported>, Vec<Skipped>) {
    import(&read(&[dir.to_owned()]).unwrap(), None)
}

/// The only profile imported from `dir`.
fn imported(dir: &Path) -> Imported {
    let (mut imported, skipped) = one(dir);
    assert_eq!(skipped, [], "nothing skipped");
    assert_eq!(imported.len(), 1);
    imported.remove(0)
}

fn skipped(dir: &Path) -> String {
    let (imported, skipped) = one(dir);
    assert_eq!(imported, [], "nothing imported");
    assert_eq!(skipped.len(), 1);
    skipped[0].reason.clone()
}

/// A config for one output, `DP-1`, with these lines after `output DP-1`,
/// and a setup giving it an EDID with a serial.
fn dp1(dir: &Path, lines: &str) {
    profile(
        dir,
        "p",
        &format!("output DP-1\n{lines}"),
        &format!("DP-1 {}\n", edid(Some("SER1"))),
    );
}

fn dp1_id() -> OutputId {
    OutputId::Edid {
        make: "DEL".into(),
        model: "0x12AB".into(),
        serial: "SER1".into(),
    }
}

#[test]
fn a_saved_profile_becomes_a_layout() {
    // As autorandr's `output_configuration` writes `autorandr --save
    // docked`: outputs by position, the one off first, keys sorted, `crtc`
    // and `rate` from `xrandr --verbose`; `setup` sorted by name.
    let dir = tempfile::tempdir().unwrap();
    profile(
        dir.path(),
        "docked",
        "output HDMI-1\noff\noutput eDP-1\ncrtc 0\nmode 1920x1080\npos 0x0\nprimary\nrate 60.00\noutput DP-1\ncrtc 1\nmode 2560x1440\npos 1920x0\nrate 59.95\nrotate left\n",
        &format!(
            "DP-1 {}\nHDMI-1 {}\neDP-1 {}\n",
            edid(Some("SER1")),
            edid(Some("SER2")),
            edid(None)
        ),
    );
    let docked = imported(dir.path());
    assert_eq!(docked.name.as_str(), "docked");
    assert_eq!(
        docked.from,
        format!("\"docked\" ({})", dir.path().join("docked").display())
    );
    let ids: Vec<&OutputId> = docked.layout.entries.iter().map(|e| &e.id).collect();
    assert_eq!(
        ids,
        [
            &OutputId::Edid {
                make: "DEL".into(),
                model: "0x12AB".into(),
                serial: "SER2".into(),
            },
            &OutputId::Connector("eDP-1".into()),
            &dp1_id(),
        ]
    );
    let states: Vec<&OutputState> = docked.layout.entries.iter().map(|e| &e.state).collect();
    assert_eq!(
        states[0],
        &OutputState {
            enabled: false,
            ..OutputState::default()
        }
    );
    assert_eq!(
        states[1],
        &OutputState {
            mode: Some(Mode::new(1920, 1080, 60_000)),
            primary: true,
            ..OutputState::default()
        }
    );
    assert_eq!(
        states[2],
        &OutputState {
            mode: Some(Mode::new(2560, 1440, 59_950)),
            x: 1920,
            transform: Transform::Rotate90,
            ..OutputState::default()
        }
    );
    assert_eq!(
        docked.notes,
        [
            "eDP-1: its EDID gives no serial; named by its connector here",
            "eDP-1: `crtc 0` not imported: a dsplr layout has no such setting",
            "DP-1: `crtc 1` not imported: a dsplr layout has no such setting",
        ]
    );
}

#[test]
fn rotate_and_reflect_make_the_transform_dsplr_sets_back() {
    use Transform::*;
    let cases = [
        (None, None, Normal),
        (Some("left"), None, Rotate90),
        (Some("inverted"), None, Rotate180),
        (Some("right"), None, Rotate270),
        (None, Some("x"), Flipped),
        (Some("left"), Some("x"), Flipped90),
        (Some("right"), Some("x"), Flipped270),
        // A reflection in y is one in x half a turn further.
        (None, Some("y"), Flipped180),
        (Some("left"), Some("y"), Flipped270),
        (Some("inverted"), Some("y"), Flipped),
        // Both are half a turn.
        (None, Some("xy"), Rotate180),
        (Some("left"), Some("xy"), Rotate270),
        (Some("normal"), Some("normal"), Normal),
    ];
    for (rotate, reflect, expected) in cases {
        assert_eq!(
            transform(rotate, reflect),
            Ok(expected),
            "{rotate:?} {reflect:?}"
        );
    }
    assert!(transform(Some("sideways"), None).is_err());
    assert!(transform(None, Some("z")).is_err());
}

#[test]
fn an_edid_that_names_no_monitor_leaves_the_connector() {
    let dir = tempfile::tempdir().unwrap();
    let config = "output A\nmode 800x600\npos 0x0\nrate 60.00\noutput B\nmode 800x600\npos 800x0\nrate 60.00\noutput C\nmode 800x600\npos 1600x0\nrate 60.00\noutput D\nmode 800x600\npos 2400x0\nrate 60.00\n";
    let pattern = format!("{}*", &edid(Some("SER1"))[..40]);
    let mut broken = edid(Some("SER1"));
    broken.replace_range(20..22, "00");
    profile(
        dir.path(),
        "p",
        config,
        &format!(
            "A --CONNECTED-BUT-EDID-UNAVAILABLE--A\nB {pattern}\nC 0123456789abcdef0123456789abcdef\nD {broken}\n"
        ),
    );
    let p = imported(dir.path());
    for (entry, name) in p.layout.entries.iter().zip(["A", "B", "C", "D"]) {
        assert_eq!(entry.id, OutputId::Connector(name.into()));
    }
    assert_eq!(p.notes.len(), 4, "{:?}", p.notes);
    assert!(
        p.notes[0].starts_with("A: autorandr read no EDID"),
        "{:?}",
        p.notes
    );
    assert!(p.notes[1].starts_with("B: setup matches it by an EDID pattern"));
    assert!(p.notes[2].starts_with("C: setup holds an older autorandr's hash"));
    assert!(p.notes[3].starts_with("D: its EDID cannot be read (the EDID's base block checksum"));
}

#[test]
fn what_a_layout_cannot_say_skips_the_profile() {
    for (lines, said) in [
        (
            "mode 1920x1080\nscale 2x2\n",
            "`scale 2x2` is a RandR transform",
        ),
        (
            "mode 1920x1080\ntransform 2,0,0,0,2,0,0,0,1\n",
            "`transform 2,0,0,0,2,0,0,0,1` is a RandR transform",
        ),
        (
            "mode 1920x1080\nright-of eDP-1\n",
            "`right-of` is not something",
        ),
        ("auto\n", "`auto` is not something"),
        ("pos 0x0\n", "on, but names no mode"),
        ("mode wide\n", "mode `wide` names no size"),
        ("mode 1920x1080\nrate fast\n", "rate `fast` is not a rate"),
        (
            "mode 1920x1080\nrate 60\npos 0,0\n",
            "pos `0,0` is not a position",
        ),
        (
            "mode 1920x1080\nrate 60\nrotate sideways\n",
            "`rotate sideways`",
        ),
        (
            "mode 1920x1080\nrate 60\nsame-as HDMI-1\n",
            "`same-as HDMI-1` names no other output",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        dp1(dir.path(), lines);
        let reason = skipped(dir.path());
        assert!(reason.contains(said), "{said:?} not in {reason:?}");
    }
}

#[test]
fn defaults_and_a_scale_or_transform_of_one_say_nothing() {
    let dir = tempfile::tempdir().unwrap();
    dp1(
        dir.path(),
        "mode 1920x1080\npos 0x0\nrate 60.00\nscale 1x1\ntransform 1.000000,0.000000,0.000000,0.000000,1.000000,0.000000,0.000000,0.000000,1.000000\npanning 0x0\ngamma 1.0:1.0:1.0\nrotate normal\nreflect normal\n",
    );
    let p = imported(dir.path());
    assert_eq!(p.notes, Vec::<String>::new());
    assert_eq!(p.layout.entries[0].id, dp1_id());
}

#[test]
fn what_a_layout_has_no_place_for_is_said_per_output() {
    let dir = tempfile::tempdir().unwrap();
    dp1(
        dir.path(),
        "mode 1920x1080_60.00\npos 0x0\nrate 60.00\ngamma 1.0:0.9:0.8\npanning 3840x2160\nfilter nearest\nx-prop-non_desktop 0\ndpi 96\n",
    );
    let p = imported(dir.path());
    assert_eq!(
        p.notes,
        [
            "DP-1: mode `1920x1080_60.00` taken as 1920x1080",
            "DP-1: `gamma 1.0:0.9:0.8`, `panning 3840x2160`, `filter nearest`, `x-prop-non_desktop 0`, `dpi 96` not imported: a dsplr layout has no such setting",
        ]
    );
}

#[test]
fn same_as_and_one_place_make_mirrors() {
    let dir = tempfile::tempdir().unwrap();
    profile(
        dir.path(),
        "p",
        "output eDP-1\nmode 1920x1080\npos 0x0\nrate 60.00\noutput HDMI-1\nmode 1920x1080\npos 500x500\nrate 60.00\nsame-as eDP-1\noutput DP-1\nmode 1280x720\npos 0x0\nrate 60.00\n",
        "eDP-1 --CONNECTED-BUT-EDID-UNAVAILABLE--eDP-1\nHDMI-1 --CONNECTED-BUT-EDID-UNAVAILABLE--HDMI-1\nDP-1 --CONNECTED-BUT-EDID-UNAVAILABLE--DP-1\n",
    );
    let p = imported(dir.path());
    let edp = OutputId::Connector("eDP-1".into());
    let hdmi = &p.layout.entries[1].state;
    assert_eq!(
        (hdmi.mirror_of.as_ref(), hdmi.x, hdmi.y),
        (Some(&edp), 0, 0),
        "on its source"
    );
    assert_eq!(p.layout.entries[2].state.mirror_of.as_ref(), Some(&edp));
    assert_eq!(
        p.notes.last().map(String::as_str),
        Some("DP-1: at 0,0 as eDP-1 is; a mirror of it here")
    );
}

#[test]
fn same_as_chains_end_on_their_source_and_an_off_source_is_no_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let unavailable = |name: &str| format!("{name} --CONNECTED-BUT-EDID-UNAVAILABLE--{name}\n");
    profile(
        dir.path(),
        "p",
        "output A\nmode 800x600\npos 800x0\nrate 60.00\noutput B\nmode 800x600\nrate 60.00\nsame-as A\noutput C\nmode 800x600\nrate 60.00\nsame-as B\noutput D\noff\noutput E\nmode 800x600\nrate 60.00\nsame-as D\n",
        &["A", "B", "C", "D", "E"].map(unavailable).concat(),
    );
    let p = imported(dir.path());
    let a = OutputId::Connector("A".into());
    let state = |at: usize| &p.layout.entries[at].state;
    // C names B, which names A: both mirror A, where it is.
    for at in [1, 2] {
        assert_eq!(
            (state(at).mirror_of.as_ref(), state(at).x, state(at).y),
            (Some(&a), 800, 0)
        );
    }
    // E names D, which is off: xrandr puts it at 0,0.
    assert_eq!(
        (state(4).mirror_of.as_ref(), state(4).x, state(4).y),
        (None, 0, 0)
    );
    // No `pos` is said of the outputs `same-as` places.
    assert!(
        !p.notes
            .iter()
            .any(|note| note.contains("names no position")),
        "{:?}",
        p.notes
    );
    assert!(
        p.notes
            .iter()
            .any(|note| note.starts_with("E: `same-as` an output that is off")),
        "{:?}",
        p.notes
    );

    let looped = tempfile::tempdir().unwrap();
    profile(
        looped.path(),
        "p",
        "output A\nmode 800x600\nrate 60.00\nsame-as B\noutput B\nmode 800x600\nrate 60.00\nsame-as A\n",
        &["A", "B"].map(unavailable).concat(),
    );
    assert!(skipped(looped.path()).contains("in a loop"));
}

#[test]
fn a_mode_without_a_rate_takes_the_highest_the_output_lists() {
    let dir = tempfile::tempdir().unwrap();
    dp1(dir.path(), "mode 2560x1440\npos 0x0\n");
    let read = read(&[dir.path().to_owned()]).unwrap();
    let (_, refused) = import(&read, None);
    assert_eq!(
        refused[0].reason,
        "DP-1: no connected monitor is DEL 0x12AB SER1 now, so the rate autorandr leaves to it cannot be had; import again where it is connected under that name"
    );
    // Named by its connector, it is said to be missing by that name.
    let by_name = tempfile::tempdir().unwrap();
    profile(
        by_name.path(),
        "p",
        "output HDMI1\nmode 2560x1440\npos 0x0\n",
        "HDMI1 --CONNECTED-BUT-EDID-UNAVAILABLE--HDMI1\n",
    );
    assert!(skipped(by_name.path()).starts_with(
        "HDMI1: no output named HDMI1 is connected now (X drivers and the Wayland desktops name connectors differently)"
    ));

    let live = Layout {
        entries: vec![OutputEntry {
            id: dp1_id(),
            output: Some(Output {
                connector: "DP-1".into(),
                description: "DEL 0x12AB SER1".into(),
                make: "DEL".into(),
                model: "0x12AB".into(),
                serial: "SER1".into(),
                physical_size_mm: None,
                modes: vec![
                    Mode::new(2560, 1440, 59_951),
                    Mode::new(2560, 1440, 143_998),
                    Mode::new(1920, 1080, 240_000),
                ],
            }),
            state: OutputState::default(),
        }],
        ..Layout::default()
    };
    let (imported, _) = import(&read, Some(&live));
    assert_eq!(
        imported[0].layout.entries[0].state.mode,
        Some(Mode::new(2560, 1440, 143_998))
    );
    assert!(
        imported[0].notes[0].contains("names no rate"),
        "{:?}",
        imported[0].notes
    );
}

#[cfg(unix)]
#[test]
fn names_scripts_and_dropped_outputs_are_said() {
    let dir = tempfile::tempdir().unwrap();
    profile(
        dir.path(),
        "at work",
        "output VGA-1\noff\noutput DP-1\nmode 640x480\npos 0x0\nrate 60.00\n",
        &format!("DP-1 {}\n", edid(Some("SER1"))),
    );
    // Executable, as autorandr runs only those.
    for script in ["at work/postswitch", "at work/preswitch.d/notify"] {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.path().join(script);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    profile(
        dir.path(),
        "home@desk",
        "output DP-1\nmode 640x480\npos 0x0\nrate 60.00\n",
        &format!("DP-1 {}\n", edid(Some("SER1"))),
    );
    profile(
        dir.path(),
        "home_desk",
        "output DP-1\nmode 640x480\npos 0x0\nrate 60.00\n",
        &format!("DP-1 {}\n", edid(Some("SER1"))),
    );
    let (imported, skipped) = one(dir.path());
    let names: Vec<&str> = imported.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["at work", "home_desk"]);
    assert_eq!(
        imported[0].notes,
        [
            "VGA-1: off and with no EDID in setup, which autorandr leaves out too",
            "its preswitch.d/notify, postswitch scripts are not run by dsplr",
        ]
    );
    assert_eq!(
        imported[1].notes,
        ["named \"home_desk\" here: \"home@desk\" is not a dsplr profile name"]
    );
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].from.starts_with("\"home_desk\""));
    assert!(skipped[0].reason.contains("imported as \"home_desk\" too"));
}

#[test]
fn a_profile_autorandr_cannot_load_is_skipped_with_why() {
    let dir = tempfile::tempdir().unwrap();
    profile(
        dir.path(),
        "p",
        "# saved\noutput DP-1\nmode 640x480\n",
        "DP-1 00\n",
    );
    let reason = skipped(dir.path());
    assert!(
        reason.contains("before its first `output` line"),
        "{reason}"
    );
}

#[test]
fn an_output_list_built_by_hand_is_taken_as_read() {
    // `import` reads only what `read` gives it.
    let read = Read {
        profiles: vec![AutorandrProfile {
            name: "p".into(),
            dir: "/nowhere/p".into(),
            outputs: vec![AutorandrOutput {
                name: "DP-1".into(),
                edid: edid(Some("SER1")),
                options: vec![
                    ("mode".into(), Some("800x600".into())),
                    ("rate".into(), Some("75.00".into())),
                    ("pos".into(), Some("-800x0".into())),
                ],
            }],
            dropped: vec![],
            scripts: vec![],
        }],
        ..Read::default()
    };
    let (imported, _) = import(&read, None);
    let state = &imported[0].layout.entries[0].state;
    assert_eq!((state.x, state.y), (-800, 0));
    assert_eq!(state.mode, Some(Mode::new(800, 600, 75_000)));
}
