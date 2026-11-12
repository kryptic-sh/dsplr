//! The X server's RandR state ([`Snapshot`]) as the model's [`Layout`]:
//! what `dsplr list`, the GUI and the apply flow read on X11.
//!
//! - **Which outputs:** every output a monitor is (or may be) connected to,
//!   and any other a CRTC still drives, in the server's order. An output
//!   reported disconnected and off is left out, as the Wayland backends
//!   leave out what is not connected.
//! - **Identity:** the `EDID` property ([`dsplr_model::edid`]), by
//!   [`Edid::output_id`] with the RandR output's name as the connector: the
//!   monitor's make, model and serial when it has a serial, else the
//!   connector. An EDID that cannot be read names it by the connector too.
//! - **Modes:** each mode the output lists, its refresh from the timings as
//!   xrandr's `mode_refresh` gives it ([`ModeInfo::refresh_mhz`]), the first
//!   `num_preferred` preferred, each carrying its mode XID as its handle.
//! - **State:** a CRTC with a mode is on: that mode, the CRTC's position,
//!   and its rotation and reflection as a [`Transform`]
//!   ([`super::plan::transform`]). Scale is 1: X11 has none per output. The
//!   output [`Snapshot::primary`] names is primary. Adaptive sync is `None`:
//!   the driver's `vrr_capable` property says whether it could be, not what
//!   is set, and the model has no place for a read-only fact.
//! - **Mirrors:** of outputs sharing a CRTC, or on CRTCs at one place (as
//!   xrandr's `--same-as` places two CRTCs), each mirrors the first of them
//!   in the server's order.
//! - **Physical size:** the output's `mm_width` and `mm_height`, when both
//!   are given.
//!
//! A CRTC's transform (`xrandr --scale`) has no place in the model either;
//! the planner refuses to plan over one ([`super::PlanError::Transformed`]).

use dsplr_model::edid::Edid;
use dsplr_model::{Layout, Mode, ModeHandle, Output, OutputEntry, OutputId, OutputState, PnpNames};

use super::plan::transform;
use super::resources::{Connection, CrtcInfo, ModeInfo, ModeXid, OutputInfo, Snapshot};

/// Why a snapshot cannot be read as a layout.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    /// The server's replies name a CRTC or mode they do not describe.
    #[error("the X server's RandR state is inconsistent: {0}")]
    Inconsistent(String),
    /// A CRTC's rotation is none of the four.
    #[error("{output}: its CRTC's rotation bits {bits:#x} are no rotation")]
    Rotation { output: String, bits: u16 },
}

/// `snapshot` as a layout, each monitor's make its EDID's PNP code as
/// `names` gives it, as wlroots and Hyprland name it.
///
/// # Errors
///
/// When the snapshot names a CRTC or mode it does not describe, or a CRTC's
/// rotation is none of the four.
pub fn layout(snapshot: &Snapshot, names: &PnpNames) -> Result<Layout, SnapshotError> {
    let mut entries: Vec<OutputEntry> = Vec::new();
    // Each entry's CRTC, where it has one with a mode.
    let mut on: Vec<Option<&CrtcInfo>> = Vec::new();
    for output in &snapshot.outputs {
        let crtc = output
            .crtc
            .map(|xid| {
                snapshot
                    .crtcs
                    .iter()
                    .find(|crtc| crtc.id == xid)
                    .ok_or_else(|| inconsistent(format!("no CRTC 0x{:x}", xid.0)))
            })
            .transpose()?
            .filter(|crtc| crtc.mode.is_some());
        if output.connection == Connection::Disconnected && crtc.is_none() {
            continue;
        }
        entries.push(entry(snapshot, output, crtc, names)?);
        on.push(crtc);
    }

    // Mirrors: a second output on one CRTC, or a CRTC where an earlier is.
    for index in 0..entries.len() {
        let Some(crtc) = on[index] else {
            continue;
        };
        let source = (0..index).find(|&earlier| {
            entries[earlier].state.mirror_of.is_none()
                && on[earlier].is_some_and(|other| {
                    other.id == crtc.id || (other.x, other.y) == (crtc.x, crtc.y)
                })
        });
        if let Some(source) = source {
            entries[index].state.mirror_of = Some(entries[source].id.clone());
        }
    }
    Ok(Layout {
        entries,
        ..Layout::default()
    })
}

fn inconsistent(what: String) -> SnapshotError {
    SnapshotError::Inconsistent(what)
}

/// `output` as an entry, on `crtc` when that drives it with a mode.
fn entry(
    snapshot: &Snapshot,
    output: &OutputInfo,
    crtc: Option<&CrtcInfo>,
    names: &PnpNames,
) -> Result<OutputEntry, SnapshotError> {
    let mode_info = |xid: ModeXid| {
        snapshot
            .resources
            .modes
            .iter()
            .find(|mode| mode.id == xid)
            .ok_or_else(|| inconsistent(format!("no mode 0x{:x}", xid.0)))
    };
    let modes = output
        .modes
        .iter()
        .enumerate()
        .map(|(index, &xid)| {
            let preferred = index < usize::from(output.num_preferred);
            Ok(model_mode(mode_info(xid)?, preferred))
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;

    let state = match crtc {
        Some(crtc) => {
            let current = crtc.mode.expect("filtered to CRTCs with a mode");
            let preferred = modes
                .iter()
                .find(|mode| mode.handle == Some(ModeHandle(u64::from(current.0))))
                .is_some_and(|mode| mode.preferred);
            OutputState {
                enabled: true,
                mode: Some(model_mode(mode_info(current)?, preferred)),
                x: i32::from(crtc.x),
                y: i32::from(crtc.y),
                transform: transform(crtc.rotation).ok_or_else(|| SnapshotError::Rotation {
                    output: output.name.clone(),
                    bits: crtc.rotation.0,
                })?,
                primary: snapshot.primary == Some(output.id),
                ..OutputState::default()
            }
        }
        None => OutputState {
            enabled: false,
            ..OutputState::default()
        },
    };

    let edid = output
        .edid
        .as_deref()
        .and_then(|bytes| Edid::parse(bytes).ok());
    let (id, make, model, serial) = match &edid {
        Some(edid) => {
            let identity = edid.identity(names);
            (
                edid.output_id(&output.name, names),
                identity.make.unwrap_or_default(),
                identity.model,
                identity.serial.unwrap_or_default(),
            )
        }
        None => (
            OutputId::Connector(output.name.clone()),
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    let description = [make.as_str(), model.as_str(), serial.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let physical_size_mm = (output.mm_width > 0 && output.mm_height > 0)
        .then_some((output.mm_width, output.mm_height));
    Ok(OutputEntry {
        id,
        output: Some(Output {
            connector: output.name.clone(),
            description: if description.is_empty() {
                output.name.clone()
            } else {
                description
            },
            make,
            model,
            serial,
            physical_size_mm,
            modes,
        }),
        state,
    })
}

/// The model's form of `mode`, carrying its XID.
fn model_mode(mode: &ModeInfo, preferred: bool) -> Mode {
    Mode {
        preferred,
        handle: Some(ModeHandle(u64::from(mode.id.0))),
        ..Mode::new(
            u32::from(mode.width),
            u32::from(mode.height),
            mode.refresh_mhz(),
        )
    }
}
