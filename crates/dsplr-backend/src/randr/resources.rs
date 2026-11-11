//! What the X server reports about its RandR state, as plain structs: the
//! screen's resources, each output and CRTC, the screen size and its range,
//! the primary output, each output's EDID and each CRTC's transform. The
//! backend fills them from crcbl-x11's replies (`x11.rs`); the planner and
//! [`super::snapshot`] read only these, so they run without a server.
//!
//! Field names follow the RandR protocol's replies (`randrproto.txt`,
//! RandR 1.5), so the conversion is a copy. XIDs are 32 bits on the wire.

use dsplr_model::hz_to_mhz;

/// A CRTC's XID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CrtcXid(pub u32);

/// An output's XID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct OutputXid(pub u32);

/// A mode's XID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ModeXid(pub u32);

/// RandR's `ROTATION` bits: one of the four rotations, and either
/// reflection or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rotation(pub u16);

impl Rotation {
    pub const ROTATE_0: Self = Self(1);
    pub const ROTATE_90: Self = Self(2);
    pub const ROTATE_180: Self = Self(4);
    pub const ROTATE_270: Self = Self(8);
    pub const REFLECT_X: Self = Self(16);
    pub const REFLECT_Y: Self = Self(32);

    /// The rotation bits alone.
    pub const fn direction(self) -> Self {
        Self(self.0 & 0xf)
    }

    /// The reflection bits alone.
    pub const fn reflection(self) -> Self {
        Self(self.0 & (Self::REFLECT_X.0 | Self::REFLECT_Y.0))
    }

    /// Whether width and height trade places: a quarter turn either way.
    pub const fn swaps_axes(self) -> bool {
        self.0 & (Self::ROTATE_90.0 | Self::ROTATE_270.0) != 0
    }
}

impl std::ops::BitOr for Rotation {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// An output's `connection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Connection {
    #[default]
    Connected,
    Disconnected,
    Unknown,
}

/// `MODEINFO`'s `ModeFlags` bits the planner reads or writes.
pub mod mode_flags {
    pub const HSYNC_POSITIVE: u32 = 1;
    pub const HSYNC_NEGATIVE: u32 = 2;
    pub const VSYNC_POSITIVE: u32 = 4;
    pub const VSYNC_NEGATIVE: u32 = 8;
    pub const INTERLACE: u32 = 16;
    pub const DOUBLE_SCAN: u32 = 32;
}

/// One `MODEINFO` with its name.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ModeInfo {
    pub id: ModeXid,
    pub width: u16,
    pub height: u16,
    /// Hertz.
    pub dot_clock: u32,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    /// [`mode_flags`] bits.
    pub flags: u32,
    pub name: String,
}

impl ModeInfo {
    /// The refresh rate in millihertz, `0` when the mode has no timings
    /// (Xvfb's modes have none). xrandr's `mode_refresh` (xrandr.c,
    /// xrandr 1.5.4): a double-scan mode draws each line twice, and an
    /// interlaced one's rate is its field rate.
    pub fn refresh_mhz(&self) -> u32 {
        let mut vtotal = f64::from(self.vtotal);
        if self.flags & mode_flags::DOUBLE_SCAN != 0 {
            vtotal *= 2.0;
        }
        if self.flags & mode_flags::INTERLACE != 0 {
            vtotal /= 2.0;
        }
        if self.htotal == 0 || vtotal == 0.0 {
            return 0;
        }
        hz_to_mhz(f64::from(self.dot_clock) / (f64::from(self.htotal) * vtotal)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{ModeInfo, mode_flags};

    #[test]
    fn refresh_counts_double_scan_and_interlace_as_xrandr_does() {
        // VESA's 640x480 at 59.94 Hz: 25.175 MHz over 800 by 525.
        let mode = |flags| ModeInfo {
            dot_clock: 25_175_000,
            htotal: 800,
            vtotal: 525,
            flags,
            ..ModeInfo::default()
        };
        assert_eq!(mode(0).refresh_mhz(), 59_940);
        assert_eq!(mode(mode_flags::DOUBLE_SCAN).refresh_mhz(), 29_970);
        assert_eq!(mode(mode_flags::INTERLACE).refresh_mhz(), 119_881);
        assert_eq!(ModeInfo::default().refresh_mhz(), 0);
    }
}

/// `RRGetScreenResourcesCurrent`'s reply.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ScreenResources {
    pub timestamp: u32,
    pub config_timestamp: u32,
    pub crtcs: Vec<CrtcXid>,
    pub outputs: Vec<OutputXid>,
    pub modes: Vec<ModeInfo>,
}

/// `RRGetOutputInfo`'s reply, with the output it describes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OutputInfo {
    pub id: OutputXid,
    pub timestamp: u32,
    /// The CRTC driving it, `None` when it is off.
    pub crtc: Option<CrtcXid>,
    pub mm_width: u32,
    pub mm_height: u32,
    pub connection: Connection,
    pub subpixel_order: u8,
    /// The CRTCs that can drive it.
    pub crtcs: Vec<CrtcXid>,
    /// Its modes; the first `num_preferred` are preferred.
    pub modes: Vec<ModeXid>,
    pub num_preferred: u16,
    /// The outputs that can share a CRTC with it.
    pub clones: Vec<OutputXid>,
    pub name: String,
    /// Its `EDID` property's bytes, when it has one.
    pub edid: Option<Vec<u8>>,
}

/// `RRGetPanning`'s reply, without its timestamp. All zero is no panning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Panning {
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    pub track_left: u16,
    pub track_top: u16,
    pub track_width: u16,
    pub track_height: u16,
    pub border_left: i16,
    pub border_top: i16,
    pub border_right: i16,
    pub border_bottom: i16,
}

/// `RRGetCrtcInfo`'s reply, with the CRTC it describes and its panning.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CrtcInfo {
    pub id: CrtcXid,
    pub timestamp: u32,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    /// `None` when the CRTC is off.
    pub mode: Option<ModeXid>,
    pub rotation: Rotation,
    /// The rotations and reflections it can do.
    pub rotations: Rotation,
    /// The outputs it drives.
    pub outputs: Vec<OutputXid>,
    /// The outputs it could drive.
    pub possible: Vec<OutputXid>,
    /// `None` when it has none (all zero) or the server is older than
    /// RandR 1.3.
    pub panning: Option<Panning>,
    /// `RRGetCrtcTransform`'s current transform, as 16.16 fixed point, when
    /// it is not the identity (`xrandr --scale` or `--transform` set it).
    pub transform: Option<[[i32; 3]; 3]>,
}

/// `RRGetScreenSizeRange`'s reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SizeRange {
    pub min_width: u16,
    pub min_height: u16,
    pub max_width: u16,
    pub max_height: u16,
}

/// The root window's size: pixels from the screen, millimetres as last set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ScreenSize {
    pub width: u16,
    pub height: u16,
    pub mm_width: u32,
    pub mm_height: u32,
}

/// Everything the planner reads, taken at one moment.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub resources: ScreenResources,
    /// One per output of `resources`, in its order.
    pub outputs: Vec<OutputInfo>,
    /// One per CRTC of `resources`, in its order.
    pub crtcs: Vec<CrtcInfo>,
    pub size_range: SizeRange,
    pub screen: ScreenSize,
    /// `RRGetOutputPrimary`'s reply, `None` when no output is primary.
    pub primary: Option<OutputXid>,
}
