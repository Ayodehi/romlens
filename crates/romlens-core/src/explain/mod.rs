//! Explanations for a student (docs/20): what a write to a hardware
//! register does, field by field, and the common SNES sequences named where
//! they appear.
//!
//! [`Explanations::build`] runs over every routine the analysis found, once;
//! the listing, the inspector, the C and the CLI all read from it, so they
//! say the same thing.

pub mod fields;
pub mod values;

use std::collections::HashMap;

pub use fields::{FieldRow, Part, RegisterWrite, describe};
pub use values::{Byte, State, Store};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::SymbolLookup;
use crate::decompile::cfg::Cfg;
use crate::decompile::function::{self, Function};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::model::symbols::Symbols;
use crate::rom::image::RomImage;

/// A store to the hardware, explained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    pub offset: FileOffset,
    pub write: RegisterWrite,
    /// Where an unknown value was loaded from.
    pub source: Option<SnesAddress>,
    /// The source's name, when the project or the analysis gave it one.
    pub source_name: Option<String>,
    /// Through an index the pass does not know: `write` is the base
    /// register only.
    pub indexed: bool,
}

impl Explained {
    /// The comment for the listing: `NMITIMEN = $81: NMI on, joypad
    /// auto-read on`, or `INIDISP ← $7E:0DAE` when only the source is known.
    pub fn short(&self) -> String {
        if self.write.value.is_some() {
            return self.write.short();
        }
        let names: Vec<&str> = self.write.parts.iter().map(|p| p.name.as_str()).collect();
        let name = names.join("/");
        if self.indexed {
            return format!("{name} (indexed)");
        }
        match (&self.source_name, self.source) {
            (Some(n), _) => fields::cap(&format!("{name} ← {n}"), fields::SHORT_LIMIT),
            (None, Some(a)) => format!("{name} ← {a}"),
            _ => name,
        }
    }
}

/// Every explanation in a ROM, by file offset.
#[derive(Debug, Clone, Default)]
pub struct Explanations {
    writes: HashMap<u32, Explained>,
    routines: usize,
}

/// Counts for measuring (docs/20 E8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub routines: usize,
    pub stores: usize,
    pub known: usize,
    pub sourced: usize,
    pub indexed: usize,
}

impl Explanations {
    pub fn build(rom: &RomImage, project: &Project, snap: &AnalysisSnapshot) -> Explanations {
        let entries = function::entries(snap);
        let mut stores: HashMap<u32, Store> = HashMap::new();
        let mut routines = 0;
        for &e in &entries {
            let Ok(f) = function::discover(rom, snap, &entries, e) else {
                continue;
            };
            routines += 1;
            for s in routine_stores(rom, &f) {
                stores
                    .entry(s.offset.0)
                    .and_modify(|old| *old = merge(old, &s))
                    .or_insert(s);
            }
        }
        let symbols = Symbols::new(rom, project, &snap.auto_labels);
        let writes = stores
            .into_values()
            .filter_map(|s| explain(rom, &symbols, &s).map(|e| (s.offset.0, e)))
            .collect();
        Explanations { writes, routines }
    }

    /// The store at `offset`, explained.
    pub fn write_at(&self, offset: FileOffset) -> Option<&Explained> {
        self.writes.get(&offset.0)
    }

    /// Every explained store, by offset.
    pub fn writes(&self) -> Vec<&Explained> {
        let mut out: Vec<&Explained> = self.writes.values().collect();
        out.sort_by_key(|e| e.offset.0);
        out
    }

    pub fn stats(&self) -> Stats {
        let mut s = Stats {
            routines: self.routines,
            stores: self.writes.len(),
            ..Stats::default()
        };
        for e in self.writes.values() {
            if e.indexed {
                s.indexed += 1;
            } else if e.write.value.is_some() {
                s.known += 1;
            } else if e.source.is_some() {
                s.sourced += 1;
            }
        }
        s
    }
}

/// The hardware stores in one routine, with their values.
pub fn routine_stores(rom: &RomImage, f: &Function) -> Vec<Store> {
    let cfg = Cfg::build(f);
    values::values(rom, f, &cfg).stores
}

/// Two routines reach the same store (a shared tail): keep what they agree
/// on.
fn merge(a: &Store, b: &Store) -> Store {
    let join = |x: Byte, y: Byte| if x == y { x } else { Byte::Unknown };
    Store {
        offset: a.offset,
        register: a.register,
        width: a.width,
        bytes: [join(a.bytes[0], b.bytes[0]), join(a.bytes[1], b.bytes[1])],
        indexed: a.indexed || b.indexed || a.register != b.register,
    }
}

fn explain(rom: &RomImage, symbols: &Symbols, s: &Store) -> Option<Explained> {
    let write = describe(s.register, s.value(), s.width)?;
    let source = s
        .source()
        .filter(|_| s.value().is_none() && !s.indexed)
        .map(|a| Project::canonical(rom, a));
    let source_name = source.and_then(|a| {
        symbols
            .name_for(a)
            .filter(|n| n.user)
            .map(|n| format!("{} ({a})", n.name))
    });
    Some(Explained {
        offset: s.offset,
        write,
        source,
        source_name,
        indexed: s.indexed,
    })
}
