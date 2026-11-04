use crate::{Mode, OutputId, Transform};

/// Every monitor and its settings: the one struct the GUI edits, the CLI
/// prints, and every backend and config format converts to and from.
///
/// Apply, test, persist and profiles take a whole `Layout`, since every
/// desktop applies a configuration atomically. Fields will be added (the
/// fallback rule for unlisted monitors, GNOME's layout mode), so build one
/// with `..Default::default()`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub entries: Vec<OutputEntry>,
}

/// One monitor: its hardware facts, when known, and its settings.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputEntry {
    pub id: OutputId,
    /// `None` for a layout read back from a config file, which knows a
    /// monitor's name and settings but not its modes; the GUI fills it from
    /// the live snapshot by `id`.
    pub output: Option<Output>,
    pub state: OutputState,
}

/// Hardware facts about a connected monitor.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    pub adaptive_sync: AdaptiveSync,
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
            adaptive_sync: AdaptiveSync::Off,
            mirror_of: None,
            primary: false,
            colour_preset: None,
            bit_depth: None,
            overscan: None,
            rgb_range: None,
        }
    }
}

/// Variable refresh rate, one form for every desktop: wlroots' bool, KDE's
/// policy and Hyprland's integer `vrr` each map onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdaptiveSync {
    #[default]
    Off,
    On,
    /// The desktop decides (KDE's automatic policy, Hyprland's
    /// fullscreen-only value).
    Automatic,
}

/// KDE's RGB range setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbRange {
    Automatic,
    Full,
    Limited,
}

#[cfg(test)]
mod tests {
    use super::{AdaptiveSync, OutputState};
    use crate::Transform;

    #[test]
    fn the_default_state_is_an_enabled_untouched_output() {
        let state = OutputState::default();
        assert!(state.enabled);
        assert_eq!((state.x, state.y), (0, 0));
        assert!((state.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Normal);
        assert_eq!(state.adaptive_sync, AdaptiveSync::Off);
        assert!(state.mode.is_none() && state.mirror_of.is_none());
    }
}
