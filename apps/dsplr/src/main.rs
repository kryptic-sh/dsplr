//! The `dsplr` binary: a display settings editor for Linux.
//!
//! `dsplr list` and `dsplr backend` read the desktop; `dsplr apply` changes
//! it from a layout file and `dsplr restore --revert` goes back. The GUI,
//! `dsplr set`, saving and profiles are not written yet.

use std::fs;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use dsplr_backend::{Backend, Detection, detect};
use dsplr_config::Config;
use dsplr_model::Layout;

mod apply;
mod backend;
mod list;
mod restore;

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
        /// Keep the change without asking. The revert point is still
        /// written, for `dsplr restore --revert`.
        #[arg(long)]
        yes: bool,
        /// Check the layout without applying it.
        #[arg(long)]
        dry_run: bool,
        /// Seconds to wait for an answer before reverting, in place of the
        /// configured `confirm.timeout_secs`.
        #[arg(long, hide = true, value_name = "SECS")]
        confirm_timeout: Option<NonZeroU32>,
    },
    /// Go back to the layout from before the last change.
    Restore {
        /// Apply the last revert point. Profiles are not written yet, so
        /// this is the only thing restore does.
        #[arg(long, required = true)]
        revert: bool,
        /// Run as a change's watchdog: wait this many seconds first.
        #[arg(long, hide = true, value_name = "SECS", requires = "token")]
        after: Option<u64>,
        /// Run as a change's watchdog: revert only while the revert point
        /// holds this token.
        #[arg(long, hide = true, requires = "after")]
        token: Option<String>,
    },
}

/// dsplr's exit codes (`docs/plan/07-cli.md`). Usage errors exit with
/// clap's own code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exit {
    Success = 0,
    Failure = 1,
    /// Validation refused the layout.
    Refused = 3,
    /// The desktop refused the layout.
    BackendRefused = 4,
    /// The change was reverted: declined, or not confirmed in time.
    Reverted = 5,
    NoBackend = 6,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit as u8)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("dsplr: cannot start the async runtime: {error}");
            return Exit::Failure.into();
        }
    };
    runtime.block_on(run(cli)).into()
}

async fn run(cli: Cli) -> Exit {
    match cli.command {
        None => {
            eprintln!(
                "dsplr: the GUI is not written yet; `dsplr list` and `dsplr backend` read the displays"
            );
            Exit::Failure
        }
        Some(Command::Backend) => {
            let detection = detect().await;
            let code = if detection.backend.is_some() {
                Exit::Success
            } else {
                Exit::NoBackend
            };
            print(&backend::describe(&detection)).unwrap_or(code)
        }
        Some(Command::List { json }) => list(json).await,
        Some(Command::Apply {
            file,
            yes,
            dry_run,
            confirm_timeout,
        }) => apply(&file, yes, dry_run, confirm_timeout).await,
        Some(Command::Restore {
            revert: _,
            after,
            token,
        }) => restore::run(after.map(Duration::from_secs), token.as_deref()).await,
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

async fn apply(file: &Path, yes: bool, dry_run: bool, confirm_timeout: Option<NonZeroU32>) -> Exit {
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
    let confirm_timeout = match confirm_timeout {
        Some(secs) => Duration::from_secs(secs.get().into()),
        None => match Config::load() {
            Ok((config, _)) => config.confirm.timeout(),
            Err(error) => {
                eprintln!("dsplr: {error}");
                return Exit::Failure;
            }
        },
    };
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    let options = apply::Options {
        yes,
        dry_run,
        confirm_timeout,
    };
    apply::run(&backend, &layout, &options).await
}

async fn list(json: bool) -> Exit {
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
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

    use super::{Cli, Command, Exit};

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
    fn restore_needs_revert_and_the_watchdog_needs_both_its_arguments() {
        assert!(Cli::try_parse_from(["dsplr", "restore"]).is_err());
        assert!(Cli::try_parse_from(["dsplr", "restore", "--revert", "--after", "25"]).is_err());
        assert!(Cli::try_parse_from(["dsplr", "restore", "--revert", "--token", "t"]).is_err());
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
                yes: true,
                dry_run: true,
                confirm_timeout: None,
                ..
            })
        ));
        assert!(
            Cli::try_parse_from(["dsplr", "apply", "l.json", "--confirm-timeout", "0"]).is_err(),
            "a zero timeout could never be confirmed"
        );
    }

    #[test]
    fn exit_codes_are_the_documented_numbers() {
        assert_eq!(ExitCode::from(Exit::Success), ExitCode::from(0));
        assert_eq!(ExitCode::from(Exit::Failure), ExitCode::from(1));
        assert_eq!(ExitCode::from(Exit::Refused), ExitCode::from(3));
        assert_eq!(ExitCode::from(Exit::BackendRefused), ExitCode::from(4));
        assert_eq!(ExitCode::from(Exit::Reverted), ExitCode::from(5));
        assert_eq!(ExitCode::from(Exit::NoBackend), ExitCode::from(6));
    }
}
