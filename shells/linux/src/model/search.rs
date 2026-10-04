//! Ctrl+F's state: the query, the hits and which one is current. Kept beside
//! the document rather than inside the sheet because it outlives the sheet:
//! Ctrl+G steps through the results of a search made minutes ago, which is the
//! whole reason Find is not just a jump. The macOS twin is `SearchModel`.

use romlens_ffi::{RomlensError, SearchHit, SearchQuery, Workbench};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Bytes,
    Text,
}

/// Enough to fill the results list without paying for a whole ROM of matches
/// nobody scrolls to. The count says when it was reached.
pub const MAX_HITS: u32 = 500;
/// Bytes either side of a hit, matching `romlens search`.
pub const CONTEXT: u32 = 8;

#[derive(Default)]
pub struct SearchModel {
    pub query: String,
    pub mode: SearchMode,
    pub ignore_case: bool,
    pub hits: Vec<SearchHit>,
    pub current: Option<usize>,
    pub error: Option<String>,
    /// The query the current hits came from, so the list is never stale.
    pub searched: String,
}

impl SearchModel {
    pub fn has_results(&self) -> bool {
        !self.hits.is_empty()
    }

    pub fn reached_limit(&self) -> bool {
        self.hits.len() >= MAX_HITS as usize
    }

    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.searched.is_empty() {
            return String::new();
        }
        if self.hits.is_empty() {
            return "No matches".into();
        }
        let position = self
            .current
            .map_or(String::new(), |c| format!("{} of ", c + 1));
        let limit = if self.reached_limit() {
            format!(" (first {MAX_HITS})")
        } else {
            String::new()
        };
        let n = self.hits.len();
        format!(
            "{position}{n} match{}{limit}",
            if n == 1 { "" } else { "es" }
        )
    }

    pub fn search(&mut self, workbench: &Workbench) {
        let trimmed = self.query.trim().to_owned();
        if trimmed.is_empty() {
            self.clear();
            return;
        }
        let result = workbench.search(SearchQuery {
            pattern: trimmed.clone(),
            text: self.mode == SearchMode::Text,
            ignore_case: self.mode == SearchMode::Text && self.ignore_case,
            start: 0,
            len: u32::MAX,
            max: MAX_HITS,
            context: CONTEXT,
        });
        self.searched = trimmed;
        self.apply(result);
    }

    fn apply(&mut self, result: Result<Vec<SearchHit>, RomlensError>) {
        match result {
            Ok(hits) => {
                self.current = (!hits.is_empty()).then_some(0);
                self.hits = hits;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.hits.clear();
                self.current = None;
            }
        }
    }

    pub fn clear(&mut self) {
        self.hits.clear();
        self.current = None;
        self.error = None;
        self.searched.clear();
    }

    /// Ctrl+G and Ctrl+Shift+G. Wraps, because a search that stops at the end
    /// of the image makes a reader wonder whether it found everything.
    pub fn step(&mut self, delta: i64) -> Option<&SearchHit> {
        if self.hits.is_empty() {
            return None;
        }
        let n = self.hits.len() as i64;
        let from = self.current.map_or(-1, |c| c as i64);
        let next = (from + delta).rem_euclid(n) as usize;
        self.current = Some(next);
        self.hits.get(next)
    }

    pub fn select(&mut self, index: usize) -> Option<&SearchHit> {
        if index >= self.hits.len() {
            return None;
        }
        self.current = Some(index);
        self.hits.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(offset: u32) -> SearchHit {
        SearchHit {
            file_offset: offset,
            snes_address: None,
            len: 1,
            context: vec![],
            match_start: 0,
            region_kind: String::new(),
        }
    }

    fn with_hits(n: u32) -> SearchModel {
        let mut m = SearchModel {
            searched: "78".into(),
            ..SearchModel::default()
        };
        m.apply(Ok((0..n).map(hit).collect()));
        m
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut m = with_hits(3);
        assert_eq!(m.current, Some(0));
        assert_eq!(m.step(1).map(|h| h.file_offset), Some(1));
        m.step(1);
        assert_eq!(m.step(1).map(|h| h.file_offset), Some(0));
        assert_eq!(m.step(-1).map(|h| h.file_offset), Some(2));
        assert!(SearchModel::default().step(1).is_none());
    }

    #[test]
    fn select_checks_bounds() {
        let mut m = with_hits(2);
        assert_eq!(m.select(1).map(|h| h.file_offset), Some(1));
        assert!(m.select(2).is_none());
        assert_eq!(m.current, Some(1));
    }

    #[test]
    fn the_summary_says_where_you_are() {
        let mut m = SearchModel::default();
        assert_eq!(m.summary(), "");
        m.searched = "zz".into();
        assert_eq!(m.summary(), "No matches");
        let m = with_hits(1);
        assert_eq!(m.summary(), "1 of 1 match");
        let mut m = with_hits(4);
        m.step(1);
        assert_eq!(m.summary(), "2 of 4 matches");
        let m = with_hits(MAX_HITS);
        assert!(m.summary().ends_with("(first 500)"), "{}", m.summary());
    }

    #[test]
    fn an_error_replaces_the_hits_and_is_the_summary() {
        let mut m = with_hits(3);
        m.apply(Err(RomlensError::BadAddress {
            msg: "bad pattern".into(),
        }));
        assert!(m.hits.is_empty() && m.current.is_none());
        assert_eq!(m.summary(), "bad pattern");
    }

    #[test]
    fn searching_the_core_finds_bytes_and_text_and_reports_bad_hex() {
        let rom = crate::model::testing::test_rom();
        let wb = Workbench::new(rom);
        let mut m = SearchModel {
            query: "78 18 FB".into(),
            ..SearchModel::default()
        };
        m.search(&wb);
        assert_eq!(m.hits.first().map(|h| h.file_offset), Some(0));
        assert_eq!(m.current, Some(0));
        m.query = "78 ?? FB".into();
        m.search(&wb);
        assert!(m.has_results());
        m.query = "zz".into();
        m.search(&wb);
        assert!(m.error.is_some() && m.hits.is_empty());
        m.query = "   ".into();
        m.search(&wb);
        assert!(m.searched.is_empty() && m.error.is_none());
    }
}
