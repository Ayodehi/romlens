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
    PreviewOptions,
    /// Name a local, note a routine, comment a statement or write a C version.
    CEdit,
}

/// What the C annotation sheet edits (docs/24, U10): the same annotations the
/// tutor makes. The macOS twin is `CEdit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CEdit {
    /// A local's name in one routine.
    Local { routine: u32, local: String },
    /// A note printed above the routine.
    Note { routine: u32 },
    /// A comment printed before the statement an instruction makes.
    Comment { address: u32 },
    /// The person's own C for a routine; `None` for a new one.
    Version { routine: u32, name: Option<String> },
}

/// View › Zoom In, Zoom Out and Zoom to Fit, for the graph or the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
    Fit,
}
