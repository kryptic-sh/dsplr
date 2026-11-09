//! The colours the GUI paints with, by role.

use crate::draw::Color;

/// Each colour the draw list uses, named for what it paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// The canvas behind the outputs.
    pub background: Color,
    /// The property panel and the bottom bar.
    pub surface: Color,
    /// The line between the canvas and the panel and bar.
    pub divider: Color,
    /// An output's rectangle.
    pub output: Color,
    /// The selected output's rectangle.
    pub output_selected: Color,
    /// An output's border.
    pub border: Color,
    /// The selected output's border.
    pub accent: Color,
    /// A pending edit's outline, and a changed value in the panel.
    pub pending: Color,
    /// An output validation refuses, and the bar saying why.
    pub error: Color,
    /// An output validation warns about, and the bar saying why.
    pub warning: Color,
    /// A tray tile: a disabled output.
    pub tray: Color,
    /// A tray tile's border.
    pub tray_border: Color,
    pub text: Color,
    /// Secondary text: descriptions and values.
    pub text_dim: Color,
    /// Labels, hints, and what a disabled output or an unsupported setting
    /// says.
    pub muted: Color,
}

impl Theme {
    /// Tokyo Night, folke's "Night" variant, as the rest of the suite.
    pub const fn tokyo_night() -> Self {
        Self {
            background: Color::hex(0x1a1b26),
            surface: Color::hex(0x16161e),
            divider: Color::hex(0x292e42),
            output: Color::hex(0x292e42),
            output_selected: Color::hex(0x283457),
            border: Color::hex(0x414868),
            accent: Color::hex(0x7aa2f7),
            pending: Color::hex(0xff9e64),
            error: Color::hex(0xf7768e),
            warning: Color::hex(0xe0af68),
            tray: Color::hex(0x16161e),
            tray_border: Color::hex(0x3b4261),
            text: Color::hex(0xc0caf5),
            text_dim: Color::hex(0xa9b1d6),
            muted: Color::hex(0x565f89),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::tokyo_night()
    }
}
