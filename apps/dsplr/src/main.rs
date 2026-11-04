//! The `dsplr` binary: a display settings editor for Linux.
//!
//! The commands so far read the desktop: `dsplr list` and `dsplr backend`.
//! The GUI and every command that changes something are not written yet.

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use dsplr_backend::{Detection, detect};

mod backend;
mod list;

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
}

/// dsplr's exit codes (`docs/plan/07-cli.md`). Usage errors exit with
/// clap's own code; the codes for refused and reverted changes are added
/// with the commands that make changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exit {
    Success = 0,
    Failure = 1,
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
    }
}

async fn list(json: bool) -> Exit {
    let Detection { checks, backend } = detect().await;
    let Some(backend) = backend else {
        eprintln!("dsplr: no backend for this desktop");
        eprint!("{}", backend::describe_checks(&checks));
        return Exit::NoBackend;
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
    fn exit_codes_are_the_documented_numbers() {
        assert_eq!(ExitCode::from(Exit::Success), ExitCode::from(0));
        assert_eq!(ExitCode::from(Exit::Failure), ExitCode::from(1));
        assert_eq!(ExitCode::from(Exit::NoBackend), ExitCode::from(6));
    }
}
