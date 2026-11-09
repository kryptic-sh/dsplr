//! What a key press does (`docs/plan/06-gui.md`'s key table). Keys are read
//! from the keysym, so a keymap's layout decides what a key is.
//!
//! Three readings, by what has the keyboard: the canvas ([`action`]), the
//! confirm dialogs and the quit prompt ([`answer`]), and the panel while a
//! field takes typing or a drop-down is open ([`nav`]).

use crcbl_shell::{Keysym, Modifiers};
use crcbl_ui::tree::{Direction as NavDirection, NavInput};
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
    /// Apply the pending edits through the confirm flow.
    Apply,
    /// Save the displays where the desktop reads them.
    Save,
    Quit,
}

/// `XK_ISO_Left_Tab`: what XKB keymaps make of Shift+Tab.
const ISO_LEFT_TAB: Keysym = Keysym(0xFE20);
/// `XK_KP_Enter`, `XK_KP_Add` and `XK_KP_Subtract`: the keypad's.
const KP_ENTER: Keysym = Keysym(0xFF8D);
const KP_ADD: Keysym = Keysym(0xFFAB);
const KP_SUBTRACT: Keysym = Keysym(0xFFAD);

/// The canvas action a press of `keysym` with `modifiers` held stands for.
pub fn action(keysym: Keysym, modifiers: Modifiers) -> Option<Action> {
    let modifiers = modifiers.chord();
    let ctrl = modifiers.contains(Modifiers::CTRL);
    let shift = modifiers.contains(Modifiers::SHIFT);
    match keysym {
        ISO_LEFT_TAB => return Some(Action::SelectPrevious),
        Keysym::TAB if shift => return Some(Action::SelectPrevious),
        Keysym::TAB => return Some(Action::SelectNext),
        Keysym::ESCAPE => return Some(Action::Quit),
        KP_ADD => return Some(Action::Scale { up: true }),
        KP_SUBTRACT => return Some(Action::Scale { up: false }),
        _ => {}
    }
    let c = keysym.to_char()?;
    if ctrl {
        return (c == 'r').then_some(Action::Redo);
    }
    Some(match c {
        'h' => Action::Nudge(Direction::Left),
        'j' => Action::Nudge(Direction::Down),
        'k' => Action::Nudge(Direction::Up),
        'l' => Action::Nudge(Direction::Right),
        'H' => Action::Touch(Direction::Left),
        'J' => Action::Touch(Direction::Down),
        'K' => Action::Touch(Direction::Up),
        'L' => Action::Touch(Direction::Right),
        'r' => Action::Rotate { clockwise: true },
        'R' => Action::Rotate { clockwise: false },
        '+' => Action::Scale { up: true },
        '-' => Action::Scale { up: false },
        ' ' => Action::ToggleEnabled,
        'a' => Action::Apply,
        's' => Action::Save,
        'u' => Action::Undo,
        'q' => Action::Quit,
        _ => return None,
    })
}

/// The answer to a yes-or-no: the quit prompt (`y` or Enter quits), the
/// confirm dialogs (Enter keeps, Escape reverts) and a question from the
/// backend thread (Enter replaces, Escape cancels). `n` and Escape are no.
pub fn answer(keysym: Keysym) -> Option<bool> {
    match keysym {
        Keysym::RETURN | KP_ENTER => Some(true),
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
        Keysym::RETURN | KP_ENTER => NavInput::ACCEPT,
        Keysym::ESCAPE => NavInput::BACK,
        Keysym::UP => NavInput::toward(NavDirection::Up),
        Keysym::DOWN => NavInput::toward(NavDirection::Down),
        ISO_LEFT_TAB => NavInput::PREV,
        Keysym::TAB if shift => NavInput::PREV,
        Keysym::TAB => NavInput::NEXT,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crcbl_shell::{Keysym, Modifiers};
    use crcbl_ui::tree::NavInput;
    use dsplr_core::Direction;

    use super::{Action, action, answer, nav};

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
            (key('a'), none, Action::Apply),
            (key('s'), none, Action::Save),
            (key('r'), Modifiers::CTRL, Action::Redo),
        ];
        for (keysym, modifiers, expected) in cases {
            assert_eq!(
                action(keysym, modifiers),
                Some(expected),
                "{keysym:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn other_keys_and_chords_do_nothing() {
        assert_eq!(action(key('q'), Modifiers::CTRL), None);
        assert_eq!(action(key('h'), Modifiers::CTRL), None);
        assert_eq!(action(key('x'), Modifiers::empty()), None);
        assert_eq!(action(Keysym::RETURN, Modifiers::empty()), None);
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
}
