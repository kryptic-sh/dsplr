//! The GNOME backend against a stand-in for Mutter's `DisplayConfig` on a
//! private `dbus-daemon`: the requests dsplr sends as D-Bus carries them,
//! and Mutter's answers as it words them. What a real Mutter does with them
//! is `mutter_e2e`'s, under `tests/e2e/headless-mutter.sh`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dsplr_model::Layout;
use zbus::fdo;
use zbus::object_server::SignalEmitter;

use super::bus::{CurrentState, LogicalMonitorConfig, SERVICE};
use super::state::tests::two_monitors;
use super::{Error, Gnome};
use crate::wayland::tests::vars;

/// A `dbus-daemon` of the test's own, stopped when dropped.
struct Bus {
    daemon: Child,
    address: String,
}

impl Bus {
    /// Starts one with the session bus's configuration.
    ///
    /// # Panics
    ///
    /// When `dbus-daemon` is not installed or does not say where it
    /// listens: the test cannot run, and passing would hide that.
    fn start() -> Self {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon runs (these tests need it installed)");
        let stdout = daemon.stdout.take().expect("piped stdout");
        let mut address = String::new();
        BufReader::new(stdout)
            .read_line(&mut address)
            .expect("dbus-daemon prints its address");
        let address = address.trim().to_owned();
        assert!(!address.is_empty(), "dbus-daemon printed no address");
        Self { daemon, address }
    }

    /// The backend, found on this bus as detection finds it.
    async fn gnome(&self) -> Result<Gnome, Error> {
        Gnome::from_vars(vars(&[("DBUS_SESSION_BUS_ADDRESS", &self.address)])).await
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        // Already gone is fine: there is nothing left to stop.
        let _gone = self.daemon.kill();
        let _reaped = self.daemon.wait();
    }
}

/// One `ApplyMonitorsConfig` the stand-in was sent.
#[derive(Debug)]
struct Applied {
    serial: u32,
    method: u32,
    logical_monitors: Vec<LogicalMonitorConfig>,
    layout_mode: Option<u32>,
}

/// How the stand-in answers the next `ApplyMonitorsConfig`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Take,
    /// As Mutter answers a configuration it cannot apply.
    Refuse,
    /// As Mutter answers a serial that is not its current one: the
    /// displays changed after `GetCurrentState` was answered.
    Stale,
}

#[derive(Debug)]
struct Seen {
    serial: u32,
    applied: Vec<Applied>,
    answer: Answer,
}

/// Mutter's `DisplayConfig` as far as dsplr uses it: `two_monitors` as the
/// state, a serial that a change moves on, and `MonitorsChanged` after it.
struct Mutter(Arc<Mutex<Seen>>);

#[zbus::interface(name = "org.gnome.Mutter.DisplayConfig")]
impl Mutter {
    #[zbus(out_args("serial", "monitors", "logical_monitors", "properties"))]
    fn get_current_state(&self) -> CurrentState {
        let (_, monitors, logical_monitors, properties) = two_monitors();
        (
            self.0.lock().expect("unpoisoned").serial,
            monitors,
            logical_monitors,
            properties,
        )
    }

    async fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<LogicalMonitorConfig>,
        properties: HashMap<String, zbus::OwnedVariant>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        {
            let mut seen = self.0.lock().expect("unpoisoned");
            match seen.answer {
                // Mutter's own words, from Mutter 51.1's
                // `meta_monitor_manager_handle_apply_monitors_config`.
                Answer::Stale => {
                    return Err(fdo::Error::AccessDenied(
                        "The requested configuration is based on stale information".into(),
                    ));
                }
                Answer::Refuse => {
                    return Err(fdo::Error::InvalidArgs(
                        "Logical monitors not adjacent".into(),
                    ));
                }
                Answer::Take => {}
            }
            seen.applied.push(Applied {
                serial,
                method,
                logical_monitors,
                layout_mode: properties
                    .get("layout-mode")
                    .and_then(|value| u32::try_from(value).ok()),
            });
            if method == 0 {
                return Ok(());
            }
            seen.serial += 1;
        }
        Self::monitors_changed(&emitter)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    #[zbus(signal)]
    async fn monitors_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// A bus with the stand-in on it, as Mutter: what it was sent, and the
/// connection that keeps it there.
async fn mutter(bus: &Bus) -> (Arc<Mutex<Seen>>, zbus::Connection) {
    let seen = Arc::new(Mutex::new(Seen {
        serial: 7,
        applied: Vec::new(),
        answer: Answer::Take,
    }));
    let connection = zbus::connection::Builder::address(bus.address.as_str())
        .name(SERVICE)
        .serve_at("/org/gnome/Mutter/DisplayConfig", Mutter(Arc::clone(&seen)))
        .build()
        .await
        .expect("the stand-in is on the bus");
    (seen, connection)
}

#[tokio::test]
async fn a_bus_without_mutter_is_not_gnome() {
    let bus = Bus::start();
    assert!(matches!(bus.gnome().await, Err(Error::NotRunning)));
}

#[tokio::test]
async fn the_state_is_read_and_each_request_sent_with_its_method_and_serial() {
    let bus = Bus::start();
    let (seen, _mutter) = mutter(&bus).await;
    let gnome = bus.gnome().await.expect("Mutter is found");
    assert!(!gnome.global_scale_required());

    let layout = gnome.snapshot().await.expect("a snapshot");
    assert_eq!(layout.entries.len(), 2);
    assert_eq!(layout.entries[1].state.x, 2048);

    gnome.test(&layout).await.expect("verified");
    gnome.apply(&layout).await.expect("applied");
    gnome.persist(&layout).await.expect("persisted");
    let seen = seen.lock().expect("unpoisoned");
    let sent: Vec<(u32, u32)> = seen
        .applied
        .iter()
        .map(|applied| (applied.method, applied.serial))
        .collect();
    // verify changes nothing, so the serial stays; each other moves it on.
    assert_eq!(sent, [(0, 7), (1, 7), (2, 8)]);
    let applied = &seen.applied[1];
    assert_eq!(applied.layout_mode, Some(1), "Mutter can change it");
    assert_eq!(applied.logical_monitors.len(), 2);
    let (x, y, _, transform, primary, monitors) = &applied.logical_monitors[1];
    assert_eq!((*x, *y, *transform, *primary), (2048, 0, 1, false));
    assert_eq!(monitors[0].0, "HDMI-1");
    assert_eq!(monitors[0].1, "1920x1080@60.000");
}

#[tokio::test]
async fn mutter_s_refusal_and_a_stale_serial_are_told_apart() {
    let bus = Bus::start();
    let (seen, _mutter) = mutter(&bus).await;
    let gnome = bus.gnome().await.expect("Mutter is found");
    let layout = gnome.snapshot().await.expect("a snapshot");

    seen.lock().expect("unpoisoned").answer = Answer::Refuse;
    let refused = crate::Error::from(gnome.apply(&layout).await.expect_err("refused"));
    assert!(refused.is_refusal(), "{refused:?}");
    assert!(
        refused
            .to_string()
            .contains("Logical monitors not adjacent"),
        "{refused}"
    );

    seen.lock().expect("unpoisoned").answer = Answer::Stale;
    let stale = crate::Error::from(gnome.apply(&layout).await.expect_err("stale"));
    assert!(stale.is_stale(), "{stale:?}");
    assert!(!stale.is_refusal(), "{stale:?}");

    // A layout dsplr cannot send never reaches Mutter.
    seen.lock().expect("unpoisoned").answer = Answer::Take;
    let mut offset: Layout = layout.clone();
    for entry in &mut offset.entries {
        entry.state.y = 5;
    }
    let before_sending = crate::Error::from(gnome.test(&offset).await.expect_err("refused"));
    assert!(
        before_sending.is_refused_before_sending(),
        "{before_sending:?}"
    );
    assert!(seen.lock().expect("unpoisoned").applied.is_empty());
}

#[tokio::test]
async fn a_watch_hears_mutter_s_monitors_changed() {
    let bus = Bus::start();
    let (_seen, _mutter) = mutter(&bus).await;
    let gnome = bus.gnome().await.expect("Mutter is found");
    let mut watch = gnome.watch().await.expect("a watch");
    let layout = gnome.snapshot().await.expect("a snapshot");

    // verify changes nothing, so Mutter says nothing.
    gnome.test(&layout).await.expect("verified");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), watch.changed())
            .await
            .is_err(),
        "no change after a verify"
    );
    gnome.apply(&layout).await.expect("applied");
    tokio::time::timeout(Duration::from_secs(5), watch.changed())
        .await
        .expect("a change after an apply")
        .expect("the watch is still there");
}

#[test]
fn mutter_s_answers_are_told_apart() {
    let apply = |error: fdo::Error| Error::from_apply(zbus::Error::FDO(Box::new(error)));
    assert!(matches!(
        apply(fdo::Error::AccessDenied(
            "The requested configuration is based on stale information".into()
        )),
        Error::Stale(_)
    ));
    assert!(matches!(
        apply(fdo::Error::AccessDenied(
            "Monitor configuration via D-Bus is disabled".into()
        )),
        Error::Refused(message) if message.contains("disabled")
    ));
    assert!(matches!(
        apply(fdo::Error::InvalidArgs("Logical monitors not adjacent".into())),
        Error::Refused(message) if message == "Logical monitors not adjacent"
    ));
    assert!(matches!(
        apply(fdo::Error::ServiceUnknown("gone".into())),
        Error::Bus(_)
    ));
}
