//! Comparing the layout read back after an apply with the one asked for
//! (`docs/plan/02-architecture.md`, step 6). KDE and Hyprland's `eval` path
//! have no serial, so a change made by something else between the snapshot
//! and the apply, or a value the desktop rounded, is only caught here; each
//! difference is shown, never silently accepted.

use std::fmt;

use crate::layout::SCALE_EPSILON;
use crate::{Capabilities, Layout, OutputEntry, OutputId, OutputState};

/// One way the layout read back differs from the one asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// An output that was asked for is not in the layout read back.
    Missing(OutputId),
    /// The layout read back has an output that was not asked for.
    Added(OutputId),
    /// A setting came back other than it was asked for, each written as
    /// `dsplr list` writes it.
    Setting {
        output: OutputId,
        setting: Setting,
        asked: String,
        got: String,
    },
}

/// The setting a [`Difference::Setting`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    Enabled,
    Mode,
    Position,
    Scale,
    Transform,
    AdaptiveSync,
    MirrorOf,
    Primary,
    ColourPreset,
    BitDepth,
    Overscan,
    RgbRange,
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Enabled => "enabled",
            Self::Mode => "mode",
            Self::Position => "position",
            Self::Scale => "scale",
            Self::Transform => "transform",
            Self::AdaptiveSync => "adaptive sync",
            Self::MirrorOf => "mirror",
            Self::Primary => "primary",
            Self::ColourPreset => "colour preset",
            Self::BitDepth => "bit depth",
            Self::Overscan => "overscan",
            Self::RgbRange => "RGB range",
        })
    }
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(output) => write!(f, "{output}: asked for, but not there"),
            Self::Added(output) => write!(f, "{output}: there, but not asked for"),
            Self::Setting {
                output,
                setting,
                asked,
                got,
            } => write!(f, "{output}: {setting} asked {asked}, got {got}"),
        }
    }
}

impl Layout {
    /// How `actual`, read back from the desktop after applying `self`,
    /// differs from it. Entries are paired by identity; both layouts come
    /// from the same backend, so its identities are the same.
    ///
    /// A setting is compared only where both sides know it: a mode that was
    /// not asked for, an adaptive sync or optional setting either side has
    /// as `None`, and `primary` on a backend without
    /// [`Capabilities::primary`] are skipped. A disabled output has nothing
    /// to compare but that. Modes match within the refresh tolerance. The
    /// scale asked for is first put through the roundings the backend's wire
    /// makes ([`Capabilities::carried_scale`]: a `wl_fixed`, KWin's 120ths),
    /// so those are not differences, and then compared within float noise,
    /// so a scale the desktop itself chose otherwise (Hyprland's clean
    /// scales) shows up.
    pub fn differences(&self, actual: &Self, capabilities: &Capabilities) -> Vec<Difference> {
        let mut differences = Vec::new();
        for asked in &self.entries {
            match actual.entries.iter().find(|got| got.id == asked.id) {
                None => differences.push(Difference::Missing(asked.id.clone())),
                Some(got) => compare(asked, got, capabilities, &mut differences),
            }
        }
        for got in &actual.entries {
            if !self.entries.iter().any(|asked| asked.id == got.id) {
                differences.push(Difference::Added(got.id.clone()));
            }
        }
        differences
    }
}

/// Pushes a [`Difference::Setting`] for each setting of one output pair that
/// differs.
fn compare(
    asked: &OutputEntry,
    got: &OutputEntry,
    capabilities: &Capabilities,
    differences: &mut Vec<Difference>,
) {
    let mut differ = |setting: Setting, asked_value: String, got_value: String| {
        differences.push(Difference::Setting {
            output: asked.id.clone(),
            setting,
            asked: asked_value,
            got: got_value,
        });
    };
    let (a, g) = (&asked.state, &got.state);
    if a.enabled != g.enabled {
        differ(
            Setting::Enabled,
            a.enabled.to_string(),
            g.enabled.to_string(),
        );
        return;
    }
    if !a.enabled {
        return;
    }
    if let Some(mode) = &a.mode
        && !g.mode.as_ref().is_some_and(|got| got.matches(mode))
    {
        differ(
            Setting::Mode,
            mode.to_string(),
            g.mode
                .as_ref()
                .map_or_else(|| "none".to_owned(), ToString::to_string),
        );
    }
    if (a.x, a.y) != (g.x, g.y) {
        differ(
            Setting::Position,
            format!("{},{}", a.x, a.y),
            format!("{},{}", g.x, g.y),
        );
    }
    if (capabilities.carried_scale(a.scale) - g.scale).abs() >= SCALE_EPSILON {
        differ(Setting::Scale, a.scale.to_string(), g.scale.to_string());
    }
    if a.transform != g.transform {
        differ(
            Setting::Transform,
            a.transform.to_string(),
            g.transform.to_string(),
        );
    }
    if let (Some(asked_sync), Some(got_sync)) = (a.adaptive_sync, g.adaptive_sync)
        && asked_sync != got_sync
    {
        differ(
            Setting::AdaptiveSync,
            asked_sync.to_string(),
            got_sync.to_string(),
        );
    }
    if a.mirror_of != g.mirror_of {
        let name = |source: &Option<OutputId>| {
            source
                .as_ref()
                .map_or_else(|| "none".to_owned(), ToString::to_string)
        };
        differ(Setting::MirrorOf, name(&a.mirror_of), name(&g.mirror_of));
    }
    if capabilities.primary && a.primary != g.primary {
        differ(
            Setting::Primary,
            a.primary.to_string(),
            g.primary.to_string(),
        );
    }
    compare_known(a, g, &mut differ);
}

/// The optional settings, compared only where both sides have a value: a
/// backend that does not read one reports `None`, which says nothing.
fn compare_known(
    a: &OutputState,
    g: &OutputState,
    differ: &mut impl FnMut(Setting, String, String),
) {
    if let (Some(asked), Some(got)) = (&a.colour_preset, &g.colour_preset)
        && asked != got
    {
        differ(Setting::ColourPreset, asked.clone(), got.clone());
    }
    if let (Some(asked), Some(got)) = (a.bit_depth, g.bit_depth)
        && asked != got
    {
        differ(Setting::BitDepth, asked.to_string(), got.to_string());
    }
    if let (Some(asked), Some(got)) = (a.overscan, g.overscan)
        && asked != got
    {
        differ(Setting::Overscan, format!("{asked}%"), format!("{got}%"));
    }
    if let (Some(asked), Some(got)) = (a.rgb_range, g.rgb_range)
        && asked != got
    {
        differ(Setting::RgbRange, asked.to_string(), got.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{Difference, Setting};
    use crate::{
        AdaptiveSync, Capabilities, Layout, Mode, OutputEntry, OutputId, OutputState, Transform,
    };

    fn entry(name: &str, state: OutputState) -> OutputEntry {
        OutputEntry {
            id: OutputId::Connector(name.into()),
            output: None,
            state,
        }
    }

    fn enabled() -> OutputState {
        OutputState {
            mode: Some(Mode::new(2560, 1440, 120_000)),
            ..OutputState::default()
        }
    }

    fn one(state: OutputState) -> Layout {
        Layout {
            entries: vec![entry("DP-1", state)],
            ..Default::default()
        }
    }

    fn setting(setting: Setting, asked: &str, got: &str) -> Difference {
        Difference::Setting {
            output: OutputId::Connector("DP-1".into()),
            setting,
            asked: asked.into(),
            got: got.into(),
        }
    }

    #[test]
    fn a_layout_that_came_back_as_asked_has_no_differences() {
        let layout = one(enabled());
        assert_eq!(layout.differences(&layout, &Capabilities::default()), []);
    }

    #[test]
    fn each_changed_setting_is_one_difference_in_list_terms() {
        let asked = one(OutputState {
            x: 1920,
            scale: 1.3,
            transform: Transform::Rotate90,
            adaptive_sync: Some(AdaptiveSync::Automatic),
            ..enabled()
        });
        let got = one(OutputState {
            mode: Some(Mode::new(2560, 1440, 59_951)),
            scale: 1.300_781_25,
            adaptive_sync: Some(AdaptiveSync::Off),
            mirror_of: Some(OutputId::Connector("eDP-1".into())),
            ..enabled()
        });
        let differences = asked.differences(&got, &Capabilities::default());
        assert_eq!(
            differences,
            [
                setting(Setting::Mode, "2560x1440@120", "2560x1440@59.951"),
                setting(Setting::Position, "1920,0", "0,0"),
                setting(Setting::Scale, "1.3", "1.30078125"),
                setting(Setting::Transform, "90", "normal"),
                setting(Setting::AdaptiveSync, "automatic", "off"),
                setting(Setting::MirrorOf, "none", "eDP-1"),
            ]
        );
        assert_eq!(
            differences[2].to_string(),
            "DP-1: scale asked 1.3, got 1.30078125"
        );
    }

    #[test]
    fn a_scale_the_wire_rounds_is_not_a_difference_but_another_is() {
        let asked = one(OutputState {
            scale: 1.3,
            ..enabled()
        });
        let got = |scale| one(OutputState { scale, ..enabled() });
        let wl_fixed = Capabilities {
            scale_steps: vec![256],
            ..Capabilities::default()
        };
        assert_eq!(asked.differences(&got(333.0 / 256.0), &wl_fixed), []);
        let other = asked.differences(&got(1.5), &wl_fixed);
        assert_eq!(other, [setting(Setting::Scale, "1.3", "1.5")]);
        // KWin's 120ths: 1.004 arrives as 1.
        let kwin = Capabilities {
            scale_steps: vec![256, 120, 256],
            ..Capabilities::default()
        };
        let asked = one(OutputState {
            scale: 1.004,
            ..enabled()
        });
        assert_eq!(asked.differences(&got(1.0), &kwin), []);
        assert_eq!(asked.differences(&got(1.0), &wl_fixed).len(), 1);
    }

    #[test]
    fn what_either_side_does_not_know_is_not_a_difference() {
        let asked = one(OutputState {
            adaptive_sync: Some(AdaptiveSync::On),
            colour_preset: Some("hdr".into()),
            primary: true,
            ..enabled()
        });
        let got = one(OutputState {
            mode: Some(Mode::new(2560, 1440, 120_004)),
            adaptive_sync: None,
            colour_preset: None,
            primary: false,
            ..enabled()
        });
        assert_eq!(asked.differences(&got, &Capabilities::default()), []);
        let with_primary = Capabilities {
            primary: true,
            ..Capabilities::default()
        };
        assert_eq!(
            asked.differences(&got, &with_primary),
            [setting(Setting::Primary, "true", "false")]
        );
    }

    #[test]
    fn a_disabled_output_compares_only_whether_it_is_enabled() {
        let asked = one(OutputState {
            enabled: false,
            ..OutputState::default()
        });
        let got_off = one(OutputState {
            enabled: false,
            x: 500,
            ..enabled()
        });
        assert_eq!(asked.differences(&got_off, &Capabilities::default()), []);
        assert_eq!(
            asked.differences(&one(enabled()), &Capabilities::default()),
            [setting(Setting::Enabled, "false", "true")]
        );
    }

    #[test]
    fn outputs_that_came_or_went_are_named() {
        let asked = Layout {
            entries: vec![entry("DP-1", enabled()), entry("DP-2", enabled())],
            ..Default::default()
        };
        let got = Layout {
            entries: vec![entry("DP-1", enabled()), entry("HDMI-A-1", enabled())],
            ..Default::default()
        };
        let differences = asked.differences(&got, &Capabilities::default());
        assert_eq!(
            differences,
            [
                Difference::Missing(OutputId::Connector("DP-2".into())),
                Difference::Added(OutputId::Connector("HDMI-A-1".into())),
            ]
        );
        assert_eq!(differences[0].to_string(), "DP-2: asked for, but not there");
    }
}
