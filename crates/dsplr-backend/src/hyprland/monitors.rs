//! Hyprland's `j/monitors all` reply, as Hyprland writes it.

use dsplr_model::{Mode, ModeSpec};
use serde::Deserialize;

/// One monitor from `j/monitors all`: every output Hyprland knows, disabled
/// ones included. Field names follow the reply; only the fields dsplr reads
/// are kept.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    /// Hyprland's own number for the monitor; what `mirrorOf` refers to.
    pub id: i64,
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
    /// The `id` of the monitor this one mirrors, or `none`. Hyprland writes
    /// the id, not the connector (`HyprCtl.cpp`, 0.56 and 0.57 alike).
    pub mirror_of: String,
    /// Every mode the monitor offers, as `WIDTHxHEIGHT@REFRESHHz` strings.
    pub available_modes: Vec<String>,
    /// The DRM format it is driven in (`XRGB8888`, `XRGB2101010`); `Invalid`
    /// while disabled. An `XRGB2101010` is 10 bits per channel, which a
    /// `bitdepth = 10` rule gets where the output can do it (0.57.2, CI run
    /// 42784568478). Empty from a Hyprland that does not say.
    #[serde(default)]
    pub current_format: String,
    /// The colour management preset in force, by the name a `cm` rule
    /// takes. A preset the output cannot do falls back: an EDID-less output
    /// given `edid`, `hdr`, `hdredid` or `auto` reported `srgb` (0.57.2, CI
    /// run 42784568478). Empty from a Hyprland that does not say.
    #[serde(default)]
    pub color_management_preset: String,
}

/// The DRM formats of 8 bits per colour channel (`drm_fourcc.h`'s 32-bit
/// RGB formats with 8 bits each), by the names Hyprland writes.
const EIGHT_BIT_FORMATS: [&str; 8] = [
    "XRGB8888", "ARGB8888", "XBGR8888", "ABGR8888", "RGBX8888", "RGBA8888", "BGRX8888", "BGRA8888",
];

/// The DRM formats of 10 bits per colour channel (`drm_fourcc.h`'s
/// 2101010 and 1010102 RGB formats).
const TEN_BIT_FORMATS: [&str; 8] = [
    "XRGB2101010",
    "ARGB2101010",
    "XBGR2101010",
    "ABGR2101010",
    "RGBX1010102",
    "RGBA1010102",
    "BGRX1010102",
    "BGRA1010102",
];

/// What `mirrorOf` holds when the monitor mirrors nothing.
const NOT_MIRRORING: &str = "none";

impl Monitor {
    /// Parses the reply to `j/monitors all`.
    pub fn parse_all(reply: &str) -> serde_json::Result<Vec<Self>> {
        serde_json::from_str(reply)
    }

    /// The `id` of the monitor this one mirrors: `Ok(None)` when it mirrors
    /// nothing, `Err` with the text when `mirrorOf` is neither `none` nor an
    /// id.
    pub fn mirror_source(&self) -> Result<Option<i64>, &str> {
        if self.mirror_of == NOT_MIRRORING {
            return Ok(None);
        }
        self.mirror_of
            .parse()
            .map(Some)
            .map_err(|_| self.mirror_of.as_str())
    }

    /// Bits per channel by [`Monitor::current_format`]: 8 or 10 for a
    /// format known to be one or the other; `None` for any other (none
    /// while disabled, a half-float format), which is then neither compared
    /// nor written.
    pub fn bit_depth(&self) -> Option<u8> {
        let format = self.current_format.as_str();
        if EIGHT_BIT_FORMATS.contains(&format) {
            Some(8)
        } else if TEN_BIT_FORMATS.contains(&format) {
            Some(10)
        } else {
            None
        }
    }

    /// The colour preset in force; `None` when Hyprland says none.
    pub fn colour_preset(&self) -> Option<String> {
        (!self.color_management_preset.is_empty()).then(|| self.color_management_preset.clone())
    }

    /// The modes in [`Monitor::available_modes`], parsed.
    pub fn modes(&self) -> Result<Vec<Mode>, ModeError> {
        self.available_modes
            .iter()
            .map(|mode| parse_mode(mode))
            .collect()
    }
}

/// A mode string Hyprland wrote that is not `WIDTHxHEIGHT@REFRESHHz`.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("Hyprland mode {0:?} is not WIDTHxHEIGHT@REFRESHHz")]
pub struct ModeError(String);

/// Parses one of Hyprland's `WIDTHxHEIGHT@REFRESHHz` modes, such as
/// `2560x1440@120.00Hz`. Hyprland lists no preferred mode and no per-mode
/// scales here, so those stay empty.
pub fn parse_mode(text: &str) -> Result<Mode, ModeError> {
    text.strip_suffix("Hz")
        .and_then(|mode| mode.parse::<ModeSpec>().ok())
        .and_then(ModeSpec::to_mode)
        .filter(|mode| mode.refresh_mhz > 0)
        .ok_or_else(|| ModeError(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use dsplr_model::Mode;

    use super::{ModeError, Monitor, parse_mode};

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
        assert_eq!(external.mirror_source(), Ok(None));

        let modes = external.modes().expect("every mode parses");
        assert_eq!(modes.len(), external.available_modes.len());
        assert!(modes.contains(&Mode::new(2560, 1440, 120_000)));
        assert!(modes.contains(&Mode::new(3840, 2160, 59_940)));
    }

    #[test]
    fn modes_parse_to_millihertz() {
        for (text, mode) in [
            ("2560x1440@120.00Hz", Mode::new(2560, 1440, 120_000)),
            ("1920x1080@119.88Hz", Mode::new(1920, 1080, 119_880)),
            ("640x480@59.94Hz", Mode::new(640, 480, 59_940)),
        ] {
            assert_eq!(parse_mode(text), Ok(mode), "{text}");
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
            "1920x1080@1e2Hz",
            "1920x1080@60HzHz",
            "1920x1080@inf",
            "+1920x1080@60.00Hz",
        ] {
            assert_eq!(
                parse_mode(text),
                Err(ModeError(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_mirror_names_its_source_by_id() {
        let mut monitor = Monitor::parse_all(OWNER).expect("parses").remove(1);
        monitor.mirror_of = "0".to_owned();
        assert_eq!(monitor.mirror_source(), Ok(Some(0)));
        monitor.mirror_of = "eDP-1".to_owned();
        assert_eq!(monitor.mirror_source(), Err("eDP-1"), "a name is not an id");
    }
}
