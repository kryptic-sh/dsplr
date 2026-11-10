//! Hyprland's `monitors.lua`: one `hl.monitor({ … })` call per output, read
//! from Hyprland v0.56.2's `hlMonitor` binding and its monitor rule parsers
//! (`src/config/lua/bindings/LuaBindingsConfigRules.cpp`,
//! `src/config/shared/monitor/Parser.cpp`).

mod lua;
mod read;
mod save;
mod write;

use dsplr_model::{Layout, OutputEntry, OutputId};

pub use read::{ReadError, ReadErrorKind, read_monitors_lua};
pub use save::{REQUIRE_LINE, SaveError, save};
pub use write::{WriteError, monitor_call, monitor_calls};

/// How the file names each monitor (`docs/plan/12-open-questions.md`,
/// question 4). Either way, a monitor without a serial is named by its
/// connector: Hyprland matches `desc:` by prefix, so a description without
/// a serial would catch every monitor of that model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonitorNaming {
    /// `desc:<make model serial>`, so a monitor keeps its settings on
    /// another port. The owner's default.
    #[default]
    Description,
    /// The connector (`HDMI-A-1`), as nwg-displays writes, so the settings
    /// stay with the port.
    Connector,
}

/// A `monitors.lua` as dsplr reads and writes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorsLua {
    pub layout: Layout,
    /// The catch-all `hl.monitor({ output = "", … })` calls for monitors the
    /// file does not name, in file order, each kept as its text, since their
    /// `preferred` and `auto` values have no place in a `Layout`. Every one
    /// is kept, not only the last: Hyprland merges a rule onto the one
    /// before it of the same name, field by field, so an earlier call's
    /// fields still count where a later one leaves them out.
    pub catch_alls: Vec<String>,
}

/// The description to name an entry's monitor by: only when it has a
/// serial, since Hyprland matches `desc:` by prefix and a description without
/// one would catch every monitor of that model.
fn description_of(entry: &OutputEntry) -> Option<String> {
    match &entry.id {
        OutputId::Connector(_) => None,
        OutputId::Edid { serial, .. } if serial.trim().is_empty() => None,
        OutputId::Edid { .. } | OutputId::Description(_) => entry.description(),
    }
}

/// The `output` value naming `entry`, or `None` when it has no usable name.
fn output_name(entry: &OutputEntry, naming: MonitorNaming) -> Option<String> {
    let by_description = || description_of(entry).map(|description| format!("desc:{description}"));
    let by_connector = || entry.connector().map(str::to_owned);
    match naming {
        MonitorNaming::Description => by_description().or_else(by_connector),
        MonitorNaming::Connector => by_connector().or_else(by_description),
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Output, OutputEntry, OutputId, OutputState};

    use super::{MonitorNaming, output_name};

    fn entry(id: OutputId, output: Option<(&str, &str)>) -> OutputEntry {
        OutputEntry {
            id,
            output: output.map(|(connector, description)| Output {
                connector: connector.into(),
                description: description.into(),
                physical_size_mm: None,
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                modes: vec![],
            }),
            state: OutputState::default(),
        }
    }

    fn edid(serial: &str) -> OutputId {
        OutputId::Edid {
            make: "AOC".into(),
            model: "Q27G4".into(),
            serial: serial.into(),
        }
    }

    #[test]
    fn description_naming_uses_the_reported_description() {
        let external = entry(
            edid("SERIAL0000001"),
            Some(("HDMI-A-1", "AOC Q27G4 SERIAL0000001")),
        );
        assert_eq!(
            output_name(&external, MonitorNaming::Description).as_deref(),
            Some("desc:AOC Q27G4 SERIAL0000001")
        );
        assert_eq!(
            output_name(&external, MonitorNaming::Connector).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn without_hardware_facts_the_description_is_rebuilt_without_commas() {
        let file_entry = entry(
            OutputId::Edid {
                make: "Dell, Inc.".into(),
                model: "U2720Q".into(),
                serial: "ABC123".into(),
            },
            None,
        );
        assert_eq!(
            output_name(&file_entry, MonitorNaming::Description).as_deref(),
            Some("desc:Dell Inc. U2720Q ABC123")
        );
        // No connector is known, so connector naming falls back to it too.
        assert_eq!(
            output_name(&file_entry, MonitorNaming::Connector).as_deref(),
            Some("desc:Dell Inc. U2720Q ABC123")
        );
    }

    /// An empty `desc:` matches every monitor, and a description without a
    /// serial every monitor of that model; neither may ever be written.
    #[test]
    fn a_monitor_without_a_serial_is_named_by_its_connector() {
        let panel = entry(
            OutputId::Connector("eDP-1".into()),
            Some(("eDP-1", "BOE 0x0AFC")),
        );
        assert_eq!(
            output_name(&panel, MonitorNaming::Description).as_deref(),
            Some("eDP-1")
        );
        let blank_serial = entry(edid("  "), Some(("DP-2", "AOC Q27G4")));
        assert_eq!(
            output_name(&blank_serial, MonitorNaming::Description).as_deref(),
            Some("DP-2")
        );
        let blank_description = entry(OutputId::Description(" ".into()), None);
        assert_eq!(
            output_name(&blank_description, MonitorNaming::Description),
            None
        );
    }
}
