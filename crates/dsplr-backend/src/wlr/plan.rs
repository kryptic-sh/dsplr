//! A [`Layout`] as the requests of one `zwlr_output_configuration_v1`: what
//! each head is set to, decided before anything is sent.

use dsplr_model::{AdaptiveSync, Layout, Mode, ModeHandle, OutputEntry, OutputId};

use super::heads::{ADAPTIVE_SYNC_DISABLED, ADAPTIVE_SYNC_ENABLED, Head, HeadMode, scale_to_fixed};

/// The first manager version with adaptive sync.
pub const ADAPTIVE_SYNC_SINCE: u32 = 4;

/// What one head is set to. The protocol requires every head in a
/// configuration, so there is one per head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadConfig {
    Disable,
    /// Enabled as it is: the layout does not name the head, and an enabled
    /// head with nothing set keeps its settings.
    Unchanged,
    Enable(Settings),
}

/// The settings of an enabled head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: ModeChoice,
    pub position: (i32, i32),
    /// A `wl_output.transform` value.
    pub transform: i32,
    /// A `wl_fixed`.
    pub scale: i32,
    /// An `adaptive_sync_state`; `None` leaves it as it is.
    pub adaptive_sync: Option<u32>,
}

/// How a head's mode is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeChoice {
    /// `set_mode` with the mode object of this [`HeadMode::handle`].
    Listed(u64),
    /// `set_custom_mode`, for a mode no object of the head has.
    Custom {
        width: i32,
        height: i32,
        refresh_mhz: i32,
    },
}

/// Why a layout cannot be put into a configuration. Nothing is sent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("{0}: no head is on its connector")]
    NotConnected(OutputId),
    #[error("{0} is in the layout more than once")]
    Twice(String),
    #[error("{0}: enabled without a mode")]
    NoMode(String),
    #[error(
        "{connector}: mode {width}x{height} at {refresh_mhz} mHz is out of the protocol's range"
    )]
    ModeRange {
        connector: String,
        width: u32,
        height: u32,
        refresh_mhz: u32,
    },
    #[error("{connector}: scale {scale} is not a scale a wl_fixed can hold")]
    Scale { connector: String, scale: String },
    #[error("{connector}: {what} cannot be set through wlr-output-management")]
    Unsupported {
        connector: String,
        what: &'static str,
    },
}

/// The configuration for `layout`, one [`HeadConfig`] per head of `heads`,
/// in their order, for a manager of `version`.
///
/// Entries name heads by connector. A head the layout does not name keeps
/// its state. A setting the protocol cannot express (mirroring, a primary
/// output, colour preset, bit depth, overscan, RGB range, automatic adaptive
/// sync, or adaptive sync before version 4) is refused rather than dropped.
pub fn plan(layout: &Layout, heads: &[Head], version: u32) -> Result<Vec<HeadConfig>, PlanError> {
    let mut configs: Vec<Option<HeadConfig>> = vec![None; heads.len()];
    for entry in &layout.entries {
        let index = entry
            .connector()
            .and_then(|connector| heads.iter().position(|head| head.name == connector))
            .ok_or_else(|| PlanError::NotConnected(entry.id.clone()))?;
        let head = &heads[index];
        if configs[index].is_some() {
            return Err(PlanError::Twice(head.name.clone()));
        }
        configs[index] = Some(config(entry, head, version)?);
    }
    Ok(configs
        .into_iter()
        .zip(heads)
        .map(|(config, head)| {
            config.unwrap_or(if head.enabled {
                HeadConfig::Unchanged
            } else {
                HeadConfig::Disable
            })
        })
        .collect())
}

fn config(entry: &OutputEntry, head: &Head, version: u32) -> Result<HeadConfig, PlanError> {
    let state = &entry.state;
    let unsupported = |what| PlanError::Unsupported {
        connector: head.name.clone(),
        what,
    };
    let refused = [
        (state.mirror_of.is_some(), "mirroring"),
        (state.primary, "a primary output"),
        (state.colour_preset.is_some(), "a colour preset"),
        (state.bit_depth.is_some(), "bit depth"),
        (state.overscan.is_some(), "overscan"),
        (state.rgb_range.is_some(), "RGB range"),
    ];
    if let Some((_, what)) = refused.into_iter().find(|(set, _)| *set) {
        return Err(unsupported(what));
    }
    if !state.enabled {
        return Ok(HeadConfig::Disable);
    }

    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| PlanError::NoMode(head.name.clone()))?;
    let scale = scale_to_fixed(state.scale).ok_or_else(|| PlanError::Scale {
        connector: head.name.clone(),
        scale: state.scale.to_string(),
    })?;
    let adaptive_sync = match state.adaptive_sync {
        None => None,
        Some(_) if version < ADAPTIVE_SYNC_SINCE => {
            return Err(unsupported(
                "adaptive sync (the compositor's output manager is older than version 4)",
            ));
        }
        Some(AdaptiveSync::Off) => Some(ADAPTIVE_SYNC_DISABLED),
        Some(AdaptiveSync::On) => Some(ADAPTIVE_SYNC_ENABLED),
        Some(AdaptiveSync::Automatic) => return Err(unsupported("automatic adaptive sync")),
    };
    Ok(HeadConfig::Enable(Settings {
        mode: choose_mode(head, mode)?,
        position: (state.x, state.y),
        transform: i32::try_from(state.transform.to_wayland())
            .expect("wl_output.transform values are 0 to 7"),
        scale,
        adaptive_sync,
    }))
}

/// The mode object `mode` names on `head`, or a custom mode when none does.
///
/// A head may list one size and refresh more than once (interlaced and
/// progressive variants), so a mode read from this compositor carries its
/// ordinal among its head's modes ([`HeadMode::handle`]), and `head` (the
/// same connector, read again on this connection) gives back the object at
/// that ordinal as long as it still has that size and refresh. A mode
/// without one (from a file), or whose ordinal now names another size,
/// takes the
/// object the head is driven at when that matches, else the first that
/// matches within the refresh tolerance. With no match at all it is a custom
/// mode, which the compositor's own `test` is the check for.
fn choose_mode(head: &Head, mode: &Mode) -> Result<ModeChoice, PlanError> {
    let exact = |listed: &&HeadMode| {
        u32::try_from(listed.width) == Ok(mode.width)
            && u32::try_from(listed.height) == Ok(mode.height)
            && u32::try_from(listed.refresh_mhz) == Ok(mode.refresh_mhz)
    };
    if let Some(ModeHandle(handle)) = mode.handle
        && let Some(listed) = head
            .modes
            .iter()
            .filter(exact)
            .find(|listed| listed.handle == handle)
    {
        return Ok(ModeChoice::Listed(listed.handle));
    }

    let matching: Vec<&HeadMode> = head
        .modes
        .iter()
        .filter(|listed| {
            let (Ok(width), Ok(height), Ok(refresh_mhz)) = (
                u32::try_from(listed.width),
                u32::try_from(listed.height),
                u32::try_from(listed.refresh_mhz),
            ) else {
                return false;
            };
            mode.matches(&Mode::new(width, height, refresh_mhz))
        })
        .collect();
    let chosen = matching
        .iter()
        .find(|listed| Some(listed.handle) == head.current_mode)
        .or_else(|| matching.first());
    if let Some(listed) = chosen {
        return Ok(ModeChoice::Listed(listed.handle));
    }

    match (
        i32::try_from(mode.width),
        i32::try_from(mode.height),
        i32::try_from(mode.refresh_mhz),
    ) {
        (Ok(width), Ok(height), Ok(refresh_mhz)) => Ok(ModeChoice::Custom {
            width,
            height,
            refresh_mhz,
        }),
        _ => Err(PlanError::ModeRange {
            connector: head.name.clone(),
            width: mode.width,
            height: mode.height,
            refresh_mhz: mode.refresh_mhz,
        }),
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Layout, Mode, ModeHandle, OutputEntry, OutputId, OutputState, RgbRange,
        Transform,
    };

    use super::super::heads::{layout, tests::owner};
    use super::{HeadConfig, ModeChoice, PlanError, Settings, plan};

    const V4: u32 = 4;

    fn settings(config: &HeadConfig) -> &Settings {
        match config {
            HeadConfig::Enable(settings) => settings,
            other => panic!("not enabled: {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_applied_back_sets_every_head_as_it_is() {
        let heads = owner();
        let snapshot = layout(&heads).expect("converts");
        let configs = plan(&snapshot, &heads, V4).expect("plans");
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0], HeadConfig::Disable);
        assert_eq!(
            configs[1],
            HeadConfig::Enable(Settings {
                mode: ModeChoice::Listed(0),
                position: (0, 0),
                transform: 0,
                scale: 256,
                adaptive_sync: Some(0),
            })
        );
    }

    #[test]
    fn each_setting_becomes_its_request() {
        let heads = owner();
        let mut layout = layout(&heads).expect("converts");
        let state = &mut layout.entries[1].state;
        (state.x, state.y) = (-1920, 240);
        state.transform = Transform::Flipped270;
        state.scale = 1.75;
        state.adaptive_sync = Some(AdaptiveSync::On);
        layout.entries[0].state = OutputState {
            mode: Some(Mode::new(1920, 1200, 48_000)),
            ..OutputState::default()
        };

        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(
            configs[1],
            HeadConfig::Enable(Settings {
                mode: ModeChoice::Listed(0),
                position: (-1920, 240),
                transform: 7,
                scale: 448,
                adaptive_sync: Some(1),
            })
        );
        let panel = settings(&configs[0]);
        assert_eq!(panel.mode, ModeChoice::Listed(1), "matched by number");
        assert_eq!(panel.adaptive_sync, None, "not asked: left alone");
    }

    #[test]
    fn a_duplicate_mode_keeps_the_object_it_was_read_from() {
        let mut heads = owner();
        heads[1].current_mode = Some(2);
        let layout = layout(&heads).expect("converts");
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(settings(&configs[1]).mode, ModeChoice::Listed(2));
    }

    #[test]
    fn a_mode_without_a_handle_takes_the_current_object_then_the_first() {
        let mut heads = owner();
        let mut layout = layout(&heads).expect("converts");
        layout.entries[1].state.mode = Some(Mode::new(1920, 1080, 60_000));
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(settings(&configs[1]).mode, ModeChoice::Listed(1), "first");

        heads[1].current_mode = Some(2);
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(settings(&configs[1]).mode, ModeChoice::Listed(2), "current");

        // Within the refresh tolerance, as a config file rounds it.
        layout.entries[1].state.mode = Some(Mode::new(1280, 720, 59_940 + 3));
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(settings(&configs[1]).mode, ModeChoice::Listed(3));
    }

    #[test]
    fn a_stale_handle_is_matched_again_by_number() {
        let heads = owner();
        let mut layout = layout(&heads).expect("converts");
        layout.entries[1].state.mode = Some(Mode {
            handle: Some(ModeHandle(2)),
            ..Mode::new(1280, 720, 59_940)
        });
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(settings(&configs[1]).mode, ModeChoice::Listed(3));
    }

    #[test]
    fn a_mode_no_object_has_is_a_custom_mode() {
        let heads = owner();
        let mut layout = layout(&heads).expect("converts");
        layout.entries[1].state.mode = Some(Mode::new(1234, 567, 0));
        let configs = plan(&layout, &heads, V4).expect("plans");
        assert_eq!(
            settings(&configs[1]).mode,
            ModeChoice::Custom {
                width: 1234,
                height: 567,
                refresh_mhz: 0
            }
        );

        layout.entries[1].state.mode = Some(Mode::new(u32::MAX, 567, 0));
        assert!(matches!(
            plan(&layout, &heads, V4),
            Err(PlanError::ModeRange {
                width: u32::MAX,
                ..
            })
        ));
    }

    #[test]
    fn a_head_the_layout_does_not_name_keeps_its_state() {
        let heads = owner();
        let mut layout = layout(&heads).expect("converts");
        layout.entries.clear();
        assert_eq!(
            plan(&layout, &heads, V4).expect("plans"),
            [HeadConfig::Disable, HeadConfig::Unchanged]
        );
    }

    #[test]
    fn what_the_protocol_cannot_express_is_refused_not_dropped() {
        let heads = owner();
        let snapshot = layout(&heads).expect("converts");
        type Edit = fn(&mut OutputState);
        let edits: [(Edit, &str); 7] = [
            (
                |state| state.mirror_of = Some(OutputId::Connector("eDP-1".into())),
                "mirroring",
            ),
            (|state| state.primary = true, "a primary output"),
            (
                |state| state.colour_preset = Some("hdr".into()),
                "a colour preset",
            ),
            (|state| state.bit_depth = Some(10), "bit depth"),
            (|state| state.overscan = Some(5), "overscan"),
            (|state| state.rgb_range = Some(RgbRange::Full), "RGB range"),
            (
                |state| state.adaptive_sync = Some(AdaptiveSync::Automatic),
                "automatic adaptive sync",
            ),
        ];
        for (edit, expected) in edits {
            let mut layout = snapshot.clone();
            edit(&mut layout.entries[1].state);
            assert_eq!(
                plan(&layout, &heads, V4),
                Err(PlanError::Unsupported {
                    connector: "HDMI-A-1".into(),
                    what: expected
                })
            );
        }

        // Mirroring is refused on a disabled output too.
        let mut layout = snapshot.clone();
        layout.entries[0].state.mirror_of = Some(snapshot.entries[1].id.clone());
        assert!(matches!(
            plan(&layout, &heads, V4),
            Err(PlanError::Unsupported {
                what: "mirroring",
                ..
            })
        ));

        // Adaptive sync needs version 4; leaving it unset does not.
        assert!(matches!(
            plan(&snapshot, &heads, 3),
            Err(PlanError::Unsupported { what, .. }) if what.starts_with("adaptive sync")
        ));
        let mut unset = snapshot;
        unset.entries[1].state.adaptive_sync = None;
        assert!(plan(&unset, &heads, 3).is_ok());
    }

    #[test]
    fn a_layout_that_does_not_fit_the_heads_is_refused() {
        let heads = owner();
        let snapshot = layout(&heads).expect("converts");

        let mut twice = snapshot.clone();
        twice.entries.push(snapshot.entries[1].clone());
        assert_eq!(
            plan(&twice, &heads, V4),
            Err(PlanError::Twice("HDMI-A-1".into()))
        );

        let stranger = OutputEntry {
            id: OutputId::Connector("DP-9".into()),
            output: None,
            state: OutputState::default(),
        };
        assert_eq!(
            plan(
                &Layout {
                    entries: vec![stranger]
                },
                &heads,
                V4
            ),
            Err(PlanError::NotConnected(OutputId::Connector("DP-9".into())))
        );

        let mut no_mode = snapshot.clone();
        no_mode.entries[1].state.mode = None;
        assert_eq!(
            plan(&no_mode, &heads, V4),
            Err(PlanError::NoMode("HDMI-A-1".into()))
        );

        let mut bad_scale = snapshot;
        bad_scale.entries[1].state.scale = 0.0;
        assert!(matches!(
            plan(&bad_scale, &heads, V4),
            Err(PlanError::Scale { .. })
        ));
    }
}
