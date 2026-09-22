//! What an emulator observed while a game actually ran.
//!
//! This is the one input that is not inference. Recursive descent reasons about
//! what *could* execute; a coverage log records what *did*, on the paths
//! someone played. It therefore outranks the disassembler in
//! `analysis::analyze` — observation beats inference — but never the user, who
//! outranks everything.
//!
//! Its second gift is subtler and matters more to the disassembler than the
//! coverage does. Mesen2 records the M and X widths at every executed opcode,
//! which is exactly what defeats static descent after a `PLP` or an `XCE` with
//! unknown carry. Those enter the walk as analyzer *hints* and never as user
//! `FlagOverride`s: they are an observation about one playthrough, not a
//! decision the user made, and they must not reach `flags.json` or the undo
//! stack.

/// A bit per ROM byte, in plain `u64` words. Five of these over a 3 MB image
/// come to under 2 MB, which is why coverage needs no new dependency and no
/// compression.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BitSet {
    words: Vec<u64>,
    len: u32,
}

impl BitSet {
    pub fn new(len: u32) -> Self {
        Self {
            words: vec![0; (len as usize).div_ceil(64)],
            len,
        }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, i: u32) -> bool {
        i < self.len && self.words[(i / 64) as usize] & (1 << (i % 64)) != 0
    }

    pub fn set(&mut self, i: u32, value: bool) {
        if i >= self.len {
            return;
        }
        let (w, bit) = ((i / 64) as usize, 1u64 << (i % 64));
        if value {
            self.words[w] |= bit;
        } else {
            self.words[w] &= !bit;
        }
    }

    pub fn insert(&mut self, i: u32) {
        self.set(i, true);
    }

    pub fn count(&self) -> u64 {
        self.words.iter().map(|w| w.count_ones() as u64).sum()
    }

    pub fn any(&self) -> bool {
        self.words.iter().any(|w| *w != 0)
    }

    /// Merge another set of the same length into this one.
    pub fn union(&mut self, other: &BitSet) {
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= b;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.len).filter(|i| self.get(*i))
    }
}

/// The M and X widths an emulator saw at each executed opcode.
///
/// Both are meaningful only where `Coverage::opcode_start` is set: a source
/// records the flags with the opcode fetch, and the value under an operand
/// byte is whatever the last fetch left there.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObservedFlags {
    /// The accumulator was 8-bit.
    pub m8: BitSet,
    /// The index registers were 8-bit.
    pub x8: BitSet,
    /// This source recorded widths at all. bsnes-plus does; a header-less CDL
    /// may not, and guessing "16-bit" from an absent record would be worse
    /// than leaving the descent to work it out.
    pub recorded: bool,
}

/// One imported trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// The ROM this describes, in bytes; every set is this long.
    pub len: u32,
    /// Fetched as an opcode or an operand.
    pub executed: BitSet,
    /// Read as data.
    pub read: BitSet,
    /// Written to. Rare in ROM space and interesting when it happens.
    pub written: BitSet,
    /// A subroutine entry, where the source distinguishes one.
    pub entry: BitSet,
    /// The first byte of an executed instruction.
    pub opcode_start: BitSet,
    pub flags: ObservedFlags,
}

impl Coverage {
    pub fn new(len: u32) -> Self {
        Self {
            len,
            executed: BitSet::new(len),
            read: BitSet::new(len),
            written: BitSet::new(len),
            entry: BitSet::new(len),
            opcode_start: BitSet::new(len),
            flags: ObservedFlags {
                m8: BitSet::new(len),
                x8: BitSet::new(len),
                recorded: false,
            },
        }
    }

    /// Mark an executed opcode, keeping `entry ⊆ opcode_start ⊆ executed`.
    ///
    /// The three sets are nested by definition — a subroutine entry is an
    /// opcode start, and an opcode start was executed — and every importer
    /// learns about them from different flags in a different order. Going
    /// through here is what stops one of them setting two of the three.
    pub fn mark_opcode(&mut self, offset: u32, entry: bool) {
        self.executed.insert(offset);
        self.opcode_start.insert(offset);
        if entry {
            self.entry.insert(offset);
        }
    }

    /// Fold another trace of the same ROM into this one. Two play sessions
    /// cover more than one, and nothing an emulator observed is ever wrong —
    /// only incomplete — so merging is a union throughout.
    pub fn union(&mut self, other: &Coverage) {
        if other.len != self.len {
            return;
        }
        // Widths are taken only where this trace recorded nothing, which is
        // why they are merged before `opcode_start` grows. A union would be
        // wrong: "was 8-bit" is recorded as a set bit and 16-bit as a clear
        // one, so OR-ing two sessions that disagree would silently claim 8-bit
        // for both. First trace wins, and a second only fills gaps.
        if other.flags.recorded {
            for i in other.opcode_start.iter() {
                if self.opcode_start.get(i) {
                    continue;
                }
                self.flags.m8.set(i, other.flags.m8.get(i));
                self.flags.x8.set(i, other.flags.x8.get(i));
            }
            self.flags.recorded = true;
        }
        self.executed.union(&other.executed);
        self.read.union(&other.read);
        self.written.union(&other.written);
        self.entry.union(&other.entry);
        self.opcode_start.union(&other.opcode_start);
    }

    pub fn is_empty(&self) -> bool {
        !self.executed.any() && !self.read.any() && !self.written.any()
    }

    /// Bytes fetched as code, as a fraction of the image.
    pub fn executed_fraction(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.executed.count() as f64 / self.len as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_set_count_and_merge() {
        let mut a = BitSet::new(200);
        assert!(!a.any());
        a.insert(0);
        a.insert(63);
        a.insert(64);
        a.insert(199);
        assert_eq!(a.count(), 4);
        assert!(a.get(64) && a.get(199) && !a.get(1));
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![0, 63, 64, 199]);
        // Out of range is ignored rather than panicking: a trace for a longer
        // ROM must not take the process down.
        a.insert(200);
        a.insert(u32::MAX);
        assert_eq!(a.count(), 4);
        assert!(!a.get(200));

        let mut b = BitSet::new(200);
        b.insert(1);
        a.union(&b);
        assert_eq!(a.count(), 5);
        a.set(0, false);
        assert_eq!(a.count(), 4);
    }

    /// Widths are not unioned: a session that saw 16-bit A at an opcode
    /// must not be overwritten by a later one that saw 8-bit there.
    #[test]
    fn the_first_trace_keeps_its_widths() {
        let mut first = Coverage::new(128);
        first.mark_opcode(4, false);
        first.flags.recorded = true; // 16-bit A at offset 4: m8 stays clear

        let mut second = Coverage::new(128);
        second.mark_opcode(4, false);
        second.flags.m8.insert(4);
        second.mark_opcode(8, false);
        second.flags.m8.insert(8);
        second.flags.recorded = true;

        first.union(&second);
        assert!(!first.flags.m8.get(4), "the first trace's width survived");
        assert!(
            first.flags.m8.get(8),
            "a gap the first trace left is filled"
        );
    }

    #[test]
    fn coverage_merges_two_sessions() {
        let mut first = Coverage::new(128);
        first.executed.insert(4);
        first.opcode_start.insert(4);
        let mut second = Coverage::new(128);
        second.executed.insert(8);
        second.read.insert(20);
        first.union(&second);
        assert_eq!(first.executed.count(), 2);
        assert_eq!(first.read.count(), 1);
        assert!(first.opcode_start.get(4));
        assert!(!first.is_empty());
        // A trace for a different ROM is refused rather than half-applied.
        let mut wrong = Coverage::new(64);
        wrong.executed.insert(0);
        first.union(&wrong);
        assert_eq!(first.executed.count(), 2);
    }
}
