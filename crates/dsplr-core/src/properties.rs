//! The property panel's rows for the selected output: each setting
//! `docs/plan/06-gui.md` lists, as text, marked where a pending edit changed
//! it and where the backend cannot set it.

use dsplr_model::{Capabilities, Mode, OutputEntry, OutputState};

use crate::Editor;

/// One row of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub label: &'static str,
    /// The pending value, or why it cannot be set.
    pub value: String,
    /// The pending value differs from the live one.
    pub changed: bool,
    /// The backend can set this; when not, `value` says so.
    pub supported: bool,
}

/// The selected output's name as the canvas and panel head it: its
/// connector, else the name its identity carries.
pub fn name(entry: &OutputEntry) -> String {
    entry
        .connector()
        .map_or_else(|| entry.id.to_string(), ToOwned::to_owned)
}

/// The rows for output `index`, from its pending settings, or `None` when
/// there is no such output.
pub fn properties(editor: &Editor, index: usize) -> Option<Vec<Property>> {
    let live = editor.live().entries.get(index)?;
    let pending = editor.pending().entries.get(index)?;
    let snapshot = editor.snapshot();
    let rows = |state: &OutputState| rows(live, state, &snapshot.capabilities);
    let unsupported = format!("not supported by {}", snapshot.backend);
    Some(
        rows(&pending.state)
            .into_iter()
            .zip(rows(&live.state))
            .map(|((label, value), (_, was))| {
                let supported = value.is_some();
                Property {
                    label,
                    changed: value != was,
                    value: value.unwrap_or_else(|| unsupported.clone()),
                    supported,
                }
            })
            .collect(),
    )
}

/// Each row's label and value for `entry` set to `state`; `None` for a
/// setting the backend cannot set. Settings only some backends have (the
/// primary output, Hyprland's colour preset and bit depth, KDE's overscan
/// and RGB range) are rows only on a backend that has them.
fn rows(
    entry: &OutputEntry,
    state: &OutputState,
    capabilities: &Capabilities,
) -> Vec<(&'static str, Option<String>)> {
    let mode = state.mode.as_ref();
    let mut rows = vec![
        ("Output", Some(name(entry))),
        (
            "Description",
            Some(entry.description().unwrap_or_else(|| "unknown".into())),
        ),
        ("Enabled", Some(yes_no(state.enabled))),
        (
            "Mode",
            Some(mode.map_or_else(
                || "none".into(),
                |mode| format!("{}x{}", mode.width, mode.height),
            )),
        ),
        (
            "Refresh",
            Some(mode.map_or_else(|| "none".into(), Mode::refresh_text)),
        ),
        (
            "Custom mode",
            capabilities
                .custom_modes
                .then(|| yes_no(mode.is_some_and(|mode| !listed(entry, mode)))),
        ),
        ("Position", Some(format!("{}, {}", state.x, state.y))),
        ("Scale", Some(state.scale.to_string())),
        ("Transform", Some(state.transform.to_string())),
        (
            "Adaptive sync",
            (!capabilities.adaptive_sync.is_empty()).then(|| {
                state
                    .adaptive_sync
                    .map_or_else(|| "unknown".into(), |sync| sync.to_string())
            }),
        ),
        (
            "Mirror of",
            capabilities.mirroring.then(|| {
                state
                    .mirror_of
                    .as_ref()
                    .map_or_else(|| "none".into(), ToString::to_string)
            }),
        ),
    ];
    if capabilities.primary {
        rows.push(("Primary", Some(yes_no(state.primary))));
    }
    if capabilities.colour_preset {
        let preset = state.colour_preset.clone();
        rows.push(("Colour preset", Some(preset.unwrap_or_else(default))));
    }
    if capabilities.bit_depth {
        let depth = state.bit_depth.map(|bits| format!("{bits} bits"));
        rows.push(("Bit depth", Some(depth.unwrap_or_else(default))));
    }
    if capabilities.overscan {
        let overscan = state.overscan.map(|percent| format!("{percent} %"));
        rows.push(("Overscan", Some(overscan.unwrap_or_else(default))));
    }
    if capabilities.rgb_range {
        let range = state.rgb_range.map(|range| range.to_string());
        rows.push(("RGB range", Some(range.unwrap_or_else(default))));
    }
    rows
}

/// Whether `entry`'s output lists `mode`. An entry with no hardware facts
/// lists nothing, so every mode is custom there.
fn listed(entry: &OutputEntry, mode: &Mode) -> bool {
    entry
        .output
        .as_ref()
        .is_some_and(|output| output.modes.iter().any(|listed| listed.matches(mode)))
}

fn yes_no(on: bool) -> String {
    if on { "yes" } else { "no" }.into()
}

/// What an optional setting the desktop chooses reads as.
fn default() -> String {
    "default".into()
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState,
    };

    use super::{Property, properties};
    use crate::{Editor, Snapshot};

    fn editor(capabilities: Capabilities) -> Editor {
        Editor::new(Snapshot {
            backend: "sway".into(),
            capabilities,
            layout: Layout {
                entries: vec![OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                    output: Some(Output {
                        connector: "HDMI-A-1".into(),
                        description: "AOC Q27G4 SERIAL0000001".into(),
                        make: "AOC".into(),
                        model: "Q27G4".into(),
                        serial: "SERIAL0000001".into(),
                        physical_size_mm: None,
                        modes: vec![Mode::new(2560, 1440, 120_000)],
                    }),
                    state: OutputState {
                        mode: Some(Mode::new(2560, 1440, 120_000)),
                        adaptive_sync: Some(AdaptiveSync::Off),
                        ..OutputState::default()
                    },
                }],
            },
        })
    }

    fn text(rows: &[Property]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                let mark = match (row.changed, row.supported) {
                    (true, _) => " *",
                    (false, false) => " -",
                    (false, true) => "",
                };
                format!("{}: {}{mark}", row.label, row.value)
            })
            .collect()
    }

    #[test]
    fn a_backend_without_optional_settings_says_which_it_cannot_set() {
        let editor = editor(Capabilities::default());
        assert_eq!(
            text(&properties(&editor, 0).unwrap()),
            [
                "Output: HDMI-A-1",
                "Description: AOC Q27G4 SERIAL0000001",
                "Enabled: yes",
                "Mode: 2560x1440",
                "Refresh: 120 Hz",
                "Custom mode: not supported by sway -",
                "Position: 0, 0",
                "Scale: 1",
                "Transform: normal",
                "Adaptive sync: not supported by sway -",
                "Mirror of: not supported by sway -",
            ]
        );
        assert_eq!(properties(&editor, 1), None);
    }

    #[test]
    fn backend_specific_rows_appear_only_where_the_backend_has_them() {
        let editor = editor(Capabilities {
            custom_modes: true,
            mirroring: true,
            adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
            primary: true,
            colour_preset: true,
            bit_depth: true,
            overscan: true,
            rgb_range: true,
            ..Capabilities::default()
        });
        let rows = text(&properties(&editor, 0).unwrap());
        assert_eq!(
            rows[5..],
            [
                "Custom mode: no",
                "Position: 0, 0",
                "Scale: 1",
                "Transform: normal",
                "Adaptive sync: off",
                "Mirror of: none",
                "Primary: no",
                "Colour preset: default",
                "Bit depth: default",
                "Overscan: default",
                "RGB range: default",
            ]
        );
    }

    #[test]
    fn a_pending_edit_marks_the_rows_it_changed() {
        let mut editor = editor(Capabilities {
            custom_modes: true,
            ..Capabilities::default()
        });
        editor.edit(0, |state| {
            state.mode = Some(Mode::new(1920, 1080, 60_000));
            state.scale = 1.25;
        });
        let rows = text(&properties(&editor, 0).unwrap());
        assert_eq!(rows[3], "Mode: 1920x1080 *");
        assert_eq!(rows[4], "Refresh: 60 Hz *");
        assert_eq!(rows[5], "Custom mode: yes *", "the output does not list it");
        assert_eq!(rows[6], "Position: 0, 0");
        assert_eq!(rows[7], "Scale: 1.25 *");
    }

    #[test]
    fn a_refresh_the_backend_did_not_report_reads_as_unknown() {
        let mut editor = editor(Capabilities::default());
        editor.edit(0, |state| state.mode = Some(Mode::new(1920, 1080, 0)));
        let rows = text(&properties(&editor, 0).unwrap());
        assert_eq!(rows[4], "Refresh: unknown *");
    }
}
