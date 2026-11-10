//! A [`Layout`] as the requests of one `kde_output_configuration_v2`: what
//! each output device is set to, and the output order, decided before
//! anything is sent.

use dsplr_model::{AdaptiveSync, Layout, OVERSCAN_MAX, OutputEntry, OutputId, RgbRange};

use super::devices::{
    CAPABILITY_CUSTOM_MODES, CAPABILITY_DISABLE, CAPABILITY_OVERSCAN, CAPABILITY_RGB_RANGE,
    CAPABILITY_VRR, Device, ListedMode, RGB_AUTOMATIC, RGB_FULL, RGB_LIMITED, VRR_ALWAYS,
    VRR_AUTOMATIC, VRR_NEVER,
};
use crate::wayland::heads::{Head, scale_to_fixed};
use crate::wayland::plan::{ModeChoice, PlanError, assign, choose_mode};

/// The protocol named in a [`PlanError::Unsupported`].
const PROTOCOL: &str = "kde-output-management-v2";

/// The first `kde_output_management_v2` version with `set_priority`.
pub const PRIORITY_SINCE: u32 = 3;

/// The first `kde_output_management_v2` version with
/// `set_replication_source`: mirroring.
pub const REPLICATION_SINCE: u32 = 13;

/// The first `kde_output_management_v2` version with `create_mode_list`
/// and `set_custom_modes`: custom modes.
pub const CUSTOM_MODES_SINCE: u32 = 18;

/// The first `kde_output_management_v2` version whose mode lists have
/// `add_cvt`.
pub const ADD_CVT_SINCE: u32 = 22;

/// The first device version whose capabilities say whether it can be
/// turned off ([`CAPABILITY_DISABLE`]).
pub const DISABLE_CAPABILITY_SINCE: u32 = 25;

/// KWin keeps a scale in 120ths: `kde_output_configuration_v2_scale` rounds
/// the `wl_fixed` it is sent to the nearest 120th (KWin 6.8's
/// `src/wayland/outputmanagement_v2.cpp`), and the device sends that back as
/// a `wl_fixed` again.
pub const KWIN_SCALE_STEPS: u32 = 120;

/// What one device is set to. KWin changes only what a configuration
/// names, so a device the layout does not name is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceConfig {
    Unchanged,
    Disable,
    Enable(Settings),
}

/// The settings of an enabled device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The [`HeadMode::handle`](crate::wayland::heads::HeadMode::handle)
    /// of a listed mode: KWin's protocol takes no other. `None` for a
    /// custom mode, which [`Plan::mode_lists`] adds first: the device's
    /// mode is chosen once KWin lists it.
    pub mode: Option<u64>,
    pub position: (i32, i32),
    /// A `wl_output.transform` value.
    pub transform: i32,
    /// A `wl_fixed`.
    pub scale: i32,
    /// Overscan in percent; `None` leaves it as it is.
    pub overscan: Option<u32>,
    /// A `vrr_policy`; `None` leaves it as it is.
    pub vrr_policy: Option<u32>,
    /// An `rgb_range`; `None` leaves it as it is.
    pub rgb_range: Option<u32>,
    /// The uuid of the device to mirror, empty to mirror none; `None` where
    /// the management protocol is too old to say.
    pub replication_source: Option<String>,
}

/// The whole configuration: one [`DeviceConfig`] per device, in their
/// order, and each device's place in the output order when the layout
/// names a primary output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub devices: Vec<DeviceConfig>,
    /// `set_priority` for each device: 1 for the primary output, then the
    /// other outputs that will be on in their order now. `None` for one
    /// that will be off, and for every device when the layout names no
    /// primary output, which leaves the order as it is.
    pub priorities: Vec<Option<u32>>,
    /// For each device, the custom mode list to send first, in a
    /// configuration of its own, when the layout sets it to a mode it does
    /// not list: the custom modes it has, then the new one, last. `set_custom_modes`
    /// adds a mode to the device rather than switching to it, so the
    /// configuration that sets the mode can only follow once KWin lists it.
    pub mode_lists: Vec<Option<Vec<ListedMode>>>,
}

/// The configuration for `layout`, against `devices` as KWin last described
/// them, its output `order` (`None` when KWin offers no
/// `kde_output_order_v1`), for a `kde_output_management_v2` of `version`.
///
/// Entries name devices by connector. A setting KWin's protocol cannot
/// express (a colour preset, bit depth), one the device says
/// it does not have, and one the bound version cannot send are refused
/// rather than dropped.
///
/// The mode of each device that `chosen` (by device, empty for none) names
/// one for is taken as that mode object: a custom mode KWin added, which it
/// may list at another refresh than asked (it makes the timings itself).
pub fn plan(
    layout: &Layout,
    devices: &[Device],
    order: Option<&[String]>,
    version: u32,
    chosen: &[Option<u64>],
) -> Result<Plan, PlanError> {
    let heads: Vec<Head> = devices.iter().map(|device| device.head.clone()).collect();
    let assigned = assign(layout, &heads)?;
    let (configs, mode_lists) = assigned
        .iter()
        .zip(devices)
        .enumerate()
        .map(|(index, (entry, device))| match entry {
            Some(entry) => config(
                entry,
                device,
                devices,
                version,
                chosen.get(index).copied().flatten(),
            ),
            None => Ok((DeviceConfig::Unchanged, None)),
        })
        .collect::<Result<(Vec<_>, Vec<_>), _>>()?;
    let priorities = priorities(&assigned, devices, order, version)?;
    Ok(Plan {
        devices: configs,
        priorities,
        mode_lists,
    })
}

fn unsupported(device: &Device, what: &'static str) -> PlanError {
    PlanError::Unsupported {
        connector: device.head.name.clone(),
        what,
        protocol: PROTOCOL,
    }
}

/// One device's configuration, and the custom mode list to send first when
/// it is set to a mode it does not list; at the mode object `chosen` when
/// given.
fn config(
    entry: &OutputEntry,
    device: &Device,
    devices: &[Device],
    version: u32,
    chosen: Option<u64>,
) -> Result<(DeviceConfig, Option<Vec<ListedMode>>), PlanError> {
    let state = &entry.state;
    let unsupported = |what| unsupported(device, what);
    let refused = [
        (state.colour_preset.is_some(), "a colour preset"),
        (state.bit_depth.is_some(), "bit depth"),
    ];
    if let Some((_, what)) = refused.into_iter().find(|(set, _)| *set) {
        return Err(unsupported(what));
    }
    if !state.enabled {
        if device.version >= DISABLE_CAPABILITY_SINCE && !device.can(CAPABILITY_DISABLE) {
            return Err(unsupported("turning it off (KWin says it cannot be)"));
        }
        return Ok((DeviceConfig::Disable, None));
    }

    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| PlanError::NoMode(device.head.name.clone()))?;
    let choice = match chosen {
        Some(handle) => ModeChoice::Listed(handle),
        None => choose_mode(&device.head, mode)?,
    };
    let (mode, mode_list) = match choice {
        ModeChoice::Listed(handle) => (Some(handle), None),
        ModeChoice::Custom { .. } if version < CUSTOM_MODES_SINCE => {
            return Err(unsupported(
                "a mode the output does not list (KWin's output management is older than version 18)",
            ));
        }
        ModeChoice::Custom { .. } if !device.can(CAPABILITY_CUSTOM_MODES) => {
            return Err(unsupported(
                "a mode the output does not list (KWin says it takes no custom modes)",
            ));
        }
        ModeChoice::Custom { refresh_mhz: 0, .. } => {
            return Err(unsupported("a custom mode without a refresh rate"));
        }
        ModeChoice::Custom {
            width,
            height,
            refresh_mhz,
        } => {
            let mut list = device.custom_modes(version >= ADD_CVT_SINCE);
            list.push(ListedMode::Mode {
                width: u32::try_from(width).expect("chosen from a u32"),
                height: u32::try_from(height).expect("chosen from a u32"),
                refresh_mhz: u32::try_from(refresh_mhz).expect("chosen from a u32"),
                // The normal blanking, which every display takes.
                reduced_blanking: false,
            });
            (None, Some(list))
        }
    };
    let scale = scale_to_fixed(state.scale).ok_or_else(|| PlanError::Scale {
        connector: device.head.name.clone(),
        scale: state.scale.to_string(),
    })?;
    let vrr_policy = match state.adaptive_sync {
        None => None,
        Some(_) if !device.can(CAPABILITY_VRR) => {
            return Err(unsupported("adaptive sync (KWin says the output has none)"));
        }
        Some(AdaptiveSync::Off) => Some(VRR_NEVER),
        Some(AdaptiveSync::On) => Some(VRR_ALWAYS),
        Some(AdaptiveSync::Automatic) => Some(VRR_AUTOMATIC),
    };
    let overscan = match state.overscan {
        None => None,
        Some(_) if !device.can(CAPABILITY_OVERSCAN) => {
            return Err(unsupported("overscan (KWin says the output has none)"));
        }
        Some(percent) if percent > OVERSCAN_MAX => {
            return Err(unsupported("overscan above 100%"));
        }
        Some(percent) => Some(u32::from(percent)),
    };
    let rgb_range = match state.rgb_range {
        None => None,
        Some(_) if !device.can(CAPABILITY_RGB_RANGE) => {
            return Err(unsupported("RGB range (KWin says the output has none)"));
        }
        Some(RgbRange::Automatic) => Some(RGB_AUTOMATIC),
        Some(RgbRange::Full) => Some(RGB_FULL),
        Some(RgbRange::Limited) => Some(RGB_LIMITED),
    };
    let replication_source = match &state.mirror_of {
        Some(_) if version < REPLICATION_SINCE => {
            return Err(unsupported(
                "mirroring (KWin's output management is older than version 13)",
            ));
        }
        Some(source) => {
            let source = devices
                .iter()
                .find(|candidate| names(source, candidate))
                .ok_or_else(|| PlanError::NotConnected(source.clone()))?;
            if source.uuid == device.uuid {
                return Err(unsupported("mirroring itself"));
            }
            Some(source.uuid.clone())
        }
        None if version < REPLICATION_SINCE => None,
        None => Some(String::new()),
    };
    let settings = Settings {
        mode,
        position: (state.x, state.y),
        transform: i32::try_from(state.transform.to_wayland())
            .expect("wl_output.transform values are 0 to 7"),
        scale,
        overscan,
        vrr_policy,
        rgb_range,
        replication_source,
    };
    Ok((DeviceConfig::Enable(settings), mode_list))
}

/// Whether `id`, a mirror source, names `device`: its identity, or its
/// connector.
fn names(id: &OutputId, device: &Device) -> bool {
    *id == device.id()
        || matches!(id, OutputId::Connector(connector) if *connector == device.head.name)
}

/// Each device's `set_priority`, when the layout names a primary output that
/// is on. An output that is off is no primary whatever a file or profile
/// says, as on GNOME: KWin then picks one, as it does when none is named.
fn priorities(
    assigned: &[Option<&OutputEntry>],
    devices: &[Device],
    order: Option<&[String]>,
    version: u32,
) -> Result<Vec<Option<u32>>, PlanError> {
    let mut primaries = assigned
        .iter()
        .zip(devices)
        .filter(|(entry, _)| entry.is_some_and(|entry| entry.state.enabled && entry.state.primary));
    let Some((_, primary)) = primaries.next() else {
        return Ok(vec![None; devices.len()]);
    };
    if let Some((_, second)) = primaries.next() {
        return Err(unsupported(second, "a second primary output"));
    }
    let Some(order) = order else {
        return Err(unsupported(
            primary,
            "a primary output (KWin offers no kde_output_order_v1 to read the order from)",
        ));
    };
    if version < PRIORITY_SINCE {
        return Err(unsupported(
            primary,
            "a primary output (KWin's output management is older than version 3)",
        ));
    }

    // The outputs that will be on, primary first, then in KWin's order now,
    // then any it does not list (a mirror), in device order.
    let place = |device: &Device| {
        order
            .iter()
            .position(|name| *name == device.head.name)
            .unwrap_or(order.len())
    };
    let mut on: Vec<usize> = assigned
        .iter()
        .zip(devices)
        .enumerate()
        .filter(|(_, (entry, device))| {
            entry.map_or(device.head.enabled, |entry| entry.state.enabled)
        })
        .map(|(index, _)| index)
        .collect();
    on.sort_by_key(|&index| {
        let device = &devices[index];
        (device.uuid != primary.uuid, place(device), index)
    });
    let mut priorities = vec![None; devices.len()];
    for (rank, index) in (1..).zip(on) {
        priorities[index] = Some(rank);
    }
    Ok(priorities)
}

#[cfg(test)]
mod tests {
    use dsplr_model::{AdaptiveSync, Layout, Mode, OutputId, OutputState, RgbRange, Transform};

    use super::super::devices::tests::{devices, order};
    use super::super::devices::{
        CAPABILITY_CUSTOM_MODES, CAPABILITY_DISABLE, CAPABILITY_VRR, Device, ListedMode,
        MODE_CUSTOM, ModeExtras, VRR_AUTOMATIC, layout,
    };
    use super::{DeviceConfig, Plan, Settings, plan};
    use crate::wayland::heads::HeadMode;
    use crate::wayland::plan::PlanError;

    const V22: u32 = 22;

    fn snapshot() -> Layout {
        layout(&devices(), Some(&order())).expect("converts")
    }

    fn planned(layout: &Layout) -> Result<Plan, PlanError> {
        plan(layout, &devices(), Some(&order()), V22, &[])
    }

    fn settings(config: &DeviceConfig) -> &Settings {
        match config {
            DeviceConfig::Enable(settings) => settings,
            other => panic!("not enabled: {other:?}"),
        }
    }

    fn refused(layout: &Layout) -> &'static str {
        match planned(layout) {
            Err(PlanError::Unsupported { what, protocol, .. }) => {
                assert_eq!(protocol, "kde-output-management-v2");
                what
            }
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_applied_back_sets_every_device_and_the_order_as_they_are() {
        let sent = planned(&snapshot()).expect("plans");
        assert_eq!(sent.devices[0], DeviceConfig::Disable);
        assert_eq!(
            sent.devices[1],
            DeviceConfig::Enable(Settings {
                mode: Some(0),
                position: (0, 0),
                transform: 0,
                scale: 256,
                overscan: Some(0),
                vrr_policy: Some(0),
                rgb_range: Some(0),
                replication_source: Some(String::new()),
            })
        );
        let virtual_output = settings(&sent.devices[2]);
        assert_eq!(
            (
                virtual_output.overscan,
                virtual_output.vrr_policy,
                virtual_output.rgb_range
            ),
            (None, None, None),
            "a device without those capabilities is sent none"
        );
        assert_eq!(sent.priorities, [None, Some(1), Some(2)]);
    }

    #[test]
    fn each_setting_becomes_its_request() {
        let mut layout = snapshot();
        let state = &mut layout.entries[1].state;
        (state.x, state.y) = (1920, 0);
        state.transform = Transform::Rotate270;
        state.scale = 1.25;
        state.adaptive_sync = Some(AdaptiveSync::Automatic);
        state.overscan = Some(4);
        state.rgb_range = Some(RgbRange::Full);
        state.mode = Some(Mode::new(1920, 1080, 60_000));
        let sent = planned(&layout).expect("plans");
        assert_eq!(
            sent.devices[1],
            DeviceConfig::Enable(Settings {
                mode: Some(1),
                position: (1920, 0),
                transform: 3,
                scale: 320,
                overscan: Some(4),
                vrr_policy: Some(VRR_AUTOMATIC),
                rgb_range: Some(1),
                replication_source: Some(String::new()),
            })
        );
    }

    #[test]
    fn a_device_the_layout_does_not_name_is_left_alone() {
        let mut layout = snapshot();
        layout.entries.remove(0);
        let sent = planned(&layout).expect("plans");
        assert_eq!(sent.devices[0], DeviceConfig::Unchanged);
    }

    #[test]
    fn a_primary_output_puts_it_first_and_the_rest_in_kwin_s_order() {
        let mut layout = snapshot();
        layout.entries[1].state.primary = false;
        layout.entries[2].state.primary = true;
        let sent = planned(&layout).expect("plans");
        assert_eq!(sent.priorities, [None, Some(2), Some(1)]);

        // An output turned on in the same change comes after those KWin
        // lists.
        layout.entries[0].state = OutputState {
            mode: Some(Mode::new(1920, 1200, 60_000)),
            x: 4480,
            ..OutputState::default()
        };
        let sent = planned(&layout).expect("plans");
        assert_eq!(sent.priorities, [Some(3), Some(2), Some(1)]);

        // No primary output named: the order is left as it is.
        for entry in &mut layout.entries {
            entry.state.primary = false;
        }
        assert_eq!(planned(&layout).expect("plans").priorities, [None; 3]);
    }

    #[test]
    fn a_primary_flag_on_an_output_that_is_off_is_ignored() {
        // A file or profile can still say so; KWin picks the primary then.
        let mut off = snapshot();
        off.entries[1].state.primary = false;
        off.entries[0].state.primary = true;
        off.entries[0].state.enabled = false;
        assert_eq!(planned(&off).expect("plans").priorities, [None; 3]);
    }

    #[test]
    fn a_primary_output_that_cannot_be_set_is_refused() {
        let mut two = snapshot();
        two.entries[2].state.primary = true;
        assert_eq!(refused(&two), "a second primary output");

        let primary = snapshot();
        assert!(matches!(
            plan(&primary, &devices(), None, V22, &[]),
            Err(PlanError::Unsupported { what, .. }) if what.contains("kde_output_order_v1")
        ));
        assert!(matches!(
            plan(&primary, &devices(), Some(&order()), 2, &[]),
            Err(PlanError::Unsupported { what, .. }) if what.contains("version 3")
        ));
    }

    #[test]
    fn a_mirror_is_sent_as_its_source_s_uuid() {
        let mut layout = snapshot();
        layout.entries[2].state.mirror_of = Some(layout.entries[1].id.clone());
        let sent = planned(&layout).expect("plans");
        assert_eq!(
            settings(&sent.devices[2]).replication_source.as_deref(),
            Some("aoc")
        );

        // By connector too, as a file may name it.
        layout.entries[2].state.mirror_of = Some(OutputId::Connector("HDMI-A-1".into()));
        let sent = planned(&layout).expect("plans");
        assert_eq!(
            settings(&sent.devices[2]).replication_source.as_deref(),
            Some("aoc")
        );

        layout.entries[2].state.mirror_of = Some(OutputId::Connector("DP-9".into()));
        assert_eq!(
            planned(&layout),
            Err(PlanError::NotConnected(OutputId::Connector("DP-9".into())))
        );
        layout.entries[2].state.mirror_of = Some(layout.entries[2].id.clone());
        assert_eq!(refused(&layout), "mirroring itself");

        // Before version 13 a mirror cannot be sent, and none is cleared.
        layout.entries[2].state.mirror_of = Some(layout.entries[1].id.clone());
        assert!(matches!(
            plan(&layout, &devices(), Some(&order()), 12, &[]),
            Err(PlanError::Unsupported { what, .. }) if what.starts_with("mirroring")
        ));
        layout.entries[2].state.mirror_of = None;
        let old = plan(&layout, &devices(), Some(&order()), 12, &[]).expect("plans");
        assert_eq!(settings(&old.devices[2]).replication_source, None);
    }

    /// [`devices`] with the virtual output taking custom modes and already
    /// holding one, 1600x900 at 75 Hz, added by its CVT timings.
    fn with_a_custom_mode() -> Vec<Device> {
        let mut devices = devices();
        let device = &mut devices[2];
        device.capabilities |= CAPABILITY_CUSTOM_MODES;
        device.head.modes.push(HeadMode {
            handle: 1,
            width: 1600,
            height: 900,
            refresh_mhz: 74_954,
            preferred: false,
        });
        device.mode_extras = vec![
            ModeExtras {
                handle: 0,
                flags: 0,
                cvt: None,
            },
            ModeExtras {
                handle: 1,
                flags: MODE_CUSTOM,
                cvt: Some(CVT),
            },
        ];
        devices
    }

    /// 1600x900 at 75 Hz's CVT timings, as KWin would send them.
    const CVT: [u32; 12] = [
        151_250, 1600, 1704, 1872, 2144, 0, 900, 903, 908, 942, 0, 0x5,
    ];

    #[test]
    fn a_custom_mode_is_added_to_the_list_kwin_has_and_then_set() {
        let mut layout = snapshot();
        layout.entries[2].state.mode = Some(Mode::new(1280, 1024, 60_000));
        let sent = plan(&layout, &with_a_custom_mode(), Some(&order()), V22, &[]).expect("plans");
        assert_eq!(
            settings(&sent.devices[2]).mode,
            None,
            "set once KWin lists it"
        );
        let new = ListedMode::Mode {
            width: 1280,
            height: 1024,
            refresh_mhz: 60_000,
            reduced_blanking: false,
        };
        // The custom mode KWin has goes in again, or KWin would delete it.
        assert_eq!(
            sent.mode_lists,
            [None, None, Some(vec![ListedMode::Cvt(CVT), new.clone()])]
        );
        // Before `add_cvt`, by its size and refresh.
        let old = plan(&layout, &with_a_custom_mode(), Some(&order()), 21, &[]).expect("plans");
        assert_eq!(
            old.mode_lists[2],
            Some(vec![
                ListedMode::Mode {
                    width: 1600,
                    height: 900,
                    refresh_mhz: 74_954,
                    reduced_blanking: false,
                },
                new
            ])
        );
        // A mode it lists, custom or not, adds nothing.
        let listed =
            plan(&snapshot(), &with_a_custom_mode(), Some(&order()), V22, &[]).expect("plans");
        assert_eq!(listed.mode_lists, [None, None, None]);
        assert_eq!(settings(&listed.devices[2]).mode, Some(0));
    }

    #[test]
    fn once_kwin_lists_the_custom_mode_it_is_set_by_its_object() {
        let mut layout = snapshot();
        layout.entries[2].state.mode = Some(Mode::new(1280, 1024, 60_000));
        // KWin made it 1280x1024 at 59.895 Hz, its handle 2.
        let mut devices = with_a_custom_mode();
        devices[2].head.modes.push(HeadMode {
            handle: 2,
            width: 1280,
            height: 1024,
            refresh_mhz: 59_895,
            preferred: false,
        });
        let sent = plan(
            &layout,
            &devices,
            Some(&order()),
            V22,
            &[None, None, Some(2)],
        )
        .expect("plans");
        assert_eq!(sent.mode_lists, [None, None, None], "nothing more to add");
        assert_eq!(settings(&sent.devices[2]).mode, Some(2));
    }

    #[test]
    fn a_custom_mode_kwin_cannot_add_is_refused() {
        let mut layout = snapshot();
        layout.entries[2].state.mode = Some(Mode::new(1280, 1024, 60_000));
        let what = |result| match result {
            Err(PlanError::Unsupported { what, .. }) => what,
            other => panic!("not refused: {other:?}"),
        };
        assert_eq!(
            what(plan(
                &layout,
                &with_a_custom_mode(),
                Some(&order()),
                17,
                &[]
            )),
            "a mode the output does not list (KWin's output management is older than version 18)"
        );
        layout.entries[2].state.mode = Some(Mode::new(1280, 1024, 0));
        assert_eq!(
            what(plan(
                &layout,
                &with_a_custom_mode(),
                Some(&order()),
                V22,
                &[]
            )),
            "a custom mode without a refresh rate"
        );
    }

    #[test]
    fn what_kwin_cannot_take_is_refused_not_dropped() {
        // The AOC has no `custom_modes` capability.
        let mut custom = snapshot();
        custom.entries[1].state.mode = Some(Mode::new(1234, 567, 60_000));
        assert_eq!(
            refused(&custom),
            "a mode the output does not list (KWin says it takes no custom modes)"
        );

        let mut colour = snapshot();
        colour.entries[1].state.colour_preset = Some("hdr".into());
        assert_eq!(refused(&colour), "a colour preset");

        let mut depth = snapshot();
        depth.entries[2].state.bit_depth = Some(10);
        assert_eq!(refused(&depth), "bit depth");

        let mut overscan = snapshot();
        overscan.entries[1].state.overscan = Some(101);
        assert_eq!(refused(&overscan), "overscan above 100%");

        // The virtual output says it has none of these.
        type Edit = fn(&mut OutputState);
        let edits: [(Edit, &str); 3] = [
            (
                |state| state.adaptive_sync = Some(AdaptiveSync::On),
                "adaptive sync",
            ),
            (|state| state.overscan = Some(0), "overscan"),
            (|state| state.rgb_range = Some(RgbRange::Full), "RGB range"),
        ];
        for (edit, what) in edits {
            let mut layout = snapshot();
            edit(&mut layout.entries[2].state);
            assert!(refused(&layout).starts_with(what), "{what}");
        }
    }

    #[test]
    fn an_output_kwin_cannot_turn_off_is_refused_from_version_25() {
        let mut devices = devices();
        devices[2].capabilities = CAPABILITY_VRR;
        let mut layout = snapshot();
        layout.entries[2].state.enabled = false;
        assert!(matches!(
            plan(&layout, &devices, Some(&order()), V22, &[]),
            Err(PlanError::Unsupported { what, .. }) if what.starts_with("turning it off")
        ));
        devices[2].capabilities |= CAPABILITY_DISABLE;
        let sent = plan(&layout, &devices, Some(&order()), V22, &[]).expect("plans");
        assert_eq!(sent.devices[2], DeviceConfig::Disable);
        // Older devices cannot say, so KWin is left to answer.
        devices[2].capabilities = 0;
        devices[2].version = 24;
        assert!(plan(&layout, &devices, Some(&order()), V22, &[]).is_ok());
    }

    #[test]
    fn a_layout_that_does_not_fit_the_devices_is_refused() {
        let mut twice = snapshot();
        twice.entries.push(twice.entries[1].clone());
        assert_eq!(planned(&twice), Err(PlanError::Twice("HDMI-A-1".into())));

        let mut no_mode = snapshot();
        no_mode.entries[1].state.mode = None;
        assert_eq!(planned(&no_mode), Err(PlanError::NoMode("HDMI-A-1".into())));

        let mut bad_scale = snapshot();
        bad_scale.entries[1].state.scale = 0.0;
        assert!(matches!(planned(&bad_scale), Err(PlanError::Scale { .. })));
    }
}
