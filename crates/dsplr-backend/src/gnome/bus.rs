//! Mutter's `org.gnome.Mutter.DisplayConfig` as D-Bus carries it: where the
//! session bus is, the proxy, and the shapes of `GetCurrentState`'s reply and
//! `ApplyMonitorsConfig`'s arguments, as Mutter 51.1's
//! `data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml` declares them.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use zbus::address::transport::{Transport, Unix, UnixSocket};
use zbus::{Address, OwnedVariant, Variant};

use super::Error;

/// The bus name Mutter owns, and the interface it serves there.
pub const SERVICE: &str = "org.gnome.Mutter.DisplayConfig";

/// An `a{sv}` of optional properties, as Mutter sends them.
pub type Properties = HashMap<String, OwnedVariant>;

/// A monitor's `(ssss)`: connector, vendor, product, serial.
pub type MonitorSpec = (String, String, String, String);

/// A mode's `(siiddada{sv})`: id, width, height, refresh rate in hertz,
/// preferred scale, supported scales, properties.
pub type WireMode = (String, i32, i32, f64, f64, Vec<f64>, Properties);

/// A monitor's `((ssss)a(siiddada{sv})a{sv})`.
pub type WireMonitor = (MonitorSpec, Vec<WireMode>, Properties);

/// A logical monitor's `(iiduba(ssss)a{sv})`: x, y, scale, transform,
/// primary, its monitors, properties.
pub type WireLogicalMonitor = (i32, i32, f64, u32, bool, Vec<MonitorSpec>, Properties);

/// `GetCurrentState`'s reply: serial, monitors, logical monitors,
/// properties.
pub type CurrentState = (u32, Vec<WireMonitor>, Vec<WireLogicalMonitor>, Properties);

/// A monitor of a logical monitor to apply, `(ssa{sv})`: connector, mode
/// id, properties.
pub type MonitorConfig = (String, String, Properties);

/// A logical monitor to apply, `(iiduba(ssa{sv}))`: x, y, scale, transform,
/// primary, its monitors.
pub type LogicalMonitorConfig = (i32, i32, f64, u32, bool, Vec<MonitorConfig>);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig"
)]
pub trait DisplayConfig {
    fn get_current_state(&self) -> zbus::Result<CurrentState>;

    fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: &[LogicalMonitorConfig],
        properties: &HashMap<&str, Variant<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn monitors_changed(&self) -> zbus::Result<()>;
}

/// The session bus, by libdbus's and GLib's rule: `DBUS_SESSION_BUS_ADDRESS`
/// when set and not empty, else the socket `bus` in `XDG_RUNTIME_DIR`
/// (which systemd's user bus listens on).
pub fn session_address(var: &impl Fn(&str) -> Option<OsString>) -> Result<Address, Error> {
    if let Some(text) = var("DBUS_SESSION_BUS_ADDRESS").filter(|text| !text.is_empty()) {
        let text = text.into_string().map_err(|text| Error::Address {
            text: text.to_string_lossy().into_owned(),
            error: None,
        })?;
        return text.parse().map_err(|error| Error::Address {
            text: text.clone(),
            error: Some(error),
        });
    }
    let runtime = var("XDG_RUNTIME_DIR").ok_or(Error::NoBus)?;
    Ok(Address::new(Transport::Unix(Unix::new(UnixSocket::File(
        Path::new(&runtime).join("bus"),
    )))))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use zbus::address::transport::{Transport, UnixSocket};
    use zbus::{Type, Variant};

    use super::{CurrentState, LogicalMonitorConfig, session_address};
    use crate::gnome::Error;
    use crate::wayland::tests::vars;

    #[test]
    fn the_wire_types_are_the_signatures_mutter_declares() {
        // `GetCurrentState`'s out arguments and `ApplyMonitorsConfig`'s in
        // arguments in Mutter 51.1's interface XML, each list as one struct.
        assert_eq!(
            CurrentState::SIGNATURE.to_string(),
            "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})"
        );
        type Apply<'a> = (
            u32,
            u32,
            Vec<LogicalMonitorConfig>,
            HashMap<&'a str, Variant<'a>>,
        );
        assert_eq!(Apply::SIGNATURE.to_string(), "(uua(iiduba(ssa{sv}))a{sv})");
    }

    #[test]
    fn the_session_bus_is_the_variable_s_or_the_runtime_dir_s() {
        let named = session_address(&vars(&[
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/tmp/b/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]))
        .expect("an address");
        let path = |address: &zbus::Address| match address.transport() {
            Transport::Unix(unix) => match unix.path() {
                UnixSocket::File(path) => path.clone(),
                other => panic!("not a file socket: {other:?}"),
            },
            other => panic!("not a unix socket: {other:?}"),
        };
        assert_eq!(path(&named), std::path::Path::new("/tmp/b/bus"));

        let fallback = session_address(&vars(&[
            ("DBUS_SESSION_BUS_ADDRESS", ""),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]))
        .expect("an address");
        assert_eq!(path(&fallback), std::path::Path::new("/run/user/1000/bus"));

        assert!(matches!(session_address(&vars(&[])), Err(Error::NoBus)));
        assert!(matches!(
            session_address(&vars(&[("DBUS_SESSION_BUS_ADDRESS", "nonsense")])),
            Err(Error::Address { text, error: Some(_) }) if text == "nonsense"
        ));
    }
}
