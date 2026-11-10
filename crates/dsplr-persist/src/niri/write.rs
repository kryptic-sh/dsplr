//! Writing `output` nodes. Every node of an enabled output states each
//! setting dsplr owns, since niri gives a setting left out its default, and
//! nothing niri 26.04's `niri validate` rejects is written: an included file
//! niri cannot parse makes it drop the whole config (a nested niri 26.04
//! started on one came up at its defaults).

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, is_valid_scale, mhz_to_hz_text};

use super::{output_name, quoted};

/// The largest scale niri takes ("value must be between 0 and 10").
const MAX_SCALE: f64 = 10.0;

/// The largest width or height of a mode niri takes: it reads them as
/// 16-bit numbers ("error parsing width" for 65536).
const MAX_SIDE: u32 = u16::MAX as u32;

/// Why a layout could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("{0}: no connector or identifier to name it by")]
    NoName(OutputId),
    #[error("{0}: enabled without a mode")]
    NoMode(OutputId),
    #[error("{0}: the scale is not a number above 0 and at most {MAX_SCALE}")]
    InvalidScale(OutputId),
    /// A connector, or a name read from a file, holding a control
    /// character, which dsplr does not write into a KDL string.
    #[error("{output}: its name {name:?} cannot be written into a niri config")]
    Unquotable { output: OutputId, name: String },
    /// A setting niri's output config has no node for, or a value it does
    /// not take.
    #[error("{output}: niri's output config cannot hold {what}")]
    Unsupported {
        output: OutputId,
        what: &'static str,
    },
}

/// The `output "<name>" { … }` node for one entry of `layout`, ending in a
/// newline. `layout` is needed to tell whether the entry's identifier is
/// its own ([`super::output_name`]).
///
/// What is written, and why:
/// - `off` alone for an output that is off;
/// - `mode custom=true` for a mode its output is known not to list, and
///   the refresh only when it is known (niri then picks the highest);
/// - `variable-refresh-rate` when adaptive sync is on, with
///   `on-demand=true` when it is automatic (niri turns it on for windows
///   that ask), and nothing when it is off or not known, which is niri's
///   default: off.
///
/// # Errors
///
/// When the entry has no name or one dsplr cannot quote, an enabled entry
/// has no mode or an unusable scale, or it asks for what niri's output
/// config cannot hold: a mirror, a primary output, colour settings, a mode
/// side over 65535, or a mode it does not list without its refresh, which
/// niri needs for a custom mode.
pub fn output_node(layout: &Layout, entry: &OutputEntry) -> Result<String, WriteError> {
    let id = || entry.id.clone();
    let name = output_name(layout, entry).ok_or_else(|| WriteError::NoName(id()))?;
    let name = quoted(&name).ok_or_else(|| WriteError::Unquotable {
        output: id(),
        name: name.clone(),
    })?;
    let state = &entry.state;
    let refused = [
        (state.mirror_of.is_some(), "a mirror"),
        (state.primary, "a primary output"),
        (state.colour_preset.is_some(), "a colour preset"),
        (state.bit_depth.is_some(), "a bit depth"),
        (state.overscan.is_some(), "overscan"),
        (state.rgb_range.is_some(), "an RGB range"),
    ];
    if let Some((_, what)) = refused.into_iter().find(|(set, _)| *set) {
        return Err(WriteError::Unsupported { output: id(), what });
    }
    if !state.enabled {
        return Ok(node(&name, &["off".to_owned()]));
    }
    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| WriteError::NoMode(id()))?;
    if !is_valid_scale(state.scale) || state.scale > MAX_SCALE {
        return Err(WriteError::InvalidScale(id()));
    }
    if mode.width > MAX_SIDE || mode.height > MAX_SIDE {
        return Err(WriteError::Unsupported {
            output: id(),
            what: "a mode wider or taller than 65535",
        });
    }
    let custom = entry.output.as_ref().is_some_and(|output| {
        !output.modes.is_empty() && !output.modes.iter().any(|listed| listed.matches(mode))
    });
    let refresh = match mode.refresh_mhz {
        0 if custom => {
            return Err(WriteError::Unsupported {
                output: id(),
                what: "a mode its output does not list without its refresh",
            });
        }
        0 => String::new(),
        mhz => format!("@{}", mhz_to_hz_text(mhz)),
    };
    let mut settings = vec![
        format!(
            "mode {}\"{}x{}{refresh}\"",
            if custom { "custom=true " } else { "" },
            mode.width,
            mode.height
        ),
        format!("scale {}", state.scale),
        format!("transform \"{}\"", state.transform),
        format!("position x={} y={}", state.x, state.y),
    ];
    match state.adaptive_sync {
        Some(AdaptiveSync::On) => settings.push("variable-refresh-rate".to_owned()),
        Some(AdaptiveSync::Automatic) => {
            settings.push("variable-refresh-rate on-demand=true".to_owned());
        }
        Some(AdaptiveSync::Off) | None => {}
    }
    Ok(node(&name, &settings))
}

/// Every entry's node, in layout order, a blank line between each.
///
/// # Errors
///
/// As [`output_node`], for the first entry that fails.
pub fn output_nodes(layout: &Layout) -> Result<String, WriteError> {
    Ok(layout
        .entries
        .iter()
        .map(|entry| output_node(layout, entry))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n"))
}

fn node(name: &str, settings: &[String]) -> String {
    let body: String = settings
        .iter()
        .map(|setting| format!("    {setting}\n"))
        .collect();
    format!("output {name} {{\n{body}}}\n")
}

#[cfg(test)]
pub(super) mod tests {
    use dsplr_model::{
        AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::{WriteError, output_node, output_nodes};

    /// The owner's machine: the panel off, the AOC at 2560x1440@120.
    pub(in crate::niri) fn owners_layout() -> Layout {
        let output = |connector: &str, description: &str| Output {
            connector: connector.into(),
            description: description.into(),
            physical_size_mm: None,
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            modes: vec![Mode::new(2560, 1440, 120_000)],
        };
        Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(output("eDP-1", "BOE 0x0AFC")),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                    output: Some(output("HDMI-A-1", "AOC Q27G4 SERIAL0000001")),
                    state: OutputState {
                        mode: Some(Mode::new(2560, 1440, 120_000)),
                        ..OutputState::default()
                    },
                },
            ],
            ..Default::default()
        }
    }

    /// The golden file for [`owners_layout`].
    pub(in crate::niri) const OWNERS_NODES: &str = r#"output "eDP-1" {
    off
}

output "AOC Q27G4 SERIAL0000001" {
    mode "2560x1440@120"
    scale 1
    transform "normal"
    position x=0 y=0
}
"#;

    #[test]
    fn the_owners_layout_writes_one_node_per_output() {
        assert_eq!(output_nodes(&owners_layout()).unwrap(), OWNERS_NODES);
    }

    /// niri names transforms as wl_output does (its `90` turns
    /// counter-clockwise, as dsplr's does), unlike sway.
    #[test]
    fn every_setting_is_written_in_niri_s_terms() {
        let mut layout = owners_layout();
        let state = &mut layout.entries[1].state;
        state.mode = Some(Mode::new(1920, 1080, 59_940));
        (state.x, state.y) = (-1920, 240);
        state.scale = 1.25;
        state.transform = Transform::Flipped90;
        state.adaptive_sync = Some(AdaptiveSync::On);
        assert_eq!(
            output_node(&layout, &layout.entries[1]).unwrap(),
            r#"output "AOC Q27G4 SERIAL0000001" {
    mode custom=true "1920x1080@59.94"
    scale 1.25
    transform "flipped-90"
    position x=-1920 y=240
    variable-refresh-rate
}
"#
        );
        layout.entries[1].state.adaptive_sync = Some(AdaptiveSync::Automatic);
        let node = output_node(&layout, &layout.entries[1]).unwrap();
        assert!(
            node.contains("    variable-refresh-rate on-demand=true\n"),
            "{node}"
        );
        for off in [Some(AdaptiveSync::Off), None] {
            layout.entries[1].state.adaptive_sync = off;
            let node = output_node(&layout, &layout.entries[1]).unwrap();
            assert!(!node.contains("variable-refresh-rate"), "{node}");
        }
    }

    /// A mode with refresh 0 (not said) is written without one, so niri
    /// picks the highest at that size; a custom mode needs one.
    #[test]
    fn a_mode_without_a_refresh_is_written_without_one_unless_it_is_custom() {
        let mut layout = owners_layout();
        layout.entries[1].state.mode = Some(Mode::new(2560, 1440, 0));
        layout.entries[1].output.as_mut().unwrap().modes = vec![Mode::new(2560, 1440, 0)];
        let node = output_node(&layout, &layout.entries[1]).unwrap();
        assert!(node.contains("    mode \"2560x1440\"\n"), "{node}");
        layout.entries[1].state.mode = Some(Mode::new(1600, 900, 0));
        assert!(matches!(
            output_node(&layout, &layout.entries[1]),
            Err(WriteError::Unsupported { what, .. }) if what.contains("without its refresh")
        ));
        // Modes not known at all: nothing is custom.
        layout.entries[1].output = None;
        let node = output_node(&layout, &layout.entries[1]).unwrap();
        assert!(node.contains("    mode \"1600x900\"\n"), "{node}");
    }

    #[test]
    fn what_niri_s_output_config_cannot_hold_is_refused() {
        let aoc = OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1");
        type Edit = fn(&mut OutputState);
        let edits: [(Edit, &str); 7] = [
            (
                |s| s.mirror_of = Some(OutputId::Connector("eDP-1".into())),
                "a mirror",
            ),
            (|s| s.primary = true, "a primary output"),
            (|s| s.colour_preset = Some("hdr".into()), "a colour preset"),
            (|s| s.bit_depth = Some(10), "a bit depth"),
            (|s| s.overscan = Some(5), "overscan"),
            (
                |s| s.rgb_range = Some(dsplr_model::RgbRange::Full),
                "an RGB range",
            ),
            (
                |s| s.mode = Some(Mode::new(65_536, 1080, 60_000)),
                "a mode wider or taller than 65535",
            ),
        ];
        for (edit, what) in edits {
            let mut layout = owners_layout();
            layout.entries[1].output = None;
            edit(&mut layout.entries[1].state);
            assert_eq!(
                output_nodes(&layout),
                Err(WriteError::Unsupported {
                    output: aoc.clone(),
                    what
                })
            );
        }
    }

    #[test]
    fn a_layout_that_cannot_be_written_says_why() {
        let aoc = OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1");
        let mut layout = owners_layout();
        layout.entries[1].state.mode = None;
        assert_eq!(
            output_node(&layout, &layout.entries[1]),
            Err(WriteError::NoMode(aoc.clone()))
        );
        for scale in [0.0, -1.0, 10.5, f64::NAN] {
            let mut layout = owners_layout();
            layout.entries[1].state.scale = scale;
            assert_eq!(
                output_node(&layout, &layout.entries[1]),
                Err(WriteError::InvalidScale(aoc.clone())),
                "{scale}"
            );
        }
        let mut layout = owners_layout();
        layout.entries[1].state.scale = 10.0;
        assert!(output_node(&layout, &layout.entries[1]).is_ok());
        let unnamed = OutputEntry {
            id: OutputId::Connector(String::new()),
            output: None,
            state: OutputState::default(),
        };
        assert_eq!(
            output_node(&Layout::default(), &unnamed),
            Err(WriteError::NoName(OutputId::Connector(String::new())))
        );
        let control = OutputEntry {
            id: OutputId::Connector("DP\n1".into()),
            output: None,
            state: OutputState::default(),
        };
        assert!(matches!(
            output_node(&Layout::default(), &control),
            Err(WriteError::Unquotable { name, .. }) if name == "DP\n1"
        ));
    }
}
