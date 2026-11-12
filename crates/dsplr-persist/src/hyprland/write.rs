//! Writing `hl.monitor` calls. Every call states each setting dsplr owns,
//! because Hyprland merges a rule onto the existing one for the same output:
//! a setting left out keeps whatever it was (`disabled`, `mirror`), which
//! matters for a live `eval` even more than for a file read fresh.

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, is_valid_scale};

use super::{MonitorNaming, MonitorsLua, output_name};

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
/// - `vrr` only when the layout knows it: a rule a snapshot could not see
///   (Hyprland reports whether adaptive sync is active, not its rule) is
///   left as it is rather than overwritten with a guess.
/// - `bitdepth` and `cm` whenever the layout knows them, 8 and `srgb`
///   included: Hyprland merges a rule onto the one before it for the same
///   output, so a setting left out keeps an earlier rule's (a `bitdepth = 8`
///   after a `bitdepth = 10` turned it off, 0.57.2, CI run 42784568478).
///   The live apply leaves out what its change does not set, and saving
///   leaves the defaults out ([`crate::hyprland::save`]); both by setting
///   them to `None` before writing.
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
    monitor_call_with(layout, entry, naming, &[])
}

/// [`monitor_call`], with `extra` fields (Lua `key = value`) after the ones
/// dsplr writes.
fn monitor_call_with(
    layout: &Layout,
    entry: &OutputEntry,
    naming: MonitorNaming,
    extra: &[String],
) -> Result<String, WriteError> {
    let name = output_name(entry, naming).ok_or_else(|| WriteError::NoName(entry.id.clone()))?;
    let state = &entry.state;
    let mut fields = vec![format!("output = {}", lua_string(&name))];
    if !state.enabled {
        fields.push("disabled = true".to_owned());
        fields.extend_from_slice(extra);
        return Ok(call(&fields));
    }
    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| WriteError::NoMode(entry.id.clone()))?;
    if !is_valid_scale(state.scale) {
        return Err(WriteError::InvalidScale(entry.id.clone()));
    }
    let mirror = match &state.mirror_of {
        None => String::new(),
        Some(source) => {
            mirror_name(layout, source).ok_or_else(|| WriteError::MirrorSourceUnnamed {
                output: entry.id.clone(),
                mirrored: Box::new(source.clone()),
            })?
        }
    };
    fields.extend([
        "disabled = false".to_owned(),
        format!("mode = {}", lua_string(&mode.to_string())),
        format!(
            "position = {}",
            lua_string(&format!("{}x{}", state.x, state.y))
        ),
        format!("scale = {}", state.scale),
        format!("transform = {}", state.transform.to_wayland()),
        format!("mirror = {}", lua_string(&mirror)),
    ]);
    if let Some(adaptive_sync) = state.adaptive_sync {
        let vrr = match adaptive_sync {
            AdaptiveSync::Off => 0,
            AdaptiveSync::On => 1,
            AdaptiveSync::Automatic => 2,
        };
        fields.push(format!("vrr = {vrr}"));
    }
    if let Some(bits) = state.bit_depth {
        fields.push(format!("bitdepth = {bits}"));
    }
    if let Some(preset) = &state.colour_preset {
        fields.push(format!("cm = {}", lua_string(preset)));
    }
    fields.extend_from_slice(extra);
    Ok(call(&fields))
}

/// Every call for `file`, in the order Hyprland should read them: the
/// catch-alls first, in their order (they only apply to monitors no other
/// rule names, so their place among the others does not matter), then each
/// entry in layout order: its [`MonitorsLua::verbatim`] text when it has
/// one, else its call with its [`MonitorsLua::extras`].
///
/// # Errors
///
/// As [`monitor_call`], for the first entry that fails.
pub fn monitor_calls(file: &MonitorsLua, naming: MonitorNaming) -> Result<String, WriteError> {
    let calls = file
        .layout
        .entries
        .iter()
        .map(|entry| {
            let name = output_name(entry, naming);
            if let Some(text) = by_name(&file.verbatim, name.as_deref()) {
                return Ok(format!("{}\n", text.trim_end()));
            }
            let extra = by_name(&file.extras, name.as_deref()).map_or(&[][..], Vec::as_slice);
            monitor_call_with(&file.layout, entry, naming, extra)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(file
        .catch_alls
        .iter()
        .map(|catch_all| format!("{}\n", catch_all.trim_end()))
        .chain(calls)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The value `list` holds for the `output` name `name`.
fn by_name<'a, T>(list: &'a [(String, T)], name: Option<&str>) -> Option<&'a T> {
    let name = name?;
    list.iter()
        .find(|(other, _)| other == name)
        .map(|(_, value)| value)
}

/// How a `mirror` field names `source`: the connector of the entry of
/// `layout` it is, or failing that names (as a file's `desc:` mirror names a
/// live output), else `source`'s own connector, or its description as
/// `desc:` when that is all it is (a mirror read from a file, of a monitor
/// not connected now). `None` when it has no usable name.
///
/// A `desc:` source is resolved by Hyprland's prefix rule
/// ([`OutputId::matches_as_hyprland`]), not dsplr's whole words: written
/// back as `desc:`, it is Hyprland that will match it, so the connector
/// written in its place must be the monitor Hyprland would pick.
fn mirror_name(layout: &Layout, source: &OutputId) -> Option<String> {
    let in_layout = layout
        .entries
        .iter()
        .find(|other| other.id == *source)
        .or_else(|| {
            layout
                .entries
                .iter()
                .find(|other| source.matches_as_hyprland(other))
        });
    if let Some(connector) = in_layout.and_then(OutputEntry::connector) {
        return Some(connector.to_owned());
    }
    match source {
        OutputId::Connector(connector) if !connector.is_empty() => Some(connector.clone()),
        OutputId::Description(description) if !description.trim().is_empty() => {
            Some(format!("desc:{}", description.trim()))
        }
        _ => None,
    }
}

fn call(fields: &[String]) -> String {
    let body: String = fields
        .iter()
        .map(|field| format!("    {field},\n"))
        .collect();
    format!("hl.monitor({{\n{body}}})\n")
}

/// `text` as a double-quoted Lua string literal.
pub(super) fn lua_string(text: &str) -> String {
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
                        make: String::new(),
                        model: String::new(),
                        serial: String::new(),
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
                        make: String::new(),
                        model: String::new(),
                        serial: String::new(),
                        modes: vec![],
                    }),
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
    fn the_owners_layout_writes_by_description_by_default() {
        let file = MonitorsLua {
            layout: owners_layout(),
            catch_alls: Vec::new(),
            ..Default::default()
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
            catch_alls: vec![
                r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })"#
                    .into(),
            ],
            ..Default::default()
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
        state.adaptive_sync = Some(AdaptiveSync::Automatic);
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
    fn unknown_adaptive_sync_is_left_to_hyprland_and_eight_bits_is_said() {
        let mut layout = owners_layout();
        let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
        assert!(!call.contains("vrr"), "{call}");
        assert!(!call.contains("bitdepth"), "not known: {call}");
        // 8 is written: Hyprland merges a rule onto the one before it, so
        // leaving it out would keep an earlier `bitdepth = 10`.
        layout.entries[1].state.bit_depth = Some(8);
        let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
        assert!(call.contains("    bitdepth = 8,\n"), "{call}");
        for (setting, vrr) in [
            (AdaptiveSync::Off, "    vrr = 0,\n"),
            (AdaptiveSync::On, "    vrr = 1,\n"),
        ] {
            layout.entries[1].state.adaptive_sync = Some(setting);
            let call = monitor_call(&layout, &layout.entries[1], MonitorNaming::Connector).unwrap();
            assert!(call.contains(vrr), "{call}");
        }
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
        let missing = OutputId::new("Some", "Monitor", "1234", "DP-9");
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

    #[test]
    fn a_mirror_named_by_description_is_written_by_connector_or_kept_as_desc() {
        let mut layout = owners_layout();
        layout.entries[0].state.enabled = true;
        layout.entries[0].state.mode = Some(Mode::new(1920, 1080, 60_000));
        // A file's `desc:` mirror of a monitor in the layout: its connector.
        layout.entries[0].state.mirror_of = Some(OutputId::Description("AOC Q27G4".into()));
        let call = monitor_call(&layout, &layout.entries[0], MonitorNaming::Connector).unwrap();
        assert!(call.contains("    mirror = \"HDMI-A-1\",\n"), "{call}");
        // Of a monitor not in the layout: written back as it was read.
        layout.entries[0].state.mirror_of = Some(OutputId::Description("Dell U2720Q X1".into()));
        let call = monitor_call(&layout, &layout.entries[0], MonitorNaming::Connector).unwrap();
        assert!(
            call.contains("    mirror = \"desc:Dell U2720Q X1\",\n"),
            "{call}"
        );
    }
}
