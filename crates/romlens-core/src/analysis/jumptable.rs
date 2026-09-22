//! Jump tables: the `JMP (abs,X)` / `JSR (abs,X)` dispatch that Phase 1's walk
//! stopped at.
//!
//! The descent cannot follow these on its own — the index is a runtime value —
//! but the table itself is in the ROM, and a table of in-bank addresses that
//! all decode as routines is not something that happens by accident. Resolving
//! them is what turns a 0.2% code map into a useful one
//! (`10-ffi-spike.md`, `16-phase2-plan.md` 2A.1).
//!
//! Resolution runs *between* descent passes rather than inside the walk. The
//! bound on a table's length asks which bytes are already decoded as code, and
//! mid-walk that answer depends on the order the worklist happened to be
//! drained in; a golden that depended on queue order would be worthless.

use std::collections::BTreeMap;

use crate::analysis::descent::{KIND_NONE, OVR_WALL, TableSite, Walk};
use crate::cpu65816::{FlagState, Instruction, Mnemonic, decode};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::memory::map::MappingMode;
use crate::rom::image::RomImage;

/// A dispatch table is never this long in practice; the cap stops a run of
/// plausible-looking filler from swallowing a bank.
pub const MAX_ENTRIES: usize = 256;

/// Entries are 16-bit addresses within the program bank.
pub const ENTRY_LEN: u32 = 2;

/// Why the table ended where it did. The reason is the confidence: a table
/// stopped by the first routine it points at is far better evidence than one
/// that simply ran out of bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The next entry would reach past the lowest routine this table points
    /// at. A dispatch table is essentially always followed by its own targets,
    /// so this is the bound that actually works.
    TargetFloor,
    /// The next entry overlaps bytes the descent decoded as code.
    Code,
    /// The user marked the next bytes as data.
    Wall,
    /// The next entry does not read as an address of a routine.
    Implausible,
    /// The next entry points inside the table.
    SelfReference,
    /// End of the bank, or of the image.
    Bank,
    /// `MAX_ENTRIES`.
    Cap,
}

impl StopReason {
    pub const fn name(self) -> &'static str {
        match self {
            StopReason::TargetFloor => "first target",
            StopReason::Code => "code",
            StopReason::Wall => "user data mark",
            StopReason::Implausible => "implausible entry",
            StopReason::SelfReference => "entry inside the table",
            StopReason::Bank => "end of bank",
            StopReason::Cap => "entry cap",
        }
    }

    /// 0.60 to 0.85. Anything the table's own contents proved ranks above
    /// anything the image's shape merely allowed.
    pub const fn confidence(self) -> f32 {
        match self {
            StopReason::TargetFloor => 0.85,
            StopReason::Code => 0.80,
            StopReason::Wall => 0.75,
            StopReason::SelfReference => 0.70,
            StopReason::Implausible => 0.65,
            StopReason::Bank | StopReason::Cap => 0.60,
        }
    }
}

/// A resolved table.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpTable {
    /// The dispatching instruction.
    pub site: u32,
    pub site_address: SnesAddress,
    /// `JSR (abs,X)` rather than `JMP (abs,X)`.
    pub call: bool,
    pub base: u32,
    pub base_address: SnesAddress,
    /// Targets in table order; one per entry, all inside the program bank.
    pub targets: Vec<(SnesAddress, u32)>,
    pub stop: StopReason,
}

impl JumpTable {
    pub fn len(&self) -> u32 {
        self.targets.len() as u32 * ENTRY_LEN
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn end(&self) -> u32 {
        self.base + self.len()
    }

    pub fn confidence(&self) -> f32 {
        self.stop.confidence()
    }

    /// The evidence string the inspector shows. Naming the dispatcher is the
    /// point: "why is this data?" is answered by the instruction that reads it.
    pub fn description(&self) -> String {
        format!(
            "jump table, {} entries, dispatched from {}",
            self.targets.len(),
            self.site_address
        )
    }
}

/// What a dispatch site resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Table(JumpTable),
    /// Why it could not be read, phrased for the warning list.
    Unresolved(String),
}

/// Resolve every dispatch site a completed walk found.
pub fn resolve(walk: &Walk<'_>) -> BTreeMap<u32, Resolution> {
    walk.found_table_sites
        .iter()
        .map(|site| (site.offset, resolve_site(walk, site)))
        .collect()
}

fn resolve_site(walk: &Walk<'_>, site: &TableSite) -> Resolution {
    let rom = walk.rom;
    let Some(base) = rom.file_offset_for(site.base) else {
        // Much the commonest failure, and worth naming precisely: a table
        // built in RAM needs value tracking along control flow (Phase 3).
        return Resolution::Unresolved(format!(
            "table base {} is not in ROM; the table is built at run time",
            site.base
        ));
    };
    let base = base.0;
    let bank = site.base.bank();
    let bytes = rom.bytes();
    let bank_end = bank_end(rom, base);

    let mut targets: Vec<(SnesAddress, u32)> = Vec::new();
    // The lowest target above the table start. Nothing past it can be table.
    let mut floor = u32::MAX;
    let stop = loop {
        let slot = base + targets.len() as u32 * ENTRY_LEN;
        if targets.len() >= MAX_ENTRIES {
            break StopReason::Cap;
        }
        if slot + ENTRY_LEN > bank_end || slot as usize + 2 > bytes.len() {
            break StopReason::Bank;
        }
        if slot + ENTRY_LEN > floor {
            break StopReason::TargetFloor;
        }
        if (slot..slot + ENTRY_LEN).any(|b| walk.ovr[b as usize] == OVR_WALL) {
            break StopReason::Wall;
        }
        if (slot..slot + ENTRY_LEN).any(|b| walk.kind[b as usize] != KIND_NONE) {
            break StopReason::Code;
        }
        let word = u16::from_le_bytes([bytes[slot as usize], bytes[slot as usize + 1]]);
        let target = SnesAddress::new(bank, word);
        let Some(target_off) = rom.file_offset_for(target).map(|o| o.0) else {
            break StopReason::Implausible;
        };
        if target_off >= base && target_off < slot + ENTRY_LEN {
            break StopReason::SelfReference;
        }
        if !plausible_entry(rom, target_off, site.flags) {
            break StopReason::Implausible;
        }
        if target_off > base {
            floor = floor.min(target_off);
        }
        targets.push((target, target_off));
    };

    if targets.is_empty() {
        return Resolution::Unresolved(format!(
            "no entry at {} reads as a routine in bank ${bank:02X} ({})",
            site.base,
            stop.name()
        ));
    }
    Resolution::Table(JumpTable {
        site: site.offset,
        site_address: site.address,
        call: site.call,
        base,
        base_address: site.base,
        targets,
        stop,
    })
}

/// A bank never straddles a mapping window, so a table cannot either.
fn bank_end(rom: &RomImage, off: u32) -> u32 {
    let window = match rom.mapping() {
        MappingMode::LoRom => 0x8000,
        _ => 0x1_0000,
    };
    (off / window + 1) * window
}

/// The linear sweep's bar for "these bytes are a routine", but read under the
/// dispatcher's own flags.
///
/// That difference is the whole reason this is not just a call into `sweep`:
/// the sweep has no caller and has to guess M and X, while a dispatch site
/// knows exactly what they are when the jump is taken. Widths decide operand
/// lengths, so the same bytes decode into different instructions under a wrong
/// guess — this is precisely the sweep's blind spot.
pub fn plausible_entry(rom: &RomImage, off: u32, flags: FlagState) -> bool {
    let Some(first) = decode_at(rom, off, flags) else {
        return false;
    };
    if !usable(rom, &first) {
        return false;
    }
    // One instruction is enough only when it ends a block: `RTS` is a real,
    // if empty, dispatch target, whereas a lone `LDA` followed by nonsense is
    // how random data decodes.
    if first.mnemonic.is_block_end() {
        return true;
    }
    let next = off + first.len as u32;
    decode_at(rom, next, first.flags_after).is_some_and(|i| usable(rom, &i))
}

fn decode_at(rom: &RomImage, off: u32, flags: FlagState) -> Option<Instruction> {
    if off as usize >= rom.len() {
        return None;
    }
    let addr = rom.snes_address_for(FileOffset(off))?;
    decode(&rom.bytes()[off as usize..], addr, FileOffset(off), flags)
}

/// The sweep's two rejections: a trap opcode, or a control transfer leaving
/// the image.
fn usable(rom: &RomImage, insn: &Instruction) -> bool {
    if matches!(
        insn.mnemonic,
        Mnemonic::BRK | Mnemonic::WDM | Mnemonic::STP | Mnemonic::COP
    ) {
        return false;
    }
    let transfers = insn.mnemonic.is_branch() || insn.mnemonic.is_jump() || insn.mnemonic.is_call();
    match insn.target {
        Some(t) if transfers => rom.file_offset_for(t.address).is_some(),
        _ => true,
    }
}
