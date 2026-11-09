//! Reading a `monitors.lua`, dsplr's own or nwg-displays', back into a
//! layout. Calls for the same `output` merge, a later field winning, as
//! Hyprland merges a rule onto the existing one of that name.
//!
//! A setting dsplr has no place for (Hyprland's SDR and luminance keys,
//! `reserved`, an `auto` position or a `preferred` mode, `vrr = 3`, or any
//! setting beside `disabled = true`) is an error naming it, never dropped: a
//! file read without it would be saved back without it.

use dsplr_model::{
    AdaptiveSync, Mode, ModeSpec, OutputEntry, OutputId, OutputState, Transform, is_valid_scale,
};

use super::MonitorsLua;
use super::lua::{self, Value};

/// Why a `monitors.lua` could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("monitors.lua line {line}: {kind}")]
pub struct ReadError {
    pub line: usize,
    pub kind: ReadErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadErrorKind {
    #[error("{0}")]
    Syntax(String),
    #[error("an `hl.monitor` call names no `output`")]
    MissingOutput,
    #[error("`{0}` is a setting dsplr cannot keep, so the file is not read rather than lose it")]
    UnsupportedField(String),
    #[error("`{field} = {value}` has no place in a dsplr layout yet")]
    UnsupportedValue { field: String, value: String },
    #[error(
        "`{0}` is set on a disabled output, and a dsplr layout keeps nothing but `disabled` for one"
    )]
    SettingOnDisabled(String),
    #[error("`{field} = {value}` is not a value Hyprland takes there")]
    InvalidValue { field: String, value: String },
}

/// One field's value and the line it is on.
type Located = (Value, usize);

/// The fields of every call for one output, merged, later ones winning.
#[derive(Default)]
struct Rule {
    disabled: Option<Located>,
    mode: Option<Located>,
    position: Option<Located>,
    scale: Option<Located>,
    transform: Option<Located>,
    vrr: Option<Located>,
    mirror: Option<Located>,
    bitdepth: Option<Located>,
    cm: Option<Located>,
}

/// Reads a `monitors.lua`.
///
/// Adaptive sync reads back as not set when the file leaves `vrr` out or
/// sets it to -1 (no override), and as automatic for 2 (fullscreen only). 3
/// (fullscreen video and games) is an error: a layout has only one
/// automatic, which is written back as 2.
///
/// # Errors
///
/// On Lua outside `hl.monitor` calls, and on any setting or value the layout
/// cannot hold; the error names the line.
pub fn read_monitors_lua(source: &str) -> Result<MonitorsLua, ReadError> {
    let calls = lua::parse(source).map_err(|error| ReadError {
        line: error.line,
        kind: ReadErrorKind::Syntax(error.message),
    })?;
    let mut catch_all = None;
    let mut rules: Vec<(String, usize, Rule)> = Vec::new();
    for call in calls {
        let Some((output, line)) = call
            .fields
            .iter()
            .rev()
            .find(|field| field.key == "output")
            .map(|field| (field.value.clone(), field.line))
        else {
            return Err(ReadError {
                line: call.line,
                kind: ReadErrorKind::MissingOutput,
            });
        };
        let Value::Str(output) = output else {
            return Err(invalid("output", &output, line));
        };
        if output.is_empty() {
            catch_all = Some(call.text);
            continue;
        }
        let index = match rules.iter().position(|(name, ..)| *name == output) {
            Some(index) => index,
            None => {
                rules.push((output, line, Rule::default()));
                rules.len() - 1
            }
        };
        let rule = &mut rules[index].2;
        for field in call.fields {
            let slot = match field.key.as_str() {
                "output" => continue,
                "disabled" => &mut rule.disabled,
                "mode" => &mut rule.mode,
                "position" => &mut rule.position,
                "scale" => &mut rule.scale,
                "transform" => &mut rule.transform,
                "vrr" => &mut rule.vrr,
                "mirror" => &mut rule.mirror,
                "bitdepth" => &mut rule.bitdepth,
                "cm" => &mut rule.cm,
                _ => {
                    return Err(ReadError {
                        line: field.line,
                        kind: ReadErrorKind::UnsupportedField(field.key),
                    });
                }
            };
            *slot = Some((field.value, field.line));
        }
    }
    let entries = rules
        .into_iter()
        .map(|(name, line, rule)| entry(&name, line, rule))
        .collect::<Result<_, _>>()?;
    Ok(MonitorsLua {
        layout: dsplr_model::Layout { entries },
        catch_all,
    })
}

fn entry(name: &str, line: usize, rule: Rule) -> Result<OutputEntry, ReadError> {
    let id = output_id(name).ok_or_else(|| unsupported("output", "\"desc:\"", line))?;
    let disabled = match rule.disabled {
        None => false,
        Some((Value::Bool(disabled), _)) => disabled,
        Some((value, line)) => return Err(invalid("disabled", &value, line)),
    };
    let mut state = OutputState {
        enabled: !disabled,
        ..OutputState::default()
    };
    if disabled {
        let Rule {
            disabled: _,
            mode,
            position,
            scale,
            transform,
            vrr,
            mirror,
            bitdepth,
            cm,
        } = rule;
        let others = [
            ("mode", mode),
            ("position", position),
            ("scale", scale),
            ("transform", transform),
            ("vrr", vrr),
            ("mirror", mirror),
            ("bitdepth", bitdepth),
            ("cm", cm),
        ];
        if let Some((field, (_, line))) = others
            .into_iter()
            .filter_map(|(field, value)| value.map(|value| (field, value)))
            .min_by_key(|(_, (_, line))| *line)
        {
            return Err(ReadError {
                line,
                kind: ReadErrorKind::SettingOnDisabled(field.to_owned()),
            });
        }
        return Ok(OutputEntry {
            id,
            output: None,
            state,
        });
    }
    state.mode = Some(mode(rule.mode, line)?);
    (state.x, state.y) = position(rule.position, line)?;
    state.scale = scale(rule.scale, line)?;
    if let Some((value, line)) = rule.transform {
        state.transform = integer(&value)
            .and_then(|n| u32::try_from(n).ok())
            .and_then(Transform::from_wayland)
            .ok_or_else(|| invalid("transform", &value, line))?;
    }
    if let Some((value, line)) = rule.vrr {
        state.adaptive_sync = match integer(&value) {
            Some(-1) => None,
            Some(0) => Some(AdaptiveSync::Off),
            Some(1) => Some(AdaptiveSync::On),
            Some(2) => Some(AdaptiveSync::Automatic),
            Some(3) => return Err(unsupported("vrr", "3", line)),
            _ => return Err(invalid("vrr", &value, line)),
        };
    }
    if let Some((value, line)) = rule.mirror {
        let Value::Str(source) = &value else {
            return Err(invalid("mirror", &value, line));
        };
        if !source.is_empty() {
            state.mirror_of =
                Some(output_id(source).ok_or_else(|| unsupported("mirror", "\"desc:\"", line))?);
        }
    }
    if let Some((value, line)) = rule.bitdepth {
        state.bit_depth = Some(
            integer(&value)
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| invalid("bitdepth", &value, line))?,
        );
    }
    if let Some((value, line)) = rule.cm {
        let Value::Str(preset) = value else {
            return Err(invalid("cm", &value, line));
        };
        state.colour_preset = Some(preset);
    }
    Ok(OutputEntry {
        id,
        output: None,
        state,
    })
}

/// `desc:X` as a description, anything else as a connector. `None` for an
/// empty description, which Hyprland matches against every monitor.
fn output_id(name: &str) -> Option<OutputId> {
    match name.strip_prefix("desc:") {
        Some(description) => {
            let description = description.trim();
            (!description.is_empty()).then(|| OutputId::Description(description.to_owned()))
        }
        None => Some(OutputId::Connector(name.to_owned())),
    }
}

/// `WxH@R`, the refresh in hertz with an optional `Hz` after it
/// ([`ModeSpec`]). Hyprland's other forms (`preferred`, `highres`, a modeline, or
/// no refresh) pick a mode from the live monitor, which a file has no list
/// of, so they are refused.
fn mode(field: Option<Located>, line: usize) -> Result<Mode, ReadError> {
    let Some((value, line)) = field else {
        return Err(unsupported("mode", "\"preferred\" (the default)", line));
    };
    let Value::Str(text) = &value else {
        return Err(invalid("mode", &value, line));
    };
    let parsed = text
        .strip_suffix("Hz")
        .unwrap_or(text)
        .parse::<ModeSpec>()
        .ok()
        .and_then(ModeSpec::to_mode);
    parsed.ok_or_else(|| unsupported("mode", &render(&value), line))
}

/// `XxY`. `auto` and its variants place the monitor next to the others when
/// Hyprland applies the rule, so a file does not say where it is.
fn position(field: Option<Located>, line: usize) -> Result<(i32, i32), ReadError> {
    let Some((value, line)) = field else {
        return Err(unsupported("position", "\"auto\" (the default)", line));
    };
    let Value::Str(text) = &value else {
        return Err(invalid("position", &value, line));
    };
    let parsed = text
        .split_once('x')
        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)));
    parsed.ok_or_else(|| unsupported("position", &render(&value), line))
}

/// A number, as a Lua number or a numeric string. `auto` lets Hyprland pick,
/// which a layout cannot hold.
fn scale(field: Option<Located>, line: usize) -> Result<f64, ReadError> {
    let Some((value, line)) = field else {
        return Err(unsupported("scale", "\"auto\" (the default)", line));
    };
    let scale = match &value {
        Value::Number(scale) => Some(*scale),
        Value::Str(text) => text.parse().ok(),
        Value::Bool(_) => None,
    };
    match scale {
        Some(scale) if is_valid_scale(scale) => Ok(scale),
        _ => Err(unsupported("scale", &render(&value), line)),
    }
}

/// A Lua number with no fractional part, as an integer.
fn integer(value: &Value) -> Option<i64> {
    match value {
        // Whole and inside i64's range, checked first, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
        _ => None,
    }
}

/// A value as it would read in Lua, for an error message.
fn render(value: &Value) -> String {
    match value {
        Value::Str(text) => format!("{text:?}"),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
    }
}

fn invalid(field: &str, value: &Value, line: usize) -> ReadError {
    ReadError {
        line,
        kind: ReadErrorKind::InvalidValue {
            field: field.to_owned(),
            value: render(value),
        },
    }
}

fn unsupported(field: &str, value: &str, line: usize) -> ReadError {
    ReadError {
        line,
        kind: ReadErrorKind::UnsupportedValue {
            field: field.to_owned(),
            value: value.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{AdaptiveSync, Layout, Mode, OutputEntry, OutputId, OutputState, Transform};

    use super::{ReadError, ReadErrorKind, read_monitors_lua};
    use crate::hyprland::{MonitorNaming, MonitorsLua, monitor_calls};

    /// The owner's `monitors.lua` as nwg-displays generated it, serial
    /// masked to the same length.
    const NWG_DISPLAYS: &str = include_str!("fixtures/nwg-displays-monitors.lua");

    #[test]
    fn the_nwg_displays_file_reads_as_the_owners_layout() {
        let file = read_monitors_lua(NWG_DISPLAYS).unwrap();
        assert_eq!(
            file,
            MonitorsLua {
                layout: Layout {
                    entries: vec![
                        OutputEntry {
                            id: OutputId::Connector("eDP-1".into()),
                            output: None,
                            state: OutputState {
                                enabled: false,
                                ..OutputState::default()
                            },
                        },
                        OutputEntry {
                            id: OutputId::Connector("HDMI-A-1".into()),
                            output: None,
                            state: OutputState {
                                mode: Some(Mode::new(2560, 1440, 120_000)),
                                ..OutputState::default()
                            },
                        },
                    ],
                },
                catch_all: None,
            }
        );
    }

    #[test]
    fn what_dsplr_writes_reads_back_the_same() {
        let file = MonitorsLua {
            layout: Layout {
                entries: vec![
                    OutputEntry {
                        id: OutputId::Connector("eDP-1".into()),
                        output: None,
                        state: OutputState {
                            mode: Some(Mode::new(1920, 1080, 59_940)),
                            x: -1920,
                            y: 180,
                            scale: 1.25,
                            transform: Transform::Rotate270,
                            adaptive_sync: Some(AdaptiveSync::On),
                            bit_depth: Some(10),
                            colour_preset: Some("srgb".into()),
                            ..OutputState::default()
                        },
                    },
                    OutputEntry {
                        id: OutputId::Description("AOC Q27G4 SERIAL0000001".into()),
                        output: None,
                        state: OutputState {
                            mode: Some(Mode::new(2560, 1440, 143_998)),
                            adaptive_sync: Some(AdaptiveSync::Automatic),
                            mirror_of: Some(OutputId::Connector("eDP-1".into())),
                            ..OutputState::default()
                        },
                    },
                    OutputEntry {
                        id: OutputId::Connector("DP-3".into()),
                        output: None,
                        state: OutputState {
                            enabled: false,
                            ..OutputState::default()
                        },
                    },
                ],
            },
            catch_all: Some(
                r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })"#
                    .into(),
            ),
        };
        let text = monitor_calls(&file, MonitorNaming::Description).unwrap();
        assert_eq!(read_monitors_lua(&text).unwrap(), file);
    }

    #[test]
    fn later_calls_for_the_same_output_win_field_by_field() {
        let file = read_monitors_lua(
            r#"
hl.monitor({ output = "DP-1", mode = "1920x1080@60", position = "0x0", scale = 1, vrr = 1 })
hl.monitor({ output = "DP-1", position = "1920x0" })
hl.monitor({ output = "", mode = "preferred" })
hl.monitor({ output = "", mode = "highres" })
"#,
        )
        .unwrap();
        let state = &file.layout.entries[0].state;
        assert_eq!(file.layout.entries.len(), 1);
        assert_eq!(
            (state.x, state.adaptive_sync),
            (1920, Some(AdaptiveSync::On))
        );
        assert_eq!(
            file.catch_all.as_deref(),
            Some(r#"hl.monitor({ output = "", mode = "highres" })"#)
        );
    }

    #[test]
    fn fullscreen_only_vrr_reads_as_automatic_and_video_and_games_is_refused() {
        let text = |vrr: u8| {
            format!(
                "hl.monitor({{ output = \"DP-1\", mode = \"1920x1080@60\", position = \"0x0\", scale = 1, vrr = {vrr} }})"
            )
        };
        let file = read_monitors_lua(&text(2)).unwrap();
        assert_eq!(
            file.layout.entries[0].state.adaptive_sync,
            Some(AdaptiveSync::Automatic)
        );
        // Read as automatic, it would be written back as 2.
        assert!(matches!(
            error(&text(3)).kind,
            ReadErrorKind::UnsupportedValue { ref field, ref value } if field == "vrr" && value == "3"
        ));
    }

    #[test]
    fn a_setting_beside_disabled_is_an_error_not_a_loss() {
        assert_eq!(
            error("hl.monitor({ output = \"DP-1\", disabled = true,\n  scale = 2 })"),
            ReadError {
                line: 2,
                kind: ReadErrorKind::SettingOnDisabled("scale".into()),
            }
        );
        // Merged from two calls, as Hyprland merges them.
        let merged = "hl.monitor({ output = \"DP-1\", mode = \"1920x1080@60\", position = \"0x0\", scale = 1 })\nhl.monitor({ output = \"DP-1\", disabled = true })";
        assert!(matches!(
            error(merged).kind,
            ReadErrorKind::SettingOnDisabled(ref field) if field == "mode"
        ));
        // `disabled = false` beside settings is an enabled output.
        assert!(read_monitors_lua("hl.monitor({ output = \"DP-1\", disabled = false, mode = \"1920x1080@60\", position = \"0x0\", scale = 1 })").is_ok());
    }

    fn error(source: &str) -> ReadError {
        read_monitors_lua(source).unwrap_err()
    }

    #[test]
    fn a_setting_dsplr_cannot_keep_is_an_error_not_a_loss() {
        assert_eq!(
            error("hl.monitor({ output = \"DP-1\",\n  sdrbrightness = 1.2 })"),
            ReadError {
                line: 2,
                kind: ReadErrorKind::UnsupportedField("sdrbrightness".into()),
            }
        );
        fn kind(source: &str) -> ReadErrorKind {
            error(source).kind
        }
        assert!(matches!(
            kind(r#"hl.monitor({ output = "DP-1", mode = "preferred", position = "0x0", scale = 1 })"#),
            ReadErrorKind::UnsupportedValue { ref field, .. } if field == "mode"
        ));
        assert!(matches!(
            kind(r#"hl.monitor({ output = "DP-1", mode = "1920x1080@60", position = "auto", scale = 1 })"#),
            ReadErrorKind::UnsupportedValue { ref field, .. } if field == "position"
        ));
        assert!(matches!(
            kind(r#"hl.monitor({ output = "DP-1", mode = "1920x1080@60", position = "0x0" })"#),
            ReadErrorKind::UnsupportedValue { ref field, .. } if field == "scale"
        ));
        assert!(matches!(
            kind(r#"hl.monitor({ output = "desc: ", disabled = true })"#),
            ReadErrorKind::UnsupportedValue { ref field, .. } if field == "output"
        ));
        for mode in [
            "1920x1080",
            "1920x1080@1e2",
            "1920x1080@60HzHz",
            "1920x1080@inf",
        ] {
            let source = format!(
                r#"hl.monitor({{ output = "DP-1", mode = "{mode}", position = "0x0", scale = 1 }})"#
            );
            assert!(
                matches!(
                    kind(&source),
                    ReadErrorKind::UnsupportedValue { ref field, .. } if field == "mode"
                ),
                "{mode}"
            );
        }
    }

    #[test]
    fn a_mode_reads_with_or_without_its_hz() {
        for mode in ["1920x1080@59.94", "1920x1080@59.94Hz"] {
            let text = format!(
                r#"hl.monitor({{ output = "DP-1", mode = "{mode}", position = "0x0", scale = 1 }})"#
            );
            let layout = read_monitors_lua(&text).expect("reads").layout;
            assert_eq!(
                layout.entries[0].state.mode,
                Some(Mode::new(1920, 1080, 59_940)),
                "{mode}"
            );
        }
    }

    #[test]
    fn a_value_hyprland_would_refuse_is_an_error() {
        fn kind(source: &str) -> ReadErrorKind {
            error(source).kind
        }
        let base = r#"output = "DP-1", mode = "1920x1080@60", position = "0x0", scale = 1"#;
        for (field, value) in [
            ("transform", "8"),
            ("transform", "1.5"),
            ("vrr", "4"),
            ("bitdepth", "300"),
            ("disabled", "\"yes\""),
            ("mirror", "1"),
            ("cm", "true"),
        ] {
            let source = format!("hl.monitor({{ {base}, {field} = {value} }})");
            assert!(
                matches!(
                    kind(&source),
                    ReadErrorKind::InvalidValue { field: ref f, .. } if f == field
                ),
                "{field} = {value}"
            );
        }
        assert_eq!(
            error("\nhl.monitor({ mode = \"1920x1080@60\" })"),
            ReadError {
                line: 2,
                kind: ReadErrorKind::MissingOutput,
            }
        );
        assert!(matches!(kind("print(1)"), ReadErrorKind::Syntax(_)));
    }
}
