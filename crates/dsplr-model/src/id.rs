use std::fmt;

use serde::{Deserialize, Serialize};

use crate::OutputEntry;

/// Stable identity of a monitor, for matching it across reconnects, between
/// backends and against saved layouts.
///
/// `make` is the company name wlroots and Hyprland report; a backend that
/// is given the EDID's PNP vendor code turns it into that name first
/// ([`crate::PnpNames::make`]), so the forms agree.
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
    /// serial. It names a live output by [`OutputId::matches`]; Hyprland's
    /// own reading is [`OutputId::matches_as_hyprland`].
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

    /// Whether this identity, from a saved layout or the command line,
    /// names the live output `live`, by dsplr's rule:
    /// - an EDID identity, when `live` has the same one;
    /// - a connector, when `live` is on that connector;
    /// - a description, when `live`'s description is it or starts with it
    ///   and a space: whole words, so `AOC Q27G4` names every Q27G4 but
    ///   `AOC Q27G4 SER1` does not name `AOC Q27G4 SER12`. A blank
    ///   description matches nothing.
    ///
    /// This is how dsplr picks an output. What Hyprland will do with a rule
    /// is [`OutputId::matches_as_hyprland`].
    pub fn matches(&self, live: &OutputEntry) -> bool {
        self.matches_with(live, |live, description| {
            live.strip_prefix(description)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
    }

    /// Whether Hyprland takes a monitor rule naming this identity to name
    /// `live`: as [`OutputId::matches`], except that a description names
    /// every monitor whose description starts with it, even mid-word, which
    /// is Hyprland's rule for `desc:` (`CMonitor::matchesStaticSelector`).
    /// For modelling the rules Hyprland reads, not for picking an output. A
    /// blank description matches nothing here, though Hyprland would match
    /// every monitor with it.
    pub fn matches_as_hyprland(&self, live: &OutputEntry) -> bool {
        self.matches_with(live, |live, description| live.starts_with(description))
    }

    /// [`OutputId::matches`], with `named` deciding whether a live
    /// description (first) is named by a non-blank one (second).
    fn matches_with(&self, live: &OutputEntry, named: impl Fn(&str, &str) -> bool) -> bool {
        match self {
            Self::Edid { .. } => live.id == *self,
            Self::Connector(connector) => live.connector() == Some(connector.as_str()),
            Self::Description(description) => {
                let description = description.trim();
                !description.is_empty()
                    && live
                        .description()
                        .is_some_and(|live| named(&live, description))
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
                make: String::new(),
                model: String::new(),
                serial: String::new(),
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
        for matches in [OutputId::matches, OutputId::matches_as_hyprland] {
            assert!(matches(&edid, &aoc) && !matches(&edid, &panel));
            assert!(matches(&desc("AOC Q27G4 SERIAL0000001"), &aoc), "exact");
            assert!(matches(&desc("  AOC Q27G4 "), &aoc), "whole words");
            assert!(!matches(&desc("AOC Q27G4 SERIAL0000002"), &aoc));
            assert!(!matches(&desc("Q27G4"), &aoc), "a prefix, not a substring");
            assert!(!matches(&desc(""), &aoc), "blank never matches");
            assert!(!matches(&desc("  "), &panel));
        }
    }

    #[test]
    fn dsplr_matches_a_description_by_whole_words_and_hyprland_by_prefix() {
        let ser = |serial: &str| {
            live(
                OutputId::new("AOC", "Q27G4", serial, "DP-1"),
                "DP-1",
                &format!("AOC Q27G4 {serial}"),
            )
        };
        let (ser1, ser12) = (ser("SER1"), ser("SER12"));
        let desc = |text: &str| OutputId::Description(text.into());

        assert!(desc("AOC Q27G4 SER1").matches(&ser1), "exact");
        assert!(!desc("AOC Q27G4 SER1").matches(&ser12), "SER1 is not SER12");
        assert!(desc("AOC Q27G4").matches(&ser12), "a word ends there");
        assert!(!desc("AOC Q27").matches(&ser12), "mid-word");

        assert!(desc("AOC Q27G4 SER1").matches_as_hyprland(&ser1));
        assert!(
            desc("AOC Q27G4 SER1").matches_as_hyprland(&ser12),
            "Hyprland's desc: is a plain prefix"
        );
        assert!(desc("AOC Q27").matches_as_hyprland(&ser12));
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
