use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::name::{ParseNameError, parse_name};

/// The eight Wayland output transforms. XRandR's rotation and reflection
/// pairs map onto the same eight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    /// In `wl_output.transform` order, so a value's index is its wire value.
    const ALL: [Self; 8] = [
        Self::Normal,
        Self::Rotate90,
        Self::Rotate180,
        Self::Rotate270,
        Self::Flipped,
        Self::Flipped90,
        Self::Flipped180,
        Self::Flipped270,
    ];

    /// From `wl_output.transform`'s value, which Hyprland, wlroots and KDE
    /// all report. `None` for a value outside the enum.
    pub fn from_wayland(value: u32) -> Option<Self> {
        usize::try_from(value)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }

    /// `wl_output.transform`'s value for this transform.
    pub const fn to_wayland(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Rotate90 => 1,
            Self::Rotate180 => 2,
            Self::Rotate270 => 3,
            Self::Flipped => 4,
            Self::Flipped90 => 5,
            Self::Flipped180 => 6,
            Self::Flipped270 => 7,
        }
    }

    /// Whether width and height trade places on screen (a quarter turn).
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }
}

/// `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180` or
/// `flipped-270`: `wl_output.transform`'s names, so `90` is its value 1,
/// which the protocol calls 90 degrees counter-clockwise. sway names the
/// quarter turns the other way round: its `90` is this `270` (sway 1.12).
impl fmt::Display for Transform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        })
    }
}

/// The names [`Display`](fmt::Display) writes, and nothing else.
impl FromStr for Transform {
    type Err = ParseNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_name("transform", &Self::ALL, text)
    }
}

#[cfg(test)]
mod tests {
    use super::Transform;

    #[test]
    fn wayland_values_match_the_protocol() {
        // wl_output.transform: normal 0, 90 1, 180 2, 270 3, flipped 4,
        // flipped_90 5, flipped_180 6, flipped_270 7.
        let expected = [
            (0, Transform::Normal),
            (1, Transform::Rotate90),
            (2, Transform::Rotate180),
            (3, Transform::Rotate270),
            (4, Transform::Flipped),
            (5, Transform::Flipped90),
            (6, Transform::Flipped180),
            (7, Transform::Flipped270),
        ];
        for (value, transform) in expected {
            assert_eq!(Transform::from_wayland(value), Some(transform));
            assert_eq!(transform.to_wayland(), value);
        }
        assert_eq!(Transform::from_wayland(8), None);
    }

    #[test]
    fn only_quarter_turns_swap_axes() {
        let swapping: Vec<_> = Transform::ALL
            .into_iter()
            .filter(|t| t.swaps_axes())
            .collect();
        assert_eq!(
            swapping,
            [
                Transform::Rotate90,
                Transform::Rotate270,
                Transform::Flipped90,
                Transform::Flipped270,
            ]
        );
    }

    #[test]
    fn each_name_reads_back_as_the_transform_that_wrote_it() {
        for transform in Transform::ALL {
            assert_eq!(transform.to_string().parse(), Ok(transform));
        }
        assert_eq!("90".parse(), Ok(Transform::Rotate90));
        assert_eq!("flipped-270".parse(), Ok(Transform::Flipped270));
    }

    #[test]
    fn other_names_are_refused_listing_the_names() {
        for text in ["", "rotate90", "Normal", " 90", "90deg", "flipped90", "360"] {
            let error = text.parse::<Transform>().unwrap_err().to_string();
            assert!(
                error.contains("normal, 90, 180, 270, flipped, flipped-90"),
                "{text:?}: {error}"
            );
        }
    }
}
