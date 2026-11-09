//! dsplr's profile store from the command line
//! (`docs/plan/05-persistence-and-profiles.md`, "dsplr's profile store"):
//! `dsplr save --profile NAME`, `dsplr restore [NAME]`, and
//! `dsplr profile list|show|rm`.

use std::path::PathBuf;
use std::time::SystemTime;

use dsplr_backend::Backend;
use dsplr_model::Layout;
use dsplr_persist::diff::line_diff;
use dsplr_persist::managed::refuse_symlink;
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

/// Why [`store_layout`] saved nothing.
pub enum Blocked {
    /// A profile of that name exists: what saving changes in it.
    Exists { diff: String },
    /// A profile of that name exists and cannot be read: why.
    Unreadable(String),
    /// Saving failed, said as a whole message.
    Failed(String),
}

/// Saves `layout` as the profile `name` in the store at `dir`. Unless
/// `approved`, a profile of that name already there blocks it, so the user
/// can be asked first. Returns what to tell the user, one message each.
pub fn store_layout(
    dir: &std::path::Path,
    name: &ProfileName,
    layout: Layout,
    approved: bool,
) -> Result<Vec<String>, Blocked> {
    // A link is refused however the question would be answered, so before
    // it is asked.
    refuse_symlink(&profile::path(dir, name))
        .map_err(|error| Blocked::Failed(format!("cannot save the profile: {error}")))?;
    if !approved {
        match profile::load(dir, name) {
            Ok(None) => {}
            Ok(Some(old)) => {
                return Err(Blocked::Exists {
                    diff: line_diff(
                        &crate::list::describe(&old.layout),
                        &crate::list::describe(&layout),
                    ),
                });
            }
            Err(error) => return Err(Blocked::Unreadable(error.to_string())),
        }
    }
    let profile = Profile {
        name: name.clone(),
        layout,
        saved_unix_ms: unix_ms(SystemTime::now()),
    };
    match profile::save(dir, &profile) {
        Ok(path) => Ok(vec![
            format!("saved profile \"{name}\" to {}", path.display()),
            format!(
                "`dsplr restore \"{name}\"` applies it, and `dsplr restore` picks it while these outputs are connected"
            ),
        ]),
        Err(error) => Err(Blocked::Failed(format!("cannot save the profile: {error}"))),
    }
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
    let replace = options.yes || options.force;
    let mut approved = false;
    loop {
        match store_layout(&dir, name, layout.clone(), approved) {
            Ok(messages) => {
                for message in messages {
                    eprintln!("dsplr: {message}");
                }
                return Exit::Success;
            }
            Err(Blocked::Exists { diff }) => {
                eprintln!(
                    "dsplr: there is a profile named \"{name}\" already; this is what saving changes in it:"
                );
                eprint!("{diff}");
                if !save::replace_allowed(&format!("profile \"{name}\""), replace) {
                    return Exit::Failure;
                }
            }
            Err(Blocked::Unreadable(error)) if options.force => {
                eprintln!("dsplr: {error}; replacing it, as --force says");
            }
            Err(Blocked::Unreadable(error)) => {
                eprintln!("dsplr: {error}; nothing was saved, and --force replaces it");
                return Exit::Failure;
            }
            Err(Blocked::Failed(message)) => {
                eprintln!("dsplr: {message}");
                return Exit::Failure;
            }
        }
        approved = true;
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
    let profiles = match readable(&dir) {
        Ok(profiles) => profiles,
        Err(exit) => return exit,
    };
    let backend = match find_backend().await {
        Ok(backend) => backend,
        Err(exit) => return exit,
    };
    let current = match crate::snapshot(&backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    let Some((profile, fit)) = profile::pick(&profiles, &current) else {
        eprintln!(
            "dsplr: no profile is for the outputs connected now ({}), so nothing changed",
            outputs(&current)
        );
        if profiles.is_empty() {
            eprintln!("dsplr: there are no profiles; `dsplr save --profile NAME` saves one");
        }
        return Exit::NoProfile;
    };
    match fit {
        Fit::Exact => eprintln!(
            "dsplr: restoring profile \"{}\", saved for the outputs connected now",
            profile.name.as_str()
        ),
        Fit::Subset => eprintln!(
            "dsplr: restoring profile \"{}\", saved for {} of the outputs connected now; the others stay as they are",
            profile.name.as_str(),
            outputs(&profile.layout)
        ),
    }
    apply::onto(&backend, current, &profile.layout, options).await
}

/// The profiles in the store at `dir` that can be read, the others
/// reported.
fn readable(dir: &std::path::Path) -> Result<Vec<Profile>, Exit> {
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
fn outputs(layout: &Layout) -> String {
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
    use dsplr_persist::profile::Profile;

    use super::{age, describe};

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
            },
            saved_unix_ms: 1_000,
        };
        assert_eq!(
            describe(&[desk], 7_201_000),
            "laptop only: eDP-1, AOC Q27G4 SER1 (saved 2 hours ago)\n"
        );
    }
}
