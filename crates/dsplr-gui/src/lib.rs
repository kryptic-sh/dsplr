//! The crcbl windows (`docs/plan/06-gui.md`). So far the main window: the
//! canvas dsplr-core draws, where outputs are dragged and moved with keys,
//! and the property panel, crcbl-ui widgets editing the selected output;
//! every edit pending, with undo and redo, and quitting asks before dropping
//! them.
//!
//! Not here yet: Apply and Save with their confirm dialogs, the identify
//! overlays, configurable keys, and following the desktop's changes.

mod gpu;
mod keys;
mod paint;
mod panel;
mod window;

use std::time::{Duration, Instant};

use crcbl_app::backend::GpuError;
use crcbl_app::present::PresentError;
use crcbl_frame::graph::GraphError;
use crcbl_hal::HalError;
use crcbl_shell::{LogicalSize, Shell, ShellError, ShellEvent, WindowDesc, WindowId, WindowRole};
use dsplr_core::Editor;

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
    let outcome =
        window::MainWindow::new(shell.as_mut(), window, editor, extent).run(&mut gpu, early);
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
