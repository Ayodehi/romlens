//! What a key or a menu item asks the document to do, independent of which
//! canvas sent it. The macOS twin is `EditorKeyCommand` and `EditorSource`.

/// Keys both canvases understand; the document interprets them per pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Back,
    ExtendLeft,
    ExtendRight,
    ExtendUp,
    ExtendDown,
    Follow,
    Rename,
    Comment,
    MarkCode,
    MarkData,
    MarkUnknown,
}

/// Which canvas a command came from; up and down mean rows in hex and lines
/// in the disassembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSource {
    Hex,
    Asm,
}

/// The sheets (dialogs) the document can ask the window to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    Jump,
    RenameLabel,
    Comment,
    Flags,
    Find,
    DataType,
    Variable,
}
