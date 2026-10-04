//! Which panes are open, with Focus on Code, and the text views by name.
//! Which tabs show is the workspace's (`workspace.rs`). Pure state, so the
//! rules are tested without a window. The macOS twin is the pane flags and
//! `toggleFocus` in `RomViewModel`.

use super::workspace::{CodeRep, EditorContent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Hex,
    Disassembly,
    Both,
    C,
    Graph,
    Source,
    Atlas,
    Compare,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Hex,
        Tab::Disassembly,
        Tab::Both,
        Tab::C,
        Tab::Graph,
        Tab::Source,
        Tab::Atlas,
        Tab::Compare,
    ];

    /// The tabs with a view behind them so far. Each joins as it is built.
    pub const BUILT: [Tab; 8] = Tab::ALL;

    pub fn id(self) -> &'static str {
        match self {
            Tab::Hex => "hex",
            Tab::Disassembly => "disassembly",
            Tab::Both => "both",
            Tab::C => "c",
            Tab::Graph => "graph",
            Tab::Source => "source",
            Tab::Atlas => "atlas",
            Tab::Compare => "compare",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Hex => "Hex",
            Tab::Disassembly => "Disassembly",
            Tab::Both => "Both",
            Tab::C => "C",
            Tab::Graph => "Graph",
            Tab::Source => "Source",
            Tab::Atlas => "Atlas",
            Tab::Compare => "Compare",
        }
    }

    pub fn from_id(id: &str) -> Option<Tab> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The content of this view's tab.
    pub fn content(self) -> EditorContent {
        match self {
            Tab::Hex => EditorContent::Code(CodeRep::Hex),
            Tab::Disassembly => EditorContent::Code(CodeRep::Assembly),
            Tab::Both => EditorContent::Code(CodeRep::Both),
            Tab::C => EditorContent::Code(CodeRep::C),
            Tab::Graph => EditorContent::Code(CodeRep::Graph),
            Tab::Source => EditorContent::Source,
            Tab::Atlas => EditorContent::Atlas,
            Tab::Compare => EditorContent::Compare,
        }
    }

    /// The text view a tab shows; `None` for graphics, sound and the tutor.
    pub fn from_content(content: EditorContent) -> Option<Tab> {
        Self::ALL.into_iter().find(|t| t.content() == content)
    }

    /// Tabs that only make sense with a finished analysis.
    pub fn needs_disassembly(self) -> bool {
        !matches!(self, Tab::Hex | Tab::Atlas | Tab::Compare)
    }
}

/// Which list the results pane shows: the last Find, or the last Find
/// References. Each keeps its own list, so switching loses neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsKind {
    Find,
    References,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panes {
    pub navigator: bool,
    pub inspector: bool,
    pub strip: bool,
    pub results: bool,
}

impl Default for Panes {
    fn default() -> Self {
        Self {
            navigator: true,
            inspector: true,
            strip: true,
            results: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub panes: Panes,
    pub results_kind: ResultsKind,
    /// What Focus on Code hid, to put back when it is turned off.
    unfocused: Option<Panes>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            panes: Panes::default(),
            results_kind: ResultsKind::Find,
            unfocused: None,
        }
    }
}

impl Layout {
    /// Only the editor showing: the sidebars, the strip and the results pane
    /// are hidden.
    pub fn is_focused(&self) -> bool {
        self.unfocused.is_some()
    }

    /// Focus on Code: hide everything but the editor, or put back exactly
    /// what it hid.
    pub fn toggle_focus(&mut self) {
        match self.unfocused.take() {
            Some(saved) => self.panes = saved,
            None => {
                self.unfocused = Some(self.panes);
                self.panes = Panes {
                    navigator: false,
                    inspector: false,
                    strip: false,
                    results: false,
                };
            }
        }
    }

    /// A pane toggled by hand while focused ends Focus: what the person
    /// chose now is the layout, and there is nothing to restore.
    pub fn set_pane(&mut self, pane: impl FnOnce(&mut Panes) -> &mut bool, shown: bool) {
        *pane(&mut self.panes) = shown;
        if shown {
            self.unfocused = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_hides_everything_and_restores_what_was_showing() {
        let mut l = Layout::default();
        l.panes.inspector = false;
        l.panes.results = true;
        let before = l.panes;
        l.toggle_focus();
        assert!(l.is_focused());
        assert_eq!(
            l.panes,
            Panes {
                navigator: false,
                inspector: false,
                strip: false,
                results: false
            }
        );
        l.toggle_focus();
        assert!(!l.is_focused());
        assert_eq!(l.panes, before);
    }

    #[test]
    fn opening_a_pane_while_focused_ends_focus() {
        let mut l = Layout::default();
        l.toggle_focus();
        l.set_pane(|p| &mut p.results, true);
        assert!(!l.is_focused());
        assert!(l.panes.results && !l.panes.navigator);
        // With nothing to restore, Focus hides again from here.
        l.toggle_focus();
        assert!(l.is_focused());
    }

    #[test]
    fn tab_ids_round_trip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_id(t.id()), Some(t));
        }
        assert_eq!(Tab::from_id("nope"), None);
        assert!(Tab::BUILT.iter().all(|t| Tab::ALL.contains(t)));
    }

    #[test]
    fn every_text_view_is_a_content_and_back() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_content(t.content()), Some(t));
        }
        assert_eq!(Tab::from_content(EditorContent::Tutor), None);
    }

    #[test]
    fn analysis_dependent_tabs() {
        assert!(!Tab::Hex.needs_disassembly());
        assert!(Tab::Disassembly.needs_disassembly());
        assert!(Tab::C.needs_disassembly());
        assert!(!Tab::Atlas.needs_disassembly());
    }
}
