//! What a key press does (`docs/plan/06-gui.md`'s key table). Keys are read
//! from the keysym, so a keymap's layout decides what a key is: the
//! window passes each function the press's `shortcut_keysym`, the keysym
//! with Caps Lock left out (Shift kept), so `h` is `h` with Caps Lock on.
//!
//! Four readings, by what has the keyboard: the canvas ([`Keymap`], from
//! `config.toml`'s `[keys]`), the confirm dialogs, the quit prompt and a
//! question ([`answer`]), the panel while a field takes typing or a
//! drop-down is open ([`nav`]), and the panel the `panel` key moved the
//! keyboard into ([`panel_nav`]). Only the first is configurable: the
//! others are the keys every dialog and form takes.

use crcbl_shell::{Keysym, Modifiers};
use crcbl_ui::tree::{Direction as NavDirection, NavInput};
use dsplr_config::keys::{self as config, Key, KeySpec, Keys, Named};
use dsplr_core::Direction;

/// What a key asks the canvas to do, to the selected output where it acts
/// on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SelectNext,
    SelectPrevious,
    /// Move one snap step.
    Nudge(Direction),
    /// Move until touching the neighbour that way.
    Touch(Direction),
    Rotate {
        clockwise: bool,
    },
    Scale {
        up: bool,
    },
    ToggleEnabled,
    Undo,
    Redo,
    /// Show each output's name on it.
    Identify,
    /// Take the keyboard into the panel, or back out of it.
    Panel,
    /// Show or hide the backend's recent messages.
    Messages,
    /// Apply the pending edits through the confirm flow.
    Apply,
    /// Save the displays where the desktop reads them.
    Save,
    Quit,
}

impl From<config::Action> for Action {
    fn from(action: config::Action) -> Self {
        use config::Action as A;
        match action {
            A::SelectNext => Self::SelectNext,
            A::SelectPrevious => Self::SelectPrevious,
            A::NudgeLeft => Self::Nudge(Direction::Left),
            A::NudgeDown => Self::Nudge(Direction::Down),
            A::NudgeUp => Self::Nudge(Direction::Up),
            A::NudgeRight => Self::Nudge(Direction::Right),
            A::TouchLeft => Self::Touch(Direction::Left),
            A::TouchDown => Self::Touch(Direction::Down),
            A::TouchUp => Self::Touch(Direction::Up),
            A::TouchRight => Self::Touch(Direction::Right),
            A::RotateClockwise => Self::Rotate { clockwise: true },
            A::RotateCounterclockwise => Self::Rotate { clockwise: false },
            A::ScaleUp => Self::Scale { up: true },
            A::ScaleDown => Self::Scale { up: false },
            A::ToggleEnabled => Self::ToggleEnabled,
            A::Identify => Self::Identify,
            A::Panel => Self::Panel,
            A::Messages => Self::Messages,
            A::Apply => Self::Apply,
            A::Save => Self::Save,
            A::Undo => Self::Undo,
            A::Redo => Self::Redo,
            A::Quit => Self::Quit,
        }
    }
}

/// `XK_ISO_Left_Tab`: what XKB keymaps make of Shift+Tab.
const ISO_LEFT_TAB: Keysym = Keysym(0xFE20);

/// The canvas's keys, as `config.toml` binds them.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<(KeySpec, Action)>,
    /// The bar's line about the keys.
    hint: String,
    /// Default keys the config's own bindings took over
    /// ([`Keys::warnings`]).
    warnings: Vec<String>,
}

impl Keymap {
    pub fn new(keys: &Keys) -> Self {
        let bindings = keys
            .bindings()
            .into_iter()
            .map(|(key, action)| (key, action.into()))
            .collect();
        Self {
            bindings,
            hint: hint(keys),
            warnings: keys.warnings.clone(),
        }
    }

    /// Default keys the config's own bindings took over, one line each.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The action a press of `keysym` with `modifiers` held stands for.
    pub fn action(&self, keysym: Keysym, modifiers: Modifiers) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(key, _)| matches(key, keysym, modifiers))
            .map(|&(_, action)| action)
    }

    /// The bar's line about the keys: each action's first key.
    pub fn hint(&self) -> &str {
        &self.hint
    }
}

impl Default for Keymap {
    /// The plan's table.
    fn default() -> Self {
        Self::new(&Keys::default())
    }
}

/// Whether a press of `keysym` with `modifiers` is `key`. Shift is part of a
/// character (`H` is Shift+h), so it is compared for named keys only; Ctrl
/// and Alt for every key.
fn matches(key: &KeySpec, keysym: Keysym, modifiers: Modifiers) -> bool {
    let modifiers = modifiers.chord();
    let held = |modifier| modifiers.contains(modifier);
    if key.ctrl != held(Modifiers::CTRL) || key.alt != held(Modifiers::ALT) {
        return false;
    }
    match key.key {
        Key::Char(c) => keysym.to_char() == Some(c),
        // XKB keymaps make Shift+Tab a key of its own.
        Key::Named(Named::Tab) if key.shift => {
            keysym == ISO_LEFT_TAB || (keysym == Keysym::TAB && held(Modifiers::SHIFT))
        }
        Key::Named(named) => key.shift == held(Modifiers::SHIFT) && keysym == named_keysym(named),
    }
}

/// The keysym of a key named in `config.toml`.
fn named_keysym(named: Named) -> Keysym {
    match named {
        Named::Tab => Keysym::TAB,
        Named::Escape => Keysym::ESCAPE,
        Named::Return => Keysym::RETURN,
        Named::BackSpace => Keysym::BACKSPACE,
        Named::Delete => Keysym::DELETE,
        Named::Left => Keysym::LEFT,
        Named::Right => Keysym::RIGHT,
        Named::Up => Keysym::UP,
        Named::Down => Keysym::DOWN,
        Named::Home => Keysym::HOME,
        Named::End => Keysym::END,
        Named::PageUp => Keysym::PAGE_UP,
        Named::PageDown => Keysym::PAGE_DOWN,
        Named::KpAdd => Keysym::KP_ADD,
        Named::KpSubtract => Keysym::KP_SUBTRACT,
        // F1 to F12 are consecutive keysyms.
        Named::F(n) => Keysym(Keysym::F1.0 + u32::from(n) - 1),
    }
}

/// The bar's line about `keys`: the first key of each, or of each group.
fn hint(keys: &Keys) -> String {
    use config::Action as A;
    let first = |action| keys.of(action).first().map(ToString::to_string);
    let group = |actions: &[config::Action], what: &str| {
        let names: Vec<String> = actions.iter().filter_map(|&a| first(a)).collect();
        (!names.is_empty()).then(|| format!("{} {what}", names.join(" ")))
    };
    [
        group(
            &[A::NudgeLeft, A::NudgeDown, A::NudgeUp, A::NudgeRight],
            "move",
        )
        .map(|keys| format!("drag or {keys}")),
        group(&[A::RotateClockwise], "rotate"),
        group(&[A::ScaleUp, A::ScaleDown], "scale"),
        group(&[A::ToggleEnabled], "on/off"),
        group(&[A::Identify], "identify"),
        group(&[A::Panel], "panel"),
        group(&[A::Messages], "messages"),
        group(&[A::Apply], "apply"),
        group(&[A::Save], "save"),
        group(&[A::Undo, A::Redo], "undo / redo"),
        group(&[A::Quit], "quit"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ·  ")
}

/// The answer to a yes-or-no: the quit prompt (`y` or Enter quits), the
/// confirm dialogs (Enter keeps, Escape reverts) and a question from the
/// backend thread (Enter replaces, Escape cancels). `n` and Escape are no.
pub fn answer(keysym: Keysym) -> Option<bool> {
    match keysym {
        Keysym::RETURN | Keysym::KP_ENTER => Some(true),
        Keysym::ESCAPE => Some(false),
        _ => match keysym.to_char()? {
            'y' => Some(true),
            'n' => Some(false),
            _ => None,
        },
    }
}

/// The panel's navigation for a press of `keysym`, while a field takes
/// typing or a drop-down is open: Enter accepts, Escape backs out, the
/// arrows move through a drop-down's list, Tab and Shift+Tab move between
/// fields.
pub fn nav(keysym: Keysym, modifiers: Modifiers) -> Option<NavInput> {
    let shift = modifiers.chord().contains(Modifiers::SHIFT);
    Some(match keysym {
        Keysym::RETURN | Keysym::KP_ENTER => NavInput::ACCEPT,
        Keysym::ESCAPE => NavInput::BACK,
        Keysym::UP => NavInput::toward(NavDirection::Up),
        Keysym::DOWN => NavInput::toward(NavDirection::Down),
        ISO_LEFT_TAB => NavInput::PREV,
        Keysym::TAB if shift => NavInput::PREV,
        Keysym::TAB => NavInput::NEXT,
        _ => return None,
    })
}

/// The panel's navigation for a press of `keysym` once the `panel` key has
/// moved the keyboard into it and nothing in it is engaged: as [`nav`],
/// with Left and Right moving too, and space accepting as Enter does (a
/// button fires, a checkbox flips, a drop-down opens).
pub fn panel_nav(keysym: Keysym, modifiers: Modifiers) -> Option<NavInput> {
    nav(keysym, modifiers).or_else(|| match keysym {
        Keysym::LEFT => Some(NavInput::toward(NavDirection::Left)),
        Keysym::RIGHT => Some(NavInput::toward(NavDirection::Right)),
        _ if keysym.to_char() == Some(' ') => Some(NavInput::ACCEPT),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use crcbl_shell::{Keysym, Modifiers};
    use crcbl_ui::tree::NavInput;
    use dsplr_core::Direction;

    use super::{Action, Keymap, answer, nav};

    fn key(c: char) -> Keysym {
        Keysym::from_char(c)
    }

    #[test]
    fn keys_map_to_the_plan_s_actions() {
        let none = Modifiers::empty();
        let shift = Modifiers::SHIFT;
        let cases = [
            (Keysym::TAB, none, Action::SelectNext),
            (Keysym::TAB, shift, Action::SelectPrevious),
            (Keysym(0xFE20), shift, Action::SelectPrevious),
            (Keysym::ESCAPE, none, Action::Quit),
            (key('q'), none, Action::Quit),
            (key('h'), none, Action::Nudge(Direction::Left)),
            (key('j'), none, Action::Nudge(Direction::Down)),
            (key('k'), none, Action::Nudge(Direction::Up)),
            (key('l'), none, Action::Nudge(Direction::Right)),
            (key('H'), shift, Action::Touch(Direction::Left)),
            (key('J'), shift, Action::Touch(Direction::Down)),
            (key('K'), shift, Action::Touch(Direction::Up)),
            (key('L'), shift, Action::Touch(Direction::Right)),
            (key('r'), none, Action::Rotate { clockwise: true }),
            (key('R'), shift, Action::Rotate { clockwise: false }),
            (key('+'), shift, Action::Scale { up: true }),
            (key('-'), none, Action::Scale { up: false }),
            (Keysym(0xFFAB), none, Action::Scale { up: true }),
            (Keysym(0xFFAD), none, Action::Scale { up: false }),
            (key(' '), none, Action::ToggleEnabled),
            (key('u'), none, Action::Undo),
            (key('i'), none, Action::Identify),
            (key('a'), none, Action::Apply),
            (key('s'), none, Action::Save),
            (key('r'), Modifiers::CTRL, Action::Redo),
        ];
        for (keysym, modifiers, expected) in cases {
            assert_eq!(
                Keymap::default().action(keysym, modifiers),
                Some(expected),
                "{keysym:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn other_keys_and_chords_do_nothing() {
        assert_eq!(Keymap::default().action(key('q'), Modifiers::CTRL), None);
        assert_eq!(Keymap::default().action(key('h'), Modifiers::CTRL), None);
        assert_eq!(Keymap::default().action(key('x'), Modifiers::empty()), None);
        assert_eq!(
            Keymap::default().action(Keysym::RETURN, Modifiers::empty()),
            None
        );
    }

    #[test]
    fn the_prompt_takes_y_and_enter_or_n_and_escape() {
        assert_eq!(answer(key('y')), Some(true));
        assert_eq!(answer(Keysym::RETURN), Some(true));
        assert_eq!(answer(key('n')), Some(false));
        assert_eq!(answer(Keysym::ESCAPE), Some(false));
        assert_eq!(answer(key('q')), None, "q again does not answer");
    }

    #[test]
    fn the_panel_takes_enter_escape_arrows_and_tab() {
        let none = Modifiers::empty();
        assert_eq!(nav(Keysym::RETURN, none), Some(NavInput::ACCEPT));
        assert_eq!(nav(Keysym::ESCAPE, none), Some(NavInput::BACK));
        assert_eq!(nav(Keysym::TAB, Modifiers::SHIFT), Some(NavInput::PREV));
        assert!(nav(Keysym::DOWN, none).is_some_and(|nav| nav.direction.is_some()));
        assert_eq!(
            nav(key('h'), none),
            None,
            "a letter is typing, not navigation"
        );
    }

    #[test]
    fn in_the_panel_left_right_and_space_navigate_too_and_letters_do_not() {
        use super::panel_nav;
        let none = Modifiers::empty();
        assert_eq!(panel_nav(key(' '), none), Some(NavInput::ACCEPT));
        assert_eq!(panel_nav(Keysym::RETURN, none), Some(NavInput::ACCEPT));
        assert!(panel_nav(Keysym::LEFT, none).is_some_and(|nav| nav.direction.is_some()));
        assert!(panel_nav(Keysym::RIGHT, none).is_some_and(|nav| nav.direction.is_some()));
        assert_eq!(panel_nav(Keysym::TAB, none), Some(NavInput::NEXT));
        assert_eq!(panel_nav(key('h'), none), None);
        // While typing, Left, Right and space are the field's.
        assert_eq!(nav(Keysym::LEFT, none), None);
        assert_eq!(nav(key(' '), none), None);
    }

    #[test]
    fn a_rebound_key_replaces_the_default() {
        use dsplr_config::keys::Keys;

        let keys = Keys {
            nudge_left: vec!["Left".parse().unwrap(), "C-h".parse().unwrap()],
            ..Keys::default()
        };
        let keymap = Keymap::new(&keys);
        let none = Modifiers::empty();
        assert_eq!(
            keymap.action(Keysym::LEFT, none),
            Some(Action::Nudge(Direction::Left))
        );
        assert_eq!(
            keymap.action(key('h'), Modifiers::CTRL),
            Some(Action::Nudge(Direction::Left))
        );
        assert_eq!(keymap.action(key('h'), none), None, "h no longer moves");
        assert_eq!(
            keymap.action(Keysym::LEFT, Modifiers::SHIFT),
            None,
            "not S-Left"
        );
        assert!(
            keymap.hint().starts_with("drag or Left j k l move"),
            "{}",
            keymap.hint()
        );
    }

    #[test]
    fn the_default_hint_names_each_action_s_first_key() {
        assert_eq!(
            Keymap::default().hint(),
            "drag or h j k l move  ·  r rotate  ·  + - scale  ·  space on/off  ·  i identify  ·  p panel  ·  m messages  ·  a apply  ·  s save  ·  u C-r undo / redo  ·  q quit"
        );
    }
}
