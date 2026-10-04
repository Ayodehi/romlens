//! Open Quickly (docs/29): what a query finds, in order. An address first,
//! when the query resolves to one; then views by name; then up to 40 labels
//! by the navigator's own filter; then variables. GTK-free, so the order is
//! tested here; `quickview.rs` draws it. The macOS twin is
//! `OpenQuicklySheet`'s `results`.

use romlens_ffi::{LabelInfo, VariableInfo};

use super::document::Document;
use super::navigator::filter_labels;
use super::sidebar::{Entry, Section};

/// At most this many labels: a list longer than a screen is a filter away.
pub const LABEL_LIMIT: usize = 40;

#[derive(Debug, Clone)]
pub enum Hit {
    /// The query as an address: its file offset.
    Address {
        query: String,
        offset: u32,
    },
    View {
        entry: Entry,
        reason: Option<&'static str>,
    },
    Label(LabelInfo),
    Variable(VariableInfo),
}

impl Hit {
    pub fn title(&self) -> String {
        match self {
            Hit::Address { query, .. } => format!("Go to {query}"),
            Hit::View { entry, .. } => entry.title.to_owned(),
            Hit::Label(l) => l.name.clone(),
            Hit::Variable(v) => v.name.clone(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Hit::Address { offset, .. } => romlens_ffi::format_file_offset(*offset),
            Hit::View { reason, .. } => reason.unwrap_or("view").to_owned(),
            Hit::Label(l) => romlens_ffi::format_snes_address(l.address),
            Hit::Variable(v) => {
                format!("variable · {}", romlens_ffi::format_snes_address(v.address))
            }
        }
    }
}

pub fn results(doc: &Document, query: &str) -> Vec<Hit> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(r) = doc.preview_address(q) {
        out.push(Hit::Address {
            query: q.to_owned(),
            offset: r.file_offset,
        });
    }
    let lower = q.to_lowercase();
    for section in Section::ALL {
        for entry in section.entries() {
            let Some(content) = entry.content else {
                continue;
            };
            if entry.title.to_lowercase().contains(&lower) {
                out.push(Hit::View {
                    entry,
                    reason: doc.unavailable_reason(content),
                });
            }
        }
    }
    let nav = doc.navigator();
    out.extend(
        filter_labels(&nav.data.labels, q)
            .into_iter()
            .take(LABEL_LIMIT)
            .map(Hit::Label),
    );
    out.extend(
        nav.data
            .variables
            .iter()
            .filter(|v| v.name.to_lowercase().contains(&lower))
            .cloned()
            .map(Hit::Variable),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::TestRuntime;
    use std::rc::Rc;

    fn routines() -> (Rc<Document>, Rc<TestRuntime>) {
        let rt = TestRuntime::new();
        let rom =
            romlens_ffi::Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into())
                .unwrap();
        let d = Document::new(rom, rt.clone());
        d.start_analysis();
        rt.pump();
        (d, rt)
    }

    #[test]
    fn an_address_comes_first() {
        let (d, _) = routines();
        let hits = results(&d, "$00:8040");
        match hits.first() {
            Some(Hit::Address { offset, .. }) => assert_eq!(*offset, 0x40),
            other => panic!("{other:?}"),
        }
        assert_eq!(hits[0].title(), "Go to $00:8040");
    }

    #[test]
    fn views_and_labels_by_name_and_nothing_for_nothing() {
        let (d, _) = routines();
        let hits = results(&d, "tile");
        assert!(
            hits.iter()
                .any(|h| matches!(h, Hit::View { entry, .. } if entry.title == "Tile Decoder"))
        );
        let label = d.navigator().data.labels[0].name.clone();
        let hits = results(&d, &label);
        assert!(
            hits.iter()
                .any(|h| matches!(h, Hit::Label(l) if l.name == label))
        );
        assert!(results(&d, "  ").is_empty());
        // Lessons and Quizzes is the tutor's, not a view.
        assert!(
            !results(&d, "lessons")
                .iter()
                .any(|h| matches!(h, Hit::View { .. }))
        );
    }

    #[test]
    fn at_most_forty_labels() {
        let (d, _) = routines();
        let labels = results(&d, "_")
            .into_iter()
            .filter(|h| matches!(h, Hit::Label(_)))
            .count();
        assert!(labels <= LABEL_LIMIT);
    }
}
