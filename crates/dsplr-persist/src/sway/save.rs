//! Saving a layout as the `outputs` file beside sway's own config
//! (`docs/plan/05-persistence-and-profiles.md`, "Rules for files dsplr
//! writes" and "wlroots compositors").

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use dsplr_model::{Layout, OutputId};

use super::{
    OutputsBlock, SETTINGS, WriteError, identifier, names, output_blocks, output_name,
    read_outputs, words,
};
use crate::managed::{
    Comment, FileError, NO_CONFIG_DIR, Saved, paths, read_existing, replace, with_header,
};

/// The file dsplr writes, beside the main config.
const OUTPUTS_FILE: &str = "outputs";

/// The line the main config needs for sway to read `outputs`. sway reads a
/// relative `include` path from the including config's directory (sway(5);
/// on headless sway 1.12 that is the directory of the path sway found, even
/// when the config there is a link to another directory). A quoted absolute
/// path with a space in it was not loaded at all, so the relative form is
/// the one that always works.
pub const INCLUDE_LINE: &str = "include outputs";

/// Why a layout could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(
        "no sway config of your own found (sway then reads /etc/sway/config; copy it to one of these to save); looked for {}",
        paths(.looked)
    )]
    NoConfig { looked: Vec<PathBuf> },
    #[error("no sway config found: {NO_CONFIG_DIR}")]
    NoConfigDir,
    /// `outputs` or the main config could not be read, is a link, or could
    /// not be written, or `outputs` is a conflict.
    #[error(transparent)]
    File(#[from] FileError),
    #[error("the layout cannot be put into output commands: {0}")]
    Write(#[from] WriteError),
}

/// Saves `layout` as the `outputs` file beside the config sway reads, found
/// through `var` as sway finds it.
///
/// The layout is checked first: one sway's output config cannot hold (a
/// mirror, say) is [`SaveError::Write`] before any file is read. Then what
/// is already in `outputs` decides, as [`crate::managed::replace`] says:
/// created, overwritten, or a [`FileError::Conflict`].
///
/// A file in the form dsplr writes ([`read_outputs`]) has its commands for
/// outputs the layout does not have, such as a monitor not connected now,
/// carried over as they are; a command naming a connected output by its
/// connector or by its sway identifier is dropped, whichever name the new
/// block uses. The kept commands go first. What headless sway 1.12 showed
/// decides both: on a reload the last setting given for an output wins,
/// whether by name, identifier or `*`; at startup and on hotplug a `*`
/// setting still counts in file order, but an identifier's setting wins
/// over a name's wherever each is. So a kept command for a connected output
/// by its identifier would beat the new block by connector at startup, and
/// the kept commands after the new blocks would beat them on a reload. A
/// file dsplr cannot read carries nothing over.
///
/// # Errors
///
/// When the layout cannot be written, no config is found, a file cannot be
/// read or written, or the file is a conflict.
pub fn save(
    var: impl Fn(&str) -> Option<OsString>,
    layout: &Layout,
    approved: Option<&str>,
) -> Result<Saved, SaveError> {
    let blocks = output_blocks(layout)?;
    let main_config = find_config(&var)?;
    let main = fs::read_to_string(&main_config).map_err(FileError::io(&main_config))?;
    let path = main_config.with_file_name(OUTPUTS_FILE);
    let existing = read_existing(&path)?;
    let (kept, unreadable) = match existing.as_deref().map(read_outputs) {
        None => (Vec::new(), None),
        Some(Ok(old)) => (
            old.into_iter()
                .filter(|block| !layout.entries.iter().any(|live| names(&block.name, live)))
                .collect(),
            None,
        ),
        Some(Err(error)) => (Vec::new(), Some(error.to_string())),
    };
    let body: Vec<&str> = kept
        .iter()
        .map(|block: &OutputsBlock| block.text.as_str())
        .chain([blocks.as_str()])
        .collect();
    let new = with_header(&body.join("\n"), Comment::Hash);
    replace(
        &path,
        existing.as_deref(),
        unreadable,
        new,
        approved,
        Comment::Hash,
    )?;
    let commands = commands(&main);
    let include = commands
        .iter()
        .position(|command| includes(command, &main_config, &path, &var));
    Ok(Saved {
        path,
        created: existing.is_none(),
        included: include.is_some(),
        overridden: include.is_some_and(|at| overridden_after(&commands, at, layout)),
        beaten_at_startup: beaten_at_startup(&commands, layout),
        main_config,
        include_line: INCLUDE_LINE,
        rules: "output commands",
        kept: kept_ids(&kept),
    })
}

/// The outputs `kept` are for, each once, as the user would name them: not
/// the catch-alls `*`, `-` and `--` (every output, the focused one), which
/// are carried over the same way but are no monitor of their own.
fn kept_ids(kept: &[OutputsBlock]) -> Vec<OutputId> {
    let mut ids = Vec::new();
    for block in kept {
        if ["*", "-", "--"].contains(&block.name.as_str()) {
            continue;
        }
        let id = if block.name.contains(char::is_whitespace) {
            OutputId::Description(block.name.clone())
        } else {
            OutputId::Connector(block.name.clone())
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The config sway reads, found as sway(1) (sway 1.12) says sway finds it:
/// `~/.sway/config`, `$XDG_CONFIG_HOME/sway/config`, `~/.i3/config`,
/// `$XDG_CONFIG_HOME/i3/config`, the first that exists, with
/// `XDG_CONFIG_HOME` defaulting to `~/.config`. A relative `HOME` or
/// `XDG_CONFIG_HOME` is not used (the XDG base directory spec ignores a
/// relative one).
///
/// Not followed: `sway --config`, which nothing outside sway can see, and
/// `/etc/sway/config` and `/etc/i3/config`, which are not the user's to
/// write beside.
fn find_config(var: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, SaveError> {
    let home = var("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute());
    let config_home = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let looked: Vec<PathBuf> = [
        home.as_ref().map(|home| home.join(".sway/config")),
        config_home.as_ref().map(|dir| dir.join("sway/config")),
        home.as_ref().map(|home| home.join(".i3/config")),
        config_home.as_ref().map(|dir| dir.join("i3/config")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if looked.is_empty() {
        return Err(SaveError::NoConfigDir);
    }
    looked
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or(SaveError::NoConfig { looked })
}

/// Each command of the main config `main` as sway runs it: lines ending in
/// `\` joined to the next, `#` comment lines left out, and a block's opening
/// words put in front of each line inside it (sway(5)). A line with a quote
/// left open is left out, as one dsplr cannot read.
fn commands(main: &str) -> Vec<Vec<String>> {
    let mut joined = Vec::new();
    let mut pending = String::new();
    for line in main.lines() {
        match line.strip_suffix('\\') {
            Some(start) => pending.push_str(start),
            None => {
                pending.push_str(line);
                joined.push(std::mem::take(&mut pending));
            }
        }
    }
    joined.push(pending);
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut commands = Vec::new();
    // Whether the last line read was an `output` command, now last in
    // `commands`: a `{` on a line of its own opens a block for it.
    let mut after_output = false;
    for line in joined {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(mut words) = words(line) else {
            after_output = false;
            continue;
        };
        let was_after_output = std::mem::take(&mut after_output);
        if words == ["}"] {
            blocks.pop();
        } else if words == ["{"] {
            // Opening nothing else, it still has its `}` to match.
            let opening = if was_after_output {
                commands.pop()
            } else {
                None
            };
            blocks.push(opening.unwrap_or_else(|| blocks.last().cloned().unwrap_or_default()));
        } else if words.last().is_some_and(|last| last == "{") {
            words.pop();
            let mut opening = blocks.last().cloned().unwrap_or_default();
            opening.extend(words);
            blocks.push(opening);
        } else {
            let mut command = blocks.last().cloned().unwrap_or_default();
            command.extend(words);
            after_output = command.first().is_some_and(|first| first == "output");
            commands.push(command);
        }
    }
    commands
}

/// Whether `command` is an `include` of `outputs`, its path read from
/// `main_config`'s directory when relative.
fn includes(
    command: &[String],
    main_config: &Path,
    outputs: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
) -> bool {
    let Some(("include", paths)) = command
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    else {
        return false;
    };
    let dir = main_config.parent().unwrap_or_else(|| Path::new(""));
    paths
        .iter()
        .filter_map(|path| expanded(path, var))
        .any(|path| dir.join(path) == outputs)
}

/// `word` with what sway's `wordexp` would expand in it: a leading `~` and
/// `$NAME` or `${NAME}` variables, `XDG_CONFIG_HOME` defaulting to
/// `~/.config`. `None` for anything else wordexp does that dsplr does not
/// (a glob, `~user`, a command substitution) or a variable that is not set:
/// dsplr cannot tell where that leads.
fn expanded(word: &str, var: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let lookup = |name: &str| {
        var(name).or_else(|| {
            (name == "XDG_CONFIG_HOME")
                .then(|| var("HOME").map(|home| Path::new(&home).join(".config").into()))
                .flatten()
        })
    };
    if word.contains(['*', '?', '[', '`']) || word.contains("$(") {
        return None;
    }
    let mut text = OsString::new();
    let mut rest = word;
    if let Some(after) = rest.strip_prefix('~') {
        if !(after.is_empty() || after.starts_with('/')) {
            return None;
        }
        text.push(lookup("HOME")?);
        rest = after;
    }
    while let Some(at) = rest.find('$') {
        text.push(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, next) = match after.strip_prefix('{') {
            Some(braced) => {
                let end = braced.find('}')?;
                (&braced[..end], &braced[end + 1..])
            }
            None => {
                let end = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        if name.is_empty() {
            return None;
        }
        text.push(lookup(name)?);
        rest = next;
    }
    text.push(rest);
    Some(PathBuf::from(text))
}

/// Whether an `output` command of the main config's `commands` after the
/// include at `include` sets a connected output of `layout`, by connector,
/// identifier or `*`: by the precedence on [`save`], it wins over the saved
/// file's block on a reload.
fn overridden_after(commands: &[Vec<String>], include: usize, layout: &Layout) -> bool {
    commands[include + 1..].iter().any(|command| {
        sets_outputs(command)
            .is_some_and(|name| name == "*" || layout.entries.iter().any(|live| names(name, live)))
    })
}

/// The names under which an `output` command of the main config's
/// `commands`, anywhere in it, sets a connected output of `layout` by its
/// sway identifier while the saved file names that output by connector, each
/// once: by the precedence on [`save`], such a command wins at startup
/// wherever it is.
fn beaten_at_startup(commands: &[Vec<String>], layout: &Layout) -> Vec<String> {
    let mut beaten: Vec<String> = Vec::new();
    for name in commands.iter().filter_map(|command| sets_outputs(command)) {
        let by_identifier = layout.entries.iter().any(|live| {
            identifier(live).as_deref() == Some(name)
                && output_name(layout, live).as_deref() != Some(name)
        });
        if by_identifier && !beaten.iter().any(|seen| seen == name) {
            beaten.push(name.to_owned());
        }
    }
    beaten
}

/// The output name of `command` when it is an `output` command setting
/// something dsplr writes.
fn sets_outputs(command: &[String]) -> Option<&str> {
    let ("output", [name, settings @ ..]) = command.split_first().map(|(f, r)| (f.as_str(), r))?
    else {
        return None;
    };
    settings
        .iter()
        .any(|word| SETTINGS.contains(&word.as_str()))
        .then_some(name.as_str())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use dsplr_model::{Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform};

    use super::{INCLUDE_LINE, SaveError, commands, expanded, find_config, includes, save};
    use crate::managed::{Comment, Conflict, FileError, Ownership, Saved, ownership, with_header};
    use crate::sway::{output_blocks, read_outputs};

    /// The owner's machine as a snapshot reads it: the panel off, the AOC
    /// at 2560x1440@120.
    fn live() -> Layout {
        let output = |connector: &str, description: &str| {
            let mut words = description.split(' ');
            let mut word = || words.next().unwrap_or_default().to_owned();
            Output {
                connector: connector.into(),
                description: description.into(),
                physical_size_mm: None,
                make: word(),
                model: word(),
                serial: word(),
                modes: vec![Mode::new(2560, 1440, 120_000)],
            }
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
    /// header: the golden file.
    const LIVE_BODY: &str = r#"output eDP-1 {
    disable
}

output "AOC Q27G4 SERIAL0000001" {
    enable
    mode 2560x1440@120Hz
    position 0 0
    scale 1
    transform normal
}
"#;

    /// A `sway/config` holding `main` in a config directory, and the
    /// variables that make sway's lookup find it.
    struct Config {
        _dir: tempfile::TempDir,
        home: PathBuf,
        sway: PathBuf,
    }

    impl Config {
        fn new(main: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let sway = home.join(".config/sway");
            fs::create_dir_all(&sway).unwrap();
            fs::write(sway.join("config"), main).unwrap();
            Self {
                _dir: dir,
                home,
                sway,
            }
        }

        fn var(&self) -> impl Fn(&str) -> Option<OsString> + '_ {
            |name| (name == "HOME").then(|| self.home.clone().into_os_string())
        }

        fn outputs(&self) -> PathBuf {
            self.sway.join("outputs")
        }

        fn read(&self) -> String {
            fs::read_to_string(self.outputs()).unwrap()
        }

        fn save(&self, layout: &Layout, approved: Option<&str>) -> Result<Saved, SaveError> {
            save(self.var(), layout, approved)
        }
    }

    fn conflict_of(result: Result<Saved, SaveError>) -> Conflict {
        match result {
            Err(SaveError::File(FileError::Conflict(conflict))) => *conflict,
            other => panic!("not a conflict: {other:?}"),
        }
    }

    #[test]
    fn with_no_file_it_is_created_and_the_include_line_is_named() {
        let config = Config::new("# the user's own config\n");
        let saved = config.save(&live(), None).unwrap();
        assert_eq!(saved.path, config.outputs());
        assert!(saved.created);
        assert!(!saved.included);
        assert!(!saved.overridden);
        assert_eq!(saved.main_config, config.sway.join("config"));
        assert_eq!(saved.include_line, "include outputs");
        assert_eq!(saved.rules, "output commands");
        assert!(saved.kept.is_empty());
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Hash));
        assert!(config.read().starts_with("# Managed by dsplr; "));
        assert_eq!(ownership(&config.read(), Comment::Hash), Ownership::Managed);
    }

    #[test]
    fn a_file_dsplr_wrote_is_overwritten_and_one_edited_waits_for_approval() {
        let config = Config::new("include outputs\n");
        assert!(config.save(&live(), None).unwrap().included);
        let mut moved = live();
        moved.entries[1].state.x = 1920;
        let saved = config.save(&moved, None).unwrap();
        assert!(!saved.created);
        assert!(config.read().contains("    position 1920 0\n"));

        let edited = config.read().replace("scale 1\n", "scale 2\n");
        fs::write(config.outputs(), &edited).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::EditedByHand);
        assert!(conflict.needs_force());
        assert_eq!(conflict.new, with_header(LIVE_BODY, Comment::Hash));
        assert_eq!(config.read(), edited, "nothing was written");
        let stale = edited.replace("scale 2", "scale 3");
        let _ = conflict_of(config.save(&live(), Some(&stale)));
        assert_eq!(config.read(), edited, "other text approves nothing");
        config.save(&live(), Some(&conflict.existing)).unwrap();
        assert_eq!(config.read(), conflict.new);
    }

    #[test]
    fn commands_for_outputs_not_connected_are_carried_over_first() {
        let config = Config::new("");
        let old = "output * scale 2\n\noutput HDMI-A-1 {\n    mode 1920x1080@60Hz\n}\n\noutput DP-9 {\n    enable\n    position 2560 0\n}\n\noutput \"Dell Inc. U2720Q X1\" disable\n";
        fs::write(config.outputs(), old).unwrap();
        // Someone else's file, readable: a yes is enough.
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Foreign);
        assert!(!conflict.needs_force());
        let saved = config.save(&live(), Some(old)).unwrap();
        assert_eq!(
            saved.kept,
            [
                OutputId::Connector("DP-9".into()),
                OutputId::Description("Dell Inc. U2720Q X1".into()),
            ],
            "* is carried but is no monitor of its own"
        );
        let text = config.read();
        let names: Vec<String> = read_outputs(&text)
            .unwrap()
            .into_iter()
            .map(|block| block.name)
            .collect();
        // HDMI-A-1 is the AOC's port, so its old block is replaced.
        assert_eq!(
            names,
            [
                "*",
                "DP-9",
                "Dell Inc. U2720Q X1",
                "eDP-1",
                "AOC Q27G4 SERIAL0000001"
            ]
        );
        assert!(
            text.contains("output DP-9 {\n    enable\n    position 2560 0\n}\n"),
            "kept as it was: {text}"
        );
        assert!(!text.contains("1920x1080"), "{text}");
    }

    /// The live AOC is named by identifier; an old block naming it by that
    /// identifier is for it, and replaced.
    /// The panel has no serial and is saved by connector, but sway also
    /// knows it as `BOE 0x0AFC Unknown`, and at startup a block by that
    /// identifier would win over the saved one.
    #[test]
    fn a_block_naming_a_connected_monitor_without_a_serial_by_identifier_is_dropped() {
        let config = Config::new("");
        let old = "output \"BOE 0x0AFC Unknown\" scale 2\n";
        fs::write(config.outputs(), old).unwrap();
        let saved = config.save(&live(), Some(old)).unwrap();
        assert!(saved.kept.is_empty(), "{:?}", saved.kept);
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Hash));
    }

    #[test]
    fn a_block_naming_a_connected_monitor_by_identifier_is_replaced() {
        let config = Config::new("");
        let old = with_header(
            "output \"AOC Q27G4 SERIAL0000001\" {\n    enable\n    mode 1920x1080\n    position 0 0\n    scale 2\n    transform normal\n}\n",
            Comment::Hash,
        );
        fs::write(config.outputs(), &old).unwrap();
        let saved = config.save(&live(), None).unwrap();
        assert!(saved.kept.is_empty());
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Hash));
    }

    #[test]
    fn a_file_dsplr_cannot_read_takes_force_and_carries_nothing_over() {
        let config = Config::new("");
        let old = "output DP-9 {\n    position 0 0\n}\noutput HDMI-A-1 bg ~/wall.png fill\n";
        fs::write(config.outputs(), old).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Foreign);
        assert!(conflict.needs_force(), "a yes would lose the wallpaper");
        let reason = conflict.unreadable.as_deref().unwrap();
        assert!(
            reason.contains("line 4") && reason.contains("`bg`"),
            "{reason}"
        );
        config.save(&live(), Some(old)).unwrap();
        assert_eq!(config.read(), with_header(LIVE_BODY, Comment::Hash));
    }

    #[test]
    fn a_mirror_is_refused_before_any_file_is_touched() {
        let config = Config::new("");
        let mut layout = live();
        layout.entries[1].state.mirror_of = Some(OutputId::Connector("eDP-1".into()));
        let result = config.save(&layout, None);
        assert!(
            matches!(
                &result,
                Err(SaveError::Write(crate::sway::WriteError::Unsupported {
                    what: "a mirror",
                    ..
                }))
            ),
            "{result:?}"
        );
        assert!(!config.outputs().exists());
    }

    #[test]
    fn what_is_saved_is_the_writer_s_output_under_the_header() {
        let config = Config::new("");
        let mut layout = live();
        layout.entries[1].state.transform = Transform::Flipped90;
        layout.entries[1].state.scale = 1.5;
        config.save(&layout, None).unwrap();
        assert_eq!(
            config.read(),
            with_header(&output_blocks(&layout).unwrap(), Comment::Hash)
        );
        assert!(config.read().contains("    transform flipped-270\n"));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_outputs_file_is_refused() {
        let config = Config::new("");
        let source = config.home.join("dotfiles-outputs");
        fs::write(&source, "output DP-9 disable\n").unwrap();
        std::os::unix::fs::symlink(&source, config.outputs()).unwrap();
        let result = config.save(&live(), Some("output DP-9 disable\n"));
        assert!(
            matches!(&result, Err(SaveError::File(FileError::Symlink { target, .. })) if *target == source),
            "{result:?}"
        );
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "output DP-9 disable\n"
        );
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
    fn the_config_is_found_where_sway_looks_first() {
        let dir = tempfile::tempdir().unwrap();
        let (xdg, home) = (dir.path().join("xdg"), dir.path().join("home"));
        let make = |path: &Path| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        };
        let env = [("XDG_CONFIG_HOME", xdg.as_path()), ("HOME", home.as_path())];
        let found = || find_config(&vars(&env));
        let looked = vec![
            home.join(".sway/config"),
            xdg.join("sway/config"),
            home.join(".i3/config"),
            xdg.join("i3/config"),
        ];
        assert!(matches!(found(), Err(SaveError::NoConfig { looked: l }) if l == looked));
        for at in (0..looked.len()).rev() {
            make(&looked[at]);
            assert_eq!(found().unwrap(), looked[at]);
        }
        // Without XDG_CONFIG_HOME, or with a relative one, ~/.config.
        let bare = dir.path().join("bare");
        let env = [
            ("XDG_CONFIG_HOME", Path::new("relative")),
            ("HOME", bare.as_path()),
        ];
        for env in [&env[1..], &env[..]] {
            assert!(matches!(
                find_config(&vars(env)),
                Err(SaveError::NoConfig { looked }) if looked[1] == bare.join(".config/sway/config")
            ));
        }
        assert!(matches!(
            find_config(&|_: &str| None::<OsString>),
            Err(SaveError::NoConfigDir)
        ));
        let message = find_config(&vars(&env[1..])).unwrap_err().to_string();
        assert!(message.contains("/etc/sway/config"), "{message}");
    }

    #[test]
    fn an_include_is_found_in_the_forms_sway_expands() {
        let main_config = Path::new("/h/.config/sway/config");
        let outputs = Path::new("/h/.config/sway/outputs");
        let env = [("HOME", Path::new("/h"))];
        let includes_it = |main: &str| {
            commands(main)
                .iter()
                .any(|command| includes(command, main_config, outputs, &vars(&env)))
        };
        for main in [
            "include outputs",
            "include ./outputs",
            "  include   'outputs'  ",
            "include ~/.config/sway/outputs",
            "include $HOME/.config/sway/outputs",
            "include ${XDG_CONFIG_HOME}/sway/outputs",
            "include /h/.config/sway/outputs",
            "include other outputs",
            "include \\\n  outputs",
        ] {
            assert!(includes_it(main), "{main}");
        }
        for main in [
            "",
            "# include outputs",
            "include outputs2",
            "include sway/outputs",
            "include /etc/sway/config.d/*",
            "include ~/.config/sway/*",
            "include $NOT_SET/outputs",
            "exec include outputs",
        ] {
            assert!(!includes_it(main), "{main}");
        }
        assert_eq!(expanded("~user/x", &vars(&env)), None);
    }

    /// By the precedence on [`save`]: the panel (no serial) is saved by
    /// connector, as `eDP-1`, though sway also knows it as
    /// `BOE 0x0AFC Unknown`; the AOC by identifier.
    #[test]
    fn output_commands_after_the_include_are_seen_to_win_on_a_reload() {
        for (main, overridden) in [
            ("include outputs\n", false),
            ("output * scale 2\ninclude outputs\n", false),
            ("include outputs\noutput * bg ~/w.png fill\n", false),
            ("include outputs\n# output eDP-1 pos 0 0\n", false),
            ("include outputs\noutput eDP-1 pos 0 0\n", true),
            ("include outputs\noutput HDMI-A-1 pos 0 0\n", true),
            ("include outputs\noutput * scale 2\n", true),
            (
                "include outputs\noutput \"BOE 0x0AFC Unknown\" scale 2\n",
                true,
            ),
            (
                "include outputs\noutput eDP-1 bg ~/w.png fill pos 0 0\n",
                true,
            ),
            (
                "include outputs\noutput HDMI-A-1 {\n    bg ~/w.png fill\n    scale 2\n}\n",
                true,
            ),
            (
                "include outputs\noutput HDMI-A-1\n{\n    scale 2\n}\n",
                true,
            ),
            (
                "include outputs\noutput HDMI-A-1 {\n    bg ~/w.png fill\n}\n",
                false,
            ),
            ("include outputs\noutput DP-9 pos 0 0\n", false),
            (
                "include outputs\nbindsym $mod+p output eDP-1 disable\n",
                false,
            ),
            // Before it, they lose on a reload.
            ("output eDP-1 scale 2\ninclude outputs\n", false),
            ("output HDMI-A-1 scale 2\ninclude outputs\n", false),
            (
                "output \"BOE 0x0AFC Unknown\" scale 2\ninclude outputs\n",
                false,
            ),
        ] {
            let config = Config::new(main);
            let saved = config.save(&live(), None).unwrap();
            assert!(saved.included, "{main}");
            assert_eq!(saved.overridden, overridden, "{main}");
        }
    }

    /// An identifier for an output the file names by connector wins at
    /// startup wherever it is, and whether or not the file is included.
    #[test]
    fn identifier_commands_that_win_at_startup_are_named() {
        let boe = vec!["BOE 0x0AFC Unknown".to_owned()];
        for (main, beaten) in [
            ("include outputs\n", vec![]),
            (
                "output \"BOE 0x0AFC Unknown\" scale 2\ninclude outputs\n",
                boe.clone(),
            ),
            (
                "include outputs\noutput \"BOE 0x0AFC Unknown\" scale 2\n",
                boe.clone(),
            ),
            // Not included at all: still named.
            ("output \"BOE 0x0AFC Unknown\" scale 2\n", boe.clone()),
            (
                "output \"BOE 0x0AFC Unknown\" {\n    scale 2\n}\noutput \"BOE 0x0AFC Unknown\" pos 0 0\n",
                boe,
            ),
            // The AOC is saved by its identifier, so its own wins in order.
            (
                "output \"AOC Q27G4 SERIAL0000001\" scale 2\ninclude outputs\n",
                vec![],
            ),
            ("output \"BOE 0x0AFC Unknown\" bg ~/w.png fill\n", vec![]),
            ("output \"Other Model Unknown\" scale 2\n", vec![]),
        ] {
            let config = Config::new(main);
            let saved = config.save(&live(), None).unwrap();
            assert_eq!(saved.beaten_at_startup, beaten, "{main}");
        }
    }

    #[test]
    fn a_block_s_opening_words_go_in_front_of_each_line() {
        assert_eq!(
            commands("output DP-1 {\n  scale 2\n  pos 0 0\n}\nexec foo"),
            [
                vec!["output", "DP-1", "scale", "2"],
                vec!["output", "DP-1", "pos", "0", "0"],
                vec!["exec", "foo"],
            ]
        );
        // sway takes a `{` on a line of its own too, after an `output`
        // command, comments and blank lines between them.
        assert_eq!(
            commands("exec foo\noutput DP-1\n\n# here\n{\n  scale 2\n}\n"),
            [vec!["exec", "foo"], vec!["output", "DP-1", "scale", "2"]]
        );
        // Not for another command, a closed block, or a line further back.
        for main in [
            "exec foo\n{\n  scale 2\n}\n",
            "output DP-1 {\n  enable\n}\n{\n  scale 2\n}\n",
            "output DP-1 pos 0 0\nexec foo\n{\n  scale 2\n}\n",
        ] {
            let read = commands(main);
            assert!(
                read.iter().any(|command| *command == ["scale", "2"]),
                "{main}: {read:?}"
            );
        }
        assert_eq!(INCLUDE_LINE, "include outputs");
    }
}
