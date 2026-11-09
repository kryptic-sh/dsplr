//! What a key press does (`docs/plan/06-gui.md`'s key table, the part this
//! window has so far). Keys are read from the keysym, so a keymap's layout
//! decides what a key is.

use crcbl_shell::{Keysym, Modifiers};

/// What a key asks the window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SelectNext,
    SelectPrevious,
    Undo,
    Redo,
    Quit,
}

/// `XK_ISO_Left_Tab`: what XKB keymaps make of Shift+Tab.
const ISO_LEFT_TAB: Keysym = Keysym(0xFE20);

/// The action a press of `keysym` with `modifiers` held stands for.
pub fn action(keysym: Keysym, modifiers: Modifiers) -> Option<Action> {
    let modifiers = modifiers.chord();
    let ctrl = modifiers.contains(Modifiers::CTRL);
    let shift = modifiers.contains(Modifiers::SHIFT);
    match keysym {
        ISO_LEFT_TAB => return Some(Action::SelectPrevious),
        Keysym::TAB if shift => return Some(Action::SelectPrevious),
        Keysym::TAB => return Some(Action::SelectNext),
        Keysym::ESCAPE => return Some(Action::Quit),
        _ => {}
    }
    match (keysym.to_char()?, ctrl) {
        ('q', false) => Some(Action::Quit),
        ('u', false) => Some(Action::Undo),
        ('r', true) => Some(Action::Redo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crcbl_shell::{Keysym, Modifiers};

    use super::{Action, action};

    fn key(c: char) -> Keysym {
        Keysym::from_char(c)
    }

    #[test]
    fn keys_map_to_the_plan_s_actions() {
        let none = Modifiers::empty();
        assert_eq!(action(Keysym::TAB, none), Some(Action::SelectNext));
        assert_eq!(
            action(Keysym::TAB, Modifiers::SHIFT),
            Some(Action::SelectPrevious)
        );
        assert_eq!(
            action(Keysym(0xFE20), Modifiers::SHIFT),
            Some(Action::SelectPrevious),
            "Shift+Tab as XKB reports it"
        );
        assert_eq!(action(Keysym::ESCAPE, none), Some(Action::Quit));
        assert_eq!(action(key('q'), none), Some(Action::Quit));
        assert_eq!(action(key('u'), none), Some(Action::Undo));
        assert_eq!(action(key('r'), Modifiers::CTRL), Some(Action::Redo));
    }

    #[test]
    fn other_keys_and_chords_do_nothing() {
        assert_eq!(action(key('r'), Modifiers::empty()), None);
        assert_eq!(action(key('q'), Modifiers::CTRL), None);
        assert_eq!(action(key('x'), Modifiers::empty()), None);
        assert_eq!(action(Keysym::RETURN, Modifiers::empty()), None);
    }
}
