//! Files dsplr owns (`docs/plan/05-persistence-and-profiles.md#rules-for-files-dsplr-writes`):
//! a first line naming dsplr and a SHA-256 of the rest, so a file edited by
//! hand since dsplr wrote it is recognised and not overwritten unasked; an
//! atomic write, so a crash mid-save never leaves half a config; and what a
//! save reports: the file it wrote ([`Saved`]), or the one it would not
//! overwrite unasked ([`Conflict`]). [`read_existing`] and [`replace`] are
//! the steps every save shares around its own format.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use dsplr_model::OutputId;
use sha2::{Digest, Sha256};

/// What starts a comment line in the file's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comment {
    /// `--`: Lua (`monitors.lua`).
    Lua,
    /// `#`: hyprlang, sway and most other config formats.
    Hash,
}

impl Comment {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Lua => "--",
            Self::Hash => "#",
        }
    }
}

/// What a file already on disk is, to dsplr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// dsplr wrote it and nobody has changed it since: safe to overwrite.
    Managed,
    /// dsplr wrote it, and it was edited since: overwrite only when asked.
    EditedByHand,
    /// Not dsplr's (no header): someone else's file, such as nwg-displays'.
    Foreign,
}

/// A save that was not written, because the file already there is not one
/// dsplr may simply overwrite. Saving again with `existing` approved
/// replaces it, as long as the file still holds exactly that text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {}", path.display(), self.why())]
pub struct Conflict {
    pub path: PathBuf,
    pub ownership: Ownership,
    /// Why dsplr could not read the file, when it could not: what it holds
    /// beyond that point would be lost, not carried into the new file.
    pub unreadable: Option<String>,
    /// The file as it is now.
    pub existing: String,
    /// What saving would write instead.
    pub new: String,
}

impl Conflict {
    /// Whether replacing the file takes `--force` rather than a yes: it was
    /// edited by hand, or holds something dsplr cannot carry over. A
    /// readable file someone else wrote takes only a yes
    /// (`docs/plan/05-persistence-and-profiles.md`, "Importing").
    pub const fn needs_force(&self) -> bool {
        self.unreadable.is_some() || matches!(self.ownership, Ownership::EditedByHand)
    }

    fn why(&self) -> String {
        let whose = match self.ownership {
            Ownership::Managed => "dsplr wrote it",
            Ownership::EditedByHand => "it was edited by hand since dsplr wrote it",
            Ownership::Foreign => "dsplr did not write it",
        };
        match &self.unreadable {
            Some(reason) => format!("{whose}, and dsplr cannot read all of it: {reason}"),
            None => whose.to_owned(),
        }
    }
}

/// Why a managed file could not be read or replaced.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The file is a symbolic link. Saving would replace the link with a
    /// file, so the change would never reach where the link points (a
    /// dotfiles checkout, say), and the next deploy would undo it.
    #[error(
        "{} is a link to {}; dsplr does not replace a link with a file, so change the file at its source instead",
        link.display(),
        target.display()
    )]
    Symlink { link: PathBuf, target: PathBuf },
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Conflict(Box<Conflict>),
}

impl FileError {
    /// The [`FileError::Io`] for `path`, to `map_err` an I/O result with.
    pub fn io(path: &Path) -> impl FnOnce(io::Error) -> Self + use<> {
        let path = path.to_owned();
        move |source| Self::Io { path, source }
    }
}

/// What the file a save would replace holds now: `None` when there is no
/// file. A symbolic link is refused ([`FileError::Symlink`]) before it is
/// read.
///
/// # Errors
///
/// When the file is a link, or cannot be read.
pub fn read_existing(path: &Path) -> Result<Option<String>, FileError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(path).map_err(FileError::io(path))?;
            return Err(FileError::Symlink {
                link: path.to_owned(),
                target,
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(FileError::io(path)(error)),
    }
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(FileError::io(path)(error)),
    }
}

/// Writes `new` over the file at `path`, which held `existing` when it was
/// read (`None`: there was none), if dsplr may:
/// - no file: it is created;
/// - a file dsplr wrote and nobody changed, which dsplr could read
///   (`unreadable` is `None`): it is overwritten;
/// - anything else: [`FileError::Conflict`], and nothing is written, unless
///   `approved` is exactly `existing`, which is how the user's answer to
///   that conflict comes back.
///
/// # Errors
///
/// A conflict, or the write failing.
pub fn replace(
    path: &Path,
    existing: Option<&str>,
    unreadable: Option<String>,
    new: String,
    approved: Option<&str>,
    comment: Comment,
) -> Result<(), FileError> {
    if let Some(existing) = existing {
        let ownership = ownership(existing, comment);
        let simple = ownership == Ownership::Managed && unreadable.is_none();
        if !simple && approved != Some(existing) {
            return Err(FileError::Conflict(Box::new(Conflict {
                path: path.to_owned(),
                ownership,
                unreadable,
                existing: existing.to_owned(),
                new,
            })));
        }
    }
    write_atomic(path, &new).map_err(FileError::io(path))
}

/// Why a save found no config to look in: what both desktops' lookups need
/// is unusable.
pub(crate) const NO_CONFIG_DIR: &str = "neither XDG_CONFIG_HOME nor HOME is set to an absolute path, so there is no config directory to look in";

/// `paths`, comma-separated, for an error listing where a lookup looked.
pub(crate) fn paths(paths: &[PathBuf]) -> String {
    let names: Vec<_> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    names.join(", ")
}

/// A managed file written by a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    /// Whether the file did not exist before.
    pub created: bool,
    /// The main config that has to include the file for the desktop to
    /// read it.
    pub main_config: PathBuf,
    /// The line that includes it there.
    pub include_line: &'static str,
    /// What the main config's own output settings are called
    /// (`hl.monitor calls`): the include line goes after them, since they
    /// override the file's when they come after it.
    pub rules: &'static str,
    /// Whether the main config was seen to include it already.
    pub included: bool,
    /// Whether the main config sets monitor rules after including it,
    /// which override the file's (on sway, on a reload). Moving the include
    /// below them fixes it.
    pub overridden: bool,
    /// The names under which the main config sets an output that the file
    /// names by its connector, each once, by sway's identifier: sway applies
    /// an identifier's setting over a name's at startup wherever each is, so
    /// these win then however the file is included. Removing those commands,
    /// or naming the outputs by connector there, fixes it. Always empty on
    /// Hyprland.
    pub beaten_at_startup: Vec<String>,
    /// Outputs the file named before that are not connected now, whose
    /// settings were carried over as they were.
    pub kept: Vec<OutputId>,
}

const MARKER: &str = "Managed by dsplr";
const HASH_KEY: &str = "sha256=";

/// `body` with dsplr's header line in front of it.
pub fn with_header(body: &str, comment: Comment) -> String {
    format!(
        "{} {MARKER}; edits are kept only if this line is removed. {HASH_KEY}{}\n{body}",
        comment.prefix(),
        sha256_hex(body),
    )
}

/// Whether `existing` is a file dsplr wrote, and whether it has been edited.
pub fn ownership(existing: &str, comment: Comment) -> Ownership {
    let (first, body) = existing.split_once('\n').unwrap_or((existing, ""));
    let Some(header) = first
        .strip_prefix(comment.prefix())
        .filter(|header| header.trim_start().starts_with(MARKER))
    else {
        return Ownership::Foreign;
    };
    match header.rsplit_once(HASH_KEY) {
        Some((_, hash)) if hash.trim() == sha256_hex(body) => Ownership::Managed,
        _ => Ownership::EditedByHand,
    }
}

/// Lowercase hex of SHA-256 over `text`'s UTF-8 bytes.
fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Replaces `path` with `contents` so that a reader, or a crash, sees the
/// old file or the new one, never part of either: the contents go to a new
/// file beside it, are flushed to disk, and are renamed over `path`. The
/// temporary file is removed if any step fails.
///
/// # Errors
///
/// When `path` has no parent directory or file name, or any step fails.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a file path", path.display()),
        ));
    };
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".dsplr-{}.tmp", std::process::id()));
    let temporary = dir.join(temporary_name);
    let written = (|| {
        let mut file: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = written {
        // The write already failed; a temporary file that cannot be removed
        // either is reported with it rather than instead of it.
        return Err(match fs::remove_file(&temporary) {
            Ok(()) => error,
            Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => error,
            Err(cleanup) => io::Error::new(
                error.kind(),
                format!(
                    "{error}; and {} was left behind: {cleanup}",
                    temporary.display()
                ),
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Comment, Ownership, ownership, sha256_hex, with_header, write_atomic};

    #[test]
    fn sha256_matches_the_standards_test_vectors() {
        // FIPS 180-2, appendix B.1 and B.2.
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn a_file_dsplr_wrote_is_managed_until_edited() {
        let body = "hl.monitor({\n    output = \"eDP-1\",\n    disabled = true,\n})\n";
        let file = with_header(body, Comment::Lua);
        assert!(file.starts_with("-- Managed by dsplr; "), "{file}");
        assert!(file.ends_with(body));
        assert_eq!(ownership(&file, Comment::Lua), Ownership::Managed);

        let edited = file.replace("true", "false");
        assert_eq!(ownership(&edited, Comment::Lua), Ownership::EditedByHand);
        let appended = format!("{file}-- a note\n");
        assert_eq!(ownership(&appended, Comment::Lua), Ownership::EditedByHand);
    }

    #[test]
    fn a_file_without_the_header_is_someone_elses() {
        let nwg = "-- Generated by nwg-displays on 2026-05-21 at 15:52:53. Do not edit manually.\n";
        assert_eq!(ownership(nwg, Comment::Lua), Ownership::Foreign);
        assert_eq!(ownership("", Comment::Lua), Ownership::Foreign);
        // The header's own language matters: a `#` header is not a Lua one.
        let hashed = with_header("monitor = eDP-1, disable\n", Comment::Hash);
        assert_eq!(ownership(&hashed, Comment::Hash), Ownership::Managed);
        assert_eq!(ownership(&hashed, Comment::Lua), Ownership::Foreign);
    }

    #[test]
    fn a_damaged_hash_counts_as_edited() {
        let file = with_header("body\n", Comment::Hash);
        let damaged = file.replacen("sha256=", "sha256=0", 1);
        assert_eq!(ownership(&damaged, Comment::Hash), Ownership::EditedByHand);
        let no_hash = "# Managed by dsplr\nbody\n";
        assert_eq!(ownership(no_hash, Comment::Hash), Ownership::EditedByHand);
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("monitors.lua");
        write_atomic(&path, "first\n").unwrap();
        write_atomic(&path, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["monitors.lua"]);
    }

    #[test]
    fn a_failed_atomic_write_leaves_the_old_file_and_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        // The target is a directory, so the final rename fails.
        let path = dir.path().join("taken");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), "kept").unwrap();
        assert!(write_atomic(&path, "new\n").is_err());
        assert_eq!(fs::read_to_string(path.join("inside")).unwrap(), "kept");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["taken"]);
    }
}
