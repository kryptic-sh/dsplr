//! `dsplr watch`: the hotplug profile daemon
//! (`docs/plan/05-persistence-and-profiles.md`, "Hotplug profiles").
//!
//! It follows the desktop's changes ([`Backend::watch`]) and, once they have
//! been quiet for [`QUIET`], reads the displays. Only a change in which
//! outputs are connected ([`Connected`]) restores a profile, picked as
//! `dsplr restore` picks one; a mode, position or on/off change, its own
//! apply's included, leaves the set as it was and does nothing, so it never
//! answers its own apply. It runs unattended, so a restore is kept without
//! asking, as `dsplr restore --yes` keeps one, with the revert point and its
//! watchdog as every change has. While another change waits to be kept (a
//! `dsplr apply` at its prompt, the GUI's confirm dialog), it does not
//! restore, which would replace that change's revert point and disarm its
//! watchdog: it looks again every [`BUSY_RETRY`] until that change is kept
//! or reverted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use dsplr_backend::{Backend, Watch};
use dsplr_model::{Layout, OutputId};
use dsplr_persist::profile::unix_ms;
use dsplr_persist::revert::{self, RevertPoint};
use tokio::time::Instant;

use crate::{Exit, apply, find_backend, profile, restore};

/// How long the desktop's changes must stop before the displays are read:
/// a dock attaches its outputs one after another, and each is a change.
pub const QUIET: Duration = Duration::from_secs(1);

/// The longest the displays are left unread after the first of a run of
/// changes, however often the desktop keeps reporting more.
pub const QUIET_MAX: Duration = Duration::from_secs(5);

/// How often the displays are read again while another change waits to be
/// kept.
pub const BUSY_RETRY: Duration = Duration::from_secs(2);

/// How long a watch must have stayed up, if it reported no change, for its
/// end to count as the desktop going away rather than a watch that cannot
/// stay open: then the tries to reach the desktop again start afresh.
const STAYED_UP: Duration = Duration::from_secs(60);

/// The wait before the first try to reach the desktop again after the
/// watch ends; each failed try doubles it, up to [`BACKOFF_MAX`].
pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);

/// The longest wait between tries to reach the desktop again.
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// How many tries in a row to reach the desktop again may fail before
/// `dsplr watch` gives up: the desktop is gone, not restarting.
const RECONNECT_TRIES: u32 = 10;

/// Which outputs are connected, on or off: a restore follows a change in
/// this, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connected(HashSet<(Option<String>, OutputId)>);

impl Connected {
    /// The outputs `layout`, a snapshot, shows: each by its connector and
    /// its identity, so a monitor moved to another port is a change too.
    pub fn of(layout: &Layout) -> Self {
        Self(
            layout
                .entries
                .iter()
                .map(|entry| (entry.connector().map(str::to_owned), entry.id.clone()))
                .collect(),
        )
    }
}

/// The connected outputs last handled, to tell a change in them from any
/// other change.
#[derive(Debug, Default)]
pub struct Tracker {
    last: Option<Connected>,
}

impl Tracker {
    /// Whether `now` differs from the outputs last handled (or none were
    /// yet).
    pub fn is_new(&self, now: &Connected) -> bool {
        self.last.as_ref() != Some(now)
    }

    /// `now` is handled: the same outputs read again are no change.
    pub fn handled(&mut self, now: Connected) {
        self.last = Some(now);
    }

    /// Forgets the outputs last seen, so the next ones read count as a
    /// change.
    pub fn forget(&mut self) {
        self.last = None;
    }
}

/// When to read the displays: [`QUIET`] after the last change reported, but
/// no later than [`QUIET_MAX`] after the first one not yet read.
#[derive(Debug, Default)]
pub struct Debounce {
    first: Option<Instant>,
    due: Option<Instant>,
}

impl Debounce {
    /// A read due at `due`, with no change reported yet.
    pub const fn due_at(due: Instant) -> Self {
        Self {
            first: Some(due),
            due: Some(due),
        }
    }

    /// A change reported at `now`: the read waits until it has been quiet
    /// for [`QUIET`], or until [`QUIET_MAX`] after the first change.
    pub fn changed(&mut self, now: Instant) {
        let first = *self.first.get_or_insert(now);
        self.due = Some((now + QUIET).min(first + QUIET_MAX));
    }

    /// When the displays are to be read, if a change is waiting.
    pub const fn due(&self) -> Option<Instant> {
        self.due
    }

    /// Whether the read is due at `now`; once it is, the change is taken
    /// and the next waits for a change of its own.
    pub fn take(&mut self, now: Instant) -> bool {
        let due = self.due.is_some_and(|due| due <= now);
        if due {
            *self = Self::default();
        }
        due
    }
}

/// The waits between tries to reach the desktop again: doubling from
/// [`BACKOFF_FIRST`] up to [`BACKOFF_MAX`], and none past
/// [`RECONNECT_TRIES`]. A watch that opens and ends at once does not start
/// them afresh ([`Backoff::ended`]), so one that cannot stay open is given
/// up on rather than reopened forever.
#[derive(Debug, Default)]
pub struct Backoff {
    tries: u32,
}

impl Backoff {
    /// The wait before the next try, or `None` when the tries are spent.
    pub fn next(&mut self) -> Option<Duration> {
        if self.tries >= RECONNECT_TRIES {
            return None;
        }
        let wait = BACKOFF_FIRST
            .saturating_mul(1 << self.tries.min(u32::BITS - 1))
            .min(BACKOFF_MAX);
        self.tries += 1;
        Some(wait)
    }

    /// A watch ended after `lasted`, having reported a change (`delivered`)
    /// or not: if it did, or stayed up for [`STAYED_UP`], it worked, and the
    /// next wait is the first again; otherwise the tries go on counting.
    pub fn ended(&mut self, lasted: Duration, delivered: bool) {
        if delivered || lasted >= STAYED_UP {
            self.tries = 0;
        }
    }
}

/// `dsplr watch`. Runs until SIGINT or SIGTERM ([`Exit::Success`]), or
/// until the desktop cannot be reached: at the start, at once; after the
/// watch ends, once [`Backoff`]'s tries are spent ([`Exit::Failure`]). On
/// KDE and GNOME, which restore their own layouts, only with `force`,
/// checked each time the desktop is reached, as it may be another desktop
/// after a restart.
pub async fn run(force: bool, confirm_timeout: Duration) -> Exit {
    let mut stop = match Stop::new() {
        Ok(stop) => stop,
        Err(error) => {
            eprintln!("dsplr: cannot listen for SIGINT and SIGTERM: {error}");
            return Exit::Failure;
        }
    };
    let dir = match profile::store_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("dsplr: {error}");
            return Exit::Failure;
        }
    };
    let revert_path = match revert::default_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("dsplr: {error}");
            return Exit::Failure;
        }
    };
    // Unattended: kept without asking, as `dsplr restore --yes` keeps one.
    let options = apply::Options {
        yes: true,
        dry_run: false,
        confirm_timeout,
    };
    let mut backoff = Backoff::default();
    let mut tracker = Tracker::default();
    let mut started = false;
    loop {
        match connect().await {
            Ok((backend, watch)) => {
                if backend.restores_on_hotplug() && !force {
                    eprintln!(
                        "dsplr: {} restores its own saved layouts when outputs are plugged in, and dsplr watch would fight it; --force runs it anyway",
                        backend.name()
                    );
                    return Exit::Refused;
                }
                if started {
                    eprintln!("dsplr: following the displays again");
                } else {
                    eprintln!(
                        "dsplr: following the displays on {}, restoring profiles from {}",
                        backend.name(),
                        dir.display()
                    );
                }
                started = true;
                // The desktop may have started afresh, with its own layout:
                // the outputs are checked as at the start.
                tracker.forget();
                let watching = Watching {
                    backend: &backend,
                    dir: &dir,
                    revert_path: &revert_path,
                    options: &options,
                };
                let opened = Instant::now();
                match watching.follow(watch, &mut tracker, &mut stop).await {
                    Ended::Stopped(signal) => {
                        eprintln!("dsplr: {signal}; stopped");
                        return Exit::Success;
                    }
                    Ended::Dropped { delivered } => backoff.ended(opened.elapsed(), delivered),
                }
            }
            Err(exit) if !started => return exit,
            Err(_) => {}
        }
        let Some(wait) = backoff.next() else {
            eprintln!("dsplr: cannot reach the desktop again; stopped");
            return Exit::Failure;
        };
        eprintln!(
            "dsplr: trying to reach the desktop again in {} s",
            wait.as_secs()
        );
        tokio::select! {
            signal = stop.wait() => {
                eprintln!("dsplr: {signal}; stopped");
                return Exit::Success;
            }
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// The desktop's backend and a watch on it, or, when either cannot be had,
/// the error reported and the exit code.
async fn connect() -> Result<(Backend, Watch), Exit> {
    let backend = find_backend().await?;
    match backend.watch().await {
        Ok(watch) => Ok((backend, watch)),
        Err(error) => {
            eprintln!(
                "dsplr: cannot follow the displays' changes on {}: {error}",
                backend.name()
            );
            Err(Exit::Failure)
        }
    }
}

/// Why one watch's following ended.
enum Ended {
    /// A signal, by its name.
    Stopped(&'static str),
    /// The watch ended, having reported a change or not.
    Dropped { delivered: bool },
}

/// Whether a check restored or left the outputs as they are, or found
/// another change waiting to be kept and is to look again.
#[derive(Debug, PartialEq, Eq)]
enum Checked {
    Done,
    Busy,
}

/// Whether `point`, the revert point now, is another change's that still
/// waits to be kept at `now_unix_ms`: its watchdog's time not yet come, or
/// come less than `revert` (the longest that watchdog's own revert takes)
/// ago, as a watchdog reverting is a change in flight too.
pub fn busy(point: Option<&RevertPoint>, now_unix_ms: u64, revert: Duration) -> bool {
    let revert_ms = u64::try_from(revert.as_millis()).unwrap_or(u64::MAX);
    point.is_some_and(|point| point.waiting(now_unix_ms.saturating_sub(revert_ms)))
}

/// One connection's worth of following the desktop.
struct Watching<'a> {
    backend: &'a Backend,
    dir: &'a Path,
    revert_path: &'a PathBuf,
    options: &'a apply::Options,
}

impl Watching<'_> {
    /// Follows `watch`, restoring a profile each time the connected outputs
    /// change, the first time it reads them included, until a signal or the
    /// watch's end.
    async fn follow(&self, mut watch: Watch, tracker: &mut Tracker, stop: &mut Stop) -> Ended {
        // The outputs as they are when it starts, without waiting for a
        // change.
        let mut debounce = Debounce::due_at(Instant::now());
        let mut delivered = false;
        loop {
            let due = debounce.due();
            tokio::select! {
                signal = stop.wait() => return Ended::Stopped(signal),
                changed = watch.changed() => match changed {
                    Ok(()) => {
                        delivered = true;
                        debounce.changed(Instant::now());
                    }
                    Err(error) => {
                        eprintln!("dsplr: stopped following the displays' changes: {error}");
                        return Ended::Dropped { delivered };
                    }
                },
                () = sleep_until(due), if due.is_some() => {
                    if debounce.take(Instant::now()) && self.check(tracker).await == Checked::Busy {
                        debounce = Debounce::due_at(Instant::now() + BUSY_RETRY);
                    }
                }
            }
        }
    }

    /// Whether another change waits to be kept ([`busy`]). A revert point
    /// that cannot be read is said, and is no change in flight: no flow
    /// could keep or revert it either.
    fn busy(&self) -> bool {
        let point = revert::load(self.revert_path).unwrap_or_else(|error| {
            eprintln!("dsplr: {error}");
            None
        });
        busy(
            point.as_ref(),
            unix_ms(SystemTime::now()),
            restore::change_limit(self.backend.request_timeout()),
        )
    }

    /// Reads the displays and, when the connected outputs are not the ones
    /// last handled, restores the profile picked for them (the same ones are
    /// said, and left as they are), unless another
    /// change waits to be kept ([`Checked::Busy`]; the outputs are then not
    /// handled yet). Nothing picked, or a restore that fails, is said, and
    /// the next change is waited for.
    async fn check(&self, tracker: &mut Tracker) -> Checked {
        let current = match self.backend.snapshot().await {
            Ok(layout) => layout,
            Err(error) => {
                eprintln!(
                    "dsplr: cannot read the displays from {}: {error}",
                    self.backend.name()
                );
                return Checked::Done;
            }
        };
        let connected = Connected::of(&current);
        if !tracker.is_new(&connected) {
            eprintln!("dsplr: the displays changed; the same outputs are connected");
            return Checked::Done;
        }
        if self.busy() {
            eprintln!(
                "dsplr: the outputs changed while another change waits to be kept; looking again in {} s",
                BUSY_RETRY.as_secs()
            );
            return Checked::Busy;
        }
        tracker.handled(connected);
        eprintln!(
            "dsplr: the outputs connected now: {}",
            profile::outputs(&current)
        );
        // Read afresh each time: profiles saved since are picked too.
        let Ok(profiles) = profile::readable(self.dir) else {
            return Checked::Done;
        };
        let Some((picked, fit)) = profile::picked(&profiles, &current) else {
            return Checked::Done;
        };
        let capabilities = self.backend.capabilities();
        if apply::bind(&picked.layout, &current)
            .is_ok_and(|asked| asked.differences(&current, &capabilities).is_empty())
        {
            eprintln!(
                "dsplr: profile \"{}\" is on screen already",
                picked.name.as_str()
            );
            return Checked::Done;
        }
        eprintln!("dsplr: {}", profile::restoring(picked, fit));
        let exit = apply::onto(self.backend, current, &picked.layout, self.options).await;
        if exit != Exit::Success {
            eprintln!(
                "dsplr: profile \"{}\" was not restored (exit {}); waiting for the next change",
                picked.name.as_str(),
                exit as u8
            );
        }
        Checked::Done
    }
}

/// Sleeps until `due`; with none, never.
async fn sleep_until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

/// SIGINT and SIGTERM, listened for from the start, so one sent while a
/// restore runs is taken once it has finished (the revert point then never
/// outlives a change that was kept).
#[cfg(unix)]
struct Stop {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Stop {
    fn new() -> std::io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    /// The next signal's name, once it comes. Cancel-safe.
    async fn wait(&mut self) -> &'static str {
        tokio::select! {
            _ = self.interrupt.recv() => "SIGINT",
            _ = self.terminate.recv() => "SIGTERM",
        }
    }
}

/// Off Unix there are no SIGINT and SIGTERM to stop on, so `dsplr watch`
/// refuses to start rather than run unstoppable. Every backend is
/// Linux-only, so it could not follow a desktop there either.
#[cfg(not(unix))]
struct Stop;

#[cfg(not(unix))]
impl Stop {
    fn new() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "dsplr watch runs on Unix only",
        ))
    }

    async fn wait(&mut self) -> &'static str {
        std::future::pending().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dsplr_model::{Layout, Mode, OutputEntry, OutputId, OutputState};
    use tokio::time::Instant;

    use dsplr_persist::revert::RevertPoint;

    use super::{
        BACKOFF_FIRST, BACKOFF_MAX, Backoff, Connected, Debounce, QUIET, QUIET_MAX,
        RECONNECT_TRIES, STAYED_UP, Tracker, busy,
    };

    fn entry(connector: &str, serial: &str) -> OutputEntry {
        let mut entry = OutputEntry {
            id: OutputId::new("AOC", "Q27G4", serial, connector),
            output: None,
            state: OutputState {
                mode: Some(Mode::new(2560, 1440, 60_000)),
                ..OutputState::default()
            },
        };
        entry.output = Some(dsplr_model::Output {
            connector: connector.into(),
            description: format!("AOC Q27G4 {serial}"),
            physical_size_mm: None,
            make: "AOC".into(),
            model: "Q27G4".into(),
            serial: serial.into(),
            modes: vec![Mode::new(2560, 1440, 60_000)],
        });
        entry
    }

    fn layout(entries: Vec<OutputEntry>) -> Layout {
        Layout {
            entries,
            ..Layout::default()
        }
    }

    /// The fake events: what the watch reports between reads, each a
    /// snapshot, and whether each restores.
    #[test]
    fn only_a_change_in_the_connected_outputs_restores() {
        let laptop = entry("eDP-1", "");
        let desk = entry("DP-1", "AAA");
        let mut tracker = Tracker::default();
        // As a check does when it restores: a new set is handled.
        let mut seen = |layout: &Layout| {
            let connected = Connected::of(layout);
            let new = tracker.is_new(&connected);
            tracker.handled(connected);
            new
        };

        assert!(seen(&layout(vec![laptop.clone()])), "the first read");
        assert!(!seen(&layout(vec![laptop.clone()])), "nothing changed");

        // Plugged in: a change; listed in another order: none.
        assert!(seen(&layout(vec![laptop.clone(), desk.clone()])));
        assert!(!seen(&layout(vec![desk.clone(), laptop.clone()])));

        // dsplr's own apply, or the user's, moves, rescales, switches off
        // and mirrors: the same outputs, so no restore, and so no loop.
        let mut applied = layout(vec![laptop.clone(), desk.clone()]);
        applied.entries[1].state.x = 1920;
        applied.entries[1].state.scale = 1.5;
        applied.entries[1].state.mode = Some(Mode::new(1920, 1080, 60_000));
        applied.entries[0].state.enabled = false;
        assert!(!seen(&applied));
        applied.entries[0].state.mirror_of = Some(desk.id.clone());
        assert!(!seen(&applied));

        // The monitor moved to another port: a change.
        let moved = entry("HDMI-A-1", "AAA");
        assert!(seen(&layout(vec![laptop.clone(), moved])));
        // Unplugged: a change, and plugged back: another.
        assert!(seen(&layout(vec![laptop.clone()])));
        assert!(seen(&layout(vec![laptop.clone(), desk.clone()])));
        // Another monitor of the same model on the same port: a change.
        assert!(seen(&layout(vec![laptop, entry("DP-1", "BBB")])));
    }

    #[test]
    fn a_set_not_handled_or_forgotten_restores_again() {
        let mut tracker = Tracker::default();
        let one = Connected::of(&layout(vec![entry("eDP-1", "")]));
        assert!(tracker.is_new(&one));
        // Busy: not handled, so the next read is still a change.
        assert!(tracker.is_new(&one));
        tracker.handled(one.clone());
        assert!(!tracker.is_new(&one));
        tracker.forget();
        assert!(tracker.is_new(&one), "as after the desktop restarted");
    }

    /// A dock's outputs arriving one after another are one read, once they
    /// have been quiet for [`QUIET`].
    #[test]
    fn a_burst_of_changes_is_read_once_after_it_is_quiet() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut debounce = Debounce::default();
        assert_eq!(debounce.due(), None);
        assert!(!debounce.take(at(10_000)), "nothing reported");

        for ms in [0, 300, 700] {
            debounce.changed(at(ms));
        }
        let quiet = u64::try_from(QUIET.as_millis()).unwrap();
        assert_eq!(debounce.due(), Some(at(700 + quiet)));
        assert!(!debounce.take(at(700 + quiet - 1)), "not quiet long enough");
        assert!(debounce.take(at(700 + quiet)));
        assert_eq!(debounce.due(), None, "taken once");
        assert!(!debounce.take(at(700 + 2 * quiet)));

        // A change during a read's wait pushes it back.
        debounce.changed(at(5_000));
        debounce.changed(at(5_000 + quiet - 1));
        assert!(!debounce.take(at(5_000 + quiet)));
        assert!(debounce.take(at(5_000 + 2 * quiet - 1)));

        // The read at the start is due at once, and a change before it is
        // taken waits for quiet like any other.
        let mut debounce = Debounce::due_at(at(0));
        assert!(debounce.take(at(0)));
        let mut debounce = Debounce::due_at(at(0));
        debounce.changed(at(0));
        assert!(!debounce.take(at(0)));
        assert!(debounce.take(at(quiet)));
    }

    /// A desktop that never stops reporting changes is still read,
    /// [`QUIET_MAX`] after the first, and then again after the next.
    #[test]
    fn changes_that_never_stop_are_read_at_the_cap() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let quiet = u64::try_from(QUIET.as_millis()).unwrap();
        let max = u64::try_from(QUIET_MAX.as_millis()).unwrap();
        assert!(max > quiet);
        let mut debounce = Debounce::default();
        let mut reads = Vec::new();
        let every = quiet / 2;
        for ms in (0..3 * max).step_by(usize::try_from(every).unwrap()) {
            if debounce.take(at(ms)) {
                reads.push(ms);
            }
            debounce.changed(at(ms));
        }
        // The first run starts at 0; each read's next run starts with the
        // change reported right after it.
        assert_eq!(reads, [max, 2 * max], "every {every} ms");
    }

    #[test]
    fn reconnecting_backs_off_to_its_cap_then_gives_up() {
        let mut backoff = Backoff::default();
        let waits: Vec<Duration> = std::iter::from_fn(|| backoff.next()).collect();
        assert_eq!(waits.len(), RECONNECT_TRIES as usize);
        assert_eq!(waits[0], BACKOFF_FIRST);
        assert_eq!(waits[1], 2 * BACKOFF_FIRST);
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]), "{waits:?}");
        assert_eq!(waits.last(), Some(&BACKOFF_MAX));
        assert_eq!(backoff.next(), None);
        backoff.ended(Duration::ZERO, true);
        assert_eq!(backoff.next(), Some(BACKOFF_FIRST), "reached again");
    }

    /// The fake events: a watch that opens and ends at once, reporting
    /// nothing, every time it is reopened.
    #[test]
    fn a_watch_that_cannot_stay_open_is_given_up_on() {
        let mut backoff = Backoff::default();
        let mut reopened = 0;
        while backoff.next().is_some() {
            reopened += 1;
            backoff.ended(Duration::from_millis(5), false);
            assert!(reopened <= RECONNECT_TRIES, "reopened forever");
        }
        assert_eq!(reopened, RECONNECT_TRIES);
    }

    #[test]
    fn a_watch_that_worked_starts_the_tries_afresh() {
        for (lasted, delivered) in [(Duration::ZERO, true), (STAYED_UP, false)] {
            let mut backoff = Backoff::default();
            backoff.next();
            backoff.next();
            backoff.ended(lasted, delivered);
            assert_eq!(
                backoff.next(),
                Some(BACKOFF_FIRST),
                "{lasted:?} {delivered}"
            );
        }
        let mut backoff = Backoff::default();
        backoff.next();
        backoff.ended(STAYED_UP - Duration::from_millis(1), false);
        assert_eq!(
            backoff.next(),
            Some(2 * BACKOFF_FIRST),
            "not up long enough"
        );
    }

    /// Another change waiting to be kept holds the watch back until it is
    /// kept or its watchdog has had time to revert it.
    #[test]
    fn a_change_waiting_to_be_kept_is_busy() {
        let revert = Duration::from_secs(10);
        let guarded = RevertPoint::guarding(layout(vec![entry("eDP-1", "")]), 100_000);
        assert!(!busy(None, 0, revert), "no revert point");
        assert!(busy(Some(&guarded), 50_000, revert), "at its prompt");
        assert!(
            busy(Some(&guarded), 109_999, revert),
            "its watchdog reverting"
        );
        assert!(
            !busy(Some(&guarded), 110_000, revert),
            "long past its watchdog"
        );
        let kept = RevertPoint::new(guarded.layout.clone());
        assert!(!busy(Some(&kept), 50_000, revert), "kept");
    }
}
