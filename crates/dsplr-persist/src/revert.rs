//! The revert point (`docs/plan/02-architecture.md`, step 3): the layout
//! before a change, written to `$XDG_STATE_HOME/dsplr/revert.json` before the
//! change is applied, so the watchdog can put it back if dsplr dies before
//! the change is kept, and `dsplr restore --revert` can by hand.
//!
//! Each revert point carries a token. The watchdog started for a change is
//! given its token and reverts only while the file still holds it, so a
//! watchdog left over from an earlier change never reverts a later one.
//! Keeping a change gives the file a new token, so the watchdog stands down
//! while `dsplr restore --revert` can still go back by hand; reverting
//! removes it.
//!
//! While its change waits to be kept, a revert point also says when the
//! watchdog reverts it ([`RevertPoint::guarded_until_unix_ms`]), so another
//! change made meanwhile (`dsplr watch`'s) can tell one is in flight and
//! wait, rather than replace the revert point and disarm that watchdog.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use dsplr_model::Layout;

use crate::layout_json;
use crate::managed::write_atomic;

/// A layout to go back to, and the token naming the change it guards.
#[derive(Debug, Clone, PartialEq)]
pub struct RevertPoint {
    pub token: String,
    pub layout: Layout,
    /// While the change waits to be kept or reverted: when its watchdog
    /// reverts it, in milliseconds since the Unix epoch. `None` once kept.
    pub guarded_until_unix_ms: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum RevertError {
    #[error("no home directory to keep the revert point in")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not a dsplr revert point: {reason}")]
    Invalid { path: PathBuf, reason: String },
}

/// The field the token is kept in, beside the layout's own fields.
const TOKEN_KEY: &str = "revert_token";

/// The field [`RevertPoint::guarded_until_unix_ms`] is kept in; absent once
/// the change is kept.
const GUARDED_KEY: &str = "revert_guarded_until_unix_ms";

impl RevertPoint {
    /// A revert point for `layout` whose change waits to be kept, which its
    /// watchdog reverts at `until_unix_ms`, with a new token
    /// ([`RevertPoint::new`]).
    pub fn guarding(layout: Layout, until_unix_ms: u64) -> Self {
        Self {
            guarded_until_unix_ms: Some(until_unix_ms),
            ..Self::new(layout)
        }
    }

    /// Whether its change still waits to be kept at `now_unix_ms`: guarded,
    /// and its watchdog's time not yet come.
    pub fn waiting(&self, now_unix_ms: u64) -> bool {
        self.guarded_until_unix_ms
            .is_some_and(|until| now_unix_ms < until)
    }

    /// A revert point for `layout` with a new token, for a change that is
    /// kept (no longer guarded): the process id and the time tell processes
    /// apart, and a counter tells apart two made in one process within the
    /// clock's resolution.
    pub fn new(layout: Layout) -> Self {
        static MADE: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let count = MADE.fetch_add(1, Ordering::Relaxed);
        Self {
            token: format!("{}-{nanos}-{count}", std::process::id()),
            layout,
            guarded_until_unix_ms: None,
        }
    }
}

/// `$XDG_STATE_HOME/dsplr/revert.json`.
///
/// # Errors
///
/// When there is no home directory to resolve it from.
pub fn default_path() -> Result<PathBuf, RevertError> {
    hjkl_xdg::state_dir("dsplr")
        .map(|dir| dir.join("revert.json"))
        .map_err(|hjkl_xdg::Error::NoHomeDir| RevertError::NoHome)
}

/// Writes `point` to `path` atomically, creating its directory.
///
/// # Errors
///
/// When the directory or the file cannot be written.
pub fn save(path: &Path, point: &RevertPoint) -> Result<(), RevertError> {
    let io_error = |source| RevertError::Io {
        path: path.to_owned(),
        source,
    };
    let invalid = |reason: String| RevertError::Invalid {
        path: path.to_owned(),
        reason,
    };
    let mut extra = vec![(TOKEN_KEY, point.token.clone().into())];
    if let Some(until) = point.guarded_until_unix_ms {
        extra.push((GUARDED_KEY, until.into()));
    }
    let text = layout_json::write(&point.layout, &extra).map_err(invalid)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_error)?;
    }
    write_atomic(path, &text).map_err(io_error)
}

/// The revert point at `path`, or `None` when there is none.
///
/// # Errors
///
/// When the file cannot be read or is not a revert point.
pub fn load(path: &Path) -> Result<Option<RevertPoint>, RevertError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RevertError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let invalid = |reason: String| RevertError::Invalid {
        path: path.to_owned(),
        reason,
    };
    let (layout, [token, guarded]) =
        layout_json::read(&text, [TOKEN_KEY, GUARDED_KEY]).map_err(invalid)?;
    let Some(serde_json::Value::String(token)) = token else {
        return Err(invalid(format!("no `{TOKEN_KEY}`")));
    };
    let guarded_until_unix_ms = guarded
        .map(|until| {
            until
                .as_u64()
                .ok_or_else(|| invalid(format!("`{GUARDED_KEY}` is not a whole number: {until}")))
        })
        .transpose()?;
    Ok(Some(RevertPoint {
        token,
        layout,
        guarded_until_unix_ms,
    }))
}

/// Marks the change `token` guards as kept: if the revert point at `path`
/// still holds `token`, it is saved again under a new one and no longer
/// guarded, so the watchdog holding `token` does not revert, and the layout
/// stays for a revert by hand. Returns whether it held `token`.
///
/// # Errors
///
/// When the file cannot be read, is not a revert point, or cannot be
/// written.
pub fn keep(path: &Path, token: &str) -> Result<bool, RevertError> {
    match load(path)? {
        Some(point) if point.token == token => {
            save(path, &RevertPoint::new(point.layout))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Removes the revert point at `path` if it holds `token`: the change it
/// guarded has been reverted. Returns whether it was removed;
/// a missing file, or one for another change, is left alone.
///
/// # Errors
///
/// When the file cannot be read, is not a revert point, or cannot be
/// removed.
pub fn remove_if(path: &Path, token: &str) -> Result<bool, RevertError> {
    match load(path)? {
        Some(point) if point.token == token => {
            fs::remove_file(path).map_err(|source| RevertError::Io {
                path: path.to_owned(),
                source,
            })?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dsplr_model::{Layout, Mode, OutputEntry, OutputId, OutputState};

    use super::{RevertError, RevertPoint, keep, load, remove_if, save};

    fn layout() -> Layout {
        Layout {
            entries: vec![OutputEntry {
                id: OutputId::Connector("HDMI-A-1".into()),
                output: None,
                state: OutputState {
                    mode: Some(Mode::new(2560, 1440, 120_000)),
                    scale: 1.25,
                    ..OutputState::default()
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_saved_revert_point_loads_back_with_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("dsplr").join("revert.json");
        for point in [
            RevertPoint::new(layout()),
            RevertPoint::guarding(layout(), 1_000),
        ] {
            save(&path, &point).unwrap();
            assert_eq!(load(&path).unwrap(), Some(point));
        }
    }

    #[test]
    fn a_change_waits_until_its_watchdog_s_time_unless_kept() {
        let point = RevertPoint::guarding(layout(), 1_000);
        assert!(point.waiting(0));
        assert!(point.waiting(999));
        assert!(!point.waiting(1_000), "the watchdog's time has come");
        assert!(!RevertPoint::new(layout()).waiting(0), "kept");
    }

    #[test]
    fn no_file_is_no_revert_point() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("revert.json")).unwrap(), None);
    }

    #[test]
    fn two_revert_points_have_different_tokens() {
        assert_ne!(
            RevertPoint::new(layout()).token,
            RevertPoint::new(layout()).token
        );
    }

    #[test]
    fn only_the_change_it_guards_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revert.json");
        let point = RevertPoint::new(layout());
        save(&path, &point).unwrap();
        assert!(!remove_if(&path, "someone-else").unwrap());
        assert!(path.exists());
        assert!(remove_if(&path, &point.token).unwrap());
        assert!(!path.exists());
        assert!(!remove_if(&path, &point.token).unwrap(), "already gone");
    }

    #[test]
    fn keeping_a_change_keeps_its_layout_under_a_new_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revert.json");
        let point = RevertPoint::guarding(layout(), u64::MAX);
        save(&path, &point).unwrap();
        assert!(!keep(&path, "someone-else").unwrap());
        assert_eq!(load(&path).unwrap().as_ref(), Some(&point));
        assert!(keep(&path, &point.token).unwrap());
        let kept = load(&path).unwrap().expect("still there");
        assert_eq!(kept.layout, point.layout);
        assert_ne!(kept.token, point.token);
        assert_eq!(kept.guarded_until_unix_ms, None, "no longer guarded");
        assert!(!kept.waiting(0));
        assert!(
            !remove_if(&path, &point.token).unwrap(),
            "the watchdog's token is spent"
        );
        assert!(!keep(&dir.path().join("none.json"), &point.token).unwrap());
    }

    #[test]
    fn a_file_that_is_not_a_revert_point_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revert.json");
        let point = RevertPoint::new(layout());
        save(&path, &point).unwrap();
        let without_token = fs::read_to_string(&path)
            .unwrap()
            .replace("revert_token", "something_else");
        fs::write(&path, without_token).unwrap();
        assert!(matches!(load(&path), Err(RevertError::Invalid { .. })));
        save(&path, &RevertPoint::guarding(layout(), 5)).unwrap();
        let guarded_text = fs::read_to_string(&path)
            .unwrap()
            .replace("_unix_ms\": 5", "_unix_ms\": \"soon\"");
        fs::write(&path, guarded_text).unwrap();
        assert!(matches!(load(&path), Err(RevertError::Invalid { .. })));
        fs::write(&path, "not json").unwrap();
        assert!(matches!(load(&path), Err(RevertError::Invalid { .. })));
    }
}
