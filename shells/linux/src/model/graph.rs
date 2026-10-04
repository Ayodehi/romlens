//! The Graph tab's state (docs/19): the routine at the cursor as blocks, or
//! with its callers and callees. Like the C tab it follows the selection and
//! is rebuilt when the analysis or the names change; one build runs at a time,
//! and a newer request follows it, so a live session's analyses never keep it
//! from finishing. The macOS twin is `GraphModel`.

use std::collections::HashMap;

use romlens_ffi::{CallNeighbourhoodInfo, RoutineGraphInfo, Workbench};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphMode {
    Blocks,
    Calls,
}

impl GraphMode {
    pub const ALL: [GraphMode; 2] = [Self::Blocks, Self::Calls];

    pub fn title(self) -> &'static str {
        match self {
            Self::Blocks => "Blocks",
            Self::Calls => "Calls",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphState {
    Idle,
    Loading,
    Ready,
    /// The selection is not inside a routine the analysis found.
    NotInRoutine,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub entry: u32,
    pub mode: GraphMode,
    /// Changes with every analysis, rename or variable.
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub enum Built {
    Blocks(RoutineGraphInfo),
    Calls(CallNeighbourhoodInfo),
}

pub struct GraphModel {
    pub mode: GraphMode,
    pub state: GraphState,
    pub blocks: Option<RoutineGraphInfo>,
    pub calls: Option<CallNeighbourhoodInfo>,
    /// Bumped whenever `blocks` or `calls` changes.
    pub result_generation: u64,
    /// The routine shown (or being built), by entry address.
    pub entry: Option<u32>,
    block_by_offset: HashMap<u32, usize>,
    key: Option<Key>,
    shown: Option<Key>,
    running: bool,
}

impl Default for GraphModel {
    fn default() -> Self {
        Self {
            mode: GraphMode::Blocks,
            state: GraphState::Idle,
            blocks: None,
            calls: None,
            result_generation: 0,
            entry: None,
            block_by_offset: HashMap::new(),
            key: None,
            shown: None,
            running: false,
        }
    }
}

impl GraphModel {
    /// Show the routine containing `instruction_start`. Returns the run to
    /// start now, if one should.
    pub fn follow(
        &mut self,
        wb: &Workbench,
        instruction_start: Option<u32>,
        generation: u64,
    ) -> Option<Key> {
        let start = instruction_start?;
        // Still inside the routine shown.
        if let Some(k) = self.key
            && k.mode == self.mode
            && k.generation == generation
            && self.contains(start)
        {
            return None;
        }
        let Some(entry) = wb.function_containing(start) else {
            if !self.contains(start) {
                self.key = None;
                self.shown = None;
                self.entry = None;
                self.blocks = None;
                self.calls = None;
                self.block_by_offset.clear();
                self.result_generation += 1;
                self.state = GraphState::NotInRoutine;
            }
            return None;
        };
        let next = Key {
            entry,
            mode: self.mode,
            generation,
        };
        if Some(next) == self.key {
            return None;
        }
        self.key = Some(next);
        self.entry = Some(entry);
        let same = self
            .shown
            .is_some_and(|s| s.entry == entry && s.mode == self.mode);
        if !same || !self.has_result() {
            self.state = GraphState::Loading;
        }
        if self.running {
            None
        } else {
            self.running = true;
            Some(next)
        }
    }

    pub fn finish(&mut self, run: Key, outcome: Result<Built, String>) -> Option<Key> {
        self.running = false;
        let key = self.key?;
        if key.entry == run.entry && key.mode == run.mode {
            match outcome {
                Ok(built) => self.install(built, run),
                Err(e) if key == run => self.state = GraphState::Failed(e),
                Err(_) => {}
            }
        }
        if key != run {
            self.running = true;
            return Some(key);
        }
        None
    }

    /// Forget the key so the next `follow` builds again; what is shown stays
    /// until the new graph replaces it.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// The block holding the instruction at `offset`, in Blocks mode.
    pub fn block_containing(&self, offset: u32) -> Option<usize> {
        self.block_by_offset.get(&offset).copied()
    }

    fn has_result(&self) -> bool {
        match self.shown.map(|s| s.mode) {
            Some(GraphMode::Blocks) => self.blocks.is_some(),
            Some(GraphMode::Calls) => self.calls.is_some(),
            None => false,
        }
    }

    fn contains(&self, offset: u32) -> bool {
        match self.mode {
            GraphMode::Blocks => self.block_by_offset.contains_key(&offset),
            // The neighbourhood is the routine's; any instruction of it will
            // do, and the blocks are not built in this mode.
            GraphMode::Calls => false,
        }
    }

    fn install(&mut self, built: Built, run: Key) {
        self.shown = Some(run);
        match built {
            Built::Blocks(g) => {
                self.block_by_offset = g
                    .blocks
                    .iter()
                    .enumerate()
                    .flat_map(|(i, b)| b.offsets.iter().map(move |o| (*o, i)))
                    .collect();
                self.blocks = Some(g);
                self.calls = None;
            }
            Built::Calls(n) => {
                self.calls = Some(n);
                self.blocks = None;
                self.block_by_offset.clear();
            }
        }
        self.result_generation += 1;
        self.state = GraphState::Ready;
    }
}

/// Run a build: the core call for a key, off whatever thread the caller likes.
pub fn build(wb: &Workbench, run: Key) -> Result<Built, String> {
    match run.mode {
        GraphMode::Blocks => wb.routine_graph_blocking(run.entry).map(Built::Blocks),
        GraphMode::Calls => wb.call_neighbourhood_blocking(run.entry).map(Built::Calls),
    }
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::{Rom, make_routines_test_rom};

    fn routines() -> std::sync::Arc<Workbench> {
        let rom = Rom::from_bytes(make_routines_test_rom(), "r.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        wb
    }

    #[test]
    fn a_selection_builds_the_routines_blocks() {
        let wb = routines();
        let mut g = GraphModel::default();
        let run = g.follow(&wb, Some(0x22), 1).expect("a build starts");
        assert_eq!(
            (g.state.clone(), run.mode),
            (GraphState::Loading, GraphMode::Blocks)
        );
        assert!(g.finish(run, build(&wb, run)).is_none());
        assert_eq!(g.state, GraphState::Ready);
        let b = g.blocks.as_ref().unwrap();
        assert_eq!(b.name, "SUB_008020");
        assert!(g.block_containing(0x22).is_some());
        // Still inside the routine: nothing to do.
        assert!(g.follow(&wb, Some(0x22), 1).is_none());
    }

    #[test]
    fn calls_mode_builds_the_neighbourhood_instead() {
        let wb = routines();
        let mut g = GraphModel {
            mode: GraphMode::Calls,
            ..GraphModel::default()
        };
        let run = g.follow(&wb, Some(0x22), 1).unwrap();
        g.finish(run, build(&wb, run));
        assert!(g.blocks.is_none());
        let n = g.calls.as_ref().unwrap();
        assert_eq!(n.name, "SUB_008020");
        assert!(g.block_containing(0x22).is_none());
    }

    #[test]
    fn a_newer_request_follows_the_running_one() {
        let wb = routines();
        let mut g = GraphModel::default();
        let first = g.follow(&wb, Some(0x22), 1).unwrap();
        assert!(
            g.follow(&wb, Some(0x22), 2).is_none(),
            "one build at a time"
        );
        let next = g
            .finish(first, build(&wb, first))
            .expect("then the newer one");
        assert_eq!(next.generation, 2);
        assert!(g.finish(next, build(&wb, next)).is_none());
    }

    #[test]
    fn switching_mode_rebuilds_and_outside_a_routine_says_so() {
        let wb = routines();
        let mut g = GraphModel::default();
        let k = g.follow(&wb, Some(0x22), 1).unwrap();
        g.finish(k, build(&wb, k));
        g.mode = GraphMode::Calls;
        g.invalidate();
        let k = g.follow(&wb, Some(0x22), 1).unwrap();
        assert_eq!(g.state, GraphState::Loading);
        g.finish(k, build(&wb, k));
        assert!(g.calls.is_some());
        let empty = Workbench::new(crate::model::testing::test_rom());
        empty.analyze_blocking().unwrap();
        let mut g = GraphModel::default();
        assert!(g.follow(&empty, Some(0x4000), 1).is_none());
        assert_eq!(g.state, GraphState::NotInRoutine);
    }
}
