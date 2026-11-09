//! The `dsplr` binary: a display settings editor for Linux.
//!
//! `dsplr list` and `dsplr backend` read the desktop; `dsplr apply` changes
//! it from a layout file and `dsplr set` one output at a time,
//! `dsplr restore --revert` goes back, `dsplr save` makes the current state
//! last, and `dsplr save --profile`, `dsplr restore` and `dsplr profile` keep
//! named layouts. `dsplr` alone opens the GUI.

use std::fs;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use dsplr_backend::{Backend, Detection, detect};
use dsplr_config::Config;
use dsplr_model::Layout;
use dsplr_persist::profile::ProfileName;

mod apply;
mod backend;
mod gui;
mod list;
mod profile;
mod restore;
mod save;
mod set;

/// Arrange, size and configure monitors on Hyprland, wlroots compositors,
/// KDE Plasma, GNOME and X11.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Outputs, their modes and their current state.
    List {
        /// Print the layout as JSON, the form profiles use.
        #[arg(long)]
        json: bool,
    },
    /// Which backend was detected, what was checked, and its capabilities.
    Backend,
    /// Apply a whole layout from a JSON file, as `dsplr list --json` writes
    /// it, and ask whether to keep it.
    Apply {
        /// The layout file.
        file: PathBuf,
        #[command(flatten)]
        apply: ApplyArgs,
    },
    /// Change one output's settings, leaving the others as they are, and ask
    /// whether to keep the change, as `dsplr apply` does.
    Set {
        /// The output: its connector (`DP-1`), or its description as
        /// `dsplr list` prints it (make, model and serial). The start of a
        /// description is enough when it names one output.
        output: String,
        #[command(flatten)]
        changes: set::Changes,
        #[command(flatten)]
        apply: ApplyArgs,
    },
    /// Save the displays as they are now where the desktop reads them when
    /// it starts: on Hyprland, the monitors.lua beside its hyprland.lua; on
    /// sway, the outputs file beside its config; on niri, the outputs.kdl
    /// beside its config.kdl, which niri also reloads at once. With
    /// --profile, as a named profile in dsplr's own store instead, on any
    /// desktop.
    Save {
        /// Overwrite the file even when it was edited by hand, or holds
        /// settings dsplr cannot keep (they are lost); replace a profile, or
        /// one that cannot be read, without asking.
        #[arg(long)]
        force: bool,
        /// Replace a file another tool wrote (nwg-displays'), or a profile of
        /// the same name, without asking.
        #[arg(long)]
        yes: bool,
        /// Save as the profile NAME in $XDG_CONFIG_HOME/dsplr/profiles:
        /// ASCII letters and digits, spaces, '-', '_' and '.', starting with a
        /// letter, a digit or '_', and not ending in a space or '.'.
        #[arg(long, value_name = "NAME")]
        profile: Option<ProfileName>,
    },
    /// Apply a saved profile: the one named, or else the one saved for
    /// exactly the outputs connected now, else the most recently saved one
    /// for some of them; with none, nothing changes (exit 7). With --revert,
    /// go back to the layout from before the last change instead.
    ///
    /// Run from a desktop's autostart, where there is no terminal to
    /// confirm on, it needs --yes to change the displays.
    Restore {
        /// The profile to apply.
        #[arg(value_name = "NAME", conflicts_with = "revert")]
        name: Option<ProfileName>,
        /// Apply the last revert point, without asking.
        #[arg(long, conflicts_with_all = ["yes", "dry_run", "confirm_timeout"])]
        revert: bool,
        /// Run as a change's watchdog: wait this many seconds first.
        #[arg(
            long,
            hide = true,
            value_name = "SECS",
            requires = "token",
            requires = "revert"
        )]
        after: Option<u64>,
        /// Run as a change's watchdog: revert only while the revert point
        /// holds this token.
        #[arg(long, hide = true, requires = "after")]
        token: Option<String>,
        #[command(flatten)]
        apply: ApplyArgs,
    },
    /// List, show and remove the profiles `dsplr save --profile` saves.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// Each profile's name, the outputs it is for, and when it was saved.
    List {
        /// Print them as JSON.
        #[arg(long)]
        json: bool,
    },
    /// A profile's layout, as `dsplr list` prints one.
    Show {
        name: ProfileName,
        /// Print it as JSON, the form `dsplr apply` reads.
        #[arg(long)]
        json: bool,
    },
    /// Remove a profile.
    Rm { name: ProfileName },
}

/// The flags of every command that changes the displays through the confirm
/// flow (`apply::run`).
#[derive(Debug, Args)]
struct ApplyArgs {
    /// Keep the change without asking. The revert point is still written,
    /// for `dsplr restore --revert`.
    #[arg(long)]
    yes: bool,
    /// Check the layout without applying it: validate it, then have the
    /// desktop test it where it has a real test.
    #[arg(long)]
    dry_run: bool,
    /// Seconds to wait for an answer before reverting, in place of the
    /// configured `confirm.timeout_secs`.
    #[arg(long, hide = true, value_name = "SECS")]
    confirm_timeout: Option<NonZeroU32>,
}

/// dsplr's exit codes (`docs/plan/07-cli.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exit {
    Success = 0,
    Failure = 1,
    /// A usage error: clap exits with this code for the ones it finds, and
    /// dsplr for the ones only the displays show, such as an output name
    /// that names no output.
    Usage = 2,
    /// dsplr refused before asking the desktop or writing anything:
    /// validation refused the layout, the backend cannot put it into the
    /// desktop's request, or the backend cannot save here at all.
    Refused = 3,
    /// The desktop refused the layout.
    BackendRefused = 4,
    /// The change was reverted: declined, or not confirmed in time.
    Reverted = 5,
    NoBackend = 6,
    /// `dsplr restore` without a name found no profile for the outputs
    /// connected now, and changed nothing.
    NoProfile = 7,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit as u8)
    }
}

fn main() -> ExitCode {
    let Some(command) = Cli::parse().command else {
        return gui::run().into();
    };
    match runtime() {
        Ok(runtime) => runtime.block_on(run(command)).into(),
        Err(exit) => exit.into(),
    }
}

/// The current-thread runtime every command and the GUI's backend thread
/// run on, or, when it cannot start, the error reported and the exit code.
fn runtime() -> Result<tokio::runtime::Runtime, Exit> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            eprintln!("dsplr: cannot start the async runtime: {error}");
            Exit::Failure
        })
}

async fn run(command: Command) -> Exit {
    match command {
        Command::Backend => {
            let detection = detect().await;
            let code = if detection.backend.is_some() {
                Exit::Success
            } else {
                Exit::NoBackend
            };
            print(&backend::describe(&detection).await).unwrap_or(code)
        }
        Command::List { json } => list(json).await,
        Command::Apply { file, apply } => apply_file(&file, &apply).await,
        Command::Set {
            output,
            changes,
            apply,
        } => set(&output, &changes, &apply).await,
        Command::Save {
            force,
            yes,
            profile,
        } => match find_backend().await {
            Ok(backend) => {
                let options = save::Options { force, yes };
                match profile {
                    Some(name) => profile::save(&backend, &name, &options).await,
                    None => save::run(&backend, &options).await,
                }
            }
            Err(exit) => exit,
        },
        Command::Restore {
            revert: true,
            after,
            token,
            ..
        } => restore::run(after.map(Duration::from_secs), token.as_deref()).await,
        Command::Restore { name, apply, .. } => match apply_options(&apply) {
            Ok(options) => profile::restore(name.as_ref(), &options).await,
            Err(exit) => exit,
        },
        Command::Profile { command } => match command {
            ProfileCommand::List { json } => profile::list(json),
            ProfileCommand::Show { name, json } => profile::show(&name, json),
            ProfileCommand::Rm { name } => profile::remove(&name),
        },
    }
}

/// The backend for this desktop, or, when there is none, the checks that
/// found none, reported, and the exit code.
async fn find_backend() -> Result<Backend, Exit> {
    let Detection { checks, backend } = detect().await;
    backend.ok_or_else(|| {
        eprintln!("dsplr: no backend for this desktop");
        eprint!("{}", backend::describe_checks(&checks));
        Exit::NoBackend
    })
}

async fn apply_file(file: &Path, args: &ApplyArgs) -> Exit {
    let layout = match fs::read_to_string(file)
        .map_err(|error| error.to_string())
        .and_then(|text| Layout::from_json(&text).map_err(|error| error.to_string()))
    {
        Ok(layout) => layout,
        Err(error) => {
            eprintln!("dsplr: {}: {error}", file.display());
            return Exit::Failure;
        }
    };
    let options = match apply_options(args) {
        Ok(options) => options,
        Err(exit) => return exit,
    };
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    apply::file(&backend, &layout, &options).await
}

async fn set(output: &str, changes: &set::Changes, args: &ApplyArgs) -> Exit {
    let options = match apply_options(args) {
        Ok(options) => options,
        Err(exit) => return exit,
    };
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    set::run(&backend, output, changes, &options).await
}

/// The confirm flow's options from its flags, the timeout from the config
/// unless `--confirm-timeout` gives one.
fn apply_options(args: &ApplyArgs) -> Result<apply::Options, Exit> {
    let confirm_timeout = match args.confirm_timeout {
        Some(secs) => Duration::from_secs(secs.get().into()),
        None => match Config::load() {
            Ok((config, _)) => config.confirm.timeout(),
            Err(error) => {
                eprintln!("dsplr: {error}");
                return Err(Exit::Failure);
            }
        },
    };
    Ok(apply::Options {
        yes: args.yes,
        dry_run: args.dry_run,
        confirm_timeout,
    })
}

/// The displays' current state, or, when they cannot be read, the error
/// reported and the exit code.
async fn snapshot(backend: &Backend) -> Result<Layout, Exit> {
    backend.snapshot().await.map_err(|error| {
        eprintln!(
            "dsplr: cannot read the displays from {}: {error}",
            backend.name()
        );
        Exit::Failure
    })
}

async fn list(json: bool) -> Exit {
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    let layout = match snapshot(&backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    let text = if json {
        match layout.to_json() {
            Ok(mut text) => {
                text.push('\n');
                text
            }
            Err(error) => {
                eprintln!("dsplr: cannot write the layout as JSON: {error}");
                return Exit::Failure;
            }
        }
    } else {
        list::describe(&layout)
    };
    print(&text).unwrap_or(Exit::Success)
}

/// Writes `text` to stdout. A failed write (a closed pipe) is reported and
/// returned as the exit code, rather than panicking as `print!` does.
fn print(text: &str) -> Option<Exit> {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => None,
        Err(error) => {
            eprintln!("dsplr: cannot write to stdout: {error}");
            Some(Exit::Failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use clap::{CommandFactory, Parser};

    use super::{ApplyArgs, Cli, Command, Exit, ProfileCommand};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn list_takes_a_json_flag() {
        let cli = Cli::try_parse_from(["dsplr", "list", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Command::List { json: true })));
        let cli = Cli::try_parse_from(["dsplr", "list"]).unwrap();
        assert!(matches!(cli.command, Some(Command::List { json: false })));
    }

    #[test]
    fn the_watchdog_needs_revert_and_both_its_arguments() {
        assert!(Cli::try_parse_from(["dsplr", "restore", "--revert", "--after", "25"]).is_err());
        assert!(Cli::try_parse_from(["dsplr", "restore", "--revert", "--token", "t"]).is_err());
        assert!(
            Cli::try_parse_from(["dsplr", "restore", "--after", "25", "--token", "t"]).is_err()
        );
        let cli = Cli::try_parse_from([
            "dsplr", "restore", "--revert", "--after", "25", "--token", "t",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Restore { after: Some(25), token: Some(ref t), .. }) if t == "t"
        ));
    }

    #[test]
    fn apply_takes_a_file_and_its_flags() {
        let cli = Cli::try_parse_from(["dsplr", "apply", "l.json", "--yes", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Apply {
                apply: ApplyArgs {
                    yes: true,
                    dry_run: true,
                    confirm_timeout: None,
                },
                ..
            })
        ));
        assert!(
            Cli::try_parse_from(["dsplr", "apply", "l.json", "--confirm-timeout", "0"]).is_err(),
            "a zero timeout could never be confirmed"
        );
    }

    #[test]
    fn save_takes_force_yes_and_a_profile() {
        let cli = Cli::try_parse_from(["dsplr", "save"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Save {
                force: false,
                yes: false,
                profile: None,
            })
        ));
        let cli = Cli::try_parse_from(["dsplr", "save", "--force", "--yes"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Save {
                force: true,
                yes: true,
                profile: None,
            })
        ));
        let cli = Cli::try_parse_from(["dsplr", "save", "--profile", "laptop only"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Save { profile: Some(ref name), .. }) if name.as_str() == "laptop only"
        ));
    }

    /// The name becomes a file name, so one that could reach outside the
    /// store is a usage error before anything runs.
    #[test]
    fn a_profile_name_that_is_not_a_plain_file_name_is_a_usage_error() {
        for name in ["", "../x", "a/b", ".hidden", "a..b", "-x", "desk.", "Büro"] {
            for args in [
                &["dsplr", "save", "--profile", name][..],
                &["dsplr", "restore", name],
                &["dsplr", "profile", "show", name],
                &["dsplr", "profile", "rm", name],
            ] {
                let error = Cli::try_parse_from(args).unwrap_err();
                assert_eq!(error.exit_code(), Exit::Usage as i32, "{args:?}: {error}");
            }
        }
    }

    #[test]
    fn restore_takes_a_name_or_revert_and_the_apply_flags() {
        let cli = Cli::try_parse_from(["dsplr", "restore"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Restore {
                name: None,
                revert: false,
                ..
            })
        ));
        let cli = Cli::try_parse_from(["dsplr", "restore", "desk", "--yes", "--dry-run"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Restore {
                name: Some(ref name),
                revert: false,
                apply: ApplyArgs { yes: true, dry_run: true, .. },
                ..
            }) if name.as_str() == "desk"
        ));
        // A revert goes back without asking, so the confirm flow's flags
        // mean nothing to it.
        for args in [
            &["dsplr", "restore", "desk", "--revert"][..],
            &["dsplr", "restore", "--revert", "--yes"],
            &["dsplr", "restore", "--revert", "--dry-run"],
            &["dsplr", "restore", "--revert", "--confirm-timeout", "5"],
        ] {
            let error = Cli::try_parse_from(args).unwrap_err();
            assert_eq!(error.exit_code(), Exit::Usage as i32, "{args:?}: {error}");
        }
    }

    #[test]
    fn profile_lists_shows_and_removes() {
        let cli = Cli::try_parse_from(["dsplr", "profile", "list", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Profile {
                command: ProfileCommand::List { json: true }
            })
        ));
        let cli = Cli::try_parse_from(["dsplr", "profile", "show", "desk", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Profile {
                command: ProfileCommand::Show { ref name, json: true }
            }) if name.as_str() == "desk"
        ));
        let cli = Cli::try_parse_from(["dsplr", "profile", "rm", "desk"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Profile {
                command: ProfileCommand::Rm { ref name }
            }) if name.as_str() == "desk"
        ));
        assert!(Cli::try_parse_from(["dsplr", "profile"]).is_err());
        assert!(Cli::try_parse_from(["dsplr", "profile", "show"]).is_err());
    }

    #[test]
    fn dsplr_s_usage_code_is_clap_s() {
        let error = Cli::try_parse_from(["dsplr", "set", "DP-1", "--scale", "0"]).unwrap_err();
        assert_eq!(error.exit_code(), Exit::Usage as i32);
    }

    #[test]
    fn set_takes_an_output_its_changes_and_the_apply_flags() {
        let cli = Cli::try_parse_from([
            "dsplr",
            "set",
            "DP-1",
            "--pos",
            "-1920,0",
            "--disable",
            "--yes",
        ])
        .unwrap();
        let Some(Command::Set {
            output,
            changes,
            apply,
        }) = cli.command
        else {
            panic!("not set: {cli:?}");
        };
        assert_eq!(output, "DP-1");
        assert_eq!(changes.pos, Some((-1920, 0)));
        assert!(changes.disable && !changes.enable);
        assert!(apply.yes && !apply.dry_run);
    }

    #[test]
    fn set_needs_a_change_and_takes_one_of_enable_and_disable() {
        for args in [
            &["dsplr", "set", "DP-1"][..],
            &["dsplr", "set", "DP-1", "--yes", "--dry-run"],
            &["dsplr", "set", "DP-1", "--enable", "--disable"],
            &["dsplr", "set", "DP-1", "--mirror", "DP-2", "--no-mirror"],
            &["dsplr", "set", "--pos", "0,0"],
        ] {
            let error = Cli::try_parse_from(args).unwrap_err();
            assert_eq!(error.exit_code(), Exit::Usage as i32, "{args:?}: {error}");
        }
        for change in [
            &["--mode", "1920x1080"][..],
            &["--pos", "0,0"],
            &["--scale", "1.5"],
            &["--transform", "flipped-90"],
            &["--vrr", "on"],
            &["--enable"],
            &["--disable"],
            &["--mirror", "DP-2"],
            &["--no-mirror"],
        ] {
            let args = [&["dsplr", "set", "DP-1"][..], change].concat();
            assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn set_refuses_values_its_parsers_refuse() {
        for (flag, value) in [
            ("--mode", "1920x1080@"),
            ("--pos", "0"),
            ("--scale", "-1"),
            ("--transform", "rotate-90"),
            ("--vrr", "yes"),
        ] {
            let error = Cli::try_parse_from(["dsplr", "set", "DP-1", flag, value]).unwrap_err();
            assert_eq!(error.exit_code(), Exit::Usage as i32, "{flag} {value}");
        }
    }

    #[test]
    fn exit_codes_are_the_documented_numbers() {
        assert_eq!(ExitCode::from(Exit::Success), ExitCode::from(0));
        assert_eq!(ExitCode::from(Exit::Failure), ExitCode::from(1));
        assert_eq!(ExitCode::from(Exit::Usage), ExitCode::from(2));
        assert_eq!(ExitCode::from(Exit::Refused), ExitCode::from(3));
        assert_eq!(ExitCode::from(Exit::BackendRefused), ExitCode::from(4));
        assert_eq!(ExitCode::from(Exit::Reverted), ExitCode::from(5));
        assert_eq!(ExitCode::from(Exit::NoBackend), ExitCode::from(6));
        assert_eq!(ExitCode::from(Exit::NoProfile), ExitCode::from(7));
    }
}
