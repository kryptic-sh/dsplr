use std::fmt;

use serde::{Deserialize, Serialize};

use crate::OutputEntry;

/// Stable identity of a monitor, for matching it across reconnects, between
/// backends and against saved layouts.
///
/// `make` is kept as the backend reported it. The plan normalises it to the
/// EDID's PNP vendor code before comparing, because backends disagree on the
/// form; that normalisation is not implemented yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputId {
    /// The monitor's own identity, used whenever it reports a serial.
    Edid {
        make: String,
        model: String,
        serial: String,
    },
    /// The connector name (`HDMI-A-1`), used when there is no serial, so the
    /// identity follows the port rather than the monitor.
    Connector(String),
    /// A monitor as a config file names it by its description (Hyprland's
    /// `desc:` form), which cannot be split back into make, model and
    /// serial. It names a live output by [`OutputId::matches`].
    Description(String),
}

impl OutputId {
    /// Picks the identity the plan's rule gives: `Edid` when `serial` has
    /// anything but whitespace in it, else `Connector`.
    pub fn new(make: &str, model: &str, serial: &str, connector: &str) -> Self {
        if serial.trim().is_empty() {
            Self::Connector(connector.to_owned())
        } else {
            Self::Edid {
                make: make.to_owned(),
                model: model.to_owned(),
                serial: serial.to_owned(),
            }
        }
    }

    /// Whether this identity, from a saved layout, names the live output
    /// `live`:
    /// - an EDID identity, when `live` has the same one;
    /// - a connector, when `live` is on that connector;
    /// - a description, when `live`'s description starts with it, which is
    ///   Hyprland's rule for `desc:` (`CMonitor::matchesStaticSelector`). A
    ///   blank description matches nothing, though Hyprland would match
    ///   every monitor with it.
    pub fn matches(&self, live: &OutputEntry) -> bool {
        match self {
            Self::Edid { .. } => live.id == *self,
            Self::Connector(connector) => live.connector() == Some(connector.as_str()),
            Self::Description(description) => {
                let description = description.trim();
                !description.is_empty()
                    && live
                        .description()
                        .is_some_and(|live| live.starts_with(description))
            }
        }
    }
}

/// The connector name, `make model serial`, or the description.
impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connector(name) | Self::Description(name) => f.write_str(name),
            Self::Edid {
                make,
                model,
                serial,
            } => write!(f, "{make} {model} {serial}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputId;
    use crate::{Output, OutputEntry, OutputState};

    #[test]
    fn a_serial_makes_the_identity_follow_the_monitor() {
        assert_eq!(
            OutputId::new("AOC", "Q27G2G4", "1A2B3C", "DP-1"),
            OutputId::Edid {
                make: "AOC".into(),
                model: "Q27G2G4".into(),
                serial: "1A2B3C".into(),
            }
        );
    }

    #[test]
    fn an_identity_reads_as_its_connector_or_its_monitor() {
        assert_eq!(OutputId::Connector("DP-1".into()).to_string(), "DP-1");
        assert_eq!(
            OutputId::new("AOC", "Q27G2G4", "1A2B3C", "DP-1").to_string(),
            "AOC Q27G2G4 1A2B3C"
        );
    }

    fn live(id: OutputId, connector: &str, description: &str) -> OutputEntry {
        OutputEntry {
            id,
            output: Some(Output {
                connector: connector.into(),
                description: description.into(),
                physical_size_mm: None,
                modes: vec![],
            }),
            state: OutputState::default(),
        }
    }

    #[test]
    fn a_saved_identity_matches_the_live_output_it_names() {
        let aoc = live(
            OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
            "HDMI-A-1",
            "AOC Q27G4 SERIAL0000001",
        );
        let panel = live(OutputId::Connector("eDP-1".into()), "eDP-1", "BOE 0x0AFC");

        let edid = OutputId::new("AOC", "Q27G4", "SERIAL0000001", "DP-9");
        assert!(edid.matches(&aoc));
        assert!(!edid.matches(&panel));

        assert!(OutputId::Connector("HDMI-A-1".into()).matches(&aoc));
        assert!(!OutputId::Connector("DP-1".into()).matches(&aoc));

        let desc = |text: &str| OutputId::Description(text.into());
        assert!(desc("AOC Q27G4 SERIAL0000001").matches(&aoc));
        assert!(desc("  AOC Q27G4 ").matches(&aoc), "a prefix, as Hyprland");
        assert!(!desc("AOC Q27G4 SERIAL0000002").matches(&aoc));
        assert!(!desc("Q27G4").matches(&aoc), "a prefix, not a substring");
        assert!(!desc("").matches(&aoc), "blank never matches");
        assert!(!desc("  ").matches(&panel));
    }

    #[test]
    fn a_description_matches_one_rebuilt_from_the_edid() {
        let file_entry = OutputEntry {
            id: OutputId::new("Dell, Inc.", "U2720Q", "ABC123", "DP-1"),
            output: None,
            state: OutputState::default(),
        };
        assert!(OutputId::Description("Dell Inc. U2720Q".into()).matches(&file_entry));
    }

    #[test]
    fn no_serial_falls_back_to_the_connector() {
        assert_eq!(
            OutputId::new("AOC", "Q27G2G4", "", "DP-1"),
            OutputId::Connector("DP-1".into())
        );
        assert_eq!(
            OutputId::new("AOC", "Q27G2G4", "  ", "DP-1"),
            OutputId::Connector("DP-1".into())
        );
    }
}
