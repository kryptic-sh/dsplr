//! One connection to a wlroots compositor: binding `zwlr_output_manager_v1`,
//! reading its heads, and sending configurations, over a [`Client`] that
//! owns the socket and routes each proxy's events here.

use core::ffi::CStr;

use crcbl_wayland::ffi::{WlArgument, WlProxy};
use dsplr_model::Layout;

use super::Error;
use super::plan::{HeadConfig, plan};
use crate::wayland::client::{Client, Protocol, State};
use crate::wayland::heads::{Head, HeadMode, SnapshotError, layout};
use crate::wayland::plan::ModeChoice;
use crate::wayland::protocol::wayland::wl_registry;
use crate::wayland::protocol::wlr_output_management::{
    zwlr_output_configuration_head_v1, zwlr_output_configuration_v1, zwlr_output_head_v1,
    zwlr_output_manager_v1, zwlr_output_mode_v1,
};

/// The newest `zwlr_output_manager_v1` dsplr speaks: the vendored XML's.
const MANAGER_VERSION: u32 = 4;

/// The first version whose heads and modes have a `release` request.
const RELEASE_SINCE: u32 = 3;

/// What a configuration is sent for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Test,
    Apply,
}

/// The compositor's answer to a configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// An open connection with `zwlr_output_manager_v1` bound and its heads
/// read up to a `done`.
///
/// It keeps the serial of the `done` it last dispatched, and configurations
/// are made against that. Nothing is read between calls, except in
/// [`Connection::changed`], so after the outputs change a configuration made
/// here is `cancelled`: read them again on a new connection, or follow them
/// with `changed`. It cannot leave the thread it was opened on.
#[derive(Debug)]
pub struct Connection {
    client: Client<Wlr>,
}

impl Connection {
    /// Connects to the compositor at `socket`, binds its output manager and
    /// reads every head up to the first `done`. On an error, dropping the
    /// client destroys what was made.
    pub async fn open(socket: &std::path::Path) -> Result<Self, Error> {
        let mut client = Client::open(socket, Wlr::default()).await?;
        let Some((name, offered)) = client.state().protocol.manager_global else {
            return Err(Error::NoManager);
        };
        let version = offered.min(MANAGER_VERSION);
        // SAFETY: the registry is live, and `name` is the global it announced
        // for this interface, bound at a version both sides speak.
        let manager = unsafe {
            wl_registry::bind(
                client.registry(),
                name,
                &zwlr_output_manager_v1::INTERFACE,
                version,
            )
        };
        let state = client.state_mut();
        state.protocol.version = version;
        state.protocol.manager = manager;
        state.track(manager, Kind::Manager)?;
        client
            .dispatch_until("describe its heads", |wlr| wlr.serial.is_some())
            .await?;
        Ok(Self { client })
    }

    /// The version the output manager was bound at.
    pub fn version(&self) -> u32 {
        self.client.state().protocol.version
    }

    /// Every head, as the events read so far describe it: complete up to the
    /// `done` that `open` waits for, and after a `test` or `apply` possibly
    /// partway through the changes it caused.
    pub fn heads(&self) -> Vec<Head> {
        self.client
            .state()
            .protocol
            .heads
            .iter()
            .filter(|slot| !slot.finished)
            .map(|slot| slot.head.clone())
            .collect()
    }

    /// The heads as a [`Layout`].
    pub fn layout(&self) -> Result<Layout, SnapshotError> {
        layout(&self.heads())
    }

    /// Asks the compositor whether it would apply `layout`, changing nothing.
    pub async fn test(&mut self, layout: &Layout) -> Result<(), Error> {
        self.configure(layout, Request::Test).await
    }

    /// Applies `layout`. `Ok` means the compositor reported it applied; it
    /// may have rounded a setting (a scale), so read the state back to see.
    pub async fn apply(&mut self, layout: &Layout) -> Result<(), Error> {
        self.configure(layout, Request::Apply).await
    }

    /// Waits, for as long as it takes, until the compositor reports that
    /// its outputs changed: a `done` after the one last dispatched. Its heads
    /// then describe the outputs as they are now. Cancel-safe, as
    /// [`Client::wait`] is.
    pub async fn changed(&mut self) -> Result<(), Error> {
        let seen = self.client.state().protocol.serial;
        Ok(self.client.wait(|wlr| wlr.serial != seen).await?)
    }

    /// Sends `layout` as one configuration, against the last `done`'s
    /// serial, and waits for the answer. Nothing is sent when the layout
    /// cannot be put into a configuration.
    async fn configure(&mut self, layout: &Layout, request: Request) -> Result<(), Error> {
        let state = self.client.state();
        state.check()?;
        let wlr = &state.protocol;
        let serial = wlr.serial.expect("`open` reads up to a `done`");
        let live: Vec<(usize, &HeadSlot)> = wlr
            .heads
            .iter()
            .enumerate()
            .filter(|(_, slot)| !slot.finished)
            .collect();
        let heads: Vec<Head> = live.iter().map(|(_, slot)| slot.head.clone()).collect();
        let configs = plan(layout, &heads, wlr.version)?;
        // Each listed mode resolved to its object before anything is sent.
        let targets: Vec<(*mut WlProxy, HeadConfig, Option<*mut WlProxy>)> = live
            .iter()
            .zip(configs)
            .map(|((index, slot), config)| {
                let mode = match &config {
                    HeadConfig::Enable(settings) => match settings.mode {
                        ModeChoice::Listed(handle) => Some(
                            state
                                .proxy_of(Kind::Mode {
                                    head: *index,
                                    handle,
                                })
                                .expect("`plan` chooses among the head's live modes"),
                        ),
                        ModeChoice::Custom { .. } => None,
                    },
                    HeadConfig::Disable | HeadConfig::Unchanged => None,
                };
                (slot.proxy, config, mode)
            })
            .collect();
        let manager = wlr.manager;

        // SAFETY: the manager is live: it is destroyed only by `Drop`, or by
        // its `finished` event, which `state.check` excludes above.
        let configuration =
            unsafe { zwlr_output_manager_v1::create_configuration(manager, serial) };
        let state = self.client.state_mut();
        state.protocol.outcome = None;
        state.track(configuration, Kind::Configuration)?;
        for (head, config, mode) in targets {
            // SAFETY: `configuration` is live and unsent, `head` is a live
            // head of this manager configured once (`plan` gives one config
            // per head), and `mode` is one of its live modes.
            unsafe { configure_head(state, configuration, head, &config, mode) };
        }
        // SAFETY: `configuration` is live, and every head is configured.
        unsafe {
            match request {
                Request::Test => zwlr_output_configuration_v1::test(configuration),
                Request::Apply => zwlr_output_configuration_v1::apply(configuration),
            }
        }

        let what = match request {
            Request::Test => "answer the test",
            Request::Apply => "answer the apply",
        };
        self.client
            .dispatch_until(what, |wlr| wlr.outcome.is_some())
            .await?;
        let state = self.client.state_mut();
        let outcome = state.protocol.outcome.take();
        destroy_configuration(state, configuration);
        match outcome {
            Some(Outcome::Succeeded) => Ok(()),
            Some(Outcome::Failed) => Err(Error::Failed),
            Some(Outcome::Cancelled) | None => Err(Error::Cancelled),
        }
    }
}

/// Sends the requests that set one head in `configuration`.
///
/// # Safety
///
/// `configuration` must be live and not yet applied or tested, `head` a live
/// head of the manager that made it and not yet configured in it, and
/// `mode` the live object of `config`'s listed mode, when it has one.
unsafe fn configure_head(
    state: &mut State<Wlr>,
    configuration: *mut WlProxy,
    head: *mut WlProxy,
    config: &HeadConfig,
    mode: Option<*mut WlProxy>,
) {
    let settings = match config {
        HeadConfig::Disable => {
            // SAFETY: the caller's.
            unsafe { zwlr_output_configuration_v1::disable_head(configuration, head) };
            return;
        }
        HeadConfig::Unchanged => None,
        HeadConfig::Enable(settings) => Some(settings),
    };
    // SAFETY: the caller's.
    let config_head = unsafe { zwlr_output_configuration_v1::enable_head(configuration, head) };
    // Head configurations have no events, so nothing to route.
    state.keep(config_head, Kind::ConfigurationHead);
    let Some(settings) = settings else {
        return;
    };
    // SAFETY (each request): `config_head` was just made, each setting is
    // sent once, and `mode` is live (the caller's).
    unsafe {
        use zwlr_output_configuration_head_v1 as h;
        match (settings.mode.clone(), mode) {
            (ModeChoice::Listed(_), Some(mode)) => h::set_mode(config_head, mode),
            (
                ModeChoice::Custom {
                    width,
                    height,
                    refresh_mhz,
                },
                _,
            ) => h::set_custom_mode(config_head, width, height, refresh_mhz),
            (ModeChoice::Listed(handle), None) => {
                unreachable!("mode {handle} was resolved to its object before sending")
            }
        }
        h::set_position(config_head, settings.position.0, settings.position.1);
        h::set_transform(config_head, settings.transform);
        h::set_scale(config_head, settings.scale);
        if let Some(adaptive_sync) = settings.adaptive_sync {
            h::set_adaptive_sync(config_head, adaptive_sync);
        }
    }
}

/// Destroys an answered configuration and its head configurations.
fn destroy_configuration(state: &mut State<Wlr>, configuration: *mut WlProxy) {
    for proxy in state.proxies(|kind| kind == Kind::ConfigurationHead) {
        state.destroy(proxy);
    }
    state.forget(configuration);
    // SAFETY: `configuration` is live, and `destroy` is its destructor.
    unsafe { zwlr_output_configuration_v1::destroy(configuration) };
}

/// What each of the protocol's proxies is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Manager,
    /// A head, by its index in [`Wlr::heads`].
    Head(usize),
    /// A mode, by its head's index and its ordinal among that head's modes.
    Mode {
        head: usize,
        handle: u64,
    },
    Configuration,
    ConfigurationHead,
}

/// One head and its proxy.
#[derive(Debug)]
struct HeadSlot {
    proxy: *mut WlProxy,
    head: Head,
    /// The compositor removed it; its proxy is released.
    finished: bool,
    /// The next mode object's ordinal among this head's modes.
    next_mode: u64,
}

/// What `zwlr_output_manager_v1`'s events say.
#[derive(Debug)]
struct Wlr {
    /// `zwlr_output_manager_v1`'s registry name and version, once announced.
    manager_global: Option<(u32, u32)>,
    manager: *mut WlProxy,
    /// The version the manager was bound at.
    version: u32,
    heads: Vec<HeadSlot>,
    /// The serial of the last `done`.
    serial: Option<u32>,
    /// The manager sent `finished`: the compositor is done with it.
    finished: bool,
    /// The answer to the configuration in flight.
    outcome: Option<Outcome>,
}

impl Default for Wlr {
    fn default() -> Self {
        Self {
            manager_global: None,
            manager: core::ptr::null_mut(),
            version: 0,
            heads: Vec::new(),
            serial: None,
            finished: false,
            outcome: None,
        }
    }
}

impl Wlr {
    fn mode_mut(&mut self, head: usize, handle: u64) -> Option<&mut HeadMode> {
        self.heads[head]
            .head
            .modes
            .iter_mut()
            .find(|mode| mode.handle == handle)
    }
}

impl Protocol for Wlr {
    type Kind = Kind;

    fn interface(kind: Kind) -> &'static str {
        match kind {
            Kind::Manager => "zwlr_output_manager_v1",
            Kind::Head(_) => "zwlr_output_head_v1",
            Kind::Mode { .. } => "zwlr_output_mode_v1",
            Kind::Configuration => "zwlr_output_configuration_v1",
            Kind::ConfigurationHead => "zwlr_output_configuration_head_v1",
        }
    }

    fn global(&mut self, name: u32, interface: &CStr, version: u32) {
        if interface == zwlr_output_manager_v1::NAME {
            self.manager_global = Some((name, version));
        }
    }

    fn finished(&self) -> Option<&'static str> {
        self.finished.then_some("zwlr_output_manager_v1")
    }

    unsafe fn event(
        state: &mut State<Self>,
        proxy: *mut WlProxy,
        kind: Kind,
        opcode: u32,
        args: &[WlArgument],
    ) {
        // SAFETY (each decoder): the caller's: `args` is `kind`'s interface's
        // array for `opcode`.
        match kind {
            Kind::Manager => match unsafe { zwlr_output_manager_v1::decode_event(opcode, args) } {
                Some(zwlr_output_manager_v1::Event::Head { head }) => {
                    let heads = &mut state.protocol.heads;
                    heads.push(HeadSlot {
                        proxy: head,
                        head: Head::default(),
                        finished: false,
                        next_mode: 0,
                    });
                    let index = heads.len() - 1;
                    state.track_or_lose(head, Kind::Head(index));
                }
                Some(zwlr_output_manager_v1::Event::Done { serial }) => {
                    state.protocol.serial = Some(serial);
                }
                Some(zwlr_output_manager_v1::Event::Finished) => {
                    state.protocol.finished = true;
                    // `finished` is the manager's destructor event.
                    state.destroy(proxy);
                }
                None => {}
            },
            Kind::Head(index) => {
                let event = unsafe { zwlr_output_head_v1::decode_event(opcode, args) };
                head_event(state, proxy, index, event);
            }
            Kind::Mode { head, handle } => {
                match unsafe { zwlr_output_mode_v1::decode_event(opcode, args) } {
                    Some(zwlr_output_mode_v1::Event::Finished) => {
                        let slot = &mut state.protocol.heads[head];
                        slot.head.modes.retain(|mode| mode.handle != handle);
                        if slot.head.current_mode == Some(handle) {
                            slot.head.current_mode = None;
                        }
                        release(state, proxy, kind);
                    }
                    Some(event) => {
                        if let Some(mode) = state.protocol.mode_mut(head, handle) {
                            match event {
                                zwlr_output_mode_v1::Event::Size { width, height } => {
                                    (mode.width, mode.height) = (width, height);
                                }
                                zwlr_output_mode_v1::Event::Refresh { refresh } => {
                                    mode.refresh_mhz = refresh;
                                }
                                zwlr_output_mode_v1::Event::Preferred => mode.preferred = true,
                                zwlr_output_mode_v1::Event::Finished => {}
                            }
                        }
                    }
                    None => {}
                }
            }
            Kind::Configuration => {
                let answer =
                    match unsafe { zwlr_output_configuration_v1::decode_event(opcode, args) } {
                        Some(zwlr_output_configuration_v1::Event::Succeeded) => {
                            Some(Outcome::Succeeded)
                        }
                        Some(zwlr_output_configuration_v1::Event::Failed) => Some(Outcome::Failed),
                        Some(zwlr_output_configuration_v1::Event::Cancelled) => {
                            Some(Outcome::Cancelled)
                        }
                        None => None,
                    };
                // The first answer is the configuration's. niri 26.04 sends
                // `cancelled` right after `succeeded` to a configuration whose
                // apply changed the outputs (its change notice cancels every
                // configuration the client still holds), so a later one is not.
                let outcome = &mut state.protocol.outcome;
                *outcome = outcome.or(answer);
            }
            // Head configurations have no events.
            Kind::ConfigurationHead => {}
        }
    }
}

/// Forgets `proxy`, a head or mode, and releases it: with the protocol's
/// `release` where the version has one, which also destroys it.
fn release(state: &mut State<Wlr>, proxy: *mut WlProxy, kind: Kind) {
    if state.protocol.version < RELEASE_SINCE {
        state.destroy(proxy);
        return;
    }
    state.forget(proxy);
    // SAFETY: `proxy` is a live head or mode, as `kind` says, at a version
    // with `release`, which destroys it.
    unsafe {
        match kind {
            Kind::Head(_) => zwlr_output_head_v1::release(proxy),
            _ => zwlr_output_mode_v1::release(proxy),
        }
    }
}

/// One event of the head at `index`.
fn head_event(
    state: &mut State<Wlr>,
    proxy: *mut WlProxy,
    index: usize,
    event: Option<zwlr_output_head_v1::Event<'_>>,
) {
    use zwlr_output_head_v1::Event;
    let text = |text: &CStr| text.to_string_lossy().into_owned();
    let Some(event) = event else {
        return;
    };
    match event {
        Event::Mode { mode } => {
            let slot = &mut state.protocol.heads[index];
            let handle = slot.next_mode;
            slot.next_mode += 1;
            slot.head.modes.push(HeadMode {
                handle,
                ..HeadMode::default()
            });
            state.track_or_lose(
                mode,
                Kind::Mode {
                    head: index,
                    handle,
                },
            );
        }
        Event::CurrentMode { mode } => {
            if let Some(Kind::Mode { handle, .. }) = state.kind_of(mode) {
                state.protocol.heads[index].head.current_mode = Some(handle);
            }
        }
        Event::Finished => {
            state.protocol.heads[index].finished = true;
            release(state, proxy, Kind::Head(index));
        }
        event => {
            let head = &mut state.protocol.heads[index].head;
            match event {
                Event::Name { name } => head.name = text(name),
                Event::Description { description } => head.description = text(description),
                Event::PhysicalSize { width, height } => head.physical_size = (width, height),
                Event::Enabled { enabled } => head.enabled = enabled != 0,
                Event::Position { x, y } => head.position = (x, y),
                Event::Transform { transform } => head.transform = transform,
                Event::Scale { scale } => head.scale = scale,
                Event::Make { make } => head.make = text(make),
                Event::Model { model } => head.model = text(model),
                Event::SerialNumber { serial_number } => head.serial_number = text(serial_number),
                Event::AdaptiveSync { state } => head.adaptive_sync = Some(state),
                Event::Mode { .. } | Event::CurrentMode { .. } | Event::Finished => {}
            }
        }
    }
}

/// The decoding the dispatcher does, on argument arrays built by hand as
/// libwayland hands them over: every event it reads with more than one
/// argument, and a string. The dispatcher itself needs a loaded libwayland, so
/// these are what Miri can interpret (`cargo +nightly miri test -p
/// dsplr-backend --lib wlr::connection`), as crcbl's own job does for its
/// decoders: a slot read outside the slice's provenance is reported there,
/// where a native run cannot tell it from a correct one.
#[cfg(test)]
mod tests {
    use crcbl_wayland::ffi::WlArgument;

    use super::{wl_registry, zwlr_output_head_v1, zwlr_output_mode_v1};
    use crate::wayland::tests::dispatched;

    #[test]
    fn a_registry_global_decodes_all_three_arguments() {
        let mut slots = [
            WlArgument { u: 7 },
            WlArgument {
                s: c"zwlr_output_manager_v1".as_ptr(),
            },
            WlArgument { u: 4 },
        ];
        let global = dispatched(c"usu", &mut slots, |args| {
            // SAFETY: `args` is a `wl_registry.global` array.
            match unsafe { wl_registry::decode_event(0, args) } {
                Some(wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                }) => Some((name, interface.to_owned(), version)),
                _ => None,
            }
        });
        assert_eq!(global, Some((7, c"zwlr_output_manager_v1".to_owned(), 4)));
    }

    #[test]
    fn head_and_mode_events_decode_their_second_argument() {
        let mut slots = [WlArgument { i: -1920 }, WlArgument { i: 360 }];
        let position = dispatched(c"ii", &mut slots, |args| {
            // SAFETY: `args` is a `zwlr_output_head_v1.position` array.
            match unsafe { zwlr_output_head_v1::decode_event(6, args) } {
                Some(zwlr_output_head_v1::Event::Position { x, y }) => Some((x, y)),
                _ => None,
            }
        });
        assert_eq!(position, Some((-1920, 360)));

        let mut slots = [WlArgument { i: 2560 }, WlArgument { i: 1440 }];
        let size = dispatched(c"ii", &mut slots, |args| {
            // SAFETY: `args` is a `zwlr_output_mode_v1.size` array.
            match unsafe { zwlr_output_mode_v1::decode_event(0, args) } {
                Some(zwlr_output_mode_v1::Event::Size { width, height }) => Some((width, height)),
                _ => None,
            }
        });
        assert_eq!(size, Some((2560, 1440)));
    }

    #[test]
    fn a_string_is_copied_out_inside_the_call() {
        let mut slots = [WlArgument {
            s: c"HDMI-A-1".as_ptr(),
        }];
        let name = dispatched(c"s", &mut slots, |args| {
            // SAFETY: `args` is a `zwlr_output_head_v1.name` array.
            match unsafe { zwlr_output_head_v1::decode_event(0, args) } {
                Some(zwlr_output_head_v1::Event::Name { name }) => {
                    Some(name.to_string_lossy().into_owned())
                }
                _ => None,
            }
        });
        assert_eq!(name.as_deref(), Some("HDMI-A-1"));
    }

    #[test]
    fn an_array_shorter_than_the_event_decodes_to_nothing() {
        let mut slots = [WlArgument { i: 2560 }];
        let size = dispatched(c"i", &mut slots, |args| {
            // SAFETY: a one-slot array, which the decoder must decline for
            // the two-argument `size`.
            unsafe { zwlr_output_mode_v1::decode_event(0, args) }.is_some()
        });
        assert!(!size);
    }
}
