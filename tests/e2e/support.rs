//! Shared by every e2e test (included with `#[path]`): the check that the
//! test runs under its harness, finding and editing outputs in a layout, and
//! running a command on a terminal. Each desktop's own requests are in its
//! `<desktop>_support.rs`.

#![allow(
    dead_code,
    reason = "included into each e2e test binary, and each uses a subset"
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use dsplr_backend::Backend;
use dsplr_model::{Layout, OutputEntry, OutputState};

/// The harness's runtime directory, which the harness names in `variable`,
/// after checking that this process really runs under `harness` and not
/// against a session's desktop, and that dsplr's state goes to the
/// harness's directory, not the user's.
pub fn harness_runtime(variable: &str, harness: &str) -> PathBuf {
    let runtime = std::env::var(variable).unwrap_or_else(|_| {
        panic!(
            "run through {harness}, which starts a compositor that cannot reach the \
             desktop; refusing to touch whatever compositor this is"
        )
    });
    assert_eq!(
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        Some(runtime.as_str()),
        "XDG_RUNTIME_DIR is not the harness's private runtime directory"
    );
    assert!(
        !runtime.starts_with("/run/user/"),
        "the harness runtime must be private, not a session's {runtime}"
    );
    let state = std::env::var("XDG_STATE_HOME").unwrap_or_default();
    assert!(
        state.starts_with(&runtime),
        "XDG_STATE_HOME ({state:?}) is not inside the harness runtime"
    );
    PathBuf::from(runtime)
}

pub fn entry<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the snapshot: {layout:#?}"))
}

pub fn entry_mut<'a>(layout: &'a mut Layout, connector: &str) -> &'a mut OutputEntry {
    layout
        .entries
        .iter_mut()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the layout"))
}

/// The outputs of `snapshot`, each at its preferred mode (else its first),
/// unscaled and untransformed, in a row in the order the desktop lists
/// them, the first primary: the layout every test of a desktop with one
/// mode per output starts from, so each leaves the desktop as it would find
/// it.
pub fn row(snapshot: &Layout) -> Layout {
    let mut layout = snapshot.clone();
    let mut x = 0;
    for (index, entry) in layout.entries.iter_mut().enumerate() {
        let output = entry.output.as_ref().expect("live outputs");
        let mode = output
            .modes
            .iter()
            .find(|mode| mode.preferred)
            .or_else(|| output.modes.first())
            .expect("a mode")
            .clone();
        let width = i32::try_from(mode.width).expect("a mode's width fits");
        entry.state = OutputState {
            mode: Some(mode),
            x,
            primary: index == 0,
            ..OutputState::default()
        };
        x += width;
    }
    layout
}

/// The connector of every output in `layout`, in its order.
pub fn connectors(layout: &Layout) -> Vec<String> {
    layout
        .entries
        .iter()
        .map(|entry| entry.connector().expect("a connector").to_owned())
        .collect()
}

/// Waits until `desktop` shows `layout` (every setting dsplr compares, by
/// `Layout::differences`), or fails after `wait`.
pub async fn shows(backend: &Backend, layout: &Layout, desktop: &str, wait: Duration) {
    let started = Instant::now();
    loop {
        let now = backend.snapshot().await.expect("snapshot");
        let differences = layout.differences(&now, &backend.capabilities());
        if differences.is_empty() {
            return;
        }
        assert!(
            started.elapsed() < wait,
            "{desktop} does not show the layout: {differences:?}\n{now:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `layout` with `edit` made to it: what a user does to a snapshot before
/// applying it.
pub fn edited(layout: &Layout, edit: impl FnOnce(&mut Layout)) -> Layout {
    let mut layout = layout.clone();
    edit(&mut layout);
    layout
}

/// `path` for a shell command line, refusing anything that would need
/// quoting: every path here is the harness's or Cargo's own.
pub fn plain(path: &Path) -> &str {
    let text = path.to_str().expect("a UTF-8 path");
    assert!(
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)),
        "{text:?} would need quoting"
    );
    text
}

/// The shell command line `command` on a terminal `script` (util-linux)
/// gives it. Its stdin is the terminal's input.
pub fn on_a_terminal(command: &str) -> Child {
    Command::new("script")
        .args(["--quiet", "--return", "--command", command, "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("script (util-linux) runs")
}

/// [`on_a_terminal`], with `answer` typed into it, run to its end.
pub fn answered_on_a_terminal(command: &str, answer: &str) -> Output {
    let mut child = on_a_terminal(command);
    let mut input = child.stdin.take().expect("piped");
    input.write_all(answer.as_bytes()).expect("answered");
    let output = child.wait_with_output().expect("it ran");
    drop(input);
    output
}

/// How long `dsplr watch` gets to restore a profile after a hotplug: its
/// quiet time, the apply and the apply's settle, with room to spare.
pub const WATCH_RESTORE: Duration = Duration::from_secs(20);

/// A running `dsplr watch`, its stderr in a log file; killed if the test
/// ends before it stops it.
pub struct Watcher {
    child: Child,
    log: PathBuf,
}

impl Watcher {
    /// Starts `command`, a `dsplr watch` with the environment that reaches
    /// the desktop, its stderr to `log`.
    pub fn start(mut command: Command, log: PathBuf) -> Self {
        let file = std::fs::File::create(&log).expect("its log");
        let child = command
            .stdout(Stdio::null())
            .stderr(file)
            .spawn()
            .expect("dsplr watch runs");
        Self { child, log }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).expect("its log")
    }

    /// How many lines of its log say a profile is being restored: each
    /// one an apply.
    pub fn restores(&self) -> usize {
        self.log()
            .lines()
            .filter(|line| line.starts_with("dsplr: restoring profile"))
            .count()
    }

    /// Waits until the log has `count` restores, the last of `name`, and
    /// `read` (the layout as dsplr lists it) gives one `shown` accepts;
    /// returns it.
    pub async fn restored(
        &self,
        read: impl Fn() -> Layout,
        count: usize,
        name: &str,
        shown: impl Fn(&Layout) -> bool,
    ) -> Layout {
        let deadline = Instant::now() + WATCH_RESTORE;
        loop {
            let log = self.log();
            let last = log
                .lines()
                .rfind(|line| line.starts_with("dsplr: restoring profile"));
            let layout = read();
            if self.restores() == count
                && last.is_some_and(|line| line.contains(&format!("\"{name}\"")))
                && shown(&layout)
            {
                return layout;
            }
            assert!(
                Instant::now() < deadline,
                "restore {count} of \"{name}\" not seen: {layout:#?}\n{log}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Waits until the log has a line containing `text`.
    pub async fn said(&self, text: &str) {
        let deadline = Instant::now() + WATCH_RESTORE;
        while !self.log().contains(text) {
            assert!(
                Instant::now() < deadline,
                "{text:?} not said:\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Sends it SIGTERM.
    pub fn terminate(&self) {
        let pid = self.child.id().to_string();
        let status = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("kill runs");
        assert!(status.success());
    }

    /// Whether it is still running.
    pub fn running(&mut self) -> bool {
        self.child.try_wait().expect("its status").is_none()
    }

    /// Sends SIGTERM and checks it stops cleanly; returns its log.
    pub fn stop(self) -> String {
        self.terminate();
        self.ended()
    }

    /// Waits for it to end, and checks it ended cleanly on SIGTERM; returns
    /// its log.
    pub fn ended(mut self) -> String {
        let status = self.child.wait().expect("dsplr watch ends");
        let log = self.log();
        assert_eq!(status.code(), Some(0), "{log}");
        assert!(log.ends_with("dsplr: SIGTERM; stopped\n"), "{log}");
        log
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Stopped already, or a failed test's: either way it is reaped.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
