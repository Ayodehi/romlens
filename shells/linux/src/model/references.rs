//! Find References' state: what was asked about, and every place that refers
//! to it. The inspector lists the first fifty references to the selection;
//! this is the whole list, which stays put while the selection moves through
//! it, so a reader can visit each caller in turn. The macOS twin is
//! `ReferencesModel`.

use romlens_ffi::{LabelInfo, LabelSource, Workbench};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file_offset: u32,
    pub snes_address: Option<u32>,
    pub kind_name: String,
    pub certain: bool,
    /// An emulator saw it happen (an execution log).
    pub observed: bool,
    /// The referring instruction, labels applied; empty when the reference is
    /// from data (a pointer or table entry).
    pub text: String,
    /// The routine the reference is in, as `SUB_8B8000+1C`.
    pub routine: Option<String>,
}

#[derive(Default)]
pub struct ReferencesModel {
    pub target: Option<u32>,
    /// The target's label, or its address when it has none.
    pub target_name: String,
    pub rows: Vec<Row>,
    pub current: Option<usize>,
}

impl ReferencesModel {
    pub fn has_results(&self) -> bool {
        self.target.is_some()
    }

    pub fn summary(&self) -> String {
        if self.target.is_none() {
            return String::new();
        }
        if self.rows.is_empty() {
            return "No references".into();
        }
        let position = self
            .current
            .map_or(String::new(), |c| format!("{} of ", c + 1));
        let n = self.rows.len();
        format!("{position}{n} reference{}", if n == 1 { "" } else { "s" })
    }

    pub fn find(&mut self, address: u32, workbench: &Workbench) {
        self.target = Some(address);
        self.target_name = workbench
            .label_at(address)
            .map_or_else(|| romlens_ffi::format_snes_address(address), |l| l.name);
        let routines = routine_labels(&workbench.labels());
        self.rows = workbench
            .xrefs_to(address)
            .into_iter()
            .map(|x| Row {
                file_offset: x.from_offset,
                snes_address: x.from_address,
                kind_name: x.kind_name,
                certain: x.certain,
                observed: x.observed,
                text: workbench
                    .instruction_at(x.from_offset)
                    .map_or_else(String::new, |i| i.text),
                routine: x
                    .from_address
                    .and_then(|a| routine_containing(a, &routines)),
            })
            .collect();
        self.current = None;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn select(&mut self, index: usize) -> Option<&Row> {
        if index >= self.rows.len() {
            return None;
        }
        self.current = Some(index);
        self.rows.get(index)
    }
}

/// The labels that can name a routine, by address. The analyzer's `CODE_`
/// labels mark branch and jump targets inside routines, and its `DATA_`,
/// `PTR_` and `JTBL_` labels are not code, so the nearest label of any kind
/// would usually name a loop rather than the routine.
pub fn routine_labels(labels: &[LabelInfo]) -> Vec<LabelInfo> {
    const INNER: [&str; 4] = ["CODE_", "DATA_", "PTR_", "JTBL_"];
    let mut v: Vec<LabelInfo> = labels
        .iter()
        .filter(|l| l.source != LabelSource::Auto || !INNER.iter().any(|p| l.name.starts_with(p)))
        .cloned()
        .collect();
    v.sort_by_key(|l| l.address);
    v
}

/// The nearest routine label at or before `address` in the same bank, with
/// the distance past it.
pub fn routine_containing(address: u32, sorted: &[LabelInfo]) -> Option<String> {
    let after = sorted.partition_point(|l| l.address <= address);
    let l = sorted.get(after.checked_sub(1)?)?;
    if l.address >> 16 != address >> 16 {
        return None;
    }
    let delta = address - l.address;
    Some(if delta == 0 {
        l.name.clone()
    } else {
        format!("{}+{delta:X}", l.name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(address: u32, name: &str, source: LabelSource) -> LabelInfo {
        LabelInfo {
            address,
            name: name.into(),
            source,
            origin: String::new(),
            file_offset: None,
        }
    }

    fn labels() -> Vec<LabelInfo> {
        routine_labels(&[
            label(0x80_9000, "SUB_809000", LabelSource::Auto),
            label(0x80_9010, "CODE_809010", LabelSource::Auto),
            label(0x80_8000, "Boot", LabelSource::User),
            label(0x80_8800, "DATA_808800", LabelSource::Auto),
            label(0x81_8000, "SUB_818000", LabelSource::Auto),
        ])
    }

    #[test]
    fn inner_auto_labels_do_not_name_routines() {
        let names: Vec<_> = labels().into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["Boot", "SUB_809000", "SUB_818000"]);
        // A person's label is kept whatever it is called.
        let kept = routine_labels(&[label(1, "CODE_mine", LabelSource::User)]);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn a_reference_is_placed_in_its_routine_with_the_distance() {
        let l = labels();
        assert_eq!(
            routine_containing(0x80_9000, &l).as_deref(),
            Some("SUB_809000")
        );
        assert_eq!(
            routine_containing(0x80_901C, &l).as_deref(),
            Some("SUB_809000+1C")
        );
        assert_eq!(
            routine_containing(0x80_8100, &l).as_deref(),
            Some("Boot+100")
        );
    }

    #[test]
    fn a_routine_in_another_bank_does_not_count() {
        let l = labels();
        assert_eq!(routine_containing(0x82_8000, &l), None);
        assert_eq!(routine_containing(0x80_7FFF, &l), None);
        assert_eq!(routine_containing(0x80_8000, &[]), None);
    }

    #[test]
    fn finds_what_calls_a_routine_in_the_test_rom() {
        let rom = crate::model::testing::test_rom();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let mut m = ReferencesModel::default();
        assert_eq!(m.summary(), "");
        // Whatever refers to the reset vector's target, found or not, the
        // model reports it in words.
        m.find(0x80_8000, &wb);
        assert!(m.has_results());
        assert!(!m.target_name.is_empty());
        assert!(m.summary().contains("reference"), "{}", m.summary());
        if !m.rows.is_empty() {
            assert_eq!(
                m.select(0).map(|r| r.file_offset),
                Some(m.rows[0].file_offset)
            );
        }
        assert!(m.select(m.rows.len()).is_none());
        m.clear();
        assert!(!m.has_results());
    }
}
