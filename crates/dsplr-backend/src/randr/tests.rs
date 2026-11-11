//! The planner against recorded and hand-built snapshots.
//!
//! The Xvfb snapshots were recorded from Xvfb 21.1.26 started as `Xvfb :97
//! -screen 0 1920x1080x24 -noreset` (one CRTC, one output, `screen`), read
//! with libXrandr's `XRRGetScreenResourcesCurrent`, `XRRGetOutputInfo`,
//! `XRRGetCrtcInfo`, `XRRGetPanning` and `XRRGetScreenSizeRange`. Where a
//! test says xrandr agrees, `xrandr --dryrun --verbose` printed the same
//! steps on that server, and where it says the server took it, the requests
//! were sent and the snapshot after them is the one recorded.
//!
//! The rest are hand-built: a laptop with three CRTCs that every output can
//! use and no clones (as Intel's driver reports), and an older card whose
//! two outputs clone each other.

use dsplr_model::{Layout, Mode, ModeHandle, OutputEntry, OutputId, OutputState, Transform};

use super::plan::{ModeRef, Note, Operation, PlanError, plan, rotation};
use super::resources::{
    CrtcInfo, CrtcXid, ModeInfo, ModeXid, OutputInfo, OutputXid, Panning, Rotation,
    ScreenResources, ScreenSize, SizeRange, Snapshot, mode_flags,
};

const SCREEN: OutputXid = OutputXid(0x3c);
const XVFB_CRTC: CrtcXid = CrtcXid(0x3b);
const XVFB_1080: ModeXid = ModeXid(0x3a);
const XVFB_720: ModeXid = ModeXid(0x3a9);

/// Xvfb's one mode: a size with no timings.
fn xvfb_1080() -> ModeInfo {
    ModeInfo {
        id: XVFB_1080,
        width: 1920,
        height: 1080,
        name: "1920x1080".into(),
        ..ModeInfo::default()
    }
}

/// The mode `cvt 1280 720 60` gives, as Xvfb held it once created.
fn xvfb_720() -> ModeInfo {
    ModeInfo {
        id: XVFB_720,
        width: 1280,
        height: 720,
        dot_clock: 74_500_000,
        hsync_start: 1344,
        hsync_end: 1472,
        htotal: 1664,
        hskew: 0,
        vsync_start: 723,
        vsync_end: 728,
        vtotal: 748,
        flags: mode_flags::HSYNC_NEGATIVE | mode_flags::VSYNC_POSITIVE,
        name: "1280x720_60.00".into(),
    }
}

/// Xvfb as it starts: `screen` on its CRTC at 1920x1080, 488x274 mm.
fn xvfb() -> Snapshot {
    Snapshot {
        resources: ScreenResources {
            timestamp: 117_196_004,
            config_timestamp: 117_196_004,
            crtcs: vec![XVFB_CRTC],
            outputs: vec![SCREEN],
            modes: vec![xvfb_1080()],
        },
        outputs: vec![OutputInfo {
            id: SCREEN,
            timestamp: 117_196_004,
            crtc: Some(XVFB_CRTC),
            crtcs: vec![XVFB_CRTC],
            modes: vec![XVFB_1080],
            name: "screen".into(),
            ..OutputInfo::default()
        }],
        crtcs: vec![CrtcInfo {
            id: XVFB_CRTC,
            timestamp: 117_196_004,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            mode: Some(XVFB_1080),
            rotation: Rotation::ROTATE_0,
            rotations: Rotation::ROTATE_0,
            outputs: vec![SCREEN],
            possible: vec![SCREEN],
            panning: None,
            transform: None,
        }],
        size_range: SizeRange {
            min_width: 1,
            min_height: 1,
            max_width: 1920,
            max_height: 1080,
        },
        screen: ScreenSize {
            width: 1920,
            height: 1080,
            mm_width: 488,
            mm_height: 274,
        },
        primary: None,
    }
}

/// Xvfb after `cvt 1280 720 60`'s mode was created and added to `screen`.
fn xvfb_with_720() -> Snapshot {
    let mut snapshot = xvfb();
    snapshot.resources.modes.push(xvfb_720());
    snapshot.outputs[0].modes.push(XVFB_720);
    snapshot
}

/// [`xvfb_with_720`] after `xrandr --output screen --mode 1280x720_60.00`.
fn xvfb_at_720() -> Snapshot {
    let mut snapshot = xvfb_with_720();
    let crtc = &mut snapshot.crtcs[0];
    (crtc.width, crtc.height, crtc.mode) = (1280, 720, Some(XVFB_720));
    snapshot.screen = ScreenSize {
        width: 1280,
        height: 720,
        mm_width: 324,
        mm_height: 182,
    };
    snapshot
}

/// The plan's requests, checking it notes nothing.
fn ops(snapshot: &Snapshot, layout: &Layout) -> Result<Vec<Operation>, PlanError> {
    plan(snapshot, layout).map(|plan| {
        assert_eq!(plan.notes, [], "unexpected notes");
        plan.operations
    })
}

fn mode(width: u32, height: u32, refresh_mhz: u32) -> Mode {
    Mode::new(width, height, refresh_mhz)
}

/// An entry naming `connector`, on at `x`,`y` in `mode`.
fn on(connector: &str, mode: Mode, x: i32, y: i32) -> OutputEntry {
    OutputEntry {
        id: OutputId::Connector(connector.into()),
        output: None,
        state: OutputState {
            mode: Some(mode),
            x,
            y,
            ..OutputState::default()
        },
    }
}

/// An entry turning `connector` off.
fn off(connector: &str) -> OutputEntry {
    OutputEntry {
        id: OutputId::Connector(connector.into()),
        output: None,
        state: OutputState {
            enabled: false,
            ..OutputState::default()
        },
    }
}

fn layout(entries: Vec<OutputEntry>) -> Layout {
    Layout {
        entries,
        ..Layout::default()
    }
}

/// `operations` between the grab and the ungrab, checking both are there.
fn grabbed(operations: &[Operation]) -> &[Operation] {
    let grab = operations
        .iter()
        .position(|op| *op == Operation::GrabServer)
        .expect("grabs the server");
    assert_eq!(operations.last(), Some(&Operation::UngrabServer));
    &operations[grab + 1..operations.len() - 1]
}

fn set_crtc(crtc: CrtcXid, mode: ModeRef, x: i16, y: i16, outputs: &[OutputXid]) -> Operation {
    Operation::SetCrtcConfig {
        crtc,
        mode,
        x,
        y,
        rotation: Rotation::ROTATE_0,
        outputs: outputs.to_vec(),
    }
}

const fn screen_size(width: u16, height: u16, mm_width: u32, mm_height: u32) -> Operation {
    Operation::SetScreenSize {
        width,
        height,
        mm_width,
        mm_height,
    }
}

#[test]
fn xvfb_as_it_is_sends_nothing_but_the_grab() {
    let snapshot = xvfb();
    let operations = ops(
        &snapshot,
        &layout(vec![on("screen", mode(1920, 1080, 0), 0, 0)]),
    );
    assert_eq!(
        operations,
        Ok(vec![Operation::GrabServer, Operation::UngrabServer])
    );
}

#[test]
fn xvfb_turned_off_shrinks_to_the_minimum() {
    // xrandr agrees: "crtc 0: disable", "screen 0: 1x1 0x0 mm".
    let operations = ops(&xvfb(), &layout(vec![off("screen")])).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [Operation::DisableCrtc(XVFB_CRTC), screen_size(1, 1, 0, 0)]
    );
}

#[test]
fn xvfb_to_a_smaller_listed_mode_keeps_its_dpi() {
    // xrandr agrees ("screen 0: 1280x720 324x182 mm 100.12dpi"), and the
    // server took it: the snapshot after is `xvfb_at_720`.
    let asked = mode(1280, 720, xvfb_720().refresh_mhz());
    let operations =
        ops(&xvfb_with_720(), &layout(vec![on("screen", asked, 0, 0)])).expect("plans");
    assert_eq!(
        operations,
        [
            Operation::GrabServer,
            Operation::DisableCrtc(XVFB_CRTC),
            screen_size(1280, 720, 324, 182),
            set_crtc(XVFB_CRTC, ModeRef::Listed(XVFB_720), 0, 0, &[SCREEN]),
            Operation::UngrabServer,
        ]
    );
}

#[test]
fn xvfb_back_to_its_size_takes_the_dpi_of_the_smaller_one() {
    // xrandr agrees: "screen 0: 1920x1080 485x273 mm 100.48dpi", the
    // millimetres drifting with the truncation each time.
    let operations = ops(
        &xvfb_at_720(),
        &layout(vec![on("screen", mode(1920, 1080, 0), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(XVFB_CRTC),
            screen_size(1920, 1080, 485, 273),
            set_crtc(XVFB_CRTC, ModeRef::Listed(XVFB_1080), 0, 0, &[SCREEN]),
        ]
    );
}

#[test]
fn xvfb_given_an_unlisted_mode_creates_cvts() {
    // The created mode is the one the server took (`xvfb_720`, with the id
    // it gave), and the rest is what xrandr then did with it.
    let operations = ops(
        &xvfb(),
        &layout(vec![on("screen", mode(1280, 720, 60_000), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(
        operations,
        [
            Operation::CreateMode(ModeInfo {
                id: ModeXid(0),
                ..xvfb_720()
            }),
            Operation::AddOutputMode {
                output: SCREEN,
                mode: ModeRef::Created(0),
            },
            Operation::GrabServer,
            Operation::DisableCrtc(XVFB_CRTC),
            screen_size(1280, 720, 324, 182),
            set_crtc(XVFB_CRTC, ModeRef::Created(0), 0, 0, &[SCREEN]),
            Operation::UngrabServer,
        ]
    );
}

#[test]
fn a_cvt_mode_made_before_is_used_again() {
    // 60 Hz asked, 59.855 Hz made: the listed mode is not near enough to be
    // the one asked, but it is the one CVT makes for it.
    let operations = ops(
        &xvfb_with_720(),
        &layout(vec![on("screen", mode(1280, 720, 60_000), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(operations[0], Operation::GrabServer);
    assert_eq!(
        grabbed(&operations)[2],
        set_crtc(XVFB_CRTC, ModeRef::Listed(XVFB_720), 0, 0, &[SCREEN])
    );
}

#[test]
fn a_mode_name_taken_by_other_timings_is_refused() {
    let mut snapshot = xvfb_with_720();
    snapshot.resources.modes[1].htotal += 8;
    let refused = ops(
        &snapshot,
        &layout(vec![on("screen", mode(1280, 720, 60_000), 0, 0)]),
    );
    assert!(
        matches!(refused, Err(PlanError::ModeUnavailable { ref connector, .. }) if connector == "screen"),
        "{refused:?}"
    );
}

#[test]
fn a_mode_cvt_cannot_make_is_refused() {
    let refused = ops(
        &xvfb(),
        &layout(vec![on("screen", mode(1279, 720, 60_000), 0, 0)]),
    );
    assert!(
        matches!(refused, Err(PlanError::ModeUnavailable { .. })),
        "{refused:?}"
    );
}

#[test]
fn a_screen_above_the_maximum_is_refused() {
    let refused = ops(
        &xvfb(),
        &layout(vec![on("screen", mode(2560, 1440, 60_000), 0, 0)]),
    );
    assert_eq!(
        refused,
        Err(PlanError::ScreenTooLarge {
            width: 2560,
            height: 1440,
            max_width: 1920,
            max_height: 1080,
        })
    );
}

#[test]
fn a_rotation_the_crtc_cannot_do_is_refused() {
    let mut entry = on("screen", mode(1920, 1080, 0), 0, 0);
    entry.state.transform = Transform::Rotate90;
    assert_eq!(
        ops(&xvfb(), &layout(vec![entry])),
        Err(PlanError::Rotation {
            connector: "screen".into(),
            transform: Transform::Rotate90,
        })
    );
}

#[test]
fn a_reflection_one_crtc_cannot_do_is_refused() {
    // Every CRTC eDP-1 could use must do it, as xrandr checks.
    let mut snapshot = laptop();
    snapshot.crtcs[2].rotations = Rotation(0xf);
    let mut entry = on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0);
    entry.state.transform = Transform::Flipped;
    assert_eq!(
        ops(&snapshot, &layout(vec![entry])),
        Err(PlanError::Rotation {
            connector: "eDP-1".into(),
            transform: Transform::Flipped,
        })
    );
}

#[test]
fn settings_x11_has_not_are_refused() {
    type Set = fn(&mut OutputState);
    let cases: [(Set, &str); 6] = [
        (|state| state.scale = 2.0, "a scale other than 1"),
        (
            |state| state.colour_preset = Some("srgb".into()),
            "a colour preset",
        ),
        (|state| state.bit_depth = Some(8), "bit depth"),
        (|state| state.overscan = Some(0), "overscan"),
        (
            |state| state.rgb_range = Some(dsplr_model::RgbRange::Full),
            "RGB range",
        ),
        (
            |state| state.adaptive_sync = Some(dsplr_model::AdaptiveSync::On),
            "adaptive sync",
        ),
    ];
    for (set, what) in cases {
        let mut entry = on("screen", mode(1920, 1080, 0), 0, 0);
        set(&mut entry.state);
        assert_eq!(
            ops(&xvfb(), &layout(vec![entry])),
            Err(PlanError::Unsupported {
                connector: "screen".into(),
                what,
            })
        );
    }
}

#[test]
fn a_layout_away_from_the_origin_is_moved_to_it() {
    // xrandr agrees that it lands at +0+0, though it sets the CRTC again.
    let operations = ops(
        &xvfb(),
        &layout(vec![on("screen", mode(1920, 1080, 0), 10, 0)]),
    );
    assert_eq!(
        operations,
        Ok(vec![Operation::GrabServer, Operation::UngrabServer])
    );
}

#[test]
fn the_primary_output_is_set_when_it_changes() {
    let mut entry = on("screen", mode(1920, 1080, 0), 0, 0);
    entry.state.primary = true;
    let operations = ops(&xvfb(), &layout(vec![entry.clone()])).expect("plans");
    assert_eq!(grabbed(&operations), [Operation::SetOutputPrimary(SCREEN)]);

    let mut snapshot = xvfb();
    snapshot.primary = Some(SCREEN);
    let operations = ops(&snapshot, &layout(vec![entry])).expect("plans");
    assert_eq!(grabbed(&operations), []);
}

#[test]
fn a_layout_with_no_primary_leaves_the_primary_alone() {
    // A profile from a desktop without a primary output says false for
    // every output, which is not `--noprimary`.
    let mut snapshot = laptop();
    snapshot.primary = Some(EDP);
    let operations = ops(&snapshot, &layout(docked_layout())).expect("plans");
    assert!(
        !operations
            .iter()
            .any(|op| matches!(op, Operation::SetOutputPrimary(_))),
        "{operations:?}"
    );

    // Making another output primary moves it there.
    let mut entries = docked_layout();
    entries[1].state.primary = true;
    let operations = ops(&snapshot, &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations).last(),
        Some(&Operation::SetOutputPrimary(HDMI))
    );
}

#[test]
fn entries_must_name_connected_outputs_once_with_a_mode() {
    let snapshot = xvfb();
    let entry = on("DP-9", mode(1920, 1080, 0), 0, 0);
    assert_eq!(
        ops(&snapshot, &layout(vec![entry.clone()])),
        Err(PlanError::NotConnected(entry.id))
    );
    let entry = on("screen", mode(1920, 1080, 0), 0, 0);
    assert_eq!(
        ops(&snapshot, &layout(vec![entry.clone(), entry.clone()])),
        Err(PlanError::Twice("screen".into()))
    );
    let mut entry = entry;
    entry.state.mode = None;
    assert_eq!(
        ops(&snapshot, &layout(vec![entry])),
        Err(PlanError::NoMode("screen".into()))
    );
}

const C1: CrtcXid = CrtcXid(0x40);
const C2: CrtcXid = CrtcXid(0x41);
const C3: CrtcXid = CrtcXid(0x42);
const EDP: OutputXid = OutputXid(0x50);
const HDMI: OutputXid = OutputXid(0x51);
const DP1: OutputXid = OutputXid(0x52);
const DP2: OutputXid = OutputXid(0x53);
const VGA: OutputXid = OutputXid(0x54);
/// CEA-861's 1920x1080 at 60 Hz.
const M1080: ModeXid = ModeXid(0x60);
/// 2560x1440 at 59.951 Hz, CVT reduced blanking.
const M1440: ModeXid = ModeXid(0x61);
/// VESA's 1024x768 at 60.004 Hz.
const M768: ModeXid = ModeXid(0x62);
/// 1280x720 at 60 Hz, CEA-861.
const M720: ModeXid = ModeXid(0x63);

#[allow(clippy::too_many_arguments)]
fn timings(
    id: ModeXid,
    (width, height): (u16, u16),
    dot_clock: u32,
    [hsync_start, hsync_end, htotal]: [u16; 3],
    [vsync_start, vsync_end, vtotal]: [u16; 3],
    name: &str,
) -> ModeInfo {
    ModeInfo {
        id,
        width,
        height,
        dot_clock,
        hsync_start,
        hsync_end,
        htotal,
        hskew: 0,
        vsync_start,
        vsync_end,
        vtotal,
        flags: mode_flags::HSYNC_POSITIVE | mode_flags::VSYNC_POSITIVE,
        name: name.into(),
    }
}

fn laptop_modes() -> Vec<ModeInfo> {
    vec![
        timings(
            M1080,
            (1920, 1080),
            148_500_000,
            [2008, 2052, 2200],
            [1084, 1089, 1125],
            "1920x1080",
        ),
        timings(
            M1440,
            (2560, 1440),
            241_500_000,
            [2608, 2640, 2720],
            [1443, 1448, 1481],
            "2560x1440",
        ),
        timings(
            M768,
            (1024, 768),
            65_000_000,
            [1048, 1184, 1344],
            [771, 777, 806],
            "1024x768",
        ),
        timings(
            M720,
            (1280, 720),
            74_250_000,
            [1390, 1430, 1650],
            [725, 730, 750],
            "1280x720",
        ),
    ]
}

fn output(id: OutputXid, name: &str, crtcs: &[CrtcXid], modes: &[ModeXid]) -> OutputInfo {
    OutputInfo {
        id,
        crtcs: crtcs.to_vec(),
        modes: modes.to_vec(),
        num_preferred: 1,
        name: name.into(),
        ..OutputInfo::default()
    }
}

fn crtc(id: CrtcXid, possible: &[OutputXid]) -> CrtcInfo {
    CrtcInfo {
        id,
        rotation: Rotation::ROTATE_0,
        rotations: Rotation(0x3f),
        possible: possible.to_vec(),
        ..CrtcInfo::default()
    }
}

/// Puts `output` on `crtc` at `x`,`y` in `mode`.
fn drive(snapshot: &mut Snapshot, crtc: CrtcXid, output: OutputXid, mode: ModeXid, x: i16, y: i16) {
    let info = snapshot
        .resources
        .modes
        .iter()
        .find(|info| info.id == mode)
        .expect("a mode of the snapshot")
        .clone();
    let state = snapshot
        .crtcs
        .iter_mut()
        .find(|state| state.id == crtc)
        .expect("a CRTC of the snapshot");
    (state.x, state.y, state.width, state.height) = (x, y, info.width, info.height);
    state.mode = Some(mode);
    state.outputs.push(output);
    snapshot
        .outputs
        .iter_mut()
        .find(|info| info.id == output)
        .expect("an output of the snapshot")
        .crtc = Some(crtc);
}

/// A laptop at 1920x1080, 508x286 mm (96 DPI): eDP-1 on the first of three
/// CRTCs, HDMI-1, DP-1 and DP-2 off. Every output can use every CRTC, none
/// clones another.
fn laptop() -> Snapshot {
    let all = [C1, C2, C3];
    let outputs = [EDP, HDMI, DP1, DP2];
    let mut snapshot = Snapshot {
        resources: ScreenResources {
            crtcs: all.to_vec(),
            outputs: outputs.to_vec(),
            modes: laptop_modes(),
            ..ScreenResources::default()
        },
        outputs: vec![
            output(EDP, "eDP-1", &all, &[M1080, M720]),
            output(HDMI, "HDMI-1", &all, &[M1440, M1080]),
            output(DP1, "DP-1", &all, &[M1440, M1080]),
            output(DP2, "DP-2", &all, &[M1080]),
        ],
        crtcs: all.iter().map(|&id| crtc(id, &outputs)).collect(),
        size_range: SizeRange {
            min_width: 320,
            min_height: 200,
            max_width: 16384,
            max_height: 16384,
        },
        screen: ScreenSize {
            width: 1920,
            height: 1080,
            mm_width: 508,
            mm_height: 286,
        },
        primary: None,
    };
    drive(&mut snapshot, C1, EDP, M1080, 0, 0);
    snapshot
}

/// [`laptop`] with HDMI-1 and DP-1 on, at 2560x1440 right of it in turn:
/// 7040x1440, 1864x381 mm.
fn docked() -> Snapshot {
    let mut snapshot = laptop();
    drive(&mut snapshot, C2, HDMI, M1440, 1920, 0);
    drive(&mut snapshot, C3, DP1, M1440, 4480, 0);
    snapshot.screen = ScreenSize {
        width: 7040,
        height: 1440,
        mm_width: 1864,
        mm_height: 381,
    };
    snapshot
}

fn hz(id: ModeXid) -> u32 {
    laptop_modes()
        .iter()
        .find(|info| info.id == id)
        .expect("a laptop mode")
        .refresh_mhz()
}

fn docked_layout() -> Vec<OutputEntry> {
    vec![
        on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0),
        on("HDMI-1", mode(2560, 1440, hz(M1440)), 1920, 0),
        on("DP-1", mode(2560, 1440, hz(M1440)), 4480, 0),
    ]
}

#[test]
fn two_externals_take_the_free_crtcs_in_turn() {
    // The millimetres are C's `(int)((25.4 * 7040) / ((25.4 * 1080) /
    // 286))` and its height's, computed by a C program as xrandr does.
    let operations = ops(&laptop(), &layout(docked_layout())).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            screen_size(7040, 1440, 1864, 381),
            set_crtc(C2, ModeRef::Listed(M1440), 1920, 0, &[HDMI]),
            set_crtc(C3, ModeRef::Listed(M1440), 4480, 0, &[DP1]),
        ]
    );
}

#[test]
fn the_docked_layout_applied_back_sends_nothing() {
    let operations = ops(&docked(), &layout(docked_layout())).expect("plans");
    assert_eq!(grabbed(&operations), []);
}

#[test]
fn turning_an_external_off_disables_its_crtc_and_shrinks_the_screen() {
    let mut entries = docked_layout();
    entries[2] = off("DP-1");
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    // 1185, not 1186: the DPI now comes from 1440 lines in 381 mm.
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(C3),
            screen_size(4480, 1440, 1185, 381)
        ]
    );
}

#[test]
fn an_output_left_out_of_the_layout_keeps_its_crtc() {
    let mut entries = docked_layout();
    entries.remove(2);
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    assert_eq!(grabbed(&operations), []);
}

#[test]
fn a_crtc_that_no_longer_fits_is_turned_off_before_the_screen_shrinks() {
    // HDMI-1 turned to portrait: 1440x2560 at 1920,0. Its CRTC keeps its
    // mode, but at its old size it would reach x 4480 on a 3360 screen.
    let mut entries = docked_layout();
    entries[1].state.transform = Transform::Rotate90;
    entries[2] = off("DP-1");
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(C2),
            Operation::DisableCrtc(C3),
            screen_size(3360, 2560, 889, 677),
            Operation::SetCrtcConfig {
                crtc: C2,
                mode: ModeRef::Listed(M1440),
                x: 1920,
                y: 0,
                rotation: Rotation::ROTATE_90,
                outputs: vec![HDMI],
            },
        ]
    );
}

#[test]
fn a_crtc_turned_is_measured_by_its_old_rotation() {
    // HDMI-1 turned to portrait with DP-1 still right of it: at its old
    // rotation its mode still has the size it had and fits, so xrandr sets
    // it without turning it off first.
    let mut entries = docked_layout();
    entries[1].state.transform = Transform::Rotate90;
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            screen_size(7040, 2560, 1862, 677),
            Operation::SetCrtcConfig {
                crtc: C2,
                mode: ModeRef::Listed(M1440),
                x: 1920,
                y: 0,
                rotation: Rotation::ROTATE_90,
                outputs: vec![HDMI],
            },
        ]
    );
}

#[test]
fn a_crtc_moving_within_the_screen_is_not_turned_off_first() {
    // HDMI-1 and DP-1 trade places: same mode, both fit, so each CRTC is
    // set where it is.
    let mut entries = docked_layout();
    (entries[1].state.x, entries[2].state.x) = (4480, 1920);
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            set_crtc(C2, ModeRef::Listed(M1440), 4480, 0, &[HDMI]),
            set_crtc(C3, ModeRef::Listed(M1440), 1920, 0, &[DP1]),
        ]
    );
}

#[test]
fn more_outputs_than_crtcs_is_refused() {
    let mut entries = docked_layout();
    entries.push(on("DP-2", mode(1920, 1080, hz(M1080)), 7040, 0));
    // The simple way gives HDMI-1 and DP-1 the free CRTCs and finds none
    // for DP-2; the search then prefers what the simple way gave them.
    assert_eq!(
        ops(&laptop(), &layout(entries)),
        Err(PlanError::NoCrtc("DP-2".into()))
    );
}

#[test]
fn an_output_moves_to_another_crtc_when_a_new_one_needs_its() {
    // VGA-1 can only use the first CRTC, which eDP-1 has; eDP-1 can use the
    // second, so the search moves it there.
    let mut snapshot = laptop();
    snapshot.resources.outputs.push(VGA);
    snapshot.outputs[0].crtcs = vec![C1, C2];
    snapshot.outputs.push(output(VGA, "VGA-1", &[C1], &[M768]));
    let entries = vec![
        on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0),
        on("VGA-1", mode(1024, 768, hz(M768)), 1920, 0),
    ];
    let operations = ops(&snapshot, &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(C1),
            screen_size(2944, 1080, 779, 286),
            set_crtc(C1, ModeRef::Listed(M768), 1920, 0, &[VGA]),
            set_crtc(C2, ModeRef::Listed(M1080), 0, 0, &[EDP]),
        ]
    );
}

#[test]
fn a_mirror_without_clones_gets_its_own_crtc_at_the_same_place() {
    let mut mirror = on("HDMI-1", mode(1920, 1080, hz(M1080)), 500, 500);
    mirror.state.mirror_of = Some(OutputId::Connector("eDP-1".into()));
    let entries = vec![on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0), mirror];
    let operations = ops(&laptop(), &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [set_crtc(C2, ModeRef::Listed(M1080), 0, 0, &[HDMI])]
    );
}

#[test]
fn a_mirror_of_an_output_off_is_refused() {
    let mut mirror = on("HDMI-1", mode(1920, 1080, hz(M1080)), 0, 0);
    mirror.state.mirror_of = Some(OutputId::Connector("eDP-1".into()));
    let entries = vec![off("eDP-1"), mirror];
    assert_eq!(
        ops(&laptop(), &layout(entries)),
        Err(PlanError::MirrorSource("HDMI-1".into()))
    );
}

const VGA0: OutputXid = OutputXid(0x70);
const DVI0: OutputXid = OutputXid(0x71);
const D1: CrtcXid = CrtcXid(0x72);
const D2: CrtcXid = CrtcXid(0x73);

/// An older card at 1024x768: VGA-0 and DVI-0 clone each other and can use
/// either CRTC; VGA-0 is on the first. DVI-0 also lists 1280x720.
fn cloning() -> Snapshot {
    let both = [D1, D2];
    let mut vga = output(VGA0, "VGA-0", &both, &[M768]);
    vga.clones = vec![DVI0];
    let mut dvi = output(DVI0, "DVI-0", &both, &[M768, M720]);
    dvi.clones = vec![VGA0];
    let mut snapshot = Snapshot {
        resources: ScreenResources {
            crtcs: both.to_vec(),
            outputs: vec![VGA0, DVI0],
            modes: laptop_modes(),
            ..ScreenResources::default()
        },
        outputs: vec![vga, dvi],
        crtcs: both.iter().map(|&id| crtc(id, &[VGA0, DVI0])).collect(),
        size_range: SizeRange {
            min_width: 320,
            min_height: 200,
            max_width: 4096,
            max_height: 4096,
        },
        screen: ScreenSize {
            width: 1024,
            height: 768,
            mm_width: 270,
            mm_height: 203,
        },
        primary: None,
    };
    drive(&mut snapshot, D1, VGA0, M768, 0, 0);
    snapshot
}

fn mirroring(mode: Mode) -> Layout {
    let mut mirror = on("DVI-0", mode, 0, 0);
    mirror.state.mirror_of = Some(OutputId::Connector("VGA-0".into()));
    layout(vec![
        on("VGA-0", Mode::new(1024, 768, hz(M768)), 0, 0),
        mirror,
    ])
}

#[test]
fn clones_in_the_same_mode_share_a_crtc() {
    let operations = ops(&cloning(), &mirroring(mode(1024, 768, hz(M768)))).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [set_crtc(D1, ModeRef::Listed(M768), 0, 0, &[VGA0, DVI0])]
    );
}

#[test]
fn clones_in_different_modes_take_a_crtc_each() {
    let operations = ops(&cloning(), &mirroring(mode(1280, 720, hz(M720)))).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            screen_size(1280, 768, 338, 203),
            set_crtc(D2, ModeRef::Listed(M720), 0, 0, &[DVI0]),
        ]
    );
}

/// Where a CRTC driving a `width` by `height` mode turned by `rotation`
/// takes its pixel `(u, v)` from in the framebuffer: the map
/// `RRTransformCompute` builds (`randr/rrtransform.c`, X server 21.1.26),
/// which rotates, then reflects in the rotated frame.
fn randr_source(rotation: Rotation, (width, height): (i64, i64), (u, v): (i64, i64)) -> (i64, i64) {
    let direction = rotation.direction();
    let (x, y) = match direction {
        Rotation::ROTATE_0 => (u, v),
        Rotation::ROTATE_90 => (height - v, u),
        Rotation::ROTATE_180 => (width - u, height - v),
        Rotation::ROTATE_270 => (v, width - u),
        other => panic!("no direction in {other:?}"),
    };
    let (across, down) = if direction.swaps_axes() {
        (height, width)
    } else {
        (width, height)
    };
    let reflects = |bit: Rotation| rotation.reflection().0 & bit.0 != 0;
    let x = if reflects(Rotation::REFLECT_X) {
        across - x
    } else {
        x
    };
    let y = if reflects(Rotation::REFLECT_Y) {
        down - y
    } else {
        y
    };
    (x, y)
}

/// Where a Wayland compositor puts the point `(x, y)` of an output's
/// `width` by `height` content in the output's buffer under `transform`:
/// wlroots 0.18.2's `transform_output_box` (`types/scene/wlr_scene.c`),
/// which is `wlr_box_transform` (`util/box.c`) by the inverse transform
/// (`wlr_output_transform_invert`), for a box of no size.
fn wayland_destination(
    transform: Transform,
    (width, height): (i64, i64),
    (x, y): (i64, i64),
) -> (i64, i64) {
    let inverse = match transform {
        Transform::Rotate90 => Transform::Rotate270,
        Transform::Rotate270 => Transform::Rotate90,
        other => other,
    };
    match inverse {
        Transform::Normal => (x, y),
        Transform::Rotate90 => (height - y, x),
        Transform::Rotate180 => (width - x, height - y),
        Transform::Rotate270 => (y, width - x),
        Transform::Flipped => (width - x, y),
        Transform::Flipped90 => (y, x),
        Transform::Flipped180 => (x, height - y),
        Transform::Flipped270 => (height - y, width - x),
    }
}

#[test]
fn each_transform_shows_what_randr_shows_for_its_rotation() {
    // A 1920x1080 mode; content points away from every axis of symmetry.
    let mode = (1920, 1080);
    for transform in Transform::ALL {
        let content = if transform.swaps_axes() {
            (mode.1, mode.0)
        } else {
            mode
        };
        for point in [(0, 0), (100, 7), (1000, 3), (5, 900)] {
            let shown = wayland_destination(transform, content, point);
            assert_eq!(
                randr_source(rotation(transform), mode, shown),
                point,
                "{transform:?} at {point:?}"
            );
        }
    }
}

#[test]
fn a_flipped_transform_is_a_reflection_in_x() {
    assert_eq!(rotation(Transform::Normal), Rotation::ROTATE_0);
    assert_eq!(rotation(Transform::Rotate270), Rotation::ROTATE_270);
    assert_eq!(
        rotation(Transform::Flipped90),
        Rotation::ROTATE_90 | Rotation::REFLECT_X
    );
    assert_eq!(
        rotation(Transform::Flipped180),
        Rotation::ROTATE_180 | Rotation::REFLECT_X
    );

    let mut entries = docked_layout();
    entries[0].state.transform = Transform::Flipped;
    let operations = ops(&docked(), &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [Operation::SetCrtcConfig {
            crtc: C1,
            mode: ModeRef::Listed(M1080),
            x: 0,
            y: 0,
            rotation: Rotation::ROTATE_0 | Rotation::REFLECT_X,
            outputs: vec![EDP],
        }]
    );
}

#[test]
fn a_mode_on_the_screen_but_not_the_output_is_added_to_it() {
    let entries = vec![on("eDP-1", mode(2560, 1440, hz(M1440)), 0, 0)];
    let operations = ops(&laptop(), &layout(entries)).expect("plans");
    assert_eq!(
        operations[0],
        Operation::AddOutputMode {
            output: EDP,
            mode: ModeRef::Listed(M1440),
        }
    );
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(C1),
            screen_size(2560, 1440, 677, 381),
            set_crtc(C1, ModeRef::Listed(M1440), 0, 0, &[EDP]),
        ]
    );
}

#[test]
fn one_custom_mode_asked_twice_is_created_once() {
    let custom = mode(1600, 900, 60_000);
    let entries = vec![
        on("HDMI-1", custom.clone(), 0, 0),
        on("DP-1", custom, 1600, 0),
        off("eDP-1"),
    ];
    let operations = ops(&laptop(), &layout(entries)).expect("plans");
    let requests: Vec<&Operation> = operations
        .iter()
        .take_while(|op| **op != Operation::GrabServer)
        .collect();
    assert!(matches!(requests[0], Operation::CreateMode(info) if info.name == "1600x900_60.00"));
    assert_eq!(
        requests[1..],
        [
            &Operation::AddOutputMode {
                output: HDMI,
                mode: ModeRef::Created(0),
            },
            &Operation::AddOutputMode {
                output: DP1,
                mode: ModeRef::Created(0),
            },
        ]
    );
}

#[test]
fn a_mode_handle_picks_among_equal_modes() {
    // Two 1920x1080 modes at 60 Hz on eDP-1, the twin listed first.
    // Asked by its XID, the twin.
    let twin = ModeXid(0x64);
    let mut snapshot = laptop();
    let mut info = snapshot.resources.modes[0].clone();
    (info.id, info.name) = (twin, "1920x1080 twin".into());
    snapshot.resources.modes.push(info);
    snapshot.outputs[0].modes.insert(0, twin);
    let mut asked = mode(1920, 1080, hz(M1080));
    asked.handle = Some(ModeHandle(u64::from(twin.0)));
    let operations =
        ops(&snapshot, &layout(vec![on("eDP-1", asked.clone(), 0, 0)])).expect("plans");
    assert_eq!(
        grabbed(&operations),
        [set_crtc(C1, ModeRef::Listed(twin), 0, 0, &[EDP])]
    );
    // Without a handle the mode it is driven at wins.
    asked.handle = None;
    let operations = ops(&snapshot, &layout(vec![on("eDP-1", asked, 0, 0)])).expect("plans");
    assert_eq!(grabbed(&operations), []);
}

#[test]
fn no_refresh_asked_skips_double_scan_modes() {
    let scanned = ModeXid(0x65);
    let mut snapshot = laptop();
    let mut info = snapshot.resources.modes[3].clone();
    (info.id, info.flags) = (scanned, info.flags | mode_flags::DOUBLE_SCAN);
    snapshot.resources.modes.push(info);
    snapshot.outputs[0].modes.insert(0, scanned);
    let operations = ops(
        &snapshot,
        &layout(vec![on("eDP-1", mode(1280, 720, 0), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(
        grabbed(&operations).last(),
        Some(&set_crtc(C1, ModeRef::Listed(M720), 0, 0, &[EDP]))
    );
}

// Panning is hand-built only: Xvfb has none (only Xorg's `xf86RandR12.c`
// implements `rrSetPanning`, X server 21.1.26), so these follow that source.

/// [`laptop`] at 3840x2160 with eDP-1's CRTC panning across all of it.
fn panning_laptop() -> (Snapshot, Panning) {
    let panning = Panning {
        width: 3840,
        height: 2160,
        ..Panning::default()
    };
    let mut snapshot = laptop();
    snapshot.crtcs[0].panning = Some(panning);
    snapshot.screen = ScreenSize {
        width: 3840,
        height: 2160,
        mm_width: 1016,
        mm_height: 571,
    };
    (snapshot, panning)
}

#[test]
fn panning_read_as_all_zero_is_none() {
    // As `get_crtcs` drops it: nothing is set again after the CRTC.
    let mut snapshot = xvfb_with_720();
    snapshot.crtcs[0].panning = Some(Panning::default());
    let asked = mode(1280, 720, xvfb_720().refresh_mhz());
    let operations = ops(&snapshot, &layout(vec![on("screen", asked, 0, 0)])).expect("plans");
    assert!(
        !operations
            .iter()
            .any(|op| matches!(op, Operation::SetPanning { .. })),
        "{operations:?}"
    );
}

#[test]
fn panning_on_a_crtc_left_alone_stays_and_keeps_the_screen_its_size() {
    let (snapshot, _) = panning_laptop();
    let operations = ops(
        &snapshot,
        &layout(vec![on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(grabbed(&operations), []);
}

#[test]
fn panning_that_still_fits_is_set_again_after_its_crtc() {
    // As xrandr's `crtc_apply` does; and the screen keeps holding the area,
    // so its size does not change.
    let (snapshot, panning) = panning_laptop();
    let operations = ops(
        &snapshot,
        &layout(vec![on("eDP-1", mode(1280, 720, hz(M720)), 0, 0)]),
    )
    .expect("plans");
    assert_eq!(
        grabbed(&operations),
        [
            Operation::DisableCrtc(C1),
            set_crtc(C1, ModeRef::Listed(M720), 0, 0, &[EDP]),
            Operation::SetPanning { crtc: C1, panning },
        ]
    );
}

#[test]
fn panning_a_crtc_moves_out_of_is_reset_and_noted() {
    // HDMI-1 pans over 2560x2000 where it is; moved right of DP-1 it would
    // be panned back into it. The reset area no longer counts, so the
    // screen stays 1440 high. The reset goes first: Xorg pans the CRTC
    // right after setting it, so after would leave it at x 1920.
    let mut snapshot = docked();
    snapshot.crtcs[1].panning = Some(Panning {
        left: 1920,
        width: 2560,
        height: 2000,
        ..Panning::default()
    });
    let mut entries = docked_layout();
    (entries[1].state.x, entries[2].state.x) = (4480, 1920);
    let plan = plan(&snapshot, &layout(entries)).expect("plans");
    assert_eq!(
        grabbed(&plan.operations),
        [
            Operation::SetPanning {
                crtc: C2,
                panning: Panning::default(),
            },
            set_crtc(C2, ModeRef::Listed(M1440), 4480, 0, &[HDMI]),
            set_crtc(C3, ModeRef::Listed(M1440), 1920, 0, &[DP1]),
        ]
    );
    assert_eq!(
        plan.notes,
        [Note::PanningReset {
            crtc: C2,
            outputs: vec!["HDMI-1".into()],
        }]
    );
    assert_eq!(plan.notes[0].to_string(), "panning on HDMI-1 reset");
}

#[test]
fn panning_narrower_than_a_turned_mode_is_reset() {
    // The area holds HDMI-1 turned to portrait, 1440 wide, but the server
    // checks its width against the mode's own 2560.
    let mut snapshot = docked();
    snapshot.crtcs[1].panning = Some(Panning {
        left: 1920,
        width: 1440,
        height: 2560,
        ..Panning::default()
    });
    let mut entries = docked_layout();
    entries[1].state.transform = Transform::Rotate90;
    entries[2] = off("DP-1");
    let plan = plan(&snapshot, &layout(entries)).expect("plans");
    // HDMI-1's CRTC is turned off first here, and its panning reset before.
    assert_eq!(
        grabbed(&plan.operations)[..2],
        [
            Operation::SetPanning {
                crtc: C2,
                panning: Panning::default(),
            },
            Operation::DisableCrtc(C2),
        ]
    );
    assert_eq!(
        plan.notes,
        [Note::PanningReset {
            crtc: C2,
            outputs: vec!["HDMI-1".into()],
        }]
    );
}

#[test]
fn a_position_a_crtc_cannot_hold_is_refused() {
    let entries = vec![
        on("eDP-1", mode(1920, 1080, hz(M1080)), 0, 0),
        on("HDMI-1", mode(2560, 1440, hz(M1440)), 40_000, 0),
    ];
    assert_eq!(
        ops(&laptop(), &layout(entries)),
        Err(PlanError::Position {
            connector: "HDMI-1".into(),
            x: 40_000,
            y: 0,
        })
    );
}

#[test]
fn positions_at_the_ends_of_i32_are_refused_by_their_own_values() {
    // Moving i32::MAX by i32::MIN to the origin does not fit an i32.
    let entries = vec![
        on("eDP-1", mode(1920, 1080, hz(M1080)), i32::MIN, i32::MIN),
        on("HDMI-1", mode(2560, 1440, hz(M1440)), i32::MAX, i32::MAX),
    ];
    assert_eq!(
        ops(&laptop(), &layout(entries)),
        Err(PlanError::Position {
            connector: "HDMI-1".into(),
            x: i32::MAX,
            y: i32::MAX,
        })
    );
}

/// An EDID 1.4 base block, built byte by byte: vendor `DEL`, product
/// 0x12AB, and `serial` in a serial number descriptor when given.
fn edid(serial: Option<&str>) -> Vec<u8> {
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
    b.to_vec()
}

fn read(snapshot: &Snapshot) -> Layout {
    super::snapshot::layout(snapshot).expect("reads as a layout")
}

fn by_connector<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|entry| entry.connector() == Some(connector))
        .unwrap_or_else(|| panic!("no {connector}"))
}

#[test]
fn xvfb_reads_as_one_output_named_by_its_connector() {
    let layout = read(&xvfb());
    assert_eq!(layout.entries.len(), 1);
    let screen = &layout.entries[0];
    assert_eq!(screen.id, OutputId::Connector("screen".into()));
    let output = screen.output.as_ref().expect("hardware facts");
    assert_eq!(output.description, "screen");
    assert_eq!(output.physical_size_mm, None, "Xvfb gives 0 mm");
    let mut mode = Mode::new(1920, 1080, 0);
    mode.handle = Some(ModeHandle(0x3a));
    assert_eq!(output.modes, [mode.clone()]);
    assert_eq!(
        screen.state,
        OutputState {
            mode: Some(mode),
            ..OutputState::default()
        }
    );
}

#[test]
fn a_docked_laptop_reads_with_identities_modes_and_positions() {
    let mut snapshot = docked();
    snapshot.outputs[0].edid = Some(edid(Some("SER1")));
    snapshot.outputs[0].mm_width = 344;
    snapshot.outputs[0].mm_height = 194;
    // A monitor without a serial: named by its connector.
    snapshot.outputs[1].edid = Some(edid(None));
    snapshot.outputs[3].connection = super::resources::Connection::Disconnected;
    snapshot.primary = Some(EDP);
    let layout = read(&snapshot);

    let names: Vec<_> = layout.entries.iter().map(|e| e.connector()).collect();
    assert_eq!(
        names,
        [Some("eDP-1"), Some("HDMI-1"), Some("DP-1")],
        "DP-2, disconnected and off, is left out"
    );
    let edp = by_connector(&layout, "eDP-1");
    assert_eq!(
        edp.id,
        OutputId::Edid {
            make: "DEL".into(),
            model: "0x12AB".into(),
            serial: "SER1".into(),
        }
    );
    let output = edp.output.as_ref().expect("hardware facts");
    assert_eq!(output.description, "DEL 0x12AB SER1");
    assert_eq!(output.physical_size_mm, Some((344, 194)));
    // eDP-1 lists 1080 first, which is preferred (num_preferred 1).
    let preferred: Vec<bool> = output.modes.iter().map(|mode| mode.preferred).collect();
    assert_eq!(preferred, [true, false]);
    assert_eq!(output.modes[0].refresh_mhz, 60_000);
    assert!(edp.state.primary && edp.state.enabled);
    assert_eq!(
        edp.state.mode.as_ref().map(|m| m.handle),
        Some(Some(ModeHandle(0x60)))
    );

    let hdmi = by_connector(&layout, "HDMI-1");
    assert_eq!(hdmi.id, OutputId::Connector("HDMI-1".into()));
    assert_eq!(
        (hdmi.state.x, hdmi.state.y, hdmi.state.primary),
        (1920, 0, false)
    );
    assert_eq!(by_connector(&layout, "DP-1").state.x, 4480);
}

#[test]
fn an_output_off_but_connected_reads_as_off() {
    let layout = read(&laptop());
    let hdmi = by_connector(&layout, "HDMI-1");
    assert!(!hdmi.state.enabled && hdmi.state.mode.is_none());
    assert_eq!(hdmi.output.as_ref().map(|o| o.modes.len()), Some(2));
}

#[test]
fn a_rotation_reads_back_as_the_transform_that_sets_it() {
    for transform in Transform::ALL {
        assert_eq!(super::plan::transform(rotation(transform)), Some(transform));
    }
    // A reflection in y is one in x half a turn further; both are half a
    // turn.
    let read_back = |bits: Rotation| super::plan::transform(bits);
    assert_eq!(
        read_back(Rotation::ROTATE_90 | Rotation::REFLECT_Y),
        Some(Transform::Flipped270)
    );
    assert_eq!(
        read_back(Rotation::ROTATE_0 | Rotation::REFLECT_X | Rotation::REFLECT_Y),
        Some(Transform::Rotate180)
    );
    assert_eq!(read_back(Rotation(0)), None);

    let mut snapshot = docked();
    snapshot.crtcs[1].rotation = Rotation::ROTATE_270 | Rotation::REFLECT_X;
    assert_eq!(
        by_connector(&read(&snapshot), "HDMI-1").state.transform,
        Transform::Flipped270
    );
    snapshot.crtcs[1].rotation = Rotation(0);
    assert!(super::snapshot::layout(&snapshot).is_err());
}

#[test]
fn clones_on_one_crtc_and_crtcs_at_one_place_read_as_mirrors() {
    let mut snapshot = cloning();
    snapshot.crtcs[0].outputs.push(DVI0);
    snapshot.outputs[1].crtc = Some(D1);
    let layout = read(&snapshot);
    assert_eq!(
        by_connector(&layout, "DVI-0").state.mirror_of,
        Some(OutputId::Connector("VGA-0".into()))
    );
    assert_eq!(by_connector(&layout, "VGA-0").state.mirror_of, None);

    // HDMI-1 on its own CRTC, where eDP-1's is.
    let mut laptop = laptop();
    drive(&mut laptop, C2, HDMI, M1080, 0, 0);
    let layout = read(&laptop);
    assert_eq!(
        by_connector(&layout, "HDMI-1").state.mirror_of,
        Some(OutputId::Connector("eDP-1".into()))
    );
}

#[test]
fn replies_naming_what_they_do_not_describe_are_refused() {
    let mut snapshot = docked();
    snapshot.crtcs[1].mode = Some(ModeXid(0x99));
    assert!(super::snapshot::layout(&snapshot).is_err());
    let mut snapshot = docked();
    snapshot.outputs[1].crtc = Some(CrtcXid(0x99));
    assert!(super::snapshot::layout(&snapshot).is_err());
}

#[test]
fn a_crtc_with_a_transform_is_not_planned_over() {
    let mut snapshot = docked();
    snapshot.crtcs[1].transform = Some([[1 << 17, 0, 0], [0, 1 << 17, 0], [0, 0, 1 << 16]]);
    let refused = plan(&snapshot, &layout(docked_layout()));
    assert_eq!(
        refused,
        Err(PlanError::Transformed {
            crtc: C2,
            outputs: vec!["HDMI-1".into()],
        })
    );
    let said = refused.unwrap_err().to_string();
    assert!(said.starts_with("HDMI-1 has a RandR transform"), "{said}");
}

#[test]
fn an_output_on_a_crtc_with_no_mode_and_half_a_size_read_as_off_and_unsized() {
    // A CRTC an output still names after it was turned off, mode none.
    let mut snapshot = laptop();
    snapshot.outputs[1].crtc = Some(C2);
    snapshot.outputs[1].mm_width = 600;
    let layout = read(&snapshot);
    let hdmi = by_connector(&layout, "HDMI-1");
    assert!(!hdmi.state.enabled);
    assert_eq!(
        hdmi.output.as_ref().and_then(|o| o.physical_size_mm),
        None,
        "no height: no size"
    );
}

#[test]
fn a_mode_named_without_a_handle_is_progressive_before_interlaced() {
    // 1920x1080i at 60 Hz (CEA-861's, its field rate 60) listed before
    // 1920x1080p at 60 Hz, eDP-1 driven at neither.
    let interlaced = ModeXid(0x66);
    let mut snapshot = laptop();
    let mut info = timings(
        interlaced,
        (1920, 1080),
        74_250_000,
        [2008, 2052, 2200],
        [1084, 1094, 1125],
        "1920x1080i",
    );
    info.flags |= mode_flags::INTERLACE;
    assert_eq!(info.refresh_mhz(), 60_000);
    snapshot.resources.modes.push(info);
    snapshot.outputs[0].modes.insert(0, interlaced);
    drive_mode(&mut snapshot, C1, M720);
    let operations = ops(
        &snapshot,
        &layout(vec![on("eDP-1", mode(1920, 1080, 60_000), 0, 0)]),
    )
    .expect("plans");
    assert!(
        operations.iter().any(|op| matches!(
            op,
            Operation::SetCrtcConfig {
                mode: ModeRef::Listed(M1080),
                ..
            }
        )),
        "{operations:?}"
    );
}

/// Sets `crtc`'s current mode to `mode`, its size with it.
fn drive_mode(snapshot: &mut Snapshot, crtc: CrtcXid, mode: ModeXid) {
    let info = snapshot
        .resources
        .modes
        .iter()
        .find(|info| info.id == mode)
        .expect("a mode of the snapshot")
        .clone();
    let state = snapshot
        .crtcs
        .iter_mut()
        .find(|state| state.id == crtc)
        .expect("a CRTC of the snapshot");
    (state.mode, state.width, state.height) = (Some(mode), info.width, info.height);
}
