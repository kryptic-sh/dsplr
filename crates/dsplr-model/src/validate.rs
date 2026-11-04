//! Checking a `Layout` against what the active backend accepts
//! (`docs/plan/03-display-model.md#validation`), before anything is tested or
//! applied.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::{Capabilities, Layout, Mode, OutputEntry, OutputId};

/// Two scales closer than this are the same scale. Backends report scales as
/// doubles or `wl_fixed`, so an exact comparison would refuse a scale over
/// its last bit.
const SCALE_EPSILON: f64 = 1e-6;

/// One thing wrong with a layout, naming the output and the setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The output it is about; `None` for the layout as a whole.
    pub output: Option<OutputId>,
    pub kind: ProblemKind,
    pub severity: Severity,
}

/// Whether a problem refuses the layout or only warns about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// The setting a problem is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Enabled,
    Identity,
    Mode,
    Scale,
    Position,
    MirrorOf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    /// No output is enabled.
    NoOutputEnabled,
    /// Two entries have the same identity.
    DuplicateOutput,
    /// An enabled output has no mode.
    NoMode,
    /// The mode is not one the output lists, and the backend takes no custom
    /// modes.
    ModeNotListed,
    /// The scale is not a positive number.
    InvalidScale,
    /// The mode lists its valid scales, and this is not one of them.
    ScaleNotForMode,
    /// A scale that is not a whole number, on a backend without fractional
    /// scaling.
    FractionalScale,
    /// Outputs with different scales, on a backend with one scale for all.
    ScalesDiffer,
    /// The output overlaps another.
    Overlap { with: OutputId },
    /// The output does not touch the others, so the pointer cannot reach it.
    Gap,
    /// A mirror, on a backend without mirroring.
    MirrorUnsupported,
    /// The mirrored output is not in the layout.
    MirrorSourceMissing,
    /// The mirrored output is disabled.
    MirrorSourceDisabled,
    /// The mirrored output is itself a mirror.
    MirrorOfMirror,
}

impl ProblemKind {
    /// The setting the problem is about.
    pub const fn field(&self) -> Field {
        match self {
            Self::NoOutputEnabled => Field::Enabled,
            Self::DuplicateOutput => Field::Identity,
            Self::NoMode | Self::ModeNotListed => Field::Mode,
            Self::InvalidScale
            | Self::ScaleNotForMode
            | Self::FractionalScale
            | Self::ScalesDiffer => Field::Scale,
            Self::Overlap { .. } | Self::Gap => Field::Position,
            Self::MirrorUnsupported
            | Self::MirrorSourceMissing
            | Self::MirrorSourceDisabled
            | Self::MirrorOfMirror => Field::MirrorOf,
        }
    }
}

impl Problem {
    pub const fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error)
    }

    fn error(output: &OutputId, kind: ProblemKind) -> Self {
        Self {
            output: Some(output.clone()),
            kind,
            severity: Severity::Error,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(output) = &self.output {
            write!(f, "{output}: ")?;
        }
        match &self.kind {
            ProblemKind::NoOutputEnabled => f.write_str("no output is enabled"),
            ProblemKind::DuplicateOutput => f.write_str("listed more than once"),
            ProblemKind::NoMode => f.write_str("enabled without a mode"),
            ProblemKind::ModeNotListed => f.write_str(
                "the mode is not one the output lists, and this desktop takes no custom modes",
            ),
            ProblemKind::InvalidScale => f.write_str("the scale is not a positive number"),
            ProblemKind::ScaleNotForMode => f.write_str("the scale is not one this mode supports"),
            ProblemKind::FractionalScale => {
                f.write_str("this desktop takes only whole-number scales")
            }
            ProblemKind::ScalesDiffer => {
                f.write_str("this desktop needs the same scale on every output")
            }
            ProblemKind::Overlap { with } => write!(f, "overlaps {with}"),
            ProblemKind::Gap => f.write_str("does not touch the other outputs"),
            ProblemKind::MirrorUnsupported => f.write_str("this desktop cannot mirror outputs"),
            ProblemKind::MirrorSourceMissing => {
                f.write_str("mirrors an output that is not connected")
            }
            ProblemKind::MirrorSourceDisabled => f.write_str("mirrors a disabled output"),
            ProblemKind::MirrorOfMirror => f.write_str("mirrors an output that is a mirror"),
        }
    }
}

/// An output's place on the canvas, in whole layout pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

impl Rect {
    fn of(entry: &OutputEntry) -> Option<Self> {
        let (width, height) = entry.state.logical_size()?;
        Some(Self {
            x: entry.state.x.into(),
            y: entry.state.y.into(),
            width: width.into(),
            height: height.into(),
        })
    }

    const fn right(self) -> i64 {
        self.x + self.width
    }

    const fn bottom(self) -> i64 {
        self.y + self.height
    }

    /// Whether the two share any area.
    const fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether the two share a stretch of edge the pointer can cross; a
    /// shared corner alone is not one.
    const fn touches(self, other: Self) -> bool {
        let side_by_side = (self.right() == other.x || other.right() == self.x)
            && self.y < other.bottom()
            && other.y < self.bottom();
        let stacked = (self.bottom() == other.y || other.bottom() == self.y)
            && self.x < other.right()
            && other.x < self.right();
        side_by_side || stacked
    }
}

impl Layout {
    /// Every problem with this layout on a backend with `capabilities`, in
    /// entry order, the layout-wide ones first. The layout is refused when
    /// any of them [`is_error`](Problem::is_error); warnings are shown and
    /// do not stop it.
    pub fn validate(&self, capabilities: &Capabilities) -> Vec<Problem> {
        let mut problems = Vec::new();
        if !self.entries.iter().any(|entry| entry.state.enabled) {
            problems.push(Problem {
                output: None,
                kind: ProblemKind::NoOutputEnabled,
                severity: Severity::Error,
            });
        }
        let mut seen = HashSet::new();
        for entry in &self.entries {
            if !seen.insert(&entry.id) {
                problems.push(Problem::error(&entry.id, ProblemKind::DuplicateOutput));
            }
        }
        let enabled: Vec<&OutputEntry> = self
            .entries
            .iter()
            .filter(|entry| entry.state.enabled)
            .collect();
        for entry in &enabled {
            check_mode_and_scale(entry, capabilities, &mut problems);
        }
        if !capabilities.per_output_scale {
            check_one_scale(&enabled, &mut problems);
        }
        self.check_mirrors(capabilities, &mut problems);
        check_placement(&enabled, capabilities, &mut problems);
        problems
    }

    fn check_mirrors(&self, capabilities: &Capabilities, problems: &mut Vec<Problem>) {
        let by_id: HashMap<&OutputId, &OutputEntry> = self
            .entries
            .iter()
            .map(|entry| (&entry.id, entry))
            .collect();
        for entry in self.entries.iter().filter(|entry| entry.state.enabled) {
            let Some(source) = &entry.state.mirror_of else {
                continue;
            };
            let kind = if !capabilities.mirroring {
                ProblemKind::MirrorUnsupported
            } else {
                match by_id.get(source) {
                    None => ProblemKind::MirrorSourceMissing,
                    Some(source) if !source.state.enabled => ProblemKind::MirrorSourceDisabled,
                    Some(source) if source.state.mirror_of.is_some() => ProblemKind::MirrorOfMirror,
                    Some(_) => continue,
                }
            };
            problems.push(Problem::error(&entry.id, kind));
        }
    }
}

fn check_mode_and_scale(
    entry: &OutputEntry,
    capabilities: &Capabilities,
    problems: &mut Vec<Problem>,
) {
    let state = &entry.state;
    let Some(mode) = &state.mode else {
        problems.push(Problem::error(&entry.id, ProblemKind::NoMode));
        return;
    };
    // Without hardware facts (a layout read from a file) there is no list to
    // check the mode against; it is checked once matched to a live output.
    let listed: Option<&Mode> = entry
        .output
        .as_ref()
        .and_then(|output| output.modes.iter().find(|listed| listed.matches(mode)));
    let has_list = entry
        .output
        .as_ref()
        .is_some_and(|output| !output.modes.is_empty());
    if has_list && listed.is_none() && !capabilities.custom_modes {
        problems.push(Problem::error(&entry.id, ProblemKind::ModeNotListed));
    }

    if !(state.scale.is_finite() && state.scale > 0.0) {
        problems.push(Problem::error(&entry.id, ProblemKind::InvalidScale));
        return;
    }
    if !capabilities.fractional_scale && state.scale.fract().abs() > SCALE_EPSILON {
        problems.push(Problem::error(&entry.id, ProblemKind::FractionalScale));
    }
    let scales = listed.map_or(&mode.scales, |listed| &listed.scales);
    if !scales.is_empty()
        && !scales
            .iter()
            .any(|scale| (scale - state.scale).abs() < SCALE_EPSILON)
    {
        problems.push(Problem::error(&entry.id, ProblemKind::ScaleNotForMode));
    }
}

/// On a backend with one scale for every output, each enabled output whose
/// scale differs from the first enabled output's.
fn check_one_scale(enabled: &[&OutputEntry], problems: &mut Vec<Problem>) {
    let Some(first) = enabled.first() else {
        return;
    };
    for entry in &enabled[1..] {
        if (entry.state.scale - first.state.scale).abs() >= SCALE_EPSILON {
            problems.push(Problem::error(&entry.id, ProblemKind::ScalesDiffer));
        }
    }
}

/// Overlaps and gaps between the enabled outputs that are not mirrors and
/// have a size. Each overlap is reported once, on the later output. Gaps are
/// reported on every output not connected, edge to edge, to the first one.
fn check_placement(
    enabled: &[&OutputEntry],
    capabilities: &Capabilities,
    problems: &mut Vec<Problem>,
) {
    let placed: Vec<(&OutputEntry, Rect)> = enabled
        .iter()
        .filter(|entry| entry.state.mirror_of.is_none())
        .filter_map(|entry| Some((*entry, Rect::of(entry)?)))
        .collect();
    for (i, (entry, rect)) in placed.iter().enumerate() {
        for (earlier, earlier_rect) in &placed[..i] {
            if rect.overlaps(*earlier_rect) {
                problems.push(Problem::error(
                    &entry.id,
                    ProblemKind::Overlap {
                        with: earlier.id.clone(),
                    },
                ));
            }
        }
    }

    if placed.len() < 2 {
        return;
    }
    // Overlapping outputs count as connected: the overlap is already
    // reported, and a second problem for the same pair would only repeat it.
    let mut reached = vec![false; placed.len()];
    reached[0] = true;
    let mut queue = vec![0];
    while let Some(i) = queue.pop() {
        for j in 0..placed.len() {
            let (a, b) = (placed[i].1, placed[j].1);
            if !reached[j] && (a.touches(b) || a.overlaps(b)) {
                reached[j] = true;
                queue.push(j);
            }
        }
    }
    let severity = if capabilities.gaps_allowed {
        Severity::Warning
    } else {
        Severity::Error
    };
    for ((entry, _), reached) in placed.iter().zip(reached) {
        if !reached {
            problems.push(Problem {
                output: Some(entry.id.clone()),
                kind: ProblemKind::Gap,
                severity,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Field, Problem, ProblemKind, Severity};
    use crate::{Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState};

    fn id(name: &str) -> OutputId {
        OutputId::Connector(name.into())
    }

    /// An output offering 1920x1080@60 and 2560x1440@144, set to the first.
    fn monitor(name: &str, x: i32, y: i32) -> OutputEntry {
        let mode = Mode::new(1920, 1080, 60_000);
        OutputEntry {
            id: id(name),
            output: Some(Output {
                connector: name.into(),
                description: String::new(),
                physical_size_mm: None,
                modes: vec![mode.clone(), Mode::new(2560, 1440, 144_000)],
            }),
            state: OutputState {
                mode: Some(mode),
                x,
                y,
                ..OutputState::default()
            },
        }
    }

    /// A backend that accepts anything optional, so a test sees only the
    /// problem it sets up.
    fn permissive() -> Capabilities {
        Capabilities {
            custom_modes: true,
            mirroring: true,
            fractional_scale: true,
            per_output_scale: true,
            gaps_allowed: true,
            ..Capabilities::default()
        }
    }

    fn kinds(layout: &Layout, capabilities: &Capabilities) -> Vec<(Option<OutputId>, ProblemKind)> {
        layout
            .validate(capabilities)
            .into_iter()
            .map(|problem| (problem.output, problem.kind))
            .collect()
    }

    fn layout(entries: Vec<OutputEntry>) -> Layout {
        Layout { entries }
    }

    #[test]
    fn two_monitors_side_by_side_are_valid_even_on_the_strictest_backend() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-2", 1920, 0)]);
        assert_eq!(layout.validate(&Capabilities::default()), []);
    }

    #[test]
    fn a_layout_with_nothing_enabled_is_refused() {
        let mut only = monitor("DP-1", 0, 0);
        only.state.enabled = false;
        let problems = layout(vec![only]).validate(&permissive());
        assert_eq!(
            problems,
            [Problem {
                output: None,
                kind: ProblemKind::NoOutputEnabled,
                severity: Severity::Error,
            }]
        );
        assert_eq!(problems[0].kind.field(), Field::Enabled);
    }

    #[test]
    fn the_same_output_twice_is_refused() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-1", 1920, 0)]);
        assert_eq!(
            kinds(&layout, &permissive()),
            [(Some(id("DP-1")), ProblemKind::DuplicateOutput)]
        );
    }

    #[test]
    fn an_enabled_output_needs_a_mode() {
        let mut entry = monitor("DP-1", 0, 0);
        entry.state.mode = None;
        assert_eq!(
            kinds(&layout(vec![entry]), &permissive()),
            [(Some(id("DP-1")), ProblemKind::NoMode)]
        );
    }

    #[test]
    fn an_unlisted_mode_needs_custom_mode_support() {
        let mut entry = monitor("DP-1", 0, 0);
        entry.state.mode = Some(Mode::new(1920, 1080, 75_000));
        let layout = layout(vec![entry]);
        assert_eq!(
            kinds(&layout, &Capabilities::default()),
            [(Some(id("DP-1")), ProblemKind::ModeNotListed)]
        );
        assert_eq!(kinds(&layout, &permissive()), []);
    }

    #[test]
    fn a_mode_matches_its_listed_mode_within_the_refresh_tolerance() {
        let mut entry = monitor("DP-1", 0, 0);
        entry.state.mode = Some(Mode::new(2560, 1440, 143_996));
        assert_eq!(kinds(&layout(vec![entry]), &Capabilities::default()), []);
    }

    #[test]
    fn a_mode_read_from_a_file_is_not_checked_against_a_list_it_lacks() {
        let mut entry = monitor("DP-1", 0, 0);
        entry.output = None;
        entry.state.mode = Some(Mode::new(1234, 567, 89_000));
        assert_eq!(kinds(&layout(vec![entry]), &Capabilities::default()), []);
    }

    #[test]
    fn scales_must_be_positive_numbers() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut entry = monitor("DP-1", 0, 0);
            entry.state.scale = scale;
            assert_eq!(
                kinds(&layout(vec![entry]), &permissive()),
                [(Some(id("DP-1")), ProblemKind::InvalidScale)],
                "scale {scale}"
            );
        }
    }

    #[test]
    fn a_fractional_scale_needs_fractional_scaling() {
        let mut entry = monitor("DP-1", 0, 0);
        entry.state.scale = 1.5;
        let layout = layout(vec![entry]);
        assert_eq!(
            kinds(&layout, &Capabilities::default()),
            [(Some(id("DP-1")), ProblemKind::FractionalScale)]
        );
        assert_eq!(kinds(&layout, &permissive()), []);
    }

    #[test]
    fn a_mode_that_lists_its_scales_accepts_only_those() {
        let mut entry = monitor("DP-1", 0, 0);
        let output = entry.output.as_mut().unwrap();
        output.modes[0].scales = vec![1.0, 1.25];
        entry.state.scale = 1.25;
        assert_eq!(kinds(&layout(vec![entry.clone()]), &permissive()), []);
        entry.state.scale = 1.5;
        assert_eq!(
            kinds(&layout(vec![entry]), &permissive()),
            [(Some(id("DP-1")), ProblemKind::ScaleNotForMode)]
        );
    }

    #[test]
    fn one_scale_for_all_refuses_outputs_that_differ() {
        let mut second = monitor("DP-2", 1920, 0);
        second.state.scale = 2.0;
        let layout = layout(vec![monitor("DP-1", 0, 0), second]);
        let capabilities = Capabilities {
            per_output_scale: false,
            ..permissive()
        };
        assert_eq!(
            kinds(&layout, &capabilities),
            [(Some(id("DP-2")), ProblemKind::ScalesDiffer)]
        );
        assert_eq!(kinds(&layout, &permissive()), []);
    }

    #[test]
    fn an_overlap_is_reported_once_on_the_later_output() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-2", 1919, 0)]);
        assert_eq!(
            kinds(&layout, &permissive()),
            [(Some(id("DP-2")), ProblemKind::Overlap { with: id("DP-1") })]
        );
    }

    #[test]
    fn rotation_and_scale_set_the_size_that_overlaps() {
        // 1920x1080 turned a quarter is 1080 wide; at scale 2 it is 540.
        let mut turned = monitor("DP-1", 0, 0);
        turned.state.transform = crate::Transform::Rotate90;
        turned.state.scale = 2.0;
        let beside = monitor("DP-2", 540, 0);
        assert_eq!(
            kinds(&layout(vec![turned.clone(), beside]), &permissive()),
            []
        );
        let overlapping = monitor("DP-2", 539, 0);
        assert_eq!(
            kinds(&layout(vec![turned, overlapping]), &permissive()),
            [(Some(id("DP-2")), ProblemKind::Overlap { with: id("DP-1") })]
        );
    }

    #[test]
    fn a_gap_is_an_error_unless_the_backend_allows_gaps() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-2", 1921, 0)]);
        let problems = layout.validate(&Capabilities::default());
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::Gap);
        assert!(problems[0].is_error());
        let problems = layout.validate(&permissive());
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, Severity::Warning);
    }

    #[test]
    fn touching_only_at_a_corner_is_a_gap() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-2", 1920, 1080)]);
        assert_eq!(
            kinds(&layout, &permissive()),
            [(Some(id("DP-2")), ProblemKind::Gap)]
        );
    }

    #[test]
    fn two_islands_are_a_gap_even_though_each_output_touches_another() {
        let layout = layout(vec![
            monitor("DP-1", 0, 0),
            monitor("DP-2", 1920, 0),
            monitor("DP-3", 5000, 0),
            monitor("DP-4", 6920, 0),
        ]);
        assert_eq!(
            kinds(&layout, &permissive()),
            [
                (Some(id("DP-3")), ProblemKind::Gap),
                (Some(id("DP-4")), ProblemKind::Gap),
            ]
        );
    }

    #[test]
    fn a_stacked_layout_is_connected() {
        let layout = layout(vec![monitor("DP-1", 0, 0), monitor("DP-2", 500, 1080)]);
        assert_eq!(kinds(&layout, &Capabilities::default()), []);
    }

    #[test]
    fn mirrors_are_neither_placed_nor_gaps() {
        let mut mirror = monitor("HDMI-A-1", 9000, 9000);
        mirror.state.mirror_of = Some(id("DP-1"));
        let layout = layout(vec![monitor("DP-1", 0, 0), mirror]);
        assert_eq!(kinds(&layout, &permissive()), []);
    }

    #[test]
    fn a_mirror_needs_mirroring_and_a_live_unmirrored_source() {
        let mirror_of = |source: &str| {
            let mut mirror = monitor("HDMI-A-1", 0, 0);
            mirror.state.mirror_of = Some(id(source));
            mirror
        };
        let unsupported = layout(vec![monitor("DP-1", 0, 0), mirror_of("DP-1")]);
        assert_eq!(
            kinds(&unsupported, &Capabilities::default()),
            [(Some(id("HDMI-A-1")), ProblemKind::MirrorUnsupported)]
        );

        let missing = layout(vec![monitor("DP-1", 0, 0), mirror_of("DP-9")]);
        assert_eq!(
            kinds(&missing, &permissive()),
            [(Some(id("HDMI-A-1")), ProblemKind::MirrorSourceMissing)]
        );

        let mut disabled_source = monitor("DP-2", 0, 0);
        disabled_source.state.enabled = false;
        let disabled = layout(vec![
            monitor("DP-1", 0, 0),
            disabled_source,
            mirror_of("DP-2"),
        ]);
        assert_eq!(
            kinds(&disabled, &permissive()),
            [(Some(id("HDMI-A-1")), ProblemKind::MirrorSourceDisabled)]
        );

        let mut source_mirror = monitor("DP-2", 0, 0);
        source_mirror.state.mirror_of = Some(id("DP-1"));
        let chained = layout(vec![
            monitor("DP-1", 0, 0),
            source_mirror,
            mirror_of("DP-2"),
        ]);
        assert_eq!(
            kinds(&chained, &permissive()),
            [(Some(id("HDMI-A-1")), ProblemKind::MirrorOfMirror)]
        );
    }

    #[test]
    fn a_problem_reads_as_its_output_and_what_is_wrong() {
        let problem = Problem {
            output: Some(id("DP-2")),
            kind: ProblemKind::Overlap { with: id("DP-1") },
            severity: Severity::Error,
        };
        assert_eq!(problem.to_string(), "DP-2: overlaps DP-1");
    }
}
