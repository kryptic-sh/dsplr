//! `dsplr list`: the layout as text, one block per output.

use dsplr_model::{AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, Transform};

/// The text `dsplr list` prints.
pub fn describe(layout: &Layout) -> String {
    if layout.entries.is_empty() {
        return "no outputs\n".to_owned();
    }
    layout.entries.iter().map(describe_entry).collect()
}

fn describe_entry(entry: &OutputEntry) -> String {
    let name = output_name(&entry.id, entry.output.as_ref());
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
            format!("adaptive sync {}", adaptive_sync_name(state.adaptive_sync)),
        ];
        if let Some(source) = &state.mirror_of {
            settings.push(format!("mirrors {}", output_name(source, None)));
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

/// The connector when it is known, else the name the identity carries.
fn output_name(id: &OutputId, output: Option<&Output>) -> String {
    match (output, id) {
        (Some(output), _) => output.connector.clone(),
        (None, OutputId::Connector(connector)) => connector.clone(),
        (
            None,
            OutputId::Edid {
                make,
                model,
                serial,
            },
        ) => format!("{make} {model} {serial}"),
    }
}

/// `2560x1440 @ 143.998 Hz`: refresh in hertz, exact to the millihertz, with
/// trailing zeros dropped (`60 Hz`, `59.94 Hz`).
fn mode_text(mode: &Mode) -> String {
    let (hz, mhz) = (mode.refresh_mhz / 1000, mode.refresh_mhz % 1000);
    let refresh = if mhz == 0 {
        hz.to_string()
    } else {
        let fraction = format!("{mhz:03}");
        format!("{hz}.{}", fraction.trim_end_matches('0'))
    };
    format!("{}x{} @ {refresh} Hz", mode.width, mode.height)
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
    fn refresh_drops_trailing_zeros_but_keeps_every_millihertz() {
        let text = |mhz| mode_text(&Mode::new(1920, 1080, mhz));
        assert_eq!(text(60_000), "1920x1080 @ 60 Hz");
        assert_eq!(text(59_940), "1920x1080 @ 59.94 Hz");
        assert_eq!(text(50_500), "1920x1080 @ 50.5 Hz");
        assert_eq!(text(143_998), "1920x1080 @ 143.998 Hz");
        assert_eq!(text(60_001), "1920x1080 @ 60.001 Hz");
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
                        adaptive_sync: AdaptiveSync::Automatic,
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
            "AOC Q27G2G4 1A2B3C\n  no mode, position 0,0, scale 1, transform normal, adaptive sync off\n"
        );
    }
}
