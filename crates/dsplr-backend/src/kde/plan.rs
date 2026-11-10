//! A [`Layout`] as the requests of one `kde_output_configuration_v2`: what
//! each output device is set to, and the output order, decided before
//! anything is sent.

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, RgbRange};

use super::devices::{
    CAPABILITY_DISABLE, CAPABILITY_OVERSCAN, CAPABILITY_RGB_RANGE, CAPABILITY_VRR, Device,
    RGB_AUTOMATIC, RGB_FULL, RGB_LIMITED, VRR_ALWAYS, VRR_AUTOMATIC, VRR_NEVER,
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

/// The first device version whose capabilities say whether it can be
/// turned off ([`CAPABILITY_DISABLE`]).
pub const DISABLE_CAPABILITY_SINCE: u32 = 25;

/// KWin keeps a scale in 120ths: `kde_output_configuration_v2_scale` rounds
/// the `wl_fixed` it is sent to the nearest 120th (KWin 6.8's
/// `src/wayland/outputmanagement_v2.cpp`), and the device sends that back as
/// a `wl_fixed` again.
pub const KWIN_SCALE_STEPS: u32 = 120;

/// The largest overscan KWin takes, in percent: it ignores a request for
/// more (`kde_output_configuration_v2_overscan` in KWin 6.8's
/// `src/wayland/outputmanagement_v2.cpp`).
pub const MAX_OVERSCAN: u8 = 100;

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
    /// of a listed mode: KWin's protocol takes no other.
    pub mode: u64,
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
}

/// The configuration for `layout`, against `devices` as KWin last described
/// them, its output `order` (`None` when KWin offers no
/// `kde_output_order_v1`), for a `kde_output_management_v2` of `version`.
///
/// Entries name devices by connector. A setting KWin's protocol cannot
/// express (a custom mode, a colour preset, bit depth), one the device says
/// it does not have, and one the bound version cannot send are refused
/// rather than dropped.
pub fn plan(
    layout: &Layout,
    devices: &[Device],
    order: Option<&[String]>,
    version: u32,
) -> Result<Plan, PlanError> {
    let heads: Vec<Head> = devices.iter().map(|device| device.head.clone()).collect();
    let assigned = assign(layout, &heads)?;
    let configs = assigned
        .iter()
        .zip(devices)
        .map(|(entry, device)| match entry {
            Some(entry) => config(entry, device, devices, version),
            None => Ok(DeviceConfig::Unchanged),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let priorities = priorities(&assigned, devices, order, version)?;
    Ok(Plan {
        devices: configs,
        priorities,
    })
}

fn unsupported(device: &Device, what: &'static str) -> PlanError {
    PlanError::Unsupported {
        connector: device.head.name.clone(),
        what,
        protocol: PROTOCOL,
    }
}

fn config(
    entry: &OutputEntry,
    device: &Device,
    devices: &[Device],
    version: u32,
) -> Result<DeviceConfig, PlanError> {
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
        return Ok(DeviceConfig::Disable);
    }

    let mode = state
        .mode
        .as_ref()
        .ok_or_else(|| PlanError::NoMode(device.head.name.clone()))?;
    let mode = match choose_mode(&device.head, mode)? {
        ModeChoice::Listed(handle) => handle,
        ModeChoice::Custom { .. } => {
            return Err(unsupported(
                "a mode the output does not list (custom modes are not written for KDE yet)",
            ));
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
        Some(percent) if percent > MAX_OVERSCAN => {
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
    Ok(DeviceConfig::Enable(Settings {
        mode,
        position: (state.x, state.y),
        transform: i32::try_from(state.transform.to_wayland())
            .expect("wl_output.transform values are 0 to 7"),
        scale,
        overscan,
        vrr_policy,
        rgb_range,
        replication_source,
    }))
}

/// Whether `id`, a mirror source, names `device`: its identity, or its
/// connector.
fn names(id: &OutputId, device: &Device) -> bool {
    *id == device.id()
        || matches!(id, OutputId::Connector(connector) if *connector == device.head.name)
}

/// Each device's `set_priority`, when the layout names a primary output.
fn priorities(
    assigned: &[Option<&OutputEntry>],
    devices: &[Device],
    order: Option<&[String]>,
    version: u32,
) -> Result<Vec<Option<u32>>, PlanError> {
    let mut primaries = assigned
        .iter()
        .zip(devices)
        .filter(|(entry, _)| entry.is_some_and(|entry| entry.state.primary));
    let Some((primary_entry, primary)) = primaries.next() else {
        return Ok(vec![None; devices.len()]);
    };
    if let Some((_, second)) = primaries.next() {
        return Err(unsupported(second, "a second primary output"));
    }
    if !primary_entry.is_some_and(|entry| entry.state.enabled) {
        return Err(unsupported(primary, "a primary output that is off"));
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
    use super::super::devices::{CAPABILITY_DISABLE, CAPABILITY_VRR, VRR_AUTOMATIC, layout};
    use super::{DeviceConfig, Plan, Settings, plan};
    use crate::wayland::plan::PlanError;

    const V22: u32 = 22;

    fn snapshot() -> Layout {
        layout(&devices(), Some(&order())).expect("converts")
    }

    fn planned(layout: &Layout) -> Result<Plan, PlanError> {
        plan(layout, &devices(), Some(&order()), V22)
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
                mode: 0,
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
                mode: 1,
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
    fn a_primary_output_that_cannot_be_set_is_refused() {
        let mut two = snapshot();
        two.entries[2].state.primary = true;
        assert_eq!(refused(&two), "a second primary output");

        let mut off = snapshot();
        off.entries[1].state.primary = false;
        off.entries[0].state.primary = true;
        assert_eq!(refused(&off), "a primary output that is off");

        let primary = snapshot();
        assert!(matches!(
            plan(&primary, &devices(), None, V22),
            Err(PlanError::Unsupported { what, .. }) if what.contains("kde_output_order_v1")
        ));
        assert!(matches!(
            plan(&primary, &devices(), Some(&order()), 2),
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
            plan(&layout, &devices(), Some(&order()), 12),
            Err(PlanError::Unsupported { what, .. }) if what.starts_with("mirroring")
        ));
        layout.entries[2].state.mirror_of = None;
        let old = plan(&layout, &devices(), Some(&order()), 12).expect("plans");
        assert_eq!(settings(&old.devices[2]).replication_source, None);
    }

    #[test]
    fn what_kwin_cannot_take_is_refused_not_dropped() {
        let mut custom = snapshot();
        custom.entries[1].state.mode = Some(Mode::new(1234, 567, 60_000));
        assert!(refused(&custom).starts_with("a mode the output does not list"));

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
            plan(&layout, &devices, Some(&order()), V22),
            Err(PlanError::Unsupported { what, .. }) if what.starts_with("turning it off")
        ));
        devices[2].capabilities |= CAPABILITY_DISABLE;
        let sent = plan(&layout, &devices, Some(&order()), V22).expect("plans");
        assert_eq!(sent.devices[2], DeviceConfig::Disable);
        // Older devices cannot say, so KWin is left to answer.
        devices[2].capabilities = 0;
        devices[2].version = 24;
        assert!(plan(&layout, &devices, Some(&order()), V22).is_ok());
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
