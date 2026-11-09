//! Shared by every e2e test (included with `#[path]`): the check that the
//! test runs under its harness, and finding and editing outputs in a layout.
//! Each desktop's own requests are in its `<desktop>_support.rs`.

#![allow(
    dead_code,
    reason = "included into each e2e test binary, and each uses a subset"
)]

use std::path::PathBuf;

use dsplr_model::{Layout, OutputEntry};

/// The harness's runtime directory, which the harness names in `variable`,
/// after checking that this process really runs under `harness` and not
/// against a session's desktop, and that dsplr's state goes to the
/// harness's directory, not the user's.
pub fn harness_runtime(variable: &str, harness: &str) -> PathBuf {
    let runtime = std::env::var(variable).unwrap_or_else(|_| {
        panic!(
            "run through {harness}, which starts a compositor that cannot reach the \
             desktop; refusing to touch whatever compositor this is"
        )
    });
    assert_eq!(
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        Some(runtime.as_str()),
        "XDG_RUNTIME_DIR is not the harness's private runtime directory"
    );
    assert!(
        !runtime.starts_with("/run/user/"),
        "the harness runtime must be private, not a session's {runtime}"
    );
    let state = std::env::var("XDG_STATE_HOME").unwrap_or_default();
    assert!(
        state.starts_with(&runtime),
        "XDG_STATE_HOME ({state:?}) is not inside the harness runtime"
    );
    PathBuf::from(runtime)
}

pub fn entry<'a>(layout: &'a Layout, connector: &str) -> &'a OutputEntry {
    layout
        .entries
        .iter()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the snapshot: {layout:#?}"))
}

pub fn entry_mut<'a>(layout: &'a mut Layout, connector: &str) -> &'a mut OutputEntry {
    layout
        .entries
        .iter_mut()
        .find(|e| e.output.as_ref().is_some_and(|o| o.connector == connector))
        .unwrap_or_else(|| panic!("{connector} is in the layout"))
}

/// `layout` with `edit` made to it: what a user does to a snapshot before
/// applying it.
pub fn edited(layout: &Layout, edit: impl FnOnce(&mut Layout)) -> Layout {
    let mut layout = layout.clone();
    edit(&mut layout);
    layout
}
