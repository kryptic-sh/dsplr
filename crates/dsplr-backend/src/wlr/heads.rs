//! The heads `zwlr_output_manager_v1` describes, as plain data, and those as
//! dsplr's [`Layout`].

use dsplr_model::{
    AdaptiveSync, Layout, Mode, ModeHandle, Output, OutputEntry, OutputId, OutputState, Transform,
};

/// A `wl_fixed` counts in units of 1/256.
const FIXED_ONE: f64 = 256.0;

/// `adaptive_sync_state.disabled`.
pub const ADAPTIVE_SYNC_DISABLED: u32 = 0;

/// `adaptive_sync_state.enabled`.
pub const ADAPTIVE_SYNC_ENABLED: u32 = 1;

/// One head as the compositor described it, up to its last `done`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// The connector name, such as `HDMI-A-1`.
    pub name: String,
    pub description: String,
    /// Empty when the compositor did not say (it is optional, and new in
    /// version 2).
    pub make: String,
    /// Empty when the compositor did not say.
    pub model: String,
    /// Empty when the compositor did not say.
    pub serial_number: String,
    /// Width and height in millimetres; zero when unknown.
    pub physical_size: (i32, i32),
    pub enabled: bool,
    /// The modes it offers, in the compositor's order.
    pub modes: Vec<HeadMode>,
    /// The [`HeadMode::handle`] of the mode it is driven at, while enabled.
    pub current_mode: Option<u64>,
    pub position: (i32, i32),
    /// A `wl_output.transform` value.
    pub transform: i32,
    /// A `wl_fixed`.
    pub scale: i32,
    /// An `adaptive_sync_state`, when the manager is version 4 or later and
    /// said.
    pub adaptive_sync: Option<u32>,
}

impl Default for Head {
    /// A head nothing has been said about yet: disabled, unscaled.
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            serial_number: String::new(),
            physical_size: (0, 0),
            enabled: false,
            modes: Vec::new(),
            current_mode: None,
            position: (0, 0),
            transform: 0,
            scale: scale_to_fixed(1.0).expect("1 is a wl_fixed"),
            adaptive_sync: None,
        }
    }
}

/// One mode object of a head.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadMode {
    /// The mode's ordinal among its head's modes, in the order the
    /// compositor announced them on this connection. It becomes the model's
    /// [`ModeHandle`], so an apply, on a connection of its own, finds the
    /// same object again: on the head with the same connector, at the same
    /// ordinal, with the same size and refresh, or else by size and refresh
    /// alone (`plan`).
    pub handle: u64,
    pub width: i32,
    pub height: i32,
    /// Millihertz; zero when the compositor did not say.
    pub refresh_mhz: i32,
    pub preferred: bool,
}

/// A head the compositor described that dsplr cannot represent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("{connector}: mode {width}x{height} at {refresh_mhz} mHz is not a mode")]
    Mode {
        connector: String,
        width: i32,
        height: i32,
        refresh_mhz: i32,
    },
    #[error("{connector}: transform {value} is not a wl_output transform")]
    Transform { connector: String, value: i32 },
    #[error("{connector}: scale {value}/256 is not a positive scale")]
    Scale { connector: String, value: i32 },
    #[error("{connector}: adaptive sync state {value} is not one the protocol defines")]
    AdaptiveSync { connector: String, value: u32 },
    #[error("{connector} is enabled, but the compositor named no current mode for it")]
    NoCurrentMode { connector: String },
}

/// The scale a `wl_fixed` holds.
pub fn fixed_to_scale(fixed: i32) -> f64 {
    f64::from(fixed) / FIXED_ONE
}

/// `scale` as a `wl_fixed`, rounded to the nearest 1/256. `None` for a scale
/// that is not a positive number, or rounds to zero or past the type.
pub fn scale_to_fixed(scale: f64) -> Option<i32> {
    let fixed = (scale * FIXED_ONE).round();
    if (1.0..=f64::from(i32::MAX)).contains(&fixed) {
        // Whole and inside i32's range, checked above, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        Some(fixed as i32)
    } else {
        None
    }
}

/// The heads as a [`Layout`], one entry per head, disabled ones included.
///
/// A disabled head has no mode and sits at the origin, unscaled and
/// untransformed: the protocol sends those only while a head is enabled.
/// Adaptive sync is known only from a version-4 manager.
pub fn layout(heads: &[Head]) -> Result<Layout, SnapshotError> {
    let entries = heads.iter().map(entry).collect::<Result<_, _>>()?;
    Ok(Layout { entries })
}

fn entry(head: &Head) -> Result<OutputEntry, SnapshotError> {
    let connector = || head.name.clone();
    let modes = head
        .modes
        .iter()
        .map(|mode| model_mode(mode).ok_or_else(|| mode_error(&head.name, mode)))
        .collect::<Result<Vec<_>, _>>()?;

    let state = if head.enabled {
        let mode = head
            .current_mode
            .and_then(|current| {
                modes
                    .iter()
                    .find(|mode| mode.handle == Some(ModeHandle(current)))
            })
            .cloned()
            .ok_or_else(|| SnapshotError::NoCurrentMode {
                connector: connector(),
            })?;
        let transform = u32::try_from(head.transform)
            .ok()
            .and_then(Transform::from_wayland)
            .ok_or_else(|| SnapshotError::Transform {
                connector: connector(),
                value: head.transform,
            })?;
        if head.scale <= 0 {
            return Err(SnapshotError::Scale {
                connector: connector(),
                value: head.scale,
            });
        }
        let adaptive_sync = match head.adaptive_sync {
            None => None,
            Some(ADAPTIVE_SYNC_DISABLED) => Some(AdaptiveSync::Off),
            Some(ADAPTIVE_SYNC_ENABLED) => Some(AdaptiveSync::On),
            Some(value) => {
                return Err(SnapshotError::AdaptiveSync {
                    connector: connector(),
                    value,
                });
            }
        };
        OutputState {
            enabled: true,
            mode: Some(mode),
            x: head.position.0,
            y: head.position.1,
            scale: fixed_to_scale(head.scale),
            transform,
            adaptive_sync,
            ..OutputState::default()
        }
    } else {
        OutputState {
            enabled: false,
            mode: None,
            ..OutputState::default()
        }
    };

    let physical_size_mm = match head.physical_size {
        (width, height) if width > 0 && height > 0 => {
            Some((width.unsigned_abs(), height.unsigned_abs()))
        }
        _ => None,
    };
    Ok(OutputEntry {
        id: OutputId::new(&head.make, &head.model, &head.serial_number, &head.name),
        output: Some(Output {
            connector: connector(),
            description: head.description.clone(),
            physical_size_mm,
            modes,
        }),
        state,
    })
}

/// The model's form of a mode object, carrying its handle. `None` for a
/// size or refresh that is negative.
fn model_mode(mode: &HeadMode) -> Option<Mode> {
    Some(Mode {
        preferred: mode.preferred,
        handle: Some(ModeHandle(mode.handle)),
        ..Mode::new(
            u32::try_from(mode.width).ok()?,
            u32::try_from(mode.height).ok()?,
            u32::try_from(mode.refresh_mhz).ok()?,
        )
    })
}

fn mode_error(connector: &str, mode: &HeadMode) -> SnapshotError {
    SnapshotError::Mode {
        connector: connector.to_owned(),
        width: mode.width,
        height: mode.height,
        refresh_mhz: mode.refresh_mhz,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use dsplr_model::{AdaptiveSync, Mode, ModeHandle, OutputId, Transform};

    use super::{
        ADAPTIVE_SYNC_ENABLED, Head, HeadMode, SnapshotError, fixed_to_scale, layout,
        scale_to_fixed,
    };

    fn mode(handle: u64, width: i32, height: i32, refresh_mhz: i32) -> HeadMode {
        HeadMode {
            handle,
            width,
            height,
            refresh_mhz,
            preferred: false,
        }
    }

    /// The owner's two monitors as a wlroots compositor would describe them:
    /// a laptop panel switched off, and an AOC at 2560x1440@120 that lists
    /// 1920x1080@60 twice, as Hyprland's reply does.
    pub fn owner() -> Vec<Head> {
        vec![
            Head {
                name: "eDP-1".into(),
                description: "BOE 0x0AFC".into(),
                make: "BOE".into(),
                model: "0x0AFC".into(),
                physical_size: (310, 170),
                enabled: false,
                modes: vec![
                    HeadMode {
                        preferred: true,
                        ..mode(0, 1920, 1200, 60_000)
                    },
                    mode(1, 1920, 1200, 48_000),
                ],
                ..Head::default()
            },
            Head {
                name: "HDMI-A-1".into(),
                description: "AOC Q27G4 SERIAL0000001".into(),
                make: "AOC".into(),
                model: "Q27G4".into(),
                serial_number: "SERIAL0000001".into(),
                physical_size: (600, 340),
                enabled: true,
                modes: vec![
                    HeadMode {
                        preferred: true,
                        ..mode(0, 2560, 1440, 120_000)
                    },
                    mode(1, 1920, 1080, 60_000),
                    mode(2, 1920, 1080, 60_000),
                    mode(3, 1280, 720, 59_940),
                ],
                current_mode: Some(0),
                position: (0, 0),
                transform: 0,
                scale: 256,
                adaptive_sync: Some(0),
            },
        ]
    }

    #[test]
    fn the_heads_become_a_layout() {
        let layout = layout(&owner()).expect("converts");
        assert_eq!(layout.entries.len(), 2);

        let panel = &layout.entries[0];
        assert_eq!(panel.id, OutputId::Connector("eDP-1".into()), "no serial");
        assert!(!panel.state.enabled);
        assert_eq!(panel.state.mode, None);
        let panel_output = panel.output.as_ref().expect("live output");
        assert_eq!(panel_output.physical_size_mm, Some((310, 170)));
        assert_eq!(panel_output.modes.len(), 2);
        assert!(panel_output.modes[0].preferred);

        let external = &layout.entries[1];
        assert_eq!(
            external.id,
            OutputId::Edid {
                make: "AOC".into(),
                model: "Q27G4".into(),
                serial: "SERIAL0000001".into(),
            }
        );
        let output = external.output.as_ref().expect("live output");
        assert_eq!(output.connector, "HDMI-A-1");
        assert_eq!(output.description, "AOC Q27G4 SERIAL0000001");
        assert_eq!(output.modes.len(), 4, "both 1920x1080@60 are kept");
        assert_eq!(output.modes[2].handle, Some(ModeHandle(2)));
        let state = &external.state;
        assert!(state.enabled);
        let current = state.mode.as_ref().expect("a mode");
        assert!(current.matches(&Mode::new(2560, 1440, 120_000)));
        assert_eq!(current.handle, Some(ModeHandle(0)), "the object in use");
        assert!(current.preferred);
        assert_eq!((state.x, state.y), (0, 0));
        assert!((state.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Normal);
        assert_eq!(state.adaptive_sync, Some(AdaptiveSync::Off));
        assert_eq!(state.mirror_of, None);
    }

    #[test]
    fn settings_are_carried_over() {
        let mut heads = owner();
        let external = &mut heads[1];
        external.position = (1920, -120);
        external.scale = 320;
        external.transform = 5;
        external.adaptive_sync = Some(ADAPTIVE_SYNC_ENABLED);
        external.current_mode = Some(2);

        let layout = layout(&heads).expect("converts");
        let state = &layout.entries[1].state;
        assert_eq!((state.x, state.y), (1920, -120));
        assert!((state.scale - 1.25).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Flipped90);
        assert_eq!(state.adaptive_sync, Some(AdaptiveSync::On));
        assert_eq!(
            state.mode.as_ref().and_then(|mode| mode.handle),
            Some(ModeHandle(2)),
            "the second of the two 1920x1080@60 objects"
        );
    }

    #[test]
    fn a_manager_older_than_version_4_leaves_adaptive_sync_unknown() {
        let mut heads = owner();
        heads[1].adaptive_sync = None;
        let layout = layout(&heads).expect("converts");
        assert_eq!(layout.entries[1].state.adaptive_sync, None);
    }

    #[test]
    fn a_disabled_head_reads_as_off_at_the_origin_whatever_was_said_before() {
        let mut heads = owner();
        heads[1].enabled = false;
        heads[1].position = (1920, 0);
        heads[1].transform = 1;
        let layout = layout(&heads).expect("converts");
        let state = &layout.entries[1].state;
        assert!(!state.enabled);
        assert_eq!(state.mode, None);
        assert_eq!((state.x, state.y), (0, 0));
        assert_eq!(state.transform, Transform::Normal);
    }

    #[test]
    fn what_dsplr_cannot_represent_is_an_error_naming_the_head() {
        let connector = || "HDMI-A-1".to_owned();
        let mut bad = owner();
        bad[1].transform = 8;
        assert_eq!(
            layout(&bad),
            Err(SnapshotError::Transform {
                connector: connector(),
                value: 8
            })
        );

        let mut bad = owner();
        bad[1].scale = 0;
        assert_eq!(
            layout(&bad),
            Err(SnapshotError::Scale {
                connector: connector(),
                value: 0
            })
        );

        let mut bad = owner();
        bad[1].adaptive_sync = Some(2);
        assert_eq!(
            layout(&bad),
            Err(SnapshotError::AdaptiveSync {
                connector: connector(),
                value: 2
            })
        );

        for current in [None, Some(99)] {
            let mut bad = owner();
            bad[1].current_mode = current;
            assert_eq!(
                layout(&bad),
                Err(SnapshotError::NoCurrentMode {
                    connector: connector()
                }),
                "{current:?}"
            );
        }

        let mut bad = owner();
        bad[0].modes[1].refresh_mhz = -1;
        assert!(matches!(
            layout(&bad),
            Err(SnapshotError::Mode { connector, refresh_mhz: -1, .. }) if connector == "eDP-1"
        ));
    }

    #[test]
    fn scales_round_to_the_nearest_256th() {
        assert_eq!(scale_to_fixed(1.0), Some(256));
        assert_eq!(scale_to_fixed(1.5), Some(384));
        assert_eq!(scale_to_fixed(1.3), Some(333), "332.8 rounds up");
        assert!((fixed_to_scale(333) - 1.300_781_25).abs() < f64::EPSILON);
        for bad in [0.0, 0.001, -1.0, f64::NAN, f64::INFINITY, 1e10] {
            assert_eq!(scale_to_fixed(bad), None, "{bad}");
        }
    }
}
