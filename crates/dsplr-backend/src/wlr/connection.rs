//! One connection to the compositor: binding `zwlr_output_manager_v1`,
//! reading its heads, and sending configurations. Everything that touches
//! libwayland is in this file.
//!
//! The connection is driven from tokio: the socket is watched with
//! [`AsyncFd`], and every wait for the compositor has [`REQUEST_TIMEOUT`],
//! so a compositor that never answers is an error rather than a hang.
//! Events reach [`dispatch`] only inside `wl_display_dispatch_pending`,
//! which only [`Connection::read_once`] calls, and it takes `&mut self`: the
//! borrow checker keeps every other reference to the [`State`] the
//! dispatcher writes from being alive while it runs.

use core::ffi::{c_int, c_void};
use core::ptr;
use std::io::{self, ErrorKind};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crcbl_wayland::Display;
use crcbl_wayland::ffi::{self, Lib, WlArgument, WlMessage, WlProxy};
use dsplr_model::Layout;
use tokio::io::unix::AsyncFd;

use super::Error;
use super::heads::{Head, HeadMode, SnapshotError, layout};
use super::plan::{HeadConfig, ModeChoice, plan};
use super::protocol::wayland::{wl_callback, wl_display, wl_registry};
use super::protocol::wlr_output_management::{
    zwlr_output_configuration_head_v1, zwlr_output_configuration_v1, zwlr_output_head_v1,
    zwlr_output_manager_v1, zwlr_output_mode_v1,
};

/// The newest `zwlr_output_manager_v1` dsplr speaks: the vendored XML's.
const MANAGER_VERSION: u32 = 4;

/// The first version whose heads and modes have a `release` request.
const RELEASE_SINCE: u32 = 3;

/// How long the compositor gets to answer anything dsplr waits for. The
/// same as Hyprland's request timeout.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

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
/// are made against that. Nothing is read between calls, so after the
/// outputs change a configuration made here is `cancelled`: read them again
/// on a new connection. It cannot leave the thread it was opened on.
#[derive(Debug)]
pub struct Connection {
    /// Dropped after [`State`] is freed and its proxies destroyed (see
    /// `Drop`): deregistered from tokio, then disconnected.
    display: AsyncFd<Display>,
    /// From `Box::into_raw` in [`State::boxed`]; freed in `Drop`.
    state: *mut State,
    path: PathBuf,
}

impl Connection {
    /// Connects to the compositor at `socket`, binds its output manager and
    /// reads every head up to the first `done`.
    pub async fn open(socket: &Path) -> Result<Self, Error> {
        let io_error = |what, source| Error::Io {
            what,
            path: socket.to_owned(),
            source,
        };
        let stream = UnixStream::connect(socket).map_err(|e| io_error("connecting to", e))?;
        let display = Display::connect_to_fd(OwnedFd::from(stream)).map_err(Error::Connect)?;
        // SAFETY: the descriptor is the display's socket, which libwayland
        // keeps open, the same one, until `wl_display_disconnect` in
        // `Display`'s `Drop`, which runs only when the `AsyncFd` drops it.
        let display = unsafe { AsyncFd::register(display) }
            .map_err(|error| io_error("watching", error.into_parts().1))?;
        let mut connection = Self {
            display,
            state: State::boxed(ffi::lib()),
            path: socket.to_owned(),
        };
        connection.bind().await?;
        Ok(connection)
    }

    /// Finds and binds the output manager and reads its first `done`. On an
    /// error, `Drop` destroys what was made.
    async fn bind(&mut self) -> Result<(), Error> {
        let display = self.display.get_ref().as_proxy();
        // SAFETY: the display is live, and `get_registry` takes it as the
        // proxy it is.
        let registry = unsafe { wl_display::get_registry(display) };
        self.state_mut().track(registry, Kind::Registry)?;
        self.roundtrip("list its globals").await?;

        let Some((name, offered)) = self.state().manager_global else {
            return Err(Error::NoManager);
        };
        let version = offered.min(MANAGER_VERSION);
        // SAFETY: the registry is live, and `name` is the global it announced
        // for this interface, bound at a version both sides speak.
        let manager = unsafe {
            wl_registry::bind(registry, name, &zwlr_output_manager_v1::INTERFACE, version)
        };
        let state = self.state_mut();
        state.version = version;
        state.manager = manager;
        state.track(manager, Kind::Manager)?;
        self.dispatch_until("describe its heads", |state| state.serial.is_some())
            .await
    }

    /// The version the output manager was bound at.
    pub fn version(&self) -> u32 {
        self.state().version
    }

    /// Every head, as the events read so far describe it: complete up to the
    /// `done` that `open` waits for, and after a `test` or `apply` possibly
    /// partway through the changes it caused.
    pub fn heads(&self) -> Vec<Head> {
        self.state()
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

    /// Sends `layout` as one configuration, against the last `done`'s
    /// serial, and waits for the answer. Nothing is sent when the layout
    /// cannot be put into a configuration.
    async fn configure(&mut self, layout: &Layout, request: Request) -> Result<(), Error> {
        let state = self.state();
        state.check()?;
        let serial = state.serial.expect("`open` reads up to a `done`");
        let live: Vec<(usize, &HeadSlot)> = state
            .heads
            .iter()
            .enumerate()
            .filter(|(_, slot)| !slot.finished)
            .collect();
        let heads: Vec<Head> = live.iter().map(|(_, slot)| slot.head.clone()).collect();
        let configs = plan(layout, &heads, state.version)?;
        // Each listed mode resolved to its object before anything is sent.
        let targets: Vec<(*mut WlProxy, HeadConfig, Option<*mut WlProxy>)> = live
            .iter()
            .zip(configs)
            .map(|((index, slot), config)| {
                let mode = match &config {
                    HeadConfig::Enable(settings) => match settings.mode {
                        ModeChoice::Listed(handle) => Some(
                            state
                                .mode_proxy(*index, handle)
                                .expect("`plan` chooses among the head's live modes"),
                        ),
                        ModeChoice::Custom { .. } => None,
                    },
                    HeadConfig::Disable | HeadConfig::Unchanged => None,
                };
                (slot.proxy, config, mode)
            })
            .collect();
        let manager = state.manager;

        // SAFETY: the manager is live: it is destroyed only by `Drop`, or by
        // its `finished` event, which `state.finished` excludes above.
        let configuration =
            unsafe { zwlr_output_manager_v1::create_configuration(manager, serial) };
        let state = self.state_mut();
        state.outcome = None;
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
        self.dispatch_until(what, |state| state.outcome.is_some())
            .await?;
        let state = self.state_mut();
        let outcome = state.outcome.take();
        state.destroy_configuration(configuration);
        match outcome {
            Some(Outcome::Succeeded) => Ok(()),
            Some(Outcome::Failed) => Err(Error::Failed),
            Some(Outcome::Cancelled) | None => Err(Error::Cancelled),
        }
    }

    /// Sends a `wl_display.sync` and dispatches until it is answered, so
    /// everything sent before it has been answered too.
    async fn roundtrip(&mut self, what: &'static str) -> Result<(), Error> {
        let display = self.display.get_ref().as_proxy();
        // SAFETY: the display is live.
        let callback = unsafe { wl_display::sync(display) };
        let state = self.state_mut();
        let target = state.synced + 1;
        state.track(callback, Kind::Callback)?;
        self.dispatch_until(what, |state| state.synced >= target)
            .await
    }

    /// Reads and dispatches until `done` holds of the state, the manager is
    /// finished, an object's events cannot be routed, or [`REQUEST_TIMEOUT`]
    /// passes.
    async fn dispatch_until(
        &mut self,
        what: &'static str,
        done: impl Fn(&State) -> bool,
    ) -> Result<(), Error> {
        let wait = async {
            loop {
                let state = self.state();
                if done(state) {
                    return Ok(());
                }
                state.check()?;
                self.read_once().await?;
            }
        };
        tokio::time::timeout(REQUEST_TIMEOUT, wait)
            .await
            .map_err(|_| Error::Timeout { what })?
    }

    /// One turn of libwayland's read cycle (`crcbl_wayland::display`'s docs):
    /// dispatch what is queued, or flush, wait for the socket, read, and
    /// dispatch what came. The only caller of `dispatch_pending`.
    async fn read_once(&mut self) -> Result<(), Error> {
        let Self { display, path, .. } = self;
        let Some(read) = display.get_ref().prepare_read() else {
            display
                .get_ref()
                .dispatch_pending()
                .map_err(Error::Disconnected)?;
            return Ok(());
        };
        flush(display, path).await?;
        let mut ready = display
            .readable()
            .await
            .map_err(|source| io_error(path, "waiting on", source))?;
        read.read().map_err(Error::Disconnected)?;
        // The readiness is edge-triggered, so it may be cleared only once the
        // socket is drained; libwayland's source (1.22 and 1.26) says when.
        // `dispatch_queue` counts the display queue's events (`delete_id`,
        // `error`) as well as the default queue's. `wl_connection_read` reads
        // with MSG_DONTWAIT, so an empty socket reads nothing without
        // blocking; 1.26 reads until EAGAIN unless its buffer is at its
        // maximum, 1.22 reads once into the buffer's free space. Either way
        // bytes are left only when the buffer filled, and a full buffer
        // holds at least one whole event, since an event is never larger
        // than the buffer and the buffer starts at an event. libwayland drops
        // without counting only events for proxies dsplr destroyed, and none
        // of those gets more: a callback after `done`, a configuration after
        // its answer, a head or mode after `finished`. So a read that
        // dispatched nothing left nothing behind.
        if display
            .get_ref()
            .dispatch_pending()
            .map_err(Error::Disconnected)?
            == 0
        {
            ready.clear_ready();
        }
        Ok(())
    }

    fn state(&self) -> &State {
        // SAFETY: `state` is live until `Drop`, and the dispatcher, its only
        // other user, runs only inside `dispatch_pending`, which only
        // `read_once(&mut self)` calls: the borrow checker keeps this `&`
        // from living across one.
        unsafe { &*self.state }
    }

    fn state_mut(&mut self) -> &mut State {
        // SAFETY: as for `state`, and `&mut self` makes this the only
        // reference.
        unsafe { &mut *self.state }
    }
}

/// Sends every queued request, waiting while the socket is full. Writing
/// dispatches nothing.
async fn flush(display: &AsyncFd<Display>, path: &Path) -> Result<(), Error> {
    loop {
        match display.get_ref().flush() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                display
                    .writable()
                    .await
                    .map_err(|source| io_error(path, "waiting on", source))?
                    .clear_ready();
            }
            Err(source) => return Err(io_error(path, "writing to", source)),
        }
    }
}

fn io_error(path: &Path, what: &'static str, source: io::Error) -> Error {
    Error::Io {
        what,
        path: path.to_owned(),
        source,
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // SAFETY: `state` came from `Box::into_raw` and is freed only here.
        // No dispatch runs during `drop`, and every proxy that could
        // dispatch into it is destroyed before it is freed.
        let state = unsafe { Box::from_raw(self.state) };
        for (proxy, _) in state.objects.iter().rev() {
            // SAFETY: each proxy is live and destroyed exactly once, here;
            // the compositor's objects go with the connection, which closes
            // when `display` drops after this.
            unsafe { (state.lib.proxy_destroy)(*proxy) };
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
    state: &mut State,
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
    state.objects.push((config_head, Kind::ConfigurationHead));
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

/// What each proxy the dispatcher was attached to is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Registry,
    Callback,
    Manager,
    /// A head, by its index in [`State::heads`].
    Head(usize),
    /// A mode, by its head's index and its ordinal among that head's modes.
    Mode {
        head: usize,
        handle: u64,
    },
    Configuration,
    ConfigurationHead,
}

impl Kind {
    /// The interface of a proxy of this kind.
    const fn interface(self) -> &'static str {
        match self {
            Self::Registry => "wl_registry",
            Self::Callback => "wl_callback",
            Self::Manager => "zwlr_output_manager_v1",
            Self::Head(_) => "zwlr_output_head_v1",
            Self::Mode { .. } => "zwlr_output_mode_v1",
            Self::Configuration => "zwlr_output_configuration_v1",
            Self::ConfigurationHead => "zwlr_output_configuration_head_v1",
        }
    }
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

/// Everything the dispatcher writes, reached only through the raw pointer
/// [`State::boxed`] returns.
#[derive(Debug)]
struct State {
    lib: &'static Lib,
    /// This allocation's address, which every dispatcher is handed.
    root: *mut Self,
    /// Every live proxy and what it is.
    objects: Vec<(*mut WlProxy, Kind)>,
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
    /// How many `wl_display.sync` callbacks have been answered.
    synced: u32,
    /// The answer to the configuration in flight.
    outcome: Option<Outcome>,
    /// An object, made inside the dispatcher, whose events libwayland would
    /// not route here: the state can no longer follow the compositor.
    lost: Option<&'static str>,
}

impl State {
    fn boxed(lib: &'static Lib) -> *mut Self {
        let raw = Box::into_raw(Box::new(Self {
            lib,
            root: ptr::null_mut(),
            objects: Vec::new(),
            manager_global: None,
            manager: ptr::null_mut(),
            version: 0,
            heads: Vec::new(),
            serial: None,
            finished: false,
            synced: 0,
            outcome: None,
            lost: None,
        }));
        // SAFETY: `raw` was just returned by `Box::into_raw`; nothing else
        // refers to it yet.
        unsafe { (*raw).root = raw };
        raw
    }

    fn kind_of(&self, proxy: *mut WlProxy) -> Option<Kind> {
        self.objects
            .iter()
            .find(|(candidate, _)| *candidate == proxy)
            .map(|(_, kind)| *kind)
    }

    /// The live object of the mode `handle` of the head at `index`.
    fn mode_proxy(&self, index: usize, handle: u64) -> Option<*mut WlProxy> {
        let kind = Kind::Mode {
            head: index,
            handle,
        };
        self.objects
            .iter()
            .find(|(_, candidate)| *candidate == kind)
            .map(|(proxy, _)| *proxy)
    }

    /// Whether the state can still be trusted to follow the compositor.
    fn check(&self) -> Result<(), Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        self.lost
            .map_or(Ok(()), |what| Err(Error::Dispatcher(what)))
    }

    /// Records `proxy` and routes its events to [`dispatch`]. On an error
    /// the proxy is still recorded, so `Drop` destroys it.
    fn track(&mut self, proxy: *mut WlProxy, kind: Kind) -> Result<(), Error> {
        if proxy.is_null() {
            return Err(Error::Dispatcher(kind.interface()));
        }
        self.objects.push((proxy, kind));
        // SAFETY: `proxy` is live and has no dispatcher yet, `dispatch` has
        // libwayland's dispatcher signature, and `root` is the allocation's
        // own pointer, live until every proxy is destroyed.
        let added = unsafe {
            (self.lib.proxy_add_dispatcher)(proxy, dispatch, self.root.cast(), ptr::null_mut())
        };
        if added == 0 {
            Ok(())
        } else {
            Err(Error::Dispatcher(kind.interface()))
        }
    }

    /// [`State::track`] from inside the dispatcher, which cannot return an
    /// error: a failure is kept for [`State::check`].
    fn track_or_lose(&mut self, proxy: *mut WlProxy, kind: Kind) {
        if self.track(proxy, kind).is_err() {
            self.lost.get_or_insert_with(|| kind.interface());
        }
    }

    /// Forgets `proxy` and destroys it client-side.
    fn destroy(&mut self, proxy: *mut WlProxy) {
        self.objects.retain(|(candidate, _)| *candidate != proxy);
        // SAFETY: `proxy` was live and tracked, and is no longer.
        unsafe { (self.lib.proxy_destroy)(proxy) };
    }

    /// Forgets `proxy`, a head or mode, and releases it: with the
    /// protocol's `release` where the version has one, which also destroys
    /// it.
    fn release(&mut self, proxy: *mut WlProxy, kind: Kind) {
        if self.version < RELEASE_SINCE {
            self.destroy(proxy);
            return;
        }
        self.objects.retain(|(candidate, _)| *candidate != proxy);
        // SAFETY: `proxy` is a live head or mode, as `kind` says, at a
        // version with `release`, which destroys it.
        unsafe {
            match kind {
                Kind::Head(_) => zwlr_output_head_v1::release(proxy),
                _ => zwlr_output_mode_v1::release(proxy),
            }
        }
    }

    /// Destroys an answered configuration and its head configurations.
    fn destroy_configuration(&mut self, configuration: *mut WlProxy) {
        let heads: Vec<*mut WlProxy> = self
            .objects
            .iter()
            .filter(|(_, kind)| *kind == Kind::ConfigurationHead)
            .map(|(proxy, _)| *proxy)
            .collect();
        for proxy in heads {
            self.destroy(proxy);
        }
        self.objects
            .retain(|(candidate, _)| *candidate != configuration);
        // SAFETY: `configuration` is live, and `destroy` is its destructor.
        unsafe { zwlr_output_configuration_v1::destroy(configuration) };
    }

    fn mode_mut(&mut self, head: usize, handle: u64) -> Option<&mut HeadMode> {
        self.heads[head]
            .head
            .modes
            .iter_mut()
            .find(|mode| mode.handle == handle)
    }
}

/// libwayland's dispatcher for every object a [`Connection`] owns.
unsafe extern "C" fn dispatch(
    user_data: *const c_void,
    target: *mut c_void,
    opcode: u32,
    _message: *const WlMessage,
    args: *mut WlArgument,
) -> c_int {
    // SAFETY: `user_data` is `State::root`, live while any proxy is, and no
    // other reference to the state is held during a dispatch (see the
    // module's docs).
    let state = unsafe { &mut *user_data.cast::<State>().cast_mut() };
    let proxy: *mut WlProxy = target.cast();
    let Some(kind) = state.kind_of(proxy) else {
        return 0;
    };
    // SAFETY: libwayland passes the closure's argument array, live for this
    // call.
    let args = unsafe { &*args.cast_const() };
    // SAFETY (each decoder): `kind` is the interface the proxy was created
    // as, so `args` is that interface's array for `opcode`, and every borrow
    // is copied out before returning.
    match kind {
        Kind::Registry => {
            if let Some(wl_registry::Event::Global {
                name,
                interface,
                version,
            }) = unsafe { wl_registry::decode_event(opcode, args) }
                && interface == zwlr_output_manager_v1::NAME
            {
                state.manager_global = Some((name, version));
            }
        }
        Kind::Callback => {
            if let Some(wl_callback::Event::Done { .. }) =
                unsafe { wl_callback::decode_event(opcode, args) }
            {
                state.synced += 1;
                // `done` is the callback's destructor event.
                state.destroy(proxy);
            }
        }
        Kind::Manager => match unsafe { zwlr_output_manager_v1::decode_event(opcode, args) } {
            Some(zwlr_output_manager_v1::Event::Head { head }) => {
                state.heads.push(HeadSlot {
                    proxy: head,
                    head: Head::default(),
                    finished: false,
                    next_mode: 0,
                });
                state.track_or_lose(head, Kind::Head(state.heads.len() - 1));
            }
            Some(zwlr_output_manager_v1::Event::Done { serial }) => state.serial = Some(serial),
            Some(zwlr_output_manager_v1::Event::Finished) => {
                state.finished = true;
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
                    let modes = &mut state.heads[head].head.modes;
                    modes.retain(|mode| mode.handle != handle);
                    if state.heads[head].head.current_mode == Some(handle) {
                        state.heads[head].head.current_mode = None;
                    }
                    state.release(proxy, kind);
                }
                Some(event) => {
                    if let Some(mode) = state.mode_mut(head, handle) {
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
            let answer = match unsafe { zwlr_output_configuration_v1::decode_event(opcode, args) } {
                Some(zwlr_output_configuration_v1::Event::Succeeded) => Some(Outcome::Succeeded),
                Some(zwlr_output_configuration_v1::Event::Failed) => Some(Outcome::Failed),
                Some(zwlr_output_configuration_v1::Event::Cancelled) => Some(Outcome::Cancelled),
                None => None,
            };
            // The first answer is the configuration's. niri 26.04 sends
            // `cancelled` right after `succeeded` to a configuration whose
            // apply changed the outputs (its change notice cancels every
            // configuration the client still holds), so a later one is not.
            state.outcome = state.outcome.or(answer);
        }
        // Head configurations have no events.
        Kind::ConfigurationHead => {}
    }
    0
}

/// One event of the head at `index`.
fn head_event(
    state: &mut State,
    proxy: *mut WlProxy,
    index: usize,
    event: Option<zwlr_output_head_v1::Event<'_>>,
) {
    use zwlr_output_head_v1::Event;
    let text = |text: &core::ffi::CStr| text.to_string_lossy().into_owned();
    let Some(event) = event else {
        return;
    };
    match event {
        Event::Mode { mode } => {
            let slot = &mut state.heads[index];
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
                state.heads[index].head.current_mode = Some(handle);
            }
        }
        Event::Finished => {
            state.heads[index].finished = true;
            state.release(proxy, Kind::Head(index));
        }
        event => {
            let head = &mut state.heads[index].head;
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
