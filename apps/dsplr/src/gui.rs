//! `dsplr` with no command: the GUI (`docs/plan/06-gui.md`).
//!
//! The process model is `docs/plan/02-architecture.md`'s: the backend lives
//! on a thread of its own with a current-thread tokio runtime, and the main
//! thread owns the window and the editor. The window sends
//! [`Request`]s; the backend thread does each with the same code as the
//! CLI's commands (the confirm flow of [`apply::drive`], `dsplr save`'s and
//! `dsplr save --profile`'s steps, `dsplr restore --revert`'s) and answers
//! with [`Reply`]s. It reads the displays again after each request, when
//! asked to, and after each change the desktop reports through
//! [`Backend::watch`]. A watch that ends is not opened again: the bar says
//! so, and Refresh and every request still read the displays
//! (`docs/plan/02-architecture.md`).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use dsplr_backend::{Backend, Error, Watch};
use dsplr_core::flow::{self, Change, Outcome};
use dsplr_core::link::{Answer, Question, Reply, Request};
use dsplr_core::{Editor, Snapshot};
use dsplr_model::Layout;
use dsplr_persist::diff::line_diff;
use dsplr_persist::profile::{self as store, ProfileName};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::apply::{self, Confirm};
use crate::profile::{self, Blocked};
use crate::{Exit, config, find_backend, restore, runtime, save};

/// How long past the dialog's own timeout the backend thread waits for its
/// answer before taking no answer as a revert: the window counts down and
/// answers itself, so this only matters when the window cannot.
const ANSWER_GRACE: Duration = Duration::from_secs(2);

/// Reads the displays on the backend thread, then opens the window on them
/// and runs it until the user quits. A desktop dsplr has no backend for, or
/// displays it cannot read, end it before any window opens.
pub fn run() -> Exit {
    let config = match config() {
        Ok(config) => config,
        Err(exit) => return exit,
    };
    let confirm_timeout = config.confirm.timeout();
    let (started, first) = mpsc::channel();
    let (requests, incoming) = mpsc::channel();
    let (answers, answered) = mpsc::channel();
    let (outgoing, replies) = mpsc::channel();
    let spawned =
        thread::Builder::new()
            .name("dsplr-backend".into())
            .spawn(move || match runtime() {
                Ok(runtime) => runtime.block_on(serve(
                    &started,
                    (incoming, &answered),
                    &outgoing,
                    confirm_timeout,
                )),
                // The main thread waits for this, so the send cannot fail.
                Err(exit) => started.send(Err(exit)).unwrap_or_default(),
            });
    let backend = match spawned {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("dsplr: cannot start the backend thread: {error}");
            return Exit::Failure;
        }
    };
    let snapshot = match first.recv() {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(exit)) => return exit,
        Err(_) => {
            eprintln!("dsplr: the backend thread stopped before reading the displays");
            return Exit::Failure;
        }
    };
    let link = dsplr_gui::Link {
        requests,
        answers,
        replies,
    };
    let exit = match dsplr_gui::run(Editor::new(snapshot), link, &config.keys) {
        Ok(()) => Exit::Success,
        Err(error) => {
            eprintln!("dsplr: {error}");
            Exit::Failure
        }
    };
    // The window has dropped its end, so the thread finishes what it is
    // doing (a revert, say) and returns.
    if backend.join().is_err() {
        eprintln!("dsplr: the backend thread panicked");
        return Exit::Failure;
    }
    exit
}

/// The backend thread: finds the backend and sends the first snapshot on
/// `started`, then does each request from `incoming` until the window hangs
/// up, answering on `outgoing`; a confirm's answer comes on `answered`.
async fn serve(
    started: &Sender<Result<Snapshot, Exit>>,
    (incoming, answered): (Receiver<Request>, &Receiver<Answer>),
    outgoing: &Sender<Reply>,
    confirm_timeout: Duration,
) {
    let backend = match find_backend().await {
        Ok(backend) => backend,
        // The main thread waits for this, so the send cannot fail.
        Err(exit) => return started.send(Err(exit)).unwrap_or_default(),
    };
    let snapshot = match crate::snapshot(&backend).await {
        Ok(layout) => snapshot_of(&backend, layout),
        // The main thread waits for this, so the send cannot fail.
        Err(exit) => return started.send(Err(exit)).unwrap_or_default(),
    };
    if started.send(Ok(snapshot)).is_err() {
        return;
    }
    let mut thread = BackendThread {
        backend,
        answered,
        confirms: 0,
        outgoing: outgoing.clone(),
        confirm_timeout,
    };
    thread.profiles();
    let mut watch = match thread.backend.watch().await {
        Ok(watch) => Some(watch),
        Err(error) => {
            thread.say(format!(
                "cannot follow the displays' changes ({error}); Refresh reads them again"
            ));
            None
        }
    };
    let mut requests = bridge(incoming);
    loop {
        tokio::select! {
            request = requests.recv() => {
                // A closed channel is the window gone: nothing left to do.
                let Some(request) = request else {
                    return;
                };
                let outcome = thread.handle(request).await;
                if outgoing.send(Reply::Done(outcome)).is_err() {
                    return;
                }
            }
            changed = next_change(watch.as_mut()) => match changed {
                Ok(()) => thread.refresh().await,
                Err(error) => {
                    thread.say(format!(
                        "stopped following the displays' changes ({error}); Refresh reads them again"
                    ));
                    watch = None;
                }
            },
        }
    }
}

/// The next change `watch` reports; with no watch, never.
async fn next_change(watch: Option<&mut Watch>) -> Result<(), dsplr_backend::Error> {
    match watch {
        Some(watch) => watch.changed().await,
        None => std::future::pending().await,
    }
}

/// The window's requests, from its std channel onto a tokio one, so the
/// backend thread can wait on them and on the desktop's changes at once. The
/// bridge ends when the window hangs up, and the channel closes with it.
fn bridge(incoming: Receiver<Request>) -> UnboundedReceiver<Request> {
    let (forward, requests) = tokio::sync::mpsc::unbounded_channel();
    thread::spawn(move || {
        while let Ok(request) = incoming.recv() {
            if forward.send(request).is_err() {
                return;
            }
        }
    });
    requests
}

/// Sends `reply` on `outgoing`. The only error is the window gone, which
/// has nobody to tell; `serve` stops at its next request.
fn tell(outgoing: &Sender<Reply>, reply: Reply) {
    outgoing.send(reply).unwrap_or_default();
}

fn snapshot_of(backend: &Backend, layout: Layout) -> Snapshot {
    Snapshot {
        backend: backend.name().into(),
        capabilities: backend.capabilities(),
        layout,
    }
}

struct BackendThread<'a> {
    backend: Backend,
    answered: &'a Receiver<Answer>,
    /// Confirms asked so far: the next one's id.
    confirms: u64,
    outgoing: Sender<Reply>,
    confirm_timeout: Duration,
}

impl BackendThread<'_> {
    /// Sends `reply` to the window. A window that is gone has nobody to
    /// tell, and `serve` stops at its next request.
    fn tell(&self, reply: Reply) {
        tell(&self.outgoing, reply);
    }

    /// Says `line` in the window, as the CLI prints it on stderr.
    fn say(&self, line: String) {
        self.tell(Reply::Message(line));
    }

    async fn handle(&mut self, request: Request) -> Option<Outcome> {
        match request {
            Request::Refresh => {
                self.refresh().await;
                None
            }
            Request::Apply(asked) => {
                let outcome = self.apply(asked).await;
                self.refresh().await;
                outcome
            }
            Request::Save { approved } => {
                self.save(approved).await;
                None
            }
            Request::SaveProfile { name, replace } => {
                self.save_profile(&name, replace).await;
                self.profiles();
                None
            }
            Request::Restore(name) => {
                let outcome = self.restore(&name).await;
                self.refresh().await;
                outcome
            }
            Request::Revert => {
                let outgoing = self.outgoing.clone();
                let exit = restore::revert(&self.backend, &mut |line| {
                    tell(&outgoing, Reply::Message(line));
                })
                .await;
                if exit == Exit::Success {
                    self.say("went back to the revert point".to_owned());
                }
                self.refresh().await;
                None
            }
        }
    }

    /// Reads the displays and sends them.
    async fn refresh(&self) {
        match self.backend.snapshot().await {
            Ok(layout) => {
                self.tell(Reply::Snapshot(snapshot_of(&self.backend, layout)));
            }
            Err(error) => self.say(format!(
                "cannot read the displays from {}: {error}",
                self.backend.name()
            )),
        }
    }

    /// The names of the profiles that can be read, sent; the others said.
    fn profiles(&self) {
        let listed = match profile::store_dir()
            .and_then(|dir| store::list(&dir).map_err(|e| e.to_string()))
        {
            Ok(listed) => listed,
            Err(error) => {
                self.say(error);
                return;
            }
        };
        let mut names = Vec::new();
        for profile in listed {
            match profile {
                Ok(profile) => names.push(profile.name.as_str().to_owned()),
                Err(error) => self.say(error.to_string()),
            }
        }
        self.tell(Reply::Profiles(names));
    }

    /// `asked` from the window through the confirm flow, as `dsplr apply`
    /// does, confirmed in the window's dialogs.
    async fn apply(&mut self, asked: Layout) -> Option<Outcome> {
        let current = self.snapshot().await?;
        self.drive(current, asked).await
    }

    /// The profile `name`, onto the displays as they are now, through the
    /// confirm flow, as `dsplr restore NAME` does.
    async fn restore(&mut self, name: &str) -> Option<Outcome> {
        let name = self.profile_name(name)?;
        let dir = profile::store_dir().map_err(|error| self.say(error)).ok()?;
        let saved = match store::load(&dir, &name) {
            Ok(Some(saved)) => saved,
            Ok(None) => {
                self.say(format!("no profile named \"{name}\""));
                return None;
            }
            Err(error) => {
                self.say(error.to_string());
                return None;
            }
        };
        let current = self.snapshot().await?;
        let asked = match apply::bind(&saved.layout, &current) {
            Ok(asked) => asked,
            Err(unbound) => {
                for output in unbound {
                    self.say(output.to_string());
                }
                return Some(Outcome::Refused);
            }
        };
        self.say(format!("restoring profile \"{name}\""));
        self.drive(current, asked).await
    }

    async fn drive(&mut self, current: Layout, asked: Layout) -> Option<Outcome> {
        let change = Change {
            current,
            asked: asked.clone(),
            capabilities: self.backend.capabilities(),
            backend: self.backend.name().into(),
        };
        let options = flow::Options {
            yes: false,
            dry_run: false,
            can_ask: true,
        };
        let mut window = Window {
            answered: self.answered,
            outgoing: &self.outgoing,
            asked,
            confirms: &mut self.confirms,
        };
        let outgoing = self.outgoing.clone();
        let outcome = apply::drive(
            &self.backend,
            change,
            options,
            self.confirm_timeout,
            &mut window,
            &mut |line| {
                tell(&outgoing, Reply::Message(line));
            },
        )
        .await;
        Some(outcome)
    }

    /// `dsplr save`: the displays as they are now, where the desktop reads
    /// them. A file that needs the user's go-ahead is asked about first.
    async fn save(&self, approved: Option<String>) {
        let Some(layout) = self.snapshot().await else {
            return;
        };
        match self.backend.persist(&layout, approved.as_deref()).await {
            Ok(persisted) => {
                for message in save::saved_messages(&persisted) {
                    self.say(message);
                }
            }
            Err(Error::Conflict(conflict)) if approved.is_some() => self.say(format!(
                "{} changed after it was shown, so nothing was saved; save again",
                conflict.path.display()
            )),
            Err(Error::Conflict(conflict)) => {
                self.tell(Reply::Ask(Question {
                    what: conflict.path.display().to_string(),
                    notes: save::conflict_notes(&conflict),
                    diff: line_diff(&conflict.existing, &conflict.new),
                    approve: Request::Save {
                        approved: Some(conflict.existing.clone()),
                    },
                }));
            }
            Err(error) => self.say(format!("cannot save on {}: {error}", self.backend.name())),
        }
    }

    /// `dsplr save --profile NAME`: a profile of that name is replaced only
    /// once the user has seen what changes.
    async fn save_profile(&self, name: &str, replace: bool) {
        let Some(name) = self.profile_name(name) else {
            return;
        };
        let Some(layout) = self.snapshot().await else {
            return;
        };
        if layout.entries.is_empty() {
            self.say(format!(
                "{} reports no outputs, so there is nothing to save",
                self.backend.name()
            ));
            return;
        }
        let dir = match profile::store_dir() {
            Ok(dir) => dir,
            Err(error) => return self.say(error),
        };
        let what = format!("profile \"{name}\"");
        let approve = Request::SaveProfile {
            name: name.as_str().to_owned(),
            replace: true,
        };
        let question = match profile::store_layout(&dir, &name, layout, replace) {
            Ok(messages) => {
                for message in messages {
                    self.say(message);
                }
                return;
            }
            Err(Blocked::Failed(message)) => return self.say(message),
            Err(Blocked::Exists { diff }) => Question {
                what,
                notes: vec![format!("there is a profile named \"{name}\" already")],
                diff,
                approve,
            },
            Err(Blocked::Unreadable(error)) => Question {
                what,
                notes: vec![format!("{error}; replacing it loses it")],
                diff: String::new(),
                approve,
            },
        };
        self.tell(Reply::Ask(question));
    }

    fn profile_name(&self, name: &str) -> Option<ProfileName> {
        name.parse::<ProfileName>()
            .map_err(|error| self.say(error.to_string()))
            .ok()
    }

    /// The displays as they are now, or `None` with why said.
    async fn snapshot(&self) -> Option<Layout> {
        self.backend
            .snapshot()
            .await
            .map_err(|error| {
                self.say(format!(
                    "cannot read the displays from {}: {error}",
                    self.backend.name()
                ));
            })
            .ok()
    }
}

/// The window, asked to confirm in its dialogs.
struct Window<'a> {
    answered: &'a Receiver<Answer>,
    outgoing: &'a Sender<Reply>,
    asked: Layout,
    confirms: &'a mut u64,
}

impl Confirm for Window<'_> {
    /// Asks the window, and waits for its answer: the window counts down to
    /// the deadline sent and answers no itself there; a window that does
    /// not answer [`ANSWER_GRACE`] past that, or is gone, is a no too. An
    /// answer for another id was for an earlier dialog, already ended.
    /// Requests sent meanwhile wait in their own queue.
    fn confirm(&mut self, timeout: Duration, watchdog: Instant) -> bool {
        *self.confirms += 1;
        let id = *self.confirms;
        let deadline = Instant::now() + timeout;
        let asked = Reply::Confirm {
            id,
            deadline,
            watchdog,
            layout: self.asked.clone(),
        };
        if self.outgoing.send(asked).is_err() {
            return false;
        }
        let give_up = deadline + ANSWER_GRACE;
        loop {
            let left = give_up.saturating_duration_since(Instant::now());
            match self.answered.recv_timeout(left) {
                Ok(answer) if answer.id == id => return answer.keep,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use dsplr_core::link::{Answer, Reply};
    use dsplr_model::Layout;

    use super::Window;
    use crate::apply::Confirm;

    /// The window's side, answering each confirm with `answers` in turn: an
    /// answer naming the confirm's own id where `None`.
    fn window_answering(answers: Vec<(Option<u64>, bool)>) -> bool {
        let (outgoing, replies) = mpsc::channel();
        let (answer, answered) = mpsc::channel();
        let window = thread::spawn(move || {
            let Ok(Reply::Confirm { id, deadline, .. }) = replies.recv() else {
                panic!("no confirm asked");
            };
            assert!(deadline > Instant::now(), "the deadline is ahead");
            for (other, keep) in answers {
                answer
                    .send(Answer {
                        id: other.unwrap_or(id),
                        keep,
                    })
                    .unwrap();
            }
        });
        let mut confirms = 0;
        let mut confirmer = Window {
            answered: &answered,
            outgoing: &outgoing,
            asked: Layout::default(),
            confirms: &mut confirms,
        };
        let kept = confirmer.confirm(Duration::from_millis(200), Instant::now());
        window.join().unwrap();
        kept
    }

    #[test]
    fn a_confirm_takes_its_own_answer_and_ignores_an_earlier_dialog_s() {
        assert!(window_answering(vec![(Some(99), false), (None, true)]));
        assert!(!window_answering(vec![(None, false)]));
    }

    #[test]
    fn no_answer_by_the_deadline_is_no() {
        assert!(!window_answering(Vec::new()));
        assert!(!window_answering(vec![(Some(99), true)]), "only another's");
    }
}
