//! The `[keys]` table: which keys do what in the GUI
//! (`docs/plan/06-gui.md`, "Keys"). Each action takes a list of keys, and an
//! action the file leaves out keeps the plan's. An unknown action, a key
//! name dsplr does not know, or one key for two actions is an error.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

/// A key as `config.toml` names it: `C-` (Ctrl), `A-` (Alt) and `S-` (Shift)
/// in front, then a character (`h`, `H`, `+`) or a key's name ([`NAMES`]).
/// Shift is part of a character (`H`, not `S-h`), so `S-` goes with names
/// only (`S-Tab`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySpec {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// A key: a character, or one with a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Named(Named),
}

/// The keys known by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Named {
    Tab,
    Escape,
    Return,
    BackSpace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    KpAdd,
    KpSubtract,
    /// `F1` to `F12`.
    F(u8),
}

/// The names [`KeySpec`] reads, besides `F1` to `F12`; `space` is the
/// character ` `.
pub const NAMES: [&str; 15] = [
    "Tab",
    "Escape",
    "Return",
    "BackSpace",
    "Delete",
    "Left",
    "Right",
    "Up",
    "Down",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "KP_Add",
    "KP_Subtract",
];

const NAMED: [Named; 15] = [
    Named::Tab,
    Named::Escape,
    Named::Return,
    Named::BackSpace,
    Named::Delete,
    Named::Left,
    Named::Right,
    Named::Up,
    Named::Down,
    Named::Home,
    Named::End,
    Named::PageUp,
    Named::PageDown,
    Named::KpAdd,
    Named::KpSubtract,
];

/// Why a key's name was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error(
        "{0:?} is not a key: write a character (h, H, +), \"space\", F1 to F12, or one of {names}, with C-, A- or S- in front",
        names = NAMES.join(", ")
    )]
    Unknown(String),
    #[error(
        "{0:?}: Shift is part of a character, so write the character itself ({1}) rather than S-"
    )]
    ShiftedChar(String, char),
}

impl FromStr for KeySpec {
    type Err = KeyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut spec = Self {
            key: Key::Char(' '),
            ctrl: false,
            alt: false,
            shift: false,
        };
        let mut rest = text;
        // A modifier prefix, unless the whole rest is a character (`-`).
        while rest.chars().count() > 2 {
            let flag = match rest.get(..2) {
                Some("C-") => &mut spec.ctrl,
                Some("A-") => &mut spec.alt,
                Some("S-") => &mut spec.shift,
                _ => break,
            };
            *flag = true;
            rest = &rest[2..];
        }
        let mut chars = rest.chars();
        spec.key = match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_control() && c != ' ' => Key::Char(c),
            _ if rest == "space" => Key::Char(' '),
            _ => match NAMES.iter().position(|name| *name == rest) {
                Some(index) => Key::Named(NAMED[index]),
                None => match rest.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
                    Some(n @ 1..=12) => Key::Named(Named::F(n)),
                    _ => return Err(KeyError::Unknown(text.to_owned())),
                },
            },
        };
        if let (Key::Char(c), true) = (spec.key, spec.shift) {
            let shifted = c.to_uppercase().next().unwrap_or(c);
            return Err(KeyError::ShiftedChar(text.to_owned(), shifted));
        }
        Ok(spec)
    }
}

/// The name [`KeySpec::from_str`] reads back.
impl fmt::Display for KeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, prefix) in [(self.ctrl, "C-"), (self.alt, "A-"), (self.shift, "S-")] {
            if on {
                f.write_str(prefix)?;
            }
        }
        match self.key {
            Key::Char(' ') => f.write_str("space"),
            Key::Char(c) => write!(f, "{c}"),
            Key::Named(Named::F(n)) => write!(f, "F{n}"),
            Key::Named(named) => {
                let index = NAMED
                    .iter()
                    .position(|n| *n == named)
                    .expect("every name is listed");
                f.write_str(NAMES[index])
            }
        }
    }
}

impl<'de> Deserialize<'de> for KeySpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// What a key can do in the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    SelectNext,
    SelectPrevious,
    NudgeLeft,
    NudgeDown,
    NudgeUp,
    NudgeRight,
    TouchLeft,
    TouchDown,
    TouchUp,
    TouchRight,
    RotateClockwise,
    RotateCounterclockwise,
    ScaleUp,
    ScaleDown,
    ToggleEnabled,
    Identify,
    Panel,
    Messages,
    Apply,
    Save,
    Undo,
    Redo,
    Quit,
}

macro_rules! keys {
    ($($field:ident: $action:ident = [$($key:literal),*],)*) => {
        /// The `[keys]` table: for each action, the keys that do it. Each
        /// field is the action's name in the file.
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(try_from = "RawKeys")]
        pub struct Keys {
            $(pub $field: Vec<KeySpec>,)*
            /// Default keys the file's own bindings took from the actions
            /// it leaves out, one line each, for the user to be told.
            pub warnings: Vec<String>,
        }

        /// [`Keys`] as the file gives it, before it is checked: `None` for
        /// an action it leaves out.
        #[derive(Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawKeys {
            $(#[serde(default)] $field: Option<Vec<KeySpec>>,)*
        }

        /// The plan's table.
        impl Default for Keys {
            fn default() -> Self {
                Self {
                    $($field: vec![$($key.parse().expect("a default key")),*],)*
                    warnings: Vec::new(),
                }
            }
        }

        impl Keys {
            /// Every key and what it does.
            pub fn bindings(&self) -> Vec<(KeySpec, Action)> {
                let mut bindings = Vec::new();
                $(bindings.extend(self.$field.iter().map(|key| (*key, Action::$action)));)*
                bindings
            }

            /// The keys for `action`.
            pub fn of(&self, action: Action) -> &[KeySpec] {
                match action {
                    $(Action::$action => &self.$field,)*
                }
            }
        }

        impl TryFrom<RawKeys> for Keys {
            type Error = String;

            /// The file's bindings, and the defaults of the actions it
            /// leaves out. One key the file binds to two actions is
            /// refused: which would win is not obvious from the file. A key
            /// the file binds that an action it leaves out has by default
            /// goes to the file's action, and that action keeps its other
            /// defaults (none left, it has no key), with a warning.
            fn try_from(raw: RawKeys) -> Result<Self, Self::Error> {
                let defaults = Self::default();
                let mut bound: HashMap<KeySpec, &'static str> = HashMap::new();
                $(for key in raw.$field.iter().flatten() {
                    if let Some(other) = bound.insert(*key, stringify!($field)) {
                        if other != stringify!($field) {
                            return Err(format!(
                                "the key {key} is bound to both {other} and {}",
                                stringify!($field)
                            ));
                        }
                    }
                })*
                let mut warnings = Vec::new();
                $(let $field = match raw.$field {
                    Some(keys) => keys,
                    None => {
                        let (kept, taken): (Vec<KeySpec>, Vec<KeySpec>) = defaults
                            .$field
                            .iter()
                            .partition(|key| !bound.contains_key(key));
                        for key in &taken {
                            warnings.push(format!(
                                "{key} is bound to {} here, so it no longer does {}{}",
                                bound[key],
                                stringify!($field),
                                if kept.is_empty() {
                                    ", which has no key now"
                                } else {
                                    ""
                                }
                            ));
                        }
                        kept
                    }
                };)*
                Ok(Self { $($field,)* warnings })
            }
        }
    };
}

keys! {
    select_next: SelectNext = ["Tab"],
    select_previous: SelectPrevious = ["S-Tab"],
    nudge_left: NudgeLeft = ["h"],
    nudge_down: NudgeDown = ["j"],
    nudge_up: NudgeUp = ["k"],
    nudge_right: NudgeRight = ["l"],
    touch_left: TouchLeft = ["H"],
    touch_down: TouchDown = ["J"],
    touch_up: TouchUp = ["K"],
    touch_right: TouchRight = ["L"],
    rotate_clockwise: RotateClockwise = ["r"],
    rotate_counterclockwise: RotateCounterclockwise = ["R"],
    scale_up: ScaleUp = ["+", "KP_Add"],
    scale_down: ScaleDown = ["-", "KP_Subtract"],
    toggle_enabled: ToggleEnabled = ["space"],
    identify: Identify = ["i"],
    panel: Panel = ["p", "F6"],
    messages: Messages = ["m"],
    apply: Apply = ["a"],
    save: Save = ["s"],
    undo: Undo = ["u"],
    redo: Redo = ["C-r"],
    quit: Quit = ["q", "Escape"],
}

#[cfg(test)]
mod tests {
    use super::{Key, KeyError, KeySpec, Keys, Named};

    fn spec(text: &str) -> KeySpec {
        text.parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn keys_read_as_a_character_or_a_name_with_modifiers() {
        assert_eq!(spec("h").key, Key::Char('h'));
        assert_eq!(spec("H").key, Key::Char('H'));
        assert_eq!(spec("+").key, Key::Char('+'));
        assert_eq!(spec("-").key, Key::Char('-'), "a dash alone is the key");
        assert_eq!(spec("space").key, Key::Char(' '));
        assert_eq!(spec("F12").key, Key::Named(Named::F(12)));
        let redo = spec("C-r");
        assert!(redo.ctrl && !redo.alt && !redo.shift);
        assert_eq!(redo.key, Key::Char('r'));
        let back = spec("S-Tab");
        assert!(back.shift);
        assert_eq!(back.key, Key::Named(Named::Tab));
        let both = spec("C-A-Left");
        assert!(both.ctrl && both.alt);
        assert_eq!(both.key, Key::Named(Named::Left));
        assert_eq!(spec("C--").key, Key::Char('-'), "Ctrl and a dash");
    }

    #[test]
    fn every_name_reads_back_as_written() {
        for text in [
            "h",
            "H",
            "+",
            "space",
            "C-r",
            "S-Tab",
            "A-F1",
            "KP_Add",
            "PageDown",
            "C-A-S-Escape",
        ] {
            assert_eq!(spec(text).to_string(), text);
        }
    }

    #[test]
    fn unknown_keys_and_shifted_characters_are_refused() {
        for text in ["", "Tabb", "F13", "F0", "ctrl-r", "X-r", "hh", " "] {
            let error = text.parse::<KeySpec>().unwrap_err();
            assert!(matches!(error, KeyError::Unknown(_)), "{text:?}: {error}");
            assert!(error.to_string().contains("KP_Subtract"), "{error}");
        }
        assert_eq!(
            "S-h".parse::<KeySpec>(),
            Err(KeyError::ShiftedChar("S-h".into(), 'H'))
        );
    }

    #[test]
    fn the_defaults_are_the_plan_s_table() {
        let keys = Keys::default();
        let names = |list: &[KeySpec]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(names(&keys.select_previous), ["S-Tab"]);
        assert_eq!(names(&keys.touch_down), ["J"]);
        assert_eq!(names(&keys.scale_up), ["+", "KP_Add"]);
        assert_eq!(names(&keys.toggle_enabled), ["space"]);
        assert_eq!(names(&keys.redo), ["C-r"]);
        assert_eq!(names(&keys.quit), ["q", "Escape"]);
        let bindings = keys.bindings();
        let distinct: std::collections::HashSet<_> = bindings.iter().map(|(key, _)| key).collect();
        assert_eq!(distinct.len(), bindings.len(), "no default key twice");
        assert_eq!(Keys::try_from_defaults(), Ok(keys));
    }

    impl Keys {
        /// The defaults, through the same check a file's keys get: a file
        /// that sets no action.
        fn try_from_defaults() -> Result<Self, String> {
            Self::try_from(super::RawKeys::default())
        }
    }
}
