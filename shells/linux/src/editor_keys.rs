//! The key map both canvases share. The macOS twin is
//! `EditorKeyCommand.from(event:)`.

use gtk::gdk::{Key, ModifierType};

use crate::model::EditorCommand;

/// The command for a key press, or `None` for keys the canvas does not own.
/// Letter keys only count without Ctrl, Alt or Super, so the window's own
/// shortcuts (Ctrl+L, Ctrl+F, ...) are never swallowed.
pub fn command_for(key: Key, state: ModifierType) -> Option<EditorCommand> {
    use EditorCommand::*;
    let shift = state.contains(ModifierType::SHIFT_MASK);
    let plain = !state
        .intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK);
    match key {
        Key::Left | Key::KP_Left => return Some(if shift { ExtendLeft } else { Left }),
        Key::Right | Key::KP_Right => return Some(if shift { ExtendRight } else { Right }),
        Key::Up | Key::KP_Up => return Some(if shift { ExtendUp } else { Up }),
        Key::Down | Key::KP_Down => return Some(if shift { ExtendDown } else { Down }),
        Key::Page_Up | Key::KP_Page_Up => return Some(PageUp),
        Key::Page_Down | Key::KP_Page_Down => return Some(PageDown),
        Key::Home | Key::KP_Home => return Some(Home),
        Key::End | Key::KP_End => return Some(End),
        Key::BackSpace | Key::Delete => return Some(Back),
        Key::Return | Key::KP_Enter | Key::ISO_Enter => return Some(Follow),
        _ => {}
    }
    if !plain {
        return None;
    }
    match key.to_unicode()? {
        'g' => Some(Follow),
        'n' => Some(Rename),
        ';' => Some(Comment),
        'c' => Some(MarkCode),
        'd' => Some(MarkData),
        'u' => Some(MarkUnknown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_extend_with_shift() {
        let none = ModifierType::empty();
        assert_eq!(command_for(Key::Left, none), Some(EditorCommand::Left));
        assert_eq!(
            command_for(Key::Down, ModifierType::SHIFT_MASK),
            Some(EditorCommand::ExtendDown)
        );
        assert_eq!(command_for(Key::Page_Up, none), Some(EditorCommand::PageUp));
    }

    #[test]
    fn letters_are_commands_only_unmodified() {
        let none = ModifierType::empty();
        assert_eq!(command_for(Key::g, none), Some(EditorCommand::Follow));
        assert_eq!(command_for(Key::n, none), Some(EditorCommand::Rename));
        assert_eq!(
            command_for(Key::semicolon, none),
            Some(EditorCommand::Comment)
        );
        assert_eq!(command_for(Key::c, none), Some(EditorCommand::MarkCode));
        assert_eq!(command_for(Key::d, none), Some(EditorCommand::MarkData));
        assert_eq!(command_for(Key::u, none), Some(EditorCommand::MarkUnknown));
        // Ctrl+C is Copy, not Mark as Code; Ctrl+G is Find Next.
        assert_eq!(command_for(Key::c, ModifierType::CONTROL_MASK), None);
        assert_eq!(command_for(Key::g, ModifierType::CONTROL_MASK), None);
        assert_eq!(command_for(Key::d, ModifierType::ALT_MASK), None);
        assert_eq!(command_for(Key::x, none), None);
    }

    #[test]
    fn return_and_delete_keys() {
        let none = ModifierType::empty();
        assert_eq!(command_for(Key::Return, none), Some(EditorCommand::Follow));
        assert_eq!(command_for(Key::BackSpace, none), Some(EditorCommand::Back));
        assert_eq!(command_for(Key::Home, none), Some(EditorCommand::Home));
    }
}
