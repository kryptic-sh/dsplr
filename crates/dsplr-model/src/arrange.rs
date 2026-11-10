//! Arranging outputs on the canvas: snapping a dragged output to the others,
//! lifting the layout off negative positions on release, and normalising it
//! to the origin where a backend needs that
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
    /// every other backend accepts. Mirrors move by the same amount, staying
    /// on their sources; outputs that are off stay where they are, since no
    /// desktop shows where they sit, and moving them would make a change
    /// that applies as nothing. A layout with nothing placed is left as it
    /// is.
    ///
    /// # Errors
    ///
    /// When an output would land outside `i32`; the layout is then unchanged.
    pub fn normalise(&mut self) -> Result<(), PositionOutOfRange> {
        let Some((min_x, min_y)) = self.placed_min() else {
            return Ok(());
        };
        self.translate(-i64::from(min_x), -i64::from(min_y))
    }

    /// Moves every enabled output by the least amount that leaves no
    /// enabled, non-mirrored output at a negative x or y: along an axis only
    /// where one is negative, and not at all when none is. Outputs that are
    /// off stay where they are, as in [`Layout::normalise`]. Unlike
    /// [`Layout::normalise`], a layout whose outputs start away from the
    /// origin (sway's at x 1920) keeps its place, so an edit to one output
    /// moves no other unless it has to.
    ///
    /// # Errors
    ///
    /// When an output would land outside `i32`; the layout is then unchanged.
    pub fn make_non_negative(&mut self) -> Result<(), PositionOutOfRange> {
        let Some((min_x, min_y)) = self.placed_min() else {
            return Ok(());
        };
        let lift = |min: i32| (-i64::from(min)).max(0);
        match (lift(min_x), lift(min_y)) {
            (0, 0) => Ok(()),
            (dx, dy) => self.translate(dx, dy),
        }
    }

    /// The smallest x and the smallest y among the placed outputs.
    pub(crate) fn placed_min(&self) -> Option<(i32, i32)> {
        let min_x = placed(&self.entries).map(|(_, e)| e.state.x).min()?;
        let min_y = placed(&self.entries).map(|(_, e)| e.state.y).min()?;
        Some((min_x, min_y))
    }

    /// Moves every enabled output (mirrors too) by `dx`, `dy`, or none when
    /// one would land outside `i32`. Outputs that are off stay.
    fn translate(&mut self, dx: i64, dy: i64) -> Result<(), PositionOutOfRange> {
        let shift = |position: i32, by: i64| i32::try_from(i64::from(position) + by);
        let moved = self
            .entries
            .iter()
            .map(|entry| {
                if !entry.state.enabled {
                    return Ok((entry.state.x, entry.state.y));
                }
                let out_of_range = |_| PositionOutOfRange {
                    output: entry.id.clone(),
                };
                Ok((
                    shift(entry.state.x, dx).map_err(out_of_range)?,
                    shift(entry.state.y, dy).map_err(out_of_range)?,
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
            .and_then(|entry| entry.state.size(self.extent_mode()))
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
            .filter_map(|(_, entry)| Rect::of(entry, self.extent_mode()))
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
    fn normalising_moves_the_placed_outputs_to_the_origin_and_mirrors_with_them() {
        let mut disabled = monitor("eDP-1", 1920, 1080, -5000, -5000);
        disabled.state.enabled = false;
        let mut mirror = monitor("HDMI-A-1", 1920, 1080, -1920, 300);
        mirror.state.mirror_of = Some(OutputId::Connector("DP-1".into()));
        let mut layout = Layout {
            entries: vec![
                monitor("DP-1", 1920, 1080, -1920, 300),
                monitor("DP-2", 1920, 1080, 0, 200),
                disabled,
                mirror,
            ],
            ..Default::default()
        };
        layout.normalise().unwrap();
        // The mirror stays on its source; the output that is off stays
        // where it was, so moving the rest changes nothing about it.
        assert_eq!(
            positions(&layout),
            [(0, 100), (1920, 0), (-5000, -5000), (0, 100)]
        );
    }

    #[test]
    fn a_layout_with_nothing_placed_is_left_alone() {
        let mut only = monitor("DP-1", 1920, 1080, 40, 40);
        only.state.enabled = false;
        let mut layout = Layout {
            entries: vec![only],
            ..Default::default()
        };
        layout.normalise().unwrap();
        assert_eq!(positions(&layout), [(40, 40)]);
    }

    #[test]
    fn normalising_out_of_range_changes_nothing() {
        let mut far = monitor("eDP-1", 1920, 1080, i32::MAX, 0);
        far.state.mirror_of = Some(OutputId::Connector("DP-1".into()));
        let mut layout = Layout {
            entries: vec![monitor("DP-1", 1920, 1080, -10, 0), far],
            ..Default::default()
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

    #[test]
    fn making_non_negative_moves_nothing_away_from_the_origin() {
        // sway's headless outputs start at x 1920.
        let mut layout = Layout {
            entries: vec![
                monitor("HEADLESS-2", 1920, 1080, 1920, 0),
                monitor("HEADLESS-1", 1280, 720, 3840, 0),
            ],
            ..Default::default()
        };
        let before = layout.clone();
        layout.make_non_negative().unwrap();
        assert_eq!(layout, before);
    }

    #[test]
    fn making_non_negative_lifts_only_the_axis_that_went_negative() {
        let mut disabled = monitor("eDP-1", 1920, 1080, -5000, 40);
        disabled.state.enabled = false;
        let mut layout = Layout {
            entries: vec![
                monitor("DP-1", 1920, 1080, -500, 300),
                monitor("DP-2", 1920, 1080, 1920, 200),
                disabled,
            ],
            ..Default::default()
        };
        layout.make_non_negative().unwrap();
        assert_eq!(
            positions(&layout),
            [(0, 300), (2420, 200), (-5000, 40)],
            "x by 500, y not at all; the output that is off not at all"
        );
    }

    #[test]
    fn making_non_negative_out_of_range_changes_nothing() {
        let mut far = monitor("eDP-1", 1920, 1080, i32::MAX, 0);
        far.state.mirror_of = Some(OutputId::Connector("DP-1".into()));
        let mut layout = Layout {
            entries: vec![monitor("DP-1", 1920, 1080, -10, 0), far],
            ..Default::default()
        };
        let before = layout.clone();
        assert!(layout.make_non_negative().is_err());
        assert_eq!(layout, before);
    }

    fn beside_dp1(moving: OutputEntry) -> Layout {
        Layout {
            entries: vec![monitor("DP-1", 1920, 1080, 0, 0), moving],
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
