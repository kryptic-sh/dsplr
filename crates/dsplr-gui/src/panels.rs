//! A small window on every monitor, each a crcbl panel (layer-shell on
//! Wayland, override-redirect on X11) drawing on the main window's device:
//! what the confirm dialogs and the identify overlay are made of. Each knows
//! which output its monitor shows.

use crcbl_shell::{
    ButtonState, MonitorInfo, PanelAnchor, PanelDesc, PanelKeyboard, PanelLayer, PointerButton,
    Shell, ShellCaps, ShellEvent, WindowDesc, WindowId, WindowRole,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::dialog::{Screen, match_screens};
use dsplr_core::panel::name;
use dsplr_core::{Color, Point, Size};
use dsplr_model::Layout;

use crate::gpu::Gpu;
use crate::{GuiError, paint, wait_for_size};

/// How a set of panels opens.
pub struct Kind {
    /// The layer-shell namespace window-system rules match it by.
    pub namespace: &'static str,
    pub title: &'static str,
    pub size: Size,
    /// The first panel takes the keyboard; else none does.
    pub takes_keys: bool,
    /// Where the window system has no panels, or none opens, one toplevel
    /// window instead; else nothing.
    pub toplevel_fallback: bool,
}

/// The output a panel's monitor shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Its name as the canvas heads it.
    pub name: String,
    pub description: Option<String>,
}

/// A panel that could not be opened: its monitor (`None` for the
/// toplevel), and why.
pub type Skipped = (Option<String>, GuiError);

/// One panel.
pub struct Panel {
    window: WindowId,
    gpu: Gpu,
    /// The output its monitor shows, when one matched.
    pub shown: Option<Shown>,
    /// The window system's name for its monitor.
    pub monitor: String,
    extent: (u32, u32),
    scale: f32,
    list: DrawList,
}

/// The panels of one dialog or overlay.
pub struct Panels {
    panels: Vec<Panel>,
}

impl Panels {
    /// Opens a panel of `kind` on every monitor `shell` knows, each knowing
    /// the output of `layout` it shows, drawing on `main`'s device. A panel
    /// that cannot be opened (a monitor gone since the list was read, a
    /// presenter refused) is closed and skipped, and the rest go on. Returns
    /// the events pumped while the windows were sized, for the caller to
    /// handle, and what was skipped: the monitor (`None` for the toplevel)
    /// and why.
    pub fn open(
        shell: &mut dyn Shell,
        main: &Gpu,
        kind: &Kind,
        layout: &Layout,
    ) -> (Self, Vec<ShellEvent>, Vec<Skipped>) {
        let mut panels = Self { panels: Vec::new() };
        let mut events = Vec::new();
        let mut skipped = Vec::new();
        let monitors: Vec<MonitorInfo> = shell.monitors().to_vec();
        if shell.caps().contains(ShellCaps::PANEL) {
            let screens: Vec<Screen> = monitors.iter().map(screen).collect();
            let outputs = match_screens(&screens, layout);
            for (index, (monitor, output)) in monitors.iter().zip(outputs).enumerate() {
                let shown = output.map(|output| {
                    let entry = &layout.entries[output];
                    Shown {
                        name: name(entry),
                        description: entry.description(),
                    }
                });
                let opened = panels.add(
                    shell,
                    main,
                    kind,
                    Some((index, monitor)),
                    shown,
                    &mut events,
                );
                if let Err(error) = opened {
                    skipped.push((Some(monitor.name.clone()), error));
                }
            }
        }
        if kind.toplevel_fallback
            && panels.panels.is_empty()
            && let Err(error) = panels.add(shell, main, kind, None, None, &mut events)
        {
            skipped.push((None, error));
        }
        (panels, events, skipped)
    }

    /// Opens one panel: on `monitor` (the first may take the keys), or a
    /// toplevel without one. A window made and then not drawable is
    /// destroyed again.
    fn add(
        &mut self,
        shell: &mut dyn Shell,
        main: &Gpu,
        kind: &Kind,
        monitor: Option<(usize, &MonitorInfo)>,
        shown: Option<Shown>,
        events: &mut Vec<ShellEvent>,
    ) -> Result<(), GuiError> {
        let role = match monitor {
            Some((index, monitor)) => WindowRole::Panel(PanelDesc {
                namespace: kind.namespace,
                monitor: Some(monitor.id),
                layer: PanelLayer::Overlay,
                // No edge: centred on its monitor.
                anchor: PanelAnchor::empty(),
                keyboard: match (kind.takes_keys, index) {
                    (true, 0) => PanelKeyboard::Exclusive,
                    (true, _) => PanelKeyboard::OnDemand,
                    (false, _) => PanelKeyboard::None,
                },
                ..PanelDesc::default()
            }),
            None => WindowRole::Toplevel,
        };
        let window = shell.create_window(&WindowDesc {
            title: kind.title,
            app_id: "dsplr",
            size: crcbl_shell::LogicalSize::new(
                f64::from(kind.size.width),
                f64::from(kind.size.height),
            ),
            resizable: false,
            role,
            ..WindowDesc::default()
        })?;
        let drawable = wait_for_size(shell, window).and_then(|(extent, pumped)| {
            events.extend(pumped);
            Ok((main.open_shared(shell, window, extent)?, extent))
        });
        match drawable {
            Ok((gpu, extent)) => {
                self.panels.push(Panel {
                    window,
                    gpu,
                    shown,
                    monitor: monitor.map_or_else(String::new, |(_, m)| m.name.clone()),
                    extent,
                    scale: 1.0,
                    list: DrawList::new(),
                });
                Ok(())
            }
            // A window left standing is worse than the reason it could not
            // be drawn on, so that is the error when both happen.
            Err(error) => match shell.destroy_window(window) {
                Ok(()) => Err(error),
                Err(standing) => Err(GuiError::Shell(standing)),
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    /// Whether `window` is one of the panels.
    pub fn owns(&self, window: WindowId) -> bool {
        self.panels.iter().any(|panel| panel.window == window)
    }

    /// Takes what `event` means to the panels: a frame's answer, a size for
    /// one. Returns a press of the primary button on one: its index, and
    /// where in logical pixels.
    pub fn handle(&mut self, event: &ShellEvent) -> Result<Option<(usize, Point)>, GuiError> {
        for panel in &mut self.panels {
            panel.gpu.observe(event);
        }
        match *event {
            ShellEvent::Resized {
                window,
                size,
                scale_factor,
            }
            | ShellEvent::ScaleFactorChanged {
                window,
                size,
                scale_factor,
            } => {
                if let Some(panel) = self.panels.iter_mut().find(|p| p.window == window) {
                    panel.extent = (size.width, size.height);
                    panel.scale = scale_factor as f32;
                    panel.gpu.resize(panel.extent)?;
                }
                Ok(None)
            }
            ShellEvent::Button {
                window,
                button: PointerButton::Left,
                state: ButtonState::Pressed,
                position: Some(at),
                ..
            } => Ok(self
                .panels
                .iter()
                .position(|panel| panel.window == window)
                .map(|index| {
                    let scale = self.panels[index].scale;
                    (index, Point::new(at.x as f32 / scale, at.y as f32 / scale))
                })),
            _ => Ok(None),
        }
    }

    /// Draws and presents each panel whose window wants a frame, with what
    /// `draw` makes of it (its index, the panel, its size in logical pixels)
    /// on `background`.
    pub fn present(
        &mut self,
        shell: &mut dyn Shell,
        (background, font, atlas): (Color, &Font, &FontAtlas),
        mut draw: impl FnMut(usize, &Panel, Size) -> dsplr_core::DrawList,
    ) -> Result<(), GuiError> {
        for (index, panel) in self.panels.iter_mut().enumerate() {
            if !panel.gpu.frame_due() {
                continue;
            }
            let size = Size {
                width: panel.extent.0 as f32 / panel.scale,
                height: panel.extent.1 as f32 / panel.scale,
            };
            let scene = draw(index, panel, size);
            panel.list.clear();
            panel.list.set_scale(panel.scale);
            paint::paint(&scene, &mut panel.list, font);
            // An out-of-date swapchain was reconfigured; the next tick draws.
            panel
                .gpu
                .present(shell, &panel.list, atlas, paint::linear(background))?;
        }
        Ok(())
    }

    /// Closes every panel, each surface before its window, going on past a
    /// window that will not close; the first such error is returned.
    pub fn close(self, shell: &mut dyn Shell) -> Result<(), GuiError> {
        let mut outcome = Ok(());
        for panel in self.panels {
            panel.gpu.destroy();
            if let Err(error) = shell.destroy_window(panel.window)
                && outcome.is_ok()
            {
                outcome = Err(GuiError::Shell(error));
            }
        }
        outcome
    }
}

/// `monitor` as dsplr matches it to an output, by its logical position.
/// crcbl-shell 0.18.0 reports a Wayland monitor's position as its
/// `xdg_output` logical position times its scale where the compositor has
/// `xdg_output`, and as `wl_output.geometry`'s logical position as it is
/// where not, without saying which; so both are offered, the first first. Its
/// size is the untransformed mode, which a turned output's logical size is
/// not, so it is not compared. No connector name yet (C3, on crcbl's main).
fn screen(monitor: &MonitorInfo) -> Screen {
    let scale = monitor.scale_factor as f32;
    let (x, y) = (monitor.bounds.x as f32, monitor.bounds.y as f32);
    Screen {
        connector: None,
        positions: vec![Point::new(x / scale, y / scale), Point::new(x, y)],
    }
}
