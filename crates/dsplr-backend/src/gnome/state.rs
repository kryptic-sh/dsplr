//! `GetCurrentState`'s reply as plain data ([`State`]), and that as dsplr's
//! [`Layout`]: one entry per monitor, the first monitor of each logical
//! monitor placed and the others mirroring it.

use dsplr_model::{
    Layout, LayoutMode, Mode, ModeHandle, Output, OutputEntry, OutputId, OutputState, PnpNames,
    RgbRange, Transform, hz_to_mhz,
};
use zbus::OwnedVariant;

use super::bus::{
    CurrentState, MonitorSpec, Properties, WireLogicalMonitor, WireMode, WireMonitor,
};

/// What Mutter sends for a vendor, product or serial it does not know
/// (`meta_monitor_generate_spec` in Mutter 51.1's
/// `src/backends/meta-monitor.c`). dsplr reads it as not reported.
const UNKNOWN: &str = "unknown";

/// `layout-mode`'s value for the logical mode.
pub const LAYOUT_MODE_LOGICAL: u32 = 1;

/// `layout-mode`'s value for the physical mode.
pub const LAYOUT_MODE_PHYSICAL: u32 = 2;

/// The display configuration Mutter described, up to its serial.
#[derive(Debug, PartialEq)]
pub struct State {
    /// What `ApplyMonitorsConfig` must be given back, or it refuses the
    /// configuration as stale.
    pub serial: u32,
    /// Every connected monitor, in Mutter's order.
    pub monitors: Vec<Monitor>,
    /// The monitors that are on, grouped as Mutter lays them out.
    pub logical_monitors: Vec<LogicalMonitor>,
    pub layout_mode: LayoutMode,
    /// Whether `ApplyMonitorsConfig` may set `layout-mode`.
    pub supports_changing_layout_mode: bool,
    /// Whether every logical monitor must have the same scale.
    pub global_scale_required: bool,
}

/// One connected monitor.
#[derive(Debug, PartialEq)]
pub struct Monitor {
    pub connector: String,
    /// The make as wlroots and Hyprland name the monitor: Mutter's vendor,
    /// the EDID's PNP code, as a company name ([`PnpNames::make`]), so its
    /// identity matches theirs. Empty when Mutter does not know it.
    pub make: String,
    /// Empty when Mutter does not know it.
    pub product: String,
    /// Empty when Mutter does not know it.
    pub serial: String,
    /// Mutter's name for it, such as `Dell 27"`, when it gave one.
    pub display_name: Option<String>,
    /// Width and height in millimetres, when Mutter gave both.
    pub physical_size_mm: Option<(u32, u32)>,
    /// The modes Mutter advertises, in its order.
    pub modes: Vec<MonitorMode>,
    /// Its `rgb-range`, when Mutter reported one it knows (`1` automatic,
    /// `2` full, `3` limited).
    pub rgb_range: Option<u32>,
    /// The properties an apply leaves as they are unless asked
    /// (`underscanning`, `color-mode`), carried back unchanged, since
    /// Mutter resets one an apply leaves out.
    pub kept: Vec<(&'static str, OwnedVariant)>,
}

/// One mode of a monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorMode {
    /// Mutter's id for it, which an apply names.
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
    /// The scales Mutter takes with this mode, in the current layout mode.
    pub scales: Vec<f64>,
    pub current: bool,
    pub preferred: bool,
}

/// A logical monitor: one place on the screen, showing one or more
/// monitors (more than one is mirroring).
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalMonitor {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// A transform as `wl_output` numbers them, which Mutter's
    /// `MtkMonitorTransform` follows.
    pub transform: u32,
    pub primary: bool,
    /// The connectors of its monitors, in Mutter's order.
    pub connectors: Vec<String>,
}

/// A reply dsplr cannot represent.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SnapshotError {
    #[error("{connector}: mode {id:?} has a size or refresh rate that is not one")]
    Mode { connector: String, id: String },
    #[error("transform {0} is not one Mutter defines")]
    Transform(u32),
    #[error("layout mode {0} is not one Mutter defines")]
    LayoutMode(u32),
    #[error("{connector} is on, but Mutter named no current mode for it")]
    NoCurrentMode { connector: String },
    #[error("a logical monitor shows {connector}, which is not among the monitors")]
    UnknownMonitor { connector: String },
    #[error("a logical monitor shows no monitor")]
    EmptyLogicalMonitor,
    #[error("property {name:?} is not of the type Mutter documents")]
    Property { name: &'static str },
}

impl State {
    /// The state `GetCurrentState` replied, each monitor's make its vendor
    /// code as `names` gives it.
    pub fn from_wire(
        (serial, monitors, logical_monitors, properties): CurrentState,
        names: &PnpNames,
    ) -> Result<Self, SnapshotError> {
        // An absent `layout-mode` means the logical one, by the interface's
        // documentation.
        let layout_mode = match property::<u32>(&properties, "layout-mode")? {
            None | Some(LAYOUT_MODE_LOGICAL) => LayoutMode::Logical,
            Some(LAYOUT_MODE_PHYSICAL) => LayoutMode::Physical,
            Some(other) => return Err(SnapshotError::LayoutMode(other)),
        };
        Ok(Self {
            serial,
            monitors: monitors
                .into_iter()
                .map(|wire| monitor(wire, names))
                .collect::<Result<_, _>>()?,
            logical_monitors: logical_monitors.into_iter().map(logical_monitor).collect(),
            layout_mode,
            supports_changing_layout_mode: property(&properties, "supports-changing-layout-mode")?
                .unwrap_or(false),
            global_scale_required: property(&properties, "global-scale-required")?.unwrap_or(false),
        })
    }

    /// The monitor on `connector`.
    pub fn monitor(&self, connector: &str) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|monitor| monitor.connector == connector)
    }

    /// The state as a [`Layout`], one entry per monitor, the ones that are
    /// off included (off, without a mode, at the origin).
    ///
    /// The first monitor of a logical monitor is placed there; each other
    /// one mirrors it, with the logical monitor's position, scale and
    /// transform. The primary logical monitor makes its first monitor
    /// primary.
    pub fn layout(&self) -> Result<Layout, SnapshotError> {
        for logical in &self.logical_monitors {
            if logical.connectors.is_empty() {
                return Err(SnapshotError::EmptyLogicalMonitor);
            }
            if let Some(connector) = logical
                .connectors
                .iter()
                .find(|connector| self.monitor(connector).is_none())
            {
                return Err(SnapshotError::UnknownMonitor {
                    connector: connector.clone(),
                });
            }
        }
        let entries = self
            .monitors
            .iter()
            .map(|monitor| self.entry(monitor))
            .collect::<Result<_, _>>()?;
        Ok(Layout {
            entries,
            layout_mode: Some(self.layout_mode),
        })
    }

    fn entry(&self, monitor: &Monitor) -> Result<OutputEntry, SnapshotError> {
        let modes: Vec<Mode> = monitor
            .modes
            .iter()
            .zip(0..)
            .map(|(mode, index)| Mode {
                width: mode.width,
                height: mode.height,
                refresh_mhz: mode.refresh_mhz,
                preferred: mode.preferred,
                scales: mode.scales.clone(),
                handle: Some(ModeHandle(index)),
            })
            .collect();
        let shown = self.logical_monitors.iter().find_map(|logical| {
            let at = logical
                .connectors
                .iter()
                .position(|connector| *connector == monitor.connector)?;
            Some((logical, at))
        });
        let state = match shown {
            None => OutputState {
                enabled: false,
                ..OutputState::default()
            },
            Some((logical, at)) => {
                let mode = monitor
                    .modes
                    .iter()
                    .position(|mode| mode.current)
                    .map(|index| modes[index].clone())
                    .ok_or_else(|| SnapshotError::NoCurrentMode {
                        connector: monitor.connector.clone(),
                    })?;
                let mirror_of = match at {
                    0 => None,
                    _ => self.monitor(&logical.connectors[0]).map(Monitor::id),
                };
                OutputState {
                    enabled: true,
                    mode: Some(mode),
                    x: logical.x,
                    y: logical.y,
                    scale: logical.scale,
                    transform: Transform::from_wayland(logical.transform)
                        .ok_or(SnapshotError::Transform(logical.transform))?,
                    mirror_of,
                    primary: logical.primary && at == 0,
                    rgb_range: monitor.rgb_range.and_then(rgb_range),
                    ..OutputState::default()
                }
            }
        };
        Ok(OutputEntry {
            id: monitor.id(),
            output: Some(Output {
                connector: monitor.connector.clone(),
                description: monitor.display_name.clone().unwrap_or_else(|| {
                    format!("{} {}", monitor.make, monitor.product)
                        .trim()
                        .to_owned()
                }),
                make: monitor.make.clone(),
                model: monitor.product.clone(),
                serial: monitor.serial.clone(),
                physical_size_mm: monitor.physical_size_mm,
                modes,
            }),
            state,
        })
    }
}

impl Monitor {
    /// Its identity, by the model's rule: make, product and serial when
    /// Mutter knows the serial, else the connector.
    pub fn id(&self) -> OutputId {
        OutputId::new(&self.make, &self.product, &self.serial, &self.connector)
    }
}

/// The model's RGB range for Mutter's `rgb-range`; `None` for one Mutter
/// does not define there.
pub const fn rgb_range(value: u32) -> Option<RgbRange> {
    match value {
        1 => Some(RgbRange::Automatic),
        2 => Some(RgbRange::Full),
        3 => Some(RgbRange::Limited),
        _ => None,
    }
}

/// Mutter's `rgb-range` for the model's RGB range.
pub const fn rgb_range_value(range: RgbRange) -> u32 {
    match range {
        RgbRange::Automatic => 1,
        RgbRange::Full => 2,
        RgbRange::Limited => 3,
    }
}

/// A vendor, product or serial as dsplr keeps it: Mutter's placeholder for
/// one it does not know is empty.
fn known(text: String) -> String {
    if text == UNKNOWN { String::new() } else { text }
}

fn monitor(
    ((connector, vendor, product, serial), modes, properties): WireMonitor,
    names: &PnpNames,
) -> Result<Monitor, SnapshotError> {
    let modes = modes
        .into_iter()
        .map(|wire| monitor_mode(&connector, wire))
        .collect::<Result<_, _>>()?;
    let millimetres = |name| {
        property::<i32>(&properties, name)
            .map(|value| value.and_then(|value| u32::try_from(value).ok().filter(|&mm| mm > 0)))
    };
    let physical_size_mm = match (millimetres("width-mm")?, millimetres("height-mm")?) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    };
    let mut kept = Vec::new();
    for name in ["underscanning", "color-mode"] {
        // GetCurrentState calls it `is-underscanning`; an apply takes
        // `underscanning`.
        let read_as = if name == "underscanning" {
            "is-underscanning"
        } else {
            name
        };
        if let Some(value) = properties.get(read_as) {
            let value = value
                .try_clone()
                .map_err(|_| SnapshotError::Property { name: read_as })?;
            kept.push((name, value));
        }
    }
    Ok(Monitor {
        display_name: properties
            .get("display-name")
            .map(|value| {
                <&str>::try_from(value).map_err(|_| SnapshotError::Property {
                    name: "display-name",
                })
            })
            .transpose()?
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned),
        physical_size_mm,
        rgb_range: property::<u32>(&properties, "rgb-range")?
            .filter(|&value| rgb_range(value).is_some()),
        kept,
        modes,
        connector,
        make: names.make(&known(vendor)),
        product: known(product),
        serial: known(serial),
    })
}

fn monitor_mode(
    connector: &str,
    (id, width, height, refresh, _preferred_scale, scales, properties): WireMode,
) -> Result<MonitorMode, SnapshotError> {
    let invalid = || SnapshotError::Mode {
        connector: connector.to_owned(),
        id: id.clone(),
    };
    let width = u32::try_from(width).map_err(|_| invalid())?;
    let height = u32::try_from(height).map_err(|_| invalid())?;
    let refresh_mhz = hz_to_mhz(refresh).ok_or_else(invalid)?;
    Ok(MonitorMode {
        current: property(&properties, "is-current")?.unwrap_or(false),
        preferred: property(&properties, "is-preferred")?.unwrap_or(false),
        id,
        width,
        height,
        refresh_mhz,
        scales,
    })
}

fn logical_monitor(
    (x, y, scale, transform, primary, monitors, _properties): WireLogicalMonitor,
) -> LogicalMonitor {
    LogicalMonitor {
        x,
        y,
        scale,
        transform,
        primary,
        connectors: monitors
            .into_iter()
            .map(|(connector, ..): MonitorSpec| connector)
            .collect(),
    }
}

/// Property `name` of `properties` as a `T`: `None` when it is absent, an
/// error when it is there as another type.
fn property<T>(properties: &Properties, name: &'static str) -> Result<Option<T>, SnapshotError>
where
    T: for<'a> TryFrom<&'a OwnedVariant>,
{
    properties
        .get(name)
        .map(|value| T::try_from(value).map_err(|_| SnapshotError::Property { name }))
        .transpose()
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use dsplr_model::{LayoutMode, Mode, OutputId, PnpNames, RgbRange, Transform};
    use zbus::OwnedVariant;

    use super::{SnapshotError, State};
    use crate::gnome::bus::{CurrentState, Properties, WireLogicalMonitor, WireMode, WireMonitor};

    pub fn properties(pairs: &[(&str, OwnedVariant)]) -> Properties {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.try_clone().expect("clones")))
            .collect()
    }

    /// A mode as Mutter sends it.
    pub fn mode(id: &str, width: i32, height: i32, current: bool, scales: &[f64]) -> WireMode {
        let mut flags = Vec::new();
        if current {
            flags.push(("is-current", OwnedVariant::from(true)));
        }
        if id.ends_with("@60.000") {
            flags.push(("is-preferred", OwnedVariant::from(true)));
        }
        (
            id.to_owned(),
            width,
            height,
            60.0,
            1.0,
            scales.to_vec(),
            properties(&flags),
        )
    }

    pub fn monitor(connector: &str, serial: &str, modes: Vec<WireMode>) -> WireMonitor {
        (
            (
                connector.to_owned(),
                "GSM".to_owned(),
                "LG ULTRAGEAR".to_owned(),
                serial.to_owned(),
            ),
            modes,
            properties(&[
                ("width-mm", OwnedVariant::from(600)),
                ("height-mm", OwnedVariant::from(340)),
                ("is-builtin", OwnedVariant::from(false)),
                (
                    "display-name",
                    OwnedVariant::from(zbus::Str::from("LG 27\"")),
                ),
                ("color-mode", OwnedVariant::from(0_u32)),
                ("rgb-range", OwnedVariant::from(2_u32)),
            ]),
        )
    }

    pub fn logical(
        x: i32,
        scale: f64,
        transform: u32,
        primary: bool,
        connectors: &[&str],
    ) -> WireLogicalMonitor {
        (
            x,
            0,
            scale,
            transform,
            primary,
            connectors
                .iter()
                .map(|connector| {
                    (
                        (*connector).to_owned(),
                        "GSM".to_owned(),
                        "LG ULTRAGEAR".to_owned(),
                        "unknown".to_owned(),
                    )
                })
                .collect(),
            Properties::new(),
        )
    }

    /// Two monitors as Mutter would describe them: DP-1 with a serial, at
    /// 2560x1440 scaled 1.25 and primary; HDMI-1 without one, at 1920x1080
    /// beside it, its second mode current, in the logical mode.
    pub fn two_monitors() -> CurrentState {
        let scales = [1.0, 1.25, 1.5, 2.0];
        (
            7,
            vec![
                monitor(
                    "DP-1",
                    "SER1",
                    vec![
                        mode("2560x1440@60.000", 2560, 1440, true, &scales),
                        mode("1920x1080@60.000", 1920, 1080, false, &scales),
                    ],
                ),
                monitor(
                    "HDMI-1",
                    "unknown",
                    vec![
                        mode("1920x1080@59.940", 1920, 1080, false, &[1.0, 2.0]),
                        mode("1920x1080@60.000", 1920, 1080, true, &[1.0, 2.0]),
                    ],
                ),
            ],
            vec![
                logical(0, 1.25, 0, true, &["DP-1"]),
                logical(2048, 1.0, 1, false, &["HDMI-1"]),
            ],
            properties(&[
                ("layout-mode", OwnedVariant::from(1_u32)),
                ("supports-changing-layout-mode", OwnedVariant::from(true)),
            ]),
        )
    }

    #[test]
    fn a_monitor_s_make_is_its_vendor_code_s_company_name() {
        let names = PnpNames::parse("GSM\tGoldstar Company Ltd\n");
        let state = State::from_wire(two_monitors(), &names).expect("reads");
        assert_eq!(state.monitors[0].make, "Goldstar Company Ltd");
        let layout = state.layout().expect("converts");
        assert_eq!(
            layout.entries[0].id,
            OutputId::Edid {
                make: "Goldstar Company Ltd".into(),
                model: "LG ULTRAGEAR".into(),
                serial: "SER1".into()
            }
        );
    }

    #[test]
    fn a_reply_becomes_one_entry_per_monitor() {
        let state = State::from_wire(two_monitors(), &PnpNames::default()).expect("reads");
        assert_eq!(state.serial, 7);
        assert!(state.supports_changing_layout_mode);
        assert!(!state.global_scale_required);
        let layout = state.layout().expect("converts");
        assert_eq!(layout.layout_mode, Some(LayoutMode::Logical));
        assert_eq!(layout.entries.len(), 2);

        let dp = &layout.entries[0];
        assert_eq!(
            dp.id,
            OutputId::Edid {
                make: "GSM".into(),
                model: "LG ULTRAGEAR".into(),
                serial: "SER1".into()
            }
        );
        let output = dp.output.as_ref().expect("live");
        assert_eq!(output.connector, "DP-1");
        assert_eq!(output.description, "LG 27\"");
        assert_eq!(output.physical_size_mm, Some((600, 340)));
        assert_eq!(output.modes.len(), 2);
        assert!(output.modes[0].preferred && output.modes[1].preferred);
        assert_eq!(output.modes[0].scales, [1.0, 1.25, 1.5, 2.0]);
        let state = &dp.state;
        assert!(state.enabled && state.primary);
        assert!(
            state
                .mode
                .as_ref()
                .is_some_and(|mode| mode.matches(&Mode::new(2560, 1440, 60_000)))
        );
        assert!((state.scale - 1.25).abs() < f64::EPSILON);
        assert_eq!((state.x, state.y), (0, 0));
        assert_eq!(state.mirror_of, None);
        assert_eq!(state.rgb_range, Some(RgbRange::Full));
        assert_eq!(state.adaptive_sync, None);

        // Mutter's "unknown" serial is no serial: the connector names it.
        let hdmi = &layout.entries[1];
        assert_eq!(hdmi.id, OutputId::Connector("HDMI-1".into()));
        assert_eq!(hdmi.output.as_ref().expect("live").serial, "");
        assert_eq!(hdmi.state.transform, Transform::Rotate90);
        assert_eq!(hdmi.state.x, 2048);
        assert!(!hdmi.state.primary);
        // Its second mode is the current one, and keeps its ordinal.
        let mode = hdmi.state.mode.as_ref().expect("a mode");
        assert_eq!(mode.handle, Some(dsplr_model::ModeHandle(1)));
        assert_eq!(mode.refresh_mhz, 60_000);
    }

    #[test]
    fn a_monitor_in_no_logical_monitor_is_off() {
        let (serial, monitors, mut logical, properties) = two_monitors();
        logical.truncate(1);
        let layout = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads")
        .layout()
        .expect("converts");
        let hdmi = &layout.entries[1].state;
        assert!(!hdmi.enabled);
        assert_eq!(hdmi.mode, None);
    }

    #[test]
    fn the_monitors_after_the_first_of_a_logical_monitor_mirror_it() {
        let (serial, monitors, _, properties) = two_monitors();
        let logical = vec![super::tests::logical(0, 1.0, 0, true, &["HDMI-1", "DP-1"])];
        let layout = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads")
        .layout()
        .expect("converts");
        let [dp, hdmi] = [&layout.entries[0], &layout.entries[1]];
        assert_eq!(hdmi.state.mirror_of, None);
        assert!(hdmi.state.primary);
        assert_eq!(dp.state.mirror_of, Some(hdmi.id.clone()));
        assert!(!dp.state.primary, "one primary output, the source");
        assert!(dp.state.enabled);
    }

    #[test]
    fn the_physical_layout_mode_and_global_scale_are_read() {
        let (serial, monitors, logical, _) = two_monitors();
        let properties = properties(&[
            ("layout-mode", OwnedVariant::from(2_u32)),
            ("global-scale-required", OwnedVariant::from(true)),
        ]);
        let state = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads");
        assert_eq!(state.layout_mode, LayoutMode::Physical);
        assert!(state.global_scale_required);
        assert!(!state.supports_changing_layout_mode, "absent means no");
        assert_eq!(
            state.layout().expect("converts").layout_mode,
            Some(LayoutMode::Physical)
        );

        // Absent, the interface says the mode is the logical one.
        let (serial, monitors, logical, _) = two_monitors();
        let state = State::from_wire(
            (serial, monitors, logical, HashMap::new()),
            &PnpNames::default(),
        )
        .expect("reads");
        assert_eq!(state.layout_mode, LayoutMode::Logical);
    }

    #[test]
    fn what_dsplr_cannot_represent_is_an_error_naming_it() {
        let (serial, monitors, logical, _) = two_monitors();
        let odd = properties(&[("layout-mode", OwnedVariant::from(3_u32))]);
        assert_eq!(
            State::from_wire((serial, monitors, logical, odd), &PnpNames::default()),
            Err(SnapshotError::LayoutMode(3))
        );

        let (serial, monitors, logical, _) = two_monitors();
        let mistyped = properties(&[("layout-mode", OwnedVariant::from(true))]);
        assert_eq!(
            State::from_wire((serial, monitors, logical, mistyped), &PnpNames::default()),
            Err(SnapshotError::Property {
                name: "layout-mode"
            })
        );

        let (serial, monitors, mut logical, properties) = two_monitors();
        logical[1].3 = 8;
        let state = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads");
        assert_eq!(state.layout(), Err(SnapshotError::Transform(8)));

        let (serial, monitors, mut logical, properties) = two_monitors();
        logical[1].5[0].0 = "DP-9".into();
        let state = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads");
        assert_eq!(
            state.layout(),
            Err(SnapshotError::UnknownMonitor {
                connector: "DP-9".into()
            })
        );

        let (serial, mut monitors, logical, properties) = two_monitors();
        for mode in &mut monitors[0].1 {
            mode.6.remove("is-current");
        }
        let state = State::from_wire(
            (serial, monitors, logical, properties),
            &PnpNames::default(),
        )
        .expect("reads");
        assert_eq!(
            state.layout(),
            Err(SnapshotError::NoCurrentMode {
                connector: "DP-1".into()
            })
        );

        let (serial, mut monitors, logical, properties) = two_monitors();
        monitors[1].1[0].1 = -1;
        assert!(matches!(
            State::from_wire((serial, monitors, logical, properties), &PnpNames::default()),
            Err(SnapshotError::Mode { connector, .. }) if connector == "HDMI-1"
        ));
    }
}
