//! `dsplr restore --revert`: going back to the revert point, by hand or as
//! the watchdog a change starts (`docs/plan/02-architecture.md`, step 5).

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dsplr_backend::{Backend, Named};
use dsplr_persist::revert::{self, RevertPoint};

use dsplr_core::flow::{Failure, report};

use crate::apply::{settle, settle_limit};
use crate::{Exit, find_backend};

/// How long the watchdog waits past the longest the dsplr that started it
/// can take to settle and ask, so that dsplr's own revert or keep comes
/// first.
const WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// The watchdog's log, beside the revert point.
const LOG_FILE: &str = "watchdog.log";

/// A log larger than this is started afresh when the next watchdog starts,
/// so it never grows without bound.
const LOG_LIMIT: u64 = 64 * 1024;

/// How long the watchdog for a change confirmed within `confirm_timeout`
/// waits before it reverts, on a backend whose requests take at most
/// `request_timeout` ([`dsplr_backend::Backend::request_timeout`]). Counted
/// from before the apply, it covers the apply and its settle, the prompt,
/// and the apply and settle of dsplr's own revert after a no, so the two
/// never go back at once even when every request runs to its timeout.
pub fn watchdog_delay(confirm_timeout: Duration, request_timeout: Duration) -> Duration {
    let change = change_limit(request_timeout);
    change + confirm_timeout + change + WATCHDOG_GRACE
}

/// The longest one apply and its settle take on a backend whose requests
/// take at most `request_timeout`: a change, or a revert ([`go_back`]).
pub fn change_limit(request_timeout: Duration) -> Duration {
    request_timeout + settle_limit(request_timeout)
}

/// The watchdog's log for the revert point at `revert_path`: the file in
/// the same directory its messages go to.
pub fn log_path(revert_path: &Path) -> PathBuf {
    revert_path.with_file_name(LOG_FILE)
}

/// Opens the log at `path` to append to, or afresh once it is past
/// [`LOG_LIMIT`].
fn open_log(path: &Path) -> io::Result<File> {
    let full = fs::metadata(path).is_ok_and(|metadata| metadata.len() > LOG_LIMIT);
    let mut options = OpenOptions::new();
    options.create(true);
    if full {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    options.open(path)
}

/// Starts the watchdog for the change `token` names: a detached
/// `dsplr restore --revert --after <secs> --token <token>`, with the
/// `--backend` this one was given. It inherits the environment, which is how
/// it reaches the desktop. Its messages go to the log at `log`
/// ([`log_path`]), since the terminal may be gone when it fires. The process
/// is returned so that a change which sent nothing can end it at once rather
/// than leave it waiting.
///
/// # Errors
///
/// When the log cannot be opened, or dsplr's own executable cannot be found
/// or started.
pub fn spawn_watchdog(token: &str, after: Duration, log: &Path) -> io::Result<std::process::Child> {
    let log = open_log(log)?;
    let mut command = Command::new(env::current_exe()?);
    command
        .args(watchdog_args(token, after, crate::forced()))
        // Nothing to read from or print to: the terminal may close while it
        // waits.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // A process group of its own, so the hangup sent to the terminal's
    // foreground group when the terminal closes does not end it too. Every
    // backend is Linux-only, so no change reaches this off Unix.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn()
}

/// The watchdog's arguments: [`spawn_watchdog`]'s command line after the
/// executable, `--backend` passed on when this dsplr was given it, so the
/// revert goes to the backend the change went to.
fn watchdog_args(token: &str, after: Duration, forced: Option<Named>) -> Vec<String> {
    let mut args: Vec<String> = ["restore", "--revert", "--after"].map(str::to_owned).into();
    args.push(after.as_secs().to_string());
    args.extend(["--token".to_owned(), token.to_owned()]);
    if let Some(named) = forced {
        args.extend(["--backend".to_owned(), named.name().to_owned()]);
    }
    args
}

/// Applies `point`'s layout, shows what did not come back as it was, and
/// removes the revert point, each message handed to `say`. Returns
/// [`Exit::Success`] when the layout was applied, else the exit code for why
/// not; then the revert point stays for the watchdog or a revert by hand.
pub async fn go_back(
    backend: &Backend,
    path: &Path,
    point: &RevertPoint,
    say: &mut dyn FnMut(String),
) -> Exit {
    let applied = backend.apply(&point.layout).await;
    if let Ok(notes) = &applied {
        for note in notes {
            say(note.clone());
        }
    }
    if let Err(error) = applied {
        // Classified as the flow classifies an apply: what sent nothing
        // (the backend cannot apply at all, or cannot take the layout) is
        // refused, and trying again would do the same.
        let retry = "; `dsplr restore --revert` tries again";
        let (exit, hint) = match crate::apply::failure(&error) {
            Failure::Unsupported(_) => (Exit::Refused, ""),
            Failure::NotTaken(_) => (Exit::Refused, retry),
            Failure::Refused(_) | Failure::Stale(_) => (Exit::BackendRefused, retry),
            Failure::Failed(_) => (Exit::Failure, retry),
        };
        say(format!("cannot go back to the revert point: {error}{hint}"));
        return exit;
    }
    match settle(backend, &point.layout, &backend.capabilities()).await {
        Ok(differences) => {
            if let Some(report) = report(&differences) {
                say(report);
            }
        }
        Err(error) => say(format!("cannot read the displays back: {error}")),
    }
    if let Err(error) = revert::remove_if(path, &point.token) {
        say(error.to_string());
    }
    Exit::Success
}

/// `dsplr restore --revert`. With `after` and `token` it is the watchdog: it
/// waits `after`, then reverts only while the revert point still holds
/// `token`, that is, while the change it guards is neither kept nor
/// reverted.
pub async fn run(after: Option<Duration>, token: Option<&str>) -> Exit {
    if let Some(after) = after {
        tokio::time::sleep(after).await;
    }
    let say = &mut |line: String| match token {
        // The watchdog's stderr is its log: each line says when, and which
        // change it was guarding.
        Some(token) => eprintln!("{}", log_line(SystemTime::now(), token, &line)),
        None => eprintln!("dsplr: {line}"),
    };
    let (path, point) = match load(token.is_none(), say) {
        Ok(Some(found)) => found,
        Ok(None) => return Exit::Success,
        Err(exit) => return exit,
    };
    if token.is_some_and(|token| token != point.token) {
        return Exit::Success;
    }
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    let exit = go_back(&backend, &path, &point, say).await;
    if token.is_some() && exit == Exit::Success {
        say("the change was not kept in time: went back to the revert point".to_owned());
    }
    exit
}

/// One line of the watchdog's log: the time in unix seconds, the token of
/// the change, and what happened.
fn log_line(at: SystemTime, token: &str, line: &str) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    format!("{secs} dsplr watchdog {token}: {line}")
}

/// Goes back to the revert point on `backend` by hand, as
/// `dsplr restore --revert` does, each message handed to `say`.
pub async fn revert(backend: &Backend, say: &mut dyn FnMut(String)) -> Exit {
    match load(true, say) {
        Ok(Some((path, point))) => go_back(backend, &path, &point, say).await,
        Ok(None) => Exit::Success,
        Err(exit) => exit,
    }
}

/// The revert point and where it is, or `None` when there is none, which
/// is an error (said, and [`Exit::Failure`]) when `required`.
fn load(
    required: bool,
    say: &mut dyn FnMut(String),
) -> Result<Option<(std::path::PathBuf, RevertPoint)>, Exit> {
    let path = revert::default_path().map_err(|error| {
        say(error.to_string());
        Exit::Failure
    })?;
    match revert::load(&path) {
        Ok(Some(point)) => Ok(Some((path, point))),
        Ok(None) if required => {
            say("no revert point to go back to".to_owned());
            Err(Exit::Failure)
        }
        Ok(None) => Ok(None),
        Err(error) => {
            say(error.to_string());
            Err(Exit::Failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use dsplr_backend::Named;

    use super::{
        LOG_LIMIT, log_line, log_path, open_log, settle_limit, watchdog_args, watchdog_delay,
    };
    use crate::apply::SETTLE;

    #[test]
    fn the_log_sits_beside_the_revert_point() {
        assert_eq!(
            log_path(Path::new("/state/dsplr/revert.json")),
            Path::new("/state/dsplr/watchdog.log")
        );
    }

    #[test]
    fn the_log_is_appended_to_until_it_is_full() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watchdog.log");
        writeln!(open_log(&path).unwrap(), "first").unwrap();
        writeln!(open_log(&path).unwrap(), "second").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "first
second
"
        );

        let full = "x".repeat(usize::try_from(LOG_LIMIT).unwrap() + 1);
        fs::write(&path, full).unwrap();
        writeln!(open_log(&path).unwrap(), "afresh").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "afresh
"
        );
    }

    #[test]
    fn a_log_line_says_when_and_for_which_change() {
        let at = UNIX_EPOCH + Duration::from_secs(1_763_000_000);
        assert_eq!(
            log_line(at, "42-7", "cannot go back"),
            "1763000000 dsplr watchdog 42-7: cannot go back"
        );
        assert!(log_line(SystemTime::now(), "t", "x").ends_with(" dsplr watchdog t: x"));
    }

    #[test]
    fn the_watchdog_is_given_the_backend_this_dsplr_was() {
        let after = Duration::from_secs(25);
        assert_eq!(
            watchdog_args("t", after, None),
            ["restore", "--revert", "--after", "25", "--token", "t"]
        );
        assert_eq!(
            watchdog_args("t", after, Some(Named::Wlr))[6..],
            ["--backend", "wlr"]
        );
    }

    #[test]
    fn the_watchdog_waits_past_two_slow_changes_and_the_prompt() {
        let confirm = Duration::from_secs(20);
        // X11's request timeout is an apply's, the longest of any backend's,
        // since a modeset can take seconds.
        #[cfg(target_os = "linux")]
        let x11 = [dsplr_backend::randr::APPLY_TIMEOUT];
        #[cfg(not(target_os = "linux"))]
        let x11: [Duration; 0] = [];
        for request in [
            Duration::ZERO,
            Duration::from_secs(5),
            Duration::from_secs(10),
        ]
        .into_iter()
        .chain(x11)
        {
            // An apply and its settle, each request running to its timeout,
            // before and after the prompt.
            let change = request + settle_limit(request);
            assert_eq!(super::change_limit(request), change);
            assert!(settle_limit(request) > SETTLE + request);
            assert!(watchdog_delay(confirm, request) > 2 * change + confirm);
        }
    }
}
