//! Apply, confirm, revert (`docs/plan/02-architecture.md`, steps 1 to 6) as
//! a state machine with no I/O: the one flow `dsplr apply`, `dsplr set`,
//! `dsplr restore` and the GUI's Apply drive. It says which [`Step`] comes
//! next; the driver does it, against the backend, the revert point file, the
//! watchdog and whoever confirms (a terminal, a dialog), and hands the
//! result back as an [`Answer`]. What it has to say to the user it keeps as
//! [`Flow::messages`], the same lines for every front end.

use dsplr_model::{Capabilities, Difference, Layout, Problem, Setting};

/// A change to make: from `current`, the snapshot just read, to `asked`, a
/// whole layout naming the live outputs.
#[derive(Debug, Clone)]
pub struct Change {
    pub current: Layout,
    pub asked: Layout,
    pub capabilities: Capabilities,
    /// The backend's name, for the messages.
    pub backend: String,
}

/// How the flow was asked to run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Keep the change without asking.
    pub yes: bool,
    /// Validate and test only.
    pub dry_run: bool,
    /// The front end can ask (a terminal on stdin, a window). Without it, a
    /// change that needs confirming is refused rather than kept unasked.
    pub can_ask: bool,
}

/// What the driver does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The backend's test of `asked`; answered with [`Answer::Tested`].
    Test,
    /// Write the revert point (the `current` layout) and start its
    /// watchdog; [`Answer::Prepared`].
    Prepare,
    /// Apply `asked`; [`Answer::Applied`].
    Apply,
    /// Read the layout back until it shows `asked`, or the settle time
    /// passes; [`Answer::Settled`] with what still differs.
    Settle,
    /// Read the layout once more after the desktop dropped a request
    /// because the displays changed; [`Answer::Shown`].
    ShowNow,
    /// Remove the revert point of a change that applied nothing;
    /// [`Answer::Discarded`].
    Discard,
    /// Ask whether to keep the change, reverting on no answer;
    /// [`Answer::Confirmed`].
    Confirm,
    /// Go back to the revert point; [`Answer::Reverted`].
    Revert,
    /// Mark the change kept, so the watchdog leaves it; [`Answer::Kept`].
    Keep,
    /// Finished.
    Done(Outcome),
}

/// How a change ended, which the CLI turns into its exit code
/// (`docs/plan/07-cli.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Applied and kept (exit 0).
    Kept,
    /// A dry run that would apply it (exit 0).
    Tested,
    /// dsplr refused it before asking the desktop: validation, or a layout
    /// the backend cannot express (exit 3).
    Refused,
    /// The desktop refused it, or the displays changed meanwhile (exit 4).
    BackendRefused,
    /// Declined or not confirmed in time, and reverted (exit 5).
    Reverted,
    /// Anything else (exit 1).
    Failed,
}

/// Why a backend request failed, as the flow tells them apart
/// (`dsplr_backend::Error`'s predicates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The backend has no such request (a test on Hyprland): the reason.
    Unsupported(String),
    /// The backend cannot put the layout into the desktop's request, so it
    /// sent nothing.
    NotTaken(String),
    /// The desktop refused it.
    Refused(String),
    /// The displays changed since they were read.
    Stale(String),
    /// Anything else: a timeout, a lost connection.
    Failed(String),
}

impl Failure {
    fn text(&self) -> &str {
        match self {
            Self::Unsupported(text)
            | Self::NotTaken(text)
            | Self::Refused(text)
            | Self::Stale(text)
            | Self::Failed(text) => text,
        }
    }
}

/// The result of the step the flow asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Tested(Result<(), Failure>),
    /// The error says what could not be done, one whole line each.
    Prepared(Result<(), Vec<String>>),
    Applied(Result<(), Failure>),
    Settled(Result<Vec<Difference>, String>),
    Shown(Result<Vec<Difference>, String>),
    Discarded(Result<(), String>),
    /// Whether the user kept the change; no answer in time is `false`.
    Confirmed(bool),
    /// Whether the revert point's layout was applied.
    Reverted(bool),
    /// `Ok(false)` when the revert point was replaced by another change.
    Kept(Result<bool, String>),
}

/// Where the flow is: the step it is waiting on, and what follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Test,
    Prepare,
    Apply,
    Settle,
    /// A re-read, then this outcome.
    ShowNow(Outcome),
    /// The revert point removed, then a re-read (`true`) or this outcome.
    Discard {
        show: bool,
        then: Outcome,
    },
    Confirm,
    /// Going back, then this outcome whatever the revert did; or, for a
    /// declined change, [`Outcome::Reverted`] when it went back.
    Revert(AfterRevert),
    Keep,
    Done(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterRevert {
    Always(Outcome),
    Declined,
}

/// One change on its way through the flow.
#[derive(Debug, Clone)]
pub struct Flow {
    change: Change,
    options: Options,
    /// The change can hide or move a screen, and is not kept unasked.
    ask: bool,
    waiting: Waiting,
    messages: Vec<String>,
}

impl Flow {
    /// Starts `change`: validates it (step 1) and returns the flow and its
    /// first step, the test, or [`Step::Done`] when validation refuses it.
    pub fn start(change: Change, options: Options) -> (Self, Step) {
        let problems = change.asked.validate(&change.capabilities);
        let ask =
            !options.yes && needs_confirm(&change.asked, &change.current, &change.capabilities);
        let mut flow = Self {
            change,
            options,
            ask,
            waiting: Waiting::Test,
            messages: Vec::new(),
        };
        for problem in &problems {
            let severity = if problem.is_error() {
                "error"
            } else {
                "warning"
            };
            flow.say(format!("{severity}: {problem}"));
        }
        let step = if problems.iter().any(Problem::is_error) {
            flow.done(Outcome::Refused)
        } else {
            Step::Test
        };
        (flow, step)
    }

    /// Whether the change will be confirmed before it is kept.
    pub const fn asks(&self) -> bool {
        self.ask
    }

    /// The change on its way.
    pub const fn change(&self) -> &Change {
        &self.change
    }

    /// What the flow has had to say since this was last called, one line
    /// each (a report of differences runs over several).
    pub fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// Takes the result of the step last returned, and returns the next.
    ///
    /// # Panics
    ///
    /// When `answer` is not the answer to that step: a driver bug.
    pub fn next(&mut self, answer: Answer) -> Step {
        let backend = self.change.backend.clone();
        match (self.waiting, answer) {
            (Waiting::Test, Answer::Tested(result)) => self.tested(&backend, result),
            (Waiting::Prepare, Answer::Prepared(Ok(()))) => {
                self.waiting = Waiting::Apply;
                Step::Apply
            }
            (Waiting::Prepare, Answer::Prepared(Err(why))) => {
                for line in why {
                    self.say(line);
                }
                self.done(Outcome::Failed)
            }
            (Waiting::Apply, Answer::Applied(result)) => self.applied(&backend, result),
            (Waiting::Settle, Answer::Settled(Ok(differences))) => {
                self.report(&differences);
                if self.ask {
                    self.waiting = Waiting::Confirm;
                    Step::Confirm
                } else {
                    self.keep()
                }
            }
            (Waiting::Settle, Answer::Settled(Err(error))) => {
                // Without a re-read nothing says what the screens show: go
                // back rather than keep it unseen.
                self.say(format!("cannot read the displays back: {error}"));
                self.revert(AfterRevert::Always(Outcome::Failed))
            }
            (Waiting::ShowNow(then), Answer::Shown(result)) => {
                match result {
                    Ok(differences) => self.report(&differences),
                    Err(error) => self.say(format!("cannot read the displays again: {error}")),
                }
                self.done(then)
            }
            (Waiting::Discard { show, then }, Answer::Discarded(result)) => {
                if let Err(error) = result {
                    self.say(error);
                }
                if show {
                    self.waiting = Waiting::ShowNow(then);
                    Step::ShowNow
                } else {
                    self.done(then)
                }
            }
            (Waiting::Confirm, Answer::Confirmed(true)) => self.keep(),
            (Waiting::Confirm, Answer::Confirmed(false)) => {
                self.say("reverting".to_owned());
                self.revert(AfterRevert::Declined)
            }
            (Waiting::Revert(after), Answer::Reverted(went_back)) => self.done(match after {
                AfterRevert::Always(outcome) => outcome,
                // A revert that failed leaves the declined layout on screen,
                // so nothing may read as "nothing was applied".
                AfterRevert::Declined if went_back => Outcome::Reverted,
                AfterRevert::Declined => Outcome::Failed,
            }),
            (Waiting::Keep, Answer::Kept(Ok(true))) => self.done(Outcome::Kept),
            (Waiting::Keep, Answer::Kept(Ok(false))) => {
                self.say(
                    "the revert point was replaced by another change; this one is kept".to_owned(),
                );
                self.done(Outcome::Kept)
            }
            (Waiting::Keep, Answer::Kept(Err(error))) => {
                self.say(format!(
                    "cannot mark the change kept, so the watchdog will revert it: {error}"
                ));
                self.done(Outcome::Failed)
            }
            (waiting, answer) => {
                panic!("the flow waits on {waiting:?} and was answered {answer:?}")
            }
        }
    }

    /// Step 2's result.
    fn tested(&mut self, backend: &str, result: Result<(), Failure>) -> Step {
        let dry_run = self.options.dry_run;
        match result {
            Ok(()) if dry_run => {
                self.say(format!("{backend} tested the layout and would apply it"));
                return self.done(Outcome::Tested);
            }
            Err(Failure::Unsupported(reason)) if dry_run => {
                self.say(format!(
                    "validated locally only; {backend} cannot test a layout: {reason}"
                ));
                return self.done(Outcome::Tested);
            }
            // Passed, or a backend with no real test (Hyprland, KDE): on.
            Ok(()) | Err(Failure::Unsupported(_)) => {}
            Err(Failure::NotTaken(error)) => {
                self.cannot_take(backend, &error);
                return self.done(Outcome::Refused);
            }
            Err(Failure::Refused(error)) => {
                self.say(format!(
                    "{backend} refused the layout in its test, so nothing was applied: {error}"
                ));
                return self.done(Outcome::BackendRefused);
            }
            Err(Failure::Stale(error)) => {
                self.say(format!(
                    "the displays changed while {backend} tested the layout, so nothing was applied: {error}"
                ));
                self.waiting = Waiting::ShowNow(Outcome::BackendRefused);
                return Step::ShowNow;
            }
            Err(Failure::Failed(error)) => {
                self.say(format!(
                    "cannot test the layout on {backend}, so nothing was applied: {error}"
                ));
                return self.done(Outcome::Failed);
            }
        }
        if self.ask && !self.options.can_ask {
            self.say(
                "this change can hide or move a screen, and there is no terminal to confirm it on; pass --yes to keep it without asking".to_owned(),
            );
            return self.done(Outcome::Failed);
        }
        self.waiting = Waiting::Prepare;
        Step::Prepare
    }

    /// Step 3's apply.
    fn applied(&mut self, backend: &str, result: Result<(), Failure>) -> Step {
        let Err(failure) = result else {
            self.waiting = Waiting::Settle;
            return Step::Settle;
        };
        let error = failure.text().to_owned();
        match failure {
            // Nothing was applied, and the displays changed since the revert
            // point was read: going back would undo that other change. Show
            // what they are now instead.
            Failure::Stale(_) => {
                self.say(format!(
                    "the displays changed before {backend} applied the layout, so nothing was applied: {error}"
                ));
                self.waiting = Waiting::Discard {
                    show: true,
                    then: Outcome::BackendRefused,
                };
                Step::Discard
            }
            // Nothing was sent, so there is nothing to go back from.
            Failure::NotTaken(_) => {
                self.cannot_take(backend, &error);
                self.waiting = Waiting::Discard {
                    show: false,
                    then: Outcome::Refused,
                };
                Step::Discard
            }
            // Refused or not, something may have changed (a connection lost
            // mid-apply), so go back either way.
            Failure::Refused(_) => {
                self.say(format!("{backend} refused the layout: {error}"));
                self.revert(AfterRevert::Always(Outcome::BackendRefused))
            }
            Failure::Unsupported(_) | Failure::Failed(_) => {
                self.say(format!("cannot apply the layout on {backend}: {error}"));
                self.revert(AfterRevert::Always(Outcome::Failed))
            }
        }
    }

    fn keep(&mut self) -> Step {
        self.waiting = Waiting::Keep;
        Step::Keep
    }

    fn revert(&mut self, after: AfterRevert) -> Step {
        self.waiting = Waiting::Revert(after);
        Step::Revert
    }

    fn done(&mut self, outcome: Outcome) -> Step {
        self.waiting = Waiting::Done(outcome);
        Step::Done(outcome)
    }

    fn cannot_take(&mut self, backend: &str, error: &str) {
        self.say(format!(
            "{backend} cannot take the layout, so nothing was applied: {error}"
        ));
    }

    /// What the desktop did not take as asked, if anything.
    fn report(&mut self, differences: &[Difference]) {
        if let Some(report) = report(differences) {
            self.say(report);
        }
    }

    fn say(&mut self, line: String) {
        self.messages.push(line);
    }
}

/// `differences` as one message: a line saying the displays differ from
/// what was asked, then one indented line each. `None` when there are none.
pub fn report(differences: &[Difference]) -> Option<String> {
    if differences.is_empty() {
        return None;
    }
    let mut text = "the displays show something other than was asked:".to_owned();
    for difference in differences {
        text.push_str("\n  ");
        text.push_str(&difference.to_string());
    }
    Some(text)
}

/// Whether going from `current` to `asked` can hide or move a screen: an
/// output enabled or disabled, or its mode, position, scale or transform
/// changed, or the layout mode, which changes every output's size.
pub fn needs_confirm(asked: &Layout, current: &Layout, capabilities: &Capabilities) -> bool {
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
            Difference::Missing(_) | Difference::Added(_) | Difference::LayoutMode { .. } => true,
        })
}

#[cfg(test)]
mod tests {
    use dsplr_model::{AdaptiveSync, Difference};

    use super::{Answer, Change, Failure, Flow, Options, Outcome, Step};
    use crate::fixtures::{three_monitors, wlr};

    /// What a fake backend and user answer each step with.
    struct Fake {
        test: Result<(), Failure>,
        prepare: Result<(), Vec<String>>,
        apply: Result<(), Failure>,
        settle: Result<Vec<Difference>, String>,
        confirm: bool,
        revert: bool,
        keep: Result<bool, String>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                test: Ok(()),
                prepare: Ok(()),
                apply: Ok(()),
                settle: Ok(Vec::new()),
                confirm: true,
                revert: true,
                keep: Ok(true),
            }
        }
    }

    /// A change that moves an output, so it is confirmed.
    fn moved() -> Change {
        let current = three_monitors().layout;
        let mut asked = current.clone();
        asked.entries[0].state.y = 0;
        Change {
            current,
            asked,
            capabilities: wlr(),
            backend: "wlr".into(),
        }
    }

    fn can_ask() -> Options {
        Options {
            can_ask: true,
            ..Options::default()
        }
    }

    /// Drives `change` against `fake`: every step taken, the outcome, and
    /// every message.
    fn drive(change: Change, options: Options, fake: &Fake) -> (Vec<Step>, Outcome, Vec<String>) {
        let (mut flow, mut step) = Flow::start(change, options);
        let mut steps = Vec::new();
        loop {
            steps.push(step.clone());
            let answer = match step {
                Step::Done(outcome) => return (steps, outcome, flow.messages()),
                Step::Test => Answer::Tested(fake.test.clone()),
                Step::Prepare => Answer::Prepared(fake.prepare.clone()),
                Step::Apply => Answer::Applied(fake.apply.clone()),
                Step::Settle => Answer::Settled(fake.settle.clone()),
                Step::ShowNow => Answer::Shown(Ok(Vec::new())),
                Step::Discard => Answer::Discarded(Ok(())),
                Step::Confirm => Answer::Confirmed(fake.confirm),
                Step::Revert => Answer::Reverted(fake.revert),
                Step::Keep => Answer::Kept(fake.keep.clone()),
            };
            step = flow.next(answer);
        }
    }

    use Step::{Apply, Confirm, Discard, Done, Keep, Prepare, Revert, Settle, ShowNow, Test};

    #[test]
    fn a_kept_change_is_tested_guarded_applied_read_back_confirmed_and_kept() {
        let (steps, outcome, messages) = drive(moved(), can_ask(), &Fake::default());
        assert_eq!(
            steps,
            [
                Test,
                Prepare,
                Apply,
                Settle,
                Confirm,
                Keep,
                Done(Outcome::Kept)
            ]
        );
        assert_eq!(outcome, Outcome::Kept);
        assert_eq!(messages, Vec::<String>::new());
    }

    /// A no and no answer in time are one answer to the flow: the driver's
    /// timeout gives `false`.
    #[test]
    fn a_declined_or_timed_out_change_is_reverted() {
        let fake = Fake {
            confirm: false,
            ..Fake::default()
        };
        let (steps, outcome, messages) = drive(moved(), can_ask(), &fake);
        assert_eq!(
            steps,
            [
                Test,
                Prepare,
                Apply,
                Settle,
                Confirm,
                Revert,
                Done(Outcome::Reverted)
            ]
        );
        assert_eq!(outcome, Outcome::Reverted);
        assert_eq!(messages, ["reverting"]);

        let failed_revert = Fake {
            revert: false,
            ..fake
        };
        assert_eq!(
            drive(moved(), can_ask(), &failed_revert).1,
            Outcome::Failed,
            "the declined layout is still on screen"
        );
    }

    #[test]
    fn a_layout_the_test_refuses_applies_nothing() {
        let fake = Fake {
            test: Err(Failure::Refused("no".into())),
            ..Fake::default()
        };
        let (steps, outcome, messages) = drive(moved(), can_ask(), &fake);
        assert_eq!(steps, [Test, Done(Outcome::BackendRefused)]);
        assert_eq!(outcome, Outcome::BackendRefused);
        assert_eq!(
            messages,
            ["wlr refused the layout in its test, so nothing was applied: no"]
        );

        let not_taken = Fake {
            test: Err(Failure::NotTaken("no mirrors".into())),
            ..Fake::default()
        };
        let (steps, outcome, _) = drive(moved(), can_ask(), &not_taken);
        assert_eq!((steps.len(), outcome), (2, Outcome::Refused));
    }

    #[test]
    fn displays_that_changed_meanwhile_are_shown_and_nothing_is_kept() {
        let at_test = Fake {
            test: Err(Failure::Stale("serial".into())),
            ..Fake::default()
        };
        let (steps, outcome, messages) = drive(moved(), can_ask(), &at_test);
        assert_eq!(steps, [Test, ShowNow, Done(Outcome::BackendRefused)]);
        assert_eq!(outcome, Outcome::BackendRefused);
        assert!(messages[0].starts_with("the displays changed while wlr tested"));

        // Stale at the apply: the revert point goes, and nothing is reverted,
        // which would undo the other change.
        let at_apply = Fake {
            apply: Err(Failure::Stale("cancelled".into())),
            ..Fake::default()
        };
        let (steps, outcome, _) = drive(moved(), can_ask(), &at_apply);
        assert_eq!(
            steps,
            [
                Test,
                Prepare,
                Apply,
                Discard,
                ShowNow,
                Done(Outcome::BackendRefused)
            ]
        );
        assert_eq!(outcome, Outcome::BackendRefused);
    }

    #[test]
    fn a_failed_apply_or_read_back_goes_back() {
        let refused = Fake {
            apply: Err(Failure::Refused("failed".into())),
            ..Fake::default()
        };
        let (steps, outcome, messages) = drive(moved(), can_ask(), &refused);
        assert_eq!(
            steps,
            [Test, Prepare, Apply, Revert, Done(Outcome::BackendRefused)]
        );
        assert_eq!(messages, ["wlr refused the layout: failed"]);
        assert_eq!(outcome, Outcome::BackendRefused);

        let unread = Fake {
            settle: Err("timed out".into()),
            ..Fake::default()
        };
        let (steps, outcome, _) = drive(moved(), can_ask(), &unread);
        assert_eq!(
            steps,
            [Test, Prepare, Apply, Settle, Revert, Done(Outcome::Failed)]
        );
        assert_eq!(outcome, Outcome::Failed);

        let not_taken = Fake {
            apply: Err(Failure::NotTaken("no".into())),
            ..Fake::default()
        };
        let (steps, outcome, _) = drive(moved(), can_ask(), &not_taken);
        assert_eq!(
            steps,
            [Test, Prepare, Apply, Discard, Done(Outcome::Refused)]
        );
        assert_eq!(outcome, Outcome::Refused);
    }

    #[test]
    fn a_dry_run_stops_after_the_test() {
        let dry = Options {
            dry_run: true,
            ..can_ask()
        };
        let (steps, outcome, messages) = drive(moved(), dry, &Fake::default());
        assert_eq!((steps.len(), outcome), (2, Outcome::Tested));
        assert_eq!(messages, ["wlr tested the layout and would apply it"]);

        let no_test = Fake {
            test: Err(Failure::Unsupported("a stub".into())),
            ..Fake::default()
        };
        let (_, outcome, messages) = drive(moved(), dry, &no_test);
        assert_eq!(outcome, Outcome::Tested);
        assert!(
            messages[0].starts_with("validated locally only"),
            "{messages:?}"
        );
        // Without a dry run, a backend with no test goes on.
        assert_eq!(drive(moved(), can_ask(), &no_test).1, Outcome::Kept);
    }

    #[test]
    fn a_change_that_needs_confirming_is_refused_with_no_way_to_ask() {
        let (steps, outcome, messages) = drive(moved(), Options::default(), &Fake::default());
        assert_eq!(steps, [Test, Done(Outcome::Failed)]);
        assert!(messages[0].contains("no terminal"), "{messages:?}");

        let yes = Options {
            yes: true,
            ..Options::default()
        };
        let (steps, outcome_yes, _) = drive(moved(), yes, &Fake::default());
        assert_eq!(outcome_yes, Outcome::Kept);
        assert!(!steps.contains(&Confirm), "kept unasked");
        assert_eq!(outcome, Outcome::Failed);
    }

    #[test]
    fn a_change_that_moves_no_screen_is_kept_unasked() {
        let mut change = moved();
        change.asked = change.current.clone();
        change.asked.entries[0].state.adaptive_sync = Some(AdaptiveSync::On);
        let (steps, outcome, _) = drive(change, can_ask(), &Fake::default());
        assert!(!steps.contains(&Confirm));
        assert_eq!(outcome, Outcome::Kept);
    }

    #[test]
    fn a_layout_validation_refuses_is_refused_before_the_desktop_is_asked() {
        let mut change = moved();
        change.asked.entries[1].state.x = 0;
        change.asked.entries[1].state.y = 360;
        let (steps, outcome, messages) = drive(change, can_ask(), &Fake::default());
        assert_eq!(steps, [Done(Outcome::Refused)]);
        assert_eq!(outcome, Outcome::Refused);
        assert!(
            messages.iter().any(|m| m.starts_with("error: ")),
            "{messages:?}"
        );
    }

    #[test]
    fn a_failed_guard_or_keep_is_reported() {
        let unguarded = Fake {
            prepare: Err(vec![
                "cannot start the revert watchdog: no such file".into(),
                "cannot remove the revert point: read-only".into(),
            ]),
            ..Fake::default()
        };
        let (steps, outcome, messages) = drive(moved(), can_ask(), &unguarded);
        assert_eq!(steps, [Test, Prepare, Done(Outcome::Failed)]);
        assert_eq!(
            messages,
            [
                "cannot start the revert watchdog: no such file",
                "cannot remove the revert point: read-only"
            ],
            "a line each, so each gets the CLI's prefix"
        );
        assert_eq!(outcome, Outcome::Failed);

        let replaced = Fake {
            keep: Ok(false),
            ..Fake::default()
        };
        let (_, outcome, messages) = drive(moved(), can_ask(), &replaced);
        assert_eq!(outcome, Outcome::Kept);
        assert!(messages[0].contains("replaced by another change"));

        let unkept = Fake {
            keep: Err("read-only".into()),
            ..Fake::default()
        };
        let (_, outcome, messages) = drive(moved(), can_ask(), &unkept);
        assert_eq!(outcome, Outcome::Failed);
        assert!(messages[0].contains("the watchdog will revert it"));
    }

    #[test]
    fn differences_after_the_apply_are_reported() {
        let current = three_monitors().layout;
        let fake = Fake {
            settle: Ok(moved().asked.differences(&current, &wlr())),
            ..Fake::default()
        };
        let (_, _, messages) = drive(moved(), can_ask(), &fake);
        assert!(
            messages[0].starts_with("the displays show something other than was asked:\n  "),
            "{messages:?}"
        );
    }

    #[test]
    fn only_a_change_that_can_hide_or_move_a_screen_needs_confirming() {
        use dsplr_model::{Capabilities, Transform};

        use super::needs_confirm;

        let current = three_monitors().layout;
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

        // GNOME's layout mode alone resizes every output.
        let mut logical = current.clone();
        logical.layout_mode = Some(dsplr_model::LayoutMode::Logical);
        let mut physical = current.clone();
        physical.layout_mode = Some(dsplr_model::LayoutMode::Physical);
        assert!(!needs_confirm(&logical, &logical, &caps));
        assert!(needs_confirm(&physical, &logical, &caps));
    }
}
