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
    let layout = match crate::snapshot(backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
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
        // dsplr declined before writing anything, as validation does.
        Exit::Refused
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

/// What to tell the user about the line their main config needs, and about
/// the config's own rules that win over the saved file. The line goes after
/// those rules: Hyprland uses the last rule for a monitor, and sway the last
/// setting on a reload, so a rule after it wins ([`Saved::overridden`]).
/// sway also lets a rule naming an output by its identifier win at startup
/// over one naming it by connector, wherever each is, so moving the line
/// does not help with those ([`Saved::beaten_at_startup`]).
fn include_note(saved: &Saved) -> String {
    let (main, line, rules) = (saved.main_config.display(), saved.include_line, saved.rules);
    let include = if !saved.included {
        format!(
            "dsplr: {main} does not seem to load it: add this line to it, after any {rules} there (dsplr never edits it):\n  {line}\n"
        )
    } else if saved.overridden {
        format!(
            "dsplr: {main} has {rules} after {line}, and they override the saved file; move {line} below them\n"
        )
    } else if saved.created {
        format!("dsplr: {main} already has {line}\n")
    } else {
        String::new()
    };
    let beaten = if saved.beaten_at_startup.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = saved
            .beaten_at_startup
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect();
        format!(
            "dsplr: {main} sets {} by sway's identifier, and when sway starts that wins over the saved file, which names the output by its connector, wherever {line} goes; remove those output commands, or name the output by its connector there\n",
            names.join(", ")
        )
    };
    include + &beaten
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dsplr_persist::managed::Saved;

    use super::include_note;

    fn saved(created: bool, included: bool) -> Saved {
        Saved {
            overridden: false,
            beaten_at_startup: Vec::new(),
            path: PathBuf::from("/h/.config/hypr/monitors.lua"),
            created,
            main_config: PathBuf::from("/h/.config/hypr/hyprland.lua"),
            include_line: r#"require("monitors")"#,
            rules: "hl.monitor calls",
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

    #[test]
    fn the_note_names_the_desktop_s_own_rules() {
        let sway = Saved {
            main_config: PathBuf::from("/h/.config/sway/config"),
            include_line: "include outputs",
            rules: "output commands",
            ..saved(true, false)
        };
        assert!(
            include_note(&sway).contains(
                "add this line to it, after any output commands there (dsplr never edits it):\n  include outputs\n"
            ),
            "{}",
            include_note(&sway)
        );
        let note = include_note(&Saved {
            included: true,
            overridden: true,
            ..sway
        });
        assert!(
            note.contains("has output commands after include outputs"),
            "{note}"
        );
        assert!(!note.contains("identifier"), "{note}");
    }

    /// An identifier rule before the include is not fixed by moving the
    /// include, so the note says what does, whether or not the config loads
    /// the file yet.
    #[test]
    fn identifier_rules_that_win_at_startup_get_their_own_fix() {
        let beaten = vec!["BOE 0x0AFC Unknown".to_owned()];
        for included in [true, false] {
            let note = include_note(&Saved {
                main_config: PathBuf::from("/h/.config/sway/config"),
                include_line: "include outputs",
                rules: "output commands",
                beaten_at_startup: beaten.clone(),
                ..saved(false, included)
            });
            assert!(
                note.contains(
                    "/h/.config/sway/config sets \"BOE 0x0AFC Unknown\" by sway's identifier"
                ) && note.contains(
                    "remove those output commands, or name the output by its connector there"
                ),
                "{note}"
            );
            assert!(!note.contains("move include outputs below"), "{note}");
            assert_eq!(note.contains("add this line"), !included, "{note}");
        }
    }
}
