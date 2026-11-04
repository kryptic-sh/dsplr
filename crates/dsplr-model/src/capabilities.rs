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
