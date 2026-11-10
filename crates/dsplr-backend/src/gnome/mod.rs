//! GNOME, over Mutter's `org.gnome.Mutter.DisplayConfig` on the session bus
//! (`docs/plan/04-backends.md#gnome-gnome`): `GetCurrentState` to read,
//! `ApplyMonitorsConfig` to test (`verify`), apply (`temporary`) and save
//! (`persistent`), and the `MonitorsChanged` signal to follow changes.
//!
//! [`Gnome`] finds Mutter and does each request on a connection of its
//! own, as the Wayland backends do, so no signal stream left unread can
//! hold up a reply. Turning Mutter's state into dsplr's display model, and
//! a layout into a configuration, is plain data and tested without Mutter.
//!
//! How Mutter treats each method is read in Mutter 51.1's
//! `src/backends/meta-monitor-manager.c`: `temporary` is never reverted or
//! saved by Mutter; `persistent` starts Mutter's confirmation (GNOME Shell's
//! "Keep changes?" dialog), saves `monitors.xml` only when kept, and puts
//! the configuration from before it back otherwise.

mod bus;
mod plan;
mod state;

use std::collections::HashMap;
use std::ffi::OsString;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::time::Duration;

use dsplr_model::Layout;
use zbus::Address;
use zbus::export::futures_core::Stream;
use zbus::proxy::CacheProperties;

use bus::{DisplayConfigProxy, MonitorsChangedStream, SERVICE};
pub use plan::{Method, PlanError};
pub use state::SnapshotError;
use state::State;

/// The longest one read, test or apply takes, connecting included: the
/// same as the Wayland backends'.
pub const OPERATION_TIMEOUT: Duration = crate::wayland::OPERATION_TIMEOUT;

/// How Mutter words a request whose serial is not its current one, a
/// D-Bus `AccessDenied` (`meta_monitor_manager_handle_apply_monitors_config`
/// in Mutter 51.1, not translated). The same error name also carries a
/// policy refusal, so the text tells them apart.
const STALE: &str = "based on stale information";

/// What went wrong talking to Mutter.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "no D-Bus session bus here: neither DBUS_SESSION_BUS_ADDRESS nor XDG_RUNTIME_DIR is set"
    )]
    NoBus,
    #[error("DBUS_SESSION_BUS_ADDRESS {text:?} is not a D-Bus address dsplr can use{}", error.as_ref().map(|error| format!(": {error}")).unwrap_or_default())]
    Address {
        text: String,
        /// zbus's reason, for an address that is text.
        error: Option<zbus::Error>,
    },
    #[error("the session bus: {0}")]
    Bus(#[from] zbus::Error),
    #[error("nothing owns {SERVICE} on the session bus, so this is not GNOME")]
    NotRunning,
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("the layout cannot be sent: {0}")]
    Plan(#[from] PlanError),
    /// Mutter refused the configuration, and changed nothing: one it cannot
    /// apply, or D-Bus configuration turned off by policy.
    #[error("Mutter refused the configuration: {0}")]
    Refused(String),
    /// The displays changed between reading them and the request, which
    /// Mutter dropped unapplied.
    #[error("Mutter dropped the configuration, as the displays changed since they were read: {0}")]
    Stale(String),
    #[error("the session bus stopped sending Mutter's changes")]
    Ended,
    #[error("Mutter did not {what} in time")]
    Timeout { what: &'static str },
}

impl Error {
    /// Mutter's answer to `ApplyMonitorsConfig`, told apart: a stale serial,
    /// a refusal (`InvalidArgs` for a configuration it cannot apply,
    /// `AccessDenied` for D-Bus configuration turned off), or anything else
    /// the bus said.
    #[must_use]
    pub fn from_apply(error: zbus::Error) -> Self {
        use zbus::fdo::Error as Fdo;
        match error {
            zbus::Error::FDO(ref fdo) => match **fdo {
                Fdo::AccessDenied(ref message) if message.contains(STALE) => {
                    Self::Stale(message.clone())
                }
                Fdo::AccessDenied(ref message) | Fdo::InvalidArgs(ref message) => {
                    Self::Refused(message.clone())
                }
                _ => Self::Bus(error),
            },
            other => Self::Bus(other),
        }
    }
}

/// Mutter on the session bus: where the bus is, and what Mutter said it
/// can do when it was found.
#[derive(Clone, Debug)]
pub struct Gnome {
    address: Address,
    global_scale_required: bool,
}

impl Gnome {
    /// The Mutter of this process's session, found from its environment.
    pub async fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name)).await
    }

    /// [`Gnome::from_env`], reading the variables through `var`: connects to
    /// the session bus they name, checks that something owns
    /// `org.gnome.Mutter.DisplayConfig` there (asking the bus, so nothing is
    /// started on demand), and reads its state, which is how detection
    /// checks that Mutter is there and answers.
    pub async fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        let address = bus::session_address(var)?;
        bounded("describe the displays", async {
            let connection = connect(&address).await?;
            let owned = zbus::fdo::DBusProxy::new(&connection)
                .await?
                .name_has_owner(SERVICE.try_into()?)
                .await
                .map_err(zbus::Error::from)?;
            if !owned {
                return Err(Error::NotRunning);
            }
            let state = read(&display_config(&connection).await?).await?;
            Ok(Self {
                global_scale_required: state.global_scale_required,
                address,
            })
        })
        .await
    }

    /// The session bus Mutter is on.
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// Whether Mutter wants one scale on every output.
    pub const fn global_scale_required(&self) -> bool {
        self.global_scale_required
    }

    /// The current state as a [`Layout`].
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        bounded("describe the displays", async {
            let proxy = self.proxy().await?;
            Ok(read(&proxy).await?.layout()?)
        })
        .await
    }

    /// Asks Mutter whether it would take `layout` (`verify`), changing
    /// nothing.
    pub async fn test(&self, layout: &Layout) -> Result<(), Error> {
        bounded("check the layout", self.send(layout, Method::Verify)).await
    }

    /// Sets the displays to `layout` (`temporary`): not saved, and never
    /// put back by Mutter. Read the state back to see what it shows.
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        bounded("apply the layout", self.send(layout, Method::Temporary)).await
    }

    /// Sends `layout` with `persistent`: Mutter applies it, then has GNOME
    /// Shell ask whether to keep it, and saves `monitors.xml` only once it is
    /// kept. Returns as soon as Mutter took the request, before anyone has
    /// answered.
    pub async fn persist(&self, layout: &Layout) -> Result<(), Error> {
        bounded("save the layout", self.send(layout, Method::Persistent)).await
    }

    /// A stream of Mutter's `MonitorsChanged`, on a connection of its own.
    pub async fn watch(&self) -> Result<Watch, Error> {
        bounded("follow the displays", async {
            let proxy = self.proxy().await?;
            Ok(Watch {
                changes: proxy.receive_monitors_changed().await?,
            })
        })
        .await
    }

    /// `layout` sent with `method` against the state read just before, on
    /// one connection, so the serial is the one the layout was planned on.
    async fn send(&self, layout: &Layout, method: Method) -> Result<(), Error> {
        let proxy = self.proxy().await?;
        let state = read(&proxy).await?;
        let current = state.layout()?;
        let config = plan::plan(&state, &current, layout)?;
        let mut properties = HashMap::new();
        if let Some(layout_mode) = config.layout_mode {
            properties.insert("layout-mode", zbus::Variant::from(layout_mode));
        }
        proxy
            .apply_monitors_config(
                state.serial,
                method as u32,
                &config.logical_monitors,
                &properties,
            )
            .await
            .map_err(Error::from_apply)
    }

    async fn proxy(&self) -> Result<DisplayConfigProxy<'static>, Error> {
        display_config(&connect(&self.address).await?).await
    }
}

/// Following Mutter's changes ([`Gnome::watch`]).
pub struct Watch {
    changes: MonitorsChangedStream,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").finish_non_exhaustive()
    }
}

impl Watch {
    /// Waits until Mutter says its monitors changed. Cancel-safe.
    pub async fn changed(&mut self) -> Result<(), Error> {
        poll_fn(|cx| Pin::new(&mut self.changes).poll_next(cx))
            .await
            .map(drop)
            .ok_or(Error::Ended)
    }
}

/// A new connection to the bus at `address`.
async fn connect(address: &Address) -> Result<zbus::Connection, Error> {
    Ok(zbus::connection::Builder::address(address.clone())
        .build()
        .await?)
}

/// Mutter's `DisplayConfig` on `connection`. Its properties are not read
/// or followed: dsplr uses none of them.
async fn display_config(
    connection: &zbus::Connection,
) -> Result<DisplayConfigProxy<'static>, Error> {
    Ok(DisplayConfigProxy::builder(connection)
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}

/// Mutter's current state.
async fn read(proxy: &DisplayConfigProxy<'_>) -> Result<State, Error> {
    Ok(State::from_wire(proxy.get_current_state().await?)?)
}

/// `operation` cut off at [`OPERATION_TIMEOUT`].
async fn bounded<T>(
    what: &'static str,
    operation: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| Error::Timeout { what })?
}

#[cfg(test)]
mod tests;
