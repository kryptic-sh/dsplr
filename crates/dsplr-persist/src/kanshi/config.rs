//! kanshi's config as kanshi 1.9.0 reads it (`config.c`, and kanshi(5)):
//! profiles of outputs, output defaults and aliases at the top level, and
//! `include`. What kanshi refuses is refused here too, so a config dsplr
//! imports is one kanshi would run.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use dsplr_model::Transform;

use super::scfg::{self, Directive};

/// A mode as a kanshi output names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KanshiMode {
    /// `mode preferred`.
    Preferred,
    /// `mode [--custom] WxH[@R[Hz]]`; without a refresh, kanshi takes the
    /// highest one listed at that size.
    Size {
        width: u32,
        height: u32,
        refresh_hz: Option<f64>,
        custom: bool,
    },
}

/// One `output` directive: its criteria and what it sets, each `None` when
/// it leaves that as the output has it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KanshiOutput {
    /// A connector name, `make model serial` (a glob pattern), `*` or an
    /// alias.
    pub criteria: String,
    /// `...output`: any number of outputs.
    pub any_number: bool,
    pub enabled: Option<bool>,
    pub mode: Option<KanshiMode>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
    pub transform: Option<Transform>,
    pub adaptive_sync: Option<bool>,
    /// `alias $name`, only at the top level.
    pub alias: Option<String>,
}

/// One profile.
#[derive(Debug, Clone, PartialEq)]
pub struct KanshiProfile {
    /// `None` for an anonymous profile.
    pub name: Option<String>,
    /// kanshi's own number for an anonymous profile, counted from 1 in the
    /// order kanshi reads them (`<anonymous profile N>` in its log).
    pub anonymous: Option<u32>,
    pub outputs: Vec<KanshiOutput>,
    /// How many `exec` directives it has, which dsplr does not run.
    pub execs: usize,
    /// Where it is defined: its file and line.
    pub at: String,
}

/// A whole config: its profiles, in kanshi's order (the first that matches
/// wins), with the top-level output defaults and aliases resolved into them
/// as kanshi resolves them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KanshiConfig {
    pub profiles: Vec<KanshiProfile>,
    /// What was read otherwise than kanshi would read it from wherever it
    /// runs, one note each: a relative `include`, resolved against `$HOME`.
    pub notes: Vec<String>,
}

/// Why a config cannot be read, where kanshi would refuse it too.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Scfg {
        path: PathBuf,
        source: scfg::ScfgError,
    },
    #[error("{path}: line {line}: {reason}")]
    Kanshi {
        path: PathBuf,
        line: usize,
        reason: String,
    },
    #[error("profile {profile}: {reason}")]
    Profile { profile: String, reason: String },
}

/// kanshi's config file as kanshi finds it: `$XDG_CONFIG_HOME/kanshi/config`,
/// else `$HOME/.config/kanshi/config` (`read_config` in kanshi's `main.c`).
/// `None` when neither variable is set.
pub fn default_path(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name| var(name).filter(|value| !value.is_empty());
    set("XDG_CONFIG_HOME")
        .map(|config| Path::new(&config).join("kanshi/config"))
        .or_else(|| set("HOME").map(|home| Path::new(&home).join(".config/kanshi/config")))
}

/// Reads the config at `path`, its includes with it, expanding them with
/// `var`'s variables.
///
/// # Errors
///
/// A file that cannot be read, scfg kanshi cannot parse, a directive or
/// setting kanshi refuses, or an include path dsplr cannot expand.
pub fn read(
    path: &Path,
    var: &impl Fn(&str) -> Option<OsString>,
) -> Result<KanshiConfig, ConfigError> {
    let mut reader = Reader {
        profiles: Vec::new(),
        defaults: Vec::new(),
        anonymous: 0,
        notes: Vec::new(),
    };
    reader.file(path, var, 0)?;
    reader.resolve()
}

/// How deep includes may nest: kanshi sets no limit, and an include of
/// itself would never end.
const MAX_INCLUDE_DEPTH: usize = 16;

struct Reader {
    profiles: Vec<KanshiProfile>,
    defaults: Vec<KanshiOutput>,
    anonymous: u32,
    notes: Vec<String>,
}

impl Reader {
    fn file(
        &mut self,
        path: &Path,
        var: &impl Fn(&str) -> Option<OsString>,
        depth: usize,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        let directives = scfg::parse(&text).map_err(|source| ConfigError::Scfg {
            path: path.to_owned(),
            source,
        })?;
        let fail = |line, reason: String| ConfigError::Kanshi {
            path: path.to_owned(),
            line,
            reason,
        };
        for directive in &directives {
            match directive.name.as_str() {
                "profile" => {
                    let profile = self
                        .profile(directive, path)
                        .map_err(|reason| fail(directive.line, reason))?;
                    self.profiles.push(profile);
                }
                "output" => {
                    let output =
                        output(directive).map_err(|reason| fail(directive.line, reason))?;
                    if output.criteria == "*" {
                        return Err(fail(
                            directive.line,
                            "wildcard outputs can only be used in profile scope".into(),
                        ));
                    }
                    if output.criteria.starts_with('$') {
                        return Err(fail(
                            directive.line,
                            "output aliases can only be used in profile scope".into(),
                        ));
                    }
                    if self.defaults.iter().any(|d| d.criteria == output.criteria) {
                        return Err(fail(
                            directive.line,
                            format!("duplicate output '{}' in global scope", output.criteria),
                        ));
                    }
                    self.defaults.push(output);
                }
                "include" => {
                    let [pattern] = directive.params.as_slice() else {
                        return Err(fail(
                            directive.line,
                            "'include' takes exactly one parameter".into(),
                        ));
                    };
                    if depth >= MAX_INCLUDE_DEPTH {
                        return Err(fail(
                            directive.line,
                            format!("includes nest more than {MAX_INCLUDE_DEPTH} deep"),
                        ));
                    }
                    let paths = expand(pattern, var, &mut self.notes)
                        .map_err(|reason| fail(directive.line, reason))?;
                    for included in paths {
                        self.file(&included, var, depth + 1)?;
                    }
                }
                other => {
                    return Err(fail(directive.line, format!("unknown directive '{other}'")));
                }
            }
        }
        Ok(())
    }

    fn profile(&mut self, directive: &Directive, path: &Path) -> Result<KanshiProfile, String> {
        let (name, anonymous) = match directive.params.as_slice() {
            [] => {
                self.anonymous += 1;
                (None, Some(self.anonymous))
            }
            [name] => (Some(name.clone()), None),
            _ => return Err("'profile' takes zero or one parameter".into()),
        };
        let mut outputs: Vec<KanshiOutput> = Vec::new();
        let mut execs = 0;
        for child in &directive.children {
            match child.name.as_str() {
                "output" | "...output" => {
                    let output = output(child)?;
                    if output.alias.is_some() {
                        return Err("output aliases can only be defined in global scope".into());
                    }
                    if outputs.iter().any(|o| o.criteria == output.criteria) {
                        return Err(format!("duplicate output '{}' in profile", output.criteria));
                    }
                    outputs.push(output);
                }
                "exec" => {
                    if child.params.is_empty() {
                        return Err("'exec' takes a command".into());
                    }
                    execs += 1;
                }
                other => return Err(format!("unknown directive '{other}' in a profile")),
            }
        }
        Ok(KanshiProfile {
            name,
            anonymous,
            outputs,
            execs,
            at: format!("{}:{}", path.display(), directive.line),
        })
    }

    /// Resolves aliases and output defaults into the profiles, as kanshi's
    /// `resolve_output_defaults` does: for each profile output, the first
    /// default whose alias or criteria it names fills what it leaves unset.
    fn resolve(self) -> Result<KanshiConfig, ConfigError> {
        let mut profiles = self.profiles;
        for profile in &mut profiles {
            let label = label(profile);
            for output in &mut profile.outputs {
                for default in &self.defaults {
                    if default.alias.as_deref() == Some(output.criteria.as_str()) {
                        output.criteria.clone_from(&default.criteria);
                    }
                    if output.criteria == default.criteria {
                        fill(output, default);
                        break;
                    }
                }
                if output.criteria.starts_with('$') {
                    return Err(ConfigError::Profile {
                        profile: label,
                        reason: format!("use of undefined output alias '{}'", output.criteria),
                    });
                }
            }
        }
        Ok(KanshiConfig {
            profiles,
            notes: self.notes,
        })
    }
}

/// How a profile is named in messages: its name, or kanshi's own name for
/// an anonymous one.
pub fn label(profile: &KanshiProfile) -> String {
    match (&profile.name, profile.anonymous) {
        (Some(name), _) => format!("\"{name}\""),
        (None, number) => format!("<anonymous profile {}>", number.unwrap_or_default()),
    }
}

fn fill(output: &mut KanshiOutput, default: &KanshiOutput) {
    output.enabled = output.enabled.or(default.enabled);
    output.mode = output.mode.or(default.mode);
    output.position = output.position.or(default.position);
    output.scale = output.scale.or(default.scale);
    output.transform = output.transform.or(default.transform);
    output.adaptive_sync = output.adaptive_sync.or(default.adaptive_sync);
}

/// One `output` directive: its criteria, then settings on its line, or one
/// per line in its block.
fn output(directive: &Directive) -> Result<KanshiOutput, String> {
    let Some((criteria, mut rest)) = directive.params.split_first() else {
        return Err(format!("'{}' takes at least one parameter", directive.name));
    };
    let mut output = KanshiOutput {
        criteria: criteria.clone(),
        any_number: directive.name == "...output",
        ..KanshiOutput::default()
    };
    while let Some((name, after)) = rest.split_first() {
        let taken = setting(&mut output, name, after)?;
        rest = &after[taken..];
    }
    for child in &directive.children {
        let taken = setting(&mut output, &child.name, &child.params)?;
        if taken != child.params.len() {
            return Err("only one setting per line is allowed in output blocks".into());
        }
    }
    Ok(output)
}

/// Sets `name` on `output` from `params`, returning how many it took.
fn setting(output: &mut KanshiOutput, name: &str, params: &[String]) -> Result<usize, String> {
    match name {
        "enable" => {
            output.enabled = Some(true);
            return Ok(0);
        }
        "disable" => {
            output.enabled = Some(false);
            return Ok(0);
        }
        _ => {}
    }
    let Some(value) = params.first() else {
        return Err(format!("'{name}' takes a parameter"));
    };
    let mut taken = 1;
    match name {
        "mode" if value == "preferred" => output.mode = Some(KanshiMode::Preferred),
        "mode" => {
            let custom = value == "--custom";
            let size = if custom {
                taken = 2;
                params.get(1).ok_or("'mode --custom' takes a mode")?
            } else {
                value
            };
            output.mode = Some(mode(size, custom).ok_or_else(|| format!("invalid mode '{size}'"))?);
        }
        "position" => {
            let (x, y) = value
                .split_once(',')
                .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
                .ok_or_else(|| format!("invalid position '{value}'"))?;
            output.position = Some((x, y));
        }
        "scale" => {
            output.scale = Some(
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|scale| scale.is_finite())
                    .ok_or_else(|| format!("invalid scale '{value}'"))?,
            );
        }
        "transform" => {
            output.transform =
                Some(transform(value).ok_or_else(|| format!("invalid transform '{value}'"))?);
        }
        "adaptive_sync" => {
            output.adaptive_sync = Some(match value.as_str() {
                "on" => true,
                "off" => false,
                _ => return Err(format!("invalid adaptive_sync '{value}'")),
            });
        }
        "alias" => {
            if !value.starts_with('$') {
                return Err(format!("invalid output alias '{value}', must start with $"));
            }
            output.alias = Some(value.clone());
        }
        other => {
            return Err(format!(
                "unknown setting '{other}' for output '{}'",
                output.criteria
            ));
        }
    }
    Ok(taken)
}

/// `WxH[@R[Hz]]`, as kanshi's `parse_mode` reads it.
fn mode(text: &str, custom: bool) -> Option<KanshiMode> {
    let (width, rest) = text.split_once('x')?;
    let (height, refresh) = match rest.split_once('@') {
        Some((height, refresh)) => (height, Some(refresh)),
        None => (rest, None),
    };
    let refresh_hz = match refresh {
        None => None,
        Some(refresh) => Some(
            refresh
                .strip_suffix("Hz")
                .unwrap_or(refresh)
                .parse::<f64>()
                .ok()
                .filter(|hz| hz.is_finite() && *hz >= 0.0)?,
        ),
    };
    Some(KanshiMode::Size {
        width: width.parse().ok()?,
        height: height.parse().ok()?,
        refresh_hz,
        custom,
    })
}

/// kanshi's transform names, which are wl_output's, as dsplr's are.
fn transform(text: &str) -> Option<Transform> {
    Some(match text {
        "normal" => Transform::Normal,
        "90" => Transform::Rotate90,
        "180" => Transform::Rotate180,
        "270" => Transform::Rotate270,
        "flipped" => Transform::Flipped,
        "flipped-90" => Transform::Flipped90,
        "flipped-180" => Transform::Flipped180,
        "flipped-270" => Transform::Flipped270,
        _ => return None,
    })
}

/// The files an `include` names. kanshi expands it with `wordexp(3)`; dsplr
/// expands the parts of that a config path uses: a leading `~` or `~/`,
/// `$NAME` and `${NAME}` (an unset one is an error, as kanshi's
/// `WRDE_UNDEF` makes it), and `*` and `?` in the last component, matched as
/// `glob(7)` matches them (a leading `.` only by a pattern's own), the
/// matches sorted. A pattern matching nothing is kept as it is, as
/// `wordexp` keeps it. The rest of the shell's syntax (quotes, `$(`,
/// backticks, braces, `[`, whitespace, which would split it into several
/// paths) is refused rather than taken literally.
///
/// A relative path is relative to kanshi's working directory, which kanshi
/// never changes (no `chdir` in kanshi 1.9.0): it is whatever started it,
/// the compositor, usually from `$HOME`. dsplr runs elsewhere, so a relative
/// path is taken against `$HOME`, with a note in `notes`.
fn expand(
    pattern: &str,
    var: &impl Fn(&str) -> Option<OsString>,
    notes: &mut Vec<String>,
) -> Result<Vec<PathBuf>, String> {
    let fail = |reason: String| format!("include '{pattern}': {reason}");
    let mut expanded = String::new();
    let mut rest = pattern;
    if let Some(after) = rest.strip_prefix('~')
        && (after.is_empty() || after.starts_with('/'))
    {
        let home = var("HOME").ok_or_else(|| fail("HOME is not set".into()))?;
        expanded.push_str(&home.to_string_lossy());
        rest = after;
    }
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' => {
                let mut name = String::new();
                if chars.next_if_eq(&'{').is_some() {
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => name.push(c),
                            None => return Err(fail("'${' left open".into())),
                        }
                    }
                } else {
                    while let Some(c) = chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '_') {
                        name.push(c);
                    }
                }
                if name.is_empty() {
                    return Err(fail("a '$' with no variable name".into()));
                }
                let value = var(&name).ok_or_else(|| fail(format!("{name} is not set")))?;
                expanded.push_str(&value.to_string_lossy());
            }
            '"' | '\'' | '`' | '{' | '}' | '[' | ']' | '\\' | '(' | ')' | ';' | '|' | '&' | '<'
            | '>' => return Err(fail(format!("dsplr does not expand '{c}'"))),
            c if c.is_whitespace() => {
                return Err(fail("dsplr does not split it into several paths".into()));
            }
            c => expanded.push(c),
        }
    }

    let mut path = PathBuf::from(&expanded);
    if path.is_relative() {
        let home = var("HOME").ok_or_else(|| {
            fail("a relative path, and HOME, which dsplr takes it against, is not set".into())
        })?;
        path = Path::new(&home).join(path);
        notes.push(format!(
            "include '{pattern}' is relative, so it is taken against $HOME ({}), kanshi's usual working directory, as {}",
            Path::new(&home).display(),
            path.display()
        ));
    }
    let Some(file) = path.file_name().map(|f| f.to_string_lossy().into_owned()) else {
        return Ok(vec![path]);
    };
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    if parent.to_string_lossy().contains(['*', '?']) {
        return Err(fail(
            "dsplr expands '*' and '?' only in the last component".into(),
        ));
    }
    if !file.contains(['*', '?']) {
        return Ok(vec![path]);
    }
    let mut matches: Vec<PathBuf> = std::fs::read_dir(&parent)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    glob_matches(&file, &name).then(|| parent.join(name))
                })
                .collect()
        })
        .unwrap_or_default();
    if matches.is_empty() {
        return Ok(vec![path]);
    }
    matches.sort();
    Ok(matches)
}

/// Whether `name` matches `pattern`, `*` and `?` as `glob(7)` reads them
/// for one path component: `*` any run of characters, `?` any one, and a
/// leading `.` in `name` matched only by a leading `.` in `pattern`.
fn glob_matches(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // The classic two-pointer wildcard match, backtracking to the last `*`.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use dsplr_model::Transform;

    use super::{ConfigError, KanshiMode, KanshiOutput, default_path, expand, glob_matches, read};

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.into())
        }
    }

    fn read_text(text: &str) -> Result<super::KanshiConfig, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, text).unwrap();
        read(&path, &vars(&[]))
    }

    #[test]
    fn kanshi_s_lookup_is_xdg_config_home_then_home() {
        assert_eq!(
            default_path(vars(&[("XDG_CONFIG_HOME", "/c"), ("HOME", "/h")])).as_deref(),
            Some(Path::new("/c/kanshi/config"))
        );
        assert_eq!(
            default_path(vars(&[("HOME", "/h")])).as_deref(),
            Some(Path::new("/h/.config/kanshi/config"))
        );
        assert_eq!(default_path(vars(&[])), None);
    }

    /// kanshi(5)'s own example, every output setting, and the defaults and
    /// alias section's example.
    #[test]
    fn the_documented_syntax_reads_as_kanshi_reads_it() {
        let config = read_text(
            "output eDP-1 scale 2\n\
             output \"Foocorp ASDF 1234\" alias $desk\n\
             profile {\n\
             \toutput LVDS-1 disable\n\
             \toutput \"Some Company ASDF 4242\" {\n\
             \t\tmode 1600x900\n\
             \t\tposition 0,0\n\
             \t}\n\
             }\n\
             profile nomad {\n\
             \toutput LVDS-1 enable scale 2\n\
             }\n\
             profile all {\n\
             \toutput eDP-1\n\
             \toutput $desk mode --custom 1280x720@60Hz position 1600,-10 transform flipped-90 adaptive_sync on\n\
             \toutput HDMI-A-1 mode 1920x1080@59.94 transform 270\n\
             \toutput DP-2 mode preferred\n\
             \texec swaymsg workspace 1, move workspace to eDP-1\n\
             }\n",
        )
        .unwrap();
        let profiles = &config.profiles;
        assert_eq!(profiles.len(), 3);
        assert_eq!(
            (profiles[0].name.as_deref(), profiles[0].anonymous),
            (None, Some(1))
        );
        assert_eq!(
            profiles[0].outputs[0],
            KanshiOutput {
                criteria: "LVDS-1".into(),
                enabled: Some(false),
                ..KanshiOutput::default()
            }
        );
        assert_eq!(
            profiles[0].outputs[1].mode,
            Some(KanshiMode::Size {
                width: 1600,
                height: 900,
                refresh_hz: None,
                custom: false
            })
        );
        assert_eq!(profiles[1].name.as_deref(), Some("nomad"));
        let all = &profiles[2];
        assert_eq!(all.execs, 1);
        assert_eq!(all.outputs[0].scale, Some(2.0), "the top-level default");
        let desk = &all.outputs[1];
        assert_eq!(desk.criteria, "Foocorp ASDF 1234", "the alias resolved");
        assert_eq!(
            desk.mode,
            Some(KanshiMode::Size {
                width: 1280,
                height: 720,
                refresh_hz: Some(60.0),
                custom: true
            })
        );
        assert_eq!(desk.position, Some((1600, -10)));
        assert_eq!(desk.transform, Some(Transform::Flipped90));
        assert_eq!(desk.adaptive_sync, Some(true));
        assert_eq!(all.outputs[2].transform, Some(Transform::Rotate270));
        assert_eq!(all.outputs[3].mode, Some(KanshiMode::Preferred));
    }

    #[test]
    fn a_profile_s_own_setting_wins_over_the_default() {
        let config = read_text(
            "output eDP-1 scale 2 position 0,0\nprofile p {\n\toutput eDP-1 scale 1.5\n}\n",
        )
        .unwrap();
        let output = &config.profiles[0].outputs[0];
        assert_eq!((output.scale, output.position), (Some(1.5), Some((0, 0))));
    }

    #[test]
    fn what_kanshi_refuses_is_refused() {
        for text in [
            "frobnicate\n",
            "output * scale 2\n",
            "output $a scale 2\n",
            "output A scale 1\noutput A scale 2\n",
            "profile a b {\n}\n",
            "profile {\n\toutput A\n\toutput A\n}\n",
            "profile {\n\toutput A alias $x\n}\n",
            "profile {\n\toutput $nobody\n}\n",
            "profile {\n\tfoo\n}\n",
            "profile {\n\texec\n}\n",
            "profile {\n\toutput A mode 1920\n}\n",
            "profile {\n\toutput A mode 1920x1080@fast\n}\n",
            "profile {\n\toutput A position 1,2,3\n}\n",
            "profile {\n\toutput A transform 45\n}\n",
            "profile {\n\toutput A adaptive_sync maybe\n}\n",
            "profile {\n\toutput A frob 1\n}\n",
            "profile {\n\toutput A {\n\t\tscale 1 position 0,0\n\t}\n}\n",
        ] {
            assert!(read_text(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn includes_expand_home_variables_and_a_glob() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("config.d");
        std::fs::create_dir(&d).unwrap();
        std::fs::write(d.join("b"), "profile b {\n\toutput B\n}\n").unwrap();
        std::fs::write(d.join("a"), "profile a {\n\toutput A\n}\n").unwrap();
        std::fs::write(d.join(".hidden"), "frobnicate\n").unwrap();
        let main = dir.path().join("config");
        std::fs::write(&main, "include $ROOT/config.d/*\ninclude ~/c2\n").unwrap();
        std::fs::write(dir.path().join("c2"), "profile c {\n\toutput C\n}\n").unwrap();
        let root = dir.path().to_str().unwrap();
        let config = read(&main, &vars(&[("ROOT", root), ("HOME", root)])).unwrap();
        let names: Vec<_> = config
            .profiles
            .iter()
            .map(|p| p.name.clone().unwrap())
            .collect();
        assert_eq!(names, ["a", "b", "c"], "sorted, the dot file left out");

        // An unset variable, and what dsplr does not expand, are errors.
        for bad in ["include $NOPE/x\n", "include $(echo x)\n", "include 'x'\n"] {
            std::fs::write(&main, bad).unwrap();
            assert!(read(&main, &vars(&[("ROOT", root)])).is_err(), "{bad:?}");
        }
        // An include of itself stops.
        std::fs::write(&main, format!("include {}\n", main.display())).unwrap();
        assert!(read(&main, &vars(&[])).is_err());
    }

    // POSIX paths (`/etc/...` absolute, `/` the separator), which the
    // Linux-only code is written for.
    #[cfg(unix)]
    #[test]
    fn variables_take_braces_and_a_pattern_matching_nothing_stays() {
        let set = vars(&[("A", "/x")]);
        let mut notes = Vec::new();
        assert_eq!(
            expand("${A}/y", &set, &mut notes).unwrap(),
            [Path::new("/x/y")]
        );
        assert_eq!(
            expand("/nonexistent-dsplr/*", &set, &mut notes).unwrap(),
            [Path::new("/nonexistent-dsplr/*")]
        );
        assert!(
            expand("/a*/b", &set, &mut notes).is_err(),
            "a glob above the last component"
        );
        assert!(expand("/a b", &set, &mut notes).is_err(), "two words");
        assert!(expand("${A", &set, &mut notes).is_err());
        assert_eq!(notes, Vec::<String>::new(), "absolute paths need no note");
    }

    /// kanshi runs where the compositor started it, usually `$HOME`, not
    /// where dsplr runs: a relative include is taken against `$HOME`, and
    /// said.
    #[test]
    fn a_relative_include_is_taken_against_home() {
        let mut notes = Vec::new();
        let home = vars(&[("HOME", "/h")]);
        assert_eq!(
            expand("kanshi/extra", &home, &mut notes).unwrap(),
            [Path::new("/h/kanshi/extra")]
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("$HOME (/h)"), "{notes:?}");
        assert!(expand("extra", &vars(&[]), &mut notes).is_err(), "no HOME");
    }

    #[test]
    fn glob_matches_as_glob_7_does_for_star_and_question_mark() {
        for (pattern, name, matches) in [
            ("*", "a", true),
            ("*", ".a", false),
            (".*", ".a", true),
            ("a*c", "abbbc", true),
            ("a*c", "abbbd", false),
            ("a?c", "abc", true),
            ("a?c", "ac", false),
            ("*.conf", "x.conf", true),
            ("*b*", "abc", true),
            ("", "", true),
        ] {
            assert_eq!(glob_matches(pattern, name), matches, "{pattern} {name}");
        }
    }
}
