//! The C tab's state: which routine it shows, at which level, and the
//! decompiled text. The routine follows the selection; the text is rebuilt
//! when the routine, the level or the analysis changes (docs/18). A state
//! machine with no threads in it: it says when a run should start, the
//! caller runs it, and `finish` says whether another is wanted. The macOS twin
//! is `DecompileModel`.

use std::collections::HashMap;

use romlens_ffi::{DecompileLevel, DecompiledInfo, NumberStyle, Workbench};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecompileState {
    Idle,
    Loading,
    Ready,
    /// The selection is not inside a routine the analysis found.
    NotInRoutine,
    Failed(String),
}

/// What a run is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub entry: u32,
    pub level: DecompileLevel,
    pub numbers: NumberStyle,
    /// Changes with every analysis, rename or variable.
    pub generation: u64,
}

pub struct Decompile {
    pub level: DecompileLevel,
    /// How the C prints numbers (addresses stay hex).
    pub numbers: NumberStyle,
    pub state: DecompileState,
    pub result: Option<DecompiledInfo>,
    /// Bumped whenever `result` changes, so views rebuild their text once.
    pub result_generation: u64,
    /// The routine shown (or being decompiled), by entry address.
    pub entry: Option<u32>,
    /// For each file offset an instruction starts at, the C lines it made.
    lines_by_offset: HashMap<u32, Vec<usize>>,
    key: Option<Key>,
    /// The key `result` was made for.
    shown: Option<Key>,
    running: bool,
}

impl Default for Decompile {
    fn default() -> Self {
        Self {
            level: DecompileLevel::Full,
            numbers: NumberStyle::Auto,
            state: DecompileState::Idle,
            result: None,
            result_generation: 0,
            entry: None,
            lines_by_offset: HashMap::new(),
            key: None,
            shown: None,
            running: false,
        }
    }
}

impl Decompile {
    /// Show the routine containing `instruction_start` (an instruction's file
    /// offset), if the key changed. Returns the run to start, if one should
    /// start now.
    ///
    /// One decompile runs at a time. A new key while one runs waits for it and
    /// follows it: cancelling it instead meant that a live session, which
    /// re-analyses every few seconds, could cancel every run of a large
    /// routine and the tab never left "Decompiling…".
    pub fn follow(
        &mut self,
        wb: &Workbench,
        instruction_start: Option<u32>,
        generation: u64,
    ) -> Option<Key> {
        let start = instruction_start?;
        // Still inside the routine shown: nothing to do.
        if let Some(k) = self.key
            && k.level == self.level
            && k.numbers == self.numbers
            && k.generation == generation
            && self.lines_by_offset.contains_key(&start)
        {
            return None;
        }
        let Some(entry) = wb.function_containing(start) else {
            if !self.lines_by_offset.contains_key(&start) {
                self.key = None;
                self.entry = None;
                self.shown = None;
                self.result = None;
                self.lines_by_offset.clear();
                self.result_generation += 1;
                self.state = DecompileState::NotInRoutine;
            }
            return None;
        };
        let next = Key {
            entry,
            level: self.level,
            numbers: self.numbers,
            generation,
        };
        if Some(next) == self.key {
            return None;
        }
        self.key = Some(next);
        self.entry = Some(entry);
        // The same routine at the same level again, as after an analysis: the
        // text shown stays up until the new one is ready.
        let same = self.shown.is_some_and(|s| {
            s.entry == entry && s.level == next.level && s.numbers == next.numbers
        });
        if !same || self.result.is_none() {
            self.state = DecompileState::Loading;
        }
        if self.running {
            None
        } else {
            self.running = true;
            Some(next)
        }
    }

    /// A run ended. Returns the next run if the key moved meanwhile.
    pub fn finish(&mut self, run: Key, outcome: Result<DecompiledInfo, String>) -> Option<Key> {
        self.running = false;
        let key = self.key?;
        // A result for the routine and level wanted is shown even if the
        // analysis moved on meanwhile; the next run brings it up to date.
        if key.entry == run.entry && key.level == run.level && key.numbers == run.numbers {
            match outcome {
                Ok(d) => self.install(d, run),
                Err(e) if key == run => self.state = DecompileState::Failed(e),
                Err(_) => {}
            }
        }
        if key != run {
            self.running = true;
            return Some(key);
        }
        None
    }

    /// Forget the key so the next `follow` decompiles again. The text shown
    /// and its line map stay until the new text replaces them.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    fn install(&mut self, d: DecompiledInfo, run: Key) {
        self.shown = Some(run);
        let mut map: HashMap<u32, Vec<usize>> = HashMap::new();
        for (line, offsets) in d.lines.iter().enumerate() {
            for o in offsets {
                map.entry(*o).or_default().push(line);
            }
        }
        self.lines_by_offset = map;
        self.result = Some(d);
        self.result_generation += 1;
        self.state = DecompileState::Ready;
    }

    /// The C lines the instruction at `offset` made.
    pub fn lines_for_instruction(&self, offset: u32) -> Vec<usize> {
        self.lines_by_offset
            .get(&offset)
            .cloned()
            .unwrap_or_default()
    }

    /// The instructions line `line` came from.
    pub fn offsets_for_line(&self, line: usize) -> Vec<u32> {
        self.result
            .as_ref()
            .and_then(|d| d.lines.get(line))
            .cloned()
            .unwrap_or_default()
    }

    /// The unit, for Export C…
    pub fn export_text(&self) -> Option<&str> {
        self.result.as_ref().map(|d| d.text.as_str())
    }
}

/// How the numbers button names each style, and what the config stores.
pub const NUMBER_STYLES: [(NumberStyle, &str, &str); 4] = [
    (NumberStyle::Auto, "Auto", "auto"),
    (NumberStyle::Hex, "Hex", "hex"),
    (NumberStyle::Decimal, "Dec", "decimal"),
    (NumberStyle::Binary, "Bin", "binary"),
];

pub const LEVELS: [(DecompileLevel, &str); 3] = [
    (DecompileLevel::Lift, "Lift"),
    (DecompileLevel::Clean, "Clean"),
    (DecompileLevel::Full, "Full"),
];

pub fn number_style_named(name: &str) -> Option<NumberStyle> {
    NUMBER_STYLES
        .iter()
        .find(|(_, _, n)| *n == name)
        .map(|(s, _, _)| *s)
}

pub fn number_style_name(style: NumberStyle) -> &'static str {
    NUMBER_STYLES
        .iter()
        .find(|(s, _, _)| *s == style)
        .map_or("auto", |(_, _, n)| n)
}

/// A C literal's value: `12`, `0x81`, `0b1000`.
pub fn literal_value(literal: &str) -> Option<u32> {
    let l = literal.to_lowercase();
    if let Some(h) = l.strip_prefix("0x") {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(b) = l.strip_prefix("0b") {
        u32::from_str_radix(b, 2).ok()
    } else {
        l.parse().ok()
    }
}

/// `129 = 0x81 = 0b10000001`.
pub fn bases(v: u32) -> String {
    [NumberStyle::Decimal, NumberStyle::Hex, NumberStyle::Binary]
        .iter()
        .map(|s| romlens_ffi::format_c_number(v, *s))
        .collect::<Vec<_>>()
        .join(" = ")
}

/// UTF-16 offsets (what the core reports) to char offsets (what a GTK text
/// buffer counts), for text with anything outside the BMP's one-unit chars.
pub fn utf16_to_chars(text: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.chars().enumerate() {
        for _ in 0..c.len_utf16() {
            map.push(i);
        }
    }
    map.push(text.chars().count());
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::test_rom;
    use romlens_ffi::{Rom, make_routines_test_rom};

    fn routines() -> std::sync::Arc<Workbench> {
        let rom = Rom::from_bytes(make_routines_test_rom(), "r.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        wb
    }

    fn run(wb: &Workbench, key: Key) -> Result<DecompiledInfo, String> {
        wb.decompile_blocking(key.entry, key.level)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn following_a_selection_starts_one_run_and_installs_its_text() {
        let wb = routines();
        let mut d = Decompile::default();
        let key = d.follow(&wb, Some(0x22), 1).expect("a run starts");
        assert_eq!(d.state, DecompileState::Loading);
        assert_eq!(key.entry, 0x00_8020);
        assert!(d.finish(key, run(&wb, key)).is_none());
        assert_eq!(d.state, DecompileState::Ready);
        let r = d.result.as_ref().unwrap();
        assert_eq!(r.name, "SUB_008020");
        assert!(r.text.contains("for ("), "{}", r.text);
        // Still inside the routine: nothing to do.
        assert!(d.follow(&wb, Some(0x22), 1).is_none());
        // Instructions map to lines and back.
        let lines = d.lines_for_instruction(0x22);
        assert!(!lines.is_empty());
        assert!(d.offsets_for_line(lines[0]).contains(&0x22));
    }

    #[test]
    fn a_new_key_while_one_runs_follows_it_instead_of_cancelling() {
        let wb = routines();
        let mut d = Decompile::default();
        let first = d.follow(&wb, Some(0x22), 1).unwrap();
        // The analysis moved on while the run was going: no second run yet.
        assert!(d.follow(&wb, Some(0x22), 2).is_none());
        let next = d
            .finish(first, run(&wb, first))
            .expect("the newer key runs next");
        assert_eq!(next.generation, 2);
        // The first result is shown meanwhile.
        assert_eq!(d.state, DecompileState::Ready);
        assert!(d.finish(next, run(&wb, next)).is_none());
    }

    #[test]
    fn changing_the_level_decompiles_again_and_keeps_the_text_until_then() {
        let wb = routines();
        let mut d = Decompile::default();
        let k = d.follow(&wb, Some(0x22), 1).unwrap();
        d.finish(k, run(&wb, k));
        let full = d.result.as_ref().unwrap().text.clone();
        d.level = DecompileLevel::Lift;
        d.invalidate();
        let k = d.follow(&wb, Some(0x22), 1).unwrap();
        assert_eq!(k.level, DecompileLevel::Lift);
        assert_eq!(
            d.state,
            DecompileState::Loading,
            "a different level shows a loading note"
        );
        d.finish(k, run(&wb, k));
        assert_ne!(d.result.as_ref().unwrap().text, full);
    }

    #[test]
    fn the_same_routine_after_an_analysis_keeps_its_text_up() {
        let wb = routines();
        let mut d = Decompile::default();
        let k = d.follow(&wb, Some(0x22), 1).unwrap();
        d.finish(k, run(&wb, k));
        d.invalidate();
        let again = d.follow(&wb, Some(0x22), 2).unwrap();
        assert_eq!(d.state, DecompileState::Ready, "no flicker to Loading");
        d.finish(again, run(&wb, again));
    }

    #[test]
    fn outside_every_routine_says_so() {
        let wb = Workbench::new(test_rom());
        wb.analyze_blocking().unwrap();
        let mut d = Decompile::default();
        // The test ROM's data area has no routine.
        assert!(d.follow(&wb, Some(0x4000), 1).is_none());
        assert_eq!(d.state, DecompileState::NotInRoutine);
        assert!(d.result.is_none());
        assert!(d.follow(&wb, None, 1).is_none());
    }

    #[test]
    fn a_failed_run_reports_its_message() {
        let wb = routines();
        let mut d = Decompile::default();
        let k = d.follow(&wb, Some(0x22), 1).unwrap();
        d.finish(k, Err("boom".into()));
        assert_eq!(d.state, DecompileState::Failed("boom".into()));
    }

    #[test]
    fn numbers_and_literals() {
        assert_eq!(literal_value("12"), Some(12));
        assert_eq!(literal_value("0x81"), Some(129));
        assert_eq!(literal_value("0b1000"), Some(8));
        assert_eq!(literal_value("0xZZ"), None);
        assert_eq!(bases(129), "129 = 0x81 = 0b10000001");
        for (style, _, name) in NUMBER_STYLES {
            assert_eq!(number_style_named(name), Some(style));
            assert_eq!(number_style_name(style), name);
        }
        assert_eq!(number_style_named("nope"), None);
    }

    #[test]
    fn utf16_offsets_map_to_chars() {
        let text = "a€𝄞b";
        let map = utf16_to_chars(text);
        // a (1 unit), € (1), 𝄞 (2 units, 1 char), b (1)
        assert_eq!(map, vec![0, 1, 2, 2, 3, 4]);
    }
}
