/// One display mode an output offers, or the mode an output is set to.
///
/// A setting whose mode is not among its output's listed modes is a custom
/// mode; a mode read back from a config file has only its size and refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    /// Millihertz, the unit `wlr-output-management` and KDE use. Backends
    /// reporting hertz convert with [`hz_to_mhz`].
    pub refresh_mhz: u32,
    pub preferred: bool,
    /// Scales valid with this mode, where the backend lists them per mode
    /// (GNOME). Empty when it does not.
    pub scales: Vec<f64>,
    /// The backend's own key for this exact mode, so it is passed back
    /// rather than matched again by number.
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
}

/// A backend's key for one of its modes: an index into its own mode table, or
/// an id such as a RandR mode XID. Only the backend that made it reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeHandle(pub u64);

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
    use super::{Mode, hz_to_mhz};

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
