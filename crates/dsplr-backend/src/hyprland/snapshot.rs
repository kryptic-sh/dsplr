//! Hyprland's monitors as dsplr's [`Layout`].

use dsplr_model::{Layout, Output, OutputEntry, OutputId, OutputState, Transform, hz_to_mhz};

use super::{ModeError, Monitor};

/// A monitor Hyprland reported that dsplr cannot represent.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SnapshotError {
    #[error("{connector}: {source}")]
    Mode {
        connector: String,
        source: ModeError,
    },
    #[error("{connector}: transform {value} is not a wl_output transform")]
    Transform { connector: String, value: u8 },
    #[error("{connector}: refresh rate {hertz} Hz is not a refresh rate")]
    Refresh { connector: String, hertz: f64 },
    #[error("{connector} mirrors monitor {mirrored:?}, which is not an id Hyprland listed")]
    MirrorSource { connector: String, mirrored: String },
}

/// Converts Hyprland's `j/monitors all` reply into a [`Layout`], one entry
/// per monitor, disabled ones included.
///
/// A disabled monitor has no mode and sits at the origin, since Hyprland
/// reports no geometry for it. `adaptive_sync` is not known: this reply
/// says only whether VRR is active right now, not Hyprland's configured
/// `vrr` rule.
pub fn layout(monitors: &[Monitor]) -> Result<Layout, SnapshotError> {
    let entries = monitors
        .iter()
        .map(|monitor| entry(monitor, monitors))
        .collect::<Result<_, _>>()?;
    Ok(Layout { entries })
}

fn id(monitor: &Monitor) -> OutputId {
    OutputId::new(
        &monitor.make,
        &monitor.model,
        &monitor.serial,
        &monitor.name,
    )
}

fn entry(monitor: &Monitor, all: &[Monitor]) -> Result<OutputEntry, SnapshotError> {
    let connector = || monitor.name.clone();
    let modes = monitor.modes().map_err(|source| SnapshotError::Mode {
        connector: connector(),
        source,
    })?;
    let transform = Transform::from_wayland(u32::from(monitor.transform)).ok_or_else(|| {
        SnapshotError::Transform {
            connector: connector(),
            value: monitor.transform,
        }
    })?;
    let unknown_mirror = |mirrored: String| SnapshotError::MirrorSource {
        connector: connector(),
        mirrored,
    };
    let mirror_of = match monitor.mirror_source() {
        Ok(None) => None,
        Ok(Some(source)) => Some(
            all.iter()
                .find(|other| other.id == source)
                .map(id)
                .ok_or_else(|| unknown_mirror(source.to_string()))?,
        ),
        Err(text) => return Err(unknown_mirror(text.to_owned())),
    };

    let state = if monitor.disabled {
        OutputState {
            enabled: false,
            mode: None,
            transform,
            mirror_of,
            ..Default::default()
        }
    } else {
        let refresh_mhz = hz_to_mhz(monitor.refresh_rate)
            .filter(|&mhz| mhz > 0)
            .ok_or_else(|| SnapshotError::Refresh {
                connector: connector(),
                hertz: monitor.refresh_rate,
            })?;
        let current = modes
            .iter()
            .find(|mode| {
                (mode.width, mode.height, mode.refresh_mhz)
                    == (monitor.width, monitor.height, refresh_mhz)
            })
            .cloned()
            // Not among the listed modes: a custom mode.
            .unwrap_or_else(|| dsplr_model::Mode::new(monitor.width, monitor.height, refresh_mhz));
        OutputState {
            enabled: true,
            mode: Some(current),
            x: monitor.x,
            y: monitor.y,
            scale: monitor.scale,
            transform,
            // `vrr` in this reply is whether adaptive sync is active right
            // now, not the configured rule: a fullscreen-only rule reads as
            // off outside fullscreen. So the rule is not known.
            adaptive_sync: None,
            mirror_of,
            ..Default::default()
        }
    };

    let physical_size_mm = (monitor.physical_width > 0 && monitor.physical_height > 0)
        .then_some((monitor.physical_width, monitor.physical_height));
    Ok(OutputEntry {
        id: id(monitor),
        output: Some(Output {
            connector: connector(),
            description: monitor.description.clone(),
            make: monitor.make.clone(),
            model: monitor.model.clone(),
            serial: monitor.serial.clone(),
            physical_size_mm,
            modes,
        }),
        state,
    })
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Mode, OutputId, Transform};

    use super::{SnapshotError, layout};
    use crate::hyprland::Monitor;

    fn owner() -> Vec<Monitor> {
        Monitor::parse_all(include_str!(
            "../../tests/fixtures/hyprland/monitors-all.json"
        ))
        .expect("parses")
    }

    #[test]
    fn the_owner_s_monitors_become_a_layout() {
        let layout = layout(&owner()).expect("converts");
        assert_eq!(layout.entries.len(), 2);

        let panel = &layout.entries[0];
        assert_eq!(panel.id, OutputId::Connector("eDP-1".into()), "no serial");
        assert!(!panel.state.enabled);
        assert_eq!(panel.state.mode, None);
        let panel_output = panel.output.as_ref().expect("live output");
        assert_eq!(panel_output.physical_size_mm, Some((310, 170)));
        assert_eq!(panel_output.modes.len(), 2);

        let external = &layout.entries[1];
        assert_eq!(
            external.id,
            OutputId::Edid {
                make: "AOC".into(),
                model: "Q27G4".into(),
                serial: "SERIAL0000001".into(),
            }
        );
        let state = &external.state;
        assert!(state.enabled);
        assert_eq!(state.mode, Some(Mode::new(2560, 1440, 120_000)));
        assert_eq!((state.x, state.y), (0, 0));
        assert!((state.scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Normal);
        assert_eq!(state.adaptive_sync, None);
        assert_eq!(state.mirror_of, None);
        let output = external.output.as_ref().expect("live output");
        assert_eq!(output.connector, "HDMI-A-1");
        assert_eq!(output.description, "AOC Q27G4 SERIAL0000001");
    }

    #[test]
    fn settings_are_carried_over() {
        let mut monitors = owner();
        let external = &mut monitors[1];
        external.x = 1920;
        external.y = 120;
        external.scale = 1.25;
        external.transform = 1;
        external.vrr = true;
        external.width = 3840;
        external.height = 2160;
        external.refresh_rate = 59.94;
        monitors[0].mirror_of = monitors[1].id.to_string();

        let layout = layout(&monitors).expect("converts");
        let state = &layout.entries[1].state;
        assert_eq!((state.x, state.y), (1920, 120));
        assert!((state.scale - 1.25).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Rotate90);
        assert_eq!(
            state.adaptive_sync, None,
            "active VRR is not the configured rule"
        );
        assert_eq!(state.mode, Some(Mode::new(3840, 2160, 59_940)));
        assert_eq!(
            layout.entries[0].state.mirror_of,
            Some(layout.entries[1].id.clone())
        );
    }

    #[test]
    fn a_mode_not_listed_is_kept_as_a_custom_mode() {
        let mut monitors = owner();
        monitors[1].width = 1234;
        monitors[1].height = 567;
        let layout = layout(&monitors).expect("converts");
        assert_eq!(
            layout.entries[1].state.mode,
            Some(Mode::new(1234, 567, 120_000))
        );
    }

    #[test]
    fn what_dsplr_cannot_represent_is_an_error_naming_the_monitor() {
        let mut bad_transform = owner();
        bad_transform[1].transform = 8;
        assert_eq!(
            layout(&bad_transform),
            Err(SnapshotError::Transform {
                connector: "HDMI-A-1".into(),
                value: 8
            })
        );

        for unknown in ["9", "eDP-1"] {
            let mut bad_mirror = owner();
            bad_mirror[1].mirror_of = unknown.into();
            assert_eq!(
                layout(&bad_mirror),
                Err(SnapshotError::MirrorSource {
                    connector: "HDMI-A-1".into(),
                    mirrored: unknown.into()
                }),
                "{unknown}"
            );
        }

        let mut bad_refresh = owner();
        bad_refresh[1].refresh_rate = f64::NAN;
        assert!(matches!(
            layout(&bad_refresh),
            Err(SnapshotError::Refresh { connector, .. }) if connector == "HDMI-A-1"
        ));

        let mut bad_mode = owner();
        bad_mode[0].available_modes.push("nonsense".into());
        assert!(matches!(
            layout(&bad_mode),
            Err(SnapshotError::Mode { connector, .. }) if connector == "eDP-1"
        ));
    }
}
