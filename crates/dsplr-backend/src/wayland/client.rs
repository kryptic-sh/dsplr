//! One connection to the compositor, driven from tokio, for a protocol whose
//! events a [`Protocol`] handles: connecting, listing the globals, tracking
//! every proxy and routing its events, and waiting for answers.
//!
//! The socket is watched with [`AsyncFd`], and every wait for the compositor
//! but [`Client::wait`] has [`REQUEST_TIMEOUT`], so a compositor that never
//! answers is an error rather than a hang. Events reach [`dispatch`] only
//! inside `wl_display_dispatch_pending`, which only [`Client::read_once`]
//! calls, and it takes `&mut self`: the borrow checker keeps every other
//! reference to the [`State`] the dispatcher writes from being alive while
//! it runs.

use core::ffi::{CStr, c_int, c_void};
use core::fmt;
use core::ptr;
use std::io::{self, ErrorKind};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crcbl_wayland::Display;
use crcbl_wayland::ffi::{self, Lib, WlArgument, WlMessage, WlProxy};
use tokio::io::unix::AsyncFd;

use super::protocol::wayland::{wl_callback, wl_display, wl_registry};
use super::{Error, REQUEST_TIMEOUT};

/// A protocol a [`Client`] speaks: what its objects are and what each of
/// their events does to its data.
pub trait Protocol: Sized {
    /// What one of the protocol's proxies is.
    type Kind: Copy + PartialEq + fmt::Debug;

    /// The interface of a proxy of `kind`, for errors.
    fn interface(kind: Self::Kind) -> &'static str;

    /// The registry announced a global.
    fn global(&mut self, name: u32, interface: &CStr, version: u32);

    /// The interface of the object the outputs are followed through, once
    /// the compositor has withdrawn it: the data can no longer follow them.
    fn finished(&self) -> Option<&'static str>;

    /// One event of `proxy`, a proxy of `kind`.
    ///
    /// # Safety
    ///
    /// `args` must be the argument array of event `opcode` of the interface
    /// `kind` is ([`Protocol::interface`]), as libwayland hands it to a
    /// dispatcher, and must not be kept past the call.
    unsafe fn event(
        state: &mut State<Self>,
        proxy: *mut WlProxy,
        kind: Self::Kind,
        opcode: u32,
        args: &[WlArgument],
    );
}

/// What a proxy the dispatcher was attached to is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object<K> {
    Registry,
    Callback,
    Of(K),
}

/// An open connection with its registry's globals listed. It cannot leave
/// the thread it was opened on.
#[derive(Debug)]
pub struct Client<P: Protocol + fmt::Debug> {
    /// Dropped after [`State`] is freed and its proxies destroyed (see
    /// `Drop`): deregistered from tokio, then disconnected.
    display: AsyncFd<Display>,
    /// From `Box::into_raw` in [`State::boxed`]; freed in `Drop`.
    state: *mut State<P>,
    path: PathBuf,
}

impl<P: Protocol + fmt::Debug> Client<P> {
    /// Connects to the compositor at `socket` and lists its globals into
    /// `protocol`.
    pub async fn open(socket: &Path, protocol: P) -> Result<Self, Error> {
        let stream =
            UnixStream::connect(socket).map_err(|e| io_error(socket, "connecting to", e))?;
        let display = Display::connect_to_fd(OwnedFd::from(stream)).map_err(Error::Connect)?;
        // SAFETY: the descriptor is the display's socket, which libwayland
        // keeps open, the same one, until `wl_display_disconnect` in
        // `Display`'s `Drop`, which runs only when the `AsyncFd` drops it.
        let display = unsafe { AsyncFd::register(display) }
            .map_err(|error| io_error(socket, "watching", error.into_parts().1))?;
        let mut client = Self {
            display,
            state: State::boxed(ffi::lib(), protocol),
            path: socket.to_owned(),
        };
        let display = client.display.get_ref().as_proxy();
        // SAFETY: the display is live, and `get_registry` takes it as the
        // proxy it is.
        let registry = unsafe { wl_display::get_registry(display) };
        client.state_mut().attach(registry, Object::Registry)?;
        client.roundtrip("list its globals").await?;
        Ok(client)
    }

    /// The registry, to bind the globals it listed with.
    pub fn registry(&self) -> *mut WlProxy {
        self.state()
            .objects
            .iter()
            .find(|(_, object)| *object == Object::Registry)
            .map(|(proxy, _)| *proxy)
            .expect("`open` attaches the registry, and nothing destroys it before `Drop`")
    }

    /// Sends a `wl_display.sync` and dispatches until it is answered, so
    /// everything sent before it has been answered too.
    pub async fn roundtrip(&mut self, what: &'static str) -> Result<(), Error> {
        let display = self.display.get_ref().as_proxy();
        // SAFETY: the display is live.
        let callback = unsafe { wl_display::sync(display) };
        let state = self.state_mut();
        let target = state.synced + 1;
        state.attach(callback, Object::Callback)?;
        self.until(what, |state| state.synced >= target).await
    }

    /// Reads and dispatches until `done` holds of the protocol's data, the
    /// compositor withdraws what it is followed through, an object's events
    /// cannot be routed, or [`REQUEST_TIMEOUT`] passes.
    pub async fn dispatch_until(
        &mut self,
        what: &'static str,
        done: impl Fn(&P) -> bool,
    ) -> Result<(), Error> {
        self.until(what, |state| done(&state.protocol)).await
    }

    /// [`Client::dispatch_until`] for as long as it takes. Cancel-safe:
    /// dropped while waiting, it has dispatched every event it read, and the
    /// read it began is cancelled ([`crcbl_wayland`]'s `ReadGuard`).
    pub async fn wait(&mut self, done: impl Fn(&P) -> bool) -> Result<(), Error> {
        loop {
            let state = self.state();
            if done(&state.protocol) {
                return Ok(());
            }
            state.check()?;
            self.read_once().await?;
        }
    }

    async fn until(
        &mut self,
        what: &'static str,
        done: impl Fn(&State<P>) -> bool,
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
        // without counting only events for proxies dsplr destroyed, and every
        // protocol here destroys a proxy only once the compositor sends it
        // nothing more: a callback after `done`, a configuration after its
        // answer, an output or mode once the compositor said it is gone. So a
        // read that dispatched nothing left nothing behind.
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

    pub fn state(&self) -> &State<P> {
        // SAFETY: `state` is live until `Drop`, and the dispatcher, its only
        // other user, runs only inside `dispatch_pending`, which only
        // `read_once(&mut self)` calls: the borrow checker keeps this `&`
        // from living across one.
        unsafe { &*self.state }
    }

    pub fn state_mut(&mut self) -> &mut State<P> {
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

impl<P: Protocol + fmt::Debug> Drop for Client<P> {
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

/// Everything the dispatcher writes, reached only through the raw pointer
/// [`State::boxed`] returns: the proxies and what each is, and the
/// protocol's own data.
#[derive(Debug)]
pub struct State<P: Protocol> {
    lib: &'static Lib,
    /// This allocation's address, which every dispatcher is handed.
    root: *mut Self,
    /// Every live proxy and what it is.
    objects: Vec<(*mut WlProxy, Object<P::Kind>)>,
    /// How many `wl_display.sync` callbacks have been answered.
    synced: u32,
    /// An object, made inside the dispatcher, whose events libwayland would
    /// not route here: the state can no longer follow the compositor.
    lost: Option<&'static str>,
    pub protocol: P,
}

impl<P: Protocol> State<P> {
    fn boxed(lib: &'static Lib, protocol: P) -> *mut Self {
        let raw = Box::into_raw(Box::new(Self {
            lib,
            root: ptr::null_mut(),
            objects: Vec::new(),
            synced: 0,
            lost: None,
            protocol,
        }));
        // SAFETY: `raw` was just returned by `Box::into_raw`; nothing else
        // refers to it yet.
        unsafe { (*raw).root = raw };
        raw
    }

    fn object_of(&self, proxy: *mut WlProxy) -> Option<Object<P::Kind>> {
        self.objects
            .iter()
            .find(|(candidate, _)| *candidate == proxy)
            .map(|(_, object)| *object)
    }

    /// What `proxy` is, when it is one of the protocol's live proxies.
    pub fn kind_of(&self, proxy: *mut WlProxy) -> Option<P::Kind> {
        match self.object_of(proxy) {
            Some(Object::Of(kind)) => Some(kind),
            _ => None,
        }
    }

    /// The live proxy of `kind`, the first when there are several.
    pub fn proxy_of(&self, kind: P::Kind) -> Option<*mut WlProxy> {
        self.objects
            .iter()
            .find(|(_, object)| *object == Object::Of(kind))
            .map(|(proxy, _)| *proxy)
    }

    /// Every live proxy whose kind `matching` takes, oldest first.
    pub fn proxies(&self, matching: impl Fn(P::Kind) -> bool) -> Vec<*mut WlProxy> {
        self.objects
            .iter()
            .filter(|(_, object)| matches!(object, Object::Of(kind) if matching(*kind)))
            .map(|(proxy, _)| *proxy)
            .collect()
    }

    /// Whether the state can still be trusted to follow the compositor.
    pub fn check(&self) -> Result<(), Error> {
        if let Some(what) = self.protocol.finished() {
            return Err(Error::Finished(what));
        }
        self.lost
            .map_or(Ok(()), |what| Err(Error::Dispatcher(what)))
    }

    /// Records `proxy`, a proxy of `kind`, and routes its events to
    /// [`Protocol::event`]. On an error the proxy is still recorded, so
    /// `Drop` destroys it.
    pub fn track(&mut self, proxy: *mut WlProxy, kind: P::Kind) -> Result<(), Error> {
        self.attach(proxy, Object::Of(kind))
    }

    /// [`State::track`] from inside the dispatcher, which cannot return an
    /// error: a failure is kept for [`State::check`].
    pub fn track_or_lose(&mut self, proxy: *mut WlProxy, kind: P::Kind) {
        if self.track(proxy, kind).is_err() {
            self.lost.get_or_insert_with(|| P::interface(kind));
        }
    }

    /// Records `proxy`, a proxy of `kind` with no events, so `Drop`
    /// destroys it.
    pub fn keep(&mut self, proxy: *mut WlProxy, kind: P::Kind) {
        self.objects.push((proxy, Object::Of(kind)));
    }

    fn attach(&mut self, proxy: *mut WlProxy, object: Object<P::Kind>) -> Result<(), Error> {
        let interface = match object {
            Object::Registry => "wl_registry",
            Object::Callback => "wl_callback",
            Object::Of(kind) => P::interface(kind),
        };
        if proxy.is_null() {
            return Err(Error::Dispatcher(interface));
        }
        self.objects.push((proxy, object));
        // SAFETY: `proxy` is live and has no dispatcher yet, `dispatch` has
        // libwayland's dispatcher signature, and `root` is the allocation's
        // own pointer, live until every proxy is destroyed.
        let added = unsafe {
            (self.lib.proxy_add_dispatcher)(proxy, dispatch::<P>, self.root.cast(), ptr::null_mut())
        };
        if added == 0 {
            Ok(())
        } else {
            Err(Error::Dispatcher(interface))
        }
    }

    /// Forgets `proxy` and destroys it client-side.
    pub fn destroy(&mut self, proxy: *mut WlProxy) {
        self.forget(proxy);
        // SAFETY: `proxy` was live and tracked, and is no longer.
        unsafe { (self.lib.proxy_destroy)(proxy) };
    }

    /// Forgets `proxy`, which the caller destroys with a destructor request
    /// right after.
    pub fn forget(&mut self, proxy: *mut WlProxy) {
        self.objects.retain(|(candidate, _)| *candidate != proxy);
    }
}

/// libwayland's dispatcher for every object a [`Client`] owns.
unsafe extern "C" fn dispatch<P: Protocol>(
    user_data: *const c_void,
    target: *mut c_void,
    opcode: u32,
    message: *const WlMessage,
    args: *mut WlArgument,
) -> c_int {
    // SAFETY: `user_data` is `State::root`, live while any proxy is, and no
    // other reference to the state is held during a dispatch (see the
    // module's docs).
    let state = unsafe { &mut *user_data.cast::<State<P>>().cast_mut() };
    let proxy: *mut WlProxy = target.cast();
    let Some(object) = state.object_of(proxy) else {
        return 0;
    };
    // SAFETY (each decoder, and `event`): `object` is what the proxy was
    // created as, so `args` is that interface's array for `opcode`, and
    // every borrow is copied out inside the closure, which is as far as the
    // slice can reach.
    let handle = |args: &[WlArgument]| match object {
        Object::Registry => {
            if let Some(wl_registry::Event::Global {
                name,
                interface,
                version,
            }) = unsafe { wl_registry::decode_event(opcode, args) }
            {
                state.protocol.global(name, interface, version);
            }
        }
        Object::Callback => {
            if let Some(wl_callback::Event::Done { .. }) =
                unsafe { wl_callback::decode_event(opcode, args) }
            {
                state.synced += 1;
                // `done` is the callback's destructor event.
                state.destroy(proxy);
            }
        }
        Object::Of(kind) => unsafe { P::event(state, proxy, kind, opcode, args) },
    };
    // SAFETY: `message` and `args` are the descriptor and closure argument
    // array libwayland passed this dispatcher, unchanged while it runs.
    unsafe { ffi::with_dispatcher_args(message, args, handle) };
    0
}
