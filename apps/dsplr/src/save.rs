//! `dsplr save`: the displays as they are now, saved where the desktop reads
//! them when it starts (`docs/plan/05-persistence-and-profiles.md`).

use std::io::{self, BufRead, IsTerminal};

use dsplr_backend::{Backend, Error, Persisted};
use dsplr_persist::diff::line_diff;
use dsplr_persist::managed::{Conflict, Ownership, Saved};

use crate::Exit;
use crate::apply::is_yes;

/// How `dsplr save` was asked to run.
pub struct Options {
    /// Overwrite whatever is there: a file edited by hand, or one with
    /// settings dsplr cannot carry over.
    pub force: bool,
    /// Replace a file someone else wrote without asking.
    pub yes: bool,
}

/// Saves the current state of `backend`'s displays.
pub async fn run(backend: &Backend, options: &Options) -> Exit {
    let layout = match backend.snapshot().await {
        Ok(layout) => layout,
        Err(error) => {
            eprintln!(
                "dsplr: cannot read the displays from {}: {error}",
                backend.name()
            );
            return Exit::Failure;
        }
    };
    let conflict = match backend.persist(&layout, None) {
        Ok(persisted) => return saved(&persisted),
        Err(Error::Conflict(conflict)) => conflict,
        Err(error) => return failed(backend, &error),
    };
    if !allowed(&conflict, options) {
        return Exit::Failure;
    }
    match backend.persist(&layout, Some(&conflict.existing)) {
        Ok(persisted) => saved(&persisted),
        Err(Error::Conflict(changed)) => {
            eprintln!(
                "dsplr: {} changed after it was shown, so nothing was saved; run dsplr save again",
                changed.path.display()
            );
            Exit::Failure
        }
        Err(error) => failed(backend, &error),
    }
}

fn failed(backend: &Backend, error: &Error) -> Exit {
    eprintln!("dsplr: cannot save on {}: {error}", backend.name());
    if matches!(error, Error::Unsupported { .. }) {
        Exit::BackendRefused
    } else {
        Exit::Failure
    }
}

/// Shows what saving would change in the file of `conflict`, and whether
/// the user allows it: `--force`, or for a readable file someone else
/// wrote, `--yes` or a yes on the terminal.
fn allowed(conflict: &Conflict, options: &Options) -> bool {
    let path = conflict.path.display();
    match (&conflict.unreadable, conflict.ownership) {
        (Some(reason), _) => eprintln!(
            "dsplr: {path} holds what dsplr cannot keep, so saving would lose it: {reason}"
        ),
        (None, Ownership::EditedByHand) => {
            eprintln!("dsplr: {path} was edited by hand since dsplr wrote it");
        }
        (None, Ownership::Foreign) => eprintln!(
            "dsplr: {path} was not written by dsplr (nwg-displays writes one too); dsplr replaces it with its own"
        ),
        (None, Ownership::Managed) => {}
    }
    eprintln!("This is what saving changes in it:");
    eprint!("{}", line_diff(&conflict.existing, &conflict.new));
    if options.force {
        return true;
    }
    if conflict.needs_force() {
        eprintln!("dsplr: nothing was saved; --force replaces the file");
        return false;
    }
    if options.yes {
        return true;
    }
    if !io::stdin().is_terminal() {
        eprintln!(
            "dsplr: there is no terminal to ask on, so nothing was saved; pass --yes to replace it"
        );
        return false;
    }
    if ask(&format!("Replace {path}? [y/N] ")) {
        true
    } else {
        eprintln!("dsplr: nothing was saved");
        false
    }
}

/// Asks `question` on the terminal and waits for a line. Anything but yes,
/// and a terminal that closes, is no.
fn ask(question: &str) -> bool {
    eprint!("{question}");
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(_) => is_yes(&line),
        Err(error) => {
            eprintln!("dsplr: cannot read the answer: {error}");
            false
        }
    }
}

/// Says where the layout went, and what is left for the user to do.
fn saved(persisted: &Persisted) -> Exit {
    let Persisted::File(saved) = persisted;
    eprintln!("dsplr: saved to {}", saved.path.display());
    for id in &saved.kept {
        eprintln!("dsplr: kept the settings for {id}, which is not connected now");
    }
    eprint!("{}", include_note(saved));
    Exit::Success
}

/// What to tell the user about the line their main config needs. The line
/// goes after the config's own monitor rules: Hyprland merges a rule into
/// the one of the same name and keeps it last, so a rule after it wins over
/// the saved file's.
fn include_note(saved: &Saved) -> String {
    let (main, line) = (saved.main_config.display(), saved.include_line);
    if !saved.included {
        format!(
            "dsplr: {main} does not seem to load it: add this line to it, after any hl.monitor calls there (dsplr never edits it):\n  {line}\n"
        )
    } else if saved.overridden {
        format!(
            "dsplr: {main} has hl.monitor calls after {line}, and they override the saved file; move {line} below them\n"
        )
    } else if saved.created {
        format!("dsplr: {main} already has {line}\n")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dsplr_persist::managed::Saved;

    use super::include_note;

    fn saved(created: bool, included: bool) -> Saved {
        Saved {
            overridden: false,
            path: PathBuf::from("/h/.config/hypr/monitors.lua"),
            created,
            main_config: PathBuf::from("/h/.config/hypr/hyprland.lua"),
            include_line: r#"require("monitors")"#,
            included,
            kept: Vec::new(),
        }
    }

    #[test]
    fn the_include_line_is_named_until_the_config_has_it() {
        for created in [true, false] {
            let note = include_note(&saved(created, false));
            assert!(
                note.contains("/h/.config/hypr/hyprland.lua")
                    && note.contains("after any hl.monitor calls")
                    && note.contains("\n  require(\"monitors\")\n"),
                "{note}"
            );
        }
        for created in [true, false] {
            let note = include_note(&Saved {
                overridden: true,
                ..saved(created, true)
            });
            assert!(note.contains("override the saved file"), "{note}");
        }
        assert_eq!(
            include_note(&saved(true, true)),
            "dsplr: /h/.config/hypr/hyprland.lua already has require(\"monitors\")\n"
        );
        assert_eq!(include_note(&saved(false, true)), "");
    }
}
