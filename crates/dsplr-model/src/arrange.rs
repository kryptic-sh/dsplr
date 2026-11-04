//! Arranging outputs on the canvas: snapping a dragged output to the others,
//! and normalising the layout to the origin on release
//! (`docs/plan/03-display-model.md#snapping`).

use crate::rect::Rect;
use crate::{Layout, OutputEntry, OutputId};

/// Normalising would move an output's position outside `i32`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{output}: moving the layout to the origin puts this output out of range")]
pub struct PositionOutOfRange {
    pub output: OutputId,
}

/// What a snap aligns, best first: an edge against the other output's edge,
/// then the same edge (left with left, top with top), then the centres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    Contact,
    Aligned,
    Centred,
}

/// Another output's extent along the axis being snapped.
struct Span {
    start: i64,
    len: i64,
    /// Whether the two outputs overlap, or nearly, along the other axis, so
    /// that putting them edge to edge on this one makes them touch.
    beside: bool,
}

/// The outputs placed on the canvas: enabled, not mirrors, with a size.
fn placed(entries: &[OutputEntry]) -> impl Iterator<Item = (usize, &OutputEntry)> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.state.enabled && entry.state.mirror_of.is_none())
}

impl Layout {
    /// Moves every output so the smallest x and the smallest y among the
    /// enabled, non-mirrored outputs are both zero, which XRandR requires and
    /// every other backend accepts. Disabled outputs and mirrors move by the
    /// same amount, keeping their places relative to the rest. A layout with
    /// nothing placed is left as it is.
    ///
    /// # Errors
    ///
    /// When an output would land outside `i32`; the layout is then unchanged.
    pub fn normalise(&mut self) -> Result<(), PositionOutOfRange> {
        let Some(min_x) = placed(&self.entries).map(|(_, e)| e.state.x).min() else {
            return Ok(());
        };
        let min_y = placed(&self.entries)
            .map(|(_, e)| e.state.y)
            .min()
            .unwrap_or_default();
        let shift = |position: i32, by: i32| i32::try_from(i64::from(position) - i64::from(by));
        let moved = self
            .entries
            .iter()
            .map(|entry| {
                let out_of_range = |_| PositionOutOfRange {
                    output: entry.id.clone(),
                };
                Ok((
                    shift(entry.state.x, min_x).map_err(out_of_range)?,
                    shift(entry.state.y, min_y).map_err(out_of_range)?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (entry, (x, y)) in self.entries.iter_mut().zip(moved) {
            entry.state.x = x;
            entry.state.y = y;
        }
        Ok(())
    }

    /// Where the output at `moving` (an index into `entries`) lands when
    /// dragged to `(x, y)`: each axis snaps to the nearest fit with another
    /// placed output within `threshold` layout pixels, preferring edge
    /// contact, then aligned edges, then centres. The canvas converts its
    /// snap distance from screen pixels to layout pixels before calling.
    ///
    /// An output without a size (no mode), or an index past the end, is not
    /// snapped: `(x, y)` comes back as given.
    pub fn snap(&self, moving: usize, x: i32, y: i32, threshold: u32) -> (i32, i32) {
        let Some((width, height)) = self
            .entries
            .get(moving)
            .and_then(|entry| entry.state.logical_size())
        else {
            return (x, y);
        };
        let me = Rect {
            x: x.into(),
            y: y.into(),
            width: width.into(),
            height: height.into(),
        };
        let threshold = i64::from(threshold);
        let others: Vec<Rect> = placed(&self.entries)
            .filter(|(i, _)| *i != moving)
            .filter_map(|(_, entry)| Rect::of(entry))
            .collect();
        let near = |start: i64, len: i64, other_start: i64, other_len: i64| {
            start < other_start + other_len + threshold && other_start < start + len + threshold
        };
        let snapped_x = snap_axis(
            me.x,
            me.width,
            others.iter().map(|o| Span {
                start: o.x,
                len: o.width,
                beside: near(me.y, me.height, o.y, o.height),
            }),
            threshold,
        );
        let snapped_y = snap_axis(
            me.y,
            me.height,
            others.iter().map(|o| Span {
                start: o.y,
                len: o.height,
                beside: near(me.x, me.width, o.x, o.width),
            }),
            threshold,
        );
        (
            i32::try_from(snapped_x).unwrap_or(x),
            i32::try_from(snapped_y).unwrap_or(y),
        )
    }
}

/// The best fit for an extent at `start` of length `len` against `others`,
/// or `start` when none is within `threshold`. Ties go to the nearer fit,
/// then to the earlier output.
fn snap_axis(start: i64, len: i64, others: impl Iterator<Item = Span>, threshold: i64) -> i64 {
    others
        .flat_map(|other| {
            let contact = other.beside.then_some([
                (Fit::Contact, other.start + other.len),
                (Fit::Contact, other.start - len),
            ]);
            contact.into_iter().flatten().chain([
                (Fit::Aligned, other.start),
                (
                    Fit::Centred,
                    (2 * other.start + other.len - len).div_euclid(2),
                ),
            ])
        })
        .filter(|(_, position)| (position - start).abs() <= threshold)
        .min_by_key(|(fit, position)| (*fit, (position - start).abs()))
        .map_or(start, |(_, position)| position)
}

#[cfg(test)]
mod tests {
    use super::PositionOutOfRange;
    use crate::{Layout, Mode, OutputEntry, OutputId, OutputState};

    fn monitor(name: &str, width: u32, height: u32, x: i32, y: i32) -> OutputEntry {
        OutputEntry {
            id: OutputId::Connector(name.into()),
            output: None,
            state: OutputState {
                mode: Some(Mode::new(width, height, 60_000)),
                x,
                y,
                ..OutputState::default()
            },
        }
    }

    fn positions(layout: &Layout) -> Vec<(i32, i32)> {
        layout
            .entries
            .iter()
            .map(|entry| (entry.state.x, entry.state.y))
            .collect()
    }

    #[test]
    fn normalising_moves_the_placed_outputs_to_the_origin() {
        let mut disabled = monitor("eDP-1", 1920, 1080, -5000, -5000);
        disabled.state.enabled = false;
        let mut layout = Layout {
            entries: vec![
                monitor("DP-1", 1920, 1080, -1920, 300),
                monitor("DP-2", 1920, 1080, 0, 200),
                disabled,
            ],
        };
        layout.normalise().unwrap();
        assert_eq!(positions(&layout), [(0, 100), (1920, 0), (-3080, -5200)]);
    }

    #[test]
    fn a_layout_with_nothing_placed_is_left_alone() {
        let mut only = monitor("DP-1", 1920, 1080, 40, 40);
        only.state.enabled = false;
        let mut layout = Layout {
            entries: vec![only],
        };
        layout.normalise().unwrap();
        assert_eq!(positions(&layout), [(40, 40)]);
    }

    #[test]
    fn normalising_out_of_range_changes_nothing() {
        let mut far = monitor("eDP-1", 1920, 1080, i32::MAX, 0);
        far.state.enabled = false;
        let mut layout = Layout {
            entries: vec![monitor("DP-1", 1920, 1080, -10, 0), far],
        };
        let before = layout.clone();
        assert_eq!(
            layout.normalise(),
            Err(PositionOutOfRange {
                output: OutputId::Connector("eDP-1".into())
            })
        );
        assert_eq!(layout, before);
    }

    fn beside_dp1(moving: OutputEntry) -> Layout {
        Layout {
            entries: vec![monitor("DP-1", 1920, 1080, 0, 0), moving],
        }
    }

    #[test]
    fn a_drag_near_an_edge_snaps_to_contact() {
        let layout = beside_dp1(monitor("DP-2", 1920, 1080, 0, 0));
        assert_eq!(layout.snap(1, 1925, 3, 10), (1920, 0));
        assert_eq!(layout.snap(1, -1913, -8, 10), (-1920, 0));
    }

    #[test]
    fn a_drag_beyond_the_threshold_stays_where_it_is() {
        let layout = beside_dp1(monitor("DP-2", 1920, 1080, 0, 0));
        assert_eq!(layout.snap(1, 1931, 500, 10), (1931, 500));
    }

    #[test]
    fn contact_wins_over_a_nearer_aligned_edge() {
        // DP-3 sits above and to the right, its left edge at 1925. Dragged to
        // 1924, DP-2 is 1 from aligning with DP-3 and 4 from touching DP-1.
        let layout = Layout {
            entries: vec![
                monitor("DP-1", 1920, 1080, 0, 0),
                monitor("DP-2", 1920, 1080, 0, 0),
                monitor("DP-3", 1920, 1080, 1925, -1080),
            ],
        };
        assert_eq!(layout.snap(1, 1924, 0, 10).0, 1920);
    }

    #[test]
    fn a_smaller_output_below_centres_under_a_larger_one() {
        let layout = beside_dp1(monitor("DP-2", 1280, 720, 0, 0));
        assert_eq!(layout.snap(1, 318, 1085, 10), (320, 1080));
    }

    #[test]
    fn edges_line_up_with_an_output_that_is_not_beside_it() {
        // Far below DP-1, so no contact is possible; the left edges align.
        let layout = beside_dp1(monitor("DP-2", 1280, 720, 0, 0));
        assert_eq!(layout.snap(1, 6, 5000, 10), (0, 5000));
    }

    #[test]
    fn an_edge_far_away_along_the_other_axis_is_no_contact() {
        // Level with DP-1's right edge, but far below it: touching is
        // impossible, so it does not pull.
        let layout = beside_dp1(monitor("DP-2", 1920, 1080, 0, 0));
        assert_eq!(layout.snap(1, 1925, 5000, 10), (1925, 5000));
    }

    #[test]
    fn disabled_outputs_mirrors_and_the_output_itself_do_not_attract() {
        let mut disabled = monitor("eDP-1", 1920, 1080, 0, 0);
        disabled.state.enabled = false;
        let mut mirror = monitor("HDMI-A-1", 1920, 1080, 0, 0);
        mirror.state.mirror_of = Some(OutputId::Connector("eDP-1".into()));
        let layout = Layout {
            entries: vec![disabled, mirror, monitor("DP-1", 1920, 1080, 0, 0)],
        };
        assert_eq!(layout.snap(2, 1925, 3, 10), (1925, 3));
    }

    #[test]
    fn an_output_without_a_size_or_an_index_past_the_end_is_not_snapped() {
        let mut no_mode = monitor("DP-2", 1920, 1080, 0, 0);
        no_mode.state.mode = None;
        let layout = beside_dp1(no_mode);
        assert_eq!(layout.snap(1, 1925, 3, 10), (1925, 3));
        assert_eq!(layout.snap(9, 1925, 3, 10), (1925, 3));
    }
}
