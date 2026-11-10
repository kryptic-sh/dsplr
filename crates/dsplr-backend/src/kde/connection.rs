//! One connection to KWin: binding `kde_output_management_v2`,
//! `kde_output_device_registry_v2` and `kde_output_order_v1`, reading the
//! output devices and the output order, and sending configurations, over a
//! [`Client`] that owns the socket and routes each proxy's events here.

use core::ffi::CStr;
use std::ffi::CString;
use std::path::Path;

use crcbl_wayland::ffi::{WlArgument, WlProxy};
use dsplr_model::Layout;

use super::Error;
use super::devices::{Device, ListedMode, ModeExtras, layout};
use super::plan::{DeviceConfig, Plan, plan, without_new};
use crate::wayland::client::{Client, Protocol, State};
use crate::wayland::heads::{HeadMode, SnapshotError};
use crate::wayland::protocol::kde_output_device::{
    kde_output_device_mode_v2, kde_output_device_registry_v2, kde_output_device_v2,
};
use crate::wayland::protocol::kde_output_management::{
    kde_mode_list_v2, kde_output_configuration_v2, kde_output_management_v2,
};
use crate::wayland::protocol::kde_output_order::kde_output_order_v1;
use crate::wayland::protocol::wayland::wl_registry;

/// The newest `kde_output_device_registry_v2` dsplr speaks: the vendored
/// XML's. Its devices and their modes are bound at the same version.
const REGISTRY_VERSION: u32 = 26;

/// The oldest `kde_output_device_registry_v2` KWin takes: it posts
/// `unsupported_version` for a bind below it (the protocol's description,
/// and `kde_output_device_registry_v2_bind_resource` in KWin 6.8's
/// `src/wayland/outputdevice_v2.cpp`).
pub const REGISTRY_SINCE: u32 = 21;

/// The newest `kde_output_management_v2` dsplr speaks: the vendored XML's.
const MANAGEMENT_VERSION: u32 = 22;

/// The newest `kde_output_order_v1` dsplr speaks: the vendored XML's.
const ORDER_VERSION: u32 = 1;

/// The first mode version with a `release` request.
const MODE_RELEASE_SINCE: u32 = 26;

/// KWin's answer to a configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Applied,
    Failed,
}

/// An open connection with KWin's output management bound and every output
/// device read up to its first `done`.
///
/// Nothing is read between calls, except in [`Connection::changed`]. KWin's
/// configurations carry no serial, so one made after the outputs changed is
/// not cancelled: read them back after applying. It cannot leave the thread
/// it was opened on.
#[derive(Debug)]
pub struct Connection {
    client: Client<Kde>,
}

impl Connection {
    /// Connects to KWin at `socket`, binds its output management, device
    /// registry and output order, and reads every device and the order.
    /// On an error, dropping the client destroys what was made.
    pub async fn open(socket: &Path) -> Result<Self, Error> {
        let mut client = Client::open(socket, Kde::default()).await?;
        let kde = &client.state().protocol;
        let (Some(management), Some(registry)) = (kde.management_global, kde.registry_global)
        else {
            return Err(Error::Missing(if kde.management_global.is_none() {
                "kde_output_management_v2"
            } else {
                "kde_output_device_registry_v2"
            }));
        };
        let order = kde.order_global;
        if registry.1 < REGISTRY_SINCE {
            return Err(Error::TooOld {
                interface: "kde_output_device_registry_v2",
                offered: registry.1,
                needed: REGISTRY_SINCE,
            });
        }
        let bind = |client: &Client<Kde>, (name, offered): (u32, u32), interface, newest: u32| {
            // SAFETY: the registry is live, and `name` is the global it
            // announced for `interface`, bound at a version both sides speak.
            unsafe { wl_registry::bind(client.registry(), name, interface, offered.min(newest)) }
        };
        let management_proxy = bind(
            &client,
            management,
            &kde_output_management_v2::INTERFACE,
            MANAGEMENT_VERSION,
        );
        let registry_proxy = bind(
            &client,
            registry,
            &kde_output_device_registry_v2::INTERFACE,
            REGISTRY_VERSION,
        );
        let order_proxy = order.map(|order| {
            bind(
                &client,
                order,
                &kde_output_order_v1::INTERFACE,
                ORDER_VERSION,
            )
        });
        let state = client.state_mut();
        state.protocol.management = management_proxy;
        state.protocol.management_version = management.1.min(MANAGEMENT_VERSION);
        state.protocol.device_version = registry.1.min(REGISTRY_VERSION);
        // Every proxy is recorded before any error is returned, so dropping
        // the client destroys them all.
        let mut tracked = vec![
            state.track(management_proxy, Kind::Management),
            state.track(registry_proxy, Kind::DeviceRegistry),
        ];
        if let Some(order_proxy) = order_proxy {
            tracked.push(state.track(order_proxy, Kind::Order));
        }
        tracked.into_iter().collect::<Result<(), _>>()?;
        // KWin announces every device, and each sends its state and `done`,
        // as the registry is bound; the order is sent as it is bound too.
        client.roundtrip("list its outputs").await?;
        client
            .dispatch_until("describe its outputs", |kde| {
                kde.live().all(|slot| slot.described)
                    && (kde.order_global.is_none() || kde.order.is_some())
            })
            .await?;
        Ok(Self { client })
    }

    /// The version `kde_output_management_v2` was bound at.
    pub fn management_version(&self) -> u32 {
        self.client.state().protocol.management_version
    }

    /// Whether KWin offers `kde_output_order_v1`, which says which output is
    /// primary.
    pub fn has_order(&self) -> bool {
        self.client.state().protocol.order_global.is_some()
    }

    /// Every connected device, as the events read so far describe it.
    pub fn devices(&self) -> Vec<Device> {
        self.client.state().protocol.devices()
    }

    /// `kde_output_order_v1`'s last list of output names, the primary
    /// output first; `None` when KWin offers no order.
    pub fn order(&self) -> Option<&[String]> {
        self.client.state().protocol.order.as_deref()
    }

    /// The devices as a [`Layout`].
    pub fn layout(&self) -> Result<Layout, SnapshotError> {
        layout(&self.devices(), self.order())
    }

    /// The configuration `layout` would be sent as, sending nothing: why it
    /// cannot be sent, if it cannot.
    pub fn plan(&self, layout: &Layout) -> Result<Plan, Error> {
        Ok(self.client.state().protocol.plan(layout)?)
    }

    /// Applies `layout` and waits for KWin's answer. `Ok` means KWin
    /// reported it applied; it may have rounded a setting (it keeps scales
    /// in 120ths), so read the state back to see. Nothing is sent when the
    /// layout cannot be put into a configuration.
    ///
    /// A mode a device does not list takes two configurations: the first
    /// sends its custom mode list (the custom modes it has, and the new
    /// one), the second, once KWin lists the new mode, the layout. When
    /// the second is not applied, the new mode stays in KWin's list,
    /// unused.
    pub async fn apply(&mut self, layout: &Layout) -> Result<(), Error> {
        self.client.state().check()?;
        let plan = self.plan(layout)?;
        if plan.mode_lists.iter().all(Option::is_none) {
            return self.configure(&plan).await;
        }
        // Each device sent a list, by its uuid, so a device KWin adds or
        // removes meanwhile does not shift the others: what it lists
        // before (KWin adds the new mode as an object of its own, at the
        // timings it makes for it), the new mode's size and refresh, and
        // the list to send back if the layout is not applied.
        let added: Vec<Added> = self
            .devices()
            .iter()
            .zip(&plan.mode_lists)
            .filter_map(|(device, list)| {
                let list = list.as_ref()?;
                let ListedMode::Mode {
                    width,
                    height,
                    refresh_mhz,
                    ..
                } = *list.last()?
                else {
                    return None;
                };
                Some(Added {
                    uuid: device.uuid.clone(),
                    connector: device.head.name.clone(),
                    before: device.head.modes.iter().map(|mode| mode.handle).collect(),
                    wanted: (
                        i32::try_from(width).ok()?,
                        i32::try_from(height).ok()?,
                        i32::try_from(refresh_mhz).ok()?,
                    ),
                    restore: without_new(list),
                })
            })
            .collect();
        self.add_modes(&plan.mode_lists).await?;
        let listed = match self
            .client
            .dispatch_until("list the custom mode it was sent", |kde| {
                chosen(kde, &added).is_some()
            })
            .await
        {
            Err(crate::wayland::Error::Timeout { .. }) => Err(Error::CustomModeNotListed {
                connectors: added.iter().map(|added| added.connector.clone()).collect(),
            }),
            waited => waited.map_err(Error::from),
        };
        let applied = match listed {
            Ok(()) => {
                let chosen = chosen(&self.client.state().protocol, &added)
                    .expect("waited for until every new mode was listed");
                match self.client.state().protocol.plan_with(layout, &chosen) {
                    Ok(plan) => self.configure(&plan).await,
                    Err(error) => Err(error.into()),
                }
            }
            Err(error) => Err(error),
        };
        let Err(error) = applied else {
            return Ok(());
        };
        // Not applied: the lists go back as they were, so a failed attempt
        // leaves no mode behind.
        let restore = self.lists_by_uuid(&added, |added| added.restore.clone());
        match self.add_modes(&restore).await {
            Ok(()) => Err(error),
            Err(_) => Err(Error::CustomModeKept {
                cause: Box::new(error),
                connectors: added.iter().map(|added| added.connector.clone()).collect(),
            }),
        }
    }

    /// For each live device, the list `list` makes of the one in `added`
    /// with its uuid; `None` for the others.
    fn lists_by_uuid(
        &self,
        added: &[Added],
        list: impl Fn(&Added) -> Vec<ListedMode>,
    ) -> Vec<Option<Vec<ListedMode>>> {
        self.devices()
            .iter()
            .map(|device| {
                added
                    .iter()
                    .find(|added| added.uuid == device.uuid)
                    .map(&list)
            })
            .collect()
    }

    /// Sends `plan`, in which every device's mode is one it lists, as one
    /// configuration, and waits for KWin's answer.
    async fn configure(&mut self, plan: &Plan) -> Result<(), Error> {
        let state = self.client.state();
        let kde = &state.protocol;
        // Each device and listed mode resolved to its object before
        // anything is sent.
        let targets: Vec<(
            *mut WlProxy,
            DeviceConfig,
            Option<*mut WlProxy>,
            Option<u32>,
        )> = kde
            .live_indices()
            .zip(&plan.devices)
            .zip(&plan.priorities)
            .map(|((index, config), priority)| {
                let device = state
                    .proxy_of(Kind::Device(index))
                    .expect("a live device is tracked");
                let mode = match config {
                    DeviceConfig::Enable(settings) => Some(
                        state
                            .proxy_of(Kind::Mode {
                                device: index,
                                handle: settings
                                    .mode
                                    .expect("a custom mode is listed before it is set"),
                            })
                            .expect("`plan` chooses among the device's live modes"),
                    ),
                    DeviceConfig::Disable | DeviceConfig::Unchanged => None,
                };
                (device, config.clone(), mode, *priority)
            })
            .collect();
        let configuration = self.new_configuration()?;
        for (device, config, mode, priority) in targets {
            // SAFETY: `configuration` is live and not applied, `device` is a
            // live device, and `mode` one of its live modes.
            unsafe { configure_device(configuration, device, &config, mode, priority) };
        }
        self.send(configuration, Vec::new()).await
    }

    /// Sends each device's custom mode list in `lists` (by device, as
    /// [`Plan::mode_lists`]) in one configuration, and waits for KWin's
    /// answer.
    async fn add_modes(&mut self, lists: &[Option<Vec<ListedMode>>]) -> Result<(), Error> {
        let devices: Vec<*mut WlProxy> = {
            let state = self.client.state();
            state
                .protocol
                .live_indices()
                .map(|index| {
                    state
                        .proxy_of(Kind::Device(index))
                        .expect("a live device is tracked")
                })
                .collect()
        };
        let management = self.client.state().protocol.management;
        let configuration = self.new_configuration()?;
        let mut made = Vec::new();
        for (device, modes) in devices.into_iter().zip(lists) {
            let Some(modes) = modes else {
                continue;
            };
            // SAFETY: the management global is live, at version 18 or
            // later (`plan` sends no list below it).
            let list = unsafe { kde_output_management_v2::create_mode_list(management) };
            made.push(list);
            self.client.state_mut().track(list, Kind::ModeList)?;
            // SAFETY: `list` is live, `configuration` live and not applied,
            // `device` a live device, and `add_cvt` only sent at the version
            // that has it (`plan`).
            unsafe {
                fill_mode_list(list, modes);
                kde_output_configuration_v2::set_custom_modes(configuration, device, list);
            }
        }
        self.send(configuration, made).await
    }

    /// A new configuration, tracked, with no answer yet.
    fn new_configuration(&mut self) -> Result<*mut WlProxy, Error> {
        let management = self.client.state().protocol.management;
        // SAFETY: the management global is live: it has no destructor event,
        // and only `Drop` destroys it.
        let configuration = unsafe { kde_output_management_v2::create_configuration(management) };
        let state = self.client.state_mut();
        state.protocol.outcome = None;
        state.protocol.failure_reason = None;
        state.track(configuration, Kind::Configuration)?;
        Ok(configuration)
    }

    /// Applies `configuration`, waits for KWin's answer, then destroys it
    /// and `lists`, the mode lists it was sent.
    async fn send(
        &mut self,
        configuration: *mut WlProxy,
        lists: Vec<*mut WlProxy>,
    ) -> Result<(), Error> {
        // SAFETY: `configuration` is live and applied only here.
        unsafe { kde_output_configuration_v2::apply(configuration) };
        let answered = self
            .client
            .dispatch_until("answer the apply", |kde| kde.outcome.is_some())
            .await;
        let state = self.client.state_mut();
        let outcome = state.protocol.outcome.take();
        let reason = state.protocol.failure_reason.take();
        state.forget(configuration);
        // SAFETY: `configuration` is live, and `destroy` is its destructor.
        unsafe { kde_output_configuration_v2::destroy(configuration) };
        for list in lists {
            self.client.state_mut().forget(list);
            // SAFETY: `list` is live, and `destroy` is its destructor.
            unsafe { kde_mode_list_v2::destroy(list) };
        }
        answered?;
        match outcome {
            Some(Outcome::Applied) => Ok(()),
            Some(Outcome::Failed) | None => Err(Error::Failed { reason }),
        }
    }

    /// Waits, for as long as it takes, until KWin reports that its outputs
    /// may have changed: a device's `done` or `removed`, or a new output
    /// order, after those already dispatched. The devices then describe the
    /// outputs as they are now, except one KWin announced and has not yet
    /// finished describing, whose own `done` is the next change.
    /// Cancel-safe, as [`Client::wait`] is.
    pub async fn changed(&mut self) -> Result<(), Error> {
        let seen = self.client.state().protocol.changes;
        Ok(self.client.wait(|kde| kde.changes != seen).await?)
    }
}

/// A device a custom mode list was sent for, by its uuid.
struct Added {
    uuid: String,
    connector: String,
    /// The handles of the modes it listed before.
    before: Vec<u64>,
    /// The new mode: width, height and refresh in mHz.
    wanted: (i32, i32, i32),
    /// The list that puts its custom modes back as they were.
    restore: Vec<ListedMode>,
}

/// The mode object each live device of `kde` is to be set to, by its place
/// among them: the new one for a device in `added`, `None` for the rest.
/// `None` as a whole until every device in `added` lists its new mode.
fn chosen(kde: &Kde, added: &[Added]) -> Option<Vec<Option<u64>>> {
    let live: Vec<&DeviceSlot> = kde.live().collect();
    for added in added {
        let device = live.iter().find(|slot| slot.device.uuid == added.uuid)?;
        new_mode(&device.device, &added.before, added.wanted)?;
    }
    Some(
        live.iter()
            .map(|slot| {
                let added = added.iter().find(|added| added.uuid == slot.device.uuid)?;
                new_mode(&slot.device, &added.before, added.wanted)
            })
            .collect(),
    )
}

/// The mode `device` lists that was not among `before` (handles), at the
/// size of `wanted` (width, height, refresh in mHz), at the refresh nearest
/// it: the mode KWin made of a custom mode it was sent, at timings of its
/// own. `None` until KWin lists one.
fn new_mode(device: &Device, before: &[u64], wanted: (i32, i32, i32)) -> Option<u64> {
    let (width, height, refresh_mhz) = wanted;
    device
        .head
        .modes
        .iter()
        .filter(|mode| !before.contains(&mode.handle))
        .filter(|mode| (mode.width, mode.height) == (width, height) && mode.refresh_mhz > 0)
        .min_by_key(|mode| (mode.refresh_mhz - refresh_mhz).unsigned_abs())
        .map(|mode| mode.handle)
}

/// Adds each of `modes` to `list`.
///
/// # Safety
///
/// `list` must be a live `kde_mode_list_v2`, at version 22 or later when
/// `modes` holds a [`ListedMode::Cvt`].
unsafe fn fill_mode_list(list: *mut WlProxy, modes: &[ListedMode]) {
    use kde_mode_list_v2 as l;
    // SAFETY (each request): the caller's.
    unsafe {
        for mode in modes {
            match *mode {
                ListedMode::Mode {
                    width,
                    height,
                    refresh_mhz,
                    reduced_blanking,
                } => {
                    l::set_resolution(list, width, height);
                    l::set_refresh_rate(list, refresh_mhz);
                    l::set_reduced_blanking(list, u32::from(reduced_blanking));
                    l::add_mode(list);
                }
                ListedMode::Cvt(
                    [
                        dot_clock,
                        hdisplay,
                        hsync_start,
                        hsync_end,
                        htotal,
                        hskew,
                        vdisplay,
                        vsync_start,
                        vsync_end,
                        vtotal,
                        vscan,
                        flags,
                    ],
                ) => l::add_cvt(
                    list,
                    dot_clock,
                    hdisplay,
                    hsync_start,
                    hsync_end,
                    htotal,
                    hskew,
                    vdisplay,
                    vsync_start,
                    vsync_end,
                    vtotal,
                    vscan,
                    flags,
                ),
            }
        }
    }
}

/// Sends the requests that set one device in `configuration`.
///
/// # Safety
///
/// `configuration` must be live and not yet applied, `device` a live output
/// device, and `mode` the live object of `config`'s mode, when it has one.
unsafe fn configure_device(
    configuration: *mut WlProxy,
    device: *mut WlProxy,
    config: &DeviceConfig,
    mode: Option<*mut WlProxy>,
    priority: Option<u32>,
) {
    use kde_output_configuration_v2 as c;
    // SAFETY (each request): the caller's, and each is a request the bound
    // version has (`plan` leaves out what it has not).
    unsafe {
        match config {
            DeviceConfig::Unchanged => {}
            DeviceConfig::Disable => c::enable(configuration, device, 0),
            DeviceConfig::Enable(settings) => {
                let mode = mode.expect("an enabled device's mode was resolved to its object");
                c::enable(configuration, device, 1);
                c::mode(configuration, device, mode);
                c::transform(configuration, device, settings.transform);
                c::position(
                    configuration,
                    device,
                    settings.position.0,
                    settings.position.1,
                );
                c::scale(configuration, device, settings.scale);
                if let Some(overscan) = settings.overscan {
                    c::overscan(configuration, device, overscan);
                }
                if let Some(policy) = settings.vrr_policy {
                    c::set_vrr_policy(configuration, device, policy);
                }
                if let Some(range) = settings.rgb_range {
                    c::set_rgb_range(configuration, device, range);
                }
                if let Some(source) = &settings.replication_source {
                    let source =
                        CString::new(source.as_str()).expect("a uuid read from a C string");
                    c::set_replication_source(configuration, device, &source);
                }
            }
        }
        if let Some(priority) = priority {
            c::set_priority(configuration, device, priority);
        }
    }
}

/// What each of the protocols' proxies is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Management,
    DeviceRegistry,
    Order,
    /// A device, by its index in [`Kde::devices`].
    Device(usize),
    /// A mode, by its device's index and its ordinal among that device's
    /// modes.
    Mode {
        device: usize,
        handle: u64,
    },
    Configuration,
    /// A custom mode list, which has no events.
    ModeList,
}

/// One device and what is known of its proxy.
#[derive(Debug, Default)]
struct DeviceSlot {
    device: Device,
    /// KWin sent its first `done`: it is fully described.
    described: bool,
    /// KWin removed it; its proxy is released.
    removed: bool,
    /// The next mode object's ordinal among this device's modes.
    next_mode: u64,
}

/// What KWin's output protocols' events say.
#[derive(Debug)]
struct Kde {
    /// Each global's registry name and version, once announced.
    management_global: Option<(u32, u32)>,
    registry_global: Option<(u32, u32)>,
    order_global: Option<(u32, u32)>,
    management: *mut WlProxy,
    /// The version the management global was bound at.
    management_version: u32,
    /// The version the device registry, and so each device, was bound at.
    device_version: u32,
    devices: Vec<DeviceSlot>,
    /// The last complete output order.
    order: Option<Vec<String>>,
    /// The output order being announced, until its `done`.
    incoming_order: Vec<String>,
    /// How many changes KWin has reported: each device's `done` and
    /// `removed`, and each output order.
    changes: u64,
    /// The device registry sent `finished`.
    finished: bool,
    /// The answer to the configuration in flight.
    outcome: Option<Outcome>,
    failure_reason: Option<String>,
}

impl Default for Kde {
    fn default() -> Self {
        Self {
            management_global: None,
            registry_global: None,
            order_global: None,
            management: core::ptr::null_mut(),
            management_version: 0,
            device_version: 0,
            devices: Vec::new(),
            order: None,
            incoming_order: Vec::new(),
            changes: 0,
            finished: false,
            outcome: None,
            failure_reason: None,
        }
    }
}

impl Kde {
    /// The devices KWin has not removed, in the order it announced them.
    fn live(&self) -> impl Iterator<Item = &DeviceSlot> {
        self.devices.iter().filter(|slot| !slot.removed)
    }

    /// Every connected device, as the events read so far describe it.
    fn devices(&self) -> Vec<Device> {
        self.live().map(|slot| slot.device.clone()).collect()
    }

    /// The configuration `layout` would be sent as, against the devices
    /// as they are now.
    fn plan(&self, layout: &Layout) -> Result<Plan, crate::wayland::plan::PlanError> {
        self.plan_with(layout, &[])
    }

    /// [`Kde::plan`], each device that `chosen` names a mode for at that
    /// mode ([`plan`]).
    fn plan_with(
        &self,
        layout: &Layout,
        chosen: &[Option<u64>],
    ) -> Result<Plan, crate::wayland::plan::PlanError> {
        plan(
            layout,
            &self.devices(),
            self.order.as_deref(),
            self.management_version,
            chosen,
        )
    }

    /// The index of each device [`Kde::live`] gives.
    fn live_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, slot)| !slot.removed)
            .map(|(index, _)| index)
    }

    /// The `flags` and `cvt` of the mode `handle` of `device`, made empty
    /// the first time either is heard of.
    fn extras_mut(&mut self, device: usize, handle: u64) -> &mut ModeExtras {
        let extras = &mut self.devices[device].device.mode_extras;
        let at = match extras.iter().position(|extras| extras.handle == handle) {
            Some(at) => at,
            None => {
                extras.push(ModeExtras {
                    handle,
                    ..ModeExtras::default()
                });
                extras.len() - 1
            }
        };
        &mut extras[at]
    }

    fn mode_mut(&mut self, device: usize, handle: u64) -> Option<&mut HeadMode> {
        self.devices[device]
            .device
            .head
            .modes
            .iter_mut()
            .find(|mode| mode.handle == handle)
    }
}

impl Protocol for Kde {
    type Kind = Kind;

    fn interface(kind: Kind) -> &'static str {
        match kind {
            Kind::Management => "kde_output_management_v2",
            Kind::DeviceRegistry => "kde_output_device_registry_v2",
            Kind::Order => "kde_output_order_v1",
            Kind::Device(_) => "kde_output_device_v2",
            Kind::Mode { .. } => "kde_output_device_mode_v2",
            Kind::Configuration => "kde_output_configuration_v2",
            Kind::ModeList => "kde_mode_list_v2",
        }
    }

    fn global(&mut self, name: u32, interface: &CStr, version: u32) {
        let global = Some((name, version));
        if interface == kde_output_management_v2::NAME {
            self.management_global = global;
        } else if interface == kde_output_device_registry_v2::NAME {
            self.registry_global = global;
        } else if interface == kde_output_order_v1::NAME {
            self.order_global = global;
        }
    }

    fn finished(&self) -> Option<&'static str> {
        self.finished.then_some("kde_output_device_registry_v2")
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
            // The management global and mode lists have no events.
            Kind::Management | Kind::ModeList => {}
            Kind::DeviceRegistry => {
                match unsafe { kde_output_device_registry_v2::decode_event(opcode, args) } {
                    Some(kde_output_device_registry_v2::Event::Output { output }) => {
                        let kde = &mut state.protocol;
                        kde.devices.push(DeviceSlot {
                            device: Device {
                                version: kde.device_version,
                                ..Device::default()
                            },
                            ..DeviceSlot::default()
                        });
                        let index = kde.devices.len() - 1;
                        state.track_or_lose(output, Kind::Device(index));
                    }
                    Some(kde_output_device_registry_v2::Event::Finished) => {
                        state.protocol.finished = true;
                        // `finished` is the registry's destructor event.
                        state.destroy(proxy);
                    }
                    None => {}
                }
            }
            Kind::Order => match unsafe { kde_output_order_v1::decode_event(opcode, args) } {
                Some(kde_output_order_v1::Event::Output { output_name }) => state
                    .protocol
                    .incoming_order
                    .push(output_name.to_string_lossy().into_owned()),
                Some(kde_output_order_v1::Event::Done) => {
                    let kde = &mut state.protocol;
                    kde.order = Some(core::mem::take(&mut kde.incoming_order));
                    kde.changes += 1;
                }
                None => {}
            },
            Kind::Device(index) => {
                let event = unsafe { kde_output_device_v2::decode_event(opcode, args) };
                device_event(state, proxy, index, event);
            }
            Kind::Mode { device, handle } => {
                match unsafe { kde_output_device_mode_v2::decode_event(opcode, args) } {
                    Some(kde_output_device_mode_v2::Event::Removed) => {
                        let slot = &mut state.protocol.devices[device].device;
                        slot.mode_extras.retain(|extras| extras.handle != handle);
                        let head = &mut slot.head;
                        head.modes.retain(|mode| mode.handle != handle);
                        if head.current_mode == Some(handle) {
                            head.current_mode = None;
                        }
                        release_mode(state, proxy);
                    }
                    Some(kde_output_device_mode_v2::Event::Flags { flags }) => {
                        state.protocol.extras_mut(device, handle).flags = flags;
                    }
                    Some(kde_output_device_mode_v2::Event::Cvt {
                        dot_clock,
                        hdisplay,
                        hsync_start,
                        hsync_end,
                        htotal,
                        hskew,
                        vdisplay,
                        vsync_start,
                        vsync_end,
                        vtotal,
                        vscan,
                        flags,
                    }) => {
                        state.protocol.extras_mut(device, handle).cvt = Some([
                            dot_clock,
                            hdisplay,
                            hsync_start,
                            hsync_end,
                            htotal,
                            hskew,
                            vdisplay,
                            vsync_start,
                            vsync_end,
                            vtotal,
                            vscan,
                            flags,
                        ]);
                    }
                    Some(event) => {
                        if let Some(mode) = state.protocol.mode_mut(device, handle) {
                            match event {
                                kde_output_device_mode_v2::Event::Size { width, height } => {
                                    (mode.width, mode.height) = (width, height);
                                }
                                kde_output_device_mode_v2::Event::Refresh { refresh } => {
                                    mode.refresh_mhz = refresh;
                                }
                                kde_output_device_mode_v2::Event::Preferred => {
                                    mode.preferred = true;
                                }
                                kde_output_device_mode_v2::Event::Removed
                                | kde_output_device_mode_v2::Event::Flags { .. }
                                | kde_output_device_mode_v2::Event::Cvt { .. } => {}
                            }
                        }
                    }
                    None => {}
                }
            }
            Kind::Configuration => {
                let kde = &mut state.protocol;
                match unsafe { kde_output_configuration_v2::decode_event(opcode, args) } {
                    Some(kde_output_configuration_v2::Event::Applied) => {
                        kde.outcome.get_or_insert(Outcome::Applied);
                    }
                    Some(kde_output_configuration_v2::Event::Failed) => {
                        kde.outcome.get_or_insert(Outcome::Failed);
                    }
                    // Sent before `failed` (since version 12).
                    Some(kde_output_configuration_v2::Event::FailureReason { reason }) => {
                        kde.failure_reason = Some(reason.to_string_lossy().into_owned());
                    }
                    None => {}
                }
            }
        }
    }
}

/// Forgets `proxy`, a mode KWin removed, and releases it: with `release`
/// where the version has one, which also destroys it.
fn release_mode(state: &mut State<Kde>, proxy: *mut WlProxy) {
    if state.protocol.device_version < MODE_RELEASE_SINCE {
        state.destroy(proxy);
        return;
    }
    state.forget(proxy);
    // SAFETY: `proxy` is a live mode at a version with `release`, which
    // destroys it.
    unsafe { kde_output_device_mode_v2::release(proxy) };
}

/// One event of the device at `index`.
fn device_event(
    state: &mut State<Kde>,
    proxy: *mut WlProxy,
    index: usize,
    event: Option<kde_output_device_v2::Event<'_>>,
) {
    use kde_output_device_v2::Event;
    let text = |text: &CStr| text.to_string_lossy().into_owned();
    let Some(event) = event else {
        return;
    };
    match event {
        Event::Mode { mode } => {
            let slot = &mut state.protocol.devices[index];
            let handle = slot.next_mode;
            slot.next_mode += 1;
            slot.device.head.modes.push(HeadMode {
                handle,
                ..HeadMode::default()
            });
            state.track_or_lose(
                mode,
                Kind::Mode {
                    device: index,
                    handle,
                },
            );
        }
        Event::CurrentMode { mode } => {
            if let Some(Kind::Mode { handle, .. }) = state.kind_of(mode) {
                state.protocol.devices[index].device.head.current_mode = Some(handle);
            }
        }
        Event::Done => {
            let kde = &mut state.protocol;
            kde.devices[index].described = true;
            kde.changes += 1;
        }
        Event::Removed => {
            let kde = &mut state.protocol;
            kde.devices[index].removed = true;
            kde.changes += 1;
            state.forget(proxy);
            // SAFETY: `proxy` is a live device, bound at version 21 or later
            // (`REGISTRY_SINCE`), which has `release`, its destructor.
            unsafe { kde_output_device_v2::release(proxy) };
        }
        event => {
            let device = &mut state.protocol.devices[index].device;
            let head = &mut device.head;
            match event {
                Event::Geometry {
                    x,
                    y,
                    physical_width,
                    physical_height,
                    make,
                    model,
                    transform,
                    ..
                } => {
                    head.position = (x, y);
                    head.physical_size = (physical_width, physical_height);
                    head.make = text(make);
                    head.model = text(model);
                    head.transform = transform;
                }
                Event::Scale { factor } => head.scale = factor,
                Event::Enabled { enabled } => head.enabled = enabled != 0,
                Event::Name { name } => head.name = text(name),
                Event::SerialNumber { serialNumber } => head.serial_number = text(serialNumber),
                Event::Uuid { uuid } => device.uuid = text(uuid),
                Event::Capabilities { flags } => device.capabilities = flags,
                Event::Overscan { overscan } => device.overscan = overscan,
                Event::VrrPolicy { vrr_policy } => device.vrr_policy = vrr_policy,
                Event::RgbRange { rgb_range } => device.rgb_range = rgb_range,
                Event::ReplicationSource { source } => device.replication_source = text(source),
                // Read where dsplr has a field for it; the rest (EDID, HDR,
                // colour, brightness and the like) is KWin's alone for now.
                _ => {}
            }
        }
    }
}

/// The decoding the dispatcher does for KDE's events, on argument arrays
/// built by hand as libwayland hands them over: the eight-argument
/// `geometry` with its two strings, a new device and a new mode, and the
/// failure reason. Miri-friendly as the wlr ones are (`cargo +nightly miri
/// test -p dsplr-backend --lib kde::connection`).
#[cfg(test)]
mod tests {
    use core::ptr;

    use crcbl_wayland::ffi::WlArgument;

    use super::{
        kde_output_configuration_v2, kde_output_device_mode_v2, kde_output_device_registry_v2,
        kde_output_device_v2, kde_output_order_v1,
    };
    use crate::wayland::tests::dispatched;

    #[test]
    fn the_new_mode_is_found_by_the_device_s_uuid_whatever_its_place() {
        use super::{Added, DeviceSlot, Kde, chosen};
        use crate::kde::devices::Device;
        use crate::wayland::heads::{Head, HeadMode};
        let slot = |uuid: &str, modes: Vec<(u64, i32, i32, i32)>| DeviceSlot {
            device: Device {
                uuid: uuid.into(),
                head: Head {
                    modes: modes
                        .into_iter()
                        .map(|(handle, width, height, refresh_mhz)| HeadMode {
                            handle,
                            width,
                            height,
                            refresh_mhz,
                            preferred: false,
                        })
                        .collect(),
                    ..Head::default()
                },
                ..Device::default()
            },
            ..DeviceSlot::default()
        };
        let added = [Added {
            uuid: "b".into(),
            connector: "Virtual-2".into(),
            before: vec![0],
            wanted: (1280, 720, 60_000),
            restore: Vec::new(),
        }];
        // A device plugged in before it meanwhile: it is the third now.
        let mut kde = Kde {
            devices: vec![
                slot("new", vec![(0, 1280, 720, 60_000)]),
                slot("a", vec![(0, 1920, 1080, 60_000)]),
                slot("b", vec![(0, 1920, 1080, 60_000)]),
            ],
            ..Kde::default()
        };
        assert_eq!(chosen(&kde, &added), None, "not listed yet");
        kde.devices[2]
            .device
            .head
            .modes
            .push(slot("", vec![(1, 1280, 720, 59_855)]).device.head.modes[0].clone());
        assert_eq!(chosen(&kde, &added), Some(vec![None, None, Some(1)]));
        // Gone meanwhile: never listed.
        kde.devices[2].removed = true;
        assert_eq!(chosen(&kde, &added), None);
    }

    #[test]
    fn the_new_mode_is_the_one_kwin_added_at_the_size_nearest_the_refresh() {
        use super::new_mode;
        use crate::kde::devices::Device;
        use crate::wayland::heads::{Head, HeadMode};
        let mode = |handle, width, height, refresh_mhz| HeadMode {
            handle,
            width,
            height,
            refresh_mhz,
            preferred: false,
        };
        let mut device = Device {
            head: Head {
                modes: vec![mode(0, 1280, 1024, 60_020)],
                ..Head::default()
            },
            ..Device::default()
        };
        let before = [0];
        let wanted = (1280, 1024, 60_000);
        assert_eq!(new_mode(&device, &before, wanted), None, "listed before");
        // Announced, its size not yet sent.
        device.head.modes.push(mode(1, 0, 0, 0));
        assert_eq!(new_mode(&device, &before, wanted), None);
        device.head.modes[1] = mode(1, 1280, 1024, 59_895);
        device.head.modes.push(mode(2, 1280, 1024, 75_025));
        device.head.modes.push(mode(3, 1600, 900, 60_000));
        assert_eq!(new_mode(&device, &before, wanted), Some(1));
    }

    #[test]
    fn geometry_decodes_all_eight_arguments_strings_included() {
        let mut slots = [
            WlArgument { i: 1920 },
            WlArgument { i: 0 },
            WlArgument { i: 600 },
            WlArgument { i: 340 },
            WlArgument { i: 0 },
            WlArgument { s: c"AOC".as_ptr() },
            WlArgument {
                s: c"Q27G4".as_ptr(),
            },
            WlArgument { i: 3 },
        ];
        let geometry = dispatched(c"iiiiissi", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_device_v2.geometry` array.
            match unsafe {
                kde_output_device_v2::decode_event(kde_output_device_v2::EVT_GEOMETRY, args)
            } {
                Some(kde_output_device_v2::Event::Geometry {
                    x,
                    y,
                    physical_width,
                    physical_height,
                    make,
                    model,
                    transform,
                    ..
                }) => Some((
                    (x, y, physical_width, physical_height, transform),
                    make.to_string_lossy().into_owned(),
                    model.to_string_lossy().into_owned(),
                )),
                _ => None,
            }
        });
        assert_eq!(
            geometry,
            Some(((1920, 0, 600, 340, 3), "AOC".into(), "Q27G4".into()))
        );
    }

    #[test]
    fn a_new_device_and_a_new_mode_decode_to_their_object() {
        // A stand-in address, never dereferenced: decoding copies it out.
        let object = ptr::without_provenance_mut(0x1000);
        let mut slots = [WlArgument { o: object }];
        let output = dispatched(c"21n", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_device_registry_v2.output`
            // array.
            match unsafe {
                kde_output_device_registry_v2::decode_event(
                    kde_output_device_registry_v2::EVT_OUTPUT,
                    args,
                )
            } {
                Some(kde_output_device_registry_v2::Event::Output { output }) => Some(output),
                _ => None,
            }
        });
        assert_eq!(output, Some(object));

        let mode = dispatched(c"n", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_device_v2.mode` array.
            match unsafe {
                kde_output_device_v2::decode_event(kde_output_device_v2::EVT_MODE, args)
            } {
                Some(kde_output_device_v2::Event::Mode { mode }) => Some(mode),
                _ => None,
            }
        });
        assert_eq!(mode, Some(object));
    }

    #[test]
    fn a_mode_s_size_and_refresh_decode() {
        let mut slots = [WlArgument { i: 2560 }, WlArgument { i: 1440 }];
        let size = dispatched(c"ii", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_device_mode_v2.size` array.
            match unsafe {
                kde_output_device_mode_v2::decode_event(kde_output_device_mode_v2::EVT_SIZE, args)
            } {
                Some(kde_output_device_mode_v2::Event::Size { width, height }) => {
                    Some((width, height))
                }
                _ => None,
            }
        });
        assert_eq!(size, Some((2560, 1440)));

        let mut slots = [WlArgument { i: 119_998 }];
        let refresh = dispatched(c"i", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_device_mode_v2.refresh` array.
            match unsafe {
                kde_output_device_mode_v2::decode_event(
                    kde_output_device_mode_v2::EVT_REFRESH,
                    args,
                )
            } {
                Some(kde_output_device_mode_v2::Event::Refresh { refresh }) => Some(refresh),
                _ => None,
            }
        });
        assert_eq!(refresh, Some(119_998));
    }

    #[test]
    fn the_failure_reason_and_an_output_name_are_copied_out() {
        let mut slots = [WlArgument {
            s: c"Position of enabled output Virtual-0 is negative (-1, 0)".as_ptr(),
        }];
        let reason = dispatched(c"12s", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_configuration_v2.failure_reason`
            // array.
            match unsafe {
                kde_output_configuration_v2::decode_event(
                    kde_output_configuration_v2::EVT_FAILURE_REASON,
                    args,
                )
            } {
                Some(kde_output_configuration_v2::Event::FailureReason { reason }) => {
                    Some(reason.to_string_lossy().into_owned())
                }
                _ => None,
            }
        });
        assert_eq!(
            reason.as_deref(),
            Some("Position of enabled output Virtual-0 is negative (-1, 0)")
        );

        let mut slots = [WlArgument {
            s: c"Virtual-1".as_ptr(),
        }];
        let name = dispatched(c"s", &mut slots, |args| {
            // SAFETY: `args` is a `kde_output_order_v1.output` array.
            match unsafe {
                kde_output_order_v1::decode_event(kde_output_order_v1::EVT_OUTPUT, args)
            } {
                Some(kde_output_order_v1::Event::Output { output_name }) => {
                    Some(output_name.to_string_lossy().into_owned())
                }
                _ => None,
            }
        });
        assert_eq!(name.as_deref(), Some("Virtual-1"));
    }

    #[test]
    fn an_array_shorter_than_geometry_decodes_to_nothing() {
        let mut slots = [WlArgument { i: 0 }, WlArgument { i: 0 }];
        let decoded = dispatched(c"ii", &mut slots, |args| {
            // SAFETY: a two-slot array, which the decoder must decline for
            // the eight-argument `geometry`.
            unsafe { kde_output_device_v2::decode_event(kde_output_device_v2::EVT_GEOMETRY, args) }
                .is_some()
        });
        assert!(!decoded);
    }
}
