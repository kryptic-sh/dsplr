//! The property panel for the selected output, as data: each field
//! `docs/plan/06-gui.md` lists, its options, whether a pending edit changed
//! it, and why it cannot be changed when it cannot. `dsplr-gui` builds the
//! widgets from it and hands the choices back to the [`Editor`]'s edits.

use dsplr_model::{
    AdaptiveSync, Capabilities, Mode, OVERSCAN_MAX, OutputEntry, OutputId, OutputState, RgbRange,
    SCALE_EPSILON, Transform,
};

use crate::Editor;
use crate::editor::scale_choices;

/// Why every field but `enabled` is locked on an output that is off.
pub const OFF_REASON: &str = "turn the output on to change this";

/// Why a setting the backend has is locked on an output it did not report
/// it for (GNOME on a monitor without an RGB range): nothing says what it
/// can be set to.
pub const NOT_REPORTED: &str = "the desktop reports no value for this output";

/// Why the primary checkbox is locked while checked: KDE and GNOME always
/// have a primary output, so unchecking it would apply as nothing.
pub const PRIMARY_KEPT: &str = "the desktop always has a primary; make another output primary";

/// Why a mirror cannot be made primary: it shows its source.
pub const PRIMARY_MIRROR: &str = "a mirror shows its source; make the source primary";

/// The overscans offered, in percent: fine steps up to 10, then two
/// coarse ones. An output's own value is offered too when not among them.
const OVERSCANS: [u8; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 20];

/// One field: its value, whether it differs from the live one, and why it
/// cannot be changed, if it cannot.
#[derive(Debug, Clone, PartialEq)]
pub struct Field<T> {
    pub value: T,
    pub changed: bool,
    pub locked: Option<String>,
}

/// A field chosen from a list: each option's label and value, and which is
/// chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice<T> {
    pub options: Vec<(String, T)>,
    pub chosen: Option<usize>,
}

impl<T> Choice<T> {
    pub fn labels(&self) -> Vec<&str> {
        self.options
            .iter()
            .map(|(label, _)| label.as_str())
            .collect()
    }
}

/// The panel for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub output: String,
    pub description: String,
    pub enabled: Field<bool>,
    /// The mode's size, among the sizes the output lists.
    pub size: Field<Choice<(u32, u32)>>,
    /// The mode, among those the output lists at its size.
    pub refresh: Field<Choice<Mode>>,
    /// The mode as `WxH@R`, which a custom mode is typed as.
    pub custom_mode: Field<String>,
    pub position: Field<String>,
    /// The scale, among the scales offered, and as a number.
    pub scale: Field<(Choice<f64>, f64)>,
    pub transform: Field<Choice<Transform>>,
    pub adaptive_sync: Field<Choice<Option<AdaptiveSync>>>,
    pub mirror: Field<Choice<Option<OutputId>>>,
    /// Whether it is the primary output; `None` where the backend has none.
    pub primary: Option<Field<bool>>,
    /// Its overscan, in percent; `None` where the backend has none.
    pub overscan: Option<Field<Choice<u8>>>,
    /// Its RGB range; `None` where the backend has none.
    pub rgb_range: Option<Field<Choice<RgbRange>>>,
    /// The settings only some backends have that are shown and not edited
    /// here (Hyprland's colour preset and bit depth, `docs/backlog.md`):
    /// label, value, changed.
    pub other: Vec<(&'static str, String, bool)>,
}

/// An output's name as the canvas and panel head it: its connector, else
/// the name its identity carries.
pub fn name(entry: &OutputEntry) -> String {
    entry
        .connector()
        .map_or_else(|| entry.id.to_string(), ToOwned::to_owned)
}

/// The panel for output `index`, or `None` when there is no such output.
pub fn panel(editor: &Editor, index: usize) -> Option<Panel> {
    let live = &editor.live().entries.get(index)?.state;
    let entry = editor.pending().entries.get(index)?;
    let state = &entry.state;
    let snapshot = editor.snapshot();
    let capabilities = &snapshot.capabilities;
    let backend = &snapshot.backend;
    let off = (!state.enabled).then(|| OFF_REASON.to_owned());
    let locked = |cannot: Option<String>| off.clone().or(cannot);
    let listed = editor.listed_modes(index);
    let no_modes = listed
        .is_empty()
        .then(|| "the desktop lists no modes for this output".to_owned());
    let size_of = |state: &OutputState| state.mode.as_ref().map(|mode| (mode.width, mode.height));
    let refresh_of = |state: &OutputState| state.mode.as_ref().map(|mode| mode.refresh_mhz);

    Some(Panel {
        output: name(entry),
        description: entry.description().unwrap_or_else(|| "unknown".into()),
        enabled: Field {
            value: state.enabled,
            changed: state.enabled != live.enabled,
            locked: None,
        },
        size: field(
            sizes(listed, state.mode.as_ref()),
            size_of(state) != size_of(live),
            locked(no_modes.clone()),
        ),
        refresh: field(
            refreshes(listed, state.mode.as_ref()),
            refresh_of(state) != refresh_of(live),
            locked(no_modes),
        ),
        custom_mode: field(
            state.mode.as_ref().map(spec_text).unwrap_or_default(),
            state.mode != live.mode,
            locked(
                (!capabilities.custom_modes).then(|| format!("{backend} takes no custom modes")),
            ),
        ),
        position: field(
            format!("{}, {}", state.x, state.y),
            (state.x, state.y) != (live.x, live.y),
            locked(Some("drag the output, or move it with h j k l".to_owned())),
        ),
        scale: field(
            (scales(state, capabilities), state.scale),
            state.scale != live.scale,
            locked(None),
        ),
        transform: field(
            choose(
                Transform::ALL.map(|t| (t.to_string(), t)).to_vec(),
                &state.transform,
            ),
            state.transform != live.transform,
            locked(None),
        ),
        adaptive_sync: field(
            adaptive_sync(state.adaptive_sync, capabilities),
            state.adaptive_sync != live.adaptive_sync,
            locked(
                capabilities
                    .adaptive_sync
                    .is_empty()
                    .then(|| format!("{backend} cannot set adaptive sync")),
            ),
        ),
        mirror: field(
            mirrors(editor, index, state.mirror_of.as_ref()),
            state.mirror_of != live.mirror_of,
            locked((!capabilities.mirroring).then(|| format!("{backend} cannot mirror outputs"))),
        ),
        primary: capabilities.primary.then(|| {
            let why = if state.primary {
                Some(PRIMARY_KEPT)
            } else if state.mirror_of.is_some() {
                Some(PRIMARY_MIRROR)
            } else {
                None
            };
            field(
                state.primary,
                state.primary != live.primary,
                locked(why.map(str::to_owned)),
            )
        }),
        overscan: capabilities.overscan.then(|| {
            field(
                overscans(state.overscan),
                state.overscan != live.overscan,
                locked(live.overscan.is_none().then(|| NOT_REPORTED.to_owned())),
            )
        }),
        rgb_range: capabilities.rgb_range.then(|| {
            let options = [RgbRange::Automatic, RgbRange::Full, RgbRange::Limited]
                .map(|range| (range.to_string(), range))
                .to_vec();
            let chosen = state
                .rgb_range
                .and_then(|current| options.iter().position(|(_, range)| *range == current));
            field(
                Choice { options, chosen },
                state.rgb_range != live.rgb_range,
                locked(live.rgb_range.is_none().then(|| NOT_REPORTED.to_owned())),
            )
        }),
        other: other(state, live, capabilities),
    })
}

/// `mode` as a custom mode is typed, which `ModeSpec` reads back:
/// `1920x1080@60`, or `1920x1080` when the refresh is not known.
fn spec_text(mode: &Mode) -> String {
    if mode.refresh_mhz == 0 {
        format!("{}x{}", mode.width, mode.height)
    } else {
        mode.to_string()
    }
}

fn field<T>(value: T, changed: bool, locked: Option<String>) -> Field<T> {
    Field {
        value,
        changed,
        locked,
    }
}

/// `options`, with the one equal to `current` chosen.
fn choose<T: PartialEq>(options: Vec<(String, T)>, current: &T) -> Choice<T> {
    let chosen = options.iter().position(|(_, value)| value == current);
    Choice { options, chosen }
}

/// The sizes `listed` holds, largest first, each once, with `current`'s size
/// added when not among them (a custom mode).
fn sizes(listed: &[Mode], current: Option<&Mode>) -> Choice<(u32, u32)> {
    let mut sizes: Vec<(u32, u32)> = listed.iter().map(|m| (m.width, m.height)).collect();
    sizes.extend(current.map(|m| (m.width, m.height)));
    sizes.sort_unstable_by_key(|&(w, h)| std::cmp::Reverse((u64::from(w) * u64::from(h), w)));
    sizes.dedup();
    let options = sizes
        .into_iter()
        .map(|(w, h)| (format!("{w}x{h}"), (w, h)))
        .collect();
    match current {
        Some(mode) => choose(options, &(mode.width, mode.height)),
        None => Choice {
            options,
            chosen: None,
        },
    }
}

/// The modes `listed` holds at `current`'s size, highest refresh first, with
/// `current` added when not among them; the option matching it is chosen.
fn refreshes(listed: &[Mode], current: Option<&Mode>) -> Choice<Mode> {
    let Some(current) = current else {
        return Choice {
            options: Vec::new(),
            chosen: None,
        };
    };
    let mut modes: Vec<&Mode> = listed
        .iter()
        .filter(|m| (m.width, m.height) == (current.width, current.height))
        .collect();
    if !modes.iter().any(|m| m.matches(current)) {
        modes.push(current);
    }
    modes.sort_by_key(|m| std::cmp::Reverse(m.refresh_mhz));
    // Hyprland lists some modes twice; they read the same.
    modes.dedup_by(|a, b| a.matches(b));
    let chosen = modes.iter().position(|m| m.matches(current));
    Choice {
        options: modes
            .into_iter()
            .map(|m| (m.refresh_text(), m.clone()))
            .collect(),
        chosen,
    }
}

/// [`scale_choices`] with `state`'s own scale added when not among them.
fn scales(state: &OutputState, capabilities: &Capabilities) -> Choice<f64> {
    let mut scales = scale_choices(state.mode.as_ref(), capabilities);
    let current = state.scale;
    if !scales
        .iter()
        .any(|&scale| (scale - current).abs() < SCALE_EPSILON)
    {
        scales.push(current);
        scales.sort_by(f64::total_cmp);
    }
    let chosen = scales
        .iter()
        .position(|&scale| (scale - current).abs() < SCALE_EPSILON);
    Choice {
        options: scales.into_iter().map(|s| (s.to_string(), s)).collect(),
        chosen,
    }
}

/// The adaptive sync settings the backend takes, after `unknown` when the
/// backend did not say which is in force.
fn adaptive_sync(
    current: Option<AdaptiveSync>,
    capabilities: &Capabilities,
) -> Choice<Option<AdaptiveSync>> {
    let unknown = current.is_none().then(|| ("unknown".to_owned(), None));
    let options = unknown
        .into_iter()
        .chain(
            capabilities
                .adaptive_sync
                .iter()
                .map(|&sync| (sync.to_string(), Some(sync))),
        )
        .collect();
    choose(options, &current)
}

/// `none`, then every other output by name.
fn mirrors(editor: &Editor, index: usize, current: Option<&OutputId>) -> Choice<Option<OutputId>> {
    let others = editor
        .pending()
        .entries
        .iter()
        .enumerate()
        .filter(|&(other, _)| other != index)
        .map(|(_, entry)| (name(entry), Some(entry.id.clone())));
    let options = std::iter::once(("none".to_owned(), None))
        .chain(others)
        .collect();
    choose(options, &current.cloned())
}

/// [`OVERSCANS`], with `current` added in its place when not among them;
/// none chosen when it is not known.
fn overscans(current: Option<u8>) -> Choice<u8> {
    let mut percents = OVERSCANS.to_vec();
    if let Some(current) = current.filter(|c| *c <= OVERSCAN_MAX && !percents.contains(c)) {
        percents.push(current);
        percents.sort_unstable();
    }
    let options = percents
        .into_iter()
        .map(|p| (format!("{p} %"), p))
        .collect();
    match current {
        Some(current) => choose(options, &current),
        None => Choice {
            options,
            chosen: None,
        },
    }
}

/// The settings only some backends have that the panel shows and does not
/// edit, on a backend that has them.
fn other(
    state: &OutputState,
    live: &OutputState,
    capabilities: &Capabilities,
) -> Vec<(&'static str, String, bool)> {
    let default = || "default".to_owned();
    let mut rows = Vec::new();
    if capabilities.colour_preset {
        rows.push((
            "Colour preset",
            state.colour_preset.clone().unwrap_or_else(default),
            state.colour_preset != live.colour_preset,
        ));
    }
    if capabilities.bit_depth {
        rows.push((
            "Bit depth",
            state
                .bit_depth
                .map_or_else(default, |bits| format!("{bits} bits")),
            state.bit_depth != live.bit_depth,
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Mode, RgbRange};

    use super::{Choice, Field, NOT_REPORTED, OFF_REASON, PRIMARY_MIRROR, Panel, panel};
    use crate::Editor;
    use crate::fixtures::owners_machine;

    fn choice<T>(field: &Field<Choice<T>>) -> String {
        let labels = field.value.labels();
        let chosen = field.value.chosen.map_or("-", |i| labels[i]);
        format!("{chosen} of [{}]{}", labels.join(", "), marks(field))
    }

    fn marks<T>(field: &Field<T>) -> String {
        let changed = if field.changed { " *" } else { "" };
        let locked = field
            .locked
            .as_ref()
            .map_or(String::new(), |why| format!(" (locked: {why})"));
        format!("{changed}{locked}")
    }

    /// Each field as a line.
    fn text(panel: &Panel) -> Vec<String> {
        let mut lines = vec![
            format!("output {} / {}", panel.output, panel.description),
            format!("enabled {}{}", panel.enabled.value, marks(&panel.enabled)),
            format!("size {}", choice(&panel.size)),
            format!("refresh {}", choice(&panel.refresh)),
            format!(
                "custom {}{}",
                panel.custom_mode.value,
                marks(&panel.custom_mode)
            ),
            format!(
                "position {}{}",
                panel.position.value,
                marks(&panel.position)
            ),
            format!(
                "scale {} ({}){}",
                panel.scale.value.0.chosen.map_or_else(
                    || "-".to_owned(),
                    |i| panel.scale.value.0.options[i].0.clone()
                ),
                panel.scale.value.1,
                marks(&panel.scale)
            ),
            format!("transform {}", choice(&panel.transform)),
            format!("vrr {}", choice(&panel.adaptive_sync)),
            format!("mirror {}", choice(&panel.mirror)),
        ];
        lines.extend(
            panel
                .primary
                .as_ref()
                .map(|field| format!("primary {}{}", field.value, marks(field))),
        );
        lines.extend(
            panel
                .overscan
                .as_ref()
                .map(|field| format!("overscan {}", choice(field))),
        );
        lines.extend(
            panel
                .rgb_range
                .as_ref()
                .map(|field| format!("rgb {}", choice(field))),
        );
        lines.extend(panel.other.iter().map(|(label, value, changed)| {
            format!("{label} {value}{}", if *changed { " *" } else { "" })
        }));
        lines
    }

    #[test]
    fn the_owners_monitor_on_hyprland() {
        let editor = Editor::new(owners_machine());
        assert_eq!(
            text(&panel(&editor, 1).unwrap()),
            [
                "output HDMI-A-1 / AOC Q27G4 SERIAL0000001",
                "enabled true",
                "size 2560x1440 of [2560x1440]",
                "refresh 120 Hz of [120 Hz]",
                "custom 2560x1440@120 (locked: hyprland takes no custom modes)",
                "position 0, 0 (locked: drag the output, or move it with h j k l)",
                "scale 1 (1)",
                "transform normal of [normal, 90, 180, 270, flipped, flipped-90, flipped-180, flipped-270]",
                "vrr off of [off, on]",
                "mirror none of [none, eDP-1]",
                "Colour preset default",
                "Bit depth default",
            ]
        );
        assert_eq!(panel(&editor, 2), None);
    }

    #[test]
    fn primary_overscan_and_rgb_range_are_edited_where_the_backend_takes_them() {
        let mut snapshot = owners_machine();
        snapshot.backend = "kde".into();
        snapshot.capabilities.primary = true;
        snapshot.capabilities.overscan = true;
        snapshot.capabilities.rgb_range = true;
        snapshot.capabilities.colour_preset = false;
        snapshot.capabilities.bit_depth = false;
        snapshot.layout.entries[1].state.overscan = Some(3);
        snapshot.layout.entries[1].state.rgb_range = Some(RgbRange::Full);
        let mut editor = Editor::new(snapshot);
        assert!(editor.set_primary(1, true));
        let lines = text(&panel(&editor, 1).unwrap());
        assert_eq!(
            lines[10..],
            [
                "primary true * (locked: the desktop always has a primary; make another output primary)",
                "overscan 3 % of [0 %, 1 %, 2 %, 3 %, 4 %, 5 %, 6 %, 7 %, 8 %, 9 %, 10 %, 15 %, 20 %]",
                "rgb full of [automatic, full, limited]",
            ]
        );
        // Not reported for an output: nothing to set it from.
        let laptop = panel(&editor, 0).unwrap();
        let rgb = laptop.rgb_range.unwrap();
        assert_eq!(rgb.locked.as_deref(), Some(OFF_REASON), "and off");
        assert_eq!(laptop.primary.unwrap().locked.as_deref(), Some(OFF_REASON));
        // Not primary, it can be made so; a mirror cannot.
        assert!(editor.set_primary(1, false));
        assert_eq!(panel(&editor, 1).unwrap().primary.unwrap().locked, None);
        editor.edit(1, |state| {
            state.mirror_of = Some(dsplr_model::OutputId::Connector("eDP-1".into()));
        });
        assert_eq!(
            panel(&editor, 1)
                .unwrap()
                .primary
                .unwrap()
                .locked
                .as_deref(),
            Some(PRIMARY_MIRROR)
        );
        let mut snapshot = owners_machine();
        snapshot.capabilities.rgb_range = true;
        snapshot.capabilities.overscan = true;
        let editor = Editor::new(snapshot);
        let monitor = panel(&editor, 1).unwrap();
        assert_eq!(
            monitor.rgb_range.unwrap().locked.as_deref(),
            Some(NOT_REPORTED)
        );
        assert_eq!(
            monitor.overscan.unwrap().locked.as_deref(),
            Some(NOT_REPORTED)
        );
        // Where the backend has none, no row at all.
        let editor = Editor::new(owners_machine());
        let monitor = panel(&editor, 1).unwrap();
        assert!(monitor.primary.is_none() && monitor.overscan.is_none());
        assert!(monitor.rgb_range.is_none());
    }

    #[test]
    fn an_output_that_is_off_locks_all_but_turning_it_on() {
        let editor = Editor::new(owners_machine());
        let laptop = panel(&editor, 0).unwrap();
        assert_eq!(laptop.enabled.locked, None);
        for locked in [
            &laptop.size.locked,
            &laptop.refresh.locked,
            &laptop.custom_mode.locked,
            &laptop.scale.locked,
            &laptop.transform.locked,
            &laptop.adaptive_sync.locked,
            &laptop.mirror.locked,
        ] {
            assert_eq!(locked.as_deref(), Some(OFF_REASON));
        }
        assert_eq!(laptop.size.value.chosen, None, "no mode: nothing chosen");
        assert_eq!(laptop.size.value.labels(), ["1920x1080"]);
    }

    #[test]
    fn a_pending_change_marks_its_fields_and_unlisted_values_are_offered() {
        let mut editor = Editor::new(owners_machine());
        editor.edit(1, |state| {
            state.mode = Some(Mode::new(1920, 1080, 0));
            state.scale = 1.3;
            state.adaptive_sync = None;
        });
        let lines = text(&panel(&editor, 1).unwrap());
        assert_eq!(lines[2], "size 1920x1080 of [2560x1440, 1920x1080] *");
        assert_eq!(lines[3], "refresh unknown of [unknown] *");
        assert_eq!(
            lines[4], "custom 1920x1080 * (locked: hyprland takes no custom modes)",
            "no @0 to type back"
        );
        assert_eq!(
            lines[6], "scale 1.3 (1.3) *",
            "an unlisted scale is offered too"
        );
        assert_eq!(lines[8], "vrr unknown of [unknown, off, on] *");
        assert_eq!(lines[1], "enabled true", "unchanged");
    }
}
