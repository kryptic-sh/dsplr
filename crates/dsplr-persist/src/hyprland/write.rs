//! Writing `hl.monitor` calls. Every call states each setting dsplr owns,
//! because Hyprland merges a rule onto the existing one for the same output:
//! a setting left out keeps whatever it was (`disabled`, `mirror`), which
//! matters for a live `eval` even more than for a file read fresh.

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, mhz_to_hz_text};

use super::{MonitorNaming, MonitorsLua, connector_of, output_name};

/// Why a layout could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("{0}: no connector or description to name it by")]
    NoName(OutputId),
    #[error("{0}: enabled without a mode")]
    NoMode(OutputId),
    #[error("{0}: the scale is not a positive number")]
    InvalidScale(OutputId),
    #[error("{output}: mirrors {mirrored}, which has no connector to name it by")]
    MirrorSourceUnnamed {
        output: OutputId,
        mirrored: Box<OutputId>,
    },
}

/// The `hl.monitor({ … })` call for one entry of `layout`, ending in a
/// newline. `layout` is needed to name the output it mirrors.
///
/// What is written, and why:
/// - `disabled = false` on every enabled output, and `mirror = ""` when it
///   mirrors nothing, so a re-enabled or unmirrored output does not keep the
///   old setting.
/// - `vrr` only when it is on or automatic: a monitor whose rule is
///   fullscreen-only reads back from Hyprland as off, and writing `vrr = 0`
///   would clobber that rule. Turning adaptive sync off against a global
///   `misc:vrr` therefore cannot be written yet.
/// - `bitdepth` only when it is 10, the one value Hyprland acts on.
/// - `cm` only when the layout has a colour preset.
///
/// # Errors
///
/// When the entry has no name, an enabled entry has no mode or an unusable
/// scale, or the output it mirrors has no connector.
pub fn monitor_call(
    layout: &Layout,
    entry: &OutputEntry,
    naming: MonitorNaming,
) -> Result<String, WriteError> {
    let name = output_name(entry, naming).ok_or_else(|| WriteError::NoName(entry.id.clone()))?;
    let state = &entry.state;
    let mut fields = vec![format!("output = {}", lua_string(&name))];
    if !state.enabled {
        fields.push("disabled = true".to_owned());
        return Ok(call(&fields));
    }
    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| WriteError::NoMode(entry.id.clone()))?;
    if !(state.scale.is_finite() && state.scale > 0.0) {
        return Err(WriteError::InvalidScale(entry.id.clone()));
    }
    let mirror = match &state.mirror_of {
        None => String::new(),
        Some(source) => layout
            .entries
            .iter()
            .find(|other| other.id == *source)
            .and_then(connector_of)
            .or(match source {
                OutputId::Connector(connector) => Some(connector.as_str()),
                _ => None,
            })
            .ok_or_else(|| WriteError::MirrorSourceUnnamed {
                output: entry.id.clone(),
                mirrored: Box::new(source.clone()),
            })?
            .to_owned(),
    };
    fields.extend([
        "disabled = false".to_owned(),
        format!(
            "mode = {}",
            lua_string(&format!(
                "{}x{}@{}",
                mode.width,
                mode.height,
                mhz_to_hz_text(mode.refresh_mhz)
            ))
        ),
        format!(
            "position = {}",
            lua_string(&format!("{}x{}", state.x, state.y))
        ),
        format!("scale = {}", state.scale),
        format!("transform = {}", state.transform.to_wayland()),
        format!("mirror = {}", lua_string(&mirror)),
    ]);
    match state.adaptive_sync {
        AdaptiveSync::Off => {}
        AdaptiveSync::On => fields.push("vrr = 1".to_owned()),
        AdaptiveSync::Automatic => fields.push("vrr = 2".to_owned()),
    }
    if state.bit_depth == Some(10) {
        fields.push("bitdepth = 10".to_owned());
    }
    if let Some(preset) = &state.colour_preset {
        fields.push(format!("cm = {}", lua_string(preset)));
    }
    Ok(call(&fields))
}

/// Every call for `file`, in the order Hyprland should read them: the
/// catch-all first (it only applies to monitors no other rule names, so its
/// place does not matter), then each entry in layout order.
///
/// # Errors
///
/// As [`monitor_call`], for the first entry that fails.
pub fn monitor_calls(file: &MonitorsLua, naming: MonitorNaming) -> Result<String, WriteError> {
    let calls = file
        .layout
        .entries
        .iter()
        .map(|entry| monitor_call(&file.layout, entry, naming))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(file
        .catch_all
        .iter()
        .map(|catch_all| format!("{}\n", catch_all.trim_end()))
        .chain(calls)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn call(fields: &[String]) -> String {
    let body: String = fields
        .iter()
        .map(|field| format!("    {field},\n"))
        .collect();
    format!("hl.monitor({{\n{body}}})\n")
}

/// `text` as a double-quoted Lua string literal.
fn lua_string(text: &str) -> String {
    let body: String = text
        .chars()
        .map(|c| match c {
            '"' => "\\\"".to_owned(),
            '\\' => "\\\\".to_owned(),
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            // Lua's decimal escape names one byte, so it is only right for
            // the single-byte controls; three digits, so a following digit
            // is not read into it. Anything else is valid UTF-8 as it is.
            c if c < ' ' || c == '\u{7f}' => format!("\\{:03}", u32::from(c)),
            c => c.to_string(),
        })
        .collect();
    format!("\"{body}\"")
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::{WriteError, lua_string, monitor_call, monitor_calls};
    use crate::hyprland::{MonitorNaming, MonitorsLua};

    /// The owner's machine: the panel off, the AOC at 2560x1440@120.
    fn owners_layout() -> Layout {
        Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(Output {
                        connector: "eDP-1".into(),
                        description: "BOE 0x0AFC".into(),
                        physical_size_mm: None,
                        modes: vec![],
                    }),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                    output: Some(Output {
                        connector: "HDMI-A-1".into(),
                        description: "AOC Q27G4 SERIAL0000001".into(),
                        physical_size_mm: None,
                        modes: vec![],
                    }),
                    state: OutputState {
                        mode: Some(Mode::new(2560, 1440, 120_000)),
                        ..OutputState::default()
                    },
                },
            ],
        }
    }

    #[test]
    fn the_owners_layout_writes_by_description_by_default() {
        let file = MonitorsLua {
            layout: owners_layout(),
            catch_all: None,
        };
        assert_eq!(
            monitor_calls(&file, MonitorNaming::default()).unwrap(),
            r#"hl.monitor({
    output = "eDP-1",
    disabled = true,
})

hl.monitor({
    output = "desc:AOC Q27G4 SERIAL0000001",
    disabled = false,
    mode = "2560x1440@120",
    position = "0x0",
    scale = 1,
    transform = 0,
    mirror = "",
})
"#
        );
    }

    #[test]
    fn the_catch_all_goes_first_and_connector_naming_names_ports() {
        let file = MonitorsLua {
            layout: owners_layout(),
            catch_all: Some(
                r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })"#
                    .into(),
            ),
        };
        let text = monitor_calls(&file, MonitorNaming::Connector).unwrap();
        assert!(
            text.starts_with(
                "hl.monitor({ output = \"\", mode = \"preferred\", position = \"auto\", scale = 1 })\n\nhl.monitor({\n    output = \"eDP-1\","
            ),
            "{text}"
        );
        assert!(text.contains("    output = \"HDMI-A-1\",\n"), "{text}");
        assert!(!text.contains("desc:"), "{text}");
    }

    #[test]
    fn every_optional_setting_is_written_in_hyprlands_terms() {
        let mut layout = owners_layout();
        layout.entries[0].state.enabled = true;
        layout.entries[0].state.mode = Some(Mode::new(1920, 1080, 59_940));
        let state = &mut layout.entries[1].state;
        state.x = -1920;
        state.y = 240;
        state.scale = 1.25;
        state.transform = Transform::Flipped90;
        state.adaptive_sync = AdaptiveSync::Automatic;
        state.mirror_of = Some(OutputId::Connector("eDP-1".into()));
        state.bit_depth = Some(10);
        state.colour_preset = Some("hdr".into());
        let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
        assert_eq!(
            call,
            r#"hl.monitor({
    output = "HDMI-A-1",
    disabled = false,
    mode = "2560x1440@120",
    position = "-1920x240",
    scale = 1.25,
    transform = 5,
    mirror = "eDP-1",
    vrr = 2,
    bitdepth = 10,
    cm = "hdr",
})
"#
        );
        let panel = monitor_call(&layout, &layout.entries[0], MonitorNaming::Connector).unwrap();
        assert!(
            panel.contains("    mode = \"1920x1080@59.94\",\n"),
            "{panel}"
        );
    }

    #[test]
    fn adaptive_sync_off_and_eight_bits_are_left_to_hyprland() {
        let mut layout = owners_layout();
        layout.entries[1].state.bit_depth = Some(8);
        let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
        assert!(!call.contains("vrr"), "{call}");
        assert!(!call.contains("bitdepth"), "{call}");
        layout.entries[1].state.adaptive_sync = AdaptiveSync::On;
        let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
        assert!(call.contains("    vrr = 1,\n"), "{call}");
    }

    #[test]
    fn a_layout_that_cannot_be_written_says_why() {
        let mut layout = owners_layout();
        layout.entries[1].state.mode = None;
        let external = OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1");
        assert_eq!(
            monitor_call(&layout, &layout.entries[1], MonitorNaming::default()),
            Err(WriteError::NoMode(external.clone()))
        );

        let mut layout = owners_layout();
        layout.entries[1].state.scale = f64::NAN;
        assert_eq!(
            monitor_call(&layout, &layout.entries[1], MonitorNaming::default()),
            Err(WriteError::InvalidScale(external.clone()))
        );

        let mut layout = owners_layout();
        let missing = OutputId::Description("Some Monitor 1234".into());
        layout.entries[1].state.mirror_of = Some(missing.clone());
        assert_eq!(
            monitor_call(&layout, &layout.entries[1], MonitorNaming::default()),
            Err(WriteError::MirrorSourceUnnamed {
                output: external,
                mirrored: Box::new(missing),
            })
        );
    }

    #[test]
    fn lua_strings_escape_what_would_end_or_break_them() {
        assert_eq!(lua_string("plain"), "\"plain\"");
        assert_eq!(lua_string(r#"a "b" \c"#), r#""a \"b\" \\c""#);
        assert_eq!(lua_string("line\nnext"), r#""line\nnext""#);
        assert_eq!(lua_string("bell\u{7}9"), r#""bell\0079""#);
    }
}
