//! Writing `output` blocks. Every block of an enabled output states each
//! setting dsplr owns, because sway merges an output's config onto what it
//! had and keeps a setting left out as it was at runtime (seen on headless
//! sway 1.12: a mode set before a reload stays when the config names none).

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, is_valid_scale, mhz_to_hz_text};

use super::{output_name, quoted, transform_name};

/// Why a layout could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("{0}: no connector or identifier to name it by")]
    NoName(OutputId),
    #[error("{0}: enabled without a mode")]
    NoMode(OutputId),
    #[error("{0}: the scale is not a positive number")]
    InvalidScale(OutputId),
    /// A connector, or a name read from a file, that dsplr cannot quote so
    /// that sway reads it back as it is: one with a quote, a backslash, a
    /// `$` (sway's variables) or a control character in it. An identifier
    /// like that is not an error: the output is named by its connector.
    #[error("{output}: its name {name:?} cannot be written into a sway config")]
    Unquotable { output: OutputId, name: String },
    /// A setting sway's output config has no command for.
    #[error("{output}: sway's output config cannot hold {what}")]
    Unsupported {
        output: OutputId,
        what: &'static str,
    },
}

/// The `output <name> { … }` block for one entry of `layout`, ending in a
/// newline. `layout` is needed to tell whether the entry's identifier is
/// its own ([`super::output_name`]).
///
/// What is written, and why:
/// - `disable` alone for an output that is off;
/// - `enable` on every other, so one that was off comes back on;
/// - `mode --custom` for a mode its output is known not to list, as
///   sway-output(5) asks, and the refresh only when it is known;
/// - `adaptive_sync` only when the layout knows it.
///
/// # Errors
///
/// When the entry has no name or one sway cannot read back, an enabled
/// entry has no mode or an unusable scale, or it asks for a setting sway's
/// output config cannot hold: a mirror, a primary output, colour settings or
/// automatic adaptive sync.
pub fn output_block(layout: &Layout, entry: &OutputEntry) -> Result<String, WriteError> {
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
        (
            state.adaptive_sync == Some(AdaptiveSync::Automatic),
            "automatic adaptive sync",
        ),
    ];
    if let Some((_, what)) = refused.into_iter().find(|(set, _)| *set) {
        return Err(WriteError::Unsupported { output: id(), what });
    }
    if !state.enabled {
        return Ok(block(&name, &["disable".to_owned()]));
    }
    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| WriteError::NoMode(id()))?;
    if !is_valid_scale(state.scale) {
        return Err(WriteError::InvalidScale(id()));
    }
    let custom = entry.output.as_ref().is_some_and(|output| {
        !output.modes.is_empty() && !output.modes.iter().any(|listed| listed.matches(mode))
    });
    let refresh = match mode.refresh_mhz {
        0 => String::new(),
        mhz => format!("@{}Hz", mhz_to_hz_text(mhz)),
    };
    let mut settings = vec![
        "enable".to_owned(),
        format!(
            "mode {}{}x{}{refresh}",
            if custom { "--custom " } else { "" },
            mode.width,
            mode.height
        ),
        format!("position {} {}", state.x, state.y),
        format!("scale {}", state.scale),
        format!("transform {}", transform_name(state.transform)),
    ];
    match state.adaptive_sync {
        Some(AdaptiveSync::On) => settings.push("adaptive_sync on".to_owned()),
        Some(AdaptiveSync::Off) => settings.push("adaptive_sync off".to_owned()),
        Some(AdaptiveSync::Automatic) | None => {}
    }
    Ok(block(&name, &settings))
}

/// Every entry's block, in layout order, a blank line between each.
///
/// # Errors
///
/// As [`output_block`], for the first entry that fails.
pub fn output_blocks(layout: &Layout) -> Result<String, WriteError> {
    Ok(layout
        .entries
        .iter()
        .map(|entry| output_block(layout, entry))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n"))
}

fn block(name: &str, settings: &[String]) -> String {
    let body: String = settings
        .iter()
        .map(|setting| format!("    {setting}\n"))
        .collect();
    format!("output {name} {{\n{body}}}\n")
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::{WriteError, output_block, output_blocks};
    use crate::sway::quoted;

    /// The owner's machine: the panel off, the AOC at 2560x1440@120.
    fn owners_layout() -> Layout {
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

    #[test]
    fn the_owners_layout_writes_one_block_per_output() {
        assert_eq!(
            output_blocks(&owners_layout()).unwrap(),
            r#"output eDP-1 {
    disable
}

output "AOC Q27G4 SERIAL0000001" {
    enable
    mode 2560x1440@120Hz
    position 0 0
    scale 1
    transform normal
}
"#
        );
    }

    #[test]
    fn every_setting_is_written_in_sway_s_terms() {
        let mut layout = owners_layout();
        let state = &mut layout.entries[1].state;
        state.mode = Some(Mode::new(1920, 1080, 59_940));
        (state.x, state.y) = (-1920, 240);
        state.scale = 1.25;
        state.transform = Transform::Rotate90;
        state.adaptive_sync = Some(AdaptiveSync::On);
        assert_eq!(
            output_block(&layout, &layout.entries[1]).unwrap(),
            r#"output "AOC Q27G4 SERIAL0000001" {
    enable
    mode --custom 1920x1080@59.94Hz
    position -1920 240
    scale 1.25
    transform 270
    adaptive_sync on
}
"#
        );
        layout.entries[1].state.adaptive_sync = Some(AdaptiveSync::Off);
        let block = output_block(&layout, &layout.entries[1]).unwrap();
        assert!(block.contains("    adaptive_sync off\n"), "{block}");
    }

    /// Headless sway reports refresh 0, the protocol's "not said": the mode
    /// is written without one. A listed mode is not `--custom`, nor is any
    /// mode of an output whose modes are not known.
    #[test]
    fn a_mode_is_custom_only_when_its_output_is_known_not_to_list_it() {
        let mut layout = owners_layout();
        let aoc = &mut layout.entries[1];
        aoc.state.mode = Some(Mode::new(1600, 900, 0));
        aoc.output.as_mut().unwrap().modes = vec![Mode::new(1600, 900, 0)];
        let block = output_block(&layout, &layout.entries[1]).unwrap();
        assert!(block.contains("    mode 1600x900\n"), "{block}");
        layout.entries[1].output = None;
        let block = output_block(&layout, &layout.entries[1]).unwrap();
        assert!(block.contains("    mode 1600x900\n"), "{block}");
    }

    #[test]
    fn what_sway_s_output_config_cannot_hold_is_refused() {
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
                |s| s.adaptive_sync = Some(AdaptiveSync::Automatic),
                "automatic adaptive sync",
            ),
        ];
        for (edit, what) in edits {
            let mut layout = owners_layout();
            edit(&mut layout.entries[1].state);
            assert_eq!(
                output_blocks(&layout),
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
            output_block(&layout, &layout.entries[1]),
            Err(WriteError::NoMode(aoc.clone()))
        );
        let mut layout = owners_layout();
        layout.entries[1].state.scale = 0.0;
        assert_eq!(
            output_block(&layout, &layout.entries[1]),
            Err(WriteError::InvalidScale(aoc))
        );
        let unnamed = OutputEntry {
            id: OutputId::Connector(String::new()),
            output: None,
            state: OutputState::default(),
        };
        assert_eq!(
            output_block(&Layout::default(), &unnamed),
            Err(WriteError::NoName(OutputId::Connector(String::new())))
        );
        let quote = OutputEntry {
            id: OutputId::new("Acme", "27\" panel", "1", "DP-1"),
            output: None,
            state: OutputState::default(),
        };
        assert!(matches!(
            output_block(&Layout::default(), &quote),
            Err(WriteError::Unquotable { name, .. }) if name == "Acme 27\" panel 1"
        ));
    }

    #[test]
    fn a_name_is_quoted_unless_it_is_plain() {
        assert_eq!(quoted("HDMI-A-1").as_deref(), Some("HDMI-A-1"));
        assert_eq!(
            quoted("Dell Inc. U2720Q").as_deref(),
            Some("\"Dell Inc. U2720Q\"")
        );
        assert_eq!(quoted("*").as_deref(), Some("\"*\""));
        for bad in ["", "a\"b", "a'b", "a\\b", "$mon", "a\nb"] {
            assert_eq!(quoted(bad), None, "{bad:?}");
        }
    }
}
