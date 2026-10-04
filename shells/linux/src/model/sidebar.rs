//! The sidebar's rows (docs/29): every view, grouped by the chip that owns
//! it, then the symbols. GTK-free, so which rows show (the filter, the closed
//! sections, the views that cannot open yet) is tested here; `sidebar.rs`
//! draws them. The macOS twin is `SidebarView`.

use std::collections::BTreeSet;

use romlens_ffi::{LabelInfo, RegionInfo, VariableInfo};

use super::audio;
use super::document::Document;
use super::graphics as gfx;
use super::navigator::{Bank, REGION_LIMIT};
use super::workspace::{CodeRep, EditorContent};

/// At most this many labels are listed; filtering finds the rest.
pub const LABEL_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Cartridge,
    Cpu,
    Ppu,
    Apu,
    Learn,
    Symbols,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Cartridge,
        Section::Cpu,
        Section::Ppu,
        Section::Apu,
        Section::Learn,
        Section::Symbols,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Section::Cartridge => "cartridge",
            Section::Cpu => "cpu",
            Section::Ppu => "ppu",
            Section::Apu => "apu",
            Section::Learn => "learn",
            Section::Symbols => "symbols",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Cartridge => "Cartridge",
            Section::Cpu => "CPU · 65816",
            Section::Ppu => "PPU · Picture",
            Section::Apu => "APU · Sound",
            Section::Learn => "Learn",
            Section::Symbols => "Symbols",
        }
    }

    /// The views it lists, in order (Symbols lists symbols instead).
    pub fn entries(self) -> Vec<Entry> {
        let code = |r| Entry::view(EditorContent::Code(r));
        match self {
            Section::Cartridge => vec![
                Entry::view(EditorContent::Atlas),
                Entry::view(EditorContent::Compare),
            ],
            Section::Cpu => vec![
                code(CodeRep::Assembly),
                code(CodeRep::C),
                code(CodeRep::Graph),
                code(CodeRep::Hex),
                code(CodeRep::Both),
                Entry::view(EditorContent::Source),
            ],
            Section::Ppu => [
                gfx::Tab::Frame,
                gfx::Tab::Layers,
                gfx::Tab::Tiles,
                gfx::Tab::Palette,
                gfx::Tab::Oam,
                gfx::Tab::Tilemap,
            ]
            .map(|t| Entry::view(EditorContent::Graphics(t)))
            .to_vec(),
            Section::Apu => [
                audio::Tab::Voices,
                audio::Tab::Timeline,
                audio::Tab::Samples,
                audio::Tab::Aram,
                audio::Tab::Ports,
                audio::Tab::Echo,
                audio::Tab::Scope,
            ]
            .map(|t| Entry::view(EditorContent::Audio(t)))
            .to_vec(),
            Section::Learn => vec![
                Entry::view(EditorContent::Tutor),
                Entry {
                    id: "lessons",
                    title: "Lessons and Quizzes",
                    content: None,
                },
            ],
            Section::Symbols => Vec::new(),
        }
    }
}

/// A view the sidebar offers: its tab's content, or (Lessons and Quizzes) a
/// place in the tutor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub id: &'static str,
    pub title: &'static str,
    pub content: Option<EditorContent>,
}

impl Entry {
    fn view(content: EditorContent) -> Self {
        Self {
            id: match content {
                EditorContent::Code(r) => r.raw(),
                EditorContent::Atlas => "atlas",
                EditorContent::Compare => "compare",
                EditorContent::Source => "source",
                EditorContent::Tutor => "tutor",
                EditorContent::Header => "header",
                EditorContent::Graphics(t) => t.id(),
                EditorContent::Audio(t) => t.id(),
            },
            title: content.title(),
            content: Some(content),
        }
    }

    /// The window action whose shortcut the selected row shows.
    pub fn action(self) -> Option<String> {
        Some(match self.content? {
            EditorContent::Code(r) => {
                let tab = super::layout::Tab::from_content(EditorContent::Code(r))?;
                format!("win.show-tab::{}", tab.id())
            }
            EditorContent::Atlas => "win.show-tab::atlas".into(),
            EditorContent::Graphics(t) => format!("win.show-{}", t.id()),
            EditorContent::Tutor => "win.show-tutor".into(),
            _ => return None,
        })
    }
}

/// The symbol lists under Symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbols {
    Labels,
    Variables,
    Regions,
    Banks,
}

impl Symbols {
    pub const ALL: [Symbols; 4] = [
        Symbols::Labels,
        Symbols::Variables,
        Symbols::Regions,
        Symbols::Banks,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Symbols::Labels => "labels",
            Symbols::Variables => "variables",
            Symbols::Regions => "regions",
            Symbols::Banks => "banks",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Symbols::Labels => "Labels",
            Symbols::Variables => "Variables",
            Symbols::Regions => "Regions",
            Symbols::Banks => "Banks",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Row {
    Section {
        section: Section,
        open: bool,
    },
    View {
        entry: Entry,
        /// The focused tab shows it.
        selected: bool,
        /// What it needs before it can open, if it cannot yet.
        reason: Option<&'static str>,
    },
    Group {
        symbols: Symbols,
        count: usize,
        open: bool,
    },
    Label(LabelInfo),
    Variable(VariableInfo),
    Region(RegionInfo),
    Bank(Bank),
    /// A line under a list: how much is left out, or what to do.
    Note(String),
    /// Variables' empty state: a Define Variable… button.
    DefineVariable,
}

/// The rows to show, for the navigator's filter (one field filters views and
/// symbols) and the ids of the closed sections and lists. While there is a
/// filter every section opens, and one with nothing matching goes.
pub fn rows(doc: &Document, collapsed: &BTreeSet<String>) -> Vec<Row> {
    let nav = doc.navigator();
    let query = nav.filter.trim().to_lowercase();
    let filtering = !query.is_empty();
    let focused = doc.focused_content();
    let open = |id: &str| filtering || !collapsed.contains(id);
    let mut out = Vec::new();
    for section in Section::ALL {
        if section == Section::Symbols {
            let lists = [
                (Symbols::Labels, nav.filtered_labels.len()),
                (Symbols::Variables, nav.filtered_variables.len()),
                (Symbols::Regions, nav.filtered_regions.len()),
                (Symbols::Banks, nav.data.banks.len()),
            ];
            let shown: Vec<_> = lists
                .into_iter()
                // Banks are not filtered; the rest go when nothing matches.
                .filter(|(s, n)| !filtering || (*s != Symbols::Banks && *n > 0))
                .collect();
            if shown.is_empty() {
                continue;
            }
            let section_open = open(section.id());
            out.push(Row::Section {
                section,
                open: section_open,
            });
            if !section_open {
                continue;
            }
            for (symbols, count) in shown {
                let list_open = open(symbols.id());
                out.push(Row::Group {
                    symbols,
                    count,
                    open: list_open,
                });
                if !list_open {
                    continue;
                }
                match symbols {
                    Symbols::Labels => {
                        out.extend(
                            nav.filtered_labels
                                .iter()
                                .take(LABEL_LIMIT)
                                .cloned()
                                .map(Row::Label),
                        );
                        if count > LABEL_LIMIT {
                            out.push(Row::Note(format!(
                                "{LABEL_LIMIT} of {count}; filter to find the rest"
                            )));
                        }
                    }
                    Symbols::Variables => {
                        if nav.data.variables.is_empty() {
                            out.push(Row::Note(
                                "None yet. Name a RAM address with Define Variable…".into(),
                            ));
                            out.push(Row::DefineVariable);
                        }
                        out.extend(nav.filtered_variables.iter().cloned().map(Row::Variable));
                    }
                    Symbols::Regions => {
                        if nav.data.regions_truncated {
                            out.push(Row::Note(format!("Largest {REGION_LIMIT} of each kind")));
                        }
                        out.extend(nav.filtered_regions.iter().cloned().map(Row::Region));
                    }
                    Symbols::Banks => out.extend(nav.data.banks.iter().cloned().map(Row::Bank)),
                }
            }
            continue;
        }
        let entries: Vec<Entry> = section
            .entries()
            .into_iter()
            .filter(|e| !filtering || e.title.to_lowercase().contains(&query))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let section_open = open(section.id());
        out.push(Row::Section {
            section,
            open: section_open,
        });
        if !section_open {
            continue;
        }
        for entry in entries {
            out.push(Row::View {
                entry,
                selected: entry.content.is_some() && entry.content == focused,
                reason: entry.content.and_then(|c| doc.unavailable_reason(c)),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::{TestRuntime, test_rom};

    fn views(rows: &[Row]) -> Vec<&'static str> {
        rows.iter()
            .filter_map(|r| match r {
                Row::View { entry, .. } => Some(entry.title),
                _ => None,
            })
            .collect()
    }

    fn sections(rows: &[Row]) -> Vec<&'static str> {
        rows.iter()
            .filter_map(|r| match r {
                Row::Section { section, .. } => Some(section.title()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_view_has_a_row() {
        let d = Document::new(test_rom(), TestRuntime::new());
        let rows = rows(&d, &BTreeSet::new());
        assert_eq!(
            &sections(&rows)[..5],
            [
                "Cartridge",
                "CPU · 65816",
                "PPU · Picture",
                "APU · Sound",
                "Learn"
            ]
        );
        let titles = views(&rows);
        for r in CodeRep::ALL {
            assert!(titles.contains(&r.view_title()), "{}", r.view_title());
        }
        for t in gfx::Tab::ALL {
            assert!(titles.contains(&t.title()), "{}", t.title());
        }
        for t in audio::Tab::ALL {
            assert!(titles.contains(&t.title()), "{}", t.title());
        }
        for t in ["Atlas", "Compare", "Source", "Tutor", "Lessons and Quizzes"] {
            assert!(titles.contains(&t), "{t}");
        }
        assert!(!titles.contains(&"Header and Vectors"), "not offered");
    }

    #[test]
    fn views_that_cannot_open_yet_say_why_and_the_focused_one_is_selected() {
        let d = Document::new(test_rom(), TestRuntime::new());
        let rows = rows(&d, &BTreeSet::new());
        let reason = |title: &str| {
            rows.iter().find_map(|r| match r {
                Row::View { entry, reason, .. } if entry.title == title => Some(*reason),
                _ => None,
            })
        };
        assert_eq!(reason("Disassembly"), Some(Some("analyzing")));
        assert_eq!(reason("Compare"), Some(Some("needs a ROM")));
        assert_eq!(reason("Source"), Some(Some("no sources")));
        assert_eq!(reason("Frame"), Some(Some("needs a recording")));
        assert_eq!(reason("Tile Decoder"), Some(None));
        assert_eq!(reason("Voices"), Some(None));
        let selected: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                Row::View {
                    entry,
                    selected: true,
                    ..
                } => Some(entry.title),
                _ => None,
            })
            .collect();
        assert_eq!(selected, ["Hex"], "a new window's tab");
    }

    #[test]
    fn a_closed_section_keeps_its_header_only_and_the_filter_opens_it() {
        let rt = TestRuntime::new();
        let d = Document::new(test_rom(), rt.clone());
        let closed = BTreeSet::from(["ppu".to_owned()]);
        let r = rows(&d, &closed);
        assert!(sections(&r).contains(&"PPU · Picture"));
        assert!(!views(&r).contains(&"Tile Decoder"));
        // A filter opens every section, and leaves out the ones with nothing
        // matching.
        d.set_nav_filter("decoder");
        rt.pump();
        let r = rows(&d, &closed);
        assert_eq!(views(&r), ["Tile Decoder"]);
        assert_eq!(sections(&r), ["PPU · Picture"]);
    }

    #[test]
    fn shortcuts_name_actions_that_have_them() {
        for section in Section::ALL {
            for entry in section.entries() {
                if let Some(action) = entry.action() {
                    // Every action a row names is one the window has.
                    assert!(crate::actions::knows(&action), "{action}");
                }
            }
        }
    }
}
