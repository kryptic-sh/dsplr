//! Layouts the tests share.

use dsplr_model::{
    AdaptiveSync, Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
};

use crate::Snapshot;

pub fn output(connector: &str, description: &str, modes: Vec<Mode>) -> Output {
    Output {
        connector: connector.into(),
        description: description.into(),
        make: String::new(),
        model: String::new(),
        serial: String::new(),
        physical_size_mm: None,
        modes,
    }
}

/// The owner's machine: the laptop panel off, one monitor at 2560x1440
/// and 120 Hz.
pub fn owners_machine() -> Snapshot {
    Snapshot {
        backend: "hyprland".into(),
        capabilities: Capabilities {
            mirroring: true,
            adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
            fractional_scale: true,
            per_output_scale: true,
            gaps_allowed: true,
            colour_preset: true,
            bit_depth: true,
            ..Capabilities::default()
        },
        layout: Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(output(
                        "eDP-1",
                        "BOE 0x0BCA",
                        vec![Mode::new(1920, 1080, 60_000)],
                    )),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                    output: Some(output(
                        "HDMI-A-1",
                        "AOC Q27G4 SERIAL0000001",
                        vec![Mode::new(2560, 1440, 120_000)],
                    )),
                    state: OutputState {
                        mode: Some(Mode::new(2560, 1440, 120_000)),
                        adaptive_sync: Some(AdaptiveSync::Off),
                        ..OutputState::default()
                    },
                },
            ],
        },
    }
}

/// What the `wlr` backend can do (`Backend::capabilities`).
pub fn wlr() -> Capabilities {
    Capabilities {
        custom_modes: true,
        adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
        fractional_scale: true,
        per_output_scale: true,
        test_before_apply: true,
        gaps_allowed: true,
        ..Capabilities::default()
    }
}

/// Three monitors side by side: one turned and scaled, one disabled, and
/// no hardware facts for any (as a layout read from a file).
pub fn three_monitors() -> Snapshot {
    let entry = |connector: &str, state| OutputEntry {
        id: OutputId::Connector(connector.into()),
        output: None,
        state,
    };
    Snapshot {
        backend: "sway".into(),
        capabilities: wlr(),
        layout: Layout {
            entries: vec![
                entry(
                    "DP-1",
                    OutputState {
                        mode: Some(Mode::new(1920, 1080, 60_000)),
                        x: 0,
                        y: 360,
                        ..OutputState::default()
                    },
                ),
                entry(
                    "DP-2",
                    OutputState {
                        mode: Some(Mode::new(2560, 1440, 144_000)),
                        x: 1920,
                        scale: 1.25,
                        transform: Transform::Rotate90,
                        ..OutputState::default()
                    },
                ),
                entry(
                    "HDMI-A-1",
                    OutputState {
                        enabled: false,
                        mode: Some(Mode::new(1920, 1080, 60_000)),
                        ..OutputState::default()
                    },
                ),
            ],
        },
    }
}
