//! The confirm dialogs (`docs/plan/06-gui.md`, "Confirm dialog"): "Keep
//! these display settings?" on every monitor ([`crate::panels`]), so it is on
//! whichever still shows; a single toplevel where the window system has no
//! panels. Enter or Keep keeps; Esc, Revert or the countdown running out
//! reverts.

use std::time::{Duration, Instant};

use crcbl_shell::{Shell, ShellEvent, WindowId};
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::Theme;
use dsplr_core::dialog::{Choice, DIALOG_SIZE, DialogFrame, draw_dialog};
use dsplr_model::Layout;

use crate::GuiError;
use crate::gpu::Gpu;
use crate::panels::{Kind, Panels};

/// The dialogs of one change, until it is answered.
pub struct Confirming {
    /// The confirm this answers.
    id: u64,
    deadline: Instant,
    /// When the change's watchdog reverts it, should nothing else.
    watchdog: Instant,
    panels: Panels,
    /// What each panel last drew, for its buttons.
    frames: Vec<Option<DialogFrame>>,
}

const KIND: Kind = Kind {
    namespace: "dsplr-confirm",
    title: "dsplr: keep these display settings?",
    size: DIALOG_SIZE,
    // Enter and Esc work from any; one takes the keys.
    takes_keys: true,
    toplevel_fallback: true,
};

impl Confirming {
    /// Opens a dialog on every monitor `shell` knows, each named for the
    /// output of `layout` it shows, drawing on `main`'s device, and counting
    /// down to `deadline`, the backend thread's own, for the confirm `id`.
    ///
    /// A dialog that cannot be opened is skipped, and the rest go on; with
    /// none open, one toplevel dialog is tried; with that failing too, there
    /// is none, and the main window's bar asks. Returns the events pumped
    /// while the windows were sized, for the caller to handle, and a line for
    /// each dialog skipped.
    pub fn open(
        shell: &mut dyn Shell,
        main: &Gpu,
        id: u64,
        (deadline, watchdog): (Instant, Instant),
        layout: &Layout,
    ) -> (Self, Vec<ShellEvent>, Vec<String>) {
        let (panels, events, skipped) = Panels::open(shell, main, &KIND, layout);
        let skipped = skipped
            .into_iter()
            .map(|(monitor, error)| match monitor {
                Some(monitor) => format!("no confirm dialog on {monitor}: {error}"),
                None => format!(
                    "no confirm dialog could be opened ({error}); answer here: Enter keeps, Esc reverts"
                ),
            })
            .collect();
        let confirming = Self {
            id,
            deadline,
            watchdog,
            panels,
            frames: Vec::new(),
        };
        (confirming, events, skipped)
    }

    /// The confirm this answers.
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// How long until the change's watchdog reverts it, should nothing else.
    pub fn watchdog_left(&self) -> Duration {
        self.watchdog.saturating_duration_since(Instant::now())
    }

    /// How long until the countdown runs out.
    pub fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Whether `window` is one of the dialogs.
    pub fn owns(&self, window: WindowId) -> bool {
        self.panels.owns(window)
    }

    /// Takes what `event` means to the dialogs. Returns the answer a click
    /// on a button gave.
    pub fn handle(&mut self, event: &ShellEvent) -> Result<Option<Choice>, GuiError> {
        let press = self.panels.handle(event)?;
        Ok(press.and_then(|(index, at)| self.frames.get(index)?.as_ref()?.hit(at)))
    }

    /// Whether the countdown has run out.
    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// How long until the countdown shows another second.
    pub fn next_tick(&self) -> Duration {
        let left = self.left();
        let into_second = Duration::from_nanos(u64::from(left.subsec_nanos()));
        if into_second.is_zero() {
            Duration::from_secs(1).min(left)
        } else {
            into_second
        }
    }

    /// Draws and presents each dialog whose window wants a frame.
    pub fn present(
        &mut self,
        shell: &mut dyn Shell,
        theme: &Theme,
        font: &Font,
        atlas: &FontAtlas,
    ) -> Result<(), GuiError> {
        let left = self.left();
        let frames = &mut self.frames;
        self.panels
            .present(shell, (theme.surface, font, atlas), |index, panel, size| {
                let name = panel.shown.as_ref().map(|shown| shown.name.as_str());
                let frame = draw_dialog(name, left, size, theme);
                let list = frame.list.clone();
                if frames.len() <= index {
                    frames.resize(index + 1, None);
                }
                frames[index] = Some(frame);
                list
            })
    }

    /// Closes every dialog.
    pub fn close(self, shell: &mut dyn Shell) -> Result<(), GuiError> {
        self.panels.close(shell)
    }
}
