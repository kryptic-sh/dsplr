//! The confirm dialogs (`docs/plan/06-gui.md`, "Confirm dialog"): "Keep
//! these display settings?" on every monitor, so it is on whichever still
//! shows, as a crcbl panel (layer-shell on Wayland, override-redirect on
//! X11) per monitor; a single toplevel where the window system has no
//! panels. Enter or Keep keeps; Esc, Revert or the countdown running out
//! reverts.

use std::time::{Duration, Instant};

use crcbl_shell::{
    ButtonState, MonitorInfo, PanelAnchor, PanelDesc, PanelKeyboard, PanelLayer, PointerButton,
    Shell, ShellCaps, ShellEvent, WindowDesc, WindowId, WindowRole,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use dsplr_core::dialog::{Choice, DIALOG_SIZE, DialogFrame, Screen, draw_dialog, match_screens};
use dsplr_core::{Point, Size, Theme};
use dsplr_model::Layout;

use crate::gpu::Gpu;
use crate::{GuiError, paint, wait_for_size};

/// The dialogs of one change, until it is answered.
pub struct Confirming {
    /// The confirm this answers.
    id: u64,
    deadline: Instant,
    /// When the change's watchdog reverts it, should nothing else.
    watchdog: Instant,
    dialogs: Vec<Dialog>,
}

struct Dialog {
    window: WindowId,
    gpu: Gpu,
    /// The output the monitor shows, when one matched.
    output: Option<String>,
    extent: (u32, u32),
    scale: f32,
    list: DrawList,
    /// What was last drawn, for its buttons.
    frame: Option<DialogFrame>,
}

impl Confirming {
    /// Opens a dialog on every monitor `shell` knows, each named for the
    /// output of `layout` it shows, drawing on `main`'s device, and counting
    /// down to `deadline`, the backend thread's own, for the confirm `id`.
    ///
    /// A dialog that cannot be opened (a monitor gone since the list was
    /// read, a presenter refused) is closed and skipped, and the rest go on;
    /// with none open, one toplevel dialog is tried; with that failing too,
    /// there is none, and the main window's bar asks. Returns the events
    /// pumped while the windows were sized, for the caller to handle, and a
    /// line for each dialog skipped.
    pub fn open(
        shell: &mut dyn Shell,
        main: &Gpu,
        id: u64,
        (deadline, watchdog): (Instant, Instant),
        layout: &Layout,
    ) -> (Self, Vec<ShellEvent>, Vec<String>) {
        let mut confirming = Self {
            id,
            deadline,
            watchdog,
            dialogs: Vec::new(),
        };
        let mut events = Vec::new();
        let mut skipped = Vec::new();
        let monitors: Vec<MonitorInfo> = shell.monitors().to_vec();
        if shell.caps().contains(ShellCaps::PANEL) && !monitors.is_empty() {
            let screens: Vec<Screen> = monitors.iter().map(screen).collect();
            let outputs = match_screens(&screens, layout);
            for (index, (monitor, output)) in monitors.iter().zip(outputs).enumerate() {
                let opened =
                    confirming.add(shell, main, Some((index, monitor)), output, &mut events);
                if let Err(error) = opened {
                    skipped.push(format!("no confirm dialog on {}: {error}", monitor.name));
                }
            }
        }
        if confirming.dialogs.is_empty()
            && let Err(error) = confirming.add(shell, main, None, None, &mut events)
        {
            skipped.push(format!(
                "no confirm dialog could be opened ({error}); answer here: Enter keeps, Esc reverts"
            ));
        }
        (confirming, events, skipped)
    }

    /// Opens one dialog: a panel on `monitor` (the first takes the keys), or
    /// a toplevel without one. A window made and then not drawable is
    /// destroyed again.
    fn add(
        &mut self,
        shell: &mut dyn Shell,
        main: &Gpu,
        monitor: Option<(usize, &MonitorInfo)>,
        output: Option<String>,
        events: &mut Vec<ShellEvent>,
    ) -> Result<(), GuiError> {
        let role = match monitor {
            Some((index, monitor)) => WindowRole::Panel(PanelDesc {
                namespace: "dsplr-confirm",
                monitor: Some(monitor.id),
                layer: PanelLayer::Overlay,
                // No edge: centred on its monitor.
                anchor: PanelAnchor::empty(),
                // The keys go to one; Enter and Esc work from any.
                keyboard: if index == 0 {
                    PanelKeyboard::Exclusive
                } else {
                    PanelKeyboard::OnDemand
                },
                ..PanelDesc::default()
            }),
            None => WindowRole::Toplevel,
        };
        let window = shell.create_window(&WindowDesc {
            title: "dsplr: keep these display settings?",
            app_id: "dsplr",
            size: crcbl_shell::LogicalSize::new(
                f64::from(DIALOG_SIZE.width),
                f64::from(DIALOG_SIZE.height),
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
                self.dialogs.push(Dialog {
                    window,
                    gpu,
                    output,
                    extent,
                    scale: 1.0,
                    list: DrawList::new(),
                    frame: None,
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
        self.dialogs.iter().any(|dialog| dialog.window == window)
    }

    /// Takes what `event` means to the dialogs: a size for one, a click on a
    /// button. Returns the answer a click gave.
    pub fn handle(&mut self, event: &ShellEvent) -> Result<Option<Choice>, GuiError> {
        for dialog in &mut self.dialogs {
            dialog.gpu.observe(event);
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
                if let Some(dialog) = self.dialog(window) {
                    dialog.extent = (size.width, size.height);
                    dialog.scale = scale_factor as f32;
                    dialog.gpu.resize(dialog.extent)?;
                }
                Ok(None)
            }
            ShellEvent::Button {
                window,
                button: PointerButton::Left,
                state: ButtonState::Pressed,
                position: Some(at),
                ..
            } => Ok(self.dialog(window).and_then(|dialog| {
                let point = Point::new(at.x as f32 / dialog.scale, at.y as f32 / dialog.scale);
                dialog.frame.as_ref()?.hit(point)
            })),
            _ => Ok(None),
        }
    }

    fn dialog(&mut self, window: WindowId) -> Option<&mut Dialog> {
        self.dialogs
            .iter_mut()
            .find(|dialog| dialog.window == window)
    }

    /// Whether the countdown has run out.
    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// How long until the countdown shows another second.
    pub fn next_tick(&self) -> Duration {
        let left = self.deadline.saturating_duration_since(Instant::now());
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
        let left = self.deadline.saturating_duration_since(Instant::now());
        for dialog in &mut self.dialogs {
            if !dialog.gpu.frame_due() {
                continue;
            }
            let size = Size {
                width: dialog.extent.0 as f32 / dialog.scale,
                height: dialog.extent.1 as f32 / dialog.scale,
            };
            let frame = draw_dialog(dialog.output.as_deref(), left, size, theme);
            dialog.list.clear();
            dialog.list.set_scale(dialog.scale);
            paint::paint(&frame.list, &mut dialog.list, font);
            dialog.frame = Some(frame);
            let background = paint::linear(theme.surface);
            // An out-of-date swapchain was reconfigured; the next tick draws.
            dialog.gpu.present(shell, &dialog.list, atlas, background)?;
        }
        Ok(())
    }

    /// Closes every dialog, each surface before its window.
    pub fn close(self, shell: &mut dyn Shell) -> Result<(), GuiError> {
        for dialog in self.dialogs {
            dialog.gpu.destroy();
            shell.destroy_window(dialog.window)?;
        }
        Ok(())
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
