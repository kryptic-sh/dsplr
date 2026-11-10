//! The main window's loop: events in, edits to the editor, frames out.
//!
//! The canvas and the bar are dsplr-core's draw list; the property panel is
//! a crcbl-ui tree laid out in the panel's region and drawn over it. A press
//! goes to the tree always (a press outside a field commits it, outside an
//! open drop-down closes it), and to the canvas when it lands there with no
//! drop-down open. Keys go to the confirm dialogs while they are up, to the
//! quit prompt while it is up, to the tree while a field takes typing or a
//! drop-down is open, and to the canvas otherwise.
//!
//! Apply, Save, Revert and the profiles go to the backend thread as
//! [`Request`]s, one at a time; its [`Reply`]s come back between events.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crcbl_app::text_input::TextPump;
use crcbl_shell::{
    ButtonState, CloseReply, CursorIcon, Keysym, Modifiers, PointerButton, Shell, ShellCaps,
    ShellEvent, WindowId,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use crcbl_ui::tree::{AvailableSpace, NavInput, Ui};
use crcbl_ui::widget::PointerInput;
use dsplr_core::dialog::{Choice, QUESTION};
use dsplr_core::flow::Outcome;
use dsplr_core::link::{Answer, Question, Reply, Request};
use dsplr_core::panel::panel;
use dsplr_core::{Editor, Point, Size, Theme, draw, regions};
use dsplr_model::{Layout, ModeSpec};
use glam::Vec2;

use crate::confirm::Confirming;
use crate::gpu::Gpu;
use crate::identify::Identifying;
use crate::keys::{self, Action, Keymap};
use crate::panel::{self as panel_ui, ActionDrafts, Actions, Built, Change, Command, Drafts};
use crate::{GuiError, fonts, paint};

/// How long to wait for the window system after an out-of-date swapchain
/// before drawing again: a new size is usually on its way.
const PRESENT_RETRY: Duration = Duration::from_millis(16);
/// How often the panel is drawn again while the pointer is over it or a
/// field takes typing, so a tooltip shows and a caret blinks on time.
const UI_TICK: Duration = Duration::from_millis(100);
/// What the bar asks when quitting would drop pending edits.
const QUIT_PROMPT: &str = "Quit and drop the pending edits?  y / n";
/// How often the loop looks for the backend thread's replies when the
/// window system cannot wake it.
const REPLY_POLL: Duration = Duration::from_millis(50);

/// A [`Reply::Confirm`]'s id, its deadline and its watchdog's, and the
/// layout applied: what the dialogs open with.
type Asked = (u64, (Instant, Instant), Layout);

/// The window's ends of its link to the backend thread
/// ([`crate::Link`], relayed).
pub struct Channels {
    pub requests: Sender<Request>,
    pub answers: Sender<Answer>,
    pub replies: Receiver<Reply>,
    /// The window system wakes the loop when a reply arrives; without that
    /// it polls.
    pub wakes: bool,
}

pub struct MainWindow<'a> {
    shell: &'a mut dyn Shell,
    window: WindowId,
    editor: Editor,
    theme: Theme,
    /// The face it writes in: the system's sans-serif, else dsplr's own.
    font: Font,
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
    /// The last thing worth telling: why an edit could not be made, or a
    /// line from the backend thread. Until the next.
    notice: Option<String>,
    /// To the backend thread, and from it.
    channels: Channels,
    /// The backend thread has stopped (a panic): nothing that needs it can
    /// be done, and a change waiting on its answer is the watchdog's.
    backend_gone: bool,
    /// A request is with the backend thread: nothing else is sent until its
    /// [`Reply::Done`].
    busy: bool,
    /// The profiles in the store, by name.
    profiles: Vec<String>,
    /// A question from the backend thread, waiting on the user.
    question: Option<Question>,
    action_drafts: ActionDrafts,
    /// The confirm dialogs, while a change waits for its answer.
    confirming: Option<Confirming>,
    /// What the canvas's keys do.
    keymap: Keymap,
    /// The identify overlay, while it is up.
    identifying: Option<Identifying>,
    /// Identify was asked for: the loop opens it, which needs the GPU.
    identify_asked: bool,
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
    /// The window on `editor`, talking to the backend thread on `channels`,
    /// its canvas's keys as `keymap` binds them.
    pub fn new(
        shell: &'a mut dyn Shell,
        window: WindowId,
        (editor, keymap): (Editor, Keymap),
        extent: (u32, u32),
        channels: Channels,
    ) -> Self {
        let theme = Theme::tokyo_night();
        let mut ui = Ui::new();
        ui.add_stylesheet("dsplr", &panel_ui::stylesheet(&theme));
        let faces = fonts::system_sans();
        let mut notice = faces.note;
        if let Err(error) = ui.register_font(fonts::FAMILY, faces.face.clone()) {
            notice = Some(format!("the panel writes in the bitmap font: {error}"));
        }
        ui.set_fallback_fonts(faces.fallbacks);
        Self {
            shell,
            window,
            editor,
            theme,
            font: faces.face,
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
            notice,
            channels,
            backend_gone: false,
            busy: false,
            profiles: Vec::new(),
            question: None,
            action_drafts: ActionDrafts::default(),
            confirming: None,
            keymap,
            identifying: None,
            identify_asked: false,
        }
    }

    /// Runs until the user quits, drawing with `gpu`, which presents to the
    /// window. `early` are events pumped before the loop started. Ended by an
    /// error, it closes the dialogs and the overlay as a quit does, their
    /// surfaces before the caller destroys `gpu`'s device, and returns the
    /// error.
    pub fn run(&mut self, gpu: &mut Gpu, early: Vec<ShellEvent>) -> Result<(), GuiError> {
        let outcome = self.run_loop(gpu, early);
        if outcome.is_err()
            && let Err(error) = self.close()
        {
            // The error that ended the loop is the one returned.
            eprintln!("dsplr: closing the dialogs after an error: {error}");
        }
        outcome
    }

    fn run_loop(&mut self, gpu: &mut Gpu, mut early: Vec<ShellEvent>) -> Result<(), GuiError> {
        let mut dirty = true;
        loop {
            let mut events = std::mem::take(&mut early);
            self.shell.pump(&mut |event| events.push(event));
            for event in events {
                gpu.observe(&event);
                if let Some(confirming) = &mut self.confirming {
                    if let Some(choice) = confirming.handle(&event)? {
                        self.answer(choice == Choice::Keep)?;
                        dirty = true;
                        continue;
                    }
                    if event.window().is_some_and(|window| confirming.owns(window)) {
                        continue;
                    }
                }
                if let Some(identifying) = &mut self.identifying {
                    identifying.handle(&event)?;
                    if event
                        .window()
                        .is_some_and(|window| identifying.owns(window))
                    {
                        continue;
                    }
                }
                match self.handle(event)? {
                    Step::Idle => {}
                    Step::Redraw => dirty = true,
                    Step::Resized => {
                        gpu.resize(self.extent)?;
                        dirty = true;
                    }
                    Step::Quit => return self.close(),
                }
            }
            loop {
                let (changed, confirm) = self.receive()?;
                dirty |= changed;
                let Some((id, times, layout)) = confirm else {
                    break;
                };
                // The canvas lets go: nothing is edited while a change waits
                // on its answer.
                self.editor.cancel_drag();
                self.ui.clear_focus();
                let (confirming, events, skipped) =
                    Confirming::open(self.shell, gpu, id, times, &layout);
                if let Some(last) = skipped.last() {
                    self.notice = Some(last.clone());
                }
                self.confirming = Some(confirming);
                early.extend(events);
            }
            if std::mem::take(&mut self.identify_asked) {
                early.extend(self.identify(gpu)?);
                dirty = true;
            }
            if self.identifying.as_ref().is_some_and(Identifying::expired)
                && let Some(identifying) = self.identifying.take()
            {
                identifying.close(self.shell)?;
            }
            if let Some(identifying) = &mut self.identifying {
                identifying.present(self.shell, &self.theme, &self.font, &self.atlas)?;
            }
            if self.confirming.as_ref().is_some_and(Confirming::expired) {
                self.answer(false)?;
                dirty = true;
            }
            if let Some(confirming) = &mut self.confirming {
                confirming.present(self.shell, &self.theme, &self.font, &self.atlas)?;
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
            if !early.is_empty() {
                continue;
            }
            self.shell.wait_events(self.timeout());
            dirty |= self.ticking();
        }
    }

    /// How long the loop may wait for the next event: until the countdown
    /// shows another second, the next poll for replies when nothing wakes
    /// it, or the panel's next tick.
    fn timeout(&self) -> Option<Duration> {
        let countdown = self.confirming.as_ref().map(Confirming::next_tick);
        let identify = self.identifying.as_ref().map(Identifying::left);
        let poll = (self.busy && !self.channels.wakes).then_some(REPLY_POLL);
        let over_panel = regions(&self.editor, self.size())
            .panel
            .contains(self.pointer);
        let tick = (over_panel || self.ui.text_editing()).then_some(UI_TICK);
        [countdown, identify, poll, tick]
            .into_iter()
            .flatten()
            .min()
    }

    /// Whether the frame is drawn again after a wait that had a timeout.
    fn ticking(&self) -> bool {
        self.timeout().is_some()
    }

    /// Ends the loop: a change waiting on its answer is reverted first.
    fn close(&mut self) -> Result<(), GuiError> {
        if self.confirming.is_some() {
            self.answer(false)?;
        }
        if let Some(identifying) = self.identifying.take() {
            identifying.close(self.shell)?;
        }
        Ok(())
    }

    /// Opens the identify overlay on the live layout, in place of one up
    /// already. Returns the events pumped meanwhile.
    fn identify(&mut self, gpu: &Gpu) -> Result<Vec<ShellEvent>, GuiError> {
        if let Some(identifying) = self.identifying.take() {
            identifying.close(self.shell)?;
        }
        if !self.shell.caps().contains(ShellCaps::PANEL) {
            self.notice = Some(
                "identify is unsupported here: this window system has no panels to show it on"
                    .to_owned(),
            );
            return Ok(Vec::new());
        }
        let live = self.editor.live().clone();
        let (identifying, events, skipped) = Identifying::open(self.shell, gpu, &live);
        if let Some(last) = skipped.last() {
            self.notice = Some(last.clone());
        }
        self.identifying = identifying;
        Ok(events)
    }

    /// Takes the backend thread's replies waiting, up to a confirm, which it
    /// returns for the caller to open the dialogs with; and notices the
    /// thread gone. Says whether anything changed on screen.
    fn receive(&mut self) -> Result<(bool, Option<Asked>), GuiError> {
        let mut changed = false;
        loop {
            match self.channels.replies.try_recv() {
                Ok(reply) => {
                    changed = true;
                    if let Some(confirm) = self.reply(reply) {
                        return Ok((changed, Some(confirm)));
                    }
                }
                Err(TryRecvError::Empty) => return Ok((changed, None)),
                Err(TryRecvError::Disconnected) => {
                    if !self.backend_gone {
                        self.backend_stopped()?;
                        changed = true;
                    }
                    return Ok((changed, None));
                }
            }
        }
    }

    /// The backend thread has stopped (it panicked: it never stops on its
    /// own while the window is open). Nothing that needs it can be done
    /// now; a change waiting on its answer is closed, and its watchdog,
    /// the detached `dsplr restore --revert --after` the flow started, is
    /// the only way back left.
    fn backend_stopped(&mut self) -> Result<(), GuiError> {
        self.backend_gone = true;
        self.busy = false;
        let notice = match self.confirming.take() {
            Some(confirming) => {
                let left = confirming.watchdog_left().as_secs();
                confirming.close(self.shell)?;
                format!("the backend stopped; the watchdog will revert the change in {left} s")
            }
            None => "the backend stopped; nothing can be applied or saved".to_owned(),
        };
        self.notice = Some(notice);
        Ok(())
    }

    /// Takes `reply` from the backend thread. Returns what a
    /// [`Reply::Confirm`] asks for (its id, its deadline and its watchdog's,
    /// and the layout applied) for the loop to open the dialogs with.
    fn reply(&mut self, reply: Reply) -> Option<Asked> {
        match reply {
            // Edits stay over a change that was not kept, and over the
            // desktop's own changes where their outputs are still there; a
            // kept change's `Done` clears them.
            Reply::Snapshot(snapshot) => {
                if let Some(notice) = self.editor.rebase(snapshot, true).notice() {
                    self.notice = Some(notice);
                }
            }
            Reply::Profiles(names) => self.profiles = names,
            // The bar has one line.
            Reply::Message(line) => self.notice = Some(line.replace('\n', "  ")),
            Reply::Ask(question) => self.question = Some(question),
            Reply::Confirm {
                id,
                deadline,
                watchdog,
                layout,
            } => return Some((id, (deadline, watchdog), layout)),
            Reply::Done(outcome) => {
                self.busy = false;
                if outcome == Some(Outcome::Kept) {
                    let live = self.editor.snapshot().clone();
                    // Nothing pending is left to drop or keep.
                    let rebased = self.editor.rebase(live, false);
                    debug_assert_eq!(rebased, dsplr_core::Rebased::default());
                }
            }
        }
        None
    }

    /// Answers the change waiting in the dialogs, and closes them.
    fn answer(&mut self, keep: bool) -> Result<(), GuiError> {
        let Some(confirming) = self.confirming.take() else {
            return Ok(());
        };
        let id = confirming.id();
        confirming.close(self.shell)?;
        // Gone, the backend thread is noticed with the next replies.
        self.channels
            .answers
            .send(Answer { id, keep })
            .unwrap_or_default();
        Ok(())
    }

    /// Sends `request` to the backend thread, unless one is already with it.
    fn request(&mut self, request: Request) -> bool {
        if self.backend_gone {
            self.notice = Some("the backend stopped; nothing can be applied or saved".to_owned());
        } else if self.busy {
            self.notice = Some("busy: the last request is not finished".to_owned());
        } else if self.channels.requests.send(request).is_err() {
            self.notice = Some("the backend stopped; nothing can be applied or saved".to_owned());
        } else {
            self.busy = true;
        }
        true
    }

    /// Does `command` from the panel or a key.
    fn command(&mut self, command: Command) -> bool {
        self.notice = None;
        let pending = self.editor.has_pending();
        let apply_first = "apply the pending edits first (a), then save";
        match command {
            Command::Apply if !pending => {
                self.notice = Some("no pending edits to apply".to_owned());
                true
            }
            Command::Apply => {
                let asked = self.editor.pending().clone();
                self.request(Request::Apply(asked))
            }
            // Saving keeps the displays as they are, as `dsplr save` does,
            // so edits not applied yet would not be in it.
            Command::Save | Command::SaveProfile(_) if pending => {
                self.notice = Some(apply_first.to_owned());
                true
            }
            Command::Save => self.request(Request::Save { approved: None }),
            Command::SaveProfile(name) if name.is_empty() => {
                self.notice = Some("type a profile name to save as".to_owned());
                true
            }
            Command::SaveProfile(name) => self.request(Request::SaveProfile {
                name,
                replace: false,
            }),
            Command::Restore(name) => self.request(Request::Restore(name)),
            Command::Identify => {
                self.identify_asked = true;
                true
            }
            Command::Revert => self.request(Request::Revert),
            Command::Refresh => self.request(Request::Refresh),
            Command::Approve => match self.question.take() {
                Some(question) => self.request(question.approve),
                None => false,
            },
            Command::Dismiss => {
                self.question = None;
                self.notice = Some("nothing was saved".to_owned());
                true
            }
        }
    }

    fn handle(&mut self, event: ShellEvent) -> Result<Step, GuiError> {
        let editing = self.ui.text_editing() || self.ui.popup_list_open();
        if self.pump.observe(&event, editing) {
            return Ok(Step::Redraw);
        }
        Ok(match event {
            ShellEvent::Resized {
                window,
                size,
                scale_factor,
            }
            | ShellEvent::ScaleFactorChanged {
                window,
                size,
                scale_factor,
            } if window == self.window => {
                self.extent = (size.width, size.height);
                self.scale = scale_factor as f32;
                // A drag's fit was for the old size.
                self.editor.cancel_drag();
                Step::Resized
            }
            ShellEvent::CloseRequested { window } => {
                self.shell.reply_close_request(window, CloseReply::Keep)?;
                // A change waiting on its answer is answered first: no.
                if self.confirming.is_some() {
                    self.answer(false)?;
                }
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
            if on_canvas && !popup && !self.quitting && self.confirming.is_none() {
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
        if self.confirming.is_some() {
            return Ok(match keys::answer(keysym) {
                Some(keep) => {
                    self.answer(keep)?;
                    Step::Redraw
                }
                None => Step::Idle,
            });
        }
        if self.question.is_some() && !editing {
            let changed = match keys::answer(keysym) {
                Some(true) => self.command(Command::Approve),
                Some(false) => self.command(Command::Dismiss),
                None => false,
            };
            return Ok(if changed { Step::Redraw } else { Step::Idle });
        }
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
        Ok(match self.keymap.action(keysym, modifiers) {
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
            Action::Identify => self.command(Command::Identify),
            Action::Apply => self.command(Command::Apply),
            Action::Save => self.command(Command::Save),
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
        let mut built = self.build(pointer, nav, dt, region);
        // Layout showed nodes the build left out (a drop-down that just
        // opened): build again, with this frame's press and keys taken.
        if self.ui.needs_rebuild() {
            let at_rest = PointerInput {
                down: self.pointer_down,
                ..PointerInput::hovering(pointer.pos)
            };
            let again = self.build(at_rest, NavInput::default(), Duration::ZERO, region);
            built.changes.extend(again.changes);
            built.commands.extend(again.commands);
        }
        // A change behind the quit prompt is not made: it would add to the
        // edits the prompt is about to drop, unseen. Nor one while a change
        // waits on its answer.
        let blocked = self.quitting || self.confirming.is_some();
        if let (Some(index), false) = (self.drafts.output(), blocked) {
            for change in built.changes {
                changed |= self.apply(index, change);
            }
        }
        if !blocked {
            for command in built.commands {
                changed |= self.command(command);
            }
        }

        let prompt = self.confirming.as_ref().map(|confirming| {
            let left = confirming.left();
            let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            format!("{QUESTION}  Enter keeps, Esc reverts  ·  reverting in {seconds} s")
        });
        let message = if self.quitting {
            Some(QUIT_PROMPT)
        } else {
            prompt.as_deref().or(self.notice.as_deref())
        };
        let scene = draw(&self.editor, size, &self.theme, message, self.keymap.hint());
        self.list.clear();
        self.list.set_scale(self.scale);
        paint::paint(&scene, &mut self.list, &self.font);
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
    ) -> Built {
        self.ui.begin_frame_with(pointer, nav);
        self.ui.set_text_input(self.pump.frame(dt));
        let panel = self
            .editor
            .selected()
            .and_then(|index| panel(&self.editor, index));
        let actions = Actions {
            busy: self.busy || self.backend_gone,
            profiles: &self.profiles,
            question: self.question.as_ref(),
        };
        let built = panel_ui::build(
            &mut self.ui,
            panel.as_ref(),
            &mut self.drafts,
            &actions,
            &mut self.action_drafts,
        );
        self.ui.layout(
            Vec2::new(region.x, region.y),
            AvailableSpace::definite(Vec2::new(region.width, region.height)),
            &self.atlas,
        );
        built
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

    use std::sync::mpsc::{self, Receiver, Sender};

    use dsplr_core::flow::Outcome;
    use dsplr_core::link::{Reply, Request};

    use super::{Channels, Keymap, MainWindow};

    /// The window's ends of a link, and the backend thread's.
    fn link() -> (Channels, Receiver<Request>, Sender<Reply>) {
        let (requests, sent) = mpsc::channel();
        let (answers, _answered) = mpsc::channel();
        let (reply, replies) = mpsc::channel();
        let channels = Channels {
            requests,
            answers,
            replies,
            wakes: true,
        };
        (channels, sent, reply)
    }

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
        let (link, _sent, _reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );
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
        let (link, _sent, _reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );
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

    #[test]
    fn apply_sends_the_pending_layout_once_and_save_waits_for_it() {
        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let (link, sent, reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );

        for event in typed(&[KeyCode::KeyA]) {
            main.handle(event).unwrap();
        }
        assert!(sent.try_recv().is_err(), "nothing pending: nothing sent");

        for event in typed(&[KeyCode::KeyL, KeyCode::KeyS]) {
            main.handle(event).unwrap();
        }
        assert!(
            sent.try_recv().is_err(),
            "saving waits for the edits to be applied"
        );
        assert!(
            main.notice
                .as_deref()
                .is_some_and(|n| n.contains("apply the pending edits first"))
        );

        for event in typed(&[KeyCode::KeyA, KeyCode::KeyA]) {
            main.handle(event).unwrap();
        }
        let pending = main.editor.pending().clone();
        assert_eq!(sent.try_recv(), Ok(Request::Apply(pending)));
        assert!(sent.try_recv().is_err(), "one request at a time");
        assert!(
            main.notice
                .as_deref()
                .is_some_and(|n| n.starts_with("busy"))
        );

        // Kept: the edits are the live layout now, so none are pending,
        // even where the desktop reports a setting a little differently (a
        // scale rounded to a 256th) from what was asked.
        let mut kept = main.editor.snapshot().clone();
        kept.layout = main.editor.pending().clone();
        kept.layout.entries[1].state.scale = 1.00390625;
        reply.send(Reply::Snapshot(kept)).unwrap();
        reply.send(Reply::Done(Some(Outcome::Kept))).unwrap();
        assert!(main.receive().unwrap().1.is_none(), "nothing to confirm");
        assert!(!main.busy);
        assert!(!main.editor.has_pending());
    }

    #[test]
    fn a_question_takes_enter_to_replace_and_escape_to_cancel() {
        use dsplr_core::link::Question;

        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let (link, sent, _reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );
        let question = Question {
            what: "/h/.config/sway/outputs".into(),
            notes: vec!["was not written by dsplr".into()],
            diff: String::new(),
            approve: Request::Save {
                approved: Some("old".into()),
            },
        };

        assert_eq!(main.reply(Reply::Ask(question.clone())), None);
        for event in typed(&[KeyCode::Escape]) {
            main.handle(event).unwrap();
        }
        assert!(main.question.is_none());
        assert!(sent.try_recv().is_err(), "cancelled: nothing sent");
        assert_eq!(main.notice.as_deref(), Some("nothing was saved"));

        main.reply(Reply::Ask(question.clone()));
        for event in typed(&[KeyCode::Enter]) {
            main.handle(event).unwrap();
        }
        assert_eq!(sent.try_recv(), Ok(question.approve));
        assert!(main.question.is_none());
    }

    /// The backend thread gone (a panic) clears `busy`, says so, and leaves
    /// nothing to send.
    #[test]
    fn a_backend_thread_gone_is_noticed_and_nothing_more_is_sent() {
        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let (link, sent, reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );
        for event in typed(&[KeyCode::KeyL, KeyCode::KeyA]) {
            main.handle(event).unwrap();
        }
        assert!(sent.try_recv().is_ok());
        assert!(main.busy);

        drop(reply);
        let (changed, confirm) = main.receive().unwrap();
        assert!(changed && confirm.is_none());
        assert!(!main.busy, "nothing is with a backend that is gone");
        assert!(main.backend_gone);
        assert!(
            main.notice
                .as_deref()
                .is_some_and(|n| n.starts_with("the backend stopped")),
            "{:?}",
            main.notice
        );
        for event in typed(&[KeyCode::KeyS]) {
            main.handle(event).unwrap();
        }
        assert!(sent.try_recv().is_err(), "nothing sent to it");
    }

    /// The desktop's own change arrives as a snapshot: an output gone takes
    /// its pending edits with it, and the bar says so.
    #[test]
    fn an_output_gone_from_the_desktop_drops_its_edits_with_a_note() {
        let mut shell = HeadlessShell::new();
        let window = shell.create_window(&WindowDesc::default()).unwrap();
        let (link, _sent, reply) = link();
        let mut main = MainWindow::new(
            &mut shell,
            window,
            (editor(), Keymap::default()),
            (1280, 800),
            link,
        );
        // DP-1 is selected first: move it.
        for event in typed(&[KeyCode::KeyL]) {
            main.handle(event).unwrap();
        }
        assert!(main.editor.has_pending());
        let mut unplugged = main.editor.snapshot().clone();
        unplugged.layout.entries.remove(0);
        reply.send(Reply::Snapshot(unplugged)).unwrap();
        main.receive().unwrap();
        assert!(!main.editor.has_pending(), "DP-1's edit went with it");
        assert_eq!(
            main.notice.as_deref(),
            Some("DP-1 went away, and the pending edits to it with it")
        );
    }
}
