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
    let conflict = match backend.persist(&layout, None).await {
        Ok(persisted) => return saved(&persisted),
        Err(Error::Conflict(conflict)) => conflict,
        Err(error) => return failed(backend, &error),
    };
    if !allowed(&conflict, options) {
        return Exit::Failure;
    }
    match backend.persist(&layout, Some(&conflict.existing)).await {
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

/// Why saving over the file of `conflict` is asked first, one message
/// each: a file dsplr cannot keep, or edited by hand, or someone else's, and
/// the settings of the main config it would stop applying.
pub fn conflict_notes(conflict: &Conflict) -> Vec<String> {
    let path = conflict.path.display();
    let mut notes = Vec::new();
    match (&conflict.unreadable, conflict.ownership) {
        (Some(reason), _) => notes.push(format!(
            "{path} holds what dsplr cannot keep, so saving would lose it: {reason}"
        )),
        (None, Ownership::EditedByHand) => {
            notes.push(format!("{path} was edited by hand since dsplr wrote it"));
        }
        (None, Ownership::Foreign) => notes.push(format!(
            "{path} was not written by dsplr (nwg-displays writes one too); dsplr replaces it with its own"
        )),
        (None, Ownership::Managed | Ownership::Absent) => {}
    }
    if !conflict.shadowed.is_empty() {
        let mut note = format!(
            "once {path} is written, these settings of your config stop applying, since its node for the same output comes first and wins whole:"
        );
        for each in &conflict.shadowed {
            note.push_str("\n  ");
            note.push_str(&each.to_string());
        }
        notes.push(note);
    }
    notes
}

/// Shows what saving would change in the file of `conflict`, and whether
/// the user allows it: `--force`, or for a readable file someone else
/// wrote, or one whose writing would stop settings of the main config
/// applying, `--yes` or a yes on the terminal.
fn allowed(conflict: &Conflict, options: &Options) -> bool {
    for note in conflict_notes(conflict) {
        eprintln!("dsplr: {note}");
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
    replace_allowed(&conflict.path.display().to_string(), options.yes)
}

/// Whether the user allows replacing `what`: `yes` (`--yes`), or a yes on
/// the terminal. Without either, says why nothing was saved.
pub fn replace_allowed(what: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    if !io::stdin().is_terminal() {
        eprintln!(
            "dsplr: there is no terminal to ask on, so nothing was saved; pass --yes to replace it"
        );
        return false;
    }
    if ask(&format!("Replace {what}? [y/N] ")) {
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
    for message in saved_messages(persisted) {
        eprintln!("dsplr: {message}");
    }
    Exit::Success
}

/// Where the layout went, and what is left for the user to do, one message
/// each.
pub fn saved_messages(persisted: &Persisted) -> Vec<String> {
    let saved = match persisted {
        Persisted::File(saved) => saved,
        Persisted::ByDesktop { by } => return vec![format!("already saved, by {by}")],
    };
    let mut messages = vec![format!("saved to {}", saved.path.display())];
    messages.extend(
        saved
            .kept
            .iter()
            .map(|id| format!("kept the settings for {id}, which is not connected now")),
    );
    messages.extend(include_note(saved));
    messages
}

/// What to tell the user about the line their main config needs, and about
/// the config's own rules that win over the saved file. The line goes where
/// the file's rules win: after the config's own on Hyprland, which uses the
/// last rule for a monitor, and on sway, which uses the last setting on a
/// reload; before them on niri, which uses the first output node naming an
/// output ([`Saved::first_wins`]). A rule on the other side wins
/// ([`Saved::overridden`]). sway also lets a rule naming an output by its
/// identifier win at startup over one naming it by connector, wherever each
/// is, so moving the line does not help with those
/// ([`Saved::beaten_at_startup`]).
fn include_note(saved: &Saved) -> Vec<String> {
    let (main, line, rules) = (saved.main_config.display(), saved.include_line, saved.rules);
    let (side, past) = if saved.first_wins {
        ("before", "above")
    } else {
        ("after", "below")
    };
    let include = if !saved.included {
        format!(
            "{main} does not seem to load it: add this line to it, {side} any {rules} there (dsplr never edits it):\n  {line}"
        )
    } else if saved.overridden {
        let (where_, wins) = if saved.first_wins {
            ("before", "win over")
        } else {
            ("after", "override")
        };
        format!(
            "{main} has {rules} {where_} {line}, and they {wins} the saved file; move {line} {past} them"
        )
    } else if saved.created {
        format!("{main} already has {line}")
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
            "{main} sets {} by sway's identifier, and when sway starts that wins over the saved file, which names the output by its connector, wherever {line} goes; remove those output commands, or name the output by its connector there",
            names.join(", ")
        )
    };
    [include, beaten]
        .into_iter()
        .filter(|message| !message.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dsplr_persist::managed::Saved;

    /// The note as the CLI prints it.
    fn include_note(saved: &Saved) -> String {
        super::include_note(saved)
            .iter()
            .map(|message| format!("dsplr: {message}\n"))
            .collect()
    }

    fn saved(created: bool, included: bool) -> Saved {
        Saved {
            overridden: false,
            beaten_at_startup: Vec::new(),
            path: PathBuf::from("/h/.config/hypr/monitors.lua"),
            created,
            main_config: PathBuf::from("/h/.config/hypr/hyprland.lua"),
            include_line: r#"require("monitors")"#,
            rules: "hl.monitor calls",
            first_wins: false,
            included,
            kept: Vec::new(),
        }
    }

    #[test]
    fn a_layout_the_desktop_saved_says_so_and_names_no_file() {
        let messages = super::saved_messages(&dsplr_backend::Persisted::ByDesktop {
            by: "KWin, in its kwinoutputconfig.json, when it applied it",
        });
        assert_eq!(
            messages,
            ["already saved, by KWin, in its kwinoutputconfig.json, when it applied it"]
        );
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

    /// niri uses the first output node naming an output, so the include
    /// goes before the config's own, and one of those before it wins.
    #[test]
    fn where_the_first_rule_wins_the_include_goes_before_the_rules() {
        let niri = Saved {
            main_config: PathBuf::from("/h/.config/niri/config.kdl"),
            include_line: r#"include "outputs.kdl""#,
            rules: "output nodes",
            first_wins: true,
            ..saved(true, false)
        };
        let note = include_note(&niri);
        assert!(
            note.contains(
                "add this line to it, before any output nodes there (dsplr never edits it):\n  include \"outputs.kdl\"\n"
            ),
            "{note}"
        );
        let note = include_note(&Saved {
            included: true,
            overridden: true,
            ..niri
        });
        assert_eq!(
            note,
            "dsplr: /h/.config/niri/config.kdl has output nodes before include \"outputs.kdl\", and they win over the saved file; move include \"outputs.kdl\" above them\n"
        );
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
