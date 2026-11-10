//! KWin's output devices (`kde_output_device_v2`) as plain data, and those
//! as dsplr's [`Layout`]: the part a wlroots head has too through
//! [`heads::entry`], then KDE's own settings over it.

use dsplr_model::{AdaptiveSync, Layout, OutputEntry, OutputId, RgbRange};

use crate::wayland::heads::{self, Head, SnapshotError};

/// `capability.overscan`.
pub const CAPABILITY_OVERSCAN: u32 = 0x1;
/// `capability.vrr`.
pub const CAPABILITY_VRR: u32 = 0x2;
/// `capability.rgb_range`.
pub const CAPABILITY_RGB_RANGE: u32 = 0x4;
/// `capability.disable`, which devices send from version 25 on.
pub const CAPABILITY_DISABLE: u32 = 0x2_0000;
/// `capability.custom_modes`, which devices send from version 18 on.
pub const CAPABILITY_CUSTOM_MODES: u32 = 0x2000;

/// `kde_output_device_mode_v2.flags.custom`: a mode a custom mode list
/// added (`kde_output_configuration_v2.set_custom_modes`).
pub const MODE_CUSTOM: u32 = 0x1;
/// `kde_output_device_mode_v2.flags.reduced_blanking`.
pub const MODE_REDUCED_BLANKING: u32 = 0x2;

/// `vrr_policy.never`.
pub const VRR_NEVER: u32 = 0;
/// `vrr_policy.always`.
pub const VRR_ALWAYS: u32 = 1;
/// `vrr_policy.automatic`.
pub const VRR_AUTOMATIC: u32 = 2;

/// `rgb_range.automatic`.
pub const RGB_AUTOMATIC: u32 = 0;
/// `rgb_range.full`.
pub const RGB_FULL: u32 = 1;
/// `rgb_range.limited`.
pub const RGB_LIMITED: u32 = 2;

/// One output device as KWin described it, up to its last `done`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// What a wlroots head would say too: the connector name (`name`), make,
    /// model and serial, physical size, whether it is enabled, its modes and
    /// the current one, position, transform and scale. KWin sends no
    /// description, so that stays empty, and adaptive sync is
    /// [`Device::vrr_policy`] instead.
    pub head: Head,
    /// KWin's own identity for the output, which mirroring names its
    /// source by.
    pub uuid: String,
    /// The `capability` flags.
    pub capabilities: u32,
    /// Overscan in percent.
    pub overscan: u32,
    /// A `vrr_policy` value.
    pub vrr_policy: u32,
    /// An `rgb_range` value.
    pub rgb_range: u32,
    /// The [`Device::uuid`] of the output this one mirrors; empty when it
    /// mirrors none.
    pub replication_source: String,
    /// The version the device was bound at, which says which capability
    /// flags it can send.
    pub version: u32,
    /// What its modes' `flags` and `cvt` events said, by mode handle:
    /// what a custom mode list is rebuilt from ([`Device::custom_modes`]).
    pub mode_extras: Vec<ModeExtras>,
}

/// A mode's `flags` and `cvt` timings, which a [`HeadMode`] has no place
/// for.
///
/// [`HeadMode`]: crate::wayland::heads::HeadMode
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeExtras {
    /// The mode's handle on its device.
    pub handle: u64,
    /// `kde_output_device_mode_v2.flags` (since device version 19).
    pub flags: u32,
    /// The `cvt` event's timings, in its argument order (since device
    /// version 24), when KWin sent them.
    pub cvt: Option<[u32; 12]>,
}

/// One mode of a custom mode list, as `kde_mode_list_v2` adds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListedMode {
    /// `set_resolution`, `set_refresh_rate` and `set_reduced_blanking`,
    /// then `add_mode`.
    Mode {
        width: u32,
        height: u32,
        refresh_mhz: u32,
        reduced_blanking: bool,
    },
    /// `add_cvt` with these timings, in its argument order.
    Cvt([u32; 12]),
}

impl Device {
    /// The identity of the output, as its layout entry has it.
    pub fn id(&self) -> OutputId {
        let head = &self.head;
        OutputId::new(&head.make, &head.model, &head.serial_number, &head.name)
    }

    /// Whether it has the `capability` flag `flag`.
    pub const fn can(&self, flag: u32) -> bool {
        self.capabilities & flag != 0
    }

    /// The custom modes KWin lists for it, as a mode list that adds them
    /// again: by their CVT timings where KWin gave them and `cvt` is true
    /// (the management version has `add_cvt`), else by size, refresh and
    /// reduced blanking. `set_custom_modes` replaces the whole list, so
    /// these go into any list sent for it, or KWin would delete them.
    pub fn custom_modes(&self, cvt: bool) -> Vec<ListedMode> {
        self.head
            .modes
            .iter()
            .filter_map(|mode| {
                let extras = self
                    .mode_extras
                    .iter()
                    .find(|extras| extras.handle == mode.handle)?;
                if extras.flags & MODE_CUSTOM == 0 {
                    return None;
                }
                if cvt && let Some(timings) = extras.cvt {
                    return Some(ListedMode::Cvt(timings));
                }
                Some(ListedMode::Mode {
                    width: u32::try_from(mode.width).ok()?,
                    height: u32::try_from(mode.height).ok()?,
                    refresh_mhz: u32::try_from(mode.refresh_mhz).ok()?,
                    reduced_blanking: extras.flags & MODE_REDUCED_BLANKING != 0,
                })
            })
            .collect()
    }
}

/// The devices as a [`Layout`], one entry per device, disabled ones
/// included. `order` is `kde_output_order_v1`'s list of output names, when
/// KWin offers it: the first is the primary output.
///
/// A disabled device has no mode, sits at the origin and has none of KDE's
/// settings, as a disabled wlroots head ([`heads::layout`]). An enabled one
/// has adaptive sync, overscan and RGB range only where its capabilities
/// say it has them, and mirrors another only when that one is enabled,
/// which is when KWin mirrors it (`OutputConfigurationStore::applyMirroring`
/// in KWin 6.8's `src/outputconfigurationstore.cpp`).
pub fn layout(devices: &[Device], order: Option<&[String]>) -> Result<Layout, SnapshotError> {
    let entries = devices
        .iter()
        .map(|device| entry(device, devices, order))
        .collect::<Result<_, _>>()?;
    Ok(Layout {
        entries,
        ..Default::default()
    })
}

fn entry(
    device: &Device,
    devices: &[Device],
    order: Option<&[String]>,
) -> Result<OutputEntry, SnapshotError> {
    let mut entry = heads::entry(&device.head)?;
    if !device.head.enabled {
        return Ok(entry);
    }
    let connector = || device.head.name.clone();
    let state = &mut entry.state;
    if device.can(CAPABILITY_VRR) {
        state.adaptive_sync = Some(match device.vrr_policy {
            VRR_NEVER => AdaptiveSync::Off,
            VRR_ALWAYS => AdaptiveSync::On,
            VRR_AUTOMATIC => AdaptiveSync::Automatic,
            value => {
                return Err(SnapshotError::VrrPolicy {
                    connector: connector(),
                    value,
                });
            }
        });
    }
    if device.can(CAPABILITY_OVERSCAN) {
        state.overscan =
            Some(
                u8::try_from(device.overscan).map_err(|_| SnapshotError::Overscan {
                    connector: connector(),
                    value: device.overscan,
                })?,
            );
    }
    if device.can(CAPABILITY_RGB_RANGE) {
        state.rgb_range = Some(match device.rgb_range {
            RGB_AUTOMATIC => RgbRange::Automatic,
            RGB_FULL => RgbRange::Full,
            RGB_LIMITED => RgbRange::Limited,
            value => {
                return Err(SnapshotError::RgbRange {
                    connector: connector(),
                    value,
                });
            }
        });
    }
    if !device.replication_source.is_empty() {
        state.mirror_of = devices
            .iter()
            .find(|source| {
                source.uuid == device.replication_source
                    && source.head.enabled
                    && source.uuid != device.uuid
            })
            .map(Device::id);
    }
    state.primary = order
        .and_then(<[String]>::first)
        .is_some_and(|first| *first == device.head.name);
    Ok(entry)
}

#[cfg(test)]
pub mod tests {
    use dsplr_model::{AdaptiveSync, Mode, OutputId, RgbRange};

    use super::{
        CAPABILITY_DISABLE, CAPABILITY_OVERSCAN, CAPABILITY_RGB_RANGE, CAPABILITY_VRR, Device,
        RGB_LIMITED, VRR_AUTOMATIC, layout,
    };
    use crate::wayland::heads::tests::owner;
    use crate::wayland::heads::{Head, HeadMode, SnapshotError};

    /// The owner's two monitors as KWin would describe them (the wlr tests'
    /// heads, which say nothing of adaptive sync, as KWin does not), with a
    /// third, a virtual output like `kwin_wayland --virtual`'s: no EDID, and
    /// no capability but being turned off.
    pub fn devices() -> Vec<Device> {
        let mut heads = owner();
        for head in &mut heads {
            head.adaptive_sync = None;
            head.description = String::new();
        }
        let [panel, aoc] = <[Head; 2]>::try_from(heads).expect("two heads");
        let virtual_output = Head {
            name: "Virtual-0".into(),
            enabled: true,
            modes: vec![HeadMode {
                handle: 0,
                width: 1920,
                height: 1080,
                refresh_mhz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            position: (2560, 0),
            ..Head::default()
        };
        let all = CAPABILITY_OVERSCAN | CAPABILITY_VRR | CAPABILITY_RGB_RANGE | CAPABILITY_DISABLE;
        vec![
            Device {
                head: panel,
                uuid: "panel".into(),
                capabilities: all,
                version: 26,
                ..Device::default()
            },
            Device {
                head: aoc,
                uuid: "aoc".into(),
                capabilities: all,
                version: 26,
                ..Device::default()
            },
            Device {
                head: virtual_output,
                uuid: "virtual".into(),
                capabilities: CAPABILITY_DISABLE,
                version: 26,
                ..Device::default()
            },
        ]
    }

    /// The order KWin announces: the AOC first, then the virtual output.
    pub fn order() -> Vec<String> {
        vec!["HDMI-A-1".into(), "Virtual-0".into()]
    }

    #[test]
    fn the_devices_become_a_layout_with_kde_s_settings() {
        let mut devices = devices();
        devices[1].vrr_policy = VRR_AUTOMATIC;
        devices[1].overscan = 3;
        devices[1].rgb_range = RGB_LIMITED;
        let layout = layout(&devices, Some(&order())).expect("converts");
        assert_eq!(layout.entries.len(), 3);

        let aoc = &layout.entries[1];
        assert!(aoc.state.primary, "first in the output order");
        assert_eq!(aoc.state.adaptive_sync, Some(AdaptiveSync::Automatic));
        assert_eq!(aoc.state.overscan, Some(3));
        assert_eq!(aoc.state.rgb_range, Some(RgbRange::Limited));
        assert_eq!(aoc.output.as_ref().expect("live").description, "");
        assert!(
            aoc.state
                .mode
                .as_ref()
                .expect("a mode")
                .matches(&Mode::new(2560, 1440, 120_000))
        );

        let virtual_output = &layout.entries[2];
        assert_eq!(virtual_output.id, OutputId::Connector("Virtual-0".into()));
        assert!(!virtual_output.state.primary);
        assert_eq!((virtual_output.state.x, virtual_output.state.y), (2560, 0));
        // No capability, no setting: not a guess at KWin's default.
        assert_eq!(virtual_output.state.adaptive_sync, None);
        assert_eq!(virtual_output.state.overscan, None);
        assert_eq!(virtual_output.state.rgb_range, None);

        let panel = &layout.entries[0];
        assert!(!panel.state.enabled);
        assert_eq!(panel.state.overscan, None, "a disabled device has none");
    }

    #[test]
    fn without_the_output_order_no_output_is_primary() {
        let layout = layout(&devices(), None).expect("converts");
        assert!(layout.entries.iter().all(|entry| !entry.state.primary));
    }

    #[test]
    fn a_mirror_names_its_enabled_source() {
        let mut devices = devices();
        devices[2].replication_source = "aoc".into();
        let mirrored = layout(&devices, Some(&order())).expect("converts");
        assert_eq!(
            mirrored.entries[2].state.mirror_of,
            Some(mirrored.entries[1].id.clone())
        );

        // A source that is off, gone, or the output itself is no mirror.
        for source in ["panel", "gone", "virtual"] {
            devices[2].replication_source = source.into();
            let layout = layout(&devices, Some(&order())).expect("converts");
            assert_eq!(layout.entries[2].state.mirror_of, None, "{source}");
        }
    }

    #[test]
    fn values_the_protocol_does_not_define_are_errors_naming_the_device() {
        let mut bad = devices();
        bad[1].vrr_policy = 3;
        assert_eq!(
            layout(&bad, None),
            Err(SnapshotError::VrrPolicy {
                connector: "HDMI-A-1".into(),
                value: 3
            })
        );
        let mut bad = devices();
        bad[1].rgb_range = 9;
        assert!(matches!(
            layout(&bad, None),
            Err(SnapshotError::RgbRange { value: 9, .. })
        ));
        let mut bad = devices();
        bad[1].overscan = 256;
        assert!(matches!(
            layout(&bad, None),
            Err(SnapshotError::Overscan { value: 256, .. })
        ));
    }
}
