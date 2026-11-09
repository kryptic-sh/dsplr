//! The GPU side of the window: crcbl-app's [`Presenter`] (device, swapchain,
//! adapter choice and frame pacing) and crcbl-frame's UI pass drawing a
//! frame's draw list into it, as pikr does.
//!
//! dsplr redraws only when something changed, so it keeps one frame in
//! flight: each frame has finished on the GPU before the next is recorded.

use crcbl_app::adapter::{PowerPreference, pin};
use crcbl_app::backend::GpuBackend;
use crcbl_app::present::{FrameOutcome, Pacing, PresentError, Presenter, PresenterDesc};
use crcbl_frame::graph::{ImportedImage, InitialClaim, RenderGraph};
use crcbl_frame::{TransientPool, UiRenderer};
use crcbl_hal::{CommandEncoderDesc, Features, ResourceState};
use crcbl_shell::{Shell, ShellEvent, WindowId};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::text::FontAtlas;

use crate::GuiError;

pub struct Gpu {
    presenter: Presenter,
    renderer: UiRenderer,
    pool: TransientPool,
}

impl Gpu {
    /// Opens a Vulkan device that presents to `window` at `extent`.
    ///
    /// The adapter is crcbl's rule: `CRCBL_ADAPTER` if set, else the GPU
    /// driving the window's display when it is discrete, else integrated
    /// before discrete; an adapter that cannot present the window is stepped
    /// past.
    ///
    /// [`Gpu::destroy`] must run before `window` is destroyed: [`crate::run`]
    /// owns both and destroys the `Gpu` first on every exit path.
    pub fn open(shell: &dyn Shell, window: WindowId, extent: (u32, u32)) -> Result<Self, GuiError> {
        let pinned = pin();
        let desc = PresenterDesc {
            adapter_pin: pinned.as_deref(),
            power_preference: PowerPreference::LowPower,
            // No present feedback or timing: the frame request already holds
            // presents back until the window system asks.
            optional_features: Features::empty(),
            ..window_desc()
        };
        let presenter =
            Presenter::open(shell, window, extent, &desc).map_err(|error| match error {
                PresentError::NoBackend(error) => GuiError::NoGpuBackend(error),
                error => GuiError::NoGpu(error),
            })?;
        Self::on(presenter)
    }

    /// Opens a presenter for another `window`, at `extent`, on this one's
    /// device: what a confirm dialog per monitor draws with, rather than a
    /// device each (see crcbl-app's `SharedGpu`). The same rule as
    /// [`Gpu::open`]: [`Gpu::destroy`] before `window` is destroyed.
    pub fn open_shared(
        &self,
        shell: &dyn Shell,
        window: WindowId,
        extent: (u32, u32),
    ) -> Result<Self, GuiError> {
        let presenter = self
            .presenter
            .shared()
            .open(shell, window, extent, &window_desc())
            .map_err(GuiError::NoGpu)?;
        Self::on(presenter)
    }

    fn on(presenter: Presenter) -> Result<Self, GuiError> {
        let renderer = UiRenderer::new(presenter.device(), presenter.queue(), presenter.format())?;
        Ok(Self {
            presenter,
            renderer,
            pool: TransientPool::new(),
        })
    }

    /// Follows a window resize. A zero extent (a minimised window) keeps the
    /// current swapchain.
    pub fn resize(&mut self, extent: (u32, u32)) -> Result<(), GuiError> {
        Ok(self.presenter.resize(extent)?)
    }

    /// Takes what `event` says about the frame request: its answer lets the
    /// next frame be presented.
    pub fn observe(&mut self, event: &ShellEvent) {
        self.presenter.observe(event);
    }

    /// Whether the window system has asked for the next frame, or none was
    /// presented yet.
    pub fn frame_due(&self) -> bool {
        self.presenter.frame_due()
    }

    /// Draws `list` and presents it, asking `shell` for the next frame.
    /// `false` when the swapchain was out of date and was reconfigured
    /// instead, so the caller draws again.
    pub fn present(
        &mut self,
        shell: &mut dyn Shell,
        list: &DrawList,
        atlas: &FontAtlas,
        background: [f32; 4],
    ) -> Result<bool, GuiError> {
        let Some(acquired) = self.presenter.acquire()? else {
            return Ok(false);
        };
        let device = self.presenter.device();
        self.renderer.begin_frame(device, list, atlas, 1.0)?;

        let compiled = {
            let mut graph = RenderGraph::new(self.presenter.queue());
            // An acquired image's contents are undefined, and it goes back to
            // the window system for presenting.
            let target = graph.import_image(
                "swapchain",
                ImportedImage {
                    image: acquired.image,
                    view: acquired.view,
                    format: self.presenter.format(),
                    extent: acquired.extent,
                    initial: ResourceState::Undefined,
                    claim: InitialClaim::Acquired,
                    final_state: ResourceState::Present,
                },
            );
            // The UI passes load what is there, so something must clear first.
            graph
                .add_render_pass("clear")
                .clear_color(target, background)
                .execute(|_| {});
            self.renderer
                .add_passes(&mut graph, target, acquired.extent);
            graph.compile(&self.pool)?
        };

        let mut encoder = device.create_command_encoder(&CommandEncoderDesc {
            label: Some("dsplr frame"),
            queue: self.presenter.queue(),
        });
        compiled.execute(device, &mut self.pool, encoder.as_mut(), None)?;
        let commands = encoder.finish()?;

        // Rides this present's commit; its answer lets the next one go.
        self.presenter.request_frame(shell)?;
        // With one frame in flight this returns after the frame has finished
        // on the GPU, so what it used can be reused or freed.
        let outcome = self.presenter.submit_and_present(&acquired, commands)?;
        self.pool.retire_unused(self.presenter.device());
        Ok(outcome == FrameOutcome::Presented)
    }

    /// Releases everything, in dependency order. Must run before the window
    /// is destroyed. A failure here is reported, since the process is on its
    /// way out and has nothing left to undo.
    pub fn destroy(mut self) {
        if let Err(error) = self.presenter.drain() {
            eprintln!("dsplr: waiting for the GPU at exit: {error}");
        }
        self.renderer.destroy(self.presenter.device());
        self.pool.destroy(self.presenter.device());
        if let Err(error) = self.presenter.destroy() {
            eprintln!("dsplr: releasing the GPU at exit: {error}");
        }
    }
}

/// What every window's presenter is opened with: dsplr redraws only when
/// something changed, so one frame in flight, paced by the display. dsplr is
/// Linux-only, where crcbl's backend is Vulkan.
fn window_desc() -> PresenterDesc<'static> {
    PresenterDesc {
        label: "dsplr",
        backend: Some(GpuBackend::Vulkan),
        pacing: Pacing::Vsync,
        frames_in_flight: 1,
        ..PresenterDesc::default()
    }
}
