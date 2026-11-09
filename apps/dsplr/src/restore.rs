//! `dsplr restore --revert`: going back to the revert point, by hand or as
//! the watchdog a change starts (`docs/plan/02-architecture.md`, step 5).

use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use dsplr_backend::Backend;
use dsplr_persist::revert::{self, RevertPoint};

use dsplr_core::flow::report;

use crate::apply::{settle, settle_limit};
use crate::{Exit, find_backend};

/// How long the watchdog waits past the longest the dsplr that started it
/// can take to settle and ask, so that dsplr's own revert or keep comes
/// first.
const WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// How long the watchdog for a change confirmed within `confirm_timeout`
/// waits before it reverts, on a backend whose requests take at most
/// `request_timeout` ([`dsplr_backend::Backend::request_timeout`]). Counted
/// from before the apply, it covers the apply and its settle, the prompt,
/// and the apply and settle of dsplr's own revert after a no, so the two
/// never go back at once even when every request runs to its timeout.
pub fn watchdog_delay(confirm_timeout: Duration, request_timeout: Duration) -> Duration {
    let change = request_timeout + settle_limit(request_timeout);
    change + confirm_timeout + change + WATCHDOG_GRACE
}

/// Starts the watchdog for the change `token` names: a detached
/// `dsplr restore --revert --after <secs> --token <token>`. It inherits the
/// environment, which is how it reaches the desktop.
///
/// # Errors
///
/// When dsplr's own executable cannot be found or started.
pub fn spawn_watchdog(token: &str, after: Duration) -> io::Result<()> {
    let mut command = Command::new(env::current_exe()?);
    command
        .args(["restore", "--revert", "--after"])
        .arg(after.as_secs().to_string())
        .args(["--token", token])
        // Nothing to talk to: the terminal may close while it waits.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A process group of its own, so the hangup sent to the terminal's
    // foreground group when the terminal closes does not end it too. Every
    // backend is Linux-only, so no change reaches this off Unix.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().map(drop)
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
    if let Err(error) = backend.apply(&point.layout).await {
        say(format!(
            "cannot go back to the revert point: {error}; `dsplr restore --revert` tries again"
        ));
        return if error.is_refused_before_sending() {
            Exit::Refused
        } else if error.is_refusal() || error.is_stale() {
            Exit::BackendRefused
        } else {
            Exit::Failure
        };
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
    let say = &mut |line: String| eprintln!("dsplr: {line}");
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
    go_back(&backend, &path, &point, say).await
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
    use std::time::Duration;

    use super::{settle_limit, watchdog_delay};
    use crate::apply::SETTLE;

    #[test]
    fn the_watchdog_waits_past_two_slow_changes_and_the_prompt() {
        let confirm = Duration::from_secs(20);
        for request in [
            Duration::ZERO,
            Duration::from_secs(5),
            Duration::from_secs(10),
        ] {
            // An apply and its settle, each request running to its timeout,
            // before and after the prompt.
            let change = request + settle_limit(request);
            assert!(settle_limit(request) > SETTLE + request);
            assert!(watchdog_delay(confirm, request) > 2 * change + confirm);
        }
    }
}
