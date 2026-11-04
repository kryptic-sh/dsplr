//! Hyprland's `j/monitors all` reply, as Hyprland writes it.

use std::fmt;

use serde::Deserialize;

/// One monitor from `j/monitors all`: every output Hyprland knows, disabled
/// ones included. Field names follow the reply; only the fields dsplr reads
/// are kept.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    /// The connector, such as `HDMI-A-1`.
    pub name: String,
    /// Make, model and serial in one string, as `desc:` monitor rules match.
    pub description: String,
    pub make: String,
    pub model: String,
    /// Empty when the monitor reports none.
    pub serial: String,
    /// Physical size in millimetres.
    pub physical_width: u32,
    pub physical_height: u32,
    /// The current mode's size in pixels; `0` while disabled.
    pub width: u32,
    pub height: u32,
    /// The current refresh rate in hertz.
    pub refresh_rate: f64,
    /// Position in layout coordinates; `-1` while disabled.
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// A Wayland `wl_output.transform` value.
    pub transform: u8,
    /// Whether adaptive sync is active right now.
    pub vrr: bool,
    pub disabled: bool,
    /// The connector this monitor mirrors, or `none`.
    pub mirror_of: String,
    /// Every mode the monitor offers, as `WIDTHxHEIGHT@REFRESHHz` strings.
    pub available_modes: Vec<String>,
    pub current_format: String,
}

/// What `mirrorOf` holds when the monitor mirrors nothing.
const NOT_MIRRORING: &str = "none";

impl Monitor {
    /// Parses the reply to `j/monitors all`.
    pub fn parse_all(reply: &str) -> serde_json::Result<Vec<Self>> {
        serde_json::from_str(reply)
    }

    /// The connector this monitor mirrors, if any.
    #[must_use]
    pub fn mirror_source(&self) -> Option<&str> {
        (self.mirror_of != NOT_MIRRORING).then_some(self.mirror_of.as_str())
    }

    /// The modes in [`Monitor::available_modes`], parsed.
    pub fn modes(&self) -> Result<Vec<Mode>, ModeError> {
        self.available_modes
            .iter()
            .map(|mode| mode.parse())
            .collect()
    }
}

/// One of Hyprland's `WIDTHxHEIGHT@REFRESHHz` modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    /// Refresh in millihertz, the unit dsplr's model uses, rounded from
    /// Hyprland's hertz with two decimals.
    pub refresh_mhz: u32,
}

/// A mode string Hyprland wrote that is not `WIDTHxHEIGHT@REFRESHHz`.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Hyprland mode {0:?} is not WIDTHxHEIGHT@REFRESHHz")]
pub struct ModeError(String);

/// Millihertz in a hertz.
const MHZ_PER_HZ: f64 = 1000.0;

impl std::str::FromStr for Mode {
    type Err = ModeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || ModeError(text.to_owned());
        let (size, refresh) = text
            .strip_suffix("Hz")
            .and_then(|mode| mode.split_once('@'))
            .ok_or_else(invalid)?;
        let (width, height) = size.split_once('x').ok_or_else(invalid)?;
        let hertz: f64 = refresh.parse().map_err(|_| invalid())?;
        let millihertz = (hertz * MHZ_PER_HZ).round();
        if !(millihertz > 0.0 && millihertz <= f64::from(u32::MAX)) {
            return Err(invalid());
        }
        Ok(Self {
            width: width.parse().map_err(|_| invalid())?,
            height: height.parse().map_err(|_| invalid())?,
            // Whole and within `u32`, checked above, so the cast is exact.
            refresh_mhz: millihertz as u32,
        })
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hertz = f64::from(self.refresh_mhz) / MHZ_PER_HZ;
        write!(f, "{}x{}@{hertz:.2}Hz", self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, ModeError, Monitor};

    /// `hyprctl -j monitors all` on the owner's machine (Hyprland 0.56.2,
    /// 2026-11-04): the laptop panel disabled, one external monitor. The
    /// external monitor's serial is replaced.
    const OWNER: &str = include_str!("../../tests/fixtures/hyprland/monitors-all.json");

    #[test]
    fn the_owner_s_monitors_parse() {
        let monitors = Monitor::parse_all(OWNER).expect("parses");
        let names: Vec<_> = monitors.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["eDP-1", "HDMI-A-1"]);

        let panel = &monitors[0];
        assert!(panel.disabled);
        assert_eq!((panel.x, panel.width), (-1, 0), "no geometry while off");
        assert_eq!(panel.serial, "");

        let external = &monitors[1];
        assert!(!external.disabled);
        assert_eq!(external.description, "AOC Q27G4 SERIAL0000001");
        assert_eq!(
            (external.make.as_str(), external.model.as_str()),
            ("AOC", "Q27G4")
        );
        assert_eq!((external.width, external.height), (2560, 1440));
        assert_eq!((external.x, external.y), (0, 0));
        assert_eq!(
            (external.physical_width, external.physical_height),
            (600, 340)
        );
        assert_eq!(external.mirror_source(), None);

        let modes = external.modes().expect("every mode parses");
        assert_eq!(modes.len(), external.available_modes.len());
        assert!(modes.contains(&Mode {
            width: 2560,
            height: 1440,
            refresh_mhz: 120_000,
        }));
        assert!(modes.contains(&Mode {
            width: 3840,
            height: 2160,
            refresh_mhz: 59_940,
        }));
    }

    #[test]
    fn a_mode_round_trips_through_hyprland_s_form() {
        for text in [
            "2560x1440@120.00Hz",
            "1920x1080@119.88Hz",
            "640x480@59.94Hz",
        ] {
            let mode: Mode = text.parse().expect(text);
            assert_eq!(mode.to_string(), text);
        }
    }

    #[test]
    fn malformed_modes_are_refused() {
        for text in [
            "",
            "2560x1440@120.00",
            "2560x1440",
            "2560@120.00Hz",
            "axb@60.00Hz",
            "1920x1080@fastHz",
            "1920x1080@0.00Hz",
            "1920x1080@-60.00Hz",
            "1920x1080@1e12Hz",
        ] {
            assert_eq!(
                text.parse::<Mode>(),
                Err(ModeError(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_mirror_names_its_source() {
        let mut monitor = Monitor::parse_all(OWNER).expect("parses").remove(1);
        monitor.mirror_of = "eDP-1".to_owned();
        assert_eq!(monitor.mirror_source(), Some("eDP-1"));
    }
}
