//! What the window and the backend thread say to each other
//! (`docs/plan/02-architecture.md`, "Process"): plain data, so neither side
//! names the other's types. The window sends [`Request`]s; the backend
//! thread answers with [`Reply`]s, and every request ends with
//! [`Reply::Done`]. The answer to a [`Reply::Confirm`] goes on a channel of
//! its own as an [`Answer`], so a request is never read, or dropped, while
//! the backend thread waits for one: it waits in its queue.

use std::time::Instant;

use dsplr_model::Layout;

use crate::Snapshot;
use crate::flow::Outcome;

/// What the window asks of the backend thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Read the displays again.
    Refresh,
    /// Take the displays to this layout through the confirm flow
    /// ([`crate::flow`]), as `dsplr apply` does.
    Apply(Layout),
    /// Save the displays as they are now where the desktop reads them, as
    /// `dsplr save` does. `approved` is the text of the file the user agreed
    /// to replace, from a [`Question`].
    Save { approved: Option<String> },
    /// Save the displays as they are now as the profile `name`, as
    /// `dsplr save --profile` does. `approved` is the text of the profile of
    /// that name the user agreed to replace, from a [`Question`]; one that
    /// holds anything else is asked about.
    SaveProfile {
        name: String,
        approved: Option<String>,
    },
    /// Apply the profile `name` through the confirm flow, as
    /// `dsplr restore NAME` does.
    Restore(String),
    /// Go back to the revert point, as `dsplr restore --revert` does.
    Revert,
}

/// The answer to the [`Reply::Confirm`] with the same `id`: keep the change.
/// An answer for another id was for a dialog that has already ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer {
    pub id: u64,
    pub keep: bool,
}

/// What the backend thread tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// The displays as they are now.
    Snapshot(Snapshot),
    /// The names of the profiles in the store.
    Profiles(Vec<String>),
    /// A line to show, as the CLI prints it after `dsplr: `.
    Message(String),
    /// Ask whether to keep the change just applied, `layout`, until
    /// `deadline`; answer with an [`Answer`] for `id`. No answer by then is
    /// a revert. Should the backend thread stop before it can revert, the
    /// watchdog does at `watchdog`, the one way back left.
    Confirm {
        id: u64,
        deadline: Instant,
        watchdog: Instant,
        layout: Layout,
    },
    /// Something needs the user's go-ahead first: answer with
    /// [`Question::approve`], or with nothing to drop it.
    Ask(Question),
    /// The request is finished; for an apply, a restore or a revert, how it
    /// ended.
    Done(Option<Outcome>),
}

/// A question the backend thread puts before going on, as `dsplr save`
/// asks on its terminal: what to show, and the request that says yes.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// What would be replaced: a path, or `profile "NAME"`.
    pub what: String,
    /// Why it is asked, one line each.
    pub notes: Vec<String>,
    /// What replacing it changes, as a line diff.
    pub diff: String,
    pub approve: Request,
}
