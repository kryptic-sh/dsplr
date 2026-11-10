//! Taking a new read of the desktop (after a request, or because the
//! desktop changed) as the live layout, under the pending edits.
//!
//! The two reads are paired by `dsplr_model::pair`, the matcher a saved
//! layout is paired with, so twins with one EDID identity keep their own
//! connectors. Each setting of each output is merged on its own: the pending
//! value where the user changed it (it differs from the old live one), the
//! new live one everywhere else, so a change another program made to a
//! setting the user did not touch is never undone by an apply.

use dsplr_model::{Layout, OutputEntry, OutputState, Setting, pair};

use super::Editor;
use crate::Snapshot;
use crate::panel::name;

/// What taking a new read did to the pending edits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rebased {
    /// The outputs gone whose pending edits went with them, by name.
    pub dropped: Vec<String>,
    /// The settings the user edited that the desktop changed too, the
    /// user's value kept: the output's name and the setting.
    pub kept_over: Vec<(String, Setting)>,
}

impl Rebased {
    /// What the bar says about it, `None` when there is nothing to say.
    pub fn notice(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.dropped.is_empty() {
            parts.push(format!(
                "{} went away, and the pending edits to it with it",
                self.dropped.join(", ")
            ));
        }
        if !self.kept_over.is_empty() {
            let settings: Vec<String> = self
                .kept_over
                .iter()
                .map(|(output, setting)| format!("{output} {setting}"))
                .collect();
            parts.push(format!(
                "{} changed on the desktop too; your pending edit is kept",
                settings.join(", ")
            ));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

impl Editor {
    /// Takes `snapshot` as the live layout.
    ///
    /// Unless `keep_pending`, the pending layout starts again from it, the
    /// undo history and any drag going with the old one. With it, a read
    /// whose layout is the old one changes nothing else: the edits, their
    /// history and a drag under way all stay. With the layout changed, each
    /// output still there takes the new live settings but for those the
    /// user edited, a new output comes in as it is, an edited output gone
    /// takes its edits with it, and the undo history goes, since its steps
    /// hold the old live settings. A drag under way goes on over the new
    /// layout while its output is still there. The selection follows its
    /// output.
    pub fn rebase(&mut self, snapshot: Snapshot, keep_pending: bool) -> Rebased {
        if keep_pending && snapshot.layout == self.snapshot.layout {
            self.snapshot = snapshot;
            return Rebased::default();
        }
        let old_live = &self.snapshot.layout.entries;
        let new_live = &snapshot.layout.entries;
        // For each old entry, its new one; the pending layout has the old
        // live one's entries in its order.
        let pairs = pair(old_live, new_live);
        let selected = self
            .selected
            .and_then(|index| pairs.get(index).copied().flatten());
        let mut rebased = Rebased::default();
        if keep_pending {
            let mut from = vec![None; new_live.len()];
            for (old, new) in pairs.iter().enumerate() {
                if let Some(new) = *new {
                    from[new] = Some(old);
                }
            }
            let drag = self.drag.take().and_then(|drag| {
                let output = pairs[drag.output()]?;
                let before = merge(
                    old_live,
                    &drag.before().entries,
                    new_live,
                    &from,
                    &mut Vec::new(),
                );
                Some(drag.carried(output, before))
            });
            let pending = merge(
                old_live,
                &self.pending.entries,
                new_live,
                &from,
                &mut rebased.kept_over,
            );
            rebased.dropped = pairs
                .iter()
                .enumerate()
                .filter(|&(old, new)| new.is_none() && self.pending.entries[old] != old_live[old])
                .map(|(old, _)| name(&self.pending.entries[old]))
                .collect();
            self.pending = pending;
            self.drag = drag;
        } else {
            self.drag = None;
            self.pending = snapshot.layout.clone();
        }
        self.undo.clear();
        self.redo.clear();
        self.snapshot = snapshot;
        self.selected = selected.or_else(|| self.order().first().copied());
        rebased
    }
}

/// The pending layout over `new_live`: each new entry with its old one's
/// edits (`from` names the old entry of each new one), the settings the
/// desktop also changed under an edit pushed onto `kept_over`.
fn merge(
    old_live: &[OutputEntry],
    pending: &[OutputEntry],
    new_live: &[OutputEntry],
    from: &[Option<usize>],
    kept_over: &mut Vec<(String, Setting)>,
) -> Layout {
    let entries = new_live
        .iter()
        .zip(from)
        .map(|(new, from)| match *from {
            Some(old) => {
                let mut settings = Vec::new();
                let state = merge_state(
                    &old_live[old].state,
                    &pending[old].state,
                    &new.state,
                    &mut settings,
                );
                kept_over.extend(settings.into_iter().map(|setting| (name(new), setting)));
                OutputEntry {
                    state,
                    ..new.clone()
                }
            }
            None => new.clone(),
        })
        .collect();
    Layout { entries }
}

/// One output's settings: `pending`'s where they differ from `old` (the user
/// changed them), `new`'s everywhere else. A setting the user changed that
/// `new` changed too, to something else, is pushed onto `kept_over`.
fn merge_state(
    old: &OutputState,
    pending: &OutputState,
    new: &OutputState,
    kept_over: &mut Vec<Setting>,
) -> OutputState {
    // Named field by field, so a setting added to `OutputState` fails to
    // compile here until it is merged too.
    let OutputState {
        enabled,
        mode,
        x,
        y,
        scale,
        transform,
        adaptive_sync,
        mirror_of,
        primary,
        colour_preset,
        bit_depth,
        overscan,
        rgb_range,
    } = pending;
    let mut pick = Picker { kept_over };
    let (x, y) = pick.one(
        Setting::Position,
        &(old.x, old.y),
        &(*x, *y),
        &(new.x, new.y),
    );
    OutputState {
        enabled: pick.one(Setting::Enabled, &old.enabled, enabled, &new.enabled),
        mode: pick.one(Setting::Mode, &old.mode, mode, &new.mode),
        x,
        y,
        scale: pick.one(Setting::Scale, &old.scale, scale, &new.scale),
        transform: pick.one(
            Setting::Transform,
            &old.transform,
            transform,
            &new.transform,
        ),
        adaptive_sync: pick.one(
            Setting::AdaptiveSync,
            &old.adaptive_sync,
            adaptive_sync,
            &new.adaptive_sync,
        ),
        mirror_of: pick.one(Setting::MirrorOf, &old.mirror_of, mirror_of, &new.mirror_of),
        primary: pick.one(Setting::Primary, &old.primary, primary, &new.primary),
        colour_preset: pick.one(
            Setting::ColourPreset,
            &old.colour_preset,
            colour_preset,
            &new.colour_preset,
        ),
        bit_depth: pick.one(Setting::BitDepth, &old.bit_depth, bit_depth, &new.bit_depth),
        overscan: pick.one(Setting::Overscan, &old.overscan, overscan, &new.overscan),
        rgb_range: pick.one(Setting::RgbRange, &old.rgb_range, rgb_range, &new.rgb_range),
    }
}

/// Picks each setting for [`merge_state`].
struct Picker<'a> {
    kept_over: &'a mut Vec<Setting>,
}

impl Picker<'_> {
    /// `pending` where the user changed it from `old`, else `new`.
    fn one<T: PartialEq + Clone>(&mut self, setting: Setting, old: &T, pending: &T, new: &T) -> T {
        if pending == old {
            return new.clone();
        }
        if new != old && new != pending {
            self.kept_over.push(setting);
        }
        pending.clone()
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Mode, Output, OutputEntry, OutputId, Setting, Transform};

    use super::Rebased;
    use crate::Editor;
    use crate::fixtures::three_monitors;

    /// DP-1 at 0,360 (1920x1080), DP-2 at 1920,0 turned, HDMI-A-1 off.
    fn editor() -> Editor {
        Editor::new(three_monitors())
    }

    fn index(editor: &Editor, connector: &str) -> usize {
        editor
            .pending()
            .entries
            .iter()
            .position(|entry| entry.connector() == Some(connector))
            .unwrap_or_else(|| panic!("{connector} is pending"))
    }

    #[test]
    fn a_read_that_changed_nothing_keeps_the_edits_and_their_history() {
        let mut editor = editor();
        editor.edit(0, |state| state.x = 2000);
        let same = editor.snapshot().clone();
        assert_eq!(editor.rebase(same, true), Rebased::default());
        assert_eq!(editor.pending().entries[0].state.x, 2000);
        assert!(editor.undo(), "the history stays");
    }

    #[test]
    fn each_setting_follows_the_desktop_unless_the_user_changed_it() {
        let mut editor = editor();
        // DP-1's scale edited; its position and mode not.
        editor.edit(0, |state| state.scale = 2.0);
        // DP-2's position edited, and moved by another program too.
        editor.edit(1, |state| state.x = 3000);
        let mut read = editor.snapshot().clone();
        let mode = Some(Mode::new(1280, 720, 60_000));
        read.layout.entries[0].state.mode.clone_from(&mode);
        read.layout.entries[0].state.y = 0;
        read.layout.entries[1].state.x = 2500;
        read.layout.entries[1].state.transform = Transform::Normal;

        let rebased = editor.rebase(read, true);
        let dp1 = &editor.pending().entries[0].state;
        assert_eq!(dp1.scale, 2.0, "the user's edit stays");
        assert_eq!(dp1.mode, mode, "another program's mode is not undone");
        assert_eq!(dp1.y, 0, "nor its position");
        let dp2 = &editor.pending().entries[1].state;
        assert_eq!(dp2.x, 3000, "the user's position wins");
        assert_eq!(dp2.transform, Transform::Normal, "the desktop's turn stays");
        assert_eq!(rebased.kept_over, [("DP-2".to_owned(), Setting::Position)]);
        assert_eq!(rebased.dropped, Vec::<String>::new());
        assert_eq!(
            rebased.notice().as_deref(),
            Some("DP-2 position changed on the desktop too; your pending edit is kept")
        );
        assert!(!editor.undo(), "the history held the old live settings");
    }

    #[test]
    fn an_edit_the_desktop_made_too_is_no_conflict() {
        let mut editor = editor();
        editor.edit(0, |state| state.scale = 2.0);
        let mut read = editor.snapshot().clone();
        read.layout.entries[0].state.scale = 2.0;
        let rebased = editor.rebase(read, true);
        assert_eq!(rebased, Rebased::default());
        assert!(!editor.has_pending(), "the desktop has it now");
    }

    #[test]
    fn outputs_gone_take_their_edits_and_new_ones_come_as_they_are() {
        let mut editor = editor();
        editor.edit(0, |state| state.y = 7);
        let dp2 = index(&editor, "DP-2");
        editor.select(Some(dp2));
        let mut read = editor.snapshot().clone();
        let mut new = read.layout.entries[2].clone();
        new.id = OutputId::Connector("HDMI-A-2".into());
        new.state.enabled = true;
        new.state.x = 4000;
        read.layout.entries.remove(0);
        read.layout.entries.push(new);

        let rebased = editor.rebase(read, true);
        assert_eq!(rebased.dropped, ["DP-1"]);
        assert_eq!(
            rebased.notice().as_deref(),
            Some("DP-1 went away, and the pending edits to it with it")
        );
        assert_eq!(editor.pending().entries.len(), 3);
        assert_eq!(editor.pending().entries[2].state.x, 4000, "as it is");
        assert_eq!(
            editor.selected(),
            Some(index(&editor, "DP-2")),
            "follows DP-2"
        );
    }

    #[test]
    fn twins_keep_their_own_edits() {
        let twin = |connector: &str, x: i32| {
            let mut entry: OutputEntry = three_monitors().layout.entries[0].clone();
            entry.id = OutputId::new("AOC", "Q27G4", "SAME", connector);
            entry.output = Some(Output {
                connector: connector.into(),
                description: "AOC Q27G4 SAME".into(),
                physical_size_mm: None,
                make: "AOC".into(),
                model: "Q27G4".into(),
                serial: "SAME".into(),
                modes: vec![],
            });
            entry.state.x = x;
            entry
        };
        let mut snapshot = three_monitors();
        snapshot.layout.entries = vec![twin("DP-1", 0), twin("DP-2", 1920)];
        let mut editor = Editor::new(snapshot);
        editor.edit(0, |state| state.scale = 1.5);
        editor.edit(1, |state| state.scale = 2.0);
        // Read again in the other order, the first twin moved elsewhere.
        let mut read = editor.snapshot().clone();
        read.layout.entries.reverse();
        read.layout.entries[1].state.y = 100;

        let rebased = editor.rebase(read, true);
        assert_eq!(rebased, Rebased::default());
        let pending = &editor.pending().entries;
        assert_eq!(pending[0].connector(), Some("DP-2"));
        assert_eq!(pending[0].state.scale, 2.0, "DP-2's own edit");
        assert_eq!(pending[0].state.y, 360, "DP-2's own live setting");
        assert_eq!(pending[1].connector(), Some("DP-1"));
        assert_eq!(pending[1].state.scale, 1.5, "DP-1's own edit");
        assert_eq!(pending[1].state.y, 100, "DP-1 moved on the desktop");
    }

    #[test]
    fn a_kept_change_starts_the_pending_layout_again() {
        let mut editor = editor();
        editor.edit(0, |state| state.x = 2000);
        let same = editor.snapshot().clone();
        assert_eq!(editor.rebase(same, false), Rebased::default());
        assert!(!editor.has_pending());
        assert!(!editor.undo());
    }
}
