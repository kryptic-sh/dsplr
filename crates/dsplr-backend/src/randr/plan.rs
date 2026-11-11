//! A [`Layout`] as the RandR requests that set it, decided before anything
//! is sent.
//!
//! This is xrandr's own algorithm, transcribed from `xrandr.c` in xrandr
//! 1.5.4. xrandr's licence is an X11-style permission notice (HPND), not
//! MIT; it asks for the notice in copies, and it is in
//! `src/randr/LICENSES/xrandr.txt`, verbatim from the release's `COPYING`.
//! Each step names the function it follows. The layout plays the
//! part of xrandr's command line: every entry is an `--output` naming its
//! connector, in the layout's order, and the outputs it does not name follow
//! in the server's order, as xrandr's `get_outputs` appends them.
//!
//! Where dsplr departs from xrandr it says so at the step. In short: an
//! output counts as changed only when its mode, position, rotation or
//! on/off state differ from the server's (xrandr marks every output it is
//! told about); panning that no longer fits a CRTC's new mode or position is
//! reset rather than carried over, and noted ([`Note`]); a mode the output
//! does not list is created; and the primary output is set only when the
//! layout makes one primary and it is not already.

use dsplr_model::{Layout, Mode, ModeHandle, OutputEntry, OutputId, SCALE_EPSILON, Transform};

use super::cvt;
use super::resources::{
    CrtcInfo, CrtcXid, ModeInfo, ModeXid, OutputInfo, OutputXid, Panning, Rotation, Snapshot,
    mode_flags,
};

/// A mode as a request names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeRef {
    /// A mode the server has.
    Listed(ModeXid),
    /// The mode made by the plan's `n`th [`Operation::CreateMode`], counting
    /// from zero: the server gives its XID in `RRCreateMode`'s reply.
    Created(usize),
}

/// One RandR request, in the order they are to be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// `RRCreateMode`; its `id` is not sent.
    CreateMode(ModeInfo),
    /// `RRAddOutputMode`.
    AddOutputMode { output: OutputXid, mode: ModeRef },
    /// `GrabServer`, so clients reading the screen mid-change see either
    /// layout, not a mix (xrandr's `apply`).
    GrabServer,
    /// `RRSetCrtcConfig` with no mode and no outputs (xrandr's
    /// `crtc_disable`).
    DisableCrtc(CrtcXid),
    /// `RRSetScreenSize`.
    SetScreenSize {
        width: u16,
        height: u16,
        mm_width: u32,
        mm_height: u32,
    },
    /// `RRSetCrtcConfig`.
    SetCrtcConfig {
        crtc: CrtcXid,
        mode: ModeRef,
        x: i16,
        y: i16,
        rotation: Rotation,
        outputs: Vec<OutputXid>,
    },
    /// `RRSetPanning`. The panning a CRTC keeps is sent after its
    /// `RRSetCrtcConfig`, as xrandr's `crtc_apply` sends it. Panning that no
    /// longer fits is reset to none before the CRTC is turned off, or, when
    /// it is not, before its `RRSetCrtcConfig` ([`Note::PanningReset`]): Xorg
    /// pans a CRTC back into its panning area right after setting it, so a
    /// reset sent after would leave it where the old area put it.
    SetPanning { crtc: CrtcXid, panning: Panning },
    /// `RRSetOutputPrimary`.
    SetOutputPrimary(OutputXid),
    /// `UngrabServer`.
    UngrabServer,
}

/// The requests for a layout, and what they do that the layout did not ask
/// for, to tell the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub operations: Vec<Operation>,
    pub notes: Vec<Note>,
}

/// Something a plan does that its layout did not ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// The CRTC's panning no longer fits its new mode or position, so it is
    /// turned off before the CRTC is set; `outputs` are the connectors the
    /// CRTC is set to drive.
    PanningReset { crtc: CrtcXid, outputs: Vec<String> },
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PanningReset { outputs, .. } => {
                write!(f, "panning on {} reset", outputs.join(", "))
            }
        }
    }
}

/// Why a layout cannot be set. Nothing is sent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("{0}: no output is on its connector")]
    NotConnected(OutputId),
    #[error("{0} is in the layout more than once")]
    Twice(String),
    #[error("{0}: enabled without a mode")]
    NoMode(String),
    #[error("{connector}: {what} cannot be set on X11")]
    Unsupported {
        connector: String,
        what: &'static str,
    },
    #[error("{0}: the output it mirrors is not an enabled output of the layout")]
    MirrorSource(String),
    #[error("{connector}: its CRTCs cannot turn it {transform:?}")]
    Rotation {
        connector: String,
        transform: Transform,
    },
    #[error("{connector}: mode {mode} is not listed and cannot be made: {reason}")]
    ModeUnavailable {
        connector: String,
        mode: String,
        reason: &'static str,
    },
    #[error("{0}: no CRTC is free to drive it")]
    NoCrtc(String),
    /// `x`,`y` is the position the layout gives, before the layout is moved
    /// to the origin.
    #[error(
        "{connector}: position {x},{y} is too far from the other outputs for a CRTC to be placed at"
    )]
    Position { connector: String, x: i32, y: i32 },
    #[error(
        "the screen cannot be larger than {max_width}x{max_height} (the layout needs {width}x{height})"
    )]
    ScreenTooLarge {
        width: i64,
        height: i64,
        max_width: u16,
        max_height: u16,
    },
    /// The snapshot names a CRTC, output or mode it does not describe.
    #[error("the server's state is inconsistent: {0}")]
    Inconsistent(String),
    /// A CRTC on screen has a transform (`xrandr --scale`, `--transform`),
    /// which the planner neither sets nor resets, and which sizes the CRTC
    /// otherwise than its mode: planning over it would size the screen
    /// wrongly.
    #[error(
        "{} {} a RandR transform (xrandr --scale or --transform), which dsplr does not set; `xrandr --output NAME --transform none` takes it off",
        if outputs.is_empty() { "a CRTC driving no output".to_owned() } else { outputs.join(", ") },
        if outputs.len() > 1 { "have" } else { "has" }
    )]
    Transformed { crtc: CrtcXid, outputs: Vec<String> },
}

/// The requests that set `layout` on the server `snapshot` was read from.
///
/// Entries name outputs by connector; an output the layout does not name
/// keeps its state. The layout's scale must be 1 (X11 has no per-output
/// scale), and colour preset, bit depth, overscan, RGB range and adaptive
/// sync are refused rather than dropped: the apply flow drops their neutral
/// values first.
pub fn plan(snapshot: &Snapshot, layout: &Layout) -> Result<Plan, PlanError> {
    let mut planner = Planner::new(snapshot)?;
    planner.read_outputs(layout)?;
    planner.set_positions()?;
    planner.pick_crtcs()?;
    planner.set_crtcs()?;
    planner.mark_changing_crtcs()?;
    planner.carry_panning()?;
    let screen = planner.set_screen_size()?;
    planner.apply(screen)
}

/// xrandr's `output_t`: one output, what it is now and what it is to be.
struct Out<'a> {
    info: &'a OutputInfo,
    /// Indices into [`Planner::crtcs`] of the CRTCs that can drive it.
    possible: Vec<usize>,
    /// The mode it is to have, `None` to turn it off.
    mode: Option<ModeRef>,
    x: i32,
    y: i32,
    rotation: Rotation,
    /// What it is now: its mode, position and rotation.
    now: (Option<ModeRef>, i32, i32, Rotation),
    /// Whether anything about it changes (xrandr's `changes`, as a flag).
    changes: bool,
    /// The CRTC chosen for it (`crtc_info`).
    crtc: Option<usize>,
    /// The CRTC it had before the search for a better assignment
    /// (`current_crtc_info`).
    current_crtc: Option<usize>,
}

/// xrandr's `crtc_t`: one CRTC, what it is now and what it is to be.
struct Crtc<'a> {
    info: &'a CrtcInfo,
    /// How many outputs it drives now (`crtc_info->noutput`), which
    /// `pick_crtcs` zeroes while it searches.
    driving_now: usize,
    mode: Option<ModeRef>,
    x: i32,
    y: i32,
    rotation: Rotation,
    /// Indices into [`Planner::outputs`].
    outputs: Vec<usize>,
    changing: bool,
    /// The panning it keeps (`crtc_t`'s `panning_info`): what it has now,
    /// unless [`Planner::carry_panning`] resets it.
    panning: Option<Panning>,
    /// Whether its panning no longer fits and is reset.
    panning_reset: bool,
}

/// The screen size [`Planner::set_screen_size`] settles on.
struct Screen {
    width: u16,
    height: u16,
    mm_width: u32,
    mm_height: u32,
}

struct Planner<'a> {
    snapshot: &'a Snapshot,
    outputs: Vec<Out<'a>>,
    crtcs: Vec<Crtc<'a>>,
    /// Modes this plan creates, in [`ModeRef::Created`] order, with the
    /// size and refresh asked.
    created: Vec<((u32, u32, u32), ModeInfo)>,
    /// `RRCreateMode` and `RRAddOutputMode` requests, sent before the grab.
    mode_requests: Vec<Operation>,
    /// The output the layout makes primary, if any.
    primary: Option<OutputXid>,
}

impl<'a> Planner<'a> {
    fn new(snapshot: &'a Snapshot) -> Result<Self, PlanError> {
        let crtcs = snapshot
            .crtcs
            .iter()
            .map(|info| Crtc {
                info,
                driving_now: info.outputs.len(),
                mode: None,
                x: 0,
                y: 0,
                rotation: Rotation::ROTATE_0,
                outputs: Vec::new(),
                changing: false,
                // `get_crtcs` keeps no panning that is all zero.
                panning: info.panning.filter(|pan| *pan != Panning::default()),
                panning_reset: false,
            })
            .collect();
        let planner = Self {
            snapshot,
            outputs: Vec::new(),
            crtcs,
            created: Vec::new(),
            mode_requests: Vec::new(),
            primary: None,
        };
        for crtc in &snapshot.crtcs {
            if let Some(mode) = crtc.mode {
                planner.listed_mode(mode)?;
                if crtc.transform.is_some() {
                    return Err(PlanError::Transformed {
                        crtc: crtc.id,
                        outputs: snapshot
                            .outputs
                            .iter()
                            .filter(|output| crtc.outputs.contains(&output.id))
                            .map(|output| output.name.clone())
                            .collect(),
                    });
                }
            }
        }
        Ok(planner)
    }

    fn crtc_index(&self, xid: CrtcXid) -> Result<usize, PlanError> {
        self.crtcs
            .iter()
            .position(|crtc| crtc.info.id == xid)
            .ok_or_else(|| PlanError::Inconsistent(format!("no CRTC 0x{:x}", xid.0)))
    }

    fn listed_mode(&self, xid: ModeXid) -> Result<&'a ModeInfo, PlanError> {
        self.snapshot
            .resources
            .modes
            .iter()
            .find(|mode| mode.id == xid)
            .ok_or_else(|| PlanError::Inconsistent(format!("no mode 0x{:x}", xid.0)))
    }

    fn mode_info(&self, mode: ModeRef) -> Result<&ModeInfo, PlanError> {
        match mode {
            ModeRef::Listed(xid) => self.listed_mode(xid),
            ModeRef::Created(index) => Ok(&self.created[index].1),
        }
    }

    /// The outputs in xrandr's order, each set as the layout asks or left as
    /// it is: `get_outputs` and `set_output_info`.
    fn read_outputs(&mut self, layout: &Layout) -> Result<(), PlanError> {
        let snapshot = self.snapshot;
        let mut named: Vec<Option<&OutputEntry>> = vec![None; snapshot.outputs.len()];
        let mut order = Vec::new();
        for entry in &layout.entries {
            let index = entry
                .connector()
                .and_then(|connector| snapshot.outputs.iter().position(|o| o.name == connector))
                .ok_or_else(|| PlanError::NotConnected(entry.id.clone()))?;
            if named[index].is_some() {
                return Err(PlanError::Twice(snapshot.outputs[index].name.clone()));
            }
            named[index] = Some(entry);
            order.push(index);
        }
        order.extend((0..snapshot.outputs.len()).filter(|&index| named[index].is_none()));

        for index in order {
            let info = &snapshot.outputs[index];
            let possible = info
                .crtcs
                .iter()
                .map(|&xid| self.crtc_index(xid))
                .collect::<Result<Vec<_>, _>>()?;
            let current = info.crtc.map(|xid| self.crtc_index(xid)).transpose()?;
            let now = current.map(|crtc| &self.crtcs[crtc]);
            let now_mode = now.and_then(|crtc| crtc.info.mode);
            let (now_x, now_y) = now.map_or((0, 0), |crtc| {
                (i32::from(crtc.info.x), i32::from(crtc.info.y))
            });
            let now_rotation = now.map_or(Rotation::ROTATE_0, |crtc| crtc.info.rotation);
            let now_mode = now_mode.map(ModeRef::Listed);
            let mut out = Out {
                info,
                possible,
                mode: now_mode,
                x: now_x,
                y: now_y,
                rotation: now_rotation,
                now: (now_mode, now_x, now_y, now_rotation),
                changes: false,
                crtc: current,
                current_crtc: current,
            };
            if let Some(entry) = named[index] {
                self.set_from(&mut out, entry, layout)?;
            }
            self.outputs.push(out);
        }
        Ok(())
    }

    /// `out` as `entry` sets it: xrandr's `--off`, `--mode`, `--pos`,
    /// `--rotate`, `--reflect`, `--same-as` and `--primary`.
    fn set_from(
        &mut self,
        out: &mut Out<'a>,
        entry: &OutputEntry,
        layout: &Layout,
    ) -> Result<(), PlanError> {
        let state = &entry.state;
        let info = out.info;
        let connector = || info.name.clone();
        let unsupported = |what| PlanError::Unsupported {
            connector: connector(),
            what,
        };
        let refused = [
            (
                (state.scale - 1.0).abs() > SCALE_EPSILON,
                "a scale other than 1",
            ),
            (state.colour_preset.is_some(), "a colour preset"),
            (state.bit_depth.is_some(), "bit depth"),
            (state.overscan.is_some(), "overscan"),
            (state.rgb_range.is_some(), "RGB range"),
            (state.adaptive_sync.is_some(), "adaptive sync"),
        ];
        if let Some((_, what)) = refused.into_iter().find(|(set, _)| *set) {
            return Err(unsupported(what));
        }

        // `--primary`. A layout making no output primary leaves the primary
        // as it is, as xrandr does without `--primary` or `--noprimary`:
        // `primary` is a plain flag, so a profile from a desktop without a
        // primary output says false for every output.
        if state.enabled && state.primary {
            self.primary = Some(out.info.id);
        }

        if !state.enabled {
            out.mode = None;
            out.crtc = None;
            return Ok(());
        }
        let mode = state
            .mode
            .as_ref()
            .ok_or_else(|| PlanError::NoMode(connector()))?;

        // `--same-as`: a mirror sits where its source does.
        (out.x, out.y) = match &state.mirror_of {
            None => (state.x, state.y),
            Some(source) => layout
                .entries
                .iter()
                .find(|other| other.id == *source)
                .filter(|other| other.state.enabled && other.state.mirror_of.is_none())
                .map(|other| (other.state.x, other.state.y))
                .ok_or_else(|| PlanError::MirrorSource(connector()))?,
        };

        // `set_output_info`'s check, through `output_can_use_rotation`:
        // every CRTC that could drive the output must be able to turn it.
        out.rotation = rotation(state.transform);
        if !out
            .possible
            .iter()
            .all(|&crtc| can_use_rotation(self.crtcs[crtc].info, out.rotation))
        {
            return Err(PlanError::Rotation {
                connector: connector(),
                transform: state.transform,
            });
        }

        let current_mode = out.crtc.and_then(|crtc| self.crtcs[crtc].info.mode);
        out.mode = Some(self.choose_mode(out.info, current_mode, mode)?);
        Ok(())
    }

    /// The mode `mode` names for `output`. A mode the output lists, as
    /// xrandr's `find_mode_for_output` picks one (no double-scan mode
    /// unless a refresh is asked, then the nearest refresh), with the one it
    /// is driven at first and a mode's handle (its XID) taken as it is. Else
    /// a mode the screen has, added to the output (`--addmode`). Else a CVT
    /// mode made for it (`--newmode` with `cvt`'s timings), once however many
    /// outputs ask for it.
    fn choose_mode(
        &mut self,
        output: &OutputInfo,
        current: Option<ModeXid>,
        mode: &Mode,
    ) -> Result<ModeRef, PlanError> {
        let fits = |listed: &ModeInfo| {
            u32::from(listed.width) == mode.width
                && u32::from(listed.height) == mode.height
                && if mode.refresh_mhz == 0 {
                    listed.flags & mode_flags::DOUBLE_SCAN == 0
                } else {
                    mode.refresh_mhz.abs_diff(listed.refresh_mhz())
                        <= dsplr_model::REFRESH_TOLERANCE_MHZ
                }
        };
        let snapshot = self.snapshot;
        let screen_modes = &snapshot.resources.modes;
        let on_output: Vec<&ModeInfo> = output
            .modes
            .iter()
            .filter_map(|&xid| screen_modes.iter().find(|listed| listed.id == xid))
            .collect();
        if let Some(ModeHandle(handle)) = mode.handle
            && let Some(listed) = on_output
                .iter()
                .find(|listed| u64::from(listed.id.0) == handle && fits(listed))
        {
            return Ok(ModeRef::Listed(listed.id));
        }
        let nearest = |candidates: Vec<&ModeInfo>| {
            candidates
                .iter()
                .find(|listed| Some(listed.id) == current)
                .or_else(|| {
                    candidates
                        .iter()
                        // A progressive mode before an interlaced one at the
                        // same refresh: `Mode` has no interlace flag, so a
                        // layout without handles names both (1080i and
                        // 1080p at 60 Hz) alike.
                        .min_by_key(|listed| {
                            (
                                mode.refresh_mhz.abs_diff(listed.refresh_mhz()),
                                listed.flags & mode_flags::INTERLACE != 0,
                            )
                        })
                })
                .map(|listed| listed.id)
        };
        if let Some(xid) = nearest(on_output.iter().copied().filter(|m| fits(m)).collect()) {
            return Ok(ModeRef::Listed(xid));
        }

        let chosen = match nearest(screen_modes.iter().filter(|m| fits(m)).collect()) {
            Some(xid) => ModeRef::Listed(xid),
            None => self.create_mode(output, mode)?,
        };
        if matches!(chosen, ModeRef::Listed(xid) if output.modes.contains(&xid)) {
            return Ok(chosen);
        }
        self.mode_requests.push(Operation::AddOutputMode {
            output: output.id,
            mode: chosen,
        });
        Ok(chosen)
    }

    /// A CVT mode for `mode`: one the server has with the same name and
    /// timings (made by an earlier apply), one this plan already makes, or
    /// a new `RRCreateMode`. Another mode by that name is refused, since
    /// the server refuses a second (`RRModeCreateUser`, `BadName`).
    fn create_mode(&mut self, output: &OutputInfo, mode: &Mode) -> Result<ModeRef, PlanError> {
        let asked = (mode.width, mode.height, mode.refresh_mhz);
        if let Some(index) = self.created.iter().position(|(other, _)| *other == asked) {
            return Ok(ModeRef::Created(index));
        }
        let unavailable = |reason| PlanError::ModeUnavailable {
            connector: output.name.clone(),
            mode: mode.describe(),
            reason,
        };
        let (Ok(width), Ok(height)) = (u16::try_from(mode.width), u16::try_from(mode.height))
        else {
            return Err(unavailable("it is larger than X11 modes can be"));
        };
        // Millihertz to the hertz libxcvt takes: below 2^24, every value is
        // exact in an f32, and no refresh comes near that.
        #[allow(clippy::cast_precision_loss)]
        let refresh = mode.refresh_mhz as f32 / 1000.0;
        let info = cvt::mode(width, height, refresh)
            .ok_or_else(|| unavailable("CVT has no mode of that size"))?;
        let named = self
            .snapshot
            .resources
            .modes
            .iter()
            .find(|listed| listed.name == info.name);
        if let Some(listed) = named {
            if *listed
                == (ModeInfo {
                    id: listed.id,
                    ..info
                })
            {
                return Ok(ModeRef::Listed(listed.id));
            }
            return Err(unavailable(
                "the server has a mode of that name with other timings",
            ));
        }
        self.mode_requests.push(Operation::CreateMode(info.clone()));
        self.created.push((asked, info));
        Ok(ModeRef::Created(self.created.len() - 1))
    }

    /// xrandr's `set_positions`, after the `--pos` and `--same-as` of
    /// [`Self::set_from`]: the enabled outputs moved together so the top
    /// left of them all is at 0,0. Positions must then fit a CRTC's `INT16`.
    ///
    /// Then which outputs change. xrandr marks every output it is told
    /// about, and every output this move moves, and so sets their CRTCs
    /// again even when nothing about them differs; here an output changes
    /// when it is turned on or off, or its mode, position or rotation differ
    /// from the server's.
    fn set_positions(&mut self) -> Result<(), PlanError> {
        let enabled = || self.outputs.iter().filter(|out| out.mode.is_some());
        let min_x = enabled().map(|out| out.x).min().unwrap_or(0);
        let min_y = enabled().map(|out| out.y).min().unwrap_or(0);
        for out in &mut self.outputs {
            let (now_mode, now_x, now_y, now_rotation) = out.now;
            out.changes = out.mode != now_mode;
            if out.mode.is_none() {
                continue;
            }
            // In i64: two positions far apart in i32 are farther apart than
            // an i32 holds.
            let moved = |at: i32, min: i32| i16::try_from(i64::from(at) - i64::from(min));
            let (Ok(x), Ok(y)) = (moved(out.x, min_x), moved(out.y, min_y)) else {
                return Err(PlanError::Position {
                    connector: out.info.name.clone(),
                    x: out.x,
                    y: out.y,
                });
            };
            (out.x, out.y) = (i32::from(x), i32::from(y));
            out.changes |= (out.x, out.y, out.rotation) != (now_x, now_y, now_rotation);
        }
        Ok(())
    }

    /// xrandr's `check_crtc_for_output`: whether `crtc` can drive output
    /// `out`, given the outputs already assigned to it.
    fn check_crtc_for_output(&self, crtc: usize, out: usize) -> bool {
        let output = &self.outputs[out];
        if !output.possible.contains(&crtc) {
            return false;
        }
        let clones_all = self.outputs.iter().enumerate().all(|(index, other)| {
            index == out
                || other.mode.is_none()
                || other.crtc != Some(crtc)
                || output.info.clones.contains(&other.info.id)
        });
        if !clones_all {
            return false;
        }
        let target = &self.crtcs[crtc];
        if !target.outputs.is_empty() {
            // Already given outputs: this one must match them.
            target.mode == output.mode
                && (target.x, target.y, target.rotation) == (output.x, output.y, output.rotation)
        } else if target.driving_now > 0 {
            // Driving outputs now: this one must match what it shows.
            target.info.mode.map(ModeRef::Listed) == output.mode
                && (
                    i32::from(target.info.x),
                    i32::from(target.info.y),
                    target.info.rotation,
                ) == (output.x, output.y, output.rotation)
        } else {
            true
        }
    }

    /// xrandr's `find_crtc_for_output`: the first CRTC the output lists that
    /// can drive it.
    fn find_crtc_for_output(&self, out: usize) -> Option<usize> {
        self.outputs[out]
            .possible
            .iter()
            .copied()
            .find(|&crtc| self.check_crtc_for_output(crtc, out))
    }

    /// xrandr's `pick_crtcs`: give each changing output a CRTC. First the
    /// simple way, each output keeping the CRTC it has alone or taking the
    /// first that can drive it; when that fails, the best assignment of all
    /// outputs at once ([`Self::pick_crtcs_score`]).
    fn pick_crtcs(&mut self) -> Result<(), PlanError> {
        let mut simple = true;
        for out in 0..self.outputs.len() {
            let output = &self.outputs[out];
            if !output.changes || output.mode.is_none() {
                continue;
            }
            if let Some(crtc) = output.crtc {
                let driving = &self.crtcs[crtc].info.outputs;
                if !driving.is_empty() && (driving.len() > 1 || driving[0] != output.info.id) {
                    simple = false;
                    break;
                }
            } else {
                self.outputs[out].crtc = self.find_crtc_for_output(out);
                if self.outputs[out].crtc.is_none() {
                    simple = false;
                    break;
                }
            }
        }
        if simple {
            return Ok(());
        }

        for out in &mut self.outputs {
            out.current_crtc = out.crtc;
        }
        for crtc in &mut self.crtcs {
            crtc.driving_now = 0;
        }
        self.pick_crtcs_score(0);
        for crtc in &mut self.crtcs {
            crtc.driving_now = crtc.info.outputs.len();
        }

        for out in &mut self.outputs {
            if out.mode.is_some() && out.crtc.is_none() {
                return Err(PlanError::NoCrtc(out.info.name.clone()));
            }
            if !out.changes && out.crtc != out.current_crtc {
                out.changes = true;
            }
        }
        Ok(())
    }

    /// xrandr's `pick_crtcs_score`: the best score for outputs `from` on,
    /// leaving each with the CRTC that reaches it. Each output given a CRTC
    /// scores 1000, one more when it is the CRTC it had.
    fn pick_crtcs_score(&mut self, from: usize) -> u32 {
        /// What each output given a CRTC adds to the score.
        const ENABLED: u32 = 1000;
        /// The slight preference for a CRTC the output already has.
        const KEPT: u32 = 1;

        if from == self.outputs.len() {
            return 0;
        }
        // Scored with this output off.
        self.outputs[from].crtc = None;
        let mut best_score = self.pick_crtcs_score(from + 1);
        if self.outputs[from].mode.is_none() {
            return best_score;
        }

        let mut best_crtc = None;
        for crtc in self.outputs[from].possible.clone() {
            // Reset the following outputs' CRTCs (`disable_outputs`).
            for out in &mut self.outputs[from + 1..] {
                out.crtc = None;
            }
            if !self.check_crtc_for_output(crtc, from) {
                continue;
            }
            let mine = if Some(crtc) == self.outputs[from].current_crtc {
                ENABLED + KEPT
            } else {
                ENABLED
            };
            self.outputs[from].crtc = Some(crtc);
            let score = mine + self.pick_crtcs_score(from + 1);
            if score > best_score {
                best_crtc = Some(crtc);
                best_score = score;
            }
        }
        self.outputs[from].crtc = best_crtc;
        // Reset the following outputs to suit this one's best CRTC.
        self.pick_crtcs_score(from + 1);
        best_score
    }

    /// xrandr's `set_crtcs` and `crtc_add_output`: each enabled output put
    /// on its CRTC, the first output of a CRTC setting its mode, position
    /// and rotation.
    fn set_crtcs(&mut self) -> Result<(), PlanError> {
        for out in 0..self.outputs.len() {
            let output = &self.outputs[out];
            if output.mode.is_none() {
                continue;
            }
            let crtc = output
                .crtc
                .ok_or_else(|| PlanError::NoCrtc(output.info.name.clone()))?;
            let (mode, x, y, rotation) = (output.mode, output.x, output.y, output.rotation);
            let target = &mut self.crtcs[crtc];
            if target.outputs.is_empty() {
                (target.mode, target.x, target.y, target.rotation) = (mode, x, y, rotation);
            }
            target.outputs.push(out);
        }
        Ok(())
    }

    /// xrandr's `mark_changing_crtcs`: a CRTC changes when an output it
    /// drives now, or one it is to drive, changes.
    fn mark_changing_crtcs(&mut self) -> Result<(), PlanError> {
        for crtc in 0..self.crtcs.len() {
            let mut changing = false;
            for xid in &self.crtcs[crtc].info.outputs {
                let output = self
                    .outputs
                    .iter()
                    .find(|out| out.info.id == *xid)
                    .ok_or_else(|| PlanError::Inconsistent(format!("no output 0x{:x}", xid.0)))?;
                changing |= output.changes;
            }
            changing |= self.crtcs[crtc]
                .outputs
                .iter()
                .any(|&out| self.outputs[out].changes);
            self.crtcs[crtc].changing = changing;
        }
        Ok(())
    }

    /// Which CRTCs being set keep their panning. xrandr carries a CRTC's
    /// panning over: `get_crtcs` reads it and `crtc_apply` sets it again
    /// after the CRTC. Here a CRTC being set keeps it only while it still
    /// fits the CRTC's new mode and position ([`panning_fits`]), and has it
    /// reset otherwise, which the plan notes.
    fn carry_panning(&mut self) -> Result<(), PlanError> {
        for index in 0..self.crtcs.len() {
            let crtc = &self.crtcs[index];
            let (Some(pan), Some(mode), true) = (crtc.panning, crtc.mode, crtc.changing) else {
                continue;
            };
            let fits = panning_fits(&pan, self.mode_info(mode)?, (crtc.x, crtc.y), crtc.rotation);
            if !fits {
                let crtc = &mut self.crtcs[index];
                crtc.panning = None;
                crtc.panning_reset = true;
            }
        }
        Ok(())
    }

    /// xrandr's `set_screen_size`, then its physical size from `main`.
    ///
    /// The screen just holds every enabled output and the panning area of
    /// its CRTC, as xrandr counts it, but for panning about to be reset. It
    /// is grown to the server's minimum, and refused above its maximum.
    /// xrandr sizes the screen before choosing CRTCs; it reads only the
    /// outputs, so sizing it after is the same but for knowing which panning
    /// is reset.
    ///
    /// Its size in millimetres keeps the screen's DPI, taken from its height
    /// now, as xrandr does without `--dpi` or `--fbmm`; the size unchanged
    /// keeps the millimetres too.
    fn set_screen_size(&self) -> Result<Screen, PlanError> {
        let (mut width, mut height) = (0_i64, 0_i64);
        for output in self.outputs.iter() {
            let Some(mode) = output.mode else {
                continue;
            };
            let (w, h) = geometry(self.mode_info(mode)?, output.rotation);
            width = width.max(i64::from(output.x) + w);
            height = height.max(i64::from(output.y) + h);
            if let Some(pan) = output.crtc.and_then(|crtc| self.crtcs[crtc].panning) {
                width = width.max(i64::from(pan.left) + i64::from(pan.width));
                height = height.max(i64::from(pan.top) + i64::from(pan.height));
            }
        }

        let range = self.snapshot.size_range;
        if width > i64::from(range.max_width) || height > i64::from(range.max_height) {
            return Err(PlanError::ScreenTooLarge {
                width,
                height,
                max_width: range.max_width,
                max_height: range.max_height,
            });
        }
        let clamp = |size: i64, min: u16| {
            u16::try_from(size.max(i64::from(min))).expect("at most the maximum, a u16")
        };
        let (width, height) = (
            clamp(width, range.min_width),
            clamp(height, range.min_height),
        );

        let now = self.snapshot.screen;
        let (mm_width, mm_height) = if (width, height) == (now.width, now.height) {
            (now.mm_width, now.mm_height)
        } else {
            let dpi = MM_PER_INCH * f64::from(now.height) / f64::from(now.mm_height);
            // C's double to int: toward zero, as xrandr assigns it.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let mm = |pixels: u16| (MM_PER_INCH * f64::from(pixels) / dpi) as u32;
            (mm(width), mm(height))
        };
        Ok(Screen {
            width,
            height,
            mm_width,
            mm_height,
        })
    }

    /// xrandr's `apply`, with the mode requests first: grab the server,
    /// turn off each CRTC that is to be off or whose new picture would not
    /// fit the new screen or differs in size from its old one, size the
    /// screen, set each changing CRTC, set the primary output, ungrab.
    fn apply(self, screen: Screen) -> Result<Plan, PlanError> {
        let mut operations = self.mode_requests.clone();
        let mut notes = Vec::new();
        operations.push(Operation::GrabServer);

        let mut changing: Vec<bool> = self.crtcs.iter().map(|crtc| crtc.changing).collect();
        // Which CRTCs have had their panning reset already.
        let mut reset = vec![false; self.crtcs.len()];
        for (index, crtc) in self.crtcs.iter().enumerate() {
            let Some(old_mode) = crtc.info.mode else {
                continue;
            };
            if let Some(mode) = crtc.mode {
                // xrandr turns the new mode by the CRTC's old rotation here.
                let old = geometry(self.listed_mode(old_mode)?, crtc.info.rotation);
                let new = geometry(self.mode_info(mode)?, crtc.info.rotation);
                let (x, y) = (i64::from(crtc.info.x), i64::from(crtc.info.y));
                if old == new
                    && x + old.0 <= i64::from(screen.width)
                    && y + old.1 <= i64::from(screen.height)
                {
                    continue;
                }
                changing[index] = true;
            }
            if crtc.panning_reset {
                operations.push(reset_panning(crtc.info.id));
                reset[index] = true;
            }
            operations.push(Operation::DisableCrtc(crtc.info.id));
        }

        let now = self.snapshot.screen;
        if (
            screen.width,
            screen.height,
            screen.mm_width,
            screen.mm_height,
        ) != (now.width, now.height, now.mm_width, now.mm_height)
        {
            operations.push(Operation::SetScreenSize {
                width: screen.width,
                height: screen.height,
                mm_width: screen.mm_width,
                mm_height: screen.mm_height,
            });
        }

        for (index, crtc) in self.crtcs.iter().enumerate() {
            let Some(mode) = crtc.mode.filter(|_| changing[index]) else {
                continue;
            };
            if crtc.panning_reset && !reset[index] {
                operations.push(reset_panning(crtc.info.id));
            }
            let position = |value: i32| i16::try_from(value).expect("checked by set_positions");
            operations.push(Operation::SetCrtcConfig {
                crtc: crtc.info.id,
                mode,
                x: position(crtc.x),
                y: position(crtc.y),
                rotation: crtc.rotation,
                outputs: crtc
                    .outputs
                    .iter()
                    .map(|&out| self.outputs[out].info.id)
                    .collect(),
            });
            // `crtc_apply` sets the CRTC's panning again after it.
            if let Some(panning) = crtc.panning {
                operations.push(Operation::SetPanning {
                    crtc: crtc.info.id,
                    panning,
                });
            } else if crtc.panning_reset {
                notes.push(Note::PanningReset {
                    crtc: crtc.info.id,
                    outputs: crtc
                        .outputs
                        .iter()
                        .map(|&out| self.outputs[out].info.name.clone())
                        .collect(),
                });
            }
        }

        if let Some(primary) = self.primary.filter(|&p| Some(p) != self.snapshot.primary) {
            operations.push(Operation::SetOutputPrimary(primary));
        }
        operations.push(Operation::UngrabServer);
        Ok(Plan { operations, notes })
    }
}

/// `RRSetPanning` turning `crtc`'s panning off.
fn reset_panning(crtc: CrtcXid) -> Operation {
    Operation::SetPanning {
        crtc,
        panning: Panning::default(),
    }
}

/// Whether panning `pan` still fits a CRTC set to `mode` at `at`, turned by
/// `rotation`, so that Xorg keeps it and leaves the CRTC where the layout
/// puts it. Only Xorg's modesetting layer pans (`xf86RandR12.c`, X server
/// 21.1.26, the only `rrSetPanning`); Xvfb has none, so nothing but that
/// source backs this.
///
/// Right after a CRTC is set, `xf86RandR12CrtcSet` runs
/// `xf86RandR13VerifyPanningArea` and then `xf86RandR13Pan`. Per axis with
/// panning, the first wants the area at least as long as the mode and its
/// two borders no longer than it, and an axis without panning to have no
/// offset either; both measure the mode by `HDisplay` and `VDisplay`, its
/// own unturned size. The second, for a CRTC with no transform, clamps its
/// position into `[x1, x2 - HDisplay]` by `[y1, y2 - VDisplay]`; so the
/// CRTC must lie inside the area, or it is moved. A turned or reflected
/// CRTC has a transform and skips that clamp, but it is held to lying
/// inside the area all the same, by its turned size, since panning follows
/// the pointer across the area it covers. For a CRTC with no transform the
/// turned size is its mode's own, so the rule is the server's there.
///
/// The pointer still pans a CRTC whose panning is kept, as panning is
/// meant to; this only keeps the set itself from moving it.
fn panning_fits(pan: &Panning, mode: &ModeInfo, (x, y): (i32, i32), rotation: Rotation) -> bool {
    let (width, height) = geometry(mode, rotation);
    let axis = |start: u16, size: u16, borders: (i16, i16), display: u16, at: i32, extent: i64| {
        if size == 0 {
            return start == 0;
        }
        let (start, size, at) = (i64::from(start), i64::from(size), i64::from(at));
        size >= i64::from(display)
            && i64::from(borders.0) + i64::from(borders.1) <= i64::from(display)
            && start <= at
            && at + extent <= start + size
    };
    axis(
        pan.left,
        pan.width,
        (pan.border_left, pan.border_right),
        mode.width,
        x,
        width,
    ) && axis(
        pan.top,
        pan.height,
        (pan.border_top, pan.border_bottom),
        mode.height,
        y,
        height,
    )
}

/// Millimetres in an inch, as xrandr's DPI arithmetic writes it.
const MM_PER_INCH: f64 = 25.4;

/// RandR's rotation and reflection for `transform`: a flipped transform is
/// `RR_Reflect_X` with the same rotation, flipped-90 and flipped-270
/// included.
///
/// `RRTransformCompute` (`randr/rrtransform.c`, X server 21.1.26) maps a
/// CRTC's pixels to the framebuffer by rotating, then reflecting in the
/// rotated frame (`pixman_f_transform_rotate`, then `_scale` by -1 with the
/// reflected axis's length as offset), so the picture is reflected in its
/// own x before it is turned. `wl_output.transform` says the same of its
/// flipped values: "an initial flip around a vertical axis followed by
/// rotation". Turning 90 with `RR_Reflect_Y` instead would flip after the
/// turn, which is flipped-270. The test
/// `each_transform_shows_what_randr_shows_for_its_rotation` checks all eight
/// against wlroots 0.18.2's own mapping (`wlr_box_transform`).
pub const fn rotation(transform: Transform) -> Rotation {
    let turn = match transform {
        Transform::Normal | Transform::Flipped => Rotation::ROTATE_0,
        Transform::Rotate90 | Transform::Flipped90 => Rotation::ROTATE_90,
        Transform::Rotate180 | Transform::Flipped180 => Rotation::ROTATE_180,
        Transform::Rotate270 | Transform::Flipped270 => Rotation::ROTATE_270,
    };
    match transform {
        Transform::Flipped
        | Transform::Flipped90
        | Transform::Flipped180
        | Transform::Flipped270 => Rotation(turn.0 | Rotation::REFLECT_X.0),
        _ => turn,
    }
}

/// The transform a CRTC's rotation and reflection show: [`rotation`]'s
/// inverse, which a flipped transform comes back from. `RR_Reflect_Y` is
/// `RR_Reflect_X` turned half a turn further, and both reflections together
/// are half a turn and none (`RRTransformCompute` reflects either axis in
/// the turned frame). `None` for bits that are not one rotation.
pub fn transform(rotation: Rotation) -> Option<Transform> {
    let turns = match rotation.direction() {
        Rotation::ROTATE_0 => 0,
        Rotation::ROTATE_90 => 1,
        Rotation::ROTATE_180 => 2,
        Rotation::ROTATE_270 => 3,
        _ => return None,
    };
    let x = rotation.0 & Rotation::REFLECT_X.0 != 0;
    let y = rotation.0 & Rotation::REFLECT_Y.0 != 0;
    let turns = if y { (turns + 2) % 4 } else { turns };
    Transform::from_wayland(if x != y { 4 + turns } else { turns })
}

/// xrandr's `crtc_can_use_rotation`: the CRTC can do the rotation and
/// every reflection asked.
const fn can_use_rotation(crtc: &CrtcInfo, rotation: Rotation) -> bool {
    let (can, dir, reflect) = (
        crtc.rotations.0,
        rotation.direction().0,
        rotation.reflection().0,
    );
    can & dir != 0 && can & reflect == reflect
}

/// xrandr's `mode_width` and `mode_height` (through `mode_geometry`, with
/// no CRTC transform): the mode's size turned by `rotation`, `0` by `0` for
/// a rotation that is none of the four.
fn geometry(mode: &ModeInfo, rotation: Rotation) -> (i64, i64) {
    let (width, height) = (i64::from(mode.width), i64::from(mode.height));
    match rotation.direction() {
        Rotation::ROTATE_0 | Rotation::ROTATE_180 => (width, height),
        Rotation::ROTATE_90 | Rotation::ROTATE_270 => (height, width),
        _ => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::{ModeInfo, Panning, Rotation, panning_fits};

    #[test]
    fn panning_fits_as_the_server_and_the_layout_need() {
        let mode = ModeInfo {
            width: 1920,
            height: 1080,
            ..ModeInfo::default()
        };
        let area = Panning {
            left: 100,
            width: 3840,
            height: 2160,
            ..Panning::default()
        };
        let fits = |pan: Panning, at| panning_fits(&pan, &mode, at, Rotation::ROTATE_0);
        assert!(fits(area, (100, 0)));
        assert!(fits(area, (2020, 1080)));
        // The CRTC left of the area, or past its end.
        assert!(!fits(area, (99, 0)));
        assert!(!fits(area, (2021, 0)));
        // No panning down, but an offset there: the server refuses it.
        let offset = Panning {
            height: 0,
            top: 5,
            ..area
        };
        assert!(!fits(offset, (100, 0)));
        assert!(fits(Panning { top: 0, ..offset }, (100, 0)));
        // Borders wider than the mode: the server refuses them.
        let borders = Panning {
            border_left: 1000,
            border_right: 921,
            ..area
        };
        assert!(!fits(borders, (100, 0)));
        assert!(fits(
            Panning {
                border_right: 920,
                ..borders
            },
            (100, 0)
        ));
    }
}
