//! `dsplr list`: the layout as text, one block per output.

use dsplr_model::{AdaptiveSync, Layout, Mode, OutputEntry, Transform, mhz_to_hz_text};

/// The text `dsplr list` prints.
pub fn describe(layout: &Layout) -> String {
    if layout.entries.is_empty() {
        return "no outputs\n".to_owned();
    }
    layout.entries.iter().map(describe_entry).collect()
}

fn describe_entry(entry: &OutputEntry) -> String {
    // The connector when it is known, else the name the identity carries.
    let name = entry
        .output
        .as_ref()
        .map_or_else(|| entry.id.to_string(), |output| output.connector.clone());
    let mut lines = vec![match &entry.output {
        Some(output) if !output.description.is_empty() => {
            format!("{name}: {}", output.description)
        }
        _ => name,
    }];
    let state = &entry.state;
    if state.enabled {
        let mut settings = vec![
            state
                .mode
                .as_ref()
                .map_or_else(|| "no mode".to_owned(), mode_text),
            format!("position {},{}", state.x, state.y),
            format!("scale {}", state.scale),
            format!("transform {}", transform_name(state.transform)),
            format!(
                "adaptive sync {}",
                state.adaptive_sync.map_or("unknown", adaptive_sync_name)
            ),
        ];
        if let Some(source) = &state.mirror_of {
            settings.push(format!("mirrors {source}"));
        }
        if state.primary {
            settings.push("primary".to_owned());
        }
        lines.push(format!("  {}", settings.join(", ")));
    } else {
        lines.push("  disabled".to_owned());
    }
    if let Some(output) = entry.output.as_ref().filter(|o| !o.modes.is_empty()) {
        let modes: Vec<_> = output
            .modes
            .iter()
            .map(|mode| {
                if mode.preferred {
                    format!("{} (preferred)", mode_text(mode))
                } else {
                    mode_text(mode)
                }
            })
            .collect();
        lines.push(format!("  modes: {}", modes.join(", ")));
    }
    lines.into_iter().map(|line| line + "\n").collect()
}

/// `2560x1440 @ 143.998 Hz`: refresh in hertz, exact to the millihertz, with
/// trailing zeros dropped (`60 Hz`, `59.94 Hz`).
fn mode_text(mode: &Mode) -> String {
    format!(
        "{}x{} @ {} Hz",
        mode.width,
        mode.height,
        mhz_to_hz_text(mode.refresh_mhz)
    )
}

const fn transform_name(transform: Transform) -> &'static str {
    match transform {
        Transform::Normal => "normal",
        Transform::Rotate90 => "90",
        Transform::Rotate180 => "180",
        Transform::Rotate270 => "270",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped-90",
        Transform::Flipped180 => "flipped-180",
        Transform::Flipped270 => "flipped-270",
    }
}

pub const fn adaptive_sync_name(value: AdaptiveSync) -> &'static str {
    match value {
        AdaptiveSync::Off => "off",
        AdaptiveSync::On => "on",
        AdaptiveSync::Automatic => "automatic",
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::{describe, mode_text};

    #[test]
    fn a_mode_reads_as_its_size_and_refresh_in_hertz() {
        assert_eq!(
            mode_text(&Mode::new(1920, 1080, 59_940)),
            "1920x1080 @ 59.94 Hz"
        );
    }

    #[test]
    fn an_empty_layout_says_so() {
        assert_eq!(describe(&Layout::default()), "no outputs\n");
    }

    /// The owner's machine as the Hyprland backend reads it: the laptop panel
    /// disabled, the external monitor at 2560x1440 and 120 Hz.
    #[test]
    fn each_output_is_a_block_of_its_settings_and_modes() {
        let current = Mode::new(2560, 1440, 120_000);
        let mut preferred = Mode::new(2560, 1440, 143_998);
        preferred.preferred = true;
        let layout = Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(Output {
                        connector: "eDP-1".into(),
                        description: String::new(),
                        physical_size_mm: None,
                        modes: vec![],
                    }),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G2G4", "1A2B3C", "HDMI-A-1"),
                    output: Some(Output {
                        connector: "HDMI-A-1".into(),
                        description: "AOC Q27G2G4 1A2B3C".into(),
                        physical_size_mm: Some((600, 340)),
                        modes: vec![preferred, current.clone()],
                    }),
                    state: OutputState {
                        mode: Some(current),
                        x: 1920,
                        scale: 1.25,
                        transform: Transform::Flipped90,
                        adaptive_sync: Some(AdaptiveSync::Automatic),
                        mirror_of: Some(OutputId::Connector("eDP-1".into())),
                        primary: true,
                        ..OutputState::default()
                    },
                },
            ],
        };
        assert_eq!(
            describe(&layout),
            "eDP-1\n  disabled\n\
             HDMI-A-1: AOC Q27G2G4 1A2B3C\n  \
             2560x1440 @ 120 Hz, position 1920,0, scale 1.25, transform flipped-90, \
             adaptive sync automatic, mirrors eDP-1, primary\n  \
             modes: 2560x1440 @ 143.998 Hz (preferred), 2560x1440 @ 120 Hz\n"
        );
    }

    #[test]
    fn an_entry_without_hardware_facts_is_named_by_its_identity() {
        let layout = Layout {
            entries: vec![OutputEntry {
                id: OutputId::new("AOC", "Q27G2G4", "1A2B3C", "DP-1"),
                output: None,
                state: OutputState::default(),
            }],
        };
        assert_eq!(
            describe(&layout),
            "AOC Q27G2G4 1A2B3C\n  no mode, position 0,0, scale 1, transform normal, adaptive sync unknown\n"
        );
    }
}
