//! `dsplr apply <FILE>`: a whole layout from a file, through the confirm flow
//! (`docs/plan/02-architecture.md`, "Apply, confirm, revert"), which
//! `dsplr set` ([`run`]) and `dsplr restore` with a profile ([`onto`])
//! share.

use std::io::{self, BufRead, IsTerminal};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::{
    Capabilities, Difference, Layout, OutputEntry, OutputId, Problem, Setting, assign,
};
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
/// whole layout naming the live outputs: validate, test, keep a revert
/// point and start its watchdog, apply, read back, and keep or revert.
pub async fn run(backend: &Backend, current: Layout, asked: &Layout, options: &Options) -> Exit {
    let capabilities = backend.capabilities();
    let problems = asked.validate(&capabilities);
    for problem in &problems {
        let severity = if problem.is_error() {
            "error"
        } else {
            "warning"
        };
        eprintln!("dsplr: {severity}: {problem}");
    }
    if problems.iter().any(Problem::is_error) {
        return Exit::Refused;
    }
    // Step 2: the desktop's own test, where it has a real one.
    match backend.test(asked).await {
        Ok(()) if options.dry_run => {
            eprintln!(
                "dsplr: {} tested the layout and would apply it",
                backend.name()
            );
            return Exit::Success;
        }
        Ok(()) => {}
        Err(dsplr_backend::Error::Unsupported { reason, .. }) if options.dry_run => {
            eprintln!(
                "dsplr: validated locally only; {} cannot test a layout: {reason}",
                backend.name()
            );
            return Exit::Success;
        }
        Err(dsplr_backend::Error::Unsupported { .. }) => {}
        Err(error) if error.is_refused_before_sending() => {
            cannot_take(backend, &error);
            return Exit::Refused;
        }
        Err(error) if error.is_refusal() => {
            eprintln!(
                "dsplr: {} refused the layout in its test, so nothing was applied: {error}",
                backend.name()
            );
            return Exit::BackendRefused;
        }
        Err(error) if error.is_stale() => {
            eprintln!(
                "dsplr: the displays changed while {} tested the layout, so nothing was applied: {error}",
                backend.name()
            );
            show_now(backend, asked, &capabilities).await;
            return Exit::BackendRefused;
        }
        Err(error) => {
            eprintln!(
                "dsplr: cannot test the layout on {}, so nothing was applied: {error}",
                backend.name()
            );
            return Exit::Failure;
        }
    }
    let ask = !options.yes && needs_confirm(asked, &current, &capabilities);
    if ask && !io::stdin().is_terminal() {
        eprintln!(
            "dsplr: this change can hide or move a screen, and there is no terminal to confirm it on; pass --yes to keep it without asking"
        );
        return Exit::Failure;
    }

    let path = match revert::default_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("dsplr: cannot keep a revert point: {error}");
            return Exit::Failure;
        }
    };
    let point = RevertPoint::new(current);
    if let Err(error) = revert::save(&path, &point) {
        eprintln!("dsplr: cannot keep a revert point: {error}");
        return Exit::Failure;
    }
    if let Err(error) = restore::spawn_watchdog(
        &point.token,
        restore::watchdog_delay(options.confirm_timeout, backend.request_timeout()),
    ) {
        eprintln!("dsplr: cannot start the revert watchdog: {error}");
        if let Err(error) = revert::remove_if(&path, &point.token) {
            eprintln!("dsplr: {error}");
        }
        return Exit::Failure;
    }

    if let Err(error) = backend.apply(asked).await {
        if error.is_stale() {
            // Nothing was applied, and the displays changed since the revert
            // point was read: going back would undo that other change. Show
            // what they are now instead.
            eprintln!(
                "dsplr: the displays changed before {} applied the layout, so nothing was applied: {error}",
                backend.name()
            );
            if let Err(error) = revert::remove_if(&path, &point.token) {
                eprintln!("dsplr: {error}");
            }
            show_now(backend, asked, &capabilities).await;
            return Exit::BackendRefused;
        }
        if error.is_refused_before_sending() {
            // Nothing was sent, so there is nothing to go back from.
            cannot_take(backend, &error);
            if let Err(error) = revert::remove_if(&path, &point.token) {
                eprintln!("dsplr: {error}");
            }
            return Exit::Refused;
        }
        // Refused or not, something may have changed (a connection lost
        // mid-apply), so go back either way.
        let exit = if error.is_refusal() {
            eprintln!("dsplr: {} refused the layout: {error}", backend.name());
            Exit::BackendRefused
        } else {
            eprintln!(
                "dsplr: cannot apply the layout on {}: {error}",
                backend.name()
            );
            Exit::Failure
        };
        restore::go_back(backend, &path, &point).await;
        return exit;
    }
    match settle(backend, asked, &capabilities).await {
        Ok(differences) => report(&differences),
        Err(error) => {
            // Without a re-read nothing says what the screens show: go back
            // rather than keep it unseen.
            eprintln!("dsplr: cannot read the displays back: {error}");
            restore::go_back(backend, &path, &point).await;
            return Exit::Failure;
        }
    }

    if ask && !confirm(options.confirm_timeout) {
        eprintln!("dsplr: reverting");
        // A revert that failed leaves the declined layout on screen, so
        // nothing here may read as "nothing was applied" (3 or 4).
        return match restore::go_back(backend, &path, &point).await {
            Exit::Success => Exit::Reverted,
            _ => Exit::Failure,
        };
    }
    match revert::keep(&path, &point.token) {
        Ok(true) => Exit::Success,
        Ok(false) => {
            eprintln!("dsplr: the revert point was replaced by another change; this one is kept");
            Exit::Success
        }
        Err(error) => {
            eprintln!(
                "dsplr: cannot mark the change kept, so the watchdog will revert it: {error}"
            );
            Exit::Failure
        }
    }
}

/// A layout file's output that has no connected output of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unbound {
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
/// outputs left unpaired keep their current state. The error lists the
/// file's outputs left without one.
fn bind(file: &Layout, live: &Layout) -> Result<Layout, Vec<Unbound>> {
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
    Ok(Layout { entries })
}

/// Whether going from `current` to `asked` can hide or move a screen: an
/// output enabled or disabled, or its mode, position, scale or transform
/// changed.
fn needs_confirm(asked: &Layout, current: &Layout, capabilities: &Capabilities) -> bool {
    asked
        .differences(current, capabilities)
        .iter()
        .any(|difference| match difference {
            Difference::Setting { setting, .. } => matches!(
                setting,
                Setting::Enabled
                    | Setting::Mode
                    | Setting::Position
                    | Setting::Scale
                    | Setting::Transform
            ),
            Difference::Missing(_) | Difference::Added(_) => true,
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

/// Reports a layout `backend` refused before sending it
/// ([`dsplr_backend::Error::is_refused_before_sending`]).
fn cannot_take(backend: &Backend, error: &dsplr_backend::Error) {
    eprintln!(
        "dsplr: {} cannot take the layout, so nothing was applied: {error}",
        backend.name()
    );
}

/// Reads the displays again after the desktop dropped a request because they
/// changed, and shows how they differ from `asked`.
async fn show_now(backend: &Backend, asked: &Layout, capabilities: &Capabilities) {
    match backend.snapshot().await {
        Ok(now) => report(&asked.differences(&now, capabilities)),
        Err(error) => eprintln!("dsplr: cannot read the displays again: {error}"),
    }
}

/// Shows what the desktop did not take as asked, if anything.
pub fn report(differences: &[Difference]) {
    if differences.is_empty() {
        return;
    }
    eprintln!("dsplr: the displays show something other than was asked:");
    for difference in differences {
        eprintln!("  {difference}");
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
    use dsplr_model::{
        Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::{Unbound, bind, is_yes, needs_confirm};

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
    fn only_a_change_that_can_hide_or_move_a_screen_is_confirmed() {
        let current = live();
        let caps = Capabilities::default();
        assert!(!needs_confirm(&current, &current, &caps));

        // A difference, but one that moves nothing.
        let with_primary = Capabilities {
            primary: true,
            ..Capabilities::default()
        };
        let mut primary = current.clone();
        primary.entries[0].state.primary = true;
        assert!(!primary.differences(&current, &with_primary).is_empty());
        assert!(!needs_confirm(&primary, &current, &with_primary));

        let mut rotated = current.clone();
        rotated.entries[0].state.transform = Transform::Rotate90;
        assert!(needs_confirm(&rotated, &current, &caps));

        let mut disabled = current.clone();
        disabled.entries[1].state.enabled = false;
        assert!(needs_confirm(&disabled, &current, &caps));
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
}
