use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// One display mode an output offers, or the mode an output is set to.
///
/// A setting whose mode is not among its output's listed modes is a custom
/// mode; a mode read back from a config file has only its size and refresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    /// Millihertz, the unit `wlr-output-management` and KDE use. Backends
    /// reporting hertz convert with [`hz_to_mhz`].
    pub refresh_mhz: u32,
    #[serde(default)]
    pub preferred: bool,
    /// Scales valid with this mode, where the backend lists them per mode
    /// (GNOME). Empty when it does not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scales: Vec<f64>,
    /// The backend's own key for this exact mode, so it is passed back
    /// rather than matched again by number. Never serialised: it means
    /// nothing outside the session that read it.
    #[serde(skip)]
    pub handle: Option<ModeHandle>,
}

impl Mode {
    /// A mode with only its size and refresh known: not preferred, no
    /// per-mode scales, no backend handle.
    pub const fn new(width: u32, height: u32, refresh_mhz: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz,
            preferred: false,
            scales: Vec::new(),
            handle: None,
        }
    }

    /// Whether `other` names the same mode: the same size, and a refresh
    /// within [`REFRESH_TOLERANCE_MHZ`], since config formats round it.
    pub const fn matches(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.refresh_mhz.abs_diff(other.refresh_mhz) <= REFRESH_TOLERANCE_MHZ
    }
}

/// `1920x1080@59.94`: the size, and the refresh in hertz as
/// [`mhz_to_hz_text`] writes it. [`ModeSpec`] reads it back.
impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{}@{}",
            self.width,
            self.height,
            mhz_to_hz_text(self.refresh_mhz)
        )
    }
}

/// A mode as text names it, `WIDTHxHEIGHT` or `WIDTHxHEIGHT@HZ`: a size, and
/// the refresh when the text gives one. Each format's reader decides whether
/// it needs the refresh, and what else (a unit) goes around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSpec {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: Option<u32>,
}

impl ModeSpec {
    /// The mode it names, when it gives a refresh.
    pub const fn to_mode(self) -> Option<Mode> {
        match self.refresh_mhz {
            Some(refresh_mhz) => Some(Mode::new(self.width, self.height, refresh_mhz)),
            None => None,
        }
    }
}

/// Text that is not `WIDTHxHEIGHT[@HZ]`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a mode: expected WIDTHxHEIGHT or WIDTHxHEIGHT@HZ, such as 2560x1440@143.998")]
pub struct ParseModeError(String);

/// Width and height are digits and above zero; the refresh is read by
/// [`hz_text_to_mhz`]. Nothing else is taken: no sign, no spaces, no unit.
impl FromStr for ModeSpec {
    type Err = ParseModeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseModeError(text.to_owned());
        let (size, refresh) = match text.split_once('@') {
            Some((size, refresh)) => (size, Some(refresh)),
            None => (text, None),
        };
        let (width, height) = size.split_once('x').ok_or_else(invalid)?;
        let pixels = |text: &str| {
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            text.parse::<u32>().ok().filter(|&pixels| pixels > 0)
        };
        Ok(Self {
            width: pixels(width).ok_or_else(invalid)?,
            height: pixels(height).ok_or_else(invalid)?,
            refresh_mhz: refresh
                .map(|refresh| hz_text_to_mhz(refresh).ok_or_else(invalid))
                .transpose()?,
        })
    }
}

/// How far apart two refresh rates may be and still name the same mode.
/// Hyprland writes refresh in hertz with two decimals, a step of 10 mHz, so
/// its rounding moves a rate by at most half that.
pub const REFRESH_TOLERANCE_MHZ: u32 = 5;

/// A backend's key for one of its modes: an index into its own mode table, or
/// an id such as a RandR mode XID. Only the backend that made it reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeHandle(pub u64);

/// A refresh rate in millihertz written in hertz, exact to the millihertz
/// with trailing zeros dropped: `60`, `59.94`, `143.998`. The form people
/// read and Hyprland's mode strings take.
pub fn mhz_to_hz_text(mhz: u32) -> String {
    let (hz, fraction) = (mhz / 1000, mhz % 1000);
    if fraction == 0 {
        hz.to_string()
    } else {
        format!("{hz}.{}", format!("{fraction:03}").trim_end_matches('0'))
    }
}

/// Reads a refresh rate written in hertz, as [`mhz_to_hz_text`] writes it
/// and people type it: digits with at most one decimal point (`60`, `59.94`,
/// `143.998`), rounded to the nearest millihertz. `None` for anything else
/// (a sign, an exponent, `inf`, a unit) or a rate that does not fit.
pub fn hz_text_to_mhz(text: &str) -> Option<u32> {
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let points = text.chars().filter(|&c| c == '.').count();
    if digits == 0 || points > 1 || digits + points != text.chars().count() {
        return None;
    }
    hz_to_mhz(text.parse().ok()?)
}

/// Converts a refresh rate in hertz, as Hyprland and GNOME report it, to
/// millihertz, rounding to the nearest. `None` for a value that is not finite,
/// is negative, or does not fit.
pub fn hz_to_mhz(hz: f64) -> Option<u32> {
    let mhz = (hz * 1000.0).round();
    if (0.0..=f64::from(u32::MAX)).contains(&mhz) {
        // Integral and inside u32's range, checked above, so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(mhz as u32)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, ModeSpec, REFRESH_TOLERANCE_MHZ, hz_text_to_mhz, hz_to_mhz, mhz_to_hz_text};

    fn spec(width: u32, height: u32, refresh_mhz: Option<u32>) -> ModeSpec {
        ModeSpec {
            width,
            height,
            refresh_mhz,
        }
    }

    #[test]
    fn a_mode_reads_as_its_size_and_hertz_and_parses_back() {
        for mode in [
            Mode::new(1920, 1080, 59_940),
            Mode::new(2560, 1440, 143_998),
            Mode::new(800, 600, 0),
            Mode::new(u32::MAX, 1, 60_001),
        ] {
            let text = mode.to_string();
            assert_eq!(
                text.parse::<ModeSpec>().map(ModeSpec::to_mode),
                Ok(Some(mode))
            );
        }
        assert_eq!(Mode::new(1920, 1080, 59_940).to_string(), "1920x1080@59.94");
    }

    #[test]
    fn mode_text_parses_with_and_without_a_refresh() {
        assert_eq!(
            "2560x1440@143.998".parse(),
            Ok(spec(2560, 1440, Some(143_998)))
        );
        assert_eq!("1920x1080@60".parse(), Ok(spec(1920, 1080, Some(60_000))));
        assert_eq!("1920x1080".parse(), Ok(spec(1920, 1080, None)));
        assert_eq!(spec(1920, 1080, None).to_mode(), None);
    }

    #[test]
    fn malformed_mode_text_is_refused() {
        for text in [
            "",
            "1920",
            "1920x",
            "x1080",
            "1920x1080@",
            "1920x1080@60Hz",
            "1920x1080@-60",
            "1920x1080@1e2",
            "1920x1080@inf",
            "1920*1080@60",
            "0x1080@60",
            "1920x0",
            "+1920x1080",
            "-1920x1080",
            " 1920x1080",
            "4294967296x1080",
            "1920x1080@5000000",
            "1920x1080@60@60",
        ] {
            assert!(text.parse::<ModeSpec>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn hertz_text_drops_trailing_zeros_but_keeps_every_millihertz() {
        assert_eq!(mhz_to_hz_text(60_000), "60");
        assert_eq!(mhz_to_hz_text(59_940), "59.94");
        assert_eq!(mhz_to_hz_text(50_500), "50.5");
        assert_eq!(mhz_to_hz_text(143_998), "143.998");
        assert_eq!(mhz_to_hz_text(60_001), "60.001");
        assert_eq!(mhz_to_hz_text(0), "0");
    }

    #[test]
    fn modes_match_by_size_and_refresh_within_the_tolerance() {
        let mode = Mode::new(1920, 1080, 59_940);
        let at = |mhz| Mode::new(1920, 1080, mhz);
        assert!(mode.matches(&at(59_940 + REFRESH_TOLERANCE_MHZ)));
        assert!(mode.matches(&at(59_940 - REFRESH_TOLERANCE_MHZ)));
        assert!(!mode.matches(&at(59_940 + REFRESH_TOLERANCE_MHZ + 1)));
        assert!(!mode.matches(&Mode::new(1920, 1200, 59_940)));
        assert!(!mode.matches(&Mode::new(1680, 1080, 59_940)));
    }

    #[test]
    fn hertz_convert_to_the_nearest_millihertz() {
        assert_eq!(hz_to_mhz(60.0), Some(60_000));
        assert_eq!(hz_to_mhz(59.951), Some(59_951));
        assert_eq!(hz_to_mhz(143.9996), Some(144_000));
        assert_eq!(hz_to_mhz(0.0), Some(0));
    }

    #[test]
    fn hertz_that_cannot_be_a_refresh_are_refused() {
        assert_eq!(hz_to_mhz(f64::NAN), None);
        assert_eq!(hz_to_mhz(f64::INFINITY), None);
        assert_eq!(hz_to_mhz(-60.0), None);
        assert_eq!(hz_to_mhz(5e6), None);
    }

    #[test]
    fn hertz_text_reads_back_what_it_wrote_and_what_people_type() {
        for mhz in [60_000, 59_940, 50_500, 143_998, 60_001, 0] {
            assert_eq!(hz_text_to_mhz(&mhz_to_hz_text(mhz)), Some(mhz), "{mhz}");
        }
        assert_eq!(hz_text_to_mhz("60.00"), Some(60_000));
        assert_eq!(hz_text_to_mhz("59.9401"), Some(59_940), "to the nearest");
        assert_eq!(hz_text_to_mhz("60."), Some(60_000));
        assert_eq!(hz_text_to_mhz(".5"), Some(500));
    }

    #[test]
    fn hertz_text_that_is_not_plain_digits_is_refused() {
        for text in [
            "", ".", "-60", "+60", "1e3", "inf", "NaN", "60Hz", " 60", "6 0", "60..0", "6.0.0",
            "5000000",
        ] {
            assert_eq!(hz_text_to_mhz(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_bare_mode_carries_nothing_but_size_and_refresh() {
        let mode = Mode::new(2560, 1440, 143_998);
        assert_eq!(
            (mode.width, mode.height, mode.refresh_mhz),
            (2560, 1440, 143_998)
        );
        assert!(!mode.preferred);
        assert!(mode.scales.is_empty());
        assert!(mode.handle.is_none());
    }
}
