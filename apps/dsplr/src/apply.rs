//! `dsplr apply <FILE>`: a whole layout from a file, through the confirm flow
//! (`docs/plan/02-architecture.md`, "Apply, confirm, revert"), which
//! `dsplr set` ([`run`]) and `dsplr restore` with a profile ([`onto`])
//! share.

use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_core::flow::{self, Answer, Change, Failure, Flow, Outcome, Step};
use dsplr_model::{Capabilities, Difference, Layout, OutputEntry, OutputId, assign};
use dsplr_persist::revert::{self, RevertPoint};

use crate::{Exit, restore};

/// How long a re-read waits for the desktop to show a layout it was sent:
/// Hyprland's `eval` replies before its rules take effect, on its next
/// frame.
pub const SETTLE: Duration = Duration::from_secs(5);

/// How often a re-read looks again while the desktop settles.
const SETTLE_POLL: Duration = Duration::from_millis(100);

/// The longest [`settle`] can take on a backend whose requests take at most
/// `request_timeout`: it checks its deadline only once a snapshot returns,
/// after a poll's sleep.
pub fn settle_limit(request_timeout: Duration) -> Duration {
    SETTLE + SETTLE_POLL + request_timeout
}

/// How `dsplr apply` or `dsplr set` was asked to run.
pub struct Options {
    /// Keep the change without asking.
    pub yes: bool,
    /// Validate and test only.
    pub dry_run: bool,
    /// How long the confirm prompt waits before reverting.
    pub confirm_timeout: Duration,
}

/// Applies `file`, read from a layout file, on `backend`.
pub async fn file(backend: &Backend, file: &Layout, options: &Options) -> Exit {
    let current = match crate::snapshot(backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    onto(backend, current, file, options).await
}

/// Applies `file`, a layout read from a file or a profile, onto `current`,
/// the snapshot just read: each of its outputs is paired with its own
/// connected output that its identity names, the others stay as they are,
/// and one left without an output refuses it all.
pub async fn onto(backend: &Backend, current: Layout, file: &Layout, options: &Options) -> Exit {
    let asked = match bind(file, &current) {
        Ok(layout) => layout,
        Err(unbound) => {
            for output in unbound {
                eprintln!("dsplr: {output}");
            }
            return Exit::Refused;
        }
    };
    run(backend, current, &asked, options).await
}

/// Takes `backend` from `current`, the snapshot just read, to `asked`, a
/// whole layout naming the live outputs, through the confirm flow
/// ([`dsplr_core::flow`]), confirming on the terminal and reporting on
/// stderr.
pub async fn run(backend: &Backend, current: Layout, asked: &Layout, options: &Options) -> Exit {
    let change = Change {
        current,
        asked: asked.clone(),
        capabilities: backend.capabilities(),
        backend: backend.name().into(),
    };
    let flow = flow::Options {
        yes: options.yes,
        dry_run: options.dry_run,
        can_ask: io::stdin().is_terminal(),
    };
    let outcome = drive(
        backend,
        change,
        flow,
        options.confirm_timeout,
        &mut Terminal,
        &mut |line| eprintln!("dsplr: {line}"),
    )
    .await;
    exit(outcome)
}

/// The exit code `dsplr apply` ends a change with (`docs/plan/07-cli.md`).
pub const fn exit(outcome: Outcome) -> Exit {
    match outcome {
        Outcome::Kept | Outcome::Tested => Exit::Success,
        Outcome::Refused => Exit::Refused,
        Outcome::BackendRefused => Exit::BackendRefused,
        Outcome::Reverted => Exit::Reverted,
        Outcome::Failed => Exit::Failure,
    }
}

/// Whoever is asked whether to keep a change.
pub trait Confirm {
    /// Whether to keep the change, asked for at most `timeout`; no answer
    /// in time is no. Its watchdog reverts it at `watchdog`, should nothing
    /// else.
    fn confirm(&mut self, timeout: Duration, watchdog: Instant) -> bool;
}

/// The terminal: a line on stdin.
struct Terminal;

impl Confirm for Terminal {
    fn confirm(&mut self, timeout: Duration, _watchdog: Instant) -> bool {
        confirm(timeout)
    }
}

/// Drives `change` through the confirm flow on `backend`: does each step
/// the flow asks for, asks `confirmer` when it comes to that, and hands
/// every message to `say`. The revert point and its watchdog are the ones
/// `dsplr restore --revert` reads, so a dsplr killed mid-confirm is still
/// reverted.
pub async fn drive(
    backend: &Backend,
    change: Change,
    options: flow::Options,
    confirm_timeout: Duration,
    confirmer: &mut dyn Confirm,
    say: &mut dyn FnMut(String),
) -> Outcome {
    let (mut flow, mut step) = Flow::start(change, options);
    let asked = flow.change().asked.clone();
    let capabilities = flow.change().capabilities.clone();
    // The revert point once written: where, and the change's own.
    let mut guard: Option<Guard> = None;
    loop {
        for line in flow.messages() {
            say(line);
        }
        let answer = match step {
            Step::Done(outcome) => return outcome,
            Step::Test => Answer::Tested(backend.test(&asked).await.map_err(|e| failure(&e))),
            Step::Prepare => Answer::Prepared(
                prepare(backend, &flow.change().current, confirm_timeout)
                    .map(|kept| guard = Some(kept)),
            ),
            Step::Apply => Answer::Applied(backend.apply(&asked).await.map_err(|e| failure(&e))),
            Step::Settle => Answer::Settled(
                settle(backend, &asked, &capabilities)
                    .await
                    .map_err(|error| error.to_string()),
            ),
            Step::ShowNow => Answer::Shown(
                backend
                    .snapshot()
                    .await
                    .map(|now| asked.differences(&now, &capabilities))
                    .map_err(|error| error.to_string()),
            ),
            Step::Discard => Answer::Discarded(match guard.take() {
                Some(Guard { path, point, .. }) => revert::remove_if(&path, &point.token)
                    .map(drop)
                    .map_err(|error| error.to_string()),
                None => Ok(()),
            }),
            Step::Confirm => Answer::Confirmed(match &guard {
                Some(guard) => confirmer.confirm(confirm_timeout, guard.watchdog),
                None => false,
            }),
            Step::Revert => Answer::Reverted(match &guard {
                Some(Guard { path, point, .. }) => {
                    restore::go_back(backend, path, point, say).await == Exit::Success
                }
                None => false,
            }),
            Step::Keep => Answer::Kept(match &guard {
                Some(Guard { path, point, .. }) => {
                    revert::keep(path, &point.token).map_err(|error| error.to_string())
                }
                None => Ok(true),
            }),
        };
        step = flow.next(answer);
    }
}

/// A change's guard: its revert point, where it is, and when its watchdog
/// reverts it.
struct Guard {
    path: PathBuf,
    point: RevertPoint,
    watchdog: Instant,
}

/// Step 3's guard: writes `current` as the revert point and starts its
/// watchdog. The error is the lines to show.
fn prepare(
    backend: &Backend,
    current: &Layout,
    confirm_timeout: Duration,
) -> Result<Guard, Vec<String>> {
    let cannot_keep = |error: dsplr_persist::revert::RevertError| {
        vec![format!("cannot keep a revert point: {error}")]
    };
    let path = revert::default_path().map_err(cannot_keep)?;
    let point = RevertPoint::new(current.clone());
    revert::save(&path, &point).map_err(cannot_keep)?;
    let delay = restore::watchdog_delay(confirm_timeout, backend.request_timeout());
    let started = Instant::now();
    if let Err(error) = restore::spawn_watchdog(&point.token, delay) {
        let mut why = vec![format!("cannot start the revert watchdog: {error}")];
        if let Err(error) = revert::remove_if(&path, &point.token) {
            why.push(error.to_string());
        }
        return Err(why);
    }
    Ok(Guard {
        path,
        point,
        watchdog: started + delay,
    })
}

/// A backend error as the flow tells them apart.
pub fn failure(error: &dsplr_backend::Error) -> Failure {
    let text = error.to_string();
    if let dsplr_backend::Error::Unsupported { reason, .. } = error {
        Failure::Unsupported((*reason).to_owned())
    } else if error.is_refused_before_sending() {
        Failure::NotTaken(text)
    } else if error.is_refusal() {
        Failure::Refused(text)
    } else if error.is_stale() {
        Failure::Stale(text)
    } else {
        Failure::Failed(text)
    }
}

/// A layout file's output that has no connected output of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unbound {
    /// It names no connected output.
    NotConnected(OutputId),
    /// Every connected output it names is another entry's.
    Taken(OutputId),
}

impl std::fmt::Display for Unbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected(id) => write!(f, "{id}: not connected"),
            Self::Taken(id) => write!(
                f,
                "{id}: every connected output it names is taken by another of the layout's outputs"
            ),
        }
    }
}

/// Pairs each entry of `file` with its own live output that its identity
/// names ([`assign`]), taking the live identity and hardware facts so the
/// backend can name it. A mirror's source is the live output its file
/// entry was paired with, or else the first one its identity names. Live
/// outputs left unpaired keep their current state. Positions keep the
/// file's layout mode, which they are measured in. The error lists the
/// file's outputs left without one.
pub fn bind(file: &Layout, live: &Layout) -> Result<Layout, Vec<Unbound>> {
    let paired = assign(&file.entries, &live.entries).map_err(|unpaired| {
        unpaired
            .into_iter()
            .map(|index| {
                let id = file.entries[index].id.clone();
                if live.entries.iter().any(|live| id.matches(live)) {
                    Unbound::Taken(id)
                } else {
                    Unbound::NotConnected(id)
                }
            })
            .collect::<Vec<_>>()
    })?;
    let source = |id: &OutputId| {
        file.entries
            .iter()
            .position(|entry| entry.id == *id)
            .map(|index| &live.entries[paired[index]])
            .or_else(|| live.entries.iter().find(|live| id.matches(live)))
            .map_or_else(|| id.clone(), |live| live.id.clone())
    };
    let mut entries: Vec<OutputEntry> = file
        .entries
        .iter()
        .zip(&paired)
        .map(|(entry, &index)| {
            let found = &live.entries[index];
            let mut state = entry.state.clone();
            state.mirror_of = entry.state.mirror_of.as_ref().map(source);
            OutputEntry {
                id: found.id.clone(),
                output: found.output.clone(),
                state,
            }
        })
        .collect();
    entries.extend(
        (0..live.entries.len())
            .filter(|index| !paired.contains(index))
            .map(|index| live.entries[index].clone()),
    );
    Ok(Layout {
        entries,
        layout_mode: file.layout_mode,
    })
}

/// Reads the layout back until it shows `asked` or [`SETTLE`] has passed,
/// and returns what still differs.
///
/// # Errors
///
/// When the layout cannot be read.
pub async fn settle(
    backend: &Backend,
    asked: &Layout,
    capabilities: &Capabilities,
) -> Result<Vec<Difference>, dsplr_backend::Error> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let differences = asked.differences(&backend.snapshot().await?, capabilities);
        if differences.is_empty() || Instant::now() >= deadline {
            return Ok(differences);
        }
        tokio::time::sleep(SETTLE_POLL).await;
    }
}

/// Asks on the terminal whether to keep the change, and waits at most
/// `timeout` for the answer. Anything but yes, and no answer, is no.
fn confirm(timeout: Duration) -> bool {
    eprint!(
        "Keep this layout? [y/N] (reverting in {} s) ",
        timeout.as_secs()
    );
    let (answer, answered) = mpsc::channel();
    // Reading stdin cannot be given a timeout, so it is read on a thread
    // that is left blocked if no answer comes.
    thread::spawn(move || {
        let mut line = String::new();
        let read = io::stdin().lock().read_line(&mut line).map(|_| line);
        // The receiver is gone once the prompt has timed out; the answer
        // then has nowhere to go.
        let _ = answer.send(read);
    });
    match answered.recv_timeout(timeout) {
        Ok(Ok(line)) => is_yes(&line),
        Ok(Err(error)) => {
            eprintln!("dsplr: cannot read the answer: {error}");
            false
        }
        Err(_) => {
            eprintln!();
            false
        }
    }
}

/// Whether `answer` is a yes: `y` or `yes`, in any case.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Layout, Mode, Output, OutputEntry, OutputId, OutputState};

    use super::{Unbound, bind, is_yes};

    fn live_entry(connector: &str, serial: &str) -> OutputEntry {
        OutputEntry {
            id: OutputId::new("AOC", "Q27G4", serial, connector),
            output: Some(Output {
                connector: connector.into(),
                description: format!("AOC Q27G4 {serial}"),
                physical_size_mm: None,
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                modes: vec![Mode::new(2560, 1440, 120_000)],
            }),
            state: OutputState {
                mode: Some(Mode::new(2560, 1440, 120_000)),
                ..OutputState::default()
            },
        }
    }

    fn live() -> Layout {
        Layout {
            entries: vec![live_entry("DP-1", "AAA"), live_entry("HDMI-A-1", "BBB")],
            ..Default::default()
        }
    }

    #[test]
    fn a_file_entry_takes_the_live_identity_and_facts_of_its_output() {
        let file = Layout {
            entries: vec![OutputEntry {
                id: OutputId::Description("AOC Q27G4 BBB".into()),
                output: None,
                state: OutputState {
                    x: 2560,
                    mirror_of: Some(OutputId::Connector("DP-1".into())),
                    ..live().entries[1].state.clone()
                },
            }],
            ..Default::default()
        };
        let bound = bind(&file, &live()).unwrap();
        assert_eq!(bound.entries.len(), 2);
        let entry = &bound.entries[0];
        assert_eq!(entry.id, live().entries[1].id);
        assert_eq!(entry.output, live().entries[1].output);
        assert_eq!(entry.state.x, 2560);
        assert_eq!(entry.state.mirror_of, Some(live().entries[0].id.clone()));
        assert_eq!(
            bound.entries[1],
            live().entries[0],
            "not in the file: as it is"
        );
    }

    #[test]
    fn a_file_naming_an_output_not_connected_is_refused() {
        // BB is not BBB: a description names whole words.
        for id in [
            OutputId::Connector("DP-9".into()),
            OutputId::Description("AOC Q27G4 BB".into()),
        ] {
            let file = Layout {
                entries: vec![OutputEntry {
                    id: id.clone(),
                    output: None,
                    state: OutputState::default(),
                }],
                ..Default::default()
            };
            assert_eq!(bind(&file, &live()), Err(vec![Unbound::NotConnected(id)]));
        }
    }

    /// Two entries whose identities both name one output, with another
    /// output that only the second names: each gets its own, whichever is
    /// listed first, though the first is the description's first match.
    #[test]
    fn a_description_naming_two_outputs_takes_the_one_left() {
        let entry = |id: OutputId, x: i32| OutputEntry {
            id,
            output: None,
            state: OutputState {
                x,
                ..live().entries[0].state.clone()
            },
        };
        let file = Layout {
            entries: vec![
                entry(OutputId::Description("AOC Q27G4".into()), 100),
                entry(OutputId::Connector("DP-1".into()), 200),
            ],
            ..Default::default()
        };
        let bound = bind(&file, &live()).unwrap();
        let x = |connector: &str| {
            bound
                .entries
                .iter()
                .find(|entry| entry.connector() == Some(connector))
                .unwrap()
                .state
                .x
        };
        assert_eq!((x("HDMI-A-1"), x("DP-1")), (100, 200));
        assert_eq!(bound.entries.len(), 2, "each output once");

        // A third for two outputs is left over, though it names one.
        let mut three = file.clone();
        three
            .entries
            .push(entry(OutputId::Connector("HDMI-A-1".into()), 300));
        assert_eq!(
            bind(&three, &live()),
            Err(vec![Unbound::Taken(OutputId::Description(
                "AOC Q27G4".into()
            ))])
        );
    }

    /// Two monitors with one EDID identity, each saved on its connector.
    #[test]
    fn twins_each_take_their_own_output() {
        let twin = |connector: &str| {
            let mut entry = live_entry(connector, "SAME");
            entry.state.x = if connector == "DP-1" { 0 } else { 2560 };
            entry
        };
        let now = Layout {
            entries: vec![twin("DP-1"), twin("HDMI-A-1")],
            ..Default::default()
        };
        let mut file = now.clone();
        file.entries.reverse();
        for entry in &mut file.entries {
            entry.state.y = 10;
        }
        let bound = bind(&file, &now).unwrap();
        assert_eq!(bound.entries.len(), 2);
        for entry in &bound.entries {
            let x = if entry.connector() == Some("DP-1") {
                0
            } else {
                2560
            };
            assert_eq!((entry.state.x, entry.state.y), (x, 10), "{entry:?}");
        }
        assert_ne!(bound.entries[0].connector(), bound.entries[1].connector());
    }

    #[test]
    fn only_yes_keeps() {
        for yes in ["y", "Y\n", " yes \r\n", "YES"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "no", "yess", "sure"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    /// `docs/plan/07-cli.md`'s codes for each way a change ends.
    #[test]
    fn each_outcome_exits_with_its_documented_code() {
        use dsplr_core::flow::Outcome;

        use super::exit;
        use crate::Exit;

        let codes = [
            (Outcome::Kept, Exit::Success),
            (Outcome::Tested, Exit::Success),
            (Outcome::Refused, Exit::Refused),
            (Outcome::BackendRefused, Exit::BackendRefused),
            (Outcome::Reverted, Exit::Reverted),
            (Outcome::Failed, Exit::Failure),
        ];
        for (outcome, code) in codes {
            assert_eq!(exit(outcome), code, "{outcome:?}");
        }
    }
}
