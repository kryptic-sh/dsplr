//! `dsplr restore --revert`: going back to the revert point, by hand or as
//! the watchdog a change starts (`docs/plan/02-architecture.md`, step 5).

use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use dsplr_backend::Backend;
use dsplr_persist::revert::{self, RevertPoint};

use crate::apply::{SETTLE, report, settle};
use crate::{Exit, find_backend};

/// How long the watchdog waits past the longest the dsplr that started it
/// can take to settle and ask, so that dsplr's own revert or keep comes
/// first.
const WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// How long the watchdog for a change confirmed within `confirm_timeout`
/// waits before it reverts. Counted from before the apply, it covers the
/// apply's settle, the prompt, and the settle of dsplr's own revert after a
/// no, so the two never go back at once.
pub fn watchdog_delay(confirm_timeout: Duration) -> Duration {
    SETTLE + confirm_timeout + SETTLE + WATCHDOG_GRACE
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
/// removes the revert point. Returns whether the layout was applied; when
/// not, the revert point stays for the watchdog or a revert by hand.
pub async fn go_back(backend: &Backend, path: &Path, point: &RevertPoint) -> bool {
    if let Err(error) = backend.apply(&point.layout).await {
        eprintln!(
            "dsplr: cannot go back to the revert point: {error}; `dsplr restore --revert` tries again"
        );
        return false;
    }
    match settle(backend, &point.layout, &backend.capabilities()).await {
        Ok(differences) => report(&differences),
        Err(error) => eprintln!("dsplr: cannot read the displays back: {error}"),
    }
    if let Err(error) = revert::remove_if(path, &point.token) {
        eprintln!("dsplr: {error}");
    }
    true
}

/// `dsplr restore --revert`. With `after` and `token` it is the watchdog: it
/// waits `after`, then reverts only while the revert point still holds
/// `token`, that is, while the change it guards is neither kept nor
/// reverted.
pub async fn run(after: Option<Duration>, token: Option<&str>) -> Exit {
    if let Some(after) = after {
        tokio::time::sleep(after).await;
    }
    let path = match revert::default_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("dsplr: {error}");
            return Exit::Failure;
        }
    };
    let point = match revert::load(&path) {
        Ok(Some(point)) => point,
        Ok(None) if token.is_some() => return Exit::Success,
        Ok(None) => {
            eprintln!("dsplr: no revert point to go back to");
            return Exit::Failure;
        }
        Err(error) => {
            eprintln!("dsplr: {error}");
            return Exit::Failure;
        }
    };
    if token.is_some_and(|token| token != point.token) {
        return Exit::Success;
    }
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    if go_back(&backend, &path, &point).await {
        Exit::Success
    } else {
        Exit::BackendRefused
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SETTLE, watchdog_delay};

    #[test]
    fn the_watchdog_waits_past_both_settles_and_the_prompt() {
        let timeout = Duration::from_secs(20);
        assert!(watchdog_delay(timeout) > SETTLE + timeout + SETTLE);
    }
}
