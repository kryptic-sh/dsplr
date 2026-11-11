//! Saving on X11 (`docs/plan/05-persistence-and-profiles.md#x11`): an X
//! server keeps no display config of its own, so the layout goes to dsplr's
//! profile store, as the profile for the outputs connected now
//! ([`profile_name`]), and `dsplr restore` puts it back at login. That
//! runs from an XDG autostart entry dsplr owns
//! (`$XDG_CONFIG_HOME/autostart/dsplr-restore.desktop`), written through the
//! managed-file rules, or, under a window manager that runs no XDG
//! autostart (i3, bspwm, as `XDG_CURRENT_DESKTOP` names it), from the line
//! the user is shown to add to its own startup. A desktop that restores its
//! own layout ([`OWN_RESTORE`]) gets neither.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dsplr_model::Layout;

use crate::managed::{self, Comment, FileError, NO_CONFIG_DIR, config_home, sha256_hex};
use crate::profile::{self, Profile, ProfileError, ProfileName, unix_ms};

/// The autostart entry's file name, in `$XDG_CONFIG_HOME/autostart`.
pub const AUTOSTART_FILE: &str = "dsplr-restore.desktop";

/// What the autostart entry runs dsplr with: `dsplr restore --login`
/// declines unless this is an X11 session whose desktop leaves the displays
/// to it ([`OWN_RESTORE`]), since the entry runs in every session that runs
/// XDG autostart, Wayland ones included.
pub const LOGIN_ARGS: &str = "restore --login --yes";

/// The desktops, as `XDG_CURRENT_DESKTOP` names them, that put their own
/// saved display layout back at login on X11: Plasma (KScreen), Xfce
/// (xfsettingsd), MATE (mate-settings-daemon's xrandr plugin), Cinnamon
/// (csd-xrandr), LXQt (lxqt-config-monitor's autostart), Budgie
/// (gsd-xrandr), Deepin (its display daemon) and Unity (unity-settings-
/// daemon). dsplr restoring a profile there too would fight them, so a save
/// writes no autostart entry and `dsplr restore --login` declines. GNOME is
/// not listed: on GNOME the Mutter backend is found before X11, and Mutter
/// restores its own `monitors.xml`.
pub const OWN_RESTORE: [&str; 8] = [
    "KDE",
    "XFCE",
    "MATE",
    "X-Cinnamon",
    "LXQt",
    "Budgie",
    "Deepin",
    "Unity",
];

/// The desktop of [`OWN_RESTORE`] `XDG_CURRENT_DESKTOP` names, if any: the
/// variable is a colon-separated list, each name compared without case.
pub fn own_restore(var: &impl Fn(&str) -> Option<OsString>) -> Option<&'static str> {
    let desktops = var("XDG_CURRENT_DESKTOP")?;
    desktops.to_str()?.split(':').find_map(|name| {
        OWN_RESTORE
            .into_iter()
            .find(|own| own.eq_ignore_ascii_case(name))
    })
}

/// What the profile name built from outputs is cut to: room in
/// [`profile`]'s limit for nothing else.
const HASH_HEX: usize = 12;

/// The characters the Desktop Entry Specification (1.5, "The Exec key")
/// reserves in an `Exec` argument, which is then quoted.
const EXEC_RESERVED: &[char] = &[
    ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')',
    '`',
];

/// A window manager that runs no XDG autostart entries, so the restore is
/// started from its own startup instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wm {
    I3,
    Bspwm,
}

impl Wm {
    /// The window manager `XDG_CURRENT_DESKTOP` names, when it is one of
    /// these: the variable is a colon-separated list, each name compared
    /// without case.
    pub fn current(var: &impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let desktops = var("XDG_CURRENT_DESKTOP")?;
        desktops
            .to_str()?
            .split(':')
            .find_map(|name| match name.to_ascii_lowercase().as_str() {
                "i3" => Some(Self::I3),
                "bspwm" => Some(Self::Bspwm),
                _ => None,
            })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::I3 => "i3",
            Self::Bspwm => "bspwm",
        }
    }

    /// Where its startup is, as its documentation puts it.
    pub const fn config(self) -> &'static str {
        match self {
            Self::I3 => "i3's config (~/.config/i3/config)",
            Self::Bspwm => "bspwmrc (~/.config/bspwm/bspwmrc)",
        }
    }

    /// The line that runs `dsplr restore --login --yes` at its start: an i3 `exec` command,
    /// or a shell line in bspwmrc, which is a shell script, sent to the
    /// background so the rest of it is not held up.
    pub const fn line(self) -> &'static str {
        match self {
            Self::I3 => "exec --no-startup-id dsplr restore --login --yes",
            Self::Bspwm => "dsplr restore --login --yes &",
        }
    }
}

/// How the profile is put back at login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// The autostart entry, written.
    Autostart {
        path: PathBuf,
        /// Whether it did not exist before.
        created: bool,
        /// Whether `XDG_CURRENT_DESKTOP` named no desktop, so whether the
        /// session runs XDG autostart is not known.
        unknown_desktop: bool,
    },
    /// The window manager's own startup, which dsplr does not edit.
    Line(Wm),
    /// Nothing: the desktop, as `XDG_CURRENT_DESKTOP` names it, puts its
    /// own layout back at login ([`OWN_RESTORE`]).
    Desktop(&'static str),
}

/// What [`save`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub name: ProfileName,
    pub profile: PathBuf,
    pub login: Login,
    /// The name the outputs would have had ([`profile_name`]) when another
    /// profile has it: one the user saved, or one kept for other outputs.
    /// `name` is then the hash name.
    pub taken: Option<ProfileName>,
}

/// Why [`save`] wrote nothing, or not all of it.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("there are no outputs to save")]
    Empty,
    #[error("{NO_CONFIG_DIR}")]
    NoConfigDir,
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error("dsplr's own path, {0}, cannot be written in an autostart entry")]
    Exe(String),
    #[error("cannot find dsplr's own path for the autostart entry: {0}")]
    NoExe(std::io::Error),
    #[error(
        "profiles named \"{0}\" and \"{1}\" are other outputs', so there is no name to save these outputs under; remove one"
    )]
    NoName(ProfileName, ProfileName),
}

/// Saves `layout` as the profile for its outputs in the store at `dir`
/// ([`profile_name`], or the hash name when another profile has that one,
/// [`Saved::taken`]), replacing the one this save kept for them before, and
/// sees that it is restored at login ([`Login`]) by `exe`, dsplr's own path.
/// The autostart entry is written as every file dsplr owns is
/// ([`managed::replace`]): one dsplr wrote is overwritten, any other is a
/// [`FileError::Conflict`] unless `approved` is what it holds. That is
/// checked before anything is written, so on a conflict nothing is; then the
/// profile is written, and the entry after it, so an entry is never left
/// without the profile it restores.
///
/// # Errors
///
/// When `layout` has no outputs, there is no config directory, `exe` cannot
/// be written in an entry, the entry is a conflict, no name is free, or a
/// write fails.
pub fn save(
    var: impl Fn(&str) -> Option<OsString>,
    exe: &Path,
    dir: &Path,
    layout: &Layout,
    approved: Option<&str>,
) -> Result<Saved, SaveError> {
    if layout.entries.is_empty() {
        return Err(SaveError::Empty);
    }
    // What the login needs, and the entry's conflict, before any write.
    let (login, write) = if let Some(desktop) = own_restore(&var) {
        (Login::Desktop(desktop), None)
    } else if let Some(wm) = Wm::current(&var) {
        (Login::Line(wm), None)
    } else {
        let path = autostart_path(&var).ok_or(SaveError::NoConfigDir)?;
        let text = entry(exe)?;
        let existing = managed::read_existing(&path)?;
        managed::check(
            &path,
            existing.as_deref(),
            None,
            text.clone(),
            approved,
            Comment::Hash,
            Vec::new(),
        )?;
        let login = Login::Autostart {
            path: path.clone(),
            created: existing.is_none(),
            unknown_desktop: var("XDG_CURRENT_DESKTOP").is_none_or(|v| v.is_empty()),
        };
        (login, Some((path, text)))
    };
    let (name, taken) = free_name(dir, layout)?;
    let profile = profile::save_for_outputs(
        dir,
        &Profile {
            name: name.clone(),
            layout: layout.clone(),
            saved_unix_ms: unix_ms(SystemTime::now()),
        },
    )?;
    if let Some((path, text)) = write {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(FileError::io(parent))?;
        }
        managed::write_atomic(&path, &text).map_err(FileError::io(&path))?;
    }
    Ok(Saved {
        name,
        profile,
        login,
        taken,
    })
}

/// The name to save `layout`'s outputs under in the store at `dir`: their
/// own ([`profile_name`]) when no profile has it, or when the one that has
/// it is one this save kept for the same outputs; else the hash name, under
/// the same rule, with the name that was taken. A profile that cannot be read
/// counts as taken.
fn free_name(dir: &Path, layout: &Layout) -> Result<(ProfileName, Option<ProfileName>), SaveError> {
    let ids = sorted_ids(layout);
    let free = |name: &ProfileName| match profile::load_marked(dir, name) {
        Ok(None) => true,
        Ok(Some((profile, for_outputs))) => for_outputs && sorted_ids(&profile.layout) == ids,
        Err(_) => false,
    };
    let readable = profile_name(layout);
    if free(&readable) {
        return Ok((readable, None));
    }
    let hashed = hash_name(&ids);
    if free(&hashed) {
        Ok((hashed, Some(readable)))
    } else {
        Err(SaveError::NoName(readable, hashed))
    }
}

/// `layout`'s outputs' identities, sorted, as text.
fn sorted_ids(layout: &Layout) -> Vec<String> {
    let mut ids: Vec<String> = layout
        .entries
        .iter()
        .map(|entry| entry.id.to_string())
        .collect();
    ids.sort();
    ids
}

/// `outputs ` and the start of a SHA-256 over `ids`.
fn hash_name(ids: &[String]) -> ProfileName {
    let hash = sha256_hex(&ids.join("\n"));
    format!("outputs {}", &hash[..HASH_HEX])
        .parse()
        .unwrap_or_else(|error| unreachable!("a hex name is valid: {error}"))
}

/// `$XDG_CONFIG_HOME/autostart/dsplr-restore.desktop`, `XDG_CONFIG_HOME`
/// defaulting to `~/.config` (XDG Autostart Specification 0.5).
pub fn autostart_path(var: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    config_home(var).map(|dir| dir.join("autostart").join(AUTOSTART_FILE))
}

/// The autostart entry for `exe`, dsplr's own path, with dsplr's header:
/// `Exec` runs it by that path with [`LOGIN_ARGS`], so it works without the
/// session's `PATH` (a `~/.cargo/bin` install), and `TryExec` skips the
/// entry once that file is gone. A comment line may come before the first
/// group (Desktop Entry Specification 1.5, "Comments").
///
/// # Errors
///
/// When `exe` is not UTF-8, or holds a control character, which no entry
/// can hold.
pub fn entry(exe: &Path) -> Result<String, SaveError> {
    let unusable = || SaveError::Exe(exe.display().to_string());
    let exe = exe.to_str().ok_or_else(unusable)?;
    if exe.chars().any(char::is_control) {
        return Err(unusable());
    }
    let body = format!(
        "[Desktop Entry]
Type=Application
Name=dsplr restore
Comment=Restores the display profile saved for the connected outputs
Exec={} {LOGIN_ARGS}
TryExec={}
NoDisplay=true
Terminal=false
",
        string_value(&exec_argument(exe)),
        string_value(exe),
    );
    Ok(managed::with_header(&body, Comment::Hash))
}

/// `argument` as an `Exec` argument (Desktop Entry Specification 1.5, "The
/// Exec key"): `%` doubled, as a field code would read it; and, when it
/// holds a reserved character ([`EXEC_RESERVED`]), in double quotes, with
/// `"`, `` ` ``, `$` and `\` each after a backslash. The string escape
/// ([`string_value`]) is applied over this.
fn exec_argument(argument: &str) -> String {
    let argument = argument.replace('%', "%%");
    if !argument.contains(EXEC_RESERVED) {
        return argument;
    }
    let mut quoted = String::from("\"");
    for c in argument.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// `text` as a value of type string in a desktop entry (Desktop Entry
/// Specification 1.5, "Possible value types"): each backslash doubled. The
/// other escapes (`\s`, `\n`, `\t`, `\r`) are for characters [`entry`]
/// refuses or, a space, that only needs one at the start of a value, where
/// an absolute path or a quote is.
fn string_value(text: &str) -> String {
    text.replace('\\', "\\\\")
}

/// The name of the profile [`save`] keeps for `layout`'s outputs: their
/// identities ([`dsplr_model::OutputId`]) in order, joined by ` - `, with
/// each character a profile name cannot hold made `_`. When that is not a
/// valid name (too long, say), `outputs ` and the start of a SHA-256 over
/// the identities.
pub fn profile_name(layout: &Layout) -> ProfileName {
    let ids = sorted_ids(layout);
    let readable = ids
        .iter()
        .map(|id| {
            id.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .replace("..", "_.")
                .trim_matches(|c| c == ' ' || c == '.')
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(" - ");
    readable.parse().unwrap_or_else(|_| hash_name(&ids))
}

#[cfg(test)]
mod tests;
