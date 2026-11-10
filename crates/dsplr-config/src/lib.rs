//! dsplr's own settings, read from `$XDG_CONFIG_HOME/dsplr/config.toml`.
//!
//! What is here: the confirm timeout override and the GUI's keys
//! ([`keys`]). Every key is optional, and a missing file means the defaults;
//! an unknown key is an error, so a typo is reported rather than ignored.
//! The theme and the other settings in `docs/plan/02-architecture.md` are
//! added with the code that reads them.

use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

pub mod keys;

pub use hjkl_config::{ConfigError, ConfigSource};

/// How long a change waits to be kept before it is reverted, unless the
/// config says otherwise: Mutter's own `DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`
/// (`docs/plan/12-open-questions.md`, question 5).
pub const CONFIRM_TIMEOUT_SECS: NonZeroU32 = NonZeroU32::new(20).unwrap();

/// Every setting, as the config file gives it or the default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub confirm: Confirm,
    pub keys: keys::Keys,
}

/// The `[confirm]` table: the keep-or-revert prompt after a change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Confirm {
    /// Seconds before an unconfirmed change is reverted. Never zero: a change
    /// that reverts before it can be confirmed could never be kept.
    pub timeout_secs: NonZeroU32,
}

impl Default for Confirm {
    fn default() -> Self {
        Self {
            timeout_secs: CONFIRM_TIMEOUT_SECS,
        }
    }
}

impl Confirm {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.get().into())
    }
}

impl hjkl_config::AppConfig for Config {
    const APPLICATION: &'static str = "dsplr";
}

impl Config {
    /// Reads `config.toml` from dsplr's XDG config directory, or the defaults
    /// when there is no file (or no home directory). Never writes.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not TOML, or has a key or value
    /// dsplr does not know. The error names the file, and the line where it
    /// can.
    pub fn load() -> Result<(Self, ConfigSource), ConfigError> {
        hjkl_config::load()
    }

    /// Reads the settings from `path`, as `--config <PATH>` will.
    ///
    /// # Errors
    ///
    /// As [`Config::load`], and when `path` does not exist.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        hjkl_config::load_from(path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::{Config, ConfigError};

    fn load(text: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        Config::load_from(&path)
    }

    #[test]
    fn an_empty_file_gives_the_defaults() {
        let config = load("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.confirm.timeout(), Duration::from_secs(20));
    }

    #[test]
    fn the_confirm_timeout_can_be_set() {
        let config = load("[confirm]\ntimeout_secs = 45\n").unwrap();
        assert_eq!(config.confirm.timeout(), Duration::from_secs(45));
    }

    #[test]
    fn a_zero_timeout_is_refused() {
        let error = load("[confirm]\ntimeout_secs = 0\n").unwrap_err();
        assert!(
            matches!(error, ConfigError::Parse { line: 2, .. }),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let error = load("[confirm]\ntimeout = 45\n").unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
        let error = load("[confrim]\ntimeout_secs = 45\n").unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
    }

    #[test]
    fn a_key_can_be_rebound_and_the_others_keep_theirs() {
        let config = load("[keys]\nnudge_left = [\"Left\", \"h\"]\nredo = [\"C-y\"]\n").unwrap();
        let names = |list: &[super::keys::KeySpec]| {
            list.iter().map(ToString::to_string).collect::<Vec<_>>()
        };
        assert_eq!(names(&config.keys.nudge_left), ["Left", "h"]);
        assert_eq!(names(&config.keys.redo), ["C-y"]);
        assert_eq!(names(&config.keys.undo), ["u"], "the default");
    }

    #[test]
    fn an_unknown_action_or_key_or_a_key_bound_twice_is_refused() {
        for (text, says) in [
            ("[keys]\nnudge_sideways = [\"h\"]\n", "nudge_sideways"),
            ("[keys]\nundo = [\"Ctrl-z\"]\n", "is not a key"),
            ("[keys]\nundo = [\"S-u\"]\n", "Shift is part of a character"),
            ("[keys]\nundo = [\"r\"]\n", "bound to both"),
        ] {
            let error = load(text).unwrap_err();
            assert!(
                matches!(error, ConfigError::Parse { .. }),
                "{text}: {error}"
            );
            assert!(error.to_string().contains(says), "{text}: {error}");
        }
    }

    #[test]
    fn a_missing_named_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = Config::load_from(&dir.path().join("absent.toml")).unwrap_err();
        assert!(matches!(error, ConfigError::Io { .. }), "{error}");
    }
}
