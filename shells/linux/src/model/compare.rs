//! The Compare tab's state (docs/22, D2): another version of the ROM,
//! analysed beside this one, and what changed from it to this one. The other
//! version is `a` throughout, this one `b`. The macOS twin is `CompareModel`.

use std::sync::Arc;

use romlens_ffi::Workbench;
use romlens_ffi::compare::{
    ByteRunInfo, ByteRunKind, ComparisonInfo, DataChangeInfo, DiffLineOp, MovedBlockInfo,
    RoutinePairInfo, RoutinePairing, RoutineSideInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareState {
    Idle,
    Loading(String),
    Ready,
    Failed(String),
}

/// A row of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Routine(usize),
    Data(usize),
    Run(usize),
    Move(usize),
}

/// Load the other version: a ROM file, or a project folder with its ROM found
/// as opening it would find it. Slow (reads, hashes and analyses), so it runs
/// off the main thread.
pub fn load_other(path: &std::path::Path) -> Result<Arc<Workbench>, String> {
    let wb = if crate::package::is_package(path) {
        let shown = |e: romlens_ffi::RomlensError| format!("{}: {e}", path.display());
        let files =
            romlens_ffi::workbench::read_project_package(path.to_string_lossy().into_owned())
                .map_err(shown)?;
        let (files, local) = crate::package::split_local(files);
        let identity = romlens_ffi::workbench::project_identity(files.clone()).map_err(shown)?;
        let rom_path = crate::locator::locate(&identity, path, &local).ok_or_else(|| {
            format!(
                "The ROM {} was made from could not be found.",
                path.display()
            )
        })?;
        let rom = romlens_ffi::Rom::open(rom_path.to_string_lossy().into_owned()).map_err(shown)?;
        Workbench::with_project_files(rom, files).map_err(shown)?
    } else {
        let rom = romlens_ffi::Rom::open(path.to_string_lossy().into_owned())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Workbench::new(rom)
    };
    wb.analyze_blocking().map_err(|e| e.to_string())?;
    Ok(wb)
}

/// The first thousand stretches are listed; the rest are counted.
pub const RUN_LIMIT: usize = 1000;

pub struct CompareModel {
    pub state: CompareState,
    /// The other version's file name, without its extension.
    pub other_name: Option<String>,
    pub info: Option<ComparisonInfo>,
    pub selected: Option<Item>,
    pub other: Option<Arc<Workbench>>,
    /// The analysis generation the comparison was made at.
    pub compared_generation: Option<u64>,
    ticket: u64,
    /// Bumped whenever the list's rows change, so a view rebuilds them only
    /// then and not on every selection.
    pub revision: u64,
}

impl Default for CompareModel {
    fn default() -> Self {
        Self {
            state: CompareState::Idle,
            other_name: None,
            info: None,
            selected: None,
            other: None,
            compared_generation: None,
            ticket: 0,
            revision: 0,
        }
    }
}

impl CompareModel {
    pub fn is_active(&self) -> bool {
        self.state != CompareState::Idle
    }

    /// A comparison with `name` was asked for. Whatever was shown before is
    /// dropped; the returned ticket tells this run's results from an older
    /// one's.
    pub fn begin(&mut self, name: &str) -> u64 {
        self.ticket += 1;
        self.other = None;
        self.other_name = Some(name.to_owned());
        self.info = None;
        self.selected = None;
        self.compared_generation = None;
        self.revision += 1;
        self.state = CompareState::Loading(format!("Analysing {name}…"));
        self.ticket
    }

    /// The other version is loaded, analysed and about to be compared.
    pub fn loaded(&mut self, other: Arc<Workbench>) {
        self.other = Some(other);
    }

    pub fn current_ticket(&self) -> u64 {
        self.ticket
    }

    /// Whether a run begun with `ticket` is still the one wanted.
    pub fn is_current(&self, ticket: u64) -> bool {
        self.ticket == ticket && self.is_active()
    }

    pub fn comparing(&mut self) {
        self.state = CompareState::Loading("Comparing…".into());
    }

    pub fn finished(&mut self, result: Result<ComparisonInfo, String>, generation: u64) {
        match result {
            Ok(info) => {
                self.info = Some(info);
                self.compared_generation = Some(generation);
                self.revision += 1;
                self.state = CompareState::Ready;
            }
            Err(e) => {
                self.revision += 1;
                self.state = CompareState::Failed(e);
            }
        }
    }

    /// Whether this version's analysis or names changed since the comparison.
    pub fn needs_refresh(&self, generation: u64) -> bool {
        self.state == CompareState::Ready && self.compared_generation != Some(generation)
    }

    pub fn refreshed(&mut self, info: Option<ComparisonInfo>, generation: u64) {
        self.compared_generation = Some(generation);
        if let Some(info) = info {
            self.info = Some(info);
            self.revision += 1;
        }
    }

    /// Close the comparison. Runs still going find their ticket stale.
    pub fn close(&mut self) {
        let (ticket, revision) = (self.ticket + 1, self.revision + 1);
        *self = Self::default();
        self.ticket = ticket;
        self.revision = revision;
    }

    pub fn routines(&self) -> &[RoutinePairInfo] {
        self.info.as_ref().map_or(&[], |i| &i.routines)
    }

    pub fn data(&self) -> &[DataChangeInfo] {
        self.info.as_ref().map_or(&[], |i| &i.data)
    }

    pub fn runs(&self) -> &[ByteRunInfo] {
        let all = self.info.as_ref().map_or(&[][..], |i| &i.runs);
        &all[..all.len().min(RUN_LIMIT)]
    }

    pub fn moves(&self) -> &[MovedBlockInfo] {
        self.info.as_ref().map_or(&[], |i| &i.moves)
    }
}

pub fn pairing_title(p: RoutinePairing) -> &'static str {
    match p {
        RoutinePairing::Same => "same",
        RoutinePairing::Moved => "moved",
        RoutinePairing::Changed => "changed",
        RoutinePairing::Added => "added",
        RoutinePairing::Removed => "removed",
    }
}

pub fn run_title(k: ByteRunKind) -> &'static str {
    match k {
        ByteRunKind::Changed => "changed",
        ByteRunKind::Inserted => "inserted",
        ByteRunKind::Deleted => "deleted",
    }
}

pub fn bytes(n: u64) -> String {
    format!("{n} byte{}", if n == 1 { "" } else { "s" })
}

/// Where a routine pair sits: one address, or from the other version's to
/// this one's.
pub fn routine_where(r: &RoutinePairInfo) -> String {
    let at = romlens_ffi::format_snes_address;
    match (&r.a, &r.b) {
        (Some(a), Some(b)) if a.entry == b.entry => at(b.entry),
        (Some(a), Some(b)) => format!("{} → {}", at(a.entry), at(b.entry)),
        (Some(a), None) => format!("{}, only in the other", at(a.entry)),
        (None, Some(b)) => format!("{}, only in this one", at(b.entry)),
        (None, None) => String::new(),
    }
}

/// What a routine pair came to, in a line.
pub fn pair_summary(r: &RoutinePairInfo) -> String {
    let changed = r.lines.iter().filter(|l| l.op != DiffLineOp::Same).count();
    let at = |side: &Option<RoutineSideInfo>| {
        side.as_ref()
            .map_or_else(String::new, |s| romlens_ffi::format_snes_address(s.entry))
    };
    match r.pairing {
        RoutinePairing::Changed => format!("{changed} of {} lines differ", r.lines.len()),
        RoutinePairing::Moved => format!("The same instructions, moved from {}", at(&r.a)),
        RoutinePairing::Added => format!(
            "Only in this version: {} instructions",
            r.b.as_ref().map_or(0, |b| b.instructions)
        ),
        RoutinePairing::Removed => format!("Only in the other version, at {}", at(&r.a)),
        RoutinePairing::Same => "The same".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::{Rom, make_compare_test_roms};

    fn pair() -> (Arc<Workbench>, Arc<Workbench>) {
        let roms = make_compare_test_roms();
        let make = |bytes: Vec<u8>| {
            let wb = Workbench::new(Rom::from_bytes(bytes, "v.sfc".into()).unwrap());
            wb.analyze_blocking().unwrap();
            wb
        };
        (make(roms[0].clone()), make(roms[1].clone()))
    }

    #[test]
    fn a_comparison_goes_idle_loading_ready_and_back() {
        let (a, b) = pair();
        let mut m = CompareModel::default();
        assert!(!m.is_active());
        let ticket = m.begin("version-1");
        m.loaded(Arc::clone(&a));
        assert_eq!(
            m.state,
            CompareState::Loading("Analysing version-1…".into())
        );
        assert!(m.is_active() && m.is_current(ticket));
        m.comparing();
        let info = b.compare_with_blocking(a).map_err(|e| e.to_string());
        m.finished(info, 3);
        assert_eq!(m.state, CompareState::Ready);
        assert!(!m.routines().is_empty());
        assert!(!m.needs_refresh(3));
        assert!(m.needs_refresh(4), "this version's analysis moved on");
        m.refreshed(None, 4);
        assert!(!m.needs_refresh(4));
        m.close();
        assert!(!m.is_current(ticket), "a run begun before Close is stale");
        assert_eq!(m.state, CompareState::Idle);
        assert!(m.info.is_none() && m.other_name.is_none());
    }

    #[test]
    fn a_failure_is_kept_as_its_message() {
        let mut m = CompareModel::default();
        m.finished(Err("boom".into()), 1);
        assert_eq!(m.state, CompareState::Failed("boom".into()));
        assert!(!m.needs_refresh(1));
    }

    #[test]
    fn the_fixture_has_a_changed_routine_with_aligned_lines_and_a_carry_over() {
        let (a, b) = pair();
        let info = b.compare_with_blocking(a).unwrap();
        let changed = info
            .routines
            .iter()
            .find(|r| r.pairing == RoutinePairing::Changed)
            .expect("a changed routine");
        assert!(!changed.lines.is_empty());
        assert!(pair_summary(changed).contains("lines differ"));
        assert!(routine_where(changed).contains('$'));
        assert!(info.changed_bytes + info.inserted_bytes + info.deleted_bytes > 0);
    }

    #[test]
    fn wording_agrees_with_counts() {
        assert_eq!(bytes(1), "1 byte");
        assert_eq!(bytes(2), "2 bytes");
        assert_eq!(pairing_title(RoutinePairing::Moved), "moved");
        assert_eq!(run_title(ByteRunKind::Inserted), "inserted");
    }

    #[test]
    fn long_runs_are_cut_at_a_thousand() {
        let (a, b) = pair();
        let mut info = b.compare_with_blocking(a).unwrap();
        let one = info.runs.first().cloned().expect("a run");
        info.runs = vec![one; RUN_LIMIT + 5];
        let mut m = CompareModel::default();
        m.finished(Ok(info), 1);
        assert_eq!(m.runs().len(), RUN_LIMIT);
    }
}
