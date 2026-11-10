//! What turning a [`Layout`] into a Wayland output configuration shares,
//! whichever protocol carries it: matching each entry to the head on its
//! connector, picking the mode object a mode names, and why a layout cannot
//! be sent. Each backend's own `plan` builds its requests on these.

use dsplr_model::{Layout, Mode, ModeHandle, OutputEntry, OutputId};

use super::heads::{Head, HeadMode};

/// How a head's mode is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeChoice {
    /// The mode object of this [`HeadMode::handle`].
    Listed(u64),
    /// A mode no object of the head has.
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
    #[error("{connector}: {what} cannot be set through {protocol}")]
    Unsupported {
        connector: String,
        what: &'static str,
        /// The protocol the layout was to be sent through.
        protocol: &'static str,
    },
}

/// The entry of `layout` naming each of `heads`, in their order: `None` for
/// a head the layout does not name. Entries name heads by connector.
pub fn assign<'a>(
    layout: &'a Layout,
    heads: &[Head],
) -> Result<Vec<Option<&'a OutputEntry>>, PlanError> {
    let mut assigned = vec![None; heads.len()];
    for entry in &layout.entries {
        let index = entry
            .connector()
            .and_then(|connector| heads.iter().position(|head| head.name == connector))
            .ok_or_else(|| PlanError::NotConnected(entry.id.clone()))?;
        if assigned[index].is_some() {
            return Err(PlanError::Twice(heads[index].name.clone()));
        }
        assigned[index] = Some(entry);
    }
    Ok(assigned)
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
pub fn choose_mode(head: &Head, mode: &Mode) -> Result<ModeChoice, PlanError> {
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
