//! The identify overlay (`docs/plan/06-gui.md`, "Identify overlay"): each
//! monitor's output, its connector and description in large text, on that
//! monitor ([`crate::panels`]) for [`IDENTIFY_TIME`]. It takes no keys and no
//! clicks. A window system without panels has nothing to show it with: the
//! caller says it is unsupported there.

use std::time::{Duration, Instant};

use crcbl_shell::{Shell, ShellEvent, WindowId};
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::Theme;
use dsplr_core::dialog::{IDENTIFY_SIZE, IDENTIFY_TIME, draw_identify};
use dsplr_model::Layout;

use crate::GuiError;
use crate::gpu::Gpu;
use crate::panels::{Kind, Panels};

const KIND: Kind = Kind {
    namespace: "dsplr-identify",
    title: "dsplr: identify",
    size: IDENTIFY_SIZE,
    takes_keys: false,
    toplevel_fallback: false,
};

/// The overlay, until it goes.
pub struct Identifying {
    until: Instant,
    panels: Panels,
}

impl Identifying {
    /// Opens the overlay on every monitor, for the outputs of `layout`.
    /// Returns the events pumped meanwhile and a line for each monitor
    /// skipped; `None` when it could be opened on none, with why.
    pub fn open(
        shell: &mut dyn Shell,
        main: &Gpu,
        layout: &Layout,
    ) -> (Option<Self>, Vec<ShellEvent>, Vec<String>) {
        let (panels, events, skipped) = Panels::open(shell, main, &KIND, layout);
        let skipped: Vec<String> = skipped
            .into_iter()
            .map(|(monitor, error)| {
                format!(
                    "cannot identify {}: {error}",
                    monitor.as_deref().unwrap_or("a monitor")
                )
            })
            .collect();
        let identifying = (!panels.is_empty()).then(|| Self {
            until: Instant::now() + IDENTIFY_TIME,
            panels,
        });
        (identifying, events, skipped)
    }

    pub fn owns(&self, window: WindowId) -> bool {
        self.panels.owns(window)
    }

    /// Takes what `event` means to the overlay: its windows' sizes.
    pub fn handle(&mut self, event: &ShellEvent) -> Result<(), GuiError> {
        self.panels.handle(event).map(drop)
    }

    /// Whether its time is up.
    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// How long until its time is up.
    pub fn left(&self) -> Duration {
        self.until.saturating_duration_since(Instant::now())
    }

    pub fn present(
        &mut self,
        shell: &mut dyn Shell,
        theme: &Theme,
        font: &Font,
        atlas: &FontAtlas,
    ) -> Result<(), GuiError> {
        self.panels
            .present(shell, (theme.surface, font, atlas), |_, panel, size| {
                match &panel.shown {
                    Some(shown) => {
                        draw_identify(&shown.name, shown.description.as_deref(), size, theme)
                    }
                    // A monitor that matched no output: the window system's
                    // own name for it.
                    None => draw_identify(&panel.monitor, None, size, theme),
                }
            })
    }

    pub fn close(self, shell: &mut dyn Shell) -> Result<(), GuiError> {
        self.panels.close(shell)
    }
}
