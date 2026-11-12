//! Saving a layout as the `outputs.kdl` beside niri's own config
//! (`docs/plan/05-persistence-and-profiles.md`, "Rules for files dsplr
//! writes" and "niri").

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use dsplr_model::{AdaptiveSync, Layout};

use super::{
    OutputsNode, SETTINGS, Token, WriteError, names, on_demand, output_nodes, read_outputs, tokens,
};
use crate::identity::kept_ids;
use crate::managed::{
    Comment, FileError, NO_CONFIG_DIR, Saved, config_home, home, paths, read_existing, replace,
    with_header,
};

/// The file dsplr writes, beside the main config.
const OUTPUTS_FILE: &str = "outputs.kdl";

/// The line the main config needs for niri to read `outputs.kdl`. niri
/// reads a relative `include` path from the including file's directory
/// (checked with `niri validate` 26.04), so the relative form works
/// wherever the config is.
pub const INCLUDE_LINE: &str = r#"include "outputs.kdl""#;

/// Why a layout could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(
        "NIRI_CONFIG is {}, not an absolute path, so the config niri reads is not known",
        .0.display()
    )]
    RelativeConfig(PathBuf),
    #[error(
        "no niri config of your own found (niri then reads /etc/niri/config.kdl; copy it there to save); looked for {}",
        paths(.looked)
    )]
    NoConfig { looked: Vec<PathBuf> },
    #[error("no niri config found: {NO_CONFIG_DIR}")]
    NoConfigDir,
    /// `outputs.kdl` or the main config could not be read, is a link, or
    /// could not be written, or `outputs.kdl` is a conflict.
    #[error(transparent)]
    File(#[from] FileError),
    #[error("the layout cannot be put into output nodes: {0}")]
    Write(#[from] WriteError),
}

/// Saves `layout` as the `outputs.kdl` beside the config niri reads, found
/// through `var` as niri finds it. niri reloads the file as soon as it
/// changes, so when the main config includes it this applies it too: the
/// layout is the one on screen, so nothing moves, and runtime changes to
/// its outputs become the config's.
///
/// The layout is checked first: one niri's output config cannot hold (a
/// mirror, say) is [`SaveError::Write`] before any file is read. Then what
/// is already in `outputs.kdl` decides, as [`crate::managed::replace`]
/// says: created, overwritten, or a [`FileError::Conflict`].
///
/// A file in the form dsplr writes ([`read_outputs`]) has its nodes for
/// outputs the layout does not have, such as a monitor not connected now,
/// carried over as they are, first; a node naming a connected output by its
/// connector or its identifier, in any case, is dropped. niri uses the first
/// node naming an output, and no kept node names a connected one, so the
/// kept nodes go first: a monitor saved by its identifier and later plugged
/// into a port the file names keeps its own settings. A file dsplr cannot
/// read carries nothing over. A connected output whose old node set
/// `variable-refresh-rate on-demand=true`, and whose adaptive sync the
/// layout has on, keeps it on demand: wlr-output-management has only on and
/// off, so the layout cannot say it.
///
/// The main config's own `output` nodes for a connected output after the
/// include (all of them, when it has no include yet, since the include goes
/// before them) are hidden whole by the saved node once the file is
/// written. When one sets anything dsplr does not write
/// (`focus-at-startup`, `backdrop-color`, on-demand adaptive sync), saving
/// is a [`FileError::Conflict`] listing them ([`Conflict::shadowed`]) even
/// with no file there yet, so they never stop applying unasked.
///
/// # Errors
///
/// When the layout cannot be written, no config is found, a file cannot be
/// read or written, or the save is a conflict.
///
/// [`Conflict::shadowed`]: crate::managed::Conflict::shadowed
pub fn save(
    var: impl Fn(&str) -> Option<OsString>,
    layout: &Layout,
    approved: Option<&str>,
) -> Result<Saved, SaveError> {
    // Checked before any file is read: a layout niri cannot hold is refused
    // first.
    output_nodes(layout)?;
    let main_config = find_config(&var)?;
    let main = fs::read_to_string(&main_config).map_err(FileError::io(&main_config))?;
    let path = main_config.with_file_name(OUTPUTS_FILE);
    let existing = read_existing(&path)?;
    let (old, unreadable) = match existing.as_deref().map(read_outputs) {
        None => (Vec::new(), None),
        Some(Ok(old)) => (old, None),
        Some(Err(error)) => (Vec::new(), Some(error.to_string())),
    };
    let (live, kept): (Vec<OutputsNode>, Vec<OutputsNode>) = old
        .into_iter()
        .partition(|node| layout.entries.iter().any(|entry| names(&node.name, entry)));
    let nodes = output_nodes(&with_on_demand(layout, &live))?;
    let body: Vec<&str> = kept
        .iter()
        .map(|node| node.text.as_str())
        .chain([nodes.as_str()])
        .collect();
    let new = with_header(&body.join("\n"), Comment::Slashes);
    let top = top_level(&main);
    let include = top
        .iter()
        .position(|node| includes(node, &main_config, &path, &var));
    let hidden = shadowed(&top[include.map_or(0, |at| at + 1)..], layout, &main_config);
    replace(
        &path,
        existing.as_deref(),
        unreadable,
        new,
        approved,
        Comment::Slashes,
        hidden,
    )?;
    Ok(Saved {
        path,
        created: existing.is_none(),
        included: include.is_some(),
        overridden: include.is_some_and(|at| overridden_before(&top[..at], layout)),
        beaten_at_startup: Vec::new(),
        main_config,
        include_line: INCLUDE_LINE,
        rules: "output nodes",
        first_wins: true,
        kept: kept_ids(kept.iter().map(|node| node.name.as_str())),
        // niri has no rule for every output: a node names one whole.
        kept_catch_all: None,
    })
}

/// The config niri reads, found as niri 26.04 finds it (`src/main.rs` and
/// `niri-config`'s `ConfigPath`): `NIRI_CONFIG` when it is set and not
/// empty, else `niri/config.kdl` in the user's config directory
/// ([`config_home`]).
///
/// Not followed: `niri --config`, which nothing outside niri can see, and
/// `/etc/niri/config.kdl`, which niri reads when the user has none and is
/// not the user's to write beside. With neither, niri creates the user's
/// config when it starts, so a running niri normally has one.
fn find_config(var: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, SaveError> {
    if let Some(explicit) = var("NIRI_CONFIG").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(explicit);
        if !path.is_absolute() {
            return Err(SaveError::RelativeConfig(path));
        }
        return if path.exists() {
            Ok(path)
        } else {
            Err(SaveError::NoConfig { looked: vec![path] })
        };
    }
    let path = config_home(var)
        .ok_or(SaveError::NoConfigDir)?
        .join("niri/config.kdl");
    if path.exists() {
        Ok(path)
    } else {
        Err(SaveError::NoConfig { looked: vec![path] })
    }
}

/// `layout`, with adaptive sync that is on made automatic (written
/// `on-demand=true`) for each output whose node in the old file, among
/// `old`, set it on demand.
fn with_on_demand(layout: &Layout, old: &[OutputsNode]) -> Layout {
    let mut layout = layout.clone();
    for entry in &mut layout.entries {
        let was_on_demand = old
            .iter()
            .any(|node| node.on_demand_vrr && names(&node.name, entry));
        if was_on_demand && entry.state.adaptive_sync == Some(AdaptiveSync::On) {
            entry.state.adaptive_sync = Some(AdaptiveSync::Automatic);
        }
    }
    layout
}

/// What the main config's `output` nodes among `nodes` set for a connected
/// output of `layout` that dsplr does not write, one entry per node that
/// sets any: the saved node for that output hides them whole.
fn shadowed(nodes: &[TopNode], layout: &Layout, main_config: &Path) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| {
            let TopNode::Output { name, settings } = node else {
                return None;
            };
            if !layout.entries.iter().any(|live| names(name, live)) {
                return None;
            }
            let mut others: Vec<&str> = Vec::new();
            for setting in settings {
                if !SETTINGS.contains(&setting.as_str()) && !others.contains(&setting.as_str()) {
                    others.push(setting);
                }
            }
            (!others.is_empty()).then(|| {
                format!(
                    "{}: output \"{name}\" sets {}",
                    main_config.display(),
                    others.join(", ")
                )
            })
        })
        .collect()
}

/// A node at the top level of the main config that decides how the saved
/// file is read: an `include` and its path, or an `output`, its name and
/// the settings its children set, by node name
/// (`variable-refresh-rate on-demand=true` whole, as dsplr does not write
/// it from a snapshot).
#[derive(Debug, Clone, PartialEq, Eq)]
enum TopNode {
    Include(String),
    Output { name: String, settings: Vec<String> },
}

/// The `include` and `output` nodes at the top level of the main config
/// `main`, in order, with each `output` node's settings. Read line by line,
/// not parsed: a node commented out with `/-` is left out, children and
/// all, and a line dsplr cannot split ([`tokens`]: a `/*` comment, a raw or
/// multi-line string) is passed over, braces and all.
fn top_level(main: &str) -> Vec<TopNode> {
    let mut found = Vec::new();
    let mut depth = 0_usize;
    // The `output` node, by its index in `found`, whose children are read.
    let mut output: Option<usize> = None;
    for line in main.lines() {
        let Some(tokens) = tokens(line) else {
            continue;
        };
        // The node being read, and whether `/-` comments it out. A node ends
        // at a `;`, a brace or the end of its line.
        let mut node: Vec<Token> = Vec::new();
        let mut commented = false;
        for token in tokens.into_iter().chain([Token::Semicolon]) {
            match token {
                Token::SlashDash if node.is_empty() => commented = true,
                Token::Semicolon | Token::Open | Token::Close => {
                    if !commented {
                        let opens = token == Token::Open;
                        match (depth, node.as_slice()) {
                            (0, [Token::Word(word), rest @ ..]) if word == "include" => {
                                found.extend(rest.iter().find_map(|token| match token {
                                    Token::Text(path) => Some(TopNode::Include(path.clone())),
                                    _ => None,
                                }));
                            }
                            (0, [Token::Word(word), Token::Text(name), ..]) if word == "output" => {
                                found.push(TopNode::Output {
                                    name: name.clone(),
                                    settings: Vec::new(),
                                });
                                output = opens.then_some(found.len() - 1);
                            }
                            (1, [Token::Word(setting), ..]) => {
                                if let Some(TopNode::Output { settings, .. }) =
                                    output.and_then(|at| found.get_mut(at))
                                {
                                    settings.push(if on_demand(&node) {
                                        "variable-refresh-rate on-demand=true".to_owned()
                                    } else {
                                        setting.clone()
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    match token {
                        Token::Open => depth += 1,
                        Token::Close => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                output = None;
                            }
                        }
                        _ => {}
                    }
                    node.clear();
                    commented = false;
                }
                token => node.push(token),
            }
        }
    }
    found
}

/// Whether `node` is an `include` of `outputs`, its path read from
/// `main_config`'s directory when relative and a leading `~` read as
/// `$HOME`, as niri reads it.
fn includes(
    node: &TopNode,
    main_config: &Path,
    outputs: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
) -> bool {
    let TopNode::Include(path) = node else {
        return false;
    };
    let path = match path.strip_prefix('~') {
        Some(after) if after.is_empty() || after.starts_with('/') => {
            let Some(home) = home(var) else {
                return false;
            };
            home.join(after.trim_start_matches('/'))
        }
        _ => PathBuf::from(path),
    };
    let dir = main_config.parent().unwrap_or_else(|| Path::new(""));
    dir.join(path) == outputs
}

/// Whether one of `before`, the main config's top-level nodes before the
/// include, is an `output` node for a connected output of `layout`: niri
/// uses the first node naming an output, so that one wins over the saved
/// file.
fn overridden_before(before: &[TopNode], layout: &Layout) -> bool {
    before.iter().any(|node| {
        matches!(node, TopNode::Output { name, .. } if layout.entries.iter().any(|live| names(name, live)))
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use dsplr_model::{AdaptiveSync, Layout, OutputId, Transform};

    use super::{SaveError, TopNode, find_config, save, top_level};
    use crate::managed::{Comment, Conflict, FileError, Ownership, Saved, ownership, with_header};
    use crate::niri::read_outputs;
    use crate::niri::write::tests::{OWNERS_NODES, owners_layout};

    /// [`owners_layout`] as a snapshot reads it: each output's make, model
    /// and serial reported.
    fn live() -> Layout {
        let mut layout = owners_layout();
        for (entry, [make, model, serial]) in layout
            .entries
            .iter_mut()
            .zip([["BOE", "0x0AFC", ""], ["AOC", "Q27G4", "SERIAL0000001"]])
        {
            let output = entry.output.as_mut().unwrap();
            (output.make, output.model, output.serial) = (make.into(), model.into(), serial.into());
        }
        layout
    }

    /// A `niri/config.kdl` holding `main` in a config directory, and the
    /// variables that make niri's lookup find it.
    struct Config {
        _dir: tempfile::TempDir,
        home: PathBuf,
        niri: PathBuf,
    }

    impl Config {
        fn new(main: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let niri = home.join(".config/niri");
            fs::create_dir_all(&niri).unwrap();
            fs::write(niri.join("config.kdl"), main).unwrap();
            Self {
                _dir: dir,
                home,
                niri,
            }
        }

        fn var(&self) -> impl Fn(&str) -> Option<OsString> + '_ {
            |name| (name == "HOME").then(|| self.home.clone().into_os_string())
        }

        fn outputs(&self) -> PathBuf {
            self.niri.join("outputs.kdl")
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
        let config = Config::new("// the user's own config\n");
        let saved = config.save(&live(), None).unwrap();
        assert_eq!(saved.path, config.outputs());
        assert!(saved.created);
        assert!(!saved.included);
        assert!(!saved.overridden);
        assert!(saved.first_wins);
        assert_eq!(saved.main_config, config.niri.join("config.kdl"));
        assert_eq!(saved.include_line, r#"include "outputs.kdl""#);
        assert_eq!(saved.rules, "output nodes");
        assert!(saved.kept.is_empty());
        assert!(saved.beaten_at_startup.is_empty());
        assert_eq!(config.read(), with_header(OWNERS_NODES, Comment::Slashes));
        assert!(config.read().starts_with("// Managed by dsplr; "));
        assert_eq!(
            ownership(&config.read(), Comment::Slashes),
            Ownership::Managed
        );
    }

    #[test]
    fn a_file_dsplr_wrote_is_overwritten_and_one_edited_waits_for_approval() {
        let config = Config::new("include \"outputs.kdl\"\n");
        assert!(config.save(&live(), None).unwrap().included);
        let mut moved = live();
        moved.entries[1].state.x = 1920;
        let saved = config.save(&moved, None).unwrap();
        assert!(!saved.created);
        assert!(config.read().contains("    position x=1920 y=0\n"));

        let edited = config.read().replace("scale 1\n", "scale 2\n");
        fs::write(config.outputs(), &edited).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::EditedByHand);
        assert!(conflict.needs_force());
        assert_eq!(conflict.new, with_header(OWNERS_NODES, Comment::Slashes));
        assert_eq!(config.read(), edited, "nothing was written");
        let stale = edited.replace("scale 2", "scale 3");
        let _ = conflict_of(config.save(&live(), Some(&stale)));
        assert_eq!(config.read(), edited, "other text approves nothing");
        config.save(&live(), Some(&conflict.existing)).unwrap();
        assert_eq!(config.read(), conflict.new);
    }

    /// The panel has no serial and is saved by connector; niri also knows
    /// it as `BOE 0x0AFC Unknown`, and a node by that name, or by its
    /// connector in another case, is for it and replaced. Nodes for outputs
    /// not connected go first.
    #[test]
    fn nodes_for_outputs_not_connected_are_carried_over_first() {
        let config = Config::new("");
        let old = "output \"DP-9\" {\n    position x=2560 y=0\n}\n\noutput \"hdmi-a-1\" { scale 2; }\noutput \"boe 0x0afc unknown\" { off; }\noutput \"Dell Inc. U2720Q X1\" { off; }\n";
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
            ]
        );
        let text = config.read();
        let names: Vec<String> = read_outputs(&text)
            .unwrap()
            .into_iter()
            .map(|node| node.name)
            .collect();
        assert_eq!(
            names,
            [
                "DP-9",
                "Dell Inc. U2720Q X1",
                "eDP-1",
                "AOC Q27G4 SERIAL0000001"
            ]
        );
        assert!(
            text.contains("output \"DP-9\" {\n    position x=2560 y=0\n}\n"),
            "kept as it was: {text}"
        );
        assert!(!text.contains("scale 2"), "{text}");
    }

    #[test]
    fn a_file_dsplr_cannot_read_takes_force_and_carries_nothing_over() {
        let config = Config::new("");
        let old = "output \"DP-9\" {\n    off\n}\noutput \"HDMI-A-1\" {\n    backdrop-color \"#001100\"\n}\n";
        fs::write(config.outputs(), old).unwrap();
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Foreign);
        assert!(conflict.needs_force(), "a yes would lose the backdrop");
        let reason = conflict.unreadable.as_deref().unwrap();
        assert!(
            reason.contains("line 5") && reason.contains("`backdrop-color`"),
            "{reason}"
        );
        config.save(&live(), Some(old)).unwrap();
        assert_eq!(config.read(), with_header(OWNERS_NODES, Comment::Slashes));
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
                Err(SaveError::Write(crate::niri::WriteError::Unsupported {
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
        layout.entries[1].state.transform = Transform::Rotate90;
        layout.entries[1].state.scale = 1.5;
        config.save(&layout, None).unwrap();
        assert_eq!(
            config.read(),
            with_header(
                &crate::niri::output_nodes(&layout).unwrap(),
                Comment::Slashes
            )
        );
        assert!(config.read().contains("    transform \"90\"\n"));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_outputs_file_is_refused() {
        let config = Config::new("");
        let source = config.home.join("dotfiles-outputs.kdl");
        fs::write(&source, "output \"DP-9\" { off; }\n").unwrap();
        std::os::unix::fs::symlink(&source, config.outputs()).unwrap();
        let result = config.save(&live(), Some("output \"DP-9\" { off; }\n"));
        assert!(
            matches!(&result, Err(SaveError::File(FileError::Symlink { target, .. })) if *target == source),
            "{result:?}"
        );
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "output \"DP-9\" { off; }\n"
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
    fn the_config_is_found_where_niri_looks() {
        let dir = tempfile::tempdir().unwrap();
        let (xdg, home) = (dir.path().join("xdg"), dir.path().join("home"));
        let make = |path: &Path| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        };
        let env = [("XDG_CONFIG_HOME", xdg.as_path()), ("HOME", home.as_path())];
        let in_xdg = xdg.join("niri/config.kdl");
        assert!(matches!(
            find_config(&vars(&env)),
            Err(SaveError::NoConfig { looked }) if looked == [in_xdg.clone()]
        ));
        make(&home.join(".config/niri/config.kdl"));
        assert!(
            find_config(&vars(&env)).is_err(),
            "~/.config is not looked in when XDG_CONFIG_HOME is set"
        );
        make(&in_xdg);
        assert_eq!(find_config(&vars(&env)).unwrap(), in_xdg);
        // Without XDG_CONFIG_HOME, or with a relative one, ~/.config.
        let relative = [
            ("XDG_CONFIG_HOME", Path::new("relative")),
            ("HOME", home.as_path()),
        ];
        for env in [&relative[1..], &relative[..]] {
            assert_eq!(
                find_config(&vars(env)).unwrap(),
                home.join(".config/niri/config.kdl")
            );
        }
        // NIRI_CONFIG wins, when absolute and not empty.
        let explicit = dir.path().join("elsewhere.kdl");
        let with = |value: &Path| {
            let env = [("NIRI_CONFIG", value), ("HOME", home.as_path())];
            find_config(&vars(&env))
        };
        assert!(
            matches!(with(&explicit), Err(SaveError::NoConfig { looked }) if looked == [explicit.clone()])
        );
        make(&explicit);
        assert_eq!(with(&explicit).unwrap(), explicit);
        assert_eq!(
            with(Path::new("")).unwrap(),
            home.join(".config/niri/config.kdl")
        );
        assert!(matches!(
            with(Path::new("rel.kdl")),
            Err(SaveError::RelativeConfig(_))
        ));
        assert!(matches!(
            find_config(&|_: &str| None::<OsString>),
            Err(SaveError::NoConfigDir)
        ));
        let message = with(&dir.path().join("gone.kdl")).unwrap_err().to_string();
        assert!(message.contains("/etc/niri/config.kdl"), "{message}");
    }

    // POSIX paths (`/etc/...` absolute, `/` the separator), which the
    // Linux-only code is written for.
    #[cfg(unix)]
    #[test]
    fn an_include_is_found_in_the_forms_niri_reads() {
        use super::{INCLUDE_LINE, includes};

        let main_config = Path::new("/h/.config/niri/config.kdl");
        let outputs = Path::new("/h/.config/niri/outputs.kdl");
        let env = [("HOME", Path::new("/h"))];
        let includes_it = |main: &str| {
            top_level(main)
                .iter()
                .any(|node| includes(node, main_config, outputs, &vars(&env)))
        };
        for main in [
            INCLUDE_LINE,
            "include \"./outputs.kdl\"",
            "  include   \"outputs.kdl\"  // saved by dsplr",
            "include optional=true \"outputs.kdl\"",
            "include \"~/.config/niri/outputs.kdl\"",
            "include \"/h/.config/niri/outputs.kdl\"",
            "input {\n}\ninclude \"outputs.kdl\"",
        ] {
            assert!(includes_it(main), "{main}");
        }
        for main in [
            "",
            "// include \"outputs.kdl\"",
            "/-include \"outputs.kdl\"",
            "include \"outputs.kdl.bak\"",
            "include \"niri/outputs.kdl\"",
            "include \"~user/outputs.kdl\"",
            "input {\n    include \"outputs.kdl\"\n}",
            "spawn-at-startup \"include\" \"outputs.kdl\"",
        ] {
            assert!(!includes_it(main), "{main}");
        }
    }

    /// niri uses the first node naming an output, so a main config's own
    /// node for a connected output before the include wins; one after it,
    /// or for an output not connected, does not.
    #[test]
    fn output_nodes_before_the_include_are_seen_to_win() {
        for (main, overridden) in [
            ("include \"outputs.kdl\"\n", false),
            (
                "include \"outputs.kdl\"\noutput \"eDP-1\" { off; }\n",
                false,
            ),
            ("output \"eDP-1\" { off; }\ninclude \"outputs.kdl\"\n", true),
            ("output \"EDP-1\" { off; }\ninclude \"outputs.kdl\"\n", true),
            (
                "output \"AOC Q27G4 SERIAL0000001\" {\n    scale 2\n}\ninclude \"outputs.kdl\"\n",
                true,
            ),
            (
                "output \"BOE 0x0AFC Unknown\" { off; }\ninclude \"outputs.kdl\"\n",
                true,
            ),
            ("output \"DP-9\" { off; }\ninclude \"outputs.kdl\"\n", false),
            (
                "/-output \"eDP-1\" { off; }\ninclude \"outputs.kdl\"\n",
                false,
            ),
            (
                "// output \"eDP-1\" { off; }\ninclude \"outputs.kdl\"\n",
                false,
            ),
        ] {
            let config = Config::new(main);
            let saved = config.save(&live(), None).unwrap();
            assert!(saved.included, "{main}");
            assert_eq!(saved.overridden, overridden, "{main}");
        }
    }

    #[test]
    fn only_top_level_nodes_count_each_with_its_settings() {
        let output = |name: &str, settings: &[&str]| TopNode::Output {
            name: name.into(),
            settings: settings.iter().map(|&s| s.to_owned()).collect(),
        };
        assert_eq!(
            top_level(
                "output \"A\" {\n    scale 2\n    /-off\n    layout {\n        gaps 8\n    }\n    focus-at-startup\n}\nbinds {\n    Mod+T { spawn \"output\"; }\n    include \"x\"\n}\ninclude optional=true \"o.kdl\"\n/* skipped */\noutput \"B\" { off; variable-refresh-rate on-demand=true; }\n/-output \"C\" {\n    backdrop-color \"#000\"\n}\noutput \"D\"\n"
            ),
            [
                output("A", &["scale", "layout", "focus-at-startup"]),
                TopNode::Include("o.kdl".into()),
                output("B", &["off", "variable-refresh-rate on-demand=true"]),
                output("D", &[]),
            ]
        );
    }

    /// niri uses the first node naming an output, whole: once the file is
    /// written, a node of the main config after the include for a connected
    /// output stops applying, so one setting what dsplr does not write asks
    /// first, even with no file there yet.
    // POSIX paths (`/etc/...` absolute, `/` the separator), which the
    // Linux-only code is written for.
    #[cfg(unix)]
    #[test]
    fn main_config_nodes_the_saved_file_would_hide_are_asked_about() {
        let main = "include \"outputs.kdl\"\noutput \"eDP-1\" {\n    focus-at-startup\n    scale 2\n}\noutput \"hdmi-a-1\" { backdrop-color \"#000\"; variable-refresh-rate on-demand=true; }\noutput \"DP-9\" { focus-at-startup; }\n";
        let config = Config::new(main);
        let conflict = conflict_of(config.save(&live(), None));
        assert_eq!(conflict.ownership, Ownership::Absent);
        assert_eq!(conflict.existing, "");
        assert!(!conflict.needs_force(), "a yes is enough");
        let main_config = config.niri.join("config.kdl");
        assert_eq!(
            conflict.shadowed,
            [
                format!(
                    "{}: output \"eDP-1\" sets focus-at-startup",
                    main_config.display()
                ),
                format!(
                    "{}: output \"hdmi-a-1\" sets backdrop-color, variable-refresh-rate on-demand=true",
                    main_config.display()
                ),
            ]
        );
        assert!(
            conflict.to_string().contains("would stop these settings"),
            "{conflict}"
        );
        assert!(!config.outputs().exists(), "nothing was written");
        config.save(&live(), Some("")).unwrap();
        assert_eq!(config.read(), with_header(OWNERS_NODES, Comment::Slashes));
        // Once written, a save over dsplr's own file asks again.
        let again = conflict_of(config.save(&live(), None));
        assert_eq!(again.ownership, Ownership::Managed);
        assert_eq!(again.shadowed.len(), 2);
        config.save(&live(), Some(&again.existing)).unwrap();
    }

    #[test]
    fn only_hidden_settings_dsplr_does_not_write_ask() {
        for (main, asks) in [
            // Before the include it wins instead (`overridden`).
            (
                "output \"eDP-1\" { focus-at-startup; }\ninclude \"outputs.kdl\"\n",
                false,
            ),
            // Only what dsplr writes: the saved node replaces it.
            (
                "include \"outputs.kdl\"\noutput \"eDP-1\" { scale 2; position x=0 y=0; }\n",
                false,
            ),
            (
                "include \"outputs.kdl\"\n/-output \"eDP-1\" { focus-at-startup; }\n",
                false,
            ),
            (
                "include \"outputs.kdl\"\noutput \"eDP-1\" { /-focus-at-startup; }\n",
                false,
            ),
            // No include yet: it goes before them, so every node counts.
            ("output \"eDP-1\" { focus-at-startup; }\n", true),
            (
                "include \"outputs.kdl\"\noutput \"BOE 0x0AFC Unknown\" { focus-at-startup; }\n",
                true,
            ),
        ] {
            let config = Config::new(main);
            let result = config.save(&live(), None);
            assert_eq!(
                matches!(result, Err(SaveError::File(FileError::Conflict(_)))),
                asks,
                "{main}: {result:?}"
            );
        }
    }

    /// wlr-output-management reads on-demand adaptive sync as on or off, so
    /// the old file's `on-demand=true` is kept where the layout has it on.
    #[test]
    fn on_demand_adaptive_sync_is_carried_over_where_it_is_on() {
        let config = Config::new("");
        let aoc_vrr = |vrr: &str| {
            with_header(
                &OWNERS_NODES.replace(
                    "    position x=0 y=0\n}\n",
                    &format!("    position x=0 y=0\n    {vrr}\n}}\n"),
                ),
                Comment::Slashes,
            )
        };
        let with = |sync| {
            let mut layout = live();
            layout.entries[1].state.adaptive_sync = Some(sync);
            layout
        };
        let on_demand = "variable-refresh-rate on-demand=true";
        fs::write(config.outputs(), aoc_vrr(on_demand)).unwrap();
        config.save(&with(AdaptiveSync::On), None).unwrap();
        assert_eq!(config.read(), aoc_vrr(on_demand));
        // Off is written as off; the next save has nothing to carry.
        config.save(&with(AdaptiveSync::Off), None).unwrap();
        assert!(!config.read().contains("variable-refresh-rate"));
        config.save(&with(AdaptiveSync::On), None).unwrap();
        assert_eq!(config.read(), aoc_vrr("variable-refresh-rate"));
    }
}
