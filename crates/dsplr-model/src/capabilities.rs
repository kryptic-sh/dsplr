use crate::AdaptiveSync;

/// What the active backend can do. The GUI greys out what is absent; the
/// conversions for desktops without a setting ignore it.
///
/// The default is a backend that can do nothing optional, so a backend
/// states each capability it has rather than each it lacks. Fields will be
/// added, so build one with `..Default::default()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Modes the output does not list are accepted.
    pub custom_modes: bool,
    /// An output can mirror another.
    pub mirroring: bool,
    /// The `AdaptiveSync` values the backend accepts.
    pub adaptive_sync: Vec<AdaptiveSync>,
    /// Scales other than whole numbers.
    pub fractional_scale: bool,
    /// Outputs can have different scales (not on X11, not when GNOME reports
    /// `global-scale-required`).
    pub per_output_scale: bool,
    /// The roundings a scale goes through between being asked for and being
    /// read back, in order: each is a denominator `n`, and the scale is
    /// rounded to the nearest multiple of `1/n`, halves away from zero (a
    /// `wl_fixed` is 256; KWin keeps scales in 120ths). Empty where a scale
    /// arrives as asked. [`Capabilities::carried_scale`] applies them.
    pub scale_steps: Vec<u32>,
    /// The backend can really test a layout before applying it (Hyprland's
    /// test is a stub that always succeeds, so it has not).
    pub test_before_apply: bool,
    /// Enabled outputs may have gaps between them (not on KDE).
    pub gaps_allowed: bool,
    /// The backend has a native place to persist a layout.
    pub native_persistence: bool,
    /// The desktop saves every applied layout itself.
    pub saves_on_apply: bool,
    /// The backend reads and sets `OutputState::primary`.
    pub primary: bool,
    /// The backend reads and sets `OutputState::colour_preset`.
    pub colour_preset: bool,
    /// The backend reads and sets `OutputState::bit_depth`.
    pub bit_depth: bool,
    /// The backend reads and sets `OutputState::overscan`.
    pub overscan: bool,
    /// The backend reads and sets `OutputState::rgb_range`.
    pub rgb_range: bool,
}

impl Capabilities {
    /// `scale` as the backend carries it to the desktop and back
    /// ([`Capabilities::scale_steps`]): what reading it back gives when the
    /// desktop took it as asked.
    #[must_use]
    pub fn carried_scale(&self, scale: f64) -> f64 {
        self.scale_steps.iter().fold(scale, |scale, &step| {
            let step = f64::from(step);
            (scale * step).round() / step
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Capabilities;

    fn steps(scale_steps: &[u32]) -> Capabilities {
        Capabilities {
            scale_steps: scale_steps.to_vec(),
            ..Capabilities::default()
        }
    }

    #[test]
    fn a_scale_is_carried_through_each_rounding_in_turn() {
        // No rounding: as asked.
        assert!((steps(&[]).carried_scale(1.1) - 1.1).abs() < f64::EPSILON);
        // A wl_fixed: 1.1 * 256 = 281.6, sent as 282.
        let wlr = steps(&[256]);
        assert!((wlr.carried_scale(1.1) - 282.0 / 256.0).abs() < f64::EPSILON);
        assert!((wlr.carried_scale(1.5) - 1.5).abs() < f64::EPSILON);
        // KWin: a wl_fixed in (1.0039…), 120ths kept (exactly 1), a wl_fixed
        // out.
        let kwin = steps(&[256, 120, 256]);
        assert!((kwin.carried_scale(1.004) - 1.0).abs() < f64::EPSILON);
        // 1.1: 282/256 in, 132/120 = 1.1 kept, 282/256 out.
        assert!((kwin.carried_scale(1.1) - 282.0 / 256.0).abs() < f64::EPSILON);
        // A half rounds away from zero, as libwayland's
        // `wl_fixed_from_double` (`round`) does: 1.5/256 above 1.
        assert!((wlr.carried_scale(1.0 + 1.5 / 256.0) - 258.0 / 256.0).abs() < f64::EPSILON);
    }
}
