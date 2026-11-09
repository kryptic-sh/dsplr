//! `dsplr` with no command: the GUI (`docs/plan/06-gui.md`).
//!
//! The process model is `docs/plan/02-architecture.md`'s: the backend lives
//! on a thread of its own with a current-thread tokio runtime, and the main
//! thread owns the window and the editor. So far the backend thread reads
//! one snapshot and ends; following the desktop's changes and carrying out
//! Apply and Save are still to come.

use std::sync::mpsc;
use std::thread;

use dsplr_core::{Editor, Snapshot};

use crate::{Exit, find_backend, runtime, snapshot};

/// Reads the displays on the backend thread, then opens the window on them
/// and runs it until the user quits. A desktop dsplr has no backend for, or
/// displays it cannot read, end it before any window opens.
pub fn run() -> Exit {
    let (send, receive) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("dsplr-backend".into())
        .spawn(move || {
            let read = runtime().and_then(|runtime| runtime.block_on(read()));
            // The main thread is waiting for this, so the send cannot fail;
            // the thread ends with it.
            send.send(read)
        });
    if let Err(error) = spawned {
        eprintln!("dsplr: cannot start the backend thread: {error}");
        return Exit::Failure;
    }
    let snapshot = match receive.recv() {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(exit)) => return exit,
        Err(_) => {
            eprintln!("dsplr: the backend thread stopped before reading the displays");
            return Exit::Failure;
        }
    };
    match dsplr_gui::run(Editor::new(snapshot)) {
        Ok(()) => Exit::Success,
        Err(error) => {
            eprintln!("dsplr: {error}");
            Exit::Failure
        }
    }
}

/// The detected backend's name, capabilities and layout, or, when there is
/// no backend or it cannot read the displays, the error reported and the
/// exit code.
async fn read() -> Result<Snapshot, Exit> {
    let backend = find_backend().await?;
    let layout = snapshot(&backend).await?;
    Ok(Snapshot {
        backend: backend.name().into(),
        capabilities: backend.capabilities(),
        layout,
    })
}
