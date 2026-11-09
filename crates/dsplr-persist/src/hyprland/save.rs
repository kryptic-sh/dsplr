//! Saving a layout as the `monitors.lua` beside Hyprland's own config
//! (`docs/plan/05-persistence-and-profiles.md`, "Rules for files dsplr
//! writes" and "Hyprland").

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{fs, io};

use dsplr_model::{Layout, OutputEntry};

use super::{
    MonitorNaming, MonitorsLua, WriteError, monitor_calls, output_name, read_monitors_lua,
};
use crate::managed::{Comment, Conflict, Ownership, Saved, ownership, with_header, write_atomic};

/// The file dsplr writes, beside the main config.
const MONITORS_FILE: &str = "monitors.lua";

/// How the saved rules name their monitors.
const NAMING: MonitorNaming = MonitorNaming::Description;

/// The module name `monitors.lua` is required by: Hyprland puts the main
/// config's directory on Lua's `package.path`
/// (`CConfigManager::init` in `src/config/lua/ConfigManager.cpp`, v0.57.1).
const MODULE: &str = "monitors";

/// The line the main config needs for Hyprland to read `monitors.lua`.
pub const REQUIRE_LINE: &str = r#"require("monitors")"#;

/// Why a layout could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(
        "HYPRLAND_CONFIG is {}, not an absolute path, so the config Hyprland reads is not known",
        .0.display()
    )]
    RelativeConfig(PathBuf),
    #[error("no Hyprland config found; looked for {}", paths(.looked))]
    NoConfig { looked: Vec<PathBuf> },
    #[error(
        "no Hyprland config found: neither XDG_CONFIG_HOME nor HOME is set to an absolute path, so there is no config directory to look in"
    )]
    NoConfigDir,
    /// `monitors.lua` is a symbolic link. Saving would replace the link
    /// with a file, so the change would never reach where the link points
    /// (a dotfiles checkout, say), and the next deploy would undo it.
    #[error(
        "{} is a link to {}; dsplr does not replace a link with a file, so change the file at its source instead",
        link.display(),
        target.display()
    )]
    Symlink { link: PathBuf, target: PathBuf },
    /// Hyprland reads a hyprlang config here; dsplr writes only the Lua
    /// config's `monitors.lua` (Hyprland 0.57 reads no hyprlang config).
    #[error(
        "{} is a hyprlang config, and dsplr writes only a Lua config's monitors.lua",
        .0.display()
    )]
    Hyprlang(PathBuf),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("the layout cannot be put into monitor rules: {0}")]
    Write(#[from] WriteError),
    #[error(transparent)]
    Conflict(Box<Conflict>),
}

fn paths(paths: &[PathBuf]) -> String {
    let names: Vec<_> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    names.join(", ")
}

/// Saves `layout` as the `monitors.lua` beside the config Hyprland reads,
/// found through `var` as Hyprland finds it.
///
/// What is already in `monitors.lua` decides what happens:
/// - nothing: the file is created;
/// - a file dsplr wrote and nobody changed: it is overwritten;
/// - anything else (edited by hand, someone else's, or holding settings
///   dsplr cannot read): [`SaveError::Conflict`], and nothing is written,
///   unless `approved` is exactly the text the file holds now, which is how
///   the user's answer to that conflict comes back.
///
/// What the old file says is carried over where the layout has nothing to
/// say instead: its catch-all `output = ""` rule, its adaptive sync, bit
/// depth and colour preset for an output the layout leaves them unknown on
/// (Hyprland reports none of them as rules), and its rules for outputs the
/// layout does not have, such as a monitor not connected now. A file dsplr
/// cannot read carries nothing over.
///
/// Outputs are named by description where they have a serial
/// ([`MonitorNaming::Description`]).
///
/// # Errors
///
/// When no config is found, the config is not Lua, a file cannot be read or
/// written, the layout cannot be written, or the file is a conflict.
pub fn save(
    var: impl Fn(&str) -> Option<OsString>,
    layout: &Layout,
    approved: Option<&str>,
) -> Result<Saved, SaveError> {
    let main_config = match find_config(&var)? {
        MainConfig::Lua(path) => path,
        MainConfig::Hyprlang(path) => return Err(SaveError::Hyprlang(path)),
    };
    let io_error = |path: &Path| {
        let path = path.to_owned();
        move |source| SaveError::Io { path, source }
    };
    let main = fs::read_to_string(&main_config).map_err(io_error(&main_config))?;
    let path = main_config.with_file_name(MONITORS_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(&path).map_err(io_error(&path))?;
            return Err(SaveError::Symlink { link: path, target });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(&path)(error)),
    }
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(&path)(error)),
    };
    let (file, kept, unreadable) = match existing.as_deref().map(read_monitors_lua) {
        None => (with_nothing_kept(layout), Vec::new(), None),
        Some(Ok(old)) => {
            let (file, kept) = merged(layout, &old);
            (file, kept, None)
        }
        Some(Err(error)) => (
            with_nothing_kept(layout),
            Vec::new(),
            Some(error.to_string()),
        ),
    };
    let new = with_header(&monitor_calls(&file, NAMING)?, Comment::Lua);
    if let Some(existing) = &existing {
        let ownership = ownership(existing, Comment::Lua);
        let simple = ownership == Ownership::Managed && unreadable.is_none();
        if !simple && approved != Some(existing.as_str()) {
            return Err(SaveError::Conflict(Box::new(Conflict {
                path,
                ownership,
                unreadable,
                existing: existing.clone(),
                new,
            })));
        }
    }
    write_atomic(&path, &new).map_err(io_error(&path))?;
    let require = require_line(&main);
    Ok(Saved {
        path,
        created: existing.is_none(),
        included: require.is_some(),
        overridden: require.is_some_and(|line| monitor_calls_after(&main, line)),
        main_config,
        include_line: REQUIRE_LINE,
        kept: kept.into_iter().map(|entry| entry.id).collect(),
    })
}

fn with_nothing_kept(layout: &Layout) -> MonitorsLua {
    MonitorsLua {
        layout: Layout {
            entries: in_hyprland_order(layout.entries.clone()),
        },
        catch_all: None,
    }
}

/// `entries` in an order Hyprland reads as meant: a rule whose `desc:` is
/// the start of another's goes before it. Hyprland uses the last rule that
/// matches and reads `desc:` as a plain prefix, so `desc:AOC Q27G4 SER1`
/// written after `desc:AOC Q27G4 SER12` would win on the SER12 too. Every
/// other pair keeps its order.
///
/// Each entry goes in just before the first one already placed that its
/// `desc:` is a strict prefix of, else last. An entry placed earlier whose
/// `desc:` is a prefix of this one's is also a prefix of that one's, so it
/// is already before it and stays so.
fn in_hyprland_order(entries: Vec<OutputEntry>) -> Vec<OutputEntry> {
    let mut ordered: Vec<(Option<String>, OutputEntry)> = Vec::with_capacity(entries.len());
    for entry in entries {
        let desc = output_name(&entry, NAMING)
            .and_then(|name| name.strip_prefix("desc:").map(str::to_owned));
        let at = desc
            .as_deref()
            .and_then(|desc| {
                ordered.iter().position(|(other, _)| {
                    other
                        .as_deref()
                        .is_some_and(|other| other != desc && other.starts_with(desc))
                })
            })
            .unwrap_or(ordered.len());
        ordered.insert(at, (desc, entry));
    }
    ordered.into_iter().map(|(_, entry)| entry).collect()
}

/// `layout`, with what `old` says that `layout` does not, and the entries of
/// `old` it kept for outputs `layout` does not have.
///
/// The kept entries come first: Hyprland uses the last rule that matches a
/// monitor, so an old rule naming a port (`DP-1`) must not come after the
/// new rule for the monitor that is next plugged into it. Within each group
/// the rules are [`in_hyprland_order`].
fn merged(layout: &Layout, old: &MonitorsLua) -> (MonitorsLua, Vec<OutputEntry>) {
    let live: Vec<OutputEntry> = layout
        .entries
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            // The rule Hyprland would use: the last one that matches, by
            // Hyprland's own `desc:` prefix rule.
            if let Some(before) = old
                .layout
                .entries
                .iter()
                .rev()
                .find(|e| e.id.matches_as_hyprland(&entry))
            {
                let (state, before) = (&mut entry.state, &before.state);
                state.adaptive_sync = state.adaptive_sync.or(before.adaptive_sync);
                state.bit_depth = state.bit_depth.or(before.bit_depth);
                if state.colour_preset.is_none() {
                    state.colour_preset.clone_from(&before.colour_preset);
                }
            }
            entry
        })
        .collect();
    // Which monitor a rule is for is dsplr's question, so by whole words: a
    // rule for `… SER1` is kept while only `… SER12` is connected. It goes
    // before the rule saved for `… SER12`, which Hyprland then uses for it.
    let kept: Vec<OutputEntry> = old
        .layout
        .entries
        .iter()
        .filter(|before| !layout.entries.iter().any(|live| before.id.matches(live)))
        .cloned()
        .collect();
    let entries = in_hyprland_order(kept.clone())
        .into_iter()
        .chain(in_hyprland_order(live))
        .collect();
    let file = MonitorsLua {
        layout: Layout { entries },
        catch_all: old.catch_all.clone(),
    };
    (file, kept)
}

/// The config Hyprland reads, by the language it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MainConfig {
    Lua(PathBuf),
    Hyprlang(PathBuf),
}

impl MainConfig {
    /// Hyprland v0.56.2 picks the Lua config manager for a path ending in
    /// `.lua` and the hyprlang one for anything else
    /// (`Config::initConfigManager`, `src/config/ConfigManager.cpp`);
    /// v0.57.1 has no hyprlang manager and reads every path as Lua. A path
    /// not ending in `.lua` is taken as hyprlang either way, so dsplr never
    /// writes `monitors.lua` beside a config that may not be Lua.
    fn of(path: PathBuf) -> Self {
        if path.extension().is_some_and(|extension| extension == "lua") {
            Self::Lua(path)
        } else {
            Self::Hyprlang(path)
        }
    }
}

/// The config Hyprland reads, found as Hyprland v0.56.2 finds it
/// (`Jeremy::getMainConfigPath` in `src/config/supplementary/jeremy/`, with
/// hyprutils 0.14.3's `Path::findConfig`): `HYPRLAND_CONFIG` when it is
/// set, else `hypr/hyprland.lua` in `$XDG_CONFIG_HOME`, then in
/// `$HOME/.config`, each only when absolute, and only then
/// `hypr/hyprland.conf` in the same two. v0.57.1 looks for the Lua one only.
///
/// Not followed: `Hyprland --config`, which nothing outside Hyprland can
/// see, and the system configs in `$XDG_CONFIG_DIRS` and `/etc/xdg`, which
/// are not the user's to write beside.
fn find_config(var: &impl Fn(&str) -> Option<OsString>) -> Result<MainConfig, SaveError> {
    if let Some(explicit) = var("HYPRLAND_CONFIG") {
        let path = PathBuf::from(explicit);
        return if path.is_absolute() {
            Ok(MainConfig::of(path))
        } else {
            Err(SaveError::RelativeConfig(path))
        };
    }
    let dirs: Vec<PathBuf> = [
        var("XDG_CONFIG_HOME").map(PathBuf::from),
        var("HOME").map(|home| Path::new(&home).join(".config")),
    ]
    .into_iter()
    .flatten()
    .filter(|dir| dir.is_absolute())
    .collect();
    if dirs.is_empty() {
        return Err(SaveError::NoConfigDir);
    }
    let mut looked = Vec::new();
    for name in ["hyprland.lua", "hyprland.conf"] {
        for dir in &dirs {
            let path = dir.join("hypr").join(name);
            if path.exists() {
                return Ok(MainConfig::of(path));
            }
            looked.push(path);
        }
    }
    Err(SaveError::NoConfig { looked })
}

/// The text of `line` before any `--` comment.
fn code(line: &str) -> &str {
    line.split_once("--").map_or(line, |(code, _)| code)
}

/// The index of the first line of `main` with `require("monitors")` on it,
/// in any of Lua's call forms (`require "monitors"`, single quotes),
/// outside a `--` comment.
///
/// Read line by line, not parsed: a `--[[ … ]]` comment spanning lines is
/// not recognised, nor is a require made another way
/// (`pcall(require, "monitors")`) or from another file the config requires.
fn require_line(main: &str) -> Option<usize> {
    main.lines().position(|line| {
        let code = code(line);
        code.match_indices("require").any(|(at, word)| {
            let before = code[..at].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                return false;
            }
            let rest = code[at + word.len()..].trim_start();
            let rest = rest.strip_prefix('(').map_or(rest, str::trim_start);
            ['"', '\''].into_iter().any(|quote| {
                rest.strip_prefix(quote)
                    .and_then(|rest| rest.strip_prefix(MODULE))
                    .is_some_and(|rest| rest.starts_with(quote))
            })
        })
    })
}

/// Whether a line of `main` after line `require` has `hl.monitor(` on it,
/// outside a `--` comment. Hyprland merges a rule into the one of the same
/// name and keeps it last, so such a call overrides `monitors.lua`'s rule
/// for that output. Read line by line as [`require_line`] is, so a call
/// spelled with a space before `(`, or inside a multi-line comment, is
/// misjudged.
fn monitor_calls_after(main: &str, require: usize) -> bool {
    main.lines()
        .skip(require + 1)
        .any(|line| code(line).contains("hl.monitor("))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use dsplr_model::{AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState};

    use super::{
        MainConfig, REQUIRE_LINE, SaveError, find_config, monitor_calls_after, require_line, save,
    };
    use crate::hyprland::{MonitorNaming, MonitorsLua, monitor_calls, read_monitors_lua};
    use crate::managed::{Comment, Ownership, ownership, with_header};

    /// The owner's machine as a snapshot reads it: the panel off, the AOC
    /// at 2560x1440@120, adaptive sync not known.
    fn live() -> Layout {
        let output = |connector: &str, description: &str| Output {
            connector: connector.into(),
            description: description.into(),
            physical_size_mm: None,
            modes: vec![Mode::new(2560, 1440, 120_000)],
        };
        Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(output("eDP-1", "BOE 0x0AFC")),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
                    output: Some(output("HDMI-A-1", "AOC Q27G4 SERIAL0000001")),
                    state: OutputState {
                        mode: Some(Mode::new(2560, 1440, 120_000)),
                        ..OutputState::default()
                    },
                },
            ],
        }
    }

    /// What saving [`live`] with nothing to carry over writes, after the
    /// header.
    const LIVE_BODY: &str = r#"hl.monitor({
    output = "eDP-1",
    disabled = true,
})

hl.monitor({
    output = "desc:AOC Q27G4 SERIAL0000001",
    disabled = false,
    mode = "2560x1440@120",
    position = "0x0",
    scale = 1,
    transform = 0,
    mirror = "",
})
"#;

    /// A config directory with a `hypr/hyprland.lua` holding `main`, and
    /// the variables that make Hyprland's lookup find it.
    struct Config {
        _dir: tempfile::TempDir,
        home: PathBuf,
        hypr: PathBuf,
    }

    impl Config {
        fn new(main: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let hypr = home.join(".config/hypr");
            fs::create_dir_all(&hypr).unwrap();
            fs::write(hypr.join("hyprland.lua"), main).unwrap();
            Self {
                _dir: dir,
                home,
                hypr,
            }
        }

        fn var(&self) -> impl Fn(&str) -> Option<OsString> + '_ {
            |name| (name == "HOME").then(|| self.home.clone().into_os_string())
        }

        fn monitors(&self) -> PathBuf {
            self.hypr.join("monitors.lua")
        }

        fn read(&self) -> String {
            fs::read_to_string(self.monitors()).unwrap()
        }

        fn save(&self, layout: &Layout, approved: Option<&str>) -> Result<super::Saved, SaveError> {
            save(self.var(), layout, approved)
        }
    }

    fn conflict_of(result: Result<super::Saved, SaveError>) -> crate::managed::Conflict {
        match result {
            Err(SaveError::Conflict(conflict)) => *conflict,
            other => panic!("not a conflict: {other:?}"),
        }
    }

    #[test]
    fn with_no_file_it_is_created_and_the_require_line_is_named() {
        let config = Config::new("-- the user's own config\n");
        let saved = config.save(&live(), None).unwrap();
        assert_eq!(saved.path, config.monitors());
        assert!(saved.created);
        assert!(!saved.included);
        assert_eq!(saved.main_config, config.hypr.join("hyprland.lua"));
        assert_eq!(saved.include_line, r#"require("monitors")"#);
        assert!(saved.kept.is_empty());
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Lua));
        assert_eq!(ownership(&config.read(), Comment::Lua), Ownership::Managed);
    }

    #[test]
    fn a_config_that_requires_it_already_is_seen_to() {
        let config = Config::new("hl.config({})\nrequire(\"monitors\")\n");
        assert!(config.save(&live(), None).unwrap().included);
    }

    #[test]
    fn a_file_dsplr_wrote_is_overwritten_with_the_new_layout() {
        let config = Config::new("");
        config.save(&live(), None).unwrap();
        let mut moved = live();
        moved.entries[1].state.x = 1920;
        let saved = config.save(&moved, None).unwrap();
        assert!(!saved.created);
        let text = config.read();
        assert_eq!(ownership(&text, Comment::Lua), Ownership::Managed);
        assert!(text.contains(r#"    position = "1920x0","#), "{text}");
    }

    #[test]
    fn a_file_edited_by_hand_is_kept_until_the_user_approves_its_text() {
        let config = Config::new("");
        config.save(&live(), None).unwrap();
        let edited = config.read().replace("scale = 1,", "scale = 2,");
        fs::write(config.monitors(), &edited).unwrap();

        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::EditedByHand);
        assert!(conflict.needs_force());
        assert_eq!(conflict.unreadable, None);
        assert_eq!(conflict.existing, edited);
        assert_eq!(conflict.new, with_header(LIVE_BODY, Comment::Lua));
        assert_eq!(config.read(), edited, "nothing was written");

        // Approving other text than the file holds approves nothing.
        let stale = conflict.existing.replace("scale = 2,", "scale = 3,");
        conflict_again(&config, Some(&stale));
        assert_eq!(config.read(), edited);

        config.save(&live(), Some(&conflict.existing)).unwrap();
        assert_eq!(config.read(), conflict.new);
    }

    fn conflict_again(config: &Config, approved: Option<&str>) {
        let _ = conflict_of(config.save(&live(), approved));
    }

    /// The owner's nwg-displays file, which names outputs by connector.
    const NWG_DISPLAYS: &str = include_str!("fixtures/nwg-displays-monitors.lua");

    #[test]
    fn someone_else_s_file_is_replaced_only_once_approved() {
        let config = Config::new("");
        fs::write(config.monitors(), NWG_DISPLAYS).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Foreign);
        assert!(!conflict.needs_force(), "a yes is enough");
        assert_eq!(config.read(), NWG_DISPLAYS);

        let saved = config.save(&live(), Some(NWG_DISPLAYS)).unwrap();
        assert!(!saved.created);
        // Both of its outputs are connected, so none of its rules is kept.
        assert!(saved.kept.is_empty());
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Lua));
    }

    #[test]
    fn what_the_live_state_cannot_say_is_carried_over_from_the_old_file() {
        let config = Config::new("");
        let old = r#"
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.monitor({ output = "HDMI-A-1", mode = "1920x1080@60", position = "0x0", scale = 1, vrr = 1, bitdepth = 10, cm = "hdr" })
hl.monitor({ output = "DP-9", mode = "1920x1080@60", position = "2560x0", scale = 1.5 })
"#;
        fs::write(config.monitors(), old).unwrap();
        let saved = config.save(&live(), Some(old)).unwrap();
        assert_eq!(saved.kept, [OutputId::Connector("DP-9".into())]);

        let file = read_monitors_lua(&config.read()).unwrap();
        assert_eq!(
            file.catch_all.as_deref(),
            Some(
                r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })"#
            )
        );
        let names: Vec<String> = file
            .layout
            .entries
            .iter()
            .map(|e| e.id.to_string())
            .collect();
        // Kept rules first, so the live ones win where both match.
        assert_eq!(names, ["DP-9", "eDP-1", "AOC Q27G4 SERIAL0000001"]);
        // The live mode wins; what the snapshot does not know comes from the
        // old rule.
        let aoc = &file.layout.entries[2].state;
        assert_eq!(aoc.mode, Some(Mode::new(2560, 1440, 120_000)));
        assert_eq!(aoc.adaptive_sync, Some(AdaptiveSync::On));
        assert_eq!(aoc.bit_depth, Some(10));
        assert_eq!(aoc.colour_preset.as_deref(), Some("hdr"));
        let dp9 = &file.layout.entries[0].state;
        assert_eq!((dp9.x, dp9.scale), (2560, 1.5));
    }

    #[test]
    fn a_setting_the_layout_has_wins_over_the_old_file() {
        let config = Config::new("");
        let old = r#"hl.monitor({ output = "HDMI-A-1", mode = "1920x1080@60", position = "0x0", scale = 1, vrr = 1 })"#;
        fs::write(config.monitors(), old).unwrap();
        let mut layout = live();
        layout.entries[1].state.adaptive_sync = Some(AdaptiveSync::Off);
        config.save(&layout, Some(old)).unwrap();
        let file = read_monitors_lua(&config.read()).unwrap();
        assert_eq!(
            file.layout.entries[1].state.adaptive_sync,
            Some(AdaptiveSync::Off)
        );
    }

    #[test]
    fn a_file_dsplr_cannot_read_takes_force_and_carries_nothing_over() {
        let config = Config::new("");
        let old = "hl.monitor({ output = \"\", mode = \"preferred\" })\nhl.monitor({ output = \"HDMI-A-1\", mode = \"1920x1080@60\", position = \"0x0\", scale = 1,\n  sdrbrightness = 1.2 })\n";
        fs::write(config.monitors(), old).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Foreign);
        assert!(conflict.needs_force(), "a yes would lose sdrbrightness");
        let reason = conflict.unreadable.as_deref().unwrap();
        assert!(
            reason.contains("line 3") && reason.contains("sdrbrightness"),
            "{reason}"
        );
        assert!(conflict.to_string().contains("sdrbrightness"), "{conflict}");

        // Approved, the whole file is replaced: the catch-all goes too.
        config.save(&live(), Some(old)).unwrap();
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Lua));

        // A file dsplr wrote that was then given such a setting by hand is
        // both edited and unreadable.
        let edited = config.read().replace(
            "    scale = 1,\n",
            "    scale = 1,\n    sdrbrightness = 1.2,\n",
        );
        fs::write(config.monitors(), &edited).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::EditedByHand);
        assert!(conflict.unreadable.is_some());
        assert!(conflict.needs_force());
    }

    #[test]
    fn the_catch_all_of_a_file_dsplr_wrote_is_kept_on_every_save() {
        let config = Config::new("");
        let catch_all =
            r#"hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })"#;
        let first = MonitorsLua {
            layout: live(),
            catch_all: Some(catch_all.into()),
        };
        let text = with_header(
            &monitor_calls(&first, MonitorNaming::Description).unwrap(),
            Comment::Lua,
        );
        fs::write(config.monitors(), &text).unwrap();
        let mut moved = live();
        moved.entries[1].state.y = 200;
        config.save(&moved, None).unwrap();
        let file = read_monitors_lua(&config.read()).unwrap();
        assert_eq!(file.catch_all.as_deref(), Some(catch_all));
        assert_eq!(file.layout.entries[1].state.y, 200);
    }

    #[test]
    fn what_is_saved_reads_back_as_the_layout() {
        let config = Config::new("");
        let mut layout = live();
        layout.entries[1].state.scale = 1.25;
        layout.entries[1].state.x = -2048;
        config.save(&layout, None).unwrap();
        let file = read_monitors_lua(&config.read()).unwrap();
        assert_eq!(file.catch_all, None);
        let entries = &file.layout.entries;
        assert_eq!(entries.len(), 2);
        for (read, saved) in entries.iter().zip(&layout.entries) {
            assert!(read.id.matches(saved), "{} names {}", read.id, saved.id);
            assert_eq!(read.state, saved.state);
        }
    }

    #[test]
    fn a_hyprlang_config_is_not_written_beside() {
        let dir = tempfile::tempdir().unwrap();
        let hypr = dir.path().join(".config/hypr");
        fs::create_dir_all(&hypr).unwrap();
        fs::write(
            hypr.join("hyprland.conf"),
            "monitor = , preferred, auto, 1\n",
        )
        .unwrap();
        let home = dir.path().as_os_str().to_owned();
        let result = save(|name| (name == "HOME").then(|| home.clone()), &live(), None);
        assert!(
            matches!(&result, Err(SaveError::Hyprlang(path)) if *path == hypr.join("hyprland.conf")),
            "{result:?}"
        );
        assert!(!hypr.join("monitors.lua").exists());
        assert!(!hypr.join("monitors.conf").exists());
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a Path)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_os_str().to_owned())
        }
    }

    #[test]
    fn the_config_is_found_where_hyprland_looks_first() {
        let dir = tempfile::tempdir().unwrap();
        let (xdg, home) = (dir.path().join("xdg"), dir.path().join("home"));
        let make = |path: &Path| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        };
        let env = [("XDG_CONFIG_HOME", xdg.as_path()), ("HOME", home.as_path())];
        let found = || find_config(&vars(&env));

        let looked = vec![
            xdg.join("hypr/hyprland.lua"),
            home.join(".config/hypr/hyprland.lua"),
            xdg.join("hypr/hyprland.conf"),
            home.join(".config/hypr/hyprland.conf"),
        ];
        assert!(matches!(found(), Err(SaveError::NoConfig { looked: l }) if l == looked));

        // A Lua config anywhere wins over a hyprlang one anywhere.
        make(&xdg.join("hypr/hyprland.conf"));
        assert_eq!(found().unwrap(), MainConfig::Hyprlang(looked[2].clone()));
        make(&home.join(".config/hypr/hyprland.lua"));
        assert_eq!(found().unwrap(), MainConfig::Lua(looked[1].clone()));
        // XDG_CONFIG_HOME before HOME.
        make(&xdg.join("hypr/hyprland.lua"));
        assert_eq!(found().unwrap(), MainConfig::Lua(looked[0].clone()));

        // A relative directory is not looked in, as Hyprland does not.
        let relative = [
            ("XDG_CONFIG_HOME", Path::new("xdg")),
            ("HOME", home.as_path()),
        ];
        assert_eq!(
            find_config(&vars(&relative)).unwrap(),
            MainConfig::Lua(looked[1].clone())
        );
    }

    #[test]
    fn hyprland_config_names_the_file_by_its_extension() {
        let explicit = |path: &Path| {
            let env = [
                ("HYPRLAND_CONFIG", path),
                ("HOME", Path::new("/nonexistent")),
            ];
            find_config(&vars(&env))
        };
        assert_eq!(
            explicit(Path::new("/etc/mine/desk.lua")).unwrap(),
            MainConfig::Lua("/etc/mine/desk.lua".into())
        );
        assert_eq!(
            explicit(Path::new("/etc/mine/desk.conf")).unwrap(),
            MainConfig::Hyprlang("/etc/mine/desk.conf".into())
        );
        assert!(matches!(
            explicit(Path::new("desk.lua")),
            Err(SaveError::RelativeConfig(path)) if path == Path::new("desk.lua")
        ));
    }

    #[test]
    fn a_require_is_found_in_its_call_forms_and_not_in_comments() {
        for main in [
            "require(\"monitors\")",
            "  require ( 'monitors' )  -- displays",
            "require \"monitors\"",
            "local _ = require('monitors')",
        ] {
            assert!(require_line(main).is_some(), "{main}");
        }
        for main in [
            "",
            "-- require(\"monitors\")",
            "hl.config({}) -- require(\"monitors\")",
            "require(\"monitors2\")",
            "require(\"my-monitors\")",
            "__require(\"monitors\")",
            "require(monitors)",
            "require(\"monitors')",
        ] {
            assert_eq!(require_line(main), None, "{main}");
        }
        assert_eq!(require_line(&format!("x = 1\n{REQUIRE_LINE}\n")), Some(1));
    }

    /// The AOC is on HDMI-A-1 now; an old rule for DP-1 is kept. Were it
    /// written after the AOC's `desc:` rule, it would win over it the day
    /// the AOC is plugged into DP-1, since Hyprland uses the last match.
    #[test]
    fn kept_rules_come_before_the_live_ones() {
        let config = Config::new("");
        let old = r#"hl.monitor({ output = "DP-1", mode = "1920x1080@60", position = "0x0", scale = 2 })"#;
        fs::write(config.monitors(), old).unwrap();
        let saved = config.save(&live(), Some(old)).unwrap();
        assert_eq!(saved.kept, [OutputId::Connector("DP-1".into())]);
        let text = config.read();
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {text}"))
        };
        assert!(
            at(r#"output = "DP-1""#) < at(r#"output = "desc:AOC Q27G4 SERIAL0000001""#),
            "{text}"
        );
    }

    /// An old rule whose description is the live AOC's cut mid-word is for
    /// another monitor (dsplr's whole words), so it is kept; but Hyprland
    /// applies it to the AOC now (its plain prefix), so the AOC's new rule
    /// carries over its `vrr`.
    #[test]
    fn a_rule_for_a_shorter_serial_is_kept_and_carried_over_as_hyprland_reads_it() {
        let config = Config::new("");
        let old = r#"hl.monitor({ output = "desc:AOC Q27G4 SERIAL000000", mode = "1920x1080@60", position = "0x0", scale = 1, vrr = 1 })"#;
        fs::write(config.monitors(), old).unwrap();
        let saved = config.save(&live(), Some(old)).unwrap();
        assert_eq!(
            saved.kept,
            [OutputId::Description("AOC Q27G4 SERIAL000000".into())]
        );
        let text = config.read();
        assert_eq!(text.matches("vrr = 1").count(), 2, "{text}");
    }

    /// Two AOCs whose serials are SER1 and SER12, the SER12 right of it.
    fn ser1_and_ser12() -> Vec<OutputEntry> {
        ["SER1", "SER12"]
            .into_iter()
            .zip([("DP-1", 0), ("DP-2", 2560)])
            .map(|(serial, (connector, x))| OutputEntry {
                id: OutputId::new("AOC", "Q27G4", serial, connector),
                output: Some(Output {
                    connector: connector.into(),
                    description: format!("AOC Q27G4 {serial}"),
                    physical_size_mm: None,
                    modes: vec![Mode::new(2560, 1440, 120_000)],
                }),
                state: OutputState {
                    mode: Some(Mode::new(2560, 1440, 120_000)),
                    x,
                    ..OutputState::default()
                },
            })
            .collect()
    }

    /// Whether `text` has SER1's rule before SER12's, so that Hyprland,
    /// using the last rule whose `desc:` prefixes a monitor's description,
    /// gives each AOC its own.
    fn ser1_first(text: &str) -> bool {
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {text}"))
        };
        at(r#""desc:AOC Q27G4 SER1""#) < at(r#""desc:AOC Q27G4 SER12""#)
    }

    #[test]
    fn a_rule_whose_desc_starts_another_s_is_written_first() {
        for reversed in [false, true] {
            let config = Config::new("");
            let mut entries = ser1_and_ser12();
            if reversed {
                entries.reverse();
            }
            let mut layout = live();
            layout.entries.extend(entries);
            config.save(&layout, None).unwrap();
            let text = config.read();
            assert!(ser1_first(&text), "reversed: {reversed}: {text}");
            // Everything else keeps its order.
            assert!(
                text.find(r#""eDP-1""#) < text.find(r#""desc:AOC Q27G4 SERIAL0000001""#),
                "{text}"
            );
        }
    }

    #[test]
    fn kept_rules_whose_desc_starts_another_s_are_written_first_too() {
        for reversed in [false, true] {
            let config = Config::new("");
            let mut old = Layout {
                entries: ser1_and_ser12(),
            };
            if reversed {
                old.entries.reverse();
            }
            let old = with_header(
                &monitor_calls(
                    &MonitorsLua {
                        layout: old,
                        catch_all: None,
                    },
                    MonitorNaming::Description,
                )
                .unwrap(),
                Comment::Lua,
            );
            fs::write(config.monitors(), &old).unwrap();
            let saved = config.save(&live(), Some(&old)).unwrap();
            assert_eq!(saved.kept.len(), 2);
            let text = config.read();
            assert!(ser1_first(&text), "reversed: {reversed}: {text}");
            assert!(
                text.find(r#""desc:AOC Q27G4 SER12""#)
                    < text.find(r#""desc:AOC Q27G4 SERIAL0000001""#),
                "kept before live: {text}"
            );
        }
    }

    #[test]
    fn a_kept_rule_mirroring_by_description_is_written_back() {
        let config = Config::new("");
        let old = r#"hl.monitor({ output = "DP-9", mode = "1920x1080@60", position = "0x0", scale = 1, mirror = "desc:Dell U2720Q X1" })"#;
        fs::write(config.monitors(), old).unwrap();
        config.save(&live(), Some(old)).unwrap();
        let text = config.read();
        assert!(
            text.contains(r#"    mirror = "desc:Dell U2720Q X1","#),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_monitors_lua_is_refused_before_anything_is_written() {
        let config = Config::new("");
        let source = config.home.join("dotfiles-monitors.lua");
        fs::write(&source, NWG_DISPLAYS).unwrap();
        std::os::unix::fs::symlink(&source, config.monitors()).unwrap();
        // Even approved, a link is not replaced.
        let result = config.save(&live(), Some(NWG_DISPLAYS));
        let Err(SaveError::Symlink { link, target }) = &result else {
            panic!("not refused as a link: {result:?}");
        };
        assert_eq!((link, target), (&config.monitors(), &source));
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("dotfiles-monitors.lua") && message.contains("at its source"),
            "{message}"
        );
        assert!(
            fs::symlink_metadata(config.monitors())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), NWG_DISPLAYS);
    }

    #[test]
    fn monitor_rules_after_the_require_are_seen_to_override_it() {
        let main = "hl.monitor({ output = \"\" })\nrequire(\"monitors\")\n-- hl.monitor({ })\n";
        assert_eq!(require_line(main), Some(1));
        assert!(!monitor_calls_after(main, 1), "before it, or in a comment");
        let main = format!("{main}hl.monitor({{ output = \"DP-1\", disabled = true }})\n");
        assert!(monitor_calls_after(&main, 1));

        let config = Config::new(&main);
        let saved = config.save(&live(), None).unwrap();
        assert!(saved.included && saved.overridden);
        let config = Config::new("require(\"monitors\")\n");
        assert!(!config.save(&live(), None).unwrap().overridden);
    }

    #[test]
    fn with_no_config_directory_the_error_says_so() {
        let result = find_config(&|_: &str| None::<OsString>);
        assert!(matches!(result, Err(SaveError::NoConfigDir)), "{result:?}");
        let relative = [("HOME", Path::new("home"))];
        let message = find_config(&vars(&relative)).unwrap_err().to_string();
        assert!(
            message.contains("neither XDG_CONFIG_HOME nor HOME"),
            "{message}"
        );
    }

    /// Read leniently, each would be saved back changed: `vrr = 3` as 2,
    /// and a disabled output without its other settings.
    #[test]
    fn a_file_dsplr_would_save_back_changed_takes_force() {
        for old in [
            r#"hl.monitor({ output = "HDMI-A-1", mode = "1920x1080@60", position = "0x0", scale = 1, vrr = 3 })"#,
            r#"hl.monitor({ output = "DP-9", disabled = true, mode = "1920x1080@60" })"#,
        ] {
            let config = Config::new("");
            fs::write(config.monitors(), old).unwrap();
            let conflict = conflict_of(config.save(&live(), None));
            assert_eq!(conflict.ownership, Ownership::Foreign, "{old}");
            assert!(conflict.unreadable.is_some(), "{old}");
            assert!(conflict.needs_force(), "{old}");
        }
    }
}
