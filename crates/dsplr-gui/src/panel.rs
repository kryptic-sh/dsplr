//! The property panel, built with crcbl-ui's widgets from
//! [`dsplr_core::panel::Panel`]: a drop-down per choice, a checkbox for
//! whether the output is on, a number for the scale and a text field for a
//! custom mode. A field that cannot be changed is disabled, with why as its
//! tooltip. What the user changes comes back as [`Change`]s for the editor.

use crcbl_ui::tree::{Engagement, Response, Ui};
use dsplr_core::Theme;
use dsplr_core::link::Question;
use dsplr_core::panel::{Choice, Field, Panel};
use dsplr_model::{AdaptiveSync, Mode, OutputId, Transform};

/// The widest a typed scale may be, and the smallest; a drag moves it by
/// `SCALE_SPEED` per pixel, an engaged step by `SCALE_STEP`.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.25..=8.0;
const SCALE_SPEED: f64 = 0.005;
const SCALE_STEP: f64 = 0.05;

/// A change the user made in the panel, to the selected output.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Enabled(bool),
    Size(u32, u32),
    Mode(Mode),
    /// The custom mode field's text, committed.
    CustomMode(String),
    Scale(f64),
    Transform(Transform),
    AdaptiveSync(AdaptiveSync),
    Mirror(Option<OutputId>),
}

/// What the panel keeps between frames: the text of the custom mode field
/// and the scale while they are being edited, so a value half typed or half
/// dragged is not one edit per frame, and the output they are for, so a
/// draft never lands on an output it was not typed for.
#[derive(Debug, Default)]
pub struct Drafts {
    output: Option<usize>,
    custom_mode: String,
    /// The custom mode field was engaged last frame.
    custom_active: bool,
    scale: f64,
    /// The scale widget was engaged or pressed last frame.
    scale_active: bool,
}

impl Drafts {
    /// The output the drafts are for.
    pub const fn output(&self) -> Option<usize> {
        self.output
    }

    /// What committing the drafts makes of `panel`, the panel of the output
    /// they are for: each field being edited whose draft differs from its
    /// value, where the field can still be changed.
    pub fn commits(&self, panel: &Panel) -> Vec<Change> {
        let mut commits = Vec::new();
        if self.custom_active
            && panel.custom_mode.locked.is_none()
            && self.custom_mode != panel.custom_mode.value
        {
            commits.push(Change::CustomMode(self.custom_mode.clone()));
        }
        if self.scale_active && panel.scale.locked.is_none() && self.scale != panel.scale.value.1 {
            commits.push(Change::Scale(self.scale));
        }
        commits
    }

    /// Takes the drafts over to `output`, whose panel is `panel`: what was
    /// being edited is dropped, and each draft holds the field's value.
    pub fn reset(&mut self, output: Option<usize>, panel: Option<&Panel>) {
        *self = Self {
            output,
            custom_mode: panel
                .map(|p| p.custom_mode.value.clone())
                .unwrap_or_default(),
            custom_active: false,
            scale: panel.map_or(1.0, |p| p.scale.value.1),
            scale_active: false,
        };
    }
}

/// The stylesheet: the panel and crcbl-ui's widgets in `theme`'s colours.
pub fn stylesheet(theme: &Theme) -> String {
    let Theme {
        surface,
        divider,
        output,
        border,
        accent,
        pending,
        text,
        text_dim,
        muted,
        background,
        ..
    } = theme;
    let family = crate::fonts::FAMILY;
    format!(
        "
.panel {{ width: 100%; height: 100%; flex-direction: column; gap: 4px; padding: 12px 16px;
  background: {surface}; border-color: {divider}; border-width: 0 0 0 1px;
  color: {text}; font-family: {family}; font-size: 13px; }}
.row {{ flex-direction: row; align-items: center; height: 22px; }}
.label {{ width: 112px; flex-shrink: 0; color: {muted}; }}
.label.changed {{ color: {pending}; }}
.value {{ color: {text_dim}; }}
.title {{ font-size: 15px; color: {text}; }}
.empty {{ color: {muted}; }}
select, drag-value, text-input {{ width: 160px; background: {output}; border-color: {border};
  color: {text}; }}
select:hover, drag-value:hover {{ background: {background}; }}
select:open, drag-value:engaged, text-input:engaged {{ border-color: {accent}; }}
select:disabled, drag-value:disabled, text-input:disabled, checkbox:disabled {{ color: {muted};
  background: {surface}; }}
.select-caret {{ color: {muted}; }}
checkbox {{ color: {text}; }}
.checkbox-box {{ background: {background}; border-color: {border}; }}
checkbox:checked > .checkbox-box > .checkbox-mark {{ background: {accent}; }}
popup {{ background: {surface}; border-color: {border}; color: {text}; font-family: {family};
  font-size: 13px; }}
.select-list > .select-option:hover {{ background: {output}; }}
.select-list > .select-option:checked {{ background: {accent}; color: {surface}; }}
tooltip {{ background: {output}; border-color: {border}; color: {text}; font-family: {family};
  font-size: 13px; }}
.fields {{ flex-direction: column; gap: 4px; flex-grow: 1; }}
.actions {{ flex-direction: column; gap: 6px; padding: 10px 0 0 0; border-color: {divider};
  border-width: 1px 0 0 0; }}
.buttons {{ flex-direction: row; gap: 6px; align-items: center; }}
button {{ padding: 3px 8px; background: {output}; border-color: {border}; color: {text}; }}
button:hover {{ background: {background}; border-color: {accent}; }}
button:disabled {{ background: {surface}; color: {muted}; }}
.buttons text-input {{ width: 140px; }}
.buttons select {{ width: 140px; }}
.note {{ color: {pending}; }}
.diff {{ flex-direction: column; flex-grow: 1; overflow: scroll; padding: 6px;
  background: {background}; color: {text_dim}; font-size: 12px; }}
.diff-line {{ white-space: pre; flex-shrink: 0; }}
"
    )
}

/// What the window can do besides edit, as the panel shows it.
pub struct Actions<'a> {
    /// The backend thread is working on a request: nothing else is sent.
    pub busy: bool,
    /// The profiles in the store, by name.
    pub profiles: &'a [String],
    /// A question waiting on the user, which takes the panel's place.
    pub question: Option<&'a Question>,
}

/// What the user asked of the window in the panel, besides edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Identify,
    Apply,
    Save,
    Revert,
    Refresh,
    /// Save the displays as the profile of this name.
    SaveProfile(String),
    /// Apply the profile of this name.
    Restore(String),
    /// Yes to the question.
    Approve,
    /// No to the question.
    Dismiss,
}

/// What the actions keep between frames: the profile name being typed, and
/// the profile chosen in the picker.
#[derive(Debug, Default)]
pub struct ActionDrafts {
    profile_name: String,
    profile: usize,
}

/// A frame of the panel: the edits and the commands the user made in it.
#[derive(Debug, Default, PartialEq)]
pub struct Built {
    pub changes: Vec<Change>,
    pub commands: Vec<Command>,
}

/// Builds the panel for `panel` (none selected: a line saying so), and
/// returns what the user changed this frame, to the output `drafts` are for;
/// the caller keeps that the output `panel` is for
/// ([`Drafts::reset`] when the selection changes).
pub fn build(
    ui: &mut Ui,
    panel: Option<&Panel>,
    drafts: &mut Drafts,
    actions: &Actions<'_>,
    action_drafts: &mut ActionDrafts,
) -> Built {
    let mut built = Built::default();
    ui.block(".panel", &[], |ui| {
        if let Some(question) = actions.question {
            ask(ui, question, &mut built.commands);
            return;
        }
        ui.block(".fields", &[], |ui| {
            fields(ui, panel, drafts, &mut built.changes)
        });
        ui.block(".actions", &[], |ui| {
            buttons(ui, actions, action_drafts, &mut built.commands);
        });
    });
    built
}

/// The question: what would be replaced, why it is asked, what replacing
/// it changes, and Replace or Cancel.
fn ask(ui: &mut Ui, question: &Question, commands: &mut Vec<Command>) {
    ui.span(
        ".title",
        format!("Replace {}?", question.what).as_str(),
        &[],
    );
    for (index, note) in question.notes.iter().enumerate() {
        ui.block_keyed(index, ".note", &[], |ui| {
            ui.span("", note.as_str(), &[]);
        });
    }
    ui.block(".diff", &[], |ui| {
        for (index, line) in question.diff.lines().enumerate() {
            ui.block_keyed(index, ".diff-line", &[], |ui| {
                ui.span("", line, &[]);
            });
        }
    });
    ui.block(".buttons", &[], |ui| {
        if ui.button("#approve", "Replace (Enter)").clicked {
            commands.push(Command::Approve);
        }
        if ui.button("#dismiss", "Cancel (Esc)").clicked {
            commands.push(Command::Dismiss);
        }
    });
}

/// Apply, Save, Revert and Refresh; saving as a profile; and, when there
/// are profiles, picking one to restore. Disabled while the backend thread
/// is busy.
fn buttons(
    ui: &mut Ui,
    actions: &Actions<'_>,
    drafts: &mut ActionDrafts,
    commands: &mut Vec<Command>,
) {
    ui.enabled(!actions.busy, |ui| {
        ui.block(".buttons", &[], |ui| {
            for (id, label, command) in [
                ("#apply", "Apply (a)", Command::Apply),
                ("#save", "Save (s)", Command::Save),
                ("#revert", "Revert", Command::Revert),
                ("#refresh", "Refresh", Command::Refresh),
                ("#identify", "Identify (i)", Command::Identify),
            ] {
                if ui.button(id, label).clicked {
                    commands.push(command);
                }
            }
        });
        ui.block("#profile-save.buttons", &[], |ui| {
            ui.text_input("#profile-name", &mut drafts.profile_name);
            if ui.button("#save-profile", "Save as profile").clicked {
                commands.push(Command::SaveProfile(drafts.profile_name.trim().to_owned()));
            }
        });
        if !actions.profiles.is_empty() {
            ui.block("#profile-restore.buttons", &[], |ui| {
                let labels: Vec<&str> = actions.profiles.iter().map(String::as_str).collect();
                drafts.profile = drafts.profile.min(labels.len() - 1);
                ui.select("#profile-pick", &labels, &mut drafts.profile);
                if ui.button("#restore", "Restore").clicked {
                    commands.push(Command::Restore(actions.profiles[drafts.profile].clone()));
                }
            });
        }
    });
}

/// The selected output's fields, or a line saying none is selected.
fn fields(ui: &mut Ui, panel: Option<&Panel>, drafts: &mut Drafts, changes: &mut Vec<Change>) {
    let Some(panel) = panel else {
        ui.span(".empty", "No output selected", &[]);
        return;
    };
    ui.block(".row", &[], |ui| {
        ui.span(".title", panel.output.as_str(), &[]);
    });
    ui.block(".row", &[], |ui| {
        ui.span(".value", panel.description.as_str(), &[]);
    });
    row(ui, "#enabled", "Enabled", &panel.enabled, |ui| {
        let mut on = panel.enabled.value;
        let response = ui.checkbox("#enabled-box", "", &mut on);
        if response.changed {
            changes.push(Change::Enabled(on));
        }
        response
    });
    choice(
        ui,
        "#size",
        "Mode",
        &panel.size,
        |&(w, h)| Some(Change::Size(w, h)),
        changes,
    );
    choice(
        ui,
        "#refresh",
        "Refresh",
        &panel.refresh,
        |mode| Some(Change::Mode(mode.clone())),
        changes,
    );
    if !drafts.custom_active {
        drafts.custom_mode.clone_from(&panel.custom_mode.value);
    }
    row(ui, "#custom", "Custom mode", &panel.custom_mode, |ui| {
        let response = ui.text_input("#custom-input", &mut drafts.custom_mode);
        if response.engagement == Engagement::Committed
            && drafts.custom_mode != panel.custom_mode.value
        {
            changes.push(Change::CustomMode(drafts.custom_mode.clone()));
        }
        drafts.custom_active = response.engagement.is_engaged();
        response
    });
    row(ui, "#position", "Position", &panel.position, |ui| {
        ui.span("#position-value.value", panel.position.value.as_str(), &[])
    });
    scale(ui, panel, drafts, changes);
    choice(
        ui,
        "#transform",
        "Transform",
        &panel.transform,
        |&t| Some(Change::Transform(t)),
        changes,
    );
    // `unknown` stands for the value as the backend reported it, which
    // nothing can be set back to: choosing it changes nothing.
    choice(
        ui,
        "#vrr",
        "Adaptive sync",
        &panel.adaptive_sync,
        |sync| sync.map(Change::AdaptiveSync),
        changes,
    );
    choice(
        ui,
        "#mirror",
        "Mirror of",
        &panel.mirror,
        |id| Some(Change::Mirror(id.clone())),
        changes,
    );
    for (label, value, changed) in &panel.other {
        ui.block_keyed(label, ".row", &[], |ui| {
            let class = if *changed { ".label.changed" } else { ".label" };
            ui.span(class, *label, &[]);
            ui.span(".value", value.as_str(), &[]);
        });
    }
}

/// A labelled row holding `widget`, disabled with a tooltip when `field` is
/// locked, its label marked when a pending edit changed it.
fn row<T>(
    ui: &mut Ui,
    id: &str,
    label: &str,
    field: &Field<T>,
    widget: impl FnOnce(&mut Ui) -> Response,
) {
    ui.block(&format!("{id}.row"), &[], |ui| {
        let class = if field.changed {
            ".label.changed"
        } else {
            ".label"
        };
        ui.span(&format!("{id}-label{class}"), label, &[]);
        let mut response = None;
        ui.enabled(field.locked.is_none(), |ui| response = Some(widget(ui)));
        if let (Some(response), Some(why)) = (response, &field.locked) {
            ui.tooltip(&response, why);
        }
    });
}

/// A row with a drop-down over `field`'s options; picking one pushes the
/// change `to_change` makes of its value, if any.
fn choice<T>(
    ui: &mut Ui,
    id: &str,
    label: &str,
    field: &Field<Choice<T>>,
    to_change: impl Fn(&T) -> Option<Change>,
    changes: &mut Vec<Change>,
) {
    row(ui, id, label, field, |ui| {
        let labels = field.value.labels();
        // Past the end shows no option as chosen.
        let mut chosen = field.value.chosen.unwrap_or(labels.len());
        let response = ui.select(&format!("{id}-select"), &labels, &mut chosen);
        if response.changed
            && let Some(change) = field
                .value
                .options
                .get(chosen)
                .and_then(|(_, v)| to_change(v))
        {
            changes.push(change);
        }
        response
    });
}

/// The scale row: a drop-down of the offered scales and a number to drag or
/// type any other. The number's edit is made once the drag or the typing
/// ends, not on every frame of it.
fn scale(ui: &mut Ui, panel: &Panel, drafts: &mut Drafts, changes: &mut Vec<Change>) {
    let field = &panel.scale;
    let (choices, current) = &field.value;
    if !drafts.scale_active {
        drafts.scale = *current;
    }
    row(ui, "#scale", "Scale", field, |ui| {
        let labels = choices.labels();
        let mut chosen = choices.chosen.unwrap_or(labels.len());
        let picked = ui.select("#scale-select", &labels, &mut chosen);
        if picked.changed
            && let Some(&(_, scale)) = choices.options.get(chosen)
        {
            changes.push(Change::Scale(scale));
        }
        picked
    });
    let locked = Field {
        value: (),
        changed: field.changed,
        locked: field.locked.clone(),
    };
    row(ui, "#scale-number", "", &locked, |ui| {
        let response = ui.drag_value(
            "#scale-value",
            &mut drafts.scale,
            SCALE_RANGE,
            SCALE_SPEED,
            SCALE_STEP,
        );
        let active = response.engagement.is_engaged() || response.pressed;
        let ended = drafts.scale_active && !active;
        if (ended || (response.changed && !active)) && drafts.scale != *current {
            changes.push(Change::Scale(drafts.scale));
        }
        drafts.scale_active = active;
        response
    });
}

#[cfg(test)]
mod tests {
    use crcbl_ui::text::FontAtlas;
    use crcbl_ui::tree::{AvailableSpace, Ui};
    use crcbl_ui::widget::PointerInput;
    use dsplr_core::link::{Question, Request};
    use dsplr_core::panel::panel;
    use dsplr_core::{Editor, Snapshot, Theme};
    use dsplr_model::{
        AdaptiveSync, Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState,
    };
    use glam::Vec2;

    use super::{ActionDrafts, Actions, Built, Drafts, build, stylesheet};

    fn editor() -> Editor {
        let output = |connector: &str| Output {
            connector: connector.into(),
            description: format!("{connector} monitor"),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            physical_size_mm: None,
            modes: vec![Mode::new(1920, 1080, 60_000), Mode::new(1280, 720, 60_000)],
        };
        let entry = |connector: &str, x| OutputEntry {
            id: OutputId::Connector(connector.into()),
            output: Some(output(connector)),
            state: OutputState {
                mode: Some(Mode::new(1920, 1080, 60_000)),
                x,
                adaptive_sync: Some(AdaptiveSync::Off),
                ..OutputState::default()
            },
        };
        Editor::new(Snapshot {
            backend: "hyprland".into(),
            capabilities: Capabilities {
                mirroring: true,
                colour_preset: true,
                bit_depth: true,
                ..Capabilities::default()
            },
            layout: Layout {
                entries: vec![entry("DP-1", 0), entry("DP-2", 1920)],
            },
        })
    }

    /// Two frames at rest, with the profile picker and with a question:
    /// every node built once, nothing reported changed or asked for.
    #[test]
    fn the_panel_builds_with_one_node_per_key_and_does_nothing_at_rest() {
        let editor = editor();
        let mut ui = Ui::new();
        ui.add_stylesheet("dsplr", &stylesheet(&Theme::tokyo_night()));
        let atlas = FontAtlas::built_in();
        let mut drafts = Drafts::default();
        let mut action_drafts = ActionDrafts::default();
        let panel = panel(&editor, 0);
        assert!(panel.is_some());
        let profiles = ["home".to_owned(), "work".to_owned()];
        let question = Question {
            what: "/h/.config/sway/outputs".into(),
            notes: vec!["edited by hand".into(), "and more".into()],
            diff: "--- mine\n+++ new\n-a\n+b\n".into(),
            approve: Request::Save { approved: None },
        };
        for question in [None, Some(&question)] {
            let actions = Actions {
                busy: false,
                profiles: &profiles,
                question,
            };
            for _ in 0..2 {
                ui.begin_frame(PointerInput::hovering(Vec2::new(-10.0, -10.0)));
                let built = build(
                    &mut ui,
                    panel.as_ref(),
                    &mut drafts,
                    &actions,
                    &mut action_drafts,
                );
                ui.layout(
                    Vec2::new(960.0, 0.0),
                    AvailableSpace::definite(Vec2::new(320.0, 768.0)),
                    &atlas,
                );
                assert_eq!(built, Built::default(), "nothing touched");
                assert_eq!(ui.duplicate_keys(), [], "two nodes shared a key");
            }
            assert!(ui.len() > 12, "the panel was built: {} nodes", ui.len());
        }
    }

    #[test]
    fn drafts_commit_only_a_field_being_edited_that_can_still_change() {
        let editor = editor();
        let first = panel(&editor, 0).unwrap();
        let mut drafts = Drafts::default();
        drafts.reset(Some(0), Some(&first));
        assert_eq!(drafts.output(), Some(0));
        assert_eq!(drafts.commits(&first), []);

        drafts.scale = 1.5;
        drafts.custom_mode = "1280x720@60".into();
        assert_eq!(drafts.commits(&first), [], "neither is being edited");
        drafts.scale_active = true;
        drafts.custom_active = true;
        assert_eq!(
            drafts.commits(&first),
            [super::Change::Scale(1.5)],
            "hyprland takes no custom modes, so that field is locked"
        );

        let second = panel(&editor, 1).unwrap();
        drafts.reset(Some(1), Some(&second));
        assert_eq!(drafts.output(), Some(1));
        assert_eq!(
            drafts.commits(&second),
            [],
            "what was being edited is dropped"
        );
        assert_eq!(
            drafts.custom_mode, "1920x1080@60",
            "the new output's own value"
        );
    }
}
