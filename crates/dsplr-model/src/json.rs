//! The JSON form of a `Layout`, shared by `dsplr list --json` and the profile
//! store: snake_case field names, refresh in millihertz, no backend mode
//! handles, and a `version` that may change before 1.0.

use serde::{Deserialize, Serialize};

use crate::Layout;

/// The `version` this build writes, and the only one it reads.
pub const JSON_VERSION: u32 = 0;

/// Why a JSON layout could not be read.
#[derive(Debug, thiserror::Error)]
pub enum JsonError {
    #[error("not a dsplr layout: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("layout version {found} is not supported (this build reads version {JSON_VERSION})")]
    UnsupportedVersion { found: u32 },
}

#[derive(Serialize)]
struct Written<'a> {
    version: u32,
    #[serde(flatten)]
    layout: &'a Layout,
}

/// Only the version, read first so a layout from another version is refused
/// by its number rather than by whichever field it happens to trip over.
#[derive(Deserialize)]
struct Version {
    version: u32,
}

impl Layout {
    /// The layout as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// When serde_json refuses a value; for these types that does not happen
    /// in practice, but the error is passed on rather than assumed away.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&Written {
            version: JSON_VERSION,
            layout: self,
        })
    }

    /// Reads a layout written by [`Layout::to_json`]. Mode handles come back
    /// as `None`, so modes are matched against the live outputs again.
    ///
    /// # Errors
    ///
    /// When the text is not a layout, or names a version other than
    /// [`JSON_VERSION`].
    pub fn from_json(text: &str) -> Result<Self, JsonError> {
        let Version { version } = serde_json::from_str(text)?;
        if version != JSON_VERSION {
            return Err(JsonError::UnsupportedVersion { found: version });
        }
        Ok(serde_json::from_str(text)?)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::JsonError;
    use crate::{
        AdaptiveSync, Layout, LayoutMode, Mode, ModeHandle, Output, OutputEntry, OutputId,
        OutputState, RgbRange, Transform,
    };

    fn two_monitors() -> Layout {
        let mut listed = Mode::new(2560, 1440, 143_998);
        listed.preferred = true;
        listed.scales = vec![1.0, 1.25];
        listed.handle = Some(ModeHandle(7));
        Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G2G4", "1A2B3C", "DP-1"),
                    output: Some(Output {
                        connector: "DP-1".into(),
                        description: "AOC Q27G2G4 1A2B3C".into(),
                        physical_size_mm: Some((600, 340)),
                        make: String::new(),
                        model: String::new(),
                        serial: String::new(),
                        modes: vec![listed.clone(), Mode::new(1920, 1080, 60_000)],
                    }),
                    state: OutputState {
                        mode: Some(listed),
                        scale: 1.25,
                        transform: Transform::Rotate90,
                        adaptive_sync: Some(AdaptiveSync::Automatic),
                        colour_preset: Some("srgb".into()),
                        bit_depth: Some(10),
                        rgb_range: Some(RgbRange::Full),
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: None,
                    state: OutputState {
                        enabled: false,
                        x: -1920,
                        y: 200,
                        mirror_of: Some(OutputId::Connector("DP-1".into())),
                        primary: true,
                        overscan: Some(5),
                        ..OutputState::default()
                    },
                },
            ],
            ..Default::default()
        }
    }

    fn without_handles(mut layout: Layout) -> Layout {
        for entry in &mut layout.entries {
            if let Some(mode) = &mut entry.state.mode {
                mode.handle = None;
            }
            for mode in entry.output.iter_mut().flat_map(|o| &mut o.modes) {
                mode.handle = None;
            }
        }
        layout
    }

    #[test]
    fn a_layout_survives_the_round_trip_except_its_mode_handles() {
        let layout = two_monitors();
        let read = Layout::from_json(&layout.to_json().unwrap()).unwrap();
        assert_eq!(read, without_handles(layout));
    }

    #[test]
    fn a_physical_layout_says_so_and_a_logical_one_leaves_it_out() {
        let mut layout = two_monitors();
        let written: serde_json::Value = serde_json::from_str(&layout.to_json().unwrap()).unwrap();
        assert_eq!(written.get("layout_mode"), None);
        layout.layout_mode = LayoutMode::Physical;
        let text = layout.to_json().unwrap();
        let written: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["layout_mode"], "physical");
        assert_eq!(Layout::from_json(&text).unwrap(), without_handles(layout));
    }

    #[test]
    fn the_written_form_is_versioned_snake_case_in_millihertz() {
        let layout = Layout {
            entries: vec![OutputEntry {
                id: OutputId::Connector("HDMI-A-1".into()),
                output: None,
                state: OutputState {
                    mode: Some(Mode {
                        handle: Some(ModeHandle(3)),
                        ..Mode::new(1920, 1080, 59_951)
                    }),
                    transform: Transform::Flipped270,
                    ..OutputState::default()
                },
            }],
            ..Default::default()
        };
        let written: serde_json::Value = serde_json::from_str(&layout.to_json().unwrap()).unwrap();
        assert_eq!(
            written,
            json!({
                "version": 0,
                "entries": [{
                    "id": { "connector": "HDMI-A-1" },
                    "output": null,
                    "state": {
                        "enabled": true,
                        "mode": {
                            "width": 1920,
                            "height": 1080,
                            "refresh_mhz": 59951,
                            "preferred": false
                        },
                        "x": 0,
                        "y": 0,
                        "scale": 1.0,
                        "transform": "flipped270",
                        "adaptive_sync": null,
                        "mirror_of": null,
                        "primary": false,
                        "colour_preset": null,
                        "bit_depth": null,
                        "overscan": null,
                        "rgb_range": null
                    }
                }]
            })
        );
    }

    #[test]
    fn missing_settings_take_their_defaults() {
        let read = Layout::from_json(
            r#"{"version": 0, "entries": [{"id": {"connector": "DP-2"}, "output": null, "state": {"x": 1920}}]}"#,
        )
        .unwrap();
        assert_eq!(
            read.entries[0].state,
            OutputState {
                x: 1920,
                ..OutputState::default()
            }
        );
    }

    /// Written before outputs carried their make, model and serial.
    #[test]
    fn an_output_without_make_model_and_serial_reads_with_them_empty() {
        let read = Layout::from_json(
            r#"{"version": 0, "entries": [{"id": {"connector": "DP-2"}, "output": {"connector": "DP-2", "description": "", "physical_size_mm": null, "modes": []}, "state": {}}]}"#,
        )
        .unwrap();
        let output = read.entries[0].output.as_ref().unwrap();
        assert_eq!(
            (&*output.make, &*output.model, &*output.serial),
            ("", "", "")
        );
    }

    #[test]
    fn another_version_is_refused_by_its_number() {
        let err = Layout::from_json(r#"{"version": 1, "entries": "anything"}"#).unwrap_err();
        assert!(matches!(err, JsonError::UnsupportedVersion { found: 1 }));
    }

    #[test]
    fn text_that_is_not_a_layout_is_refused() {
        assert!(matches!(
            Layout::from_json(r#"{"entries": []}"#),
            Err(JsonError::Parse(_))
        ));
        assert!(matches!(
            Layout::from_json("not json"),
            Err(JsonError::Parse(_))
        ));
    }
}
