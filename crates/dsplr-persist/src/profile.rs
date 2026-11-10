//! dsplr's profile store (`docs/plan/05-persistence-and-profiles.md`,
//! "dsplr's profile store"): named layouts in
//! `$XDG_CONFIG_HOME/dsplr/profiles/<name>.json`, for desktops that keep no
//! display config of their own and for anyone who wants named layouts.
//!
//! A profile file is a layout in its JSON form (`dsplr list --json`), so
//! `dsplr apply` reads one too, with the time it was saved beside the
//! layout's fields. The outputs a profile applies to are its entries'
//! identities: a profile saved from the displays names every output that was
//! connected, on or off. [`pick`] chooses the profile for the outputs
//! connected now: one for exactly those, as kanshi picks, else, as dsplr's
//! own fallback, one for some of them.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fmt, fs, io};

use dsplr_model::{Layout, OutputId, assign};
use serde_json::{Value, json};

use crate::layout_json;
use crate::managed::{FileError, refuse_symlink, write_atomic};

/// The extension of a profile file.
const EXTENSION: &str = "json";

/// The field a profile keeps its save time in, beside the layout's own.
const SAVED_KEY: &str = "saved_unix_ms";

/// The longest name, in bytes: with the extension it stays well under the
/// 255 bytes a file name can have.
const MAX_NAME_BYTES: usize = 200;

/// A profile's name, checked to be usable as a file name: not empty, at most
/// [`MAX_NAME_BYTES`] long, made of ASCII letters and digits, spaces, `-`,
/// `_` and `.`, starting with a letter, digit or `_` (so never `.` or `-`),
/// not ending in a space or `.`, and without `..`. So it names a file in the
/// store directory and nothing outside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileName(String);

/// Why a name cannot be a profile's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a profile name cannot be empty")]
    Empty,
    #[error("a profile name is at most {MAX_NAME_BYTES} bytes long")]
    TooLong,
    #[error("a profile name starts with a letter, a digit or '_', not {0:?}")]
    Start(char),
    #[error("a profile name cannot end with {0:?}")]
    End(char),
    #[error("a profile name cannot contain \"..\"")]
    DotDot,
    #[error(
        "a profile name is ASCII letters and digits, spaces, '-', '_' and '.', so it cannot contain {0:?}"
    )]
    Character(char),
}

impl FromStr for ProfileName {
    type Err = NameError;

    fn from_str(name: &str) -> Result<Self, NameError> {
        let Some(first) = name.chars().next() else {
            return Err(NameError::Empty);
        };
        if name.len() > MAX_NAME_BYTES {
            return Err(NameError::TooLong);
        }
        if let Some(bad) = name
            .chars()
            .find(|&c| !(c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')))
        {
            return Err(NameError::Character(bad));
        }
        if !(first.is_ascii_alphanumeric() || first == '_') {
            return Err(NameError::Start(first));
        }
        if let Some(last @ (' ' | '.')) = name.chars().next_back() {
            return Err(NameError::End(last));
        }
        if name.contains("..") {
            return Err(NameError::DotDot);
        }
        Ok(Self(name.to_owned()))
    }
}

impl ProfileName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A named layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: ProfileName,
    pub layout: Layout,
    /// When it was saved, in milliseconds since the Unix epoch. A file
    /// without it (a layout written by `dsplr list --json`, or by hand) takes
    /// its modification time.
    pub saved_unix_ms: u64,
}

/// How a profile's outputs fit the outputs connected now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// Each of its outputs is connected, and so are others it leaves as they
    /// are.
    Subset,
    /// Its outputs are exactly the ones connected.
    Exact,
}

impl Profile {
    /// The identities of the outputs it applies to.
    pub fn outputs(&self) -> impl Iterator<Item = &OutputId> {
        self.layout.entries.iter().map(|entry| &entry.id)
    }

    /// How it fits `live`, the displays as they are now: each of its
    /// outputs must have its own connected output that it names, paired as
    /// `dsplr apply` pairs them ([`dsplr_model::assign`]; a monitor with a
    /// serial matches on any port, one without on its connector). `None`
    /// when they cannot all be paired, or it has no outputs.
    pub fn fit(&self, live: &Layout) -> Option<Fit> {
        if self.layout.entries.is_empty() {
            return None;
        }
        let paired = assign(&self.layout.entries, &live.entries).ok()?;
        Some(if paired.len() == live.entries.len() {
            Fit::Exact
        } else {
            Fit::Subset
        })
    }
}

/// The profile for `live`, the displays as they are now: one whose outputs
/// are exactly the connected ones (kanshi's rule), else one covering some of
/// them (dsplr's own fallback; kanshi has none); among several, the most
/// recently saved, and between two saved at the same moment, the first by
/// name. `None` when no profile fits.
pub fn pick<'a>(profiles: &'a [Profile], live: &Layout) -> Option<(&'a Profile, Fit)> {
    profiles
        .iter()
        .filter_map(|profile| profile.fit(live).map(|fit| (profile, fit)))
        .max_by(|(a, a_fit), (b, b_fit)| {
            a_fit
                .cmp(b_fit)
                .then(a.saved_unix_ms.cmp(&b.saved_unix_ms))
                .then_with(|| b.name.cmp(&a.name))
        })
}

/// Milliseconds since the Unix epoch at `time`; 0 for a time before it.
pub fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

/// Why the store could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("no home directory to keep profiles in")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not a dsplr profile: {reason}")]
    Invalid { path: PathBuf, reason: String },
    /// A file in the store whose name is not a profile's, which is passed
    /// over rather than read.
    #[error("{path} is passed over: its name is not a profile's: {reason}")]
    Name { path: PathBuf, reason: String },
    /// The profile file is a symbolic link, which saving would replace.
    #[error(transparent)]
    File(#[from] FileError),
}

impl ProfileError {
    fn io(path: &Path) -> impl FnOnce(io::Error) -> Self + use<> {
        let path = path.to_owned();
        move |source| Self::Io { path, source }
    }
}

/// `$XDG_CONFIG_HOME/dsplr/profiles`.
///
/// # Errors
///
/// When there is no home directory to resolve it from.
pub fn default_dir() -> Result<PathBuf, ProfileError> {
    hjkl_xdg::config_dir("dsplr")
        .map(|dir| dir.join("profiles"))
        .map_err(|hjkl_xdg::Error::NoHomeDir| ProfileError::NoHome)
}

/// The file the profile `name` is kept in, in the store at `dir`.
pub fn path(dir: &Path, name: &ProfileName) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

/// The profile `name` in the store at `dir`, or `None` when there is none.
///
/// # Errors
///
/// When its file cannot be read or is not a profile.
pub fn load(dir: &Path, name: &ProfileName) -> Result<Option<Profile>, ProfileError> {
    let path = path(dir, name);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ProfileError::io(&path)(error)),
    };
    let invalid = |reason: String| ProfileError::Invalid {
        path: path.clone(),
        reason,
    };
    let (layout, saved) = layout_json::read(&text, SAVED_KEY).map_err(invalid)?;
    let saved_unix_ms = match saved {
        Some(saved) => saved
            .as_u64()
            .ok_or_else(|| invalid(format!("`{SAVED_KEY}` is not a whole number: {saved}")))?,
        None => fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map(unix_ms)
            .map_err(ProfileError::io(&path))?,
    };
    Ok(Some(Profile {
        name: name.clone(),
        layout,
        saved_unix_ms,
    }))
}

/// Every profile in the store at `dir`, by file name, each with the error
/// that kept it from being read, if one did. A `.json` file whose name is
/// not a profile's is [`ProfileError::Name`]; files with other extensions
/// are not the store's and are left out. No directory is no profiles.
///
/// # Errors
///
/// When the directory cannot be read.
pub fn list(dir: &Path) -> Result<Vec<Result<Profile, ProfileError>>, ProfileError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ProfileError::io(dir)(error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(ProfileError::io(dir))?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == EXTENSION)
        {
            paths.push(path);
        }
    }
    paths.sort();
    // A profile removed since the directory was read is passed over too.
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| "it is not UTF-8".to_owned())
                .and_then(|stem| stem.parse::<ProfileName>().map_err(|e| e.to_string()));
            match name {
                Ok(name) => load(dir, &name).transpose(),
                Err(reason) => Some(Err(ProfileError::Name { path, reason })),
            }
        })
        .collect())
}

/// Writes `profile` to the store at `dir`, creating the directory, and
/// replacing a profile of that name. The write is atomic. Returns the path
/// written.
///
/// # Errors
///
/// When the profile's file is a symbolic link ([`FileError::Symlink`]), or
/// the directory or the file cannot be written.
pub fn save(dir: &Path, profile: &Profile) -> Result<PathBuf, ProfileError> {
    let path = path(dir, &profile.name);
    refuse_symlink(&path)?;
    let text = layout_json::write(&profile.layout, SAVED_KEY, profile.saved_unix_ms.into())
        .map_err(|reason| ProfileError::Invalid {
            path: path.clone(),
            reason,
        })?;
    fs::create_dir_all(dir).map_err(ProfileError::io(dir))?;
    write_atomic(&path, &text).map_err(ProfileError::io(&path))?;
    Ok(path)
}

/// Removes the profile `name` from the store at `dir`. Returns whether there
/// was one.
///
/// # Errors
///
/// When its file cannot be removed.
pub fn remove(dir: &Path, name: &ProfileName) -> Result<bool, ProfileError> {
    let path = path(dir, name);
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProfileError::io(&path)(error)),
    }
}

/// `profiles` as a JSON array of their names, outputs and save times, for
/// `dsplr profile list --json`.
///
/// # Errors
///
/// When serde_json refuses a value, which for these types it does not.
pub fn summaries_json(profiles: &[Profile]) -> Result<String, serde_json::Error> {
    let summaries: Vec<Value> = profiles
        .iter()
        .map(|profile| {
            json!({
                "name": profile.name.as_str(),
                "outputs": profile.outputs().collect::<Vec<_>>(),
                SAVED_KEY: profile.saved_unix_ms,
            })
        })
        .collect();
    serde_json::to_string_pretty(&summaries)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dsplr_model::{Layout, Mode, Output, OutputEntry, OutputId, OutputState};

    use super::{
        Fit, NameError, Profile, ProfileError, ProfileName, list, load, path, pick, remove, save,
        summaries_json,
    };
    use crate::managed::FileError;

    fn name(text: &str) -> ProfileName {
        text.parse().unwrap()
    }

    /// A connected output as a backend reports it: with a serial it is
    /// known by its EDID, without one by its connector.
    fn live_entry(connector: &str, serial: &str) -> OutputEntry {
        OutputEntry {
            id: OutputId::new("AOC", "Q27G4", serial, connector),
            output: Some(Output {
                connector: connector.into(),
                description: format!("AOC Q27G4 {serial}").trim().to_owned(),
                physical_size_mm: None,
                make: "AOC".into(),
                model: "Q27G4".into(),
                serial: serial.into(),
                modes: vec![Mode::new(2560, 1440, 120_000)],
            }),
            state: OutputState {
                mode: Some(Mode::new(2560, 1440, 120_000)),
                ..OutputState::default()
            },
        }
    }

    fn live(entries: &[(&str, &str)]) -> Layout {
        Layout {
            entries: entries
                .iter()
                .map(|&(connector, serial)| live_entry(connector, serial))
                .collect(),
            ..Default::default()
        }
    }

    fn profile(text: &str, entries: &[(&str, &str)], saved_unix_ms: u64) -> Profile {
        Profile {
            name: name(text),
            layout: live(entries),
            saved_unix_ms,
        }
    }

    #[test]
    fn a_name_is_a_plain_file_name() {
        for good in [
            "desk",
            "laptop only",
            "Office 2",
            "_x",
            "a.b",
            "v1.2-final_3",
        ] {
            assert_eq!(name(good).as_str(), good);
        }
        let refused = |text: &str| text.parse::<ProfileName>().unwrap_err();
        assert_eq!(refused(""), NameError::Empty);
        assert_eq!(refused("a/b"), NameError::Character('/'));
        assert_eq!(refused("a\\b"), NameError::Character('\\'));
        assert_eq!(refused("a\0b"), NameError::Character('\0'));
        assert_eq!(refused("tab\there"), NameError::Character('\t'));
        assert_eq!(refused(".hidden"), NameError::Start('.'));
        assert_eq!(refused(".."), NameError::Start('.'));
        assert_eq!(refused("-rf"), NameError::Start('-'));
        assert_eq!(refused(" desk"), NameError::Start(' '));
        assert_eq!(refused("desk "), NameError::End(' '));
        assert_eq!(refused("desk."), NameError::End('.'));
        assert_eq!(refused("Büro"), NameError::Character('ü'));
        assert_eq!(refused("ｄesk"), NameError::Character('ｄ'));
        assert_eq!(refused("٣"), NameError::Character('٣'));
        assert_eq!(refused("a..b"), NameError::DotDot);
        assert_eq!(refused(&"a".repeat(201)), NameError::TooLong);
        assert_eq!(name(&"a".repeat(200)).as_str().len(), 200);
    }

    #[test]
    fn a_saved_profile_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("dsplr").join("profiles");
        let desk = profile(
            "desk",
            &[("DP-1", "SER1"), ("eDP-1", "")],
            1_700_000_000_123,
        );
        let written = save(&store, &desk).unwrap();
        assert_eq!(written, store.join("desk.json"));
        assert_eq!(path(&store, &desk.name), written);
        let mut expected = desk.clone();
        for entry in &mut expected.layout.entries {
            // Mode handles are the backend's, and are not kept.
            entry.state.mode.as_mut().unwrap().handle = None;
        }
        assert_eq!(load(&store, &desk.name).unwrap(), Some(expected));
        // Saved again, it is replaced, and nothing else is left beside it.
        let later = Profile {
            saved_unix_ms: 1_800_000_000_000,
            ..desk.clone()
        };
        save(&store, &later).unwrap();
        assert_eq!(
            load(&store, &desk.name).unwrap().unwrap().saved_unix_ms,
            later.saved_unix_ms
        );
        let names: Vec<_> = fs::read_dir(&store)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["desk.json"]);
    }

    #[test]
    fn a_profile_file_is_a_layout_file() {
        let dir = tempfile::tempdir().unwrap();
        let desk = profile("desk", &[("DP-1", "SER1")], 5);
        let text = fs::read_to_string(save(dir.path(), &desk).unwrap()).unwrap();
        assert!(text.contains("\"saved_unix_ms\": 5"), "{text}");
        assert_eq!(
            Layout::from_json(&text).unwrap().entries.len(),
            1,
            "dsplr apply reads it"
        );
    }

    /// A layout without the save time, as `dsplr list --json` writes it,
    /// is a profile saved when its file was last changed.
    #[test]
    fn a_layout_without_a_save_time_takes_the_file_s() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("listed.json");
        fs::write(&file, live(&[("DP-1", "")]).to_json().unwrap()).unwrap();
        let modified = super::unix_ms(fs::metadata(&file).unwrap().modified().unwrap());
        let read = load(dir.path(), &name("listed")).unwrap().unwrap();
        assert_eq!(read.saved_unix_ms, modified);
        assert!(modified > 0);
        assert_eq!(read.layout.entries.len(), 1);
    }

    #[test]
    fn a_file_that_is_not_a_profile_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for text in [
            "not json",
            r#"{"version": 0, "entries": [], "saved_unix_ms": "yesterday"}"#,
            r#"{"version": 0, "entries": [], "saved_unix_ms": -1}"#,
            r#"{"version": 9, "entries": []}"#,
        ] {
            fs::write(dir.path().join("bad.json"), text).unwrap();
            assert!(
                matches!(
                    load(dir.path(), &name("bad")),
                    Err(ProfileError::Invalid { .. })
                ),
                "{text}"
            );
        }
        assert!(load(dir.path(), &name("none")).unwrap().is_none());
    }

    #[test]
    fn the_list_is_every_profile_by_file_name_with_the_broken_ones_as_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("none")).unwrap().is_empty());
        save(dir.path(), &profile("work", &[("DP-1", "")], 2)).unwrap();
        save(dir.path(), &profile("desk", &[("DP-1", "")], 1)).unwrap();
        fs::write(dir.path().join("broken.json"), "{").unwrap();
        // Not the store's: another extension, a save's leftover temporary.
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        fs::write(dir.path().join(".desk.json.dsplr-1.tmp"), "{").unwrap();
        // In the store, but not a name a profile can have: said, not read.
        fs::write(dir.path().join(".hidden.json"), "{").unwrap();
        let listed = list(dir.path()).unwrap();
        assert_eq!(listed.len(), 4, "{listed:?}");
        assert!(
            matches!(&listed[0], Err(ProfileError::Name { path, reason }) if path.ends_with(".hidden.json") && reason.contains("'.'")),
            "{listed:?}"
        );
        assert!(
            matches!(&listed[1], Err(ProfileError::Invalid { path, .. }) if path.ends_with("broken.json"))
        );
        let names: Vec<_> = listed[2..]
            .iter()
            .map(|profile| profile.as_ref().unwrap().name.as_str())
            .collect();
        assert_eq!(names, ["desk", "work"]);
    }

    #[test]
    fn removing_a_profile_says_whether_there_was_one() {
        let dir = tempfile::tempdir().unwrap();
        let desk = profile("desk", &[("DP-1", "")], 1);
        save(dir.path(), &desk).unwrap();
        assert!(remove(dir.path(), &desk.name).unwrap());
        assert!(load(dir.path(), &desk.name).unwrap().is_none());
        assert!(!remove(dir.path(), &desk.name).unwrap());
    }

    #[test]
    fn a_link_in_the_store_is_read_but_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.json");
        let desk = profile("desk", &[("DP-1", "")], 1);
        fs::write(&source, desk.layout.to_json().unwrap()).unwrap();
        std::os::unix::fs::symlink(&source, path(dir.path(), &desk.name)).unwrap();
        assert!(load(dir.path(), &desk.name).unwrap().is_some());
        assert!(matches!(
            save(dir.path(), &desk),
            Err(ProfileError::File(FileError::Symlink { .. }))
        ));
    }

    #[test]
    fn an_exact_fit_is_every_connected_output_and_no_other() {
        let now = live(&[("DP-1", "SER1"), ("eDP-1", "")]);
        let fit = |entries: &[(&str, &str)]| profile("p", entries, 0).fit(&now);
        assert_eq!(fit(&[("DP-1", "SER1"), ("eDP-1", "")]), Some(Fit::Exact));
        // A description naming both outputs takes the one the other
        // identity leaves it, though the first it names is the other's.
        let described = Profile {
            layout: Layout {
                entries: vec![
                    OutputEntry {
                        id: OutputId::Description("AOC".into()),
                        output: None,
                        state: OutputState::default(),
                    },
                    live_entry("DP-1", "SER1"),
                ],
                ..Default::default()
            },
            ..profile("p", &[], 0)
        };
        assert_eq!(described.fit(&now), Some(Fit::Exact));
        // A monitor with a serial matches on another port; one without
        // matches by its connector only.
        assert_eq!(
            fit(&[("HDMI-A-1", "SER1"), ("eDP-1", "")]),
            Some(Fit::Exact)
        );
        assert_eq!(fit(&[("DP-1", "SER1")]), Some(Fit::Subset));
        assert_eq!(fit(&[("eDP-1", "")]), Some(Fit::Subset));
        assert_eq!(
            fit(&[("DP-1", "SER2"), ("eDP-1", "")]),
            None,
            "not connected"
        );
        assert_eq!(fit(&[("DP-1", "SER1"), ("eDP-2", "")]), None);
        assert_eq!(
            fit(&[("DP-1", "SER1"), ("eDP-1", ""), ("DP-2", "")]),
            None,
            "a superset is not connected in full"
        );
        assert_eq!(
            fit(&[("eDP-1", ""), ("eDP-1", "")]),
            None,
            "one output cannot stand for two"
        );
        assert_eq!(fit(&[]), None, "an empty profile fits nothing");
    }

    #[test]
    fn an_exact_fit_wins_then_the_most_recent() {
        let now = live(&[("DP-1", "SER1"), ("eDP-1", "")]);
        let picked = |profiles: &[Profile]| {
            pick(profiles, &now).map(|(profile, fit)| (profile.name.as_str().to_owned(), fit))
        };
        let exact_old = profile("a-exact", &[("DP-1", "SER1"), ("eDP-1", "")], 10);
        let exact_new = profile("b-exact", &[("eDP-1", ""), ("DP-1", "SER1")], 20);
        let subset_newest = profile("b-subset", &[("eDP-1", "")], 30);
        let elsewhere = profile("elsewhere", &[("DP-9", "")], 40);
        assert_eq!(
            picked(&[
                exact_old.clone(),
                subset_newest.clone(),
                exact_new,
                elsewhere.clone()
            ]),
            Some(("b-exact".to_owned(), Fit::Exact))
        );
        assert_eq!(
            picked(&[exact_old.clone(), subset_newest.clone()]),
            Some(("a-exact".to_owned(), Fit::Exact)),
            "exact beats a newer subset"
        );

        // With no exact fit, the most recently saved subset.
        let subset_old = profile("a-subset", &[("DP-1", "SER1")], 5);
        assert_eq!(
            picked(&[subset_newest.clone(), subset_old.clone(), elsewhere.clone()]),
            Some(("b-subset".to_owned(), Fit::Subset))
        );
        assert_eq!(
            picked(&[subset_old, elsewhere.clone()]),
            Some(("a-subset".to_owned(), Fit::Subset))
        );

        // A tie goes to the first by name, whatever the order read.
        let a = profile("a", &[("eDP-1", "")], 7);
        let b = profile("b", &[("eDP-1", "")], 7);
        assert_eq!(picked(&[b.clone(), a.clone()]).unwrap().0, "a");
        assert_eq!(picked(&[a, b]).unwrap().0, "a");

        assert_eq!(picked(&[elsewhere]), None);
        assert_eq!(picked(&[]), None);
    }

    #[test]
    fn the_summary_names_each_profile_s_outputs_and_save_time() {
        let summary: serde_json::Value = serde_json::from_str(
            &summaries_json(&[profile("desk", &[("DP-1", "SER1"), ("eDP-1", "")], 42)]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            summary,
            serde_json::json!([{
                "name": "desk",
                "outputs": [
                    { "edid": { "make": "AOC", "model": "Q27G4", "serial": "SER1" } },
                    { "connector": "eDP-1" }
                ],
                "saved_unix_ms": 42
            }])
        );
    }
}
