//! dsplr's profile store from the command line
//! (`docs/plan/05-persistence-and-profiles.md`, "dsplr's profile store"):
//! `dsplr save --profile NAME`, `dsplr restore [NAME]`,
//! `dsplr profile list|show|rm`, and `dsplr profile import kanshi|autorandr`.

use std::path::PathBuf;
use std::time::SystemTime;

use dsplr_backend::{Backend, Named};
use dsplr_model::Layout;
use dsplr_persist::diff::line_diff;
use dsplr_persist::login;
use dsplr_persist::managed::{FileError, refuse_symlink};
use dsplr_persist::profile::{self, Fit, Profile, ProfileError, ProfileName, unix_ms};

use crate::{Exit, apply, find_backend, print, save};

/// The store's directory, or, when there is none, the error reported and
/// the exit code.
fn store() -> Result<PathBuf, Exit> {
    store_dir().map_err(|error| {
        eprintln!("dsplr: {error}");
        Exit::Failure
    })
}

/// The store's directory, or why there is none.
pub fn store_dir() -> Result<PathBuf, String> {
    profile::default_dir().map_err(|error| error.to_string())
}

/// What [`store_layout`] may replace in the store.
pub enum Replace {
    /// Nothing: a profile of that name blocks the save.
    Nothing,
    /// The profile of that name as it was when the user agreed to replace
    /// it: its file's text, from [`Blocked`]. A profile changed since then
    /// is asked about again.
    Seen(String),
    /// Whatever is there (`--force`).
    Anything,
}

/// Why [`store_layout`] saved nothing.
pub enum Blocked {
    /// A profile of that name exists: what saving changes in it, and its
    /// file's text, which [`Replace::Seen`] gives back once the user agrees.
    Exists { diff: String, seen: String },
    /// A profile of that name exists and cannot be read: why, and its
    /// file's text, as for [`Blocked::Exists`].
    Unreadable { why: String, seen: String },
    /// Saving failed, said as a whole message.
    Failed(String),
}

/// Saves `layout` as the profile `name` in the store at `dir`. A profile of
/// that name already there blocks it unless `replace` covers it, so the
/// user can be asked first, and it is replaced only if it still holds what
/// it held then ([`profile::save_replacing`]). Returns what to tell the
/// user, one message each.
pub fn store_layout(
    dir: &std::path::Path,
    name: &ProfileName,
    layout: Layout,
    replace: &Replace,
) -> Result<Vec<String>, Blocked> {
    let failed = |error: ProfileError| Blocked::Failed(format!("cannot save the profile: {error}"));
    // A link is refused however the question would be answered, so before
    // it is asked.
    refuse_symlink(&profile::path(dir, name)).map_err(|error| failed(error.into()))?;
    let existing = profile::read_text(dir, name).map_err(failed)?;
    if let Some(text) = &existing {
        let agreed = match replace {
            Replace::Nothing => false,
            Replace::Seen(seen) => seen == text,
            Replace::Anything => true,
        };
        if !agreed {
            let seen = text.clone();
            return Err(match profile::load(dir, name) {
                Ok(Some(old)) => Blocked::Exists {
                    diff: line_diff(
                        &crate::list::describe(&old.layout),
                        &crate::list::describe(&layout),
                    ),
                    seen,
                },
                // Removed since it was read: said as the change it is.
                Ok(None) => failed(
                    FileError::Changed {
                        path: profile::path(dir, name),
                    }
                    .into(),
                ),
                Err(error) => Blocked::Unreadable {
                    why: error.to_string(),
                    seen,
                },
            });
        }
    }
    let profile = Profile {
        name: name.clone(),
        layout,
        saved_unix_ms: unix_ms(SystemTime::now()),
    };
    match profile::save_replacing(dir, &profile, existing.as_deref()) {
        Ok(path) => Ok(vec![
            format!("saved profile \"{name}\" to {}", path.display()),
            format!(
                "`dsplr restore \"{name}\"` applies it, and `dsplr restore` picks it while these outputs are connected"
            ),
        ]),
        Err(error) => Err(Blocked::Failed(format!("cannot save the profile: {error}"))),
    }
}

/// `dsplr profile import kanshi [PATH]`: kanshi's profiles into the store
/// (`docs/plan/05-persistence-and-profiles.md`, "Importing kanshi
/// profiles"). The displays are read first, when a desktop is found, for
/// what kanshi leaves to an output. A profile of the same name in the store
/// is replaced only with `force`. Exits `0` when every kanshi profile was
/// imported, `1` when one was skipped or not written, and says which.
pub async fn import_kanshi(path: Option<PathBuf>, force: bool) -> Exit {
    use dsplr_persist::kanshi::{self, config};

    let Some(path) = path.or_else(|| config::default_path(|name| std::env::var_os(name))) else {
        eprintln!(
            "dsplr: neither XDG_CONFIG_HOME nor HOME is set, so kanshi's config cannot be found; name it"
        );
        return Exit::Failure;
    };
    let read = match config::read(&path, &|name| std::env::var_os(name)) {
        Ok(read) => read,
        Err(error) => {
            eprintln!("dsplr: cannot import kanshi's config: {error}");
            return Exit::Failure;
        }
    };
    for note in &read.notes {
        eprintln!("dsplr: {note}");
    }
    let live = live("kanshi").await;
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    let (imported, skipped) = kanshi::import(&read, live.as_ref());
    if imported.is_empty() && skipped.is_empty() {
        eprintln!("dsplr: {} has no profiles", path.display());
        return Exit::Success;
    }
    store_imported("kanshi", &dir, imported, skipped, force)
}

/// `dsplr profile import autorandr [PATH]`: autorandr's profiles into the
/// store (`docs/plan/05-persistence-and-profiles.md`, "Importing autorandr
/// profiles"), from `PATH` (a directory of profiles, or one profile's) or
/// from every directory autorandr loads profiles from. As the kanshi
/// import: the displays are read for what autorandr leaves to an output, a
/// profile of the same name in the store is replaced only with `force`,
/// and the exit is `0` when every profile was imported, else `1`.
pub async fn import_autorandr(path: Option<PathBuf>, force: bool) -> Exit {
    use dsplr_persist::autorandr;

    let dirs = match path {
        Some(path) if path.is_file() => {
            eprintln!(
                "dsplr: {} is a file; name a directory of autorandr's profiles, or one profile's (holding config and setup)",
                path.display()
            );
            return Exit::Failure;
        }
        Some(path) => vec![path],
        None => autorandr::default_dirs(|name| std::env::var_os(name)),
    };
    let read = match &dirs[..] {
        [dir] if autorandr::is_profile(dir) => autorandr::read_one(dir),
        dirs => match autorandr::read(dirs) {
            Ok(read) => read,
            Err(error) => {
                eprintln!("dsplr: cannot import autorandr's profiles: {error}");
                return Exit::Failure;
            }
        },
    };
    for note in &read.notes {
        eprintln!("dsplr: {note}");
    }
    let live = live("autorandr").await;
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    let (imported, skipped) =
        autorandr::import(&read, live.as_ref(), dsplr_persist::pnp::system_names());
    if imported.is_empty() && skipped.is_empty() {
        let dirs: Vec<String> = dirs.iter().map(|dir| dir.display().to_string()).collect();
        eprintln!("dsplr: no autorandr profiles in {}", dirs.join(", "));
        return Exit::Success;
    }
    store_imported("autorandr", &dir, imported, skipped, force)
}

/// The desktop's outputs now, for what `tool` leaves to an output, or
/// `None`, said, when there is no desktop or it cannot be read.
async fn live(tool: &str) -> Option<Layout> {
    match crate::detection().await.backend {
        Some(backend) => match backend.snapshot().await {
            Ok(layout) => Some(layout),
            Err(error) => {
                eprintln!(
                    "dsplr: cannot read the displays from {} ({error}), so nothing {tool} leaves to an output is taken from it",
                    backend.name()
                );
                None
            }
        },
        None => {
            eprintln!(
                "dsplr: no desktop found, so nothing {tool} leaves to an output is taken from it"
            );
            None
        }
    }
}

/// Each of `tool`'s profiles imported into the store at `dir`, with its
/// notes, and each skipped one with why. A profile of the same name in the
/// store is replaced only with `force`. `1` when one was skipped or not
/// written.
fn store_imported(
    tool: &str,
    dir: &std::path::Path,
    imported: Vec<dsplr_persist::kanshi::Imported>,
    skipped: Vec<dsplr_persist::kanshi::Skipped>,
    force: bool,
) -> Exit {
    let mut exit = Exit::Success;
    for profile in imported {
        eprintln!("dsplr: {tool}'s {}:", profile.from);
        for note in &profile.notes {
            eprintln!("dsplr:   {note}");
        }
        // Said, since a name made from the tool's can be one the store had.
        let replacing = force && profile::path(dir, &profile.name).exists();
        let replace = if force {
            Replace::Anything
        } else {
            Replace::Nothing
        };
        match store_layout(dir, &profile.name, profile.layout, &replace) {
            Ok(messages) => {
                if replacing {
                    eprintln!(
                        "dsplr:   replaces the existing profile \"{}\"",
                        profile.name
                    );
                }
                for message in messages {
                    eprintln!("dsplr:   {message}");
                }
            }
            Err(Blocked::Exists { .. }) => {
                eprintln!(
                    "dsplr:   not imported: the store has a profile \"{}\"; --force replaces it",
                    profile.name
                );
                exit = Exit::Failure;
            }
            Err(Blocked::Unreadable { why, .. }) => {
                eprintln!(
                    "dsplr:   not imported: the store's profile \"{}\" cannot be read ({why}); --force replaces it",
                    profile.name
                );
                exit = Exit::Failure;
            }
            Err(Blocked::Failed(why)) => {
                eprintln!("dsplr:   {why}");
                exit = Exit::Failure;
            }
        }
    }
    for profile in skipped {
        eprintln!(
            "dsplr: {tool}'s {}: skipped: {}",
            profile.from, profile.reason
        );
        exit = Exit::Failure;
    }
    exit
}

/// The profile `name` in the store at `dir`, or, when there is none or it
/// cannot be read, the error reported and the exit code: a name that names
/// no profile is a usage error, as a name that names no output is.
fn load(dir: &std::path::Path, name: &ProfileName) -> Result<Profile, Exit> {
    match profile::load(dir, name) {
        Ok(Some(profile)) => Ok(profile),
        Ok(None) => {
            eprintln!("dsplr: no profile named \"{name}\"; `dsplr profile list` lists them");
            Err(Exit::Usage)
        }
        Err(error) => {
            eprintln!("dsplr: {error}");
            Err(Exit::Failure)
        }
    }
}

/// `dsplr save --profile NAME`: the displays as they are now, saved as the
/// profile `name`. A profile of that name is replaced only with `--yes` or
/// `--force`, or a yes on the terminal, after the change is shown.
pub async fn save(backend: &Backend, name: &ProfileName, options: &save::Options) -> Exit {
    let layout = match crate::snapshot(backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    if layout.entries.is_empty() {
        eprintln!(
            "dsplr: {} reports no outputs, so there is nothing to save",
            backend.name()
        );
        return Exit::Failure;
    }
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    let allowed = options.yes || options.force;
    let mut replace = Replace::Nothing;
    loop {
        match store_layout(&dir, name, layout.clone(), &replace) {
            Ok(messages) => {
                for message in messages {
                    eprintln!("dsplr: {message}");
                }
                return Exit::Success;
            }
            Err(Blocked::Exists { diff, seen }) => {
                eprintln!(
                    "dsplr: there is a profile named \"{name}\" already; this is what saving changes in it:"
                );
                eprint!("{diff}");
                if !save::replace_allowed(&format!("profile \"{name}\""), allowed) {
                    return Exit::Failure;
                }
                replace = Replace::Seen(seen);
            }
            Err(Blocked::Unreadable { why, seen }) if options.force => {
                eprintln!("dsplr: {why}; replacing it, as --force says");
                replace = Replace::Seen(seen);
            }
            Err(Blocked::Unreadable { why, .. }) => {
                eprintln!("dsplr: {why}; nothing was saved, and --force replaces it");
                return Exit::Failure;
            }
            Err(Blocked::Failed(message)) => {
                eprintln!("dsplr: {message}");
                return Exit::Failure;
            }
        }
    }
}

/// `dsplr restore [NAME]`: applies the profile `name`, or without one the
/// profile [`profile::pick`] picks for the outputs connected now, through
/// the confirm flow. With no profile to pick it changes nothing, says so and
/// exits [`Exit::NoProfile`]. Unreadable profiles are reported and passed
/// over.
pub async fn restore(name: Option<&ProfileName>, options: &apply::Options) -> Exit {
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    if let Some(name) = name {
        let profile = match load(&dir, name) {
            Ok(profile) => profile,
            Err(exit) => return exit,
        };
        let backend = match find_backend().await {
            Ok(backend) => backend,
            Err(exit) => return exit,
        };
        eprintln!("dsplr: restoring profile \"{name}\"");
        return apply::file(&backend, &profile.layout, options).await;
    }
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    restore_picked(&backend, &dir, options).await
}

/// `dsplr restore --login`: [`restore`] of the profile picked for the
/// outputs, unless the desktop puts its own layout back
/// ([`login::own_restore`]) or the session is not X11 (no backend, or
/// another than RandR: `WAYLAND_DISPLAY` set, or Mutter found first); then
/// nothing, said to no one, and [`Exit::Success`], as the autostart entry
/// runs in every session that runs XDG autostart.
pub async fn restore_at_login(options: &apply::Options) -> Exit {
    if login::own_restore(&|name: &str| std::env::var_os(name)).is_some() {
        return Exit::Success;
    }
    let Some(backend) = crate::detection()
        .await
        .backend
        .filter(|backend| backend.name() == Named::Randr.name())
    else {
        return Exit::Success;
    };
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    restore_picked(&backend, &dir, options).await
}

/// Applies the profile [`profile::pick`] picks in the store at `dir` for
/// `backend`'s outputs, as [`restore`] does without a name.
async fn restore_picked(
    backend: &Backend,
    dir: &std::path::Path,
    options: &apply::Options,
) -> Exit {
    let profiles = match readable(dir) {
        Ok(profiles) => profiles,
        Err(exit) => return exit,
    };
    let current = match crate::snapshot(backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    let Some((profile, fit)) = picked(&profiles, &current) else {
        return Exit::NoProfile;
    };
    eprintln!("dsplr: {}", restoring(profile, fit));
    apply::onto(backend, current, &profile.layout, options).await
}

/// The profile [`profile::pick`] picks among `profiles` for `current`, the
/// displays as they are now, and how it fits; with none, `None`, said on
/// stderr.
pub fn picked<'a>(profiles: &'a [Profile], current: &Layout) -> Option<(&'a Profile, Fit)> {
    let picked = profile::pick(profiles, current);
    if picked.is_none() {
        eprintln!(
            "dsplr: no profile is for the outputs connected now ({}), so nothing changed",
            outputs(current)
        );
        if profiles.is_empty() {
            eprintln!("dsplr: there are no profiles; `dsplr save --profile NAME` saves one");
        }
    }
    picked
}

/// The line saying `profile`, which fits the outputs connected now as
/// `fit` says, is being restored.
pub fn restoring(profile: &Profile, fit: Fit) -> String {
    match fit {
        Fit::Exact => format!(
            "restoring profile \"{}\", saved for the outputs connected now",
            profile.name.as_str()
        ),
        Fit::Subset => format!(
            "restoring profile \"{}\", saved for {} of the outputs connected now; the others stay as they are",
            profile.name.as_str(),
            outputs(&profile.layout)
        ),
    }
}

/// The profiles in the store at `dir` that can be read, the others
/// reported.
pub fn readable(dir: &std::path::Path) -> Result<Vec<Profile>, Exit> {
    let listed = profile::list(dir).map_err(|error| {
        eprintln!("dsplr: {error}");
        Exit::Failure
    })?;
    Ok(listed
        .into_iter()
        .filter_map(|profile| {
            profile
                .inspect_err(|error| match error {
                    ProfileError::Name { .. } => eprintln!("dsplr: {error}"),
                    _ => eprintln!("dsplr: {error}; passed over"),
                })
                .ok()
        })
        .collect())
}

/// `layout`'s outputs by their identities, comma-separated.
pub fn outputs(layout: &Layout) -> String {
    let names: Vec<String> = layout
        .entries
        .iter()
        .map(|entry| entry.id.to_string())
        .collect();
    names.join(", ")
}

/// `dsplr profile list`: each profile's name, outputs and age, or as JSON.
/// A profile that cannot be read is reported, and the exit code is then
/// [`Exit::Failure`] once the others are listed; a `.json` file whose name
/// is not a profile's is reported and passed over.
pub fn list(json: bool) -> Exit {
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    let listed = match profile::list(&dir) {
        Ok(listed) => listed,
        Err(error) => {
            eprintln!("dsplr: {error}");
            return Exit::Failure;
        }
    };
    let mut exit = Exit::Success;
    let profiles: Vec<Profile> = listed
        .into_iter()
        .filter_map(|profile| {
            profile
                .inspect_err(|error| {
                    eprintln!("dsplr: {error}");
                    // A stray file is said, not a failure; a profile that
                    // cannot be read is.
                    if !matches!(error, ProfileError::Name { .. }) {
                        exit = Exit::Failure;
                    }
                })
                .ok()
        })
        .collect();
    let text = if json {
        match profile::summaries_json(&profiles) {
            Ok(text) => text + "\n",
            Err(error) => {
                eprintln!("dsplr: cannot write the profiles as JSON: {error}");
                return Exit::Failure;
            }
        }
    } else {
        describe(&profiles, unix_ms(SystemTime::now()))
    };
    print(&text).unwrap_or(exit)
}

/// The text `dsplr profile list` prints at `now_ms`.
fn describe(profiles: &[Profile], now_ms: u64) -> String {
    if profiles.is_empty() {
        return "no profiles\n".to_owned();
    }
    profiles
        .iter()
        .map(|profile| {
            format!(
                "{}: {} (saved {})\n",
                profile.name,
                outputs(&profile.layout),
                age(now_ms, profile.saved_unix_ms)
            )
        })
        .collect()
}

/// How long before `now_ms` something saved at `saved_ms` was, in the
/// largest whole unit up to days. A time after `now_ms` (a clock set back)
/// is "just now".
fn age(now_ms: u64, saved_ms: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let secs = now_ms.saturating_sub(saved_ms) / 1000;
    let (count, unit) = match secs {
        0..MINUTE => return "just now".to_owned(),
        MINUTE..HOUR => (secs / MINUTE, "minute"),
        HOUR..DAY => (secs / HOUR, "hour"),
        _ => (secs / DAY, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// `dsplr profile show NAME`: the profile's layout as `dsplr list` prints
/// one, or as JSON in the form `dsplr apply` reads.
pub fn show(name: &ProfileName, json: bool) -> Exit {
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    let profile = match load(&dir, name) {
        Ok(profile) => profile,
        Err(exit) => return exit,
    };
    let text = if json {
        match profile.layout.to_json() {
            Ok(text) => text + "\n",
            Err(error) => {
                eprintln!("dsplr: cannot write the layout as JSON: {error}");
                return Exit::Failure;
            }
        }
    } else {
        crate::list::describe(&profile.layout)
    };
    print(&text).unwrap_or(Exit::Success)
}

/// `dsplr profile rm NAME`.
pub fn remove(name: &ProfileName) -> Exit {
    let dir = match store() {
        Ok(dir) => dir,
        Err(exit) => return exit,
    };
    match profile::remove(&dir, name) {
        Ok(true) => {
            eprintln!("dsplr: removed profile \"{name}\"");
            Exit::Success
        }
        Ok(false) => {
            eprintln!("dsplr: no profile named \"{name}\"; `dsplr profile list` lists them");
            Exit::Usage
        }
        Err(error) => {
            eprintln!("dsplr: {error}");
            Exit::Failure
        }
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Layout, OutputEntry, OutputId, OutputState};
    use dsplr_persist::profile::{Profile, ProfileName};

    use super::{Blocked, Replace, age, describe, store_layout};

    #[test]
    fn an_age_is_its_largest_whole_unit() {
        let now = 10_000_000_000;
        let ago = |secs: u64| age(now, now - secs * 1000);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(119), "1 minute ago");
        assert_eq!(ago(120), "2 minutes ago");
        assert_eq!(ago(3_599), "59 minutes ago");
        assert_eq!(ago(3_600), "1 hour ago");
        assert_eq!(ago(86_399), "23 hours ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(30 * 86_400), "30 days ago");
        assert_eq!(age(now, now + 5_000), "just now", "a clock set back");
    }

    /// A profile of that name blocks the save and is replaced once the user
    /// agrees to what was shown; changed meanwhile, it is asked about again.
    #[test]
    fn a_profile_is_replaced_only_as_the_user_saw_it() {
        let dir = tempfile::tempdir().unwrap();
        let name: ProfileName = "desk".parse().unwrap();
        let layout = |connector: &str| Layout {
            entries: vec![OutputEntry {
                id: OutputId::Connector(connector.into()),
                output: None,
                state: OutputState::default(),
            }],
            ..Default::default()
        };
        assert!(store_layout(dir.path(), &name, layout("DP-1"), &Replace::Nothing).is_ok());
        let Err(Blocked::Exists { seen, .. }) =
            store_layout(dir.path(), &name, layout("DP-2"), &Replace::Nothing)
        else {
            panic!("a profile of that name blocks the save");
        };

        // Changed while the question waited: asked again, nothing written.
        assert!(store_layout(dir.path(), &name, layout("HDMI-A-1"), &Replace::Anything).is_ok());
        let Err(Blocked::Exists { seen: now, .. }) = store_layout(
            dir.path(),
            &name,
            layout("DP-2"),
            &Replace::Seen(seen.clone()),
        ) else {
            panic!("a changed profile is asked about again");
        };
        assert_ne!(now, seen);
        let held = |connector: &str| {
            dsplr_persist::profile::load(dir.path(), &name)
                .unwrap()
                .is_some_and(|profile| profile.layout == layout(connector))
        };
        assert!(held("HDMI-A-1"));

        assert!(store_layout(dir.path(), &name, layout("DP-2"), &Replace::Seen(now)).is_ok());
        assert!(held("DP-2"));
    }

    #[test]
    fn the_list_names_each_profile_s_outputs_and_age() {
        assert_eq!(describe(&[], 0), "no profiles\n");
        let entry = |id: OutputId| OutputEntry {
            id,
            output: None,
            state: OutputState::default(),
        };
        let desk = Profile {
            name: "laptop only".parse().unwrap(),
            layout: Layout {
                entries: vec![
                    entry(OutputId::Connector("eDP-1".into())),
                    entry(OutputId::new("AOC", "Q27G4", "SER1", "DP-1")),
                ],
                ..Default::default()
            },
            saved_unix_ms: 1_000,
        };
        assert_eq!(
            describe(&[desk], 7_201_000),
            "laptop only: eDP-1, AOC Q27G4 SER1 (saved 2 hours ago)\n"
        );
    }
}
