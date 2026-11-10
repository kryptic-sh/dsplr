//! The identify overlay (`docs/plan/06-gui.md`, "Identify overlay"): each
//! monitor's output, its connector and description in large text, on that
//! monitor ([`crate::panels`]) for [`IDENTIFY_TIME`]. It takes no keys; in
//! the main window a click on it does nothing, and shown alone ([`show`], for
//! `dsplr identify`) a click closes it. A window system without panels has
//! nothing to show it with: the caller says it is unsupported there.

use std::time::{Duration, Instant};

use crcbl_shell::{Shell, ShellCaps, ShellEvent, WindowId};
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::Theme;
use dsplr_core::dialog::{IDENTIFY_SIZE, IDENTIFY_TIME, draw_identify};
use dsplr_model::Layout;

use crate::gpu::Gpu;
use crate::panels::{Kind, Panels};
use crate::{GuiError, fonts};

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
        main: Option<&Gpu>,
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

    /// Takes what `event` means to the overlay: its windows' sizes. Says
    /// whether it was a click on one of them.
    pub fn handle(&mut self, event: &ShellEvent) -> Result<bool, GuiError> {
        Ok(self.panels.handle(event)?.is_some())
    }

    /// What each panel names: its output, or its monitor where none matched.
    pub fn labels(&self) -> Vec<String> {
        self.panels.labels()
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
                let description = panel.shown.as_ref().and_then(|s| s.description.as_deref());
                draw_identify(panel.label(), description, size, theme)
            })
    }

    pub fn close(self, shell: &mut dyn Shell) -> Result<(), GuiError> {
        self.panels.close(shell)
    }
}

/// What a window system without panels is told when identify is asked of it.
pub const IDENTIFY_UNSUPPORTED: &str =
    "identify is unsupported here: this window system has no panels to show it on";

/// How the overlay shown alone ([`show`]) ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identified {
    /// It was shown, until its time was up or a panel was clicked.
    Shown,
    /// The window system has no panels to show it on.
    Unsupported,
    /// It opened on no monitor; each one's reason was said.
    NotShown,
}

/// Shows the overlay for the outputs of `layout` on every monitor `shell`
/// knows, without the main window, until [`IDENTIFY_TIME`] is up or a panel
/// is clicked. What there is to tell (which outputs are shown, each monitor
/// skipped and why, the font's note) goes to `say`, a line each, as it
/// happens.
pub fn show(
    shell: &mut dyn Shell,
    layout: &Layout,
    mut say: impl FnMut(&str),
) -> Result<Identified, GuiError> {
    if !shell.caps().contains(ShellCaps::PANEL) {
        return Ok(Identified::Unsupported);
    }
    let (identifying, early, skipped) = Identifying::open(shell, None, layout);
    for line in &skipped {
        say(line);
    }
    let Some(mut identifying) = identifying else {
        return Ok(Identified::NotShown);
    };
    say(&format!(
        "identifying {} for {}s",
        identifying.labels().join(", "),
        IDENTIFY_TIME.as_secs()
    ));
    let faces = fonts::system_sans();
    if let Some(note) = &faces.note {
        say(note);
    }
    let theme = Theme::tokyo_night();
    let atlas = FontAtlas::built_in();
    let shown = run(
        shell,
        &mut identifying,
        early,
        (&theme, &faces.face, &atlas),
    );
    // The panels go on every path, their presenters before their windows.
    let closed = identifying.close(shell);
    shown.and(closed).map(|()| Identified::Shown)
}

/// [`show`]'s loop: draws the panels as their windows ask, until the time is
/// up or one is clicked.
fn run(
    shell: &mut dyn Shell,
    identifying: &mut Identifying,
    mut events: Vec<ShellEvent>,
    (theme, font, atlas): (&Theme, &Font, &FontAtlas),
) -> Result<(), GuiError> {
    loop {
        shell.pump(&mut |event| events.push(event));
        for event in events.drain(..) {
            if identifying.handle(&event)? {
                return Ok(());
            }
        }
        if identifying.expired() {
            return Ok(());
        }
        identifying.present(shell, theme, font, atlas)?;
        shell.wait_events(Some(identifying.left()));
    }
}

#[cfg(test)]
mod tests {
    use crcbl_shell::{ButtonState, HeadlessShell, PhysicalPoint, PointerButton};

    use super::*;

    #[test]
    fn a_window_system_without_panels_cannot_show_it() {
        let mut shell = HeadlessShell::new().with_caps(ShellCaps::empty());
        let mut said = Vec::new();
        let shown = show(&mut shell, &Layout::default(), |line| {
            said.push(line.to_owned())
        });
        assert_eq!(shown.unwrap(), Identified::Unsupported);
        assert!(said.is_empty(), "{said:?}");
    }

    /// What the GPU tests below need, said where they are skipped: a
    /// Vulkan device presenting offscreen, lavapipe will do.
    const NEEDS_VULKAN: &str =
        "needs a Vulkan device: VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json";

    #[test]
    #[ignore = "needs a Vulkan device (see NEEDS_VULKAN)"]
    fn shown_alone_it_stays_its_time_and_names_what_it_shows() {
        let mut shell = HeadlessShell::new();
        let mut said = Vec::new();
        let started = Instant::now();
        let shown = show(&mut shell, &Layout::default(), |line| {
            said.push(line.to_owned())
        });
        assert_eq!(shown.expect(NEEDS_VULKAN), Identified::Shown, "{said:?}");
        assert!(started.elapsed() >= IDENTIFY_TIME);
        let monitor = &shell.monitors()[0].name;
        let opened = format!("identifying {monitor} for {}s", IDENTIFY_TIME.as_secs());
        assert!(said.contains(&opened), "{said:?}");
    }

    #[test]
    #[ignore = "needs a Vulkan device (see NEEDS_VULKAN)"]
    fn a_click_on_a_panel_closes_it_early() {
        let mut shell = HeadlessShell::new();
        let (identifying, early, skipped) = Identifying::open(&mut shell, None, &Layout::default());
        let mut identifying = identifying.unwrap_or_else(|| panic!("{NEEDS_VULKAN}: {skipped:?}"));
        let panel = early
            .iter()
            .filter_map(ShellEvent::window)
            .find(|window| identifying.owns(*window))
            .expect("the panel was sized while it opened");
        shell
            .button(
                panel,
                PointerButton::Left,
                ButtonState::Pressed,
                Some(PhysicalPoint::new(10.0, 10.0)),
            )
            .unwrap();
        let theme = Theme::tokyo_night();
        let faces = fonts::system_sans();
        run(
            &mut shell,
            &mut identifying,
            early,
            (&theme, &faces.face, &FontAtlas::built_in()),
        )
        .unwrap();
        // Its time is not up: the click ended it.
        assert!(!identifying.expired(), "{:?} left", identifying.left());
        identifying.close(&mut shell).unwrap();
    }
}
