//! `dsplr identify`: the GUI's identify overlay, each monitor's output named
//! on it, without the main window.

use dsplr_gui::{GuiError, IDENTIFY_UNSUPPORTED, Identified};

use crate::{Exit, find_backend, snapshot};

pub async fn run() -> Exit {
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    let layout = match snapshot(&backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    let shown = dsplr_gui::identify(&layout, |line| eprintln!("dsplr: {line}"));
    let (exit, line) = outcome(shown);
    if let Some(line) = line {
        eprintln!("dsplr: {line}");
    }
    exit
}

/// The exit code for how the overlay ended, and the line to say about it.
/// No GUI session to show it in (no display, or a window or GPU that cannot
/// be opened) is a failure; a window system that has no panels for it is
/// refused, as a save the backend cannot do there is.
fn outcome(shown: Result<Identified, GuiError>) -> (Exit, Option<String>) {
    match shown {
        Ok(Identified::Shown) => (Exit::Success, None),
        Ok(Identified::Unsupported) => (Exit::Refused, Some(IDENTIFY_UNSUPPORTED.to_owned())),
        // Each monitor's reason was said as it was skipped.
        Ok(Identified::NotShown) => (
            Exit::Failure,
            Some("the identify overlay opened on no monitor".to_owned()),
        ),
        Err(error) => (
            Exit::Failure,
            Some(format!(
                "cannot show the identify overlay (it needs a GUI session): {error}"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_is_success_and_says_nothing_more() {
        assert_eq!(outcome(Ok(Identified::Shown)), (Exit::Success, None));
    }

    #[test]
    fn no_panels_is_refused_with_why() {
        let (exit, line) = outcome(Ok(Identified::Unsupported));
        assert_eq!(exit, Exit::Refused);
        assert_eq!(line.as_deref(), Some(IDENTIFY_UNSUPPORTED));
    }

    #[test]
    fn opened_nowhere_or_no_session_is_a_failure() {
        let (exit, line) = outcome(Ok(Identified::NotShown));
        assert_eq!(exit, Exit::Failure);
        assert!(line.is_some());
        let (exit, line) = outcome(Err(GuiError::NeverSized));
        assert_eq!(exit, Exit::Failure);
        let line = line.unwrap();
        assert!(line.contains("needs a GUI session"), "{line}");
        assert!(line.contains(&GuiError::NeverSized.to_string()), "{line}");
    }
}
