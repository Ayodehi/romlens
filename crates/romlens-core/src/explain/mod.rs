//! Explanations for a student (docs/20): what a write to a hardware
//! register does, field by field, and the common SNES sequences named where
//! they appear.
//!
//! [`Explanations::build`] runs over every routine the analysis found, once;
//! the listing, the inspector, the C and the CLI all read from it, so they
//! say the same thing.

pub mod fields;
pub mod idioms;
pub mod screen;
pub mod setup;
pub mod values;

use std::collections::{BTreeMap, HashMap};

pub use fields::{FieldRow, Part, RegisterWrite, describe};
pub use idioms::{DmaDest, DmaTransfer, Idiom, IdiomKind, IdiomRow, IdiomTable};
pub use values::{Byte, State, Store};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::SymbolLookup;
use crate::decompile::cfg::Cfg;
use crate::decompile::function;
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
    /// Ascending by first offset.
    idioms: Vec<Idiom>,
    /// The idioms covering each instruction, by index.
    covering: HashMap<u32, Vec<usize>>,
    routines: usize,
}

/// Counts for measuring (docs/20 E8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub routines: usize,
    pub stores: usize,
    pub known: usize,
    pub sourced: usize,
    pub indexed: usize,
    /// Idioms found, by kind.
    pub idioms: Vec<(IdiomKind, usize)>,
}

impl Explanations {
    pub fn build(rom: &RomImage, project: &Project, snap: &AnalysisSnapshot) -> Explanations {
        let entries = function::entries(snap);
        let symbols = Symbols::new(rom, project, &snap.auto_labels);
        let name = |a: SnesAddress| address_name(rom, &symbols, a);
        let mut stores: HashMap<u32, Store> = HashMap::new();
        let mut idioms: BTreeMap<(u32, IdiomKind), Idiom> = BTreeMap::new();
        let mut routines = 0;
        for &e in &entries {
            let Ok(f) = function::discover(rom, snap, &entries, e) else {
                continue;
            };
            routines += 1;
            let cfg = Cfg::build(&f);
            let v = values::values(rom, &f, &cfg);
            for i in idioms::find(rom, &f, &cfg, &v, &entries, &name) {
                // A shared tail is found from each routine that reaches it:
                // the first one found stands.
                idioms.entry((i.first().0, i.kind)).or_insert(i);
            }
            for s in v.stores {
                stores
                    .entry(s.offset.0)
                    .and_modify(|old| *old = merge(old, &s))
                    .or_insert(s);
            }
        }
        let writes = stores
            .into_values()
            .filter_map(|s| explain(rom, &symbols, &s).map(|e| (s.offset.0, e)))
            .collect();
        let idioms: Vec<Idiom> = idioms.into_values().collect();
        let mut covering: HashMap<u32, Vec<usize>> = HashMap::new();
        for (k, i) in idioms.iter().enumerate() {
            for o in &i.offsets {
                covering.entry(o.0).or_default().push(k);
            }
        }
        Explanations {
            writes,
            idioms,
            covering,
            routines,
        }
    }

    /// Every idiom, ascending by where it starts.
    pub fn idioms(&self) -> &[Idiom] {
        &self.idioms
    }

    /// The idioms an instruction is part of.
    pub fn idioms_at(&self, offset: FileOffset) -> Vec<&Idiom> {
        self.covering
            .get(&offset.0)
            .map(|v| v.iter().map(|&k| &self.idioms[k]).collect())
            .unwrap_or_default()
    }

    /// The idioms whose note goes above the instruction at `offset`.
    pub fn idioms_starting_at(&self, offset: FileOffset) -> Vec<&Idiom> {
        self.idioms_at(offset)
            .into_iter()
            .filter(|i| i.first() == offset)
            .collect()
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
        let mut kinds: BTreeMap<IdiomKind, usize> = BTreeMap::new();
        for i in &self.idioms {
            *kinds.entry(i.kind).or_default() += 1;
        }
        s.idioms = kinds.into_iter().collect();
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

/// One routine's explanations, for the C (docs/20): its stores and the
/// idioms in it, found from this routine alone.
pub fn routine(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    f: &crate::decompile::function::Function,
    cfg: &Cfg,
    entries: &std::collections::BTreeSet<SnesAddress>,
) -> (Vec<Explained>, Vec<Idiom>) {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let name = |a: SnesAddress| address_name(rom, &symbols, a);
    let v = values::values(rom, f, cfg);
    let idioms = idioms::find(rom, f, cfg, &v, entries, &name);
    let writes = v
        .stores
        .iter()
        .filter_map(|s| explain(rom, &symbols, s))
        .collect();
    (writes, idioms)
}

/// An address as a summary says it: `Brightness ($7E:0DAE)`, an automatic
/// name alone (it already says where it is), or `$7E:0DAE`.
pub(crate) fn address_name(rom: &RomImage, symbols: &Symbols, a: SnesAddress) -> String {
    let a = Project::canonical(rom, a);
    match symbols.name_for(a) {
        Some(n) if !n.user => n.name,
        Some(n) => format!("{} ({a})", n.name),
        None => format!("{a}"),
    }
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

impl Explanations {
    /// The DMA transfers the idioms found, with a known destination.
    fn transfers(&self) -> impl Iterator<Item = &DmaTransfer> {
        self.idioms.iter().flat_map(|i| i.transfers.iter())
    }

    fn upload(
        &self,
        rom: &RomImage,
        matching: &[&DmaTransfer],
        what: &str,
    ) -> Option<(String, Option<FileOffset>, FileOffset)> {
        // Prefer a transfer from ROM, which a viewer can show.
        let in_rom = |t: &&&DmaTransfer| t.source.is_some_and(|a| rom.file_offset_for(a).is_some());
        let t = matching.iter().find(in_rom).or(matching.first())?;
        let at = rom
            .snes_address_for(t.at)
            .map_or_else(|| format!("{}", t.at), |a| format!("{a}"));
        let mut text = match (t.source, t.bytes) {
            (Some(s), Some(n)) => format!("{what} by the DMA at {at}: ${n:04X} bytes from {s}"),
            (Some(s), None) => format!("{what} by the DMA at {at}, from {s}"),
            _ => format!("{what} by the DMA at {at}"),
        };
        if matching.len() > 1 {
            let others = matching.len() - 1;
            text.push_str(&format!(
                "; {others} other transfer{} also write{} here",
                if others == 1 { "" } else { "s" },
                if others == 1 { "s" } else { "" }
            ));
        }
        let src = t.source.and_then(|a| rom.file_offset_for(a));
        Some((text, src, t.at))
    }
}

/// `Explanations` with the ROM, to say where VRAM and the palette were
/// filled from (docs/21).
pub struct UploadIndex<'a> {
    pub explain: &'a Explanations,
    pub rom: &'a RomImage,
}

impl screen::Uploads for UploadIndex<'_> {
    fn vram(&self, word: u16) -> Option<(String, Option<FileOffset>, FileOffset)> {
        let w = u32::from(word & 0x7FFF);
        let matching: Vec<&DmaTransfer> = self
            .explain
            .transfers()
            .filter(|t| !t.reverse && !t.fill)
            .filter(|t| match (t.dest, t.bytes) {
                (DmaDest::Vram(Some(start)), Some(n)) => {
                    let s = u32::from(start & 0x7FFF);
                    w >= s && w < s + n.div_ceil(2)
                }
                (DmaDest::Vram(Some(start)), None) => u32::from(start & 0x7FFF) == w,
                _ => false,
            })
            .collect();
        self.explain
            .upload(self.rom, &matching, "VRAM here is written")
    }

    fn palette(&self) -> Option<(String, Option<FileOffset>, FileOffset)> {
        let matching: Vec<&DmaTransfer> = self
            .explain
            .transfers()
            .filter(|t| !t.reverse && !t.fill && matches!(t.dest, DmaDest::Cgram(Some(_))))
            .collect();
        self.explain
            .upload(self.rom, &matching, "The palette is written")
    }
}
