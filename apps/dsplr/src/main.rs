//! The `dsplr` binary: a display settings editor for Linux.

use std::process::ExitCode;

use clap::Parser;

/// Arrange, size and configure monitors on Hyprland, wlroots compositors,
/// KDE Plasma, GNOME and X11.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {}

/// Why this build cannot do anything yet.
#[cfg(target_os = "linux")]
const UNAVAILABLE: &str = "nothing is implemented yet; the first commands land in M1";

/// dsplr is Linux-only; every other target says so instead of doing nothing.
#[cfg(not(target_os = "linux"))]
const UNAVAILABLE: &str = "dsplr supports Linux only";

fn main() -> ExitCode {
    Cli::parse();
    eprintln!("dsplr: {UNAVAILABLE}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
