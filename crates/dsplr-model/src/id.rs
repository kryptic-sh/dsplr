use std::fmt;

use serde::{Deserialize, Serialize};

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
}

/// The connector name, or `make model serial`.
impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connector(connector) => f.write_str(connector),
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
