//! The crcbl windows (`docs/plan/06-gui.md`): the main window, with the
//! canvas dsplr-core draws, where outputs are dragged and moved with keys,
//! and the property panel, crcbl-ui widgets editing the selected output and
//! the actions (Apply, Save, Revert, profiles), every edit pending with undo
//! and redo; and the confirm dialogs, one per monitor. The backend is on a
//! thread of its own ([`Link`]).
//!
//! Not here yet: the identify overlays, configurable keys, and following
//! the desktop's changes.

mod confirm;
mod fonts;
mod gpu;
mod identify;
mod keys;
mod paint;
mod panel;
mod panels;
mod window;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crcbl_app::backend::GpuError;
use crcbl_app::present::PresentError;
use crcbl_frame::graph::GraphError;
use crcbl_hal::HalError;
use crcbl_shell::{LogicalSize, Shell, ShellError, ShellEvent, WindowDesc, WindowId, WindowRole};
use dsplr_config::keys::Keys;
use dsplr_core::Editor;
use dsplr_core::link::{Answer, Reply, Request};

use crate::gpu::Gpu;

/// The window's size when it opens, in logical pixels.
const WINDOW_SIZE: LogicalSize = LogicalSize {
    width: 1280.0,
    height: 800.0,
};
/// How long the window system gets to size a new window before dsplr gives
/// up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// The window's line to the backend thread (`docs/plan/02-architecture.md`,
/// "Process").
pub struct Link {
    pub requests: Sender<Request>,
    /// The answers to [`Reply::Confirm`], on their own channel.
    pub answers: Sender<Answer>,
    pub replies: Receiver<Reply>,
}

/// Opens the main window on `editor`, with `link` to the backend thread and
/// the canvas's keys as `keys` binds them, and runs it until the user quits. Dropping `link` on the way out tells the
/// backend thread to finish.
pub fn run(editor: Editor, link: Link, keys: &Keys) -> Result<(), GuiError> {
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
    let (replies, wakes) = relay(shell.as_mut(), link.replies);
    let outcome = window::MainWindow::new(
        shell.as_mut(),
        window,
        (editor, keys::Keymap::new(keys)),
        extent,
        window::Channels {
            requests: link.requests,
            answers: link.answers,
            replies,
            wakes,
        },
    )
    .run(&mut gpu, early);
    // The surface goes before the window it was made from.
    gpu.destroy();
    drop(shell);
    outcome
}

/// Relays `replies` to the loop, waking it as each arrives, as pikr's
/// `forward` does. The flag says whether it can: without the window system's
/// waker the loop has to poll.
fn relay(shell: &mut dyn Shell, replies: Receiver<Reply>) -> (Receiver<Reply>, bool) {
    let Some(waker) = shell.waker() else {
        return (replies, false);
    };
    let (forward, relayed) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(reply) = replies.recv() {
            // The loop has gone and wants no more.
            if forward.send(reply).is_err() {
                return;
            }
            waker.wake();
        }
        // The backend thread is gone: the loop sees the channel close once
        // this one drops, and is woken to look.
        drop(forward);
        waker.wake();
    });
    (relayed, true)
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
