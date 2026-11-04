//! Files dsplr owns (`docs/plan/05-persistence-and-profiles.md#rules-for-files-dsplr-writes`):
//! a first line naming dsplr and a SHA-256 of the rest, so a file edited by
//! hand since dsplr wrote it is recognised and not overwritten unasked; and
//! an atomic write, so a crash mid-save never leaves half a config.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

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
