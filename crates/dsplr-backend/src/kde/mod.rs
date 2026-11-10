//! KDE Plasma 6 on Wayland, over KWin's own protocols
//! (`docs/plan/04-backends.md#kde-plasma-6-kde`): `kde_output_device_v2`
//! through `kde_output_device_registry_v2` to read, `kde_output_order_v1`
//! for the primary output, and `kde_output_management_v2` to apply.
//!
//! [`Kde`] finds KWin and does each read and apply on a connection of its
//! own; [`Connection`] is one such connection, and [`Device`] what it
//! reports about each output. Turning devices into dsplr's display model,
//! and a layout into a configuration, is plain data and tested without a
//! compositor.
//!
//! The management protocol has no test request, and KWin saves every
//! configuration it applies (`kwinoutputconfig.json`), so there is nothing
//! to test with and nothing left to save.

mod connection;
mod devices;
mod plan;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use dsplr_model::Layout;

pub use crate::wayland::heads::SnapshotError;
pub use crate::wayland::plan::PlanError;
pub use crate::wayland::{OPERATION_TIMEOUT, REQUEST_TIMEOUT};
use crate::wayland::{bounded, socket_path};
pub use connection::{Connection, REGISTRY_SINCE};
pub use devices::Device;
pub use plan::REPLICATION_SINCE;

/// What went wrong talking to KWin.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Wayland(#[from] crate::wayland::Error),
    #[error("the compositor offers no {0}")]
    Missing(&'static str),
    #[error("KWin offers {interface} version {offered}, and dsplr needs {needed} or later")]
    TooOld {
        interface: &'static str,
        offered: u32,
        needed: u32,
    },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("the layout cannot be sent: {0}")]
    Plan(#[from] PlanError),
    #[error("KWin refused the configuration{}", reason.as_ref().map(|reason| format!(": {reason}")).unwrap_or_default())]
    Failed {
        /// Why, when KWin said (its `failure_reason`, since version 12).
        reason: Option<String>,
    },
}

/// KWin with its output management: where its socket is, and what the
/// versions it offers allow.
#[derive(Clone, Debug)]
pub struct Kde {
    socket: PathBuf,
    management_version: u32,
    has_order: bool,
}

impl Kde {
    /// The KWin this process runs under, found from its environment.
    pub async fn from_env() -> Result<Self, Error> {
        Self::from_vars(|name| std::env::var_os(name)).await
    }

    /// [`Kde::from_env`], reading the variables through `var`: connects to
    /// the socket `WAYLAND_DISPLAY` names and reads its outputs, which is how
    /// detection checks that KWin's output management is there and answers.
    pub async fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, Error> {
        let socket = socket_path(var)?;
        let connection = bounded("describe its outputs", Connection::open(&socket)).await?;
        Ok(Self {
            management_version: connection.management_version(),
            has_order: connection.has_order(),
            socket,
        })
    }

    /// KWin's Wayland socket.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The `kde_output_management_v2` version dsplr binds: KWin's, up to
    /// the newest dsplr speaks.
    pub const fn management_version(&self) -> u32 {
        self.management_version
    }

    /// Whether KWin offers `kde_output_order_v1`, through which dsplr reads
    /// the primary output.
    pub const fn has_order(&self) -> bool {
        self.has_order
    }

    /// A new connection, with its devices read.
    pub async fn connect(&self) -> Result<Connection, Error> {
        Connection::open(&self.socket).await
    }

    /// The current state as a [`Layout`].
    pub async fn snapshot(&self) -> Result<Layout, Error> {
        bounded("describe its outputs", async {
            Ok(self.connect().await?.layout()?)
        })
        .await
    }

    /// Whether `layout` can be sent to KWin as it is now, sending nothing:
    /// [`Error::Plan`] says why not. KWin has no test request, so this is as
    /// far as a check before applying goes.
    pub async fn check(&self, layout: &Layout) -> Result<(), Error> {
        bounded("describe its outputs", async {
            self.connect().await?.plan(layout).map(drop)
        })
        .await
    }

    /// Sets the outputs to `layout` in one configuration, read and sent on a
    /// fresh connection. KWin may round a setting, so read the state back to
    /// see what it shows.
    pub async fn apply(&self, layout: &Layout) -> Result<(), Error> {
        bounded("apply the layout", async {
            self.connect().await?.apply(layout).await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Kde};
    use crate::wayland::tests::vars;

    #[tokio::test]
    async fn a_socket_that_is_not_there_is_an_io_error_naming_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = dir.path().to_str().expect("UTF-8");
        let env = [
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", runtime),
        ];
        assert!(matches!(
            Kde::from_vars(vars(&env)).await,
            Err(Error::Wayland(crate::wayland::Error::Io { path, .. }))
                if path == dir.path().join("wayland-0")
        ));
        assert!(matches!(
            Kde::from_vars(vars(&[])).await,
            Err(Error::Wayland(crate::wayland::Error::NotRunning))
        ));
    }

    #[test]
    fn a_refusal_says_kwin_s_reason_when_it_gave_one() {
        let with = Error::Failed {
            reason: Some("Position of enabled output Virtual-0 is negative (-1, 0)".into()),
        };
        assert_eq!(
            with.to_string(),
            "KWin refused the configuration: Position of enabled output Virtual-0 is negative (-1, 0)"
        );
        assert_eq!(
            Error::Failed { reason: None }.to_string(),
            "KWin refused the configuration"
        );
    }
}
