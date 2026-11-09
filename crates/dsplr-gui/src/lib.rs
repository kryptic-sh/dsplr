//! The crcbl windows (`docs/plan/06-gui.md`). So far the main window: the
//! canvas and the read-only property panel dsplr-core draws, Tab and click
//! selection, undo and redo, and quitting.
//!
//! Not here yet: dragging, editing in the panel, Apply and Save with their
//! confirm dialogs, the identify overlays, configurable keys, and following
//! the desktop's changes.

mod gpu;
mod keys;
mod paint;

use std::time::{Duration, Instant};

use crcbl_app::backend::GpuError;
use crcbl_app::present::PresentError;
use crcbl_frame::graph::GraphError;
use crcbl_hal::HalError;
use crcbl_shell::{
    ButtonState, CloseReply, LogicalSize, PointerButton, Shell, ShellError, ShellEvent, WindowDesc,
    WindowId, WindowRole,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::{Editor, Point, Size, Theme, draw};

use crate::gpu::Gpu;
use crate::keys::Action;

/// The window's size when it opens, in logical pixels.
const WINDOW_SIZE: LogicalSize = LogicalSize {
    width: 1280.0,
    height: 800.0,
};
/// How long the window system gets to size a new window before dsplr gives
/// up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the window system after an out-of-date swapchain
/// before drawing again: a new size is usually on its way.
const PRESENT_RETRY: Duration = Duration::from_millis(16);

/// Why the GUI could not run.
#[derive(Debug, thiserror::Error)]
pub enum GuiError {
    #[error("cannot open a window: {0}")]
    Shell(#[from] ShellError),
    #[error("the window system never gave the window a size")]
    NeverSized,
    /// No GPU backend opened. Kept apart from [`GuiError::NoGpu`] because
    /// crcbl's own wording for it suggests a `--backend` flag dsplr lacks.
    #[error("no usable GPU: {0}")]
    NoGpuBackend(GpuError),
    #[error("no usable GPU: {0}")]
    NoGpu(PresentError),
    #[error("presenting a frame: {0}")]
    Present(#[from] PresentError),
    #[error("drawing a frame: {0}")]
    Hal(#[from] HalError),
    #[error("drawing a frame: {0}")]
    Graph(#[from] GraphError),
}

/// Opens the main window on `editor` and runs it until the user quits.
pub fn run(editor: Editor) -> Result<(), GuiError> {
    let mut shell = crcbl_shell::open()?;
    let window = shell.create_window(&WindowDesc {
        title: "dsplr",
        app_id: "dsplr",
        size: WINDOW_SIZE,
        resizable: true,
        role: WindowRole::Toplevel,
        ..WindowDesc::default()
    })?;
    let (extent, early) = wait_for_size(shell.as_mut(), window)?;
    let mut gpu = Gpu::open(shell.as_ref(), window, extent)?;
    let outcome = MainWindow {
        shell: shell.as_mut(),
        gpu: &mut gpu,
        editor,
        theme: Theme::tokyo_night(),
        font: Font::sans(),
        atlas: FontAtlas::built_in(),
        list: DrawList::new(),
        extent,
        scale: 1.0,
        pointer: Point::new(0.0, 0.0),
    }
    .run(early);
    // The surface goes before the window it was made from.
    gpu.destroy();
    drop(shell);
    outcome
}

/// Pumps until the window system gives the new window a size. Returns the
/// size and the events pumped on the way, which the window loop handles
/// first: the scale factor arrives here.
fn wait_for_size(
    shell: &mut dyn Shell,
    window: WindowId,
) -> Result<((u32, u32), Vec<ShellEvent>), GuiError> {
    let started = Instant::now();
    let mut events = Vec::new();
    loop {
        shell.pump(&mut |event| events.push(event));
        if let Some(size) = shell.window_state(window)?.size() {
            return Ok(((size.width, size.height), events));
        }
        if started.elapsed() > CONFIGURE_TIMEOUT {
            return Err(GuiError::NeverSized);
        }
        shell.wait_events(Some(Duration::from_millis(8)));
    }
}

struct MainWindow<'a> {
    shell: &'a mut dyn Shell,
    gpu: &'a mut Gpu,
    editor: Editor,
    theme: Theme,
    font: &'static Font,
    /// crcbl's built-in glyph atlas, which its UI pass takes alongside the
    /// parsed font's glyphs.
    atlas: FontAtlas,
    list: DrawList,
    /// The window's size in device pixels, and device pixels per logical
    /// pixel.
    extent: (u32, u32),
    scale: f32,
    /// The pointer, in logical pixels.
    pointer: Point,
}

/// What the loop does after an event.
enum Step {
    /// Nothing on screen changed.
    Idle,
    Redraw,
    Quit,
}

impl MainWindow<'_> {
    fn run(&mut self, mut early: Vec<ShellEvent>) -> Result<(), GuiError> {
        let mut dirty = true;
        loop {
            let mut events = std::mem::take(&mut early);
            self.shell.pump(&mut |event| events.push(event));
            for event in events {
                self.gpu.observe(&event);
                match self.handle(event)? {
                    Step::Idle => {}
                    Step::Redraw => dirty = true,
                    Step::Quit => return Ok(()),
                }
            }
            if dirty && self.gpu.frame_due() {
                self.frame();
                let background = paint::linear(self.theme.background);
                if self
                    .gpu
                    .present(self.shell, &self.list, &self.atlas, background)?
                {
                    dirty = false;
                } else {
                    // Out of date and reconfigured. Retrying at once would
                    // spin while it stays out of date.
                    self.shell.wait_events(Some(PRESENT_RETRY));
                }
                continue;
            }
            self.shell.wait_events(None);
        }
    }

    fn handle(&mut self, event: ShellEvent) -> Result<Step, GuiError> {
        Ok(match event {
            ShellEvent::Resized {
                size, scale_factor, ..
            }
            | ShellEvent::ScaleFactorChanged {
                size, scale_factor, ..
            } => {
                self.extent = (size.width, size.height);
                self.scale = scale_factor as f32;
                self.gpu.resize(self.extent)?;
                Step::Redraw
            }
            ShellEvent::CloseRequested { window } => {
                self.shell.reply_close_request(window, CloseReply::Keep)?;
                Step::Quit
            }
            ShellEvent::Key {
                keysym,
                state: ButtonState::Pressed,
                modifiers,
                ..
            } => match keys::action(keysym, modifiers) {
                Some(action) => self.act(action),
                None => Step::Idle,
            },
            ShellEvent::PointerMotion { abs: Some(at), .. }
            | ShellEvent::PointerFocus {
                entered: true,
                position: Some(at),
                ..
            } => {
                self.pointer = self.logical(at.x, at.y);
                Step::Idle
            }
            ShellEvent::Button {
                button: PointerButton::Left,
                state: ButtonState::Pressed,
                position,
                ..
            } => {
                if let Some(at) = position {
                    self.pointer = self.logical(at.x, at.y);
                }
                let scene = draw(&self.editor, self.size(), &self.theme);
                match scene.hit(self.pointer) {
                    Some(output) => {
                        self.editor.select(Some(output));
                        Step::Redraw
                    }
                    None => Step::Idle,
                }
            }
            _ => Step::Idle,
        })
    }

    fn act(&mut self, action: Action) -> Step {
        let changed = match action {
            Action::SelectNext => {
                self.editor.select_next();
                true
            }
            Action::SelectPrevious => {
                self.editor.select_previous();
                true
            }
            Action::Undo => self.editor.undo(),
            Action::Redo => self.editor.redo(),
            Action::Quit => return Step::Quit,
        };
        if changed { Step::Redraw } else { Step::Idle }
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

    /// Records this frame's draw list, in logical pixels drawn at the
    /// window's scale.
    fn frame(&mut self) {
        let scene = draw(&self.editor, self.size(), &self.theme);
        self.list.clear();
        self.list.set_scale(self.scale);
        paint::paint(&scene, &mut self.list, self.font);
    }
}
