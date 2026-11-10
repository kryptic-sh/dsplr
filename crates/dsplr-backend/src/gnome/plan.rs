//! A [`Layout`] as `ApplyMonitorsConfig`'s logical monitors and properties,
//! decided before anything is sent, and what Mutter would refuse that dsplr
//! can see first (Mutter 51.1's `src/backends/meta-monitor-manager.c` and
//! `meta-monitor-config-utils.c`).

use std::collections::HashMap;

use dsplr_model::{Layout, LayoutMode, Mode, ModeHandle, OutputEntry, OutputId, SCALE_EPSILON};
use zbus::OwnedVariant;

use super::bus::{LogicalMonitorConfig, MonitorConfig};
use super::state::{
    LAYOUT_MODE_LOGICAL, LAYOUT_MODE_PHYSICAL, Monitor, MonitorMode, State, rgb_range_value,
};

/// `ApplyMonitorsConfig`'s method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Check the configuration, change nothing.
    Verify = 0,
    /// Apply it, not saved and never reverted by Mutter.
    Temporary = 1,
    /// Apply it, then ask (GNOME Shell's dialog) whether to keep it, and
    /// save it to `monitors.xml` only when kept.
    Persistent = 2,
}

/// What `ApplyMonitorsConfig` is given, but for the serial and the method.
#[derive(Debug, PartialEq)]
pub struct Config {
    pub logical_monitors: Vec<LogicalMonitorConfig>,
    /// The `layout-mode` property: set wherever Mutter can change the mode,
    /// since one left out is Mutter's default, not the current mode.
    pub layout_mode: Option<u32>,
}

/// Why a layout cannot be sent to Mutter. Nothing is sent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("{0}: no monitor is on its connector")]
    NotConnected(OutputId),
    #[error("{0} is in the layout more than once")]
    Twice(String),
    #[error("{0}: enabled without a mode")]
    NoMode(String),
    #[error("{connector}: Mutter does not list mode {mode} for it, and takes no other")]
    ModeNotListed { connector: String, mode: String },
    #[error("{connector}: Mutter takes scale {scale} with mode {mode} only as one of {listed}")]
    ScaleNotForMode {
        connector: String,
        scale: String,
        mode: String,
        listed: String,
    },
    #[error("{0}: it mirrors an output that is not on, or is a mirror itself, or is not there")]
    MirrorSource(String),
    #[error("{connector}: a mirror's mode must be as big as its source's ({mirrored})")]
    MirrorModeSize { connector: String, mirrored: String },
    #[error("{0}: a second primary output; Mutter has one")]
    SecondPrimary(String),
    #[error(
        "Mutter needs the outputs to start at 0,0 (the leftmost at x 0, the topmost at y 0), and this layout starts at {x},{y}"
    )]
    Offset { x: i32, y: i32 },
    #[error(
        "the layout is in the {asked} layout mode, Mutter is in the {current} one, and cannot change it"
    )]
    LayoutMode {
        asked: LayoutMode,
        current: LayoutMode,
    },
    #[error("{connector}: {what} cannot be set through Mutter's DisplayConfig")]
    Unsupported {
        connector: String,
        what: &'static str,
    },
}

/// The configuration for `layout` on Mutter as `state` describes it, where
/// `current` is that state as a layout ([`State::layout`]).
///
/// Entries name monitors by connector. A monitor the layout does not name
/// keeps its current settings. Each output that is on and mirrors nothing is
/// a logical monitor, at its position with its scale and transform; each
/// mirror is put in its source's, whose position, scale and transform it
/// then has. The primary output's logical monitor is the primary one; with
/// none named among the outputs that are on, the one Mutter has as primary
/// now stays so while it is on, else the first, since Mutter requires one. A mode is the one the
/// monitor lists (Mutter takes no custom modes), and a scale is sent as the
/// mode lists it, so a scale read back as a double goes back exactly.
///
/// A layout that names no layout mode is in Mutter's current one. One that
/// names another is sent with it, and then its scales are not checked
/// here: Mutter lists each mode's scales for its current layout mode only
/// (`GetCurrentState` calculates them with `manager->layout_mode`), so the
/// list dsplr has is the wrong one, and Mutter checks the scale against the
/// new mode's itself.
/// Mutter resets a monitor's underscanning and colour mode when an apply
/// leaves them out, so they go back as they are, and its RGB range as the
/// layout says, else as it is.
pub fn plan(state: &State, current: &Layout, layout: &Layout) -> Result<Config, PlanError> {
    let entries = assign(state, current, layout)?;
    for (entry, monitor) in entries.iter().zip(&state.monitors) {
        refuse_unsupported(entry, monitor)?;
    }
    let asked_mode = layout.layout_mode.unwrap_or(state.layout_mode);
    let switching = asked_mode != state.layout_mode;
    let layout_mode = if state.supports_changing_layout_mode {
        Some(match asked_mode {
            LayoutMode::Logical => LAYOUT_MODE_LOGICAL,
            LayoutMode::Physical => LAYOUT_MODE_PHYSICAL,
        })
    } else if !switching {
        None
    } else {
        return Err(PlanError::LayoutMode {
            asked: asked_mode,
            current: state.layout_mode,
        });
    };

    // Each logical monitor: the index of its source, then its mirrors'.
    let mut groups: Vec<Vec<usize>> = (0..entries.len())
        .filter(|&index| entries[index].state.enabled && entries[index].state.mirror_of.is_none())
        .map(|index| vec![index])
        .collect();
    for (index, entry) in entries.iter().enumerate() {
        let Some(source) = entry
            .state
            .mirror_of
            .as_ref()
            .filter(|_| entry.state.enabled)
        else {
            continue;
        };
        let group = groups
            .iter_mut()
            .find(|group| entries[group[0]].id == *source)
            .ok_or_else(|| PlanError::MirrorSource(state.monitors[index].connector.clone()))?;
        group.push(index);
    }

    let primary = primary_group(state, &entries, &groups)?;
    let (min_x, min_y) = groups
        .iter()
        .map(|group| (entries[group[0]].state.x, entries[group[0]].state.y))
        .fold((i32::MAX, i32::MAX), |(x, y), (gx, gy)| {
            (x.min(gx), y.min(gy))
        });
    if !groups.is_empty() && (min_x, min_y) != (0, 0) {
        return Err(PlanError::Offset { x: min_x, y: min_y });
    }

    let logical_monitors = groups
        .iter()
        .enumerate()
        .map(|(at, group)| {
            let source = entries[group[0]];
            let monitor = &state.monitors[group[0]];
            let mode = listed_mode(monitor, source)?;
            let scale = if switching {
                source.state.scale
            } else {
                listed_scale(monitor, mode, source.state.scale)?
            };
            let mut monitors = vec![monitor_config(monitor, mode, source)];
            for &index in &group[1..] {
                let mirror = &state.monitors[index];
                let mirror_mode = listed_mode(mirror, entries[index])?;
                if (mirror_mode.width, mirror_mode.height) != (mode.width, mode.height) {
                    return Err(PlanError::MirrorModeSize {
                        connector: mirror.connector.clone(),
                        mirrored: monitor.connector.clone(),
                    });
                }
                monitors.push(monitor_config(mirror, mirror_mode, entries[index]));
            }
            Ok((
                source.state.x,
                source.state.y,
                scale,
                source.state.transform.to_wayland(),
                Some(at) == primary,
                monitors,
            ))
        })
        .collect::<Result<_, _>>()?;
    Ok(Config {
        logical_monitors,
        layout_mode,
    })
}

/// The entry for each of `state`'s monitors, in their order: the layout's
/// entry on its connector, else its current one.
fn assign<'a>(
    state: &State,
    current: &'a Layout,
    layout: &'a Layout,
) -> Result<Vec<&'a OutputEntry>, PlanError> {
    let mut named: Vec<Option<&OutputEntry>> = vec![None; state.monitors.len()];
    for entry in &layout.entries {
        let index = entry
            .connector()
            .and_then(|connector| {
                state
                    .monitors
                    .iter()
                    .position(|monitor| monitor.connector == connector)
            })
            .ok_or_else(|| PlanError::NotConnected(entry.id.clone()))?;
        if named[index].is_some() {
            return Err(PlanError::Twice(state.monitors[index].connector.clone()));
        }
        named[index] = Some(entry);
    }
    named
        .into_iter()
        .zip(&state.monitors)
        .map(|(entry, monitor)| {
            entry
                .or_else(|| {
                    current
                        .entries
                        .iter()
                        .find(|entry| entry.connector() == Some(monitor.connector.as_str()))
                })
                .ok_or_else(|| PlanError::NotConnected(monitor.id()))
        })
        .collect()
}

/// A setting Mutter's interface does not carry, refused rather than
/// dropped.
fn refuse_unsupported(entry: &OutputEntry, monitor: &Monitor) -> Result<(), PlanError> {
    let state = &entry.state;
    let refused = [
        (state.colour_preset.is_some(), "a colour preset"),
        (state.bit_depth.is_some(), "bit depth"),
        (state.overscan.is_some(), "overscan"),
        (
            state.adaptive_sync.is_some(),
            "adaptive sync (Mutter offers it as a mode of its own, which dsplr does not read yet)",
        ),
        (
            state.rgb_range.is_some() && monitor.rgb_range.is_none(),
            "RGB range (Mutter reports none for this monitor)",
        ),
    ];
    match refused.into_iter().find(|(set, _)| *set) {
        Some((_, what)) => Err(PlanError::Unsupported {
            connector: monitor.connector.clone(),
            what,
        }),
        None => Ok(()),
    }
}

/// Which of `groups` is the primary logical monitor.
fn primary_group(
    state: &State,
    entries: &[&OutputEntry],
    groups: &[Vec<usize>],
) -> Result<Option<usize>, PlanError> {
    let mut named = None;
    for (index, entry) in entries.iter().enumerate() {
        if !entry.state.primary {
            continue;
        }
        // An output turned off keeps its flag in a layout made from a
        // snapshot (`dsplr set … --disable`), and shows nothing: it names no
        // primary.
        if !entry.state.enabled {
            continue;
        }
        if named.is_some() {
            return Err(PlanError::SecondPrimary(
                state.monitors[index].connector.clone(),
            ));
        }
        named = Some(index);
    }
    let in_group = |index: usize| groups.iter().position(|group| group.contains(&index));
    if let Some(index) = named {
        return Ok(in_group(index));
    }
    let now = state
        .logical_monitors
        .iter()
        .find(|logical| logical.primary)
        .and_then(|logical| logical.connectors.first())
        .and_then(|connector| {
            state
                .monitors
                .iter()
                .position(|monitor| monitor.connector == *connector)
        })
        .and_then(in_group);
    Ok(now.or_else(|| (!groups.is_empty()).then_some(0)))
}

/// The mode of `monitor` that `entry`'s mode names.
///
/// A monitor may list one size and refresh more than once (an interlaced
/// mode, or a variable refresh rate one), so a mode read from Mutter carries
/// its ordinal among the monitor's modes ([`ModeHandle`]), and the mode at
/// that ordinal is taken while it still has that size and refresh. A mode
/// without one (from a file), or whose ordinal now names another, takes the
/// current mode when that matches, else the first that matches within the
/// refresh tolerance.
fn listed_mode<'a>(
    monitor: &'a Monitor,
    entry: &OutputEntry,
) -> Result<&'a MonitorMode, PlanError> {
    let mode = entry
        .state
        .mode
        .as_ref()
        .ok_or_else(|| PlanError::NoMode(monitor.connector.clone()))?;
    let as_model =
        |listed: &MonitorMode| Mode::new(listed.width, listed.height, listed.refresh_mhz);
    if let Some(ModeHandle(handle)) = mode.handle
        && let Some(listed) = usize::try_from(handle)
            .ok()
            .and_then(|handle| monitor.modes.get(handle))
        && (listed.width, listed.height, listed.refresh_mhz)
            == (mode.width, mode.height, mode.refresh_mhz)
    {
        return Ok(listed);
    }
    let matching: Vec<&MonitorMode> = monitor
        .modes
        .iter()
        .filter(|listed| as_model(listed).matches(mode))
        .collect();
    matching
        .iter()
        .find(|listed| listed.current)
        .or_else(|| matching.first())
        .copied()
        .ok_or_else(|| PlanError::ModeNotListed {
            connector: monitor.connector.clone(),
            mode: mode.to_string(),
        })
}

/// `scale` as `mode` lists it, within [`SCALE_EPSILON`]. Mutter compares
/// the scale it is sent with each it lists within a float's epsilon, so the
/// listed value is sent, not the one asked for.
fn listed_scale(monitor: &Monitor, mode: &MonitorMode, scale: f64) -> Result<f64, PlanError> {
    if mode.scales.is_empty() {
        return Ok(scale);
    }
    mode.scales
        .iter()
        .copied()
        .find(|listed| (listed - scale).abs() < SCALE_EPSILON)
        .ok_or_else(|| PlanError::ScaleNotForMode {
            connector: monitor.connector.clone(),
            scale: scale.to_string(),
            mode: Mode::new(mode.width, mode.height, mode.refresh_mhz).to_string(),
            listed: mode
                .scales
                .iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        })
}

/// One monitor of a logical monitor: its connector, its mode's id, and the
/// properties carried over or set ([`plan`]).
fn monitor_config(monitor: &Monitor, mode: &MonitorMode, entry: &OutputEntry) -> MonitorConfig {
    let mut properties: HashMap<String, OwnedVariant> = monitor
        .kept
        .iter()
        .filter_map(|(name, value)| Some(((*name).to_owned(), value.try_clone().ok()?)))
        .collect();
    let rgb_range = entry
        .state
        .rgb_range
        .map(rgb_range_value)
        .or(monitor.rgb_range);
    if let Some(value) = rgb_range {
        properties.insert("rgb-range".to_owned(), OwnedVariant::from(value));
    }
    (monitor.connector.clone(), mode.id.clone(), properties)
}

#[cfg(test)]
mod tests {
    use dsplr_model::{AdaptiveSync, Layout, LayoutMode, Mode, RgbRange, Transform};
    use zbus::OwnedVariant;

    use super::{LAYOUT_MODE_LOGICAL, PlanError, plan};
    use crate::gnome::state::State;
    use crate::gnome::state::tests::{properties, two_monitors};

    fn state() -> State {
        State::from_wire(two_monitors()).expect("reads")
    }

    fn planned(layout: &Layout) -> Result<super::Config, PlanError> {
        let state = state();
        let current = state.layout().expect("converts");
        plan(&state, &current, layout)
    }

    fn current() -> Layout {
        state().layout().expect("converts")
    }

    #[test]
    fn a_snapshot_sent_back_is_the_logical_monitors_mutter_has() {
        let config = planned(&current()).expect("plans");
        assert_eq!(config.layout_mode, Some(LAYOUT_MODE_LOGICAL));
        assert_eq!(config.logical_monitors.len(), 2);
        let (x, y, scale, transform, primary, monitors) = &config.logical_monitors[0];
        assert_eq!((*x, *y, *transform, *primary), (0, 0, 0, true));
        assert!((scale - 1.25).abs() < f64::EPSILON);
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].0, "DP-1");
        assert_eq!(monitors[0].1, "2560x1440@60.000");
        // Its colour mode goes back as it is, and its RGB range.
        assert_eq!(
            monitors[0].2,
            properties(&[
                ("color-mode", OwnedVariant::from(0_u32)),
                ("rgb-range", OwnedVariant::from(2_u32)),
            ])
        );
        let (x, _, _, transform, primary, monitors) = &config.logical_monitors[1];
        assert_eq!((*x, *transform, *primary), (2048, 1, false));
        // The current one of its two 1920x1080 modes, by its ordinal.
        assert_eq!(monitors[0].1, "1920x1080@60.000");
    }

    #[test]
    fn a_mode_from_a_file_is_matched_by_size_and_refresh() {
        let mut layout = current();
        let state = &mut layout.entries[0].state;
        state.mode = Some(Mode::new(1920, 1080, 60_000));
        state.scale = 1.0;
        layout.entries[1].state.x = 1536;
        let config = planned(&layout).expect("plans");
        assert_eq!(config.logical_monitors[0].5[0].1, "1920x1080@60.000");

        layout.entries[0].state.mode = Some(Mode::new(1280, 720, 60_000));
        assert_eq!(
            planned(&layout),
            Err(PlanError::ModeNotListed {
                connector: "DP-1".into(),
                mode: "1280x720@60".into()
            })
        );
    }

    #[test]
    fn a_scale_is_sent_as_the_mode_lists_it_and_refused_when_it_does_not() {
        let mut layout = current();
        layout.entries[0].state.scale = 1.25 + SCALE_NOISE;
        let config = planned(&layout).expect("plans");
        assert!((config.logical_monitors[0].2 - 1.25).abs() < f64::EPSILON);

        layout.entries[0].state.scale = 1.75;
        assert!(matches!(
            planned(&layout),
            Err(PlanError::ScaleNotForMode { connector, .. }) if connector == "DP-1"
        ));
    }

    /// Less than `SCALE_EPSILON`: a double that went through a float.
    const SCALE_NOISE: f64 = 1e-9;

    #[test]
    fn a_mirror_joins_its_source_s_logical_monitor() {
        let mut layout = current();
        let source = layout.entries[1].id.clone();
        let state = &mut layout.entries[0].state;
        state.mirror_of = Some(source);
        state.mode = Some(Mode::new(1920, 1080, 60_000));
        state.primary = false;
        layout.entries[1].state.x = 0;
        layout.entries[1].state.primary = true;
        let config = planned(&layout).expect("plans");
        assert_eq!(config.logical_monitors.len(), 1);
        let (_, _, _, _, primary, monitors) = &config.logical_monitors[0];
        assert!(*primary);
        let connectors: Vec<&str> = monitors.iter().map(|m| m.0.as_str()).collect();
        assert_eq!(connectors, ["HDMI-1", "DP-1"]);

        // Mutter needs every monitor of a logical monitor at one mode size.
        layout.entries[0].state.mode = Some(Mode::new(2560, 1440, 60_000));
        assert_eq!(
            planned(&layout),
            Err(PlanError::MirrorModeSize {
                connector: "DP-1".into(),
                mirrored: "HDMI-1".into()
            })
        );
    }

    #[test]
    fn an_output_turned_off_is_in_no_logical_monitor() {
        let mut layout = current();
        layout.entries[1].state.enabled = false;
        let config = planned(&layout).expect("plans");
        assert_eq!(config.logical_monitors.len(), 1);
        assert_eq!(config.logical_monitors[0].5[0].0, "DP-1");
    }

    #[test]
    fn mutter_needs_one_primary_so_one_is_kept_or_picked() {
        // None named: the one Mutter has now stays primary.
        let mut layout = current();
        layout.entries[0].state.primary = false;
        let config = planned(&layout).expect("plans");
        assert!(config.logical_monitors[0].4);

        // That one off: the first logical monitor left.
        layout.entries[0].state.enabled = false;
        layout.entries[1].state.x = 0;
        let config = planned(&layout).expect("plans");
        assert!(config.logical_monitors[0].4);

        let mut layout = current();
        layout.entries[1].state.primary = true;
        assert_eq!(
            planned(&layout),
            Err(PlanError::SecondPrimary("HDMI-1".into()))
        );
        // The primary output turned off with its flag still on, as
        // `dsplr set DP-1 --disable` leaves it: the one left takes it.
        let mut layout = current();
        layout.entries[0].state.enabled = false;
        layout.entries[1].state.x = 0;
        let config = planned(&layout).expect("plans");
        assert_eq!(config.logical_monitors.len(), 1);
        assert!(config.logical_monitors[0].4);
    }

    #[test]
    fn a_layout_away_from_the_origin_is_refused() {
        let mut layout = current();
        for entry in &mut layout.entries {
            entry.state.x += 10;
        }
        assert_eq!(planned(&layout), Err(PlanError::Offset { x: 10, y: 0 }));
    }

    #[test]
    fn what_mutter_cannot_carry_is_refused_and_an_unnamed_monitor_kept() {
        let mut layout = current();
        layout.entries[1].state.adaptive_sync = Some(AdaptiveSync::On);
        assert!(matches!(
            planned(&layout),
            Err(PlanError::Unsupported { connector, .. }) if connector == "HDMI-1"
        ));

        let mut layout = current();
        layout.entries[0].state.rgb_range = Some(RgbRange::Limited);
        let config = planned(&layout).expect("plans");
        assert_eq!(
            config.logical_monitors[0].5[0].2.get("rgb-range"),
            Some(&OwnedVariant::from(3_u32))
        );

        // HDMI-1 not named: it keeps its place and turn.
        let mut layout = current();
        layout.entries.truncate(1);
        let config = planned(&layout).expect("plans");
        assert_eq!(config.logical_monitors.len(), 2);
        assert_eq!(config.logical_monitors[1].0, 2048);
        assert_eq!(
            config.logical_monitors[1].3,
            Transform::Rotate90.to_wayland()
        );

        let mut layout = current();
        layout.entries.push(layout.entries[0].clone());
        assert_eq!(planned(&layout), Err(PlanError::Twice("DP-1".into())));
    }

    #[test]
    fn a_layout_naming_no_mode_is_in_mutter_s_and_a_switch_leaves_scales_to_mutter() {
        // A file from another desktop names no mode: Mutter's stays.
        let mut layout = current();
        layout.layout_mode = None;
        assert_eq!(
            planned(&layout).expect("plans").layout_mode,
            Some(LAYOUT_MODE_LOGICAL)
        );

        // In the logical mode 1.75 is not one of DP-1's listed scales, and is
        // refused; asked with a switch to the physical mode, the list dsplr
        // has is the logical mode's, so the scale goes as asked and Mutter
        // checks it against the physical mode's.
        layout.entries[0].state.scale = 1.75;
        assert!(matches!(
            planned(&layout),
            Err(PlanError::ScaleNotForMode { .. })
        ));
        layout.layout_mode = Some(LayoutMode::Physical);
        let config = planned(&layout).expect("plans");
        assert!((config.logical_monitors[0].2 - 1.75).abs() < f64::EPSILON);
    }

    #[test]
    fn the_layout_mode_is_sent_where_mutter_can_change_it_and_must_match_elsewhere() {
        let mut layout = current();
        layout.layout_mode = Some(LayoutMode::Physical);
        layout.entries[0].state.scale = 1.0;
        layout.entries[1].state.x = 2560;
        assert_eq!(
            planned(&layout).expect("plans").layout_mode,
            Some(super::LAYOUT_MODE_PHYSICAL)
        );

        let mut fixed = state();
        fixed.supports_changing_layout_mode = false;
        let current = fixed.layout().expect("converts");
        assert_eq!(
            plan(&fixed, &current, &current).expect("plans").layout_mode,
            None
        );
        assert_eq!(
            plan(&fixed, &current, &layout),
            Err(PlanError::LayoutMode {
                asked: LayoutMode::Physical,
                current: LayoutMode::Logical
            })
        );
    }
}
