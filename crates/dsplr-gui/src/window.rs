//! The main window's loop: events in, edits to the editor, frames out.
//!
//! The canvas and the bar are dsplr-core's draw list; the property panel is
//! a crcbl-ui tree laid out in the panel's region and drawn over it. A press
//! goes to the tree always (a press outside a field commits it, outside an
//! open drop-down closes it), and to the canvas when it lands there with no
//! drop-down open. Keys go to the quit prompt while it is up, to the tree
//! while a field takes typing or a drop-down is open, and to the canvas
//! otherwise.

use std::time::{Duration, Instant};

use crcbl_app::text_input::TextPump;
use crcbl_shell::{
    ButtonState, CloseReply, CursorIcon, Keysym, Modifiers, PointerButton, Shell, ShellEvent,
    WindowId,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use crcbl_ui::tree::{AvailableSpace, NavInput, Ui};
use crcbl_ui::widget::PointerInput;
use dsplr_core::panel::panel;
use dsplr_core::{Editor, Point, Size, Theme, draw, regions};
use dsplr_model::ModeSpec;
use glam::Vec2;

use crate::gpu::Gpu;
use crate::keys::{self, Action};
use crate::panel::{self as panel_ui, Change, Drafts};
use crate::{GuiError, paint};

/// How long to wait for the window system after an out-of-date swapchain
/// before drawing again: a new size is usually on its way.
const PRESENT_RETRY: Duration = Duration::from_millis(16);
/// How often the panel is drawn again while the pointer is over it or a
/// field takes typing, so a tooltip shows and a caret blinks on time.
const UI_TICK: Duration = Duration::from_millis(100);
/// What the bar asks when quitting would drop pending edits.
const QUIT_PROMPT: &str = "Quit and drop the pending edits?  y / n";

pub struct MainWindow<'a> {
    shell: &'a mut dyn Shell,
    window: WindowId,
    editor: Editor,
    theme: Theme,
    font: &'static Font,
    /// crcbl's built-in glyph atlas, which its UI pass takes alongside the
    /// parsed font's glyphs.
    atlas: FontAtlas,
    list: DrawList,
    ui: Ui,
    pump: TextPump,
    drafts: Drafts,
    /// The window's size in device pixels, and device pixels per logical
    /// pixel.
    extent: (u32, u32),
    scale: f32,
    /// The pointer, in logical pixels, and whether the primary button is
    /// down.
    pointer: Point,
    pointer_down: bool,
    /// Navigation for the tree's next frame.
    nav: NavInput,
    last_frame: Instant,
    cursor: Option<CursorIcon>,
    /// The quit prompt is up.
    quitting: bool,
    /// Why the last edit could not be made, until the next one.
    notice: Option<String>,
}

/// What the loop does after an event.
enum Step {
    /// Nothing on screen changed.
    Idle,
    Redraw,
    /// The window's size or scale changed: the swapchain follows, and the
    /// frame is drawn again.
    Resized,
    Quit,
}

impl<'a> MainWindow<'a> {
    pub fn new(
        shell: &'a mut dyn Shell,
        window: WindowId,
        editor: Editor,
        extent: (u32, u32),
    ) -> Self {
        let theme = Theme::tokyo_night();
        let mut ui = Ui::new();
        ui.add_stylesheet("dsplr", &panel_ui::stylesheet(&theme));
        Self {
            shell,
            window,
            editor,
            theme,
            font: Font::sans(),
            atlas: FontAtlas::built_in(),
            list: DrawList::new(),
            ui,
            pump: TextPump::new(),
            drafts: Drafts::default(),
            extent,
            scale: 1.0,
            pointer: Point::new(0.0, 0.0),
            pointer_down: false,
            nav: NavInput::default(),
            last_frame: Instant::now(),
            cursor: Some(CursorIcon::Default),
            quitting: false,
            notice: None,
        }
    }

    /// Runs until the user quits, drawing with `gpu`, which presents to the
    /// window. `early` are events pumped before the loop started.
    pub fn run(&mut self, gpu: &mut Gpu, mut early: Vec<ShellEvent>) -> Result<(), GuiError> {
        let mut dirty = true;
        loop {
            let mut events = std::mem::take(&mut early);
            self.shell.pump(&mut |event| events.push(event));
            for event in events {
                gpu.observe(&event);
                match self.handle(event)? {
                    Step::Idle => {}
                    Step::Redraw => dirty = true,
                    Step::Resized => {
                        gpu.resize(self.extent)?;
                        dirty = true;
                    }
                    Step::Quit => return Ok(()),
                }
            }
            if dirty && gpu.frame_due() {
                let pointer = PointerInput {
                    down: self.pointer_down,
                    ..PointerInput::hovering(vec(self.pointer))
                };
                // An edit the panel made this frame shows in the next.
                let changed = self.frame(pointer)?;
                let background = paint::linear(self.theme.background);
                if gpu.present(self.shell, &self.list, &self.atlas, background)? {
                    dirty = changed;
                } else {
                    // Out of date and reconfigured. Retrying at once would
                    // spin while it stays out of date.
                    self.shell.wait_events(Some(PRESENT_RETRY));
                }
                continue;
            }
            let over_panel = regions(&self.editor, self.size())
                .panel
                .contains(self.pointer);
            let tick = (over_panel || self.ui.text_editing()).then_some(UI_TICK);
            self.shell.wait_events(tick);
            dirty |= tick.is_some();
        }
    }

    fn handle(&mut self, event: ShellEvent) -> Result<Step, GuiError> {
        let editing = self.ui.text_editing() || self.ui.popup_list_open();
        if self.pump.observe(&event, editing) {
            return Ok(Step::Redraw);
        }
        Ok(match event {
            ShellEvent::Resized {
                size, scale_factor, ..
            }
            | ShellEvent::ScaleFactorChanged {
                size, scale_factor, ..
            } => {
                self.extent = (size.width, size.height);
                self.scale = scale_factor as f32;
                // A drag's fit was for the old size.
                self.editor.cancel_drag();
                Step::Resized
            }
            ShellEvent::CloseRequested { window } => {
                self.shell.reply_close_request(window, CloseReply::Keep)?;
                self.quit()
            }
            ShellEvent::Key {
                keysym,
                state: ButtonState::Pressed,
                modifiers,
                ..
            } => self.key(keysym, modifiers, editing)?,
            ShellEvent::PointerMotion { abs: Some(at), .. }
            | ShellEvent::PointerFocus {
                entered: true,
                position: Some(at),
                ..
            } => {
                self.pointer = self.logical(at.x, at.y);
                self.editor.drag_to(self.pointer);
                // A drag moves an output; otherwise hover may change in the
                // panel.
                Step::Redraw
            }
            ShellEvent::Button {
                button: PointerButton::Left,
                state,
                position,
                ..
            } => {
                if let Some(at) = position {
                    self.pointer = self.logical(at.x, at.y);
                }
                self.button(state == ButtonState::Pressed)?
            }
            _ => Step::Idle,
        })
    }

    /// A press or release of the primary button. Each edge is a frame of
    /// its own for the tree, so a press and its release in one pump still
    /// make a click.
    fn button(&mut self, down: bool) -> Result<Step, GuiError> {
        let at = vec(self.pointer);
        let popup = self.ui.popup_list_open();
        let pointer = PointerInput {
            pos: at,
            down,
            released: self.pointer_down && !down,
            secondary_pressed: false,
        };
        self.frame(pointer)?;
        self.pointer_down = down;
        if down {
            let on_canvas = regions(&self.editor, self.size())
                .canvas
                .contains(self.pointer);
            if on_canvas && !popup && !self.quitting {
                self.notice = None;
                // The keyboard has left the panel: a field being typed into
                // is committed, to the output it was for (`frame`).
                self.ui.clear_focus();
                self.editor.press(self.size(), self.pointer);
            }
        } else {
            self.editor.release();
        }
        Ok(Step::Redraw)
    }

    /// A key press, as its own action: a run of presses arriving together
    /// acts once for each.
    fn key(
        &mut self,
        keysym: Keysym,
        modifiers: Modifiers,
        editing: bool,
    ) -> Result<Step, GuiError> {
        if self.quitting {
            return Ok(match keys::answer(keysym) {
                Some(true) => Step::Quit,
                Some(false) => {
                    self.quitting = false;
                    Step::Redraw
                }
                None => Step::Idle,
            });
        }
        if editing {
            let Some(nav) = keys::nav(keysym, modifiers) else {
                return Ok(Step::Idle);
            };
            // The tree takes one navigation per frame, so each press gets a
            // frame of its own: two Downs in one batch move two options.
            self.nav = nav;
            self.frame(PointerInput {
                down: self.pointer_down,
                ..PointerInput::hovering(vec(self.pointer))
            })?;
            return Ok(Step::Redraw);
        }
        if self.editor.dragging().is_some() {
            return Ok(if keysym == Keysym::ESCAPE && self.editor.cancel_drag() {
                Step::Redraw
            } else {
                Step::Idle
            });
        }
        Ok(match keys::action(keysym, modifiers) {
            Some(action) => self.act(action),
            None => Step::Idle,
        })
    }

    fn act(&mut self, action: Action) -> Step {
        self.notice = None;
        let selected = self.editor.selected();
        let on_selected = |editor: &mut Editor, edit: &dyn Fn(&mut Editor, usize) -> bool| {
            selected.is_some_and(|index| edit(editor, index))
        };
        let editor = &mut self.editor;
        let changed = match action {
            Action::SelectNext => {
                editor.select_next();
                true
            }
            Action::SelectPrevious => {
                editor.select_previous();
                true
            }
            Action::Nudge(direction) => on_selected(editor, &|e, i| e.nudge(i, direction)),
            Action::Touch(direction) => {
                on_selected(editor, &|e, i| e.move_to_neighbour(i, direction))
            }
            Action::Rotate { clockwise } => on_selected(editor, &|e, i| e.rotate(i, clockwise)),
            Action::Scale { up } => on_selected(editor, &|e, i| e.step_scale(i, up)),
            Action::ToggleEnabled => on_selected(editor, &|e, i| e.toggle_enabled(i)),
            Action::Undo => editor.undo(),
            Action::Redo => editor.redo(),
            Action::Quit => return self.quit(),
        };
        if changed { Step::Redraw } else { Step::Idle }
    }

    /// Quits, or asks first when that would drop pending edits.
    fn quit(&mut self) -> Step {
        if self.editor.has_pending() {
            self.quitting = true;
            Step::Redraw
        } else {
            Step::Quit
        }
    }

    /// Makes `change`, from the panel, to output `index`. Returns whether
    /// anything changed on screen.
    fn apply(&mut self, index: usize, change: Change) -> bool {
        self.notice = None;
        let editor = &mut self.editor;
        match change {
            Change::Enabled(on) => editor.set_enabled(index, on),
            Change::Size(width, height) => editor.set_size(index, width, height),
            Change::Mode(mode) => editor.set_mode(index, mode),
            Change::CustomMode(text) => {
                let made = text
                    .parse::<ModeSpec>()
                    .map_err(|error| error.to_string())
                    .and_then(|spec| editor.set_custom_mode(index, spec));
                match made {
                    Ok(changed) => changed,
                    Err(why) => {
                        self.notice = Some(why);
                        true
                    }
                }
            }
            Change::Scale(scale) => editor.set_scale(index, scale),
            Change::Transform(transform) => editor.set_transform(index, transform),
            Change::AdaptiveSync(sync) => editor.set_adaptive_sync(index, sync),
            Change::Mirror(source) => editor.set_mirror(index, source),
        }
    }

    /// Builds the panel for `pointer`, makes the changes it reports, and
    /// records the frame's draw list. Returns whether a change was made,
    /// which the next frame shows.
    fn frame(&mut self, pointer: PointerInput) -> Result<bool, GuiError> {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        let size = self.size();
        let region = regions(&self.editor, size).panel;

        let mut changed = self.follow_selection();
        let nav = std::mem::take(&mut self.nav);
        let mut changes = self.build(pointer, nav, dt, region);
        // Layout showed nodes the build left out (a drop-down that just
        // opened): build again, with this frame's press and keys taken.
        if self.ui.needs_rebuild() {
            let at_rest = PointerInput {
                down: self.pointer_down,
                ..PointerInput::hovering(pointer.pos)
            };
            changes.extend(self.build(at_rest, NavInput::default(), Duration::ZERO, region));
        }
        // A change behind the quit prompt is not made: it would add to the
        // edits the prompt is about to drop, unseen.
        if let (Some(index), false) = (self.drafts.output(), self.quitting) {
            for change in changes {
                changed |= self.apply(index, change);
            }
        }

        let message = if self.quitting {
            Some(QUIT_PROMPT)
        } else {
            self.notice.as_deref()
        };
        let scene = draw(&self.editor, size, &self.theme, message);
        self.list.clear();
        self.list.set_scale(self.scale);
        paint::paint(&scene, &mut self.list, self.font);
        self.ui.emit(&mut self.list);

        self.pump.serve(
            self.ui.take_clipboard_requests(),
            &mut *self.shell,
            self.window,
        );
        self.pump.engage(
            self.ui.text_editing_purpose(),
            &mut *self.shell,
            self.window,
        );
        self.pump.place(
            self.ui.text_input_area(),
            self.scale,
            &mut *self.shell,
            self.window,
        );
        let cursor = self.ui.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            self.shell.set_cursor(self.window, cursor)?;
        }
        Ok(changed)
    }

    /// When the selection has moved off the output the panel's drafts are
    /// for: commits what was being typed or dragged to that output, takes
    /// the keyboard out of the panel, and starts the drafts afresh for the
    /// selected one. Returns whether a commit changed anything.
    fn follow_selection(&mut self) -> bool {
        let selected = self.editor.selected();
        if self.drafts.output() == selected {
            return false;
        }
        let mut changed = false;
        if let Some(old) = self.drafts.output()
            && !self.quitting
            && let Some(old_panel) = panel(&self.editor, old)
        {
            for change in self.drafts.commits(&old_panel) {
                changed |= self.apply(old, change);
            }
        }
        self.ui.clear_focus();
        let new_panel = selected.and_then(|index| panel(&self.editor, index));
        self.drafts.reset(selected, new_panel.as_ref());
        changed
    }

    /// One build and layout of the panel's tree, in `region`.
    fn build(
        &mut self,
        pointer: PointerInput,
        nav: NavInput,
        dt: Duration,
        region: dsplr_core::Bounds,
    ) -> Vec<Change> {
        self.ui.begin_frame_with(pointer, nav);
        self.ui.set_text_input(self.pump.frame(dt));
        let panel = self
            .editor
            .selected()
            .and_then(|index| panel(&self.editor, index));
        let changes = panel_ui::build(&mut self.ui, panel.as_ref(), &mut self.drafts);
        self.ui.layout(
            Vec2::new(region.x, region.y),
            AvailableSpace::definite(Vec2::new(region.width, region.height)),
            &self.atlas,
        );
        changes
    }

    /// The window's size in logical pixels.
    fn size(&self) -> Size {
        Size {
            width: self.extent.0 as f32 / self.scale,
            height: self.extent.1 as f32 / self.scale,
        }
    }

    /// A point in device pixels, in logical ones.
    fn logical(&self, x: f64, y: f64) -> Point {
        Point::new(x as f32 / self.scale, y as f32 / self.scale)
    }
}

fn vec(point: Point) -> Vec2 {
    Vec2::new(point.x, point.y)
}

#[cfg(test)]
mod tests {
    use crcbl_shell::{HeadlessShell, KeyCode, Shell, ShellEvent, WindowDesc};
    use crcbl_ui::widget::PointerInput;
    use dsplr_core::{Editor, NUDGE_STEP, Snapshot};
    use dsplr_model::{Capabilities, Layout, Mode, OutputEntry, OutputId, OutputState};
    use glam::Vec2;

    use super::MainWindow;

    fn editor() -> Editor {
        let entry = |connector: &str, x| OutputEntry {
            id: OutputId::Connector(connector.into()),
            output: None,
            state: OutputState {
                mode: Some(Mode::new(1920, 1080, 60_000)),
                x,
                ..OutputState::default()
            },
        };
        Editor::new(Snapshot {
            backend: "sway".into(),
            capabilities: Capabilities::default(),
            layout: Layout {
                entries: vec![entry("DP-1", 0), entry("DP-2", 1920)],
            },
        })
    }

    /// The key events of pressing and releasing each of `keys` in turn, all
    /// in one batch, as a pump delivers keys typed faster than a frame.
    fn typed(keys: &[KeyCode]) -> Vec<ShellEvent> {
        let mut source = HeadlessShell::new();
        let window = source.create_window(&WindowDesc::default()).unwrap();
        source.pump(&mut |_| {});
        for &key in keys {
            source.key_press(window, key).unwrap();
            source.key_release(window, key).unwrap();
        }
        let mut events = Vec::new();
        source.pump(&mut |event| events.push(event));
        events.retain(|event| matches!(event, ShellEvent::Key { .. }));
        assert_eq!(events.len(), 2 * keys.len());
        events
    }

    #[test]
    fn each_press_in_a_batch_is_its_own_action() {
        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let mut main = MainWindow::new(&mut shell, window, editor(), (1280, 800));
        assert_eq!(main.editor.selected(), Some(0));
        for event in typed(&[KeyCode::KeyL, KeyCode::KeyL, KeyCode::KeyL]) {
            main.handle(event).unwrap();
        }
        assert_eq!(main.editor.pending().entries[0].state.x, 3 * NUDGE_STEP);
    }

    #[test]
    fn the_drafts_follow_the_selection() {
        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let mut main = MainWindow::new(&mut shell, window, editor(), (1280, 800));
        let at_rest = PointerInput::hovering(Vec2::new(-1.0, -1.0));
        main.frame(at_rest).unwrap();
        assert_eq!(main.drafts.output(), Some(0));
        for event in typed(&[KeyCode::Tab]) {
            main.handle(event).unwrap();
        }
        assert_eq!(main.editor.selected(), Some(1));
        main.frame(at_rest).unwrap();
        assert_eq!(main.drafts.output(), Some(1));
        assert!(!main.editor.has_pending(), "following it changed nothing");
    }
}
