use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::name::{ParseNameError, parse_name};
use crate::{Mode, OutputId, Transform};

/// Two scales closer than this are the same scale. Backends report scales as
/// doubles or `wl_fixed`, so an exact comparison would tell two scales apart
/// over their last bit.
pub const SCALE_EPSILON: f64 = 1e-6;

/// Whether `scale` can be an output's scale: a finite number above zero.
pub fn is_valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

/// Every monitor and its settings: the one struct the GUI edits, the CLI
/// prints, and every backend and config format converts to and from.
///
/// Apply, test, persist and profiles take a whole `Layout`, since every
/// desktop applies a configuration atomically. Fields will be added (the
/// fallback rule for unlisted monitors, GNOME's layout mode), so build one
/// with `..Default::default()`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Layout {
    pub entries: Vec<OutputEntry>,
}

/// One monitor: its hardware facts, when known, and its settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputEntry {
    pub id: OutputId,
    /// `None` for a layout read back from a config file, which knows a
    /// monitor's name and settings but not its modes; the GUI fills it from
    /// the live snapshot by `id`.
    pub output: Option<Output>,
    pub state: OutputState,
}

impl OutputEntry {
    /// The connector this monitor is on: its hardware facts' connector, or
    /// the one its identity names. `None` when neither is known or it is
    /// empty.
    pub fn connector(&self) -> Option<&str> {
        self.output
            .as_ref()
            .map(|output| output.connector.as_str())
            .filter(|connector| !connector.is_empty())
            .or(match &self.id {
                OutputId::Connector(connector) if !connector.is_empty() => Some(connector),
                _ => None,
            })
    }

    /// The monitor's description: the one the backend reported, the one its
    /// identity holds, or rebuilt from its EDID identity as Hyprland builds
    /// it (`make model serial`, commas removed, trimmed). `None` when there
    /// is none, or it is blank.
    pub fn description(&self) -> Option<String> {
        let reported = self
            .output
            .as_ref()
            .map(|output| output.description.trim())
            .filter(|description| !description.is_empty());
        let description = match (reported, &self.id) {
            (Some(reported), _) => reported.to_owned(),
            (None, OutputId::Description(description)) => description.trim().to_owned(),
            (
                None,
                OutputId::Edid {
                    make,
                    model,
                    serial,
                },
            ) => format!("{make} {model} {serial}")
                .replace(',', "")
                .trim()
                .to_owned(),
            (None, OutputId::Connector(_)) => return None,
        };
        (!description.is_empty()).then_some(description)
    }
}

/// Hardware facts about a connected monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// `HDMI-A-1`, `DP-2`, `eDP-1`.
    pub connector: String,
    /// The human-readable name the backend reports.
    pub description: String,
    /// Width and height in millimetres, when the monitor reports them.
    pub physical_size_mm: Option<(u32, u32)>,
    /// The modes the monitor offers.
    pub modes: Vec<Mode>,
}

/// The settings of one monitor.
///
/// Fields will be added, so build one with `..Default::default()`. The
/// default is an enabled output at the origin, scale 1, untransformed, with
/// no mode chosen and no optional setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputState {
    pub enabled: bool,
    /// The mode it is set to; a custom mode when its output does not list it.
    pub mode: Option<Mode>,
    /// Horizontal position in layout (logical) coordinates.
    pub x: i32,
    /// Vertical position in layout (logical) coordinates.
    pub y: i32,
    pub scale: f64,
    pub transform: Transform,
    /// `None` when it is not known or not set: a backend that reports only
    /// whether adaptive sync is active right now cannot say which rule is in
    /// force, and a file may leave it to the desktop. Writers leave a `None`
    /// out rather than guess.
    pub adaptive_sync: Option<AdaptiveSync>,
    /// The output this one mirrors, if any.
    pub mirror_of: Option<OutputId>,
    pub primary: bool,
    /// Hyprland's colour management preset (`srgb`, `wide`, `hdr`, …).
    pub colour_preset: Option<String>,
    /// Bits per colour channel (Hyprland's `bitdepth`).
    pub bit_depth: Option<u8>,
    /// Overscan in percent (KDE).
    pub overscan: Option<u8>,
    /// RGB range (KDE).
    pub rgb_range: Option<RgbRange>,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: None,
            x: 0,
            y: 0,
            scale: 1.0,
            transform: Transform::Normal,
            adaptive_sync: None,
            mirror_of: None,
            primary: false,
            colour_preset: None,
            bit_depth: None,
            overscan: None,
            rgb_range: None,
        }
    }
}

impl OutputState {
    /// The output's size on the canvas, in layout coordinates: its mode's
    /// size, turned by its transform, divided by its scale. `None` without a
    /// mode, or with a scale that is not a positive number.
    ///
    /// Rounded to the nearest whole pixel. Each backend rounds a fractional
    /// size its own way (2560 at scale 1.75); recording those is open, and
    /// until then this is the one rule.
    pub fn logical_size(&self) -> Option<(u32, u32)> {
        let mode = self.mode.as_ref()?;
        if !is_valid_scale(self.scale) {
            return None;
        }
        let (width, height) = if self.transform.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        let logical = |pixels: u32| {
            let size = (f64::from(pixels) / self.scale).round();
            // Whole and inside u32's range, checked here, so the cast is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            (size <= f64::from(u32::MAX)).then_some(size as u32)
        };
        Some((logical(width)?, logical(height)?))
    }
}

/// Variable refresh rate, one form for every desktop: wlroots' bool, KDE's
/// policy and Hyprland's integer `vrr` each map onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveSync {
    #[default]
    Off,
    On,
    /// The desktop decides (KDE's automatic policy, Hyprland's
    /// fullscreen-only value).
    Automatic,
}

impl AdaptiveSync {
    const ALL: [Self; 3] = [Self::Off, Self::On, Self::Automatic];
}

/// `off`, `on` or `automatic`.
impl fmt::Display for AdaptiveSync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Automatic => "automatic",
        })
    }
}

/// The names [`Display`](fmt::Display) writes, and nothing else.
impl FromStr for AdaptiveSync {
    type Err = ParseNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_name("adaptive sync setting", &Self::ALL, text)
    }
}

/// KDE's RGB range setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbRange {
    Automatic,
    Full,
    Limited,
}

/// `automatic`, `full` or `limited`.
impl fmt::Display for RgbRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Automatic => "automatic",
            Self::Full => "full",
            Self::Limited => "limited",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AdaptiveSync, OutputState};
    use crate::{Mode, Transform};

    fn state(transform: Transform, scale: f64) -> OutputState {
        OutputState {
            mode: Some(Mode::new(2560, 1440, 60_000)),
            transform,
            scale,
            ..OutputState::default()
        }
    }

    #[test]
    fn the_logical_size_is_the_mode_turned_and_scaled() {
        assert_eq!(
            state(Transform::Normal, 1.0).logical_size(),
            Some((2560, 1440))
        );
        assert_eq!(
            state(Transform::Rotate90, 1.0).logical_size(),
            Some((1440, 2560))
        );
        assert_eq!(
            state(Transform::Flipped270, 2.0).logical_size(),
            Some((720, 1280))
        );
        assert_eq!(
            state(Transform::Rotate180, 1.25).logical_size(),
            Some((2048, 1152))
        );
    }

    #[test]
    fn a_fractional_logical_size_rounds_to_the_nearest_pixel() {
        // 2560 / 1.75 = 1462.86, 1440 / 1.75 = 822.86.
        assert_eq!(
            state(Transform::Normal, 1.75).logical_size(),
            Some((1463, 823))
        );
    }

    #[test]
    fn no_mode_or_no_usable_scale_has_no_logical_size() {
        assert_eq!(OutputState::default().logical_size(), None);
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-12] {
            assert_eq!(
                state(Transform::Normal, scale).logical_size(),
                None,
                "scale {scale}"
            );
        }
    }

    #[test]
    fn the_default_state_is_an_enabled_untouched_output() {
        let state = OutputState::default();
        assert!(state.enabled);
        assert_eq!((state.x, state.y), (0, 0));
        assert!((state.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Normal);
        assert_eq!(state.adaptive_sync, None);
        assert!(state.mode.is_none() && state.mirror_of.is_none());
    }

    #[test]
    fn adaptive_sync_reads_back_its_own_names_only() {
        for sync in AdaptiveSync::ALL {
            assert_eq!(sync.to_string().parse(), Ok(sync));
        }
        for text in ["", "On", "true", "1", "auto"] {
            let error = text.parse::<AdaptiveSync>().unwrap_err().to_string();
            assert!(error.contains("off, on, automatic"), "{text:?}: {error}");
        }
    }
}
