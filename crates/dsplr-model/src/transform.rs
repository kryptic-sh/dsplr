use serde::{Deserialize, Serialize};

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
}
