//! autorandr's profiles read from disk as autorandr reads them.
//!
//! The format is autorandr's (`autorandr.py`, commit `0855315d`, version
//! 1.15, <https://github.com/phillipberndt/autorandr>): a directory per
//! profile holding `config` and `setup`. Only the format is followed here;
//! none of autorandr's code is. Where the text says what autorandr does with
//! an odd file, it was checked by running its `load_profiles` on one.
//!
//! - `setup` (written by `output_setup`) is one `OUTPUT EDID` line per
//!   connected output: the EDID as hex, or a pattern with one `*`, or an
//!   older autorandr's 32-character MD5 of it, or
//!   `--CONNECTED-BUT-EDID-UNAVAILABLE--OUTPUT` (`EDID_UNAVAILABLE` and the
//!   output's name) for an output it read none for. Blank lines and lines
//!   starting `#` are skipped.
//! - `config` (written by `output_configuration`, each output's
//!   `option_string`) is a block per output, started by a line beginning
//!   `output`; each line is a key and, after whitespace, its value, which
//!   are `xrandr --output` options without their `--` (`mode`, `pos`,
//!   `rate`, `rotate`, `reflect`, `primary`, `off`, `panning`,
//!   `transform`, `gamma`, `crtc`, `filter`, `x-prop-*`, as
//!   `from_xrandr_output` records them; a hand-written one may hold any
//!   other, which `option_vector` passes on to xrandr). Blank lines, and
//!   keys starting `#`, are skipped; a later line for a key replaces an
//!   earlier one, and a later block for the same output replaces an earlier
//!   one, in its place. A block is the output its first line names: an
//!   indented `output X` inside it is an option, under which autorandr looks
//!   up the EDID and drives `X`, while the profile still lists the block by
//!   its first name (`load_profiles` keys it so); the import does the same.
//! - An output's EDID is the `setup` line of its name, else of the name
//!   with `-` and `cardN` removed from both (`from_config_file`, for older
//!   autorandr's names). An output `off` with none is dropped
//!   (`load_profiles`).
//!
//! autorandr fails to load every profile, and so does nothing, when one of
//! them has a `setup` line not of two words, a `config` line before its
//! first block or one of only whitespace inside a block, or an output on
//! with no EDID. dsplr skips just that profile, with why, and imports the
//! rest: deliberately, since the others are as autorandr wrote them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::import::Skipped;
use crate::managed::{config_home, home};

/// The hook scripts autorandr runs (`exec_scripts`), from a profile's
/// directory and from its own directories, each also as a `NAME.d`
/// directory of scripts.
const SCRIPTS: [&str; 5] = ["block", "preswitch", "postswitch", "postsave", "predetect"];

/// One profile, as autorandr loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorandrProfile {
    /// Its directory's name, which is the profile's name.
    pub name: String,
    pub dir: PathBuf,
    /// Its outputs, in the order `config` gives them.
    pub outputs: Vec<AutorandrOutput>,
    /// Outputs `off` in `config` with no `setup` line, which autorandr
    /// drops.
    pub dropped: Vec<String>,
    /// The hook scripts in its directory autorandr would run.
    pub scripts: Vec<String>,
}

/// One output of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorandrOutput {
    /// The name its block's first line gives.
    pub name: String,
    /// Its `setup` fingerprint.
    pub edid: String,
    /// Its `config` lines other than `output`, each key once (a later line
    /// for a key replaces an earlier one), in the order first given.
    pub options: Options,
}

impl AutorandrOutput {
    /// The value of option `key`: `None` when it is not given, `Some(None)`
    /// when it is given without one.
    pub fn option(&self, key: &str) -> Option<Option<&str>> {
        self.options
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_deref())
    }
}

/// What reading found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    /// Each profile read, by name: a later directory's profile replaces an
    /// earlier one of the same name, as autorandr loads them.
    pub profiles: Vec<AutorandrProfile>,
    /// Each profile autorandr would fail to load, and why.
    pub unreadable: Vec<Skipped>,
    /// What else to tell the user.
    pub notes: Vec<String>,
}

/// Why nothing could be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("none of autorandr's directories exists ({})", crate::managed::paths(.0))]
    NoDirectory(Vec<PathBuf>),
}

/// autorandr's profile directories, in the order it loads them, a later
/// one's profile replacing an earlier one's of the same name (`main`): each
/// of `$XDG_CONFIG_DIRS` (`/etc/xdg` when unset), last first, with
/// `/autorandr`; then `~/.autorandr` when it is a directory, else the
/// config home's `autorandr` (`managed::config_home`: `$XDG_CONFIG_HOME`,
/// else `~/.config`).
///
/// autorandr takes a relative value as it is, against its working
/// directory: an empty `XDG_CONFIG_DIRS` (set, so not `/etc/xdg`) is one
/// empty entry, so it loads `./autorandr`; an empty or relative
/// `XDG_CONFIG_HOME` gives a relative directory too. dsplr does not look
/// in a relative directory, as the XDG spec has them ignored and dsplr's
/// other lookups ignore them: such an entry is left out, so an empty
/// `XDG_CONFIG_DIRS` gives no system directory, and the config home falls
/// back to `~/.config`. `HOME` too must be absolute.
pub fn default_dirs(var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let system = var("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into());
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&system)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("autorandr"))
        .collect();
    dirs.reverse();
    let legacy = home(&var).map(|home| home.join(".autorandr"));
    let user = match legacy {
        Some(legacy) if legacy.is_dir() => Some(legacy),
        _ => config_home(&var).map(|config| config.join("autorandr")),
    };
    dirs.extend(user);
    dirs
}

/// The profiles in each of `dirs` that exists, as autorandr loads them.
///
/// A link in a profile directory is a profile of its own, as any directory
/// holding `config` and `setup` is, unless its target, as written, is the
/// name of a profile loaded: then autorandr takes it as another name for
/// that profile (`get_symlinks`, filtered in `main`), and it is not
/// imported, which is said. So a link a dotfile manager made to a profile
/// kept elsewhere is imported. autorandr also takes a link naming one of
/// its built-in profiles (`virtual_profiles`) as a name for it, but such a
/// link leads to no `config` and `setup` (or to a profile of that name,
/// which is loaded), so it is no profile to import either way.
///
/// # Errors
///
/// When none of `dirs` exists, or one cannot be listed.
pub fn read(dirs: &[PathBuf]) -> Result<Read, ReadError> {
    let existing: Vec<&PathBuf> = dirs.iter().filter(|dir| dir.is_dir()).collect();
    if existing.is_empty() {
        return Err(ReadError::NoDirectory(dirs.to_vec()));
    }
    let mut read = Read::default();
    // Each link's name and target as written; a later one replaces one of
    // the same name.
    let mut links: Vec<(String, PathBuf)> = Vec::new();
    for dir in existing {
        let global = scripts(dir);
        if !global.is_empty() {
            read.notes.push(format!(
                "autorandr's own {} in {} not run by dsplr",
                said_scripts(&global),
                dir.display()
            ));
        }
        let io = |source| ReadError::Io {
            path: dir.clone(),
            source,
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()
            .map_err(io)?;
        entries.sort();
        for path in entries {
            let name = file_name(&path);
            if let Ok(target) = std::fs::read_link(&path) {
                links.retain(|(other, _)| *other != name);
                links.push((name.clone(), target));
            }
            if !is_profile(&path) {
                continue;
            }
            let from = from(&name, &path);
            read.profiles.retain(|profile| profile.name != name);
            read.unreadable
                .retain(|other| !other.from.starts_with(&from_prefix(&name)));
            match read_profile(&path) {
                Ok(profile) => read.profiles.push(profile),
                Err(reason) => read.unreadable.push(Skipped { from, reason }),
            }
        }
    }
    for (name, target) in links {
        let target = target.to_string_lossy();
        let alias = read.profiles.iter().any(|profile| profile.name == target);
        let loaded = read.profiles.iter().any(|profile| profile.name == name)
            || read
                .unreadable
                .iter()
                .any(|other| other.from.starts_with(&from_prefix(&name)));
        if alias && loaded {
            read.profiles.retain(|profile| profile.name != name);
            read.unreadable
                .retain(|other| !other.from.starts_with(&from_prefix(&name)));
            read.notes.push(format!(
                "autorandr's \"{name}\" is a link to \"{target}\", which autorandr reads as another name for that profile, so it is not imported on its own"
            ));
        }
    }
    Ok(read)
}

/// The name of the file at `path`.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// How a profile is named to the user: its name and where it is.
fn from(name: &str, path: &Path) -> String {
    format!("{}{})", from_prefix(name), path.display())
}

fn from_prefix(name: &str) -> String {
    format!("\"{name}\" (")
}

/// The one profile in `dir`, which holds `config` and `setup`, as [`read`]
/// gives profiles: in [`Read::profiles`], or in [`Read::unreadable`] with
/// why.
pub fn read_one(dir: &Path) -> Read {
    let mut read = Read::default();
    match read_profile(dir) {
        Ok(profile) => read.profiles.push(profile),
        Err(reason) => read.unreadable.push(Skipped {
            from: from(&file_name(dir), dir),
            reason,
        }),
    }
    read
}

/// Whether `dir` is one profile's directory, holding `config` and `setup`,
/// rather than a directory of profiles.
pub fn is_profile(dir: &Path) -> bool {
    dir.join("config").is_file() && dir.join("setup").is_file()
}

/// The hook scripts in `dir` autorandr would run: each of [`SCRIPTS`] that
/// is an executable file, and each executable file in its `.d` directory.
fn scripts(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for script in SCRIPTS {
        if executable(&dir.join(script)) {
            found.push(script.to_owned());
        }
        let folder = dir.join(format!("{script}.d"));
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| executable(&entry.path()))
            .map(|entry| format!("{script}.d/{}", entry.file_name().to_string_lossy()))
            .collect();
        names.sort();
        found.extend(names);
    }
    found
}

/// `scripts` as a sentence's subject: "the postswitch script is", "the
/// postswitch, preswitch.d/x scripts are".
pub(super) fn said_scripts(scripts: &[String]) -> String {
    format!(
        "{} {}",
        scripts.join(", "),
        if scripts.len() == 1 {
            "script is"
        } else {
            "scripts are"
        }
    )
}

/// Whether `path` is a file autorandr would run (`os.access` with `X_OK`):
/// on Unix, one with an execute bit, which is what that check comes to
/// for its owner; elsewhere there is no execute bit, and Python's
/// `os.access` grants `X_OK` to any file, so every file counts. Verified on
/// Linux only.
fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The profile in `dir`, which holds `config` and `setup`.
///
/// # Errors
///
/// Why autorandr would fail to load it.
pub fn read_profile(dir: &Path) -> Result<AutorandrProfile, String> {
    let text = |file: &str| {
        let path = dir.join(file);
        std::fs::read_to_string(&path).map_err(|error| format!("cannot read {file}: {error}"))
    };
    let setup = parse_setup(&text("setup")?)?;
    let blocks = parse_config(&text("config")?)?;
    let mut profile = AutorandrProfile {
        name: file_name(dir),
        dir: dir.to_owned(),
        outputs: Vec::new(),
        dropped: Vec::new(),
        scripts: scripts(dir),
    };
    for block in blocks {
        let off = block.options.iter().any(|(key, _)| key == "off");
        match edid_for(&setup, &block.driven) {
            Some(edid) => profile.outputs.push(AutorandrOutput {
                name: block.name,
                edid: edid.to_owned(),
                options: block.options,
            }),
            None if off => profile.dropped.push(block.name),
            None => {
                return Err(format!(
                    "`{}` is on in config but has no EDID in setup, which autorandr refuses",
                    block.driven
                ));
            }
        }
    }
    Ok(profile)
}

/// `setup`'s `OUTPUT EDID` pairs.
fn parse_setup(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let [output, edid] = words[..] else {
            return Err(format!(
                "setup line {} is not an output and its EDID, which autorandr fails to load",
                index + 1
            ));
        };
        pairs.retain(|(other, _): &(String, String)| other != output);
        pairs.push((output.to_owned(), edid.to_owned()));
    }
    Ok(pairs)
}

/// The EDID `setup` gives `output`: by its name, else by the name with `-`
/// and `cardN` taken out of both.
fn edid_for<'a>(setup: &'a [(String, String)], output: &str) -> Option<&'a str> {
    let exact = setup.iter().find(|(name, _)| name == output);
    exact
        .or_else(|| {
            let fuzzy = legacy_name(output);
            setup.iter().find(|(name, _)| legacy_name(name) == fuzzy)
        })
        .map(|(_, edid)| edid.as_str())
}

/// `name` with each `-`, and each `card` followed by digits, taken out.
fn legacy_name(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("card") {
            let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits > 0 {
                rest = &after[digits..];
                continue;
            }
        }
        let mut chars = rest.chars();
        let c = chars.next().expect("not empty");
        if c != '-' {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// An output's `config` options: each key, and its value when it has one.
type Options = Vec<(String, Option<String>)>;

/// One `config` block.
#[derive(Debug, PartialEq, Eq)]
struct Block {
    /// The name its first line gives.
    name: String,
    /// The output its `output` option names, the last one given: the one
    /// autorandr looks up the EDID of and drives.
    driven: String,
    /// Its options other than `output`.
    options: Options,
}

/// `config`'s blocks.
fn parse_config(text: &str) -> Result<Vec<Block>, String> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.starts_with("output") {
            blocks.push(vec![line]);
        } else if !line.is_empty() && line.trim().is_empty() {
            return Err(format!(
                "config line {} is only whitespace, which autorandr fails to load",
                index + 1
            ));
        } else if let Some(block) = blocks.last_mut() {
            block.push(line);
        } else {
            return Err(
                "config has a line before its first `output` line, which autorandr fails to load"
                    .into(),
            );
        }
    }

    let mut outputs: Vec<Block> = Vec::new();
    for lines in blocks {
        let name = lines[0]
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .to_owned();
        let mut options: Options = Vec::new();
        for (key, value) in lines.into_iter().filter_map(option) {
            match options.iter_mut().find(|(other, _)| *other == key) {
                Some(slot) => slot.1 = value,
                None => options.push((key, value)),
            }
        }
        let index = options
            .iter()
            .position(|(key, _)| key == "output")
            .ok_or_else(|| format!("config's block `{name}` names no output"))?;
        let driven = options
            .remove(index)
            .1
            .ok_or_else(|| "a config `output` line names no output".to_owned())?;
        let block = Block {
            name,
            driven,
            options,
        };
        match outputs.iter_mut().find(|other| other.name == block.name) {
            Some(slot) => *slot = block,
            None => outputs.push(block),
        }
    }
    Ok(outputs)
}

/// A `config` line's key and value; `None` for a blank line or a comment.
fn option(line: &str) -> Option<(String, Option<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    match line.split_once(char::is_whitespace) {
        Some((key, value)) => Some((key.to_owned(), Some(value.trim().to_owned()))),
        None => Some((line.to_owned(), None)),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Block, legacy_name, parse_config, parse_setup, read, read_profile};

    fn write(dir: &Path, file: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
    }

    /// An executable script `file` in `dir`.
    #[cfg(unix)]
    fn script(dir: &Path, file: &str) {
        use std::os::unix::fs::PermissionsExt;
        write(dir, file, "#!/bin/sh\n");
        let path = dir.join(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn names(blocks: &[Block]) -> Vec<&str> {
        blocks.iter().map(|block| block.name.as_str()).collect()
    }

    /// A `config` as autorandr's `output_configuration` writes one: blocks
    /// sorted by position, `off` alone, keys sorted, and an option without
    /// a value written bare (`option_string`).
    const CONFIG: &str = "output HDMI-1\noff\noutput eDP-1\ncrtc 0\nmode 1920x1080\npos 0x0\nprimary\nrate 60.00\noutput DP-1\ncrtc 1\nmode 2560x1440\npos 1920x0\nrate 59.95\n";

    #[test]
    fn config_blocks_are_read_as_autorandr_reads_them() {
        let blocks = parse_config(CONFIG).unwrap();
        assert_eq!(names(&blocks), ["HDMI-1", "eDP-1", "DP-1"]);
        assert_eq!(blocks[1].driven, "eDP-1");
        assert_eq!(blocks[0].options, [("off".to_owned(), None)]);
        assert_eq!(
            blocks[1].options[..3],
            [
                ("crtc".to_owned(), Some("0".to_owned())),
                ("mode".to_owned(), Some("1920x1080".to_owned())),
                ("pos".to_owned(), Some("0x0".to_owned())),
            ]
        );
        assert_eq!(blocks[1].options[3], ("primary".to_owned(), None));
    }

    #[test]
    fn comments_and_repeats_are_read_as_autorandr_reads_them() {
        let blocks = parse_config(
            "output A\n# a comment\nmode 640x480\n\nmode 1024x768\noutput B\noff\noutput A\npos 5x5\n",
        )
        .unwrap();
        // A's later block replaces its first, in its first place.
        assert_eq!(names(&blocks), ["A", "B"]);
        assert_eq!(
            blocks[0].options,
            [("pos".to_owned(), Some("5x5".to_owned()))]
        );
        // A later `mode` replaces the earlier one.
        let first = parse_config("output A\nmode 640x480\nmode 1024x768\n").unwrap();
        assert_eq!(
            first[0].options,
            [("mode".to_owned(), Some("1024x768".to_owned()))]
        );
        assert_eq!(parse_config("").unwrap(), []);
    }

    #[test]
    fn an_indented_output_line_drives_another_output_under_the_first_name() {
        // autorandr's `load_profiles` keys this block `DP-1`, and looks up
        // the EDID of, and drives, `X`.
        let blocks = parse_config("output DP-1\n  output X\nmode 640x480\n").unwrap();
        assert_eq!(
            (blocks[0].name.as_str(), blocks[0].driven.as_str()),
            ("DP-1", "X")
        );
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("p");
        write(
            &profile,
            "config",
            "output DP-1\n  output X\nmode 640x480\n",
        );
        write(&profile, "setup", "DP-1 00\nX 11\n");
        let read = read_profile(&profile).unwrap();
        assert_eq!(
            (read.outputs[0].name.as_str(), read.outputs[0].edid.as_str()),
            ("DP-1", "11")
        );
    }

    #[test]
    fn lines_autorandr_fails_on_are_refused() {
        // autorandr's own `load_profiles` raises on each of these: a line
        // before the first block, blank or not, and a line of whitespace
        // inside one.
        for config in [
            "# saved\noutput A\n",
            "\noutput A\n",
            "mode 640x480\noutput A\n",
            "output A\n   \nmode 640x480\n",
            "output A\nmode 640x480\n\t\n",
        ] {
            assert!(parse_config(config).is_err(), "{config:?}");
        }
    }

    #[test]
    fn setup_lines_are_an_output_and_its_edid() {
        let setup = parse_setup("# fingerprints\n\n   \nDP-1 00ff\n  eDP-1   abcd  \n").unwrap();
        assert_eq!(
            setup,
            [
                ("DP-1".to_owned(), "00ff".to_owned()),
                ("eDP-1".to_owned(), "abcd".to_owned())
            ]
        );
        assert!(parse_setup("DP-1\n").is_err());
        assert!(parse_setup("DP-1 a b\n").is_err());
    }

    #[test]
    fn older_names_match_without_dashes_or_cards() {
        assert_eq!(legacy_name("card0-HDMI-A-1"), "HDMIA1");
        assert_eq!(legacy_name("HDMI-A-1"), "HDMIA1");
        assert_eq!(legacy_name("cardboard"), "cardboard");
    }

    #[test]
    fn an_output_off_without_an_edid_is_dropped_and_one_on_fails_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("docked");
        write(
            &profile,
            "config",
            "output DP-1\nmode 1920x1080\noutput VGA-1\noff\n",
        );
        write(&profile, "setup", "card0-DP-1 00\n");
        let read = read_profile(&profile).unwrap();
        assert_eq!(read.name, "docked");
        assert_eq!(read.outputs.len(), 1);
        assert_eq!(read.outputs[0].edid, "00", "by its older name");
        assert_eq!(read.dropped, ["VGA-1"]);

        write(&profile, "setup", "HDMI-1 00\n");
        let refused = read_profile(&profile).unwrap_err();
        assert!(refused.contains("DP-1"), "{refused}");
    }

    #[test]
    fn a_later_directory_replaces_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (system, user) = (dir.path().join("system"), dir.path().join("user"));
        write(&system.join("docked"), "config", "output DP-1\noff\n");
        write(&system.join("docked"), "setup", "DP-1 00\n");
        write(
            &user.join("docked"),
            "config",
            "output DP-1\nmode 640x480\n",
        );
        write(&user.join("docked"), "setup", "DP-1 00\n");
        write(&user, "settings.ini", "[config]\n");
        let read = read(&[system, user.clone(), dir.path().join("missing")]).unwrap();
        assert_eq!(read.profiles.len(), 1);
        assert_eq!(read.profiles[0].dir, user.join("docked"));
        assert_eq!(read.notes, Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_naming_a_profile_is_a_name_for_it_and_any_other_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (autorandr, dotfiles) = (dir.path().join("autorandr"), dir.path().join("dotfiles"));
        write(
            &autorandr.join("docked"),
            "config",
            "output DP-1\nmode 640x480\n",
        );
        write(&autorandr.join("docked"), "setup", "DP-1 00\n");
        write(
            &dotfiles.join("mobile"),
            "config",
            "output eDP-1\nmode 640x480\n",
        );
        write(&dotfiles.join("mobile"), "setup", "eDP-1 00\n");
        // As `ln -s docked default`, autorandr's way to name one; and as a
        // dotfile manager links a profile kept elsewhere; and a name for a
        // built-in profile, which holds no profile to read.
        std::os::unix::fs::symlink("docked", autorandr.join("default")).unwrap();
        std::os::unix::fs::symlink(dotfiles.join("mobile"), autorandr.join("mobile")).unwrap();
        std::os::unix::fs::symlink("horizontal", autorandr.join("wide")).unwrap();

        let read = read(&[autorandr]).unwrap();
        let names: Vec<&str> = read.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["docked", "mobile"]);
        assert_eq!(read.notes.len(), 1, "{:?}", read.notes);
        assert!(read.notes[0].starts_with("autorandr's \"default\" is a link to \"docked\""));
    }

    #[cfg(unix)]
    #[test]
    fn executable_hook_scripts_are_found_in_profiles_and_autorandr_s_directory() {
        let dir = tempfile::tempdir().unwrap();
        let autorandr = dir.path().join("autorandr");
        let docked = autorandr.join("docked");
        write(&docked, "config", "output DP-1\nmode 640x480\n");
        write(&docked, "setup", "DP-1 00\n");
        script(&docked, "postswitch");
        script(&docked.join("preswitch.d"), "notify");
        // Not executable, so autorandr runs neither.
        write(&docked, "block", "#!/bin/sh\n");
        write(&docked.join("preswitch.d"), "README", "notes\n");
        script(&autorandr, "postswitch");

        let read = read(std::slice::from_ref(&autorandr)).unwrap();
        assert_eq!(
            read.profiles[0].scripts,
            ["preswitch.d/notify", "postswitch"]
        );
        assert_eq!(
            read.notes,
            [format!(
                "autorandr's own postswitch script is in {} not run by dsplr",
                autorandr.display()
            )]
        );
    }

    #[test]
    fn no_directory_at_all_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&[dir.path().join("missing")]).is_err());
    }

    // POSIX paths (`/etc/...` absolute, `/` the separator), which the
    // Linux-only code is written for.
    #[cfg(unix)]
    #[test]
    fn the_directories_are_autorandr_s() {
        use std::ffi::OsString;
        use std::path::PathBuf;

        use super::default_dirs;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_owned();
        let vars = |config_dirs: Option<&str>, config_home: Option<&str>| {
            let home = home.clone();
            let (config_dirs, config_home) = (
                config_dirs.map(OsString::from),
                config_home.map(OsString::from),
            );
            move |name: &str| match name {
                "HOME" => Some(home.clone().into_os_string()),
                "XDG_CONFIG_DIRS" => config_dirs.clone(),
                "XDG_CONFIG_HOME" => config_home.clone(),
                _ => None,
            }
        };
        assert_eq!(
            default_dirs(vars(None, None)),
            [
                PathBuf::from("/etc/xdg/autorandr"),
                home.join(".config/autorandr")
            ]
        );
        assert_eq!(
            default_dirs(vars(Some("/a:/b"), Some("/c"))),
            [
                PathBuf::from("/b/autorandr"),
                PathBuf::from("/a/autorandr"),
                PathBuf::from("/c/autorandr")
            ]
        );
        // Set empty, `XDG_CONFIG_DIRS` is one relative entry to autorandr,
        // which dsplr leaves out, as it does any relative entry; an empty or
        // relative config home falls back to `~/.config`.
        assert_eq!(
            default_dirs(vars(Some(""), Some(""))),
            [home.join(".config/autorandr")]
        );
        assert_eq!(
            default_dirs(vars(Some("rel:/a"), Some("rel"))),
            [
                PathBuf::from("/a/autorandr"),
                home.join(".config/autorandr")
            ]
        );
        std::fs::create_dir(home.join(".autorandr")).unwrap();
        assert_eq!(
            default_dirs(vars(None, Some("/c")))[1],
            home.join(".autorandr")
        );
    }
}
