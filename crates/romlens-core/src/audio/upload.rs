//! How a game gets its sound driver, songs and samples into audio RAM
//! (docs/23, A9), read from the ROM without running it.
//!
//! At power on the SPC700's boot program waits on the ports for an upload
//! (`apu::ipl` describes the protocol). Nearly every game sends it with
//! one routine that walks a block list through a long pointer: a length
//! and an audio RAM address, then that many bytes, again and again, ending
//! with a zero length and the address to run. This finds that routine by
//! what it does to the ports (waits for `$BBAA`, kicks with `$CC`, reads
//! `[dp],Y`), finds the constant pointers other code stores into its
//! pointer before calling it, and reads each block list there. Every block
//! then says where in the ROM each byte of audio RAM came from.
//!
//! Constants stored to the ports anywhere else are the game's sound
//! commands: the values its code sends the driver.

use std::collections::BTreeSet;

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::{AddressingMode, Mnemonic};
use crate::decompile::cfg::Cfg;
use crate::decompile::function::{self, Function};
use crate::explain::values::{self, Byte, State, Values};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::recording::apu::{ApuEvent, ApuEventKind};
use crate::rom::image::RomImage;

/// The routine that runs the upload protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRoutine {
    pub entry: SnesAddress,
    /// The direct-page address of its 24-bit pointer to a block list.
    pub pointer: u16,
    /// The instructions that show what it is: the reads of APUIO0, the
    /// `$BBAA` and `$CC`, the reads through the pointer.
    pub evidence: Vec<FileOffset>,
}

/// One block of a list: `len` bytes from `rom` to audio RAM at `aram`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub aram: u16,
    pub len: u16,
    /// Where the bytes are, and the address the code reads them from.
    pub rom: FileOffset,
    pub from: SnesAddress,
}

/// A block list some code points the upload routine at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub list: SnesAddress,
    /// The first of the stores that set the pointer, and their routine.
    pub set_at: FileOffset,
    pub set_in: SnesAddress,
    pub blocks: Vec<Block>,
    /// Where the SPC700 goes when the list ends.
    pub entry: u16,
    /// The list's bytes in the ROM, headers included.
    pub list_len: u32,
}

impl Upload {
    pub fn bytes(&self) -> u32 {
        self.blocks.iter().map(|b| b.len as u32).sum()
    }

    /// The upload's blocks, as the high-level boot takes them.
    pub fn apu_blocks(&self, rom: &RomImage) -> Vec<crate::apu::UploadBlock> {
        self.blocks
            .iter()
            .map(|b| crate::apu::UploadBlock {
                at: b.aram,
                bytes: rom.bytes()[b.rom.as_usize()..b.rom.as_usize() + b.len as usize].to_vec(),
            })
            .collect()
    }
}

/// A constant the code stores to a port: a command to the sound driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundCommand {
    pub at: FileOffset,
    pub routine: SnesAddress,
    /// 0-3, the first port written.
    pub port: u8,
    /// 1 or 2 ports.
    pub width: u8,
    pub value: u32,
    /// Set in this RAM byte, which other code copies to the port (as a
    /// game's NMI often does), rather than stored to the port itself.
    pub via: Option<SnesAddress>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadReport {
    pub routines: Vec<UploadRoutine>,
    pub uploads: Vec<Upload>,
    pub commands: Vec<SoundCommand>,
}

/// The APUIO port an operand names, in any bank that maps the registers.
fn port_of(insn: &crate::cpu65816::Instruction) -> Option<u8> {
    let addr = match insn.mode {
        AddressingMode::Absolute | AddressingMode::AbsoluteX => insn.operand.value(),
        AddressingMode::AbsoluteLong | AddressingMode::AbsoluteLongX => {
            let a = insn.operand.value();
            let bank = (a >> 16) as u8;
            if !(bank < 0x40 || (0x80..0xC0).contains(&bank)) {
                return None;
            }
            a & 0xFFFF
        }
        _ => return None,
    };
    (0x2140..=0x2143)
        .contains(&addr)
        .then(|| (addr - 0x2140) as u8)
}

/// Whether the routine is an upload routine, and its pointer.
pub(crate) fn upload_routine(f: &Function) -> Option<UploadRoutine> {
    use Mnemonic::*;
    let mut reads_port = Vec::new();
    let mut kick = Vec::new();
    let mut pointer: Option<(u16, Vec<FileOffset>)> = None;
    for s in &f.steps {
        let i = &s.insn;
        if matches!(i.mnemonic, CMP | LDA) && port_of(i) == Some(0) {
            reads_port.push(i.file_offset);
        }
        if i.mode.is_immediate() && matches!(i.operand.value(), 0xBBAA | 0xCC) {
            kick.push(i.file_offset);
        }
        if i.mnemonic == LDA
            && matches!(
                i.mode,
                AddressingMode::DirectIndirectLongIndexed | AddressingMode::DirectIndirectLong
            )
        {
            let dp = i.operand.value() as u16;
            match &mut pointer {
                Some((p, at)) if *p == dp => at.push(i.file_offset),
                None => pointer = Some((dp, vec![i.file_offset])),
                _ => {}
            }
        }
    }
    let (pointer, reads) = pointer?;
    if reads_port.is_empty() || kick.is_empty() {
        return None;
    }
    let mut evidence: Vec<FileOffset> = reads_port.into_iter().chain(kick).chain(reads).collect();
    evidence.sort();
    evidence.dedup();
    Some(UploadRoutine {
        entry: f.entry,
        pointer,
        evidence,
    })
}

/// The other form (Super Metroid's): a routine that takes the list's
/// offset into Y from a direct-page pointer and its bank into the data
/// bank from the byte two past it (`LDY $00`, `LDA $02`, `PHA`, `PLB`),
/// then calls a routine that waits for `$BBAA`, kicks with `$CC` and reads
/// the list as `LDA $0000,Y`. The pointer is the same three bytes as a
/// long pointer's, so the lists are found the same way.
pub(crate) fn banked_upload_routine(
    f: &Function,
    callee: &dyn Fn(SnesAddress) -> Option<Function>,
) -> Option<UploadRoutine> {
    use crate::decompile::function::{Callee, Transfer};
    use Mnemonic::*;
    let steps = &f.steps;
    let mut offset_from = None;
    let mut bank_from = None;
    for (k, s) in steps.iter().enumerate() {
        let i = &s.insn;
        if i.mnemonic == LDY && i.mode == AddressingMode::Direct {
            offset_from = Some((i.operand.value() as u16, i.file_offset));
        }
        if i.mnemonic == LDA
            && i.mode == AddressingMode::Direct
            && steps.get(k + 1).is_some_and(|n| n.insn.mnemonic == PHA)
            && steps.get(k + 2).is_some_and(|n| n.insn.mnemonic == PLB)
        {
            bank_from = Some((i.operand.value() as u16, i.file_offset));
        }
    }
    let ((dp, at_y), (bank, at_bank)) = (offset_from?, bank_from?);
    if bank != dp.wrapping_add(2) {
        return None;
    }
    for s in steps {
        let Transfer::Call {
            callee: Callee::Direct(t),
            ..
        } = &s.transfer
        else {
            continue;
        };
        let Some(g) = callee(*t) else { continue };
        let mut reads_port = Vec::new();
        let mut kick = Vec::new();
        let mut reads = Vec::new();
        for gs in &g.steps {
            let i = &gs.insn;
            if matches!(i.mnemonic, CMP | LDA) && port_of(i) == Some(0) {
                reads_port.push(i.file_offset);
            }
            if i.mode.is_immediate() && matches!(i.operand.value(), 0xBBAA | 0xCC) {
                kick.push(i.file_offset);
            }
            if i.mnemonic == LDA && i.mode == AddressingMode::AbsoluteY && i.operand.value() < 0x100
            {
                reads.push(i.file_offset);
            }
        }
        if reads_port.is_empty() || kick.is_empty() || reads.is_empty() {
            continue;
        }
        let mut evidence: Vec<FileOffset> = [at_y, at_bank]
            .into_iter()
            .chain(reads_port)
            .chain(kick)
            .chain(reads)
            .collect();
        evidence.sort();
        evidence.dedup();
        return Some(UploadRoutine {
            entry: f.entry,
            pointer: dp,
            evidence,
        });
    }
    None
}

/// The low-RAM address a store writes, when it is one: the direct page
/// (taken as page 0, where games keep such pointers), `$0000-$1FFF` in a
/// bank that mirrors it, or bank `$7E`.
fn low_ram(insn: &crate::cpu65816::Instruction) -> Option<u16> {
    match insn.mode {
        AddressingMode::Direct => Some(insn.operand.value() as u16),
        AddressingMode::Absolute => {
            let a = insn.operand.value() as u16;
            (a < 0x2000).then_some(a)
        }
        AddressingMode::AbsoluteLong => {
            let a = insn.operand.value();
            let (bank, off) = ((a >> 16) as u8, (a & 0xFFFF) as u16);
            let mirrors = bank < 0x40 || (0x80..0xC0).contains(&bank);
            ((mirrors && off < 0x2000) || bank == 0x7E).then_some(off)
        }
        _ => None,
    }
}

/// A RAM address as its low-RAM offset: `$0000-$1FFF` in a bank that
/// mirrors it, or anything in bank `$7E`.
fn low(a: SnesAddress) -> Option<u16> {
    let (bank, off) = (a.bank(), a.offset());
    let mirrors = bank < 0x40 || (0x80..0xC0).contains(&bank);
    ((mirrors && off < 0x2000) || bank == 0x7E).then_some(off)
}

/// The bytes a store writes, as far as the values know, and how many.
fn stored(insn: &crate::cpu65816::Instruction, s: &State) -> Option<([Byte; 2], usize)> {
    use Mnemonic::*;
    let (bytes, eight) = match insn.mnemonic {
        STA => (s.a, insn.flags_before.m),
        STX => (s.x, insn.flags_before.x),
        STY => (s.y, insn.flags_before.x),
        STZ => ([Byte::Known(0); 2], insn.flags_before.m),
        _ => return None,
    };
    Some((bytes, if eight { 1 } else { 2 }))
}

/// Constant pointers stored into `pointer`: each as (the 24-bit value,
/// the first store). Stores are taken in the routine's order, the last to
/// each byte counting, a call forgetting nothing (the pointer is set just
/// before the call that uses it).
fn pointers_set(f: &Function, v: &Values, pointer: u16) -> Vec<(u32, FileOffset)> {
    let mut bytes: [Option<(u8, FileOffset)>; 3] = [None; 3];
    let mut out = Vec::new();
    for (i, s) in f.steps.iter().enumerate() {
        let insn = &s.insn;
        let Some(at) = low_ram(insn) else { continue };
        let Some(state) = v.before.get(i).and_then(|b| b.as_ref()) else {
            continue;
        };
        let Some((value, width)) = stored(insn, state) else {
            continue;
        };
        let mut touched = false;
        for (k, v) in value.iter().take(width).enumerate() {
            let a = at.wrapping_add(k as u16);
            if (pointer..pointer + 3).contains(&a) {
                touched = true;
                bytes[(a - pointer) as usize] = v.known().map(|b| (b, insn.file_offset));
            }
        }
        if touched && let [Some(lo), Some(hi), Some(bank)] = bytes {
            let first = [lo.1, hi.1, bank.1].into_iter().min().unwrap();
            let value = lo.0 as u32 | (hi.0 as u32) << 8 | (bank.0 as u32) << 16;
            out.push((value, first));
            bytes = [None; 3];
        }
    }
    out
}

/// Tables of three-byte pointers a routine copies into the pointer by
/// index: `LDA table,X` stored to it and `LDA table+1,X` to the byte after
/// (Super Metroid's song banks, `$8F:E7E1`). The table's base, and the
/// instruction that reads it.
fn pointer_tables(f: &Function, pointer: u16) -> Vec<(u32, FileOffset)> {
    use Mnemonic::*;
    let steps = &f.steps;
    let loads = |target: u16| -> Vec<(u32, FileOffset)> {
        steps
            .windows(2)
            .filter_map(|w| {
                let (l, st) = (&w[0].insn, &w[1].insn);
                let base = match l.mode {
                    AddressingMode::AbsoluteLongX => l.operand.value(),
                    AddressingMode::AbsoluteX => {
                        (l.address.bank() as u32) << 16 | l.operand.value() & 0xFFFF
                    }
                    _ => return None,
                };
                (l.mnemonic == LDA
                    && !l.flags_before.m
                    && st.mnemonic == STA
                    && low_ram(st) == Some(target))
                .then_some((base, l.file_offset))
            })
            .collect()
    };
    let second = loads(pointer.wrapping_add(1));
    loads(pointer)
        .into_iter()
        .filter(|(base, _)| second.iter().any(|(b, _)| *b == base + 1))
        .collect()
}

/// The lists a pointer table holds: entries three bytes apart from its
/// base, read until two in a row are not lists.
fn table_lists(rom: &RomImage, base: u32) -> Vec<SnesAddress> {
    let mut out = Vec::new();
    let mut misses = 0;
    for k in 0..256u32 {
        let at = base + k * 3;
        let byte = |p: u32| {
            rom.file_offset_for(SnesAddress::from_u24(p))
                .and_then(|o| rom.bytes().get(o.as_usize()).copied())
        };
        let (Some(lo), Some(hi), Some(bank)) = (byte(at), byte(at + 1), byte(at + 2)) else {
            break;
        };
        let list = SnesAddress::from_u24(lo as u32 | (hi as u32) << 8 | (bank as u32) << 16);
        if parse_list(rom, list).is_some() {
            out.push(list);
            misses = 0;
        } else {
            misses += 1;
            if misses == 2 {
                break;
            }
        }
    }
    out
}

/// The routines that may set an upload's pointer: the upload routines,
/// those that call one, and those that call those. A pointer set anywhere
/// else is for something else (Super Metroid fills `$00–$02` for its
/// decompressor too).
fn senders<'a>(
    routines: &[UploadRoutine],
    functions: impl Iterator<Item = &'a Function> + Clone,
) -> BTreeSet<SnesAddress> {
    use crate::decompile::function::{Callee, Transfer};
    let mut out: BTreeSet<SnesAddress> = routines.iter().map(|r| r.entry).collect();
    for _ in 0..2 {
        let callers: Vec<SnesAddress> = functions
            .clone()
            .filter(|f| {
                f.steps.iter().any(|s| {
                    matches!(&s.transfer, Transfer::Call { callee: Callee::Direct(t), .. } if out.contains(t))
                })
            })
            .map(|f| f.entry)
            .collect();
        out.extend(callers);
    }
    out
}

/// Read the block list at `at`: `(blocks, entry, bytes read)`, or `None`
/// when what is there is not a list.
pub fn parse_list(rom: &RomImage, at: SnesAddress) -> Option<(Vec<Block>, u16, u32)> {
    // Read on through the file: a loader that steps its bank when the
    // offset wraps (Super Metroid's, `$CF:FFFF` then `$D0:8000`) reads
    // the file in order, as one that stays in a bank does.
    let start = rom.file_offset_for(at)?.0;
    let bytes = rom.bytes();
    let word = |p: u32| -> Option<u16> {
        Some(u16::from_le_bytes([
            *bytes.get(p as usize)?,
            *bytes.get(p as usize + 1)?,
        ]))
    };
    let mut pos = start;
    let mut blocks = Vec::new();
    let mut total = 0u32;
    loop {
        let len = word(pos)?;
        let aram = word(pos + 2)?;
        pos += 4;
        if len == 0 {
            if blocks.is_empty() {
                return None;
            }
            return Some((blocks, aram, pos - start));
        }
        total += len as u32;
        if aram as u32 + len as u32 > 0x10000 || total > 0x10000 || blocks.len() >= 64 {
            return None;
        }
        if (pos + len as u32) as usize > bytes.len() {
            return None;
        }
        blocks.push(Block {
            aram,
            len,
            rom: FileOffset(pos),
            from: rom.snes_address_for(FileOffset(pos))?,
        });
        pos += len as u32;
    }
}

/// The routines that could hold the upload and set its pointer, found
/// from the cross-references rather than by walking every routine: those
/// that read or write the ports, and their callers three calls up.
fn near_the_ports(rom: &RomImage, snap: &AnalysisSnapshot) -> Vec<Function> {
    use crate::model::xref::XRefKind;
    let entries = function::entries(snap);
    let mut found: Vec<Function> = Vec::new();
    let mut seen: BTreeSet<SnesAddress> = BTreeSet::new();
    let mut frontier: Vec<FileOffset> = snap
        .xrefs_by_target
        .iter()
        .filter(|x| {
            let a = x.to.offset();
            let bank = x.to.bank();
            (0x2140..=0x2143).contains(&a) && (bank < 0x40 || (0x80..0xC0).contains(&bank))
        })
        .map(|x| x.from)
        .collect();
    let by_offset: Vec<(u32, SnesAddress)> = {
        let mut v: Vec<(u32, SnesAddress)> = entries
            .iter()
            .filter_map(|a| rom.file_offset_for(*a).map(|o| (o.0, *a)))
            .collect();
        v.sort_unstable();
        v
    };
    for _ in 0..4 {
        let mut next = Vec::new();
        for off in frontier.drain(..) {
            // Every routine the instruction is in: one can run on into
            // another's entry (a pointer set, then into the call).
            let lo = by_offset.partition_point(|(o, _)| *o + WINDOW < off.0);
            let hi = by_offset.partition_point(|(o, _)| *o <= off.0 + WINDOW);
            for &(_, e) in &by_offset[lo..hi] {
                if seen.contains(&e) {
                    continue;
                }
                let Ok(f) = function::discover(rom, snap, &entries, e) else {
                    continue;
                };
                if f.step_containing(off).is_none() {
                    continue;
                }
                seen.insert(e);
                // Who calls it: the next ring out.
                next.extend(
                    snap.xrefs_by_target
                        .iter()
                        .filter(|x| x.to == f.entry && x.kind == XRefKind::Call)
                        .map(|x| x.from),
                );
                found.push(f);
            }
        }
        frontier = next;
    }
    found
}

/// How far from an instruction a routine that contains it may start: before
/// it, or after it for one that branches back into it.
const WINDOW: u32 = 0x400;

/// The uploads alone, quickly: for the analysis, which runs on every edit.
pub fn trace_uploads(rom: &RomImage, snap: &AnalysisSnapshot) -> (Vec<UploadRoutine>, Vec<Upload>) {
    let near = near_the_ports(rom, snap);
    let entries = function::entries(snap);
    let callee = |t: SnesAddress| function::discover(rom, snap, &entries, t).ok();
    let routines: Vec<UploadRoutine> = near
        .iter()
        .filter_map(|f| upload_routine(f).or_else(|| banked_upload_routine(f, &callee)))
        .collect();
    if routines.is_empty() {
        return (routines, Vec::new());
    }
    let pointers: BTreeSet<u16> = routines.iter().map(|r| r.pointer).collect();
    let senders = senders(&routines, near.iter());
    let mut uploads: Vec<Upload> = Vec::new();
    for f in near.iter().filter(|f| senders.contains(&f.entry)) {
        let v = values::values(rom, f, &Cfg::build(f));
        collect_uploads(rom, f, &v, &pointers, &mut uploads);
    }
    uploads.sort_by_key(|u| u.set_at);
    (routines, uploads)
}

fn collect_uploads(
    rom: &RomImage,
    f: &Function,
    v: &Values,
    pointers: &BTreeSet<u16>,
    uploads: &mut Vec<Upload>,
) {
    for &p in pointers {
        let tables = pointer_tables(f, p).into_iter().flat_map(|(base, set_at)| {
            table_lists(rom, base)
                .into_iter()
                .map(move |l| (l.as_u24(), set_at))
        });
        for (value, set_at) in pointers_set(f, v, p).into_iter().chain(tables) {
            if uploads.iter().any(|u| u.list.as_u24() == value) {
                continue;
            }
            let list = SnesAddress::from_u24(value);
            if let Some((blocks, entry, list_len)) = parse_list(rom, list) {
                uploads.push(Upload {
                    list,
                    set_at,
                    set_in: f.entry,
                    blocks,
                    entry,
                    list_len,
                });
            }
        }
    }
}

/// Everything about sound uploads the ROM's analysed code shows.
pub fn trace(rom: &RomImage, snap: &AnalysisSnapshot) -> UploadReport {
    let entries = function::entries(snap);
    let callee = |t: SnesAddress| function::discover(rom, snap, &entries, t).ok();
    let mut routines = Vec::new();
    let mut analysed: Vec<(Function, Values)> = Vec::new();
    for &e in &entries {
        let Ok(f) = function::discover(rom, snap, &entries, e) else {
            continue;
        };
        let cfg = Cfg::build(&f);
        let v = values::values(rom, &f, &cfg);
        if let Some(r) = upload_routine(&f).or_else(|| banked_upload_routine(&f, &callee)) {
            routines.push(r);
        }
        analysed.push((f, v));
    }
    let upload_entries: BTreeSet<SnesAddress> = routines.iter().map(|r| r.entry).collect();
    let pointers: BTreeSet<u16> = routines.iter().map(|r| r.pointer).collect();
    let mut uploads: Vec<Upload> = Vec::new();
    let mut commands = Vec::new();
    // RAM bytes the code copies to a port, and the port.
    let mut mirrors: BTreeSet<(Option<u16>, u8)> = BTreeSet::new();
    let senders = senders(&routines, analysed.iter().map(|(f, _)| f));
    for (f, v) in &analysed {
        if senders.contains(&f.entry) {
            collect_uploads(rom, f, v, &pointers, &mut uploads);
        }
        if upload_entries.contains(&f.entry) {
            continue;
        }
        for s in &v.stores {
            if !(0x2140..=0x2143).contains(&s.register) {
                continue;
            }
            let port = (s.register - 0x2140) as u8;
            if let Some(value) = s.value() {
                commands.push(SoundCommand {
                    at: s.offset,
                    routine: f.entry,
                    port,
                    width: s.width,
                    value,
                    via: None,
                });
            } else if let Some(from) = s.source() {
                for k in 0..s.width {
                    mirrors.insert((low(from).map(|o| o + k as u16), port + k));
                }
            }
        }
    }
    // Constants stored to the RAM bytes copied to the ports.
    for (f, v) in &analysed {
        for (i, st) in f.steps.iter().enumerate() {
            let insn = &st.insn;
            let Some(at) = low_ram(insn) else { continue };
            let Some(state) = v.before.get(i).and_then(|b| b.as_ref()) else {
                continue;
            };
            let Some((value, width)) = stored(insn, state) else {
                continue;
            };
            for (k, v) in value.iter().take(width).enumerate() {
                let a = at.wrapping_add(k as u16);
                for &(m, port) in &mirrors {
                    if m == Some(a)
                        && let Some(b) = v.known()
                        // Clearing the byte after the driver took it is
                        // not a command.
                        && b != 0
                    {
                        commands.push(SoundCommand {
                            at: insn.file_offset,
                            routine: f.entry,
                            port,
                            width: 1,
                            value: b as u32,
                            via: Some(SnesAddress::new(0x7E, a)),
                        });
                    }
                }
            }
        }
    }
    uploads.sort_by_key(|u| u.set_at);
    commands.sort_by_key(|c| (c.at, c.port));
    commands.dedup_by_key(|c| (c.at, c.port));
    UploadReport {
        routines,
        uploads,
        commands,
    }
}

/// An upload read back from the S-CPU's port writes (a recording's): the
/// blocks the boot program (or a driver speaking the same protocol) was
/// sent, with the bytes themselves, and where it was told to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentUpload {
    pub blocks: Vec<(u16, Vec<u8>)>,
    pub entry: Option<u16>,
}

/// SPC700 cycles between the two bytes of one 16-bit store to the ports.
const SAME_STORE: u64 = 4;

/// Decode uploads from the S-CPU's writes to the ports, in order. A
/// transfer starts when port 0 gets `$CC` with an address on ports 2 and
/// 3; each byte is port 1 when port 0 gets its index; a port 0 value past
/// the next index is a new block (port 1 non-zero) or the end (port 1
/// zero). A 16-bit store sets port 0 then port 1 a cycle or two later, so
/// a write to another port that close after port 0 counts as already
/// there; a game that writes the byte first and the index after needs
/// nothing more.
pub fn from_ports(events: &[ApuEvent]) -> Vec<SentUpload> {
    let cpu: Vec<&ApuEvent> = events
        .iter()
        .filter(|e| e.kind == ApuEventKind::CpuPort)
        .collect();
    let mut ports = [0u8; 4];
    let mut out: Vec<SentUpload> = Vec::new();
    // The current upload, the current block, and the index expected next.
    let mut open: Option<SentUpload> = None;
    let mut next: u8 = 0;
    let mut k = 0;
    while k < cpu.len() {
        let e = cpu[k];
        ports[e.address as usize & 3] = e.value;
        if e.address != 0 {
            k += 1;
            continue;
        }
        // Port 0 changed: take the other ports as they stand once the
        // store that set it is done (until port 0 is written again).
        let mut after = ports;
        let mut j = k + 1;
        while j < cpu.len() && cpu[j].address != 0 && cpu[j].spc_cycle <= e.spc_cycle + SAME_STORE {
            after[cpu[j].address as usize & 3] = cpu[j].value;
            j += 1;
        }
        let address = u16::from_le_bytes([after[2], after[3]]);
        match &mut open {
            None if e.value == 0xCC => {
                open = Some(SentUpload {
                    blocks: vec![(address, Vec::new())],
                    entry: None,
                });
                next = 0;
            }
            None => {}
            Some(u) if e.value == next => {
                u.blocks.last_mut().unwrap().1.push(after[1]);
                next = next.wrapping_add(1);
            }
            Some(u) if e.value == next.wrapping_sub(1) => {
                let _ = u;
            }
            Some(u) => {
                if after[1] != 0 {
                    u.blocks.push((address, Vec::new()));
                    next = 0;
                } else {
                    u.entry = Some(address);
                    u.blocks.retain(|b| !b.1.is_empty());
                    out.push(open.take().unwrap());
                }
            }
        }
        k += 1;
    }
    if let Some(mut u) = open {
        u.blocks.retain(|b| !b.1.is_empty());
        if !u.blocks.is_empty() {
            out.push(u);
        }
    }
    out
}

/// A stretch of the ROM an upload sends, and what it is in audio RAM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedSpan {
    pub start: FileOffset,
    pub len: u32,
    pub sample: bool,
    /// For the region's evidence: where it goes and what it is.
    pub what: String,
}

/// The upload whose entry is inside its own blocks: the driver itself.
pub fn driver(uploads: &[Upload]) -> Option<&Upload> {
    uploads.iter().find(|u| {
        u.blocks
            .iter()
            .any(|b| (b.aram as u32..b.aram as u32 + b.len as u32).contains(&(u.entry as u32)))
    })
}

/// What each traced upload's bytes are. The driver is booted on
/// Romlens's own SPC700 for a tenth of a second (booted straight into, not
/// through the boot program), long enough to point the DSP at its sample
/// directory; each other upload is then laid over that audio RAM and its
/// bytes the directory names are samples.
pub fn uploaded_spans(rom: &RomImage, uploads: &[Upload]) -> Vec<UploadedSpan> {
    let Some(d) = driver(uploads) else {
        return Vec::new();
    };
    let mut apu = crate::apu::Apu::new();
    crate::apu::boot_upload(&mut apu, &d.apu_blocks(rom), d.entry);
    apu.run_until(102_400);
    let dir = apu.bus.dsp[0x5D];
    let mut out: Vec<UploadedSpan> = Vec::new();
    let mut done: BTreeSet<u32> = BTreeSet::new();
    for u in uploads {
        let mut aram = apu.bus.aram.clone();
        for b in &u.blocks {
            let from = b.rom.as_usize();
            aram[b.aram as usize..b.aram as usize + b.len as usize]
                .copy_from_slice(&rom.bytes()[from..from + b.len as usize]);
        }
        // What each audio RAM byte is: 0 driver or data, 1 a sample, 2 the
        // directory.
        let mut what = vec![0u8; 0x10000];
        if dir != 0 {
            let entries = crate::audio::directory(&aram, dir, &[]);
            for e in &entries {
                let end = (e.start as usize + e.blocks as usize * 9).min(0x10000);
                what[e.start as usize..end].fill(1);
            }
            let base = (dir as usize) << 8;
            let len = entries
                .iter()
                .map(|e| e.index as usize + 1)
                .max()
                .unwrap_or(0)
                * 4;
            what[base..(base + len).min(0x10000)].fill(2);
        }
        let role = if u.list == d.list {
            "the sound driver's upload"
        } else {
            "an upload for the running driver"
        };
        for b in &u.blocks {
            if !done.insert(b.rom.0) {
                continue;
            }
            let mut i = 0u32;
            while i < b.len as u32 {
                let is = what[b.aram as usize + i as usize];
                let mut j = i + 1;
                while j < b.len as u32 && what[b.aram as usize + j as usize] == is {
                    j += 1;
                }
                let at = b.aram as u32 + i;
                let (lead, tail) = match is {
                    1 => ("BRR samples, uploaded", ""),
                    2 => ("the sample directory, uploaded", ""),
                    _ => ("uploaded", ": driver code or its data"),
                };
                out.push(UploadedSpan {
                    start: FileOffset(b.rom.0 + i),
                    len: j - i,
                    sample: is == 1,
                    what: format!(
                        "{lead} to audio RAM ${at:04X}-${:04X} ({role}, the list at {}){tail}",
                        at + (j - i) - 1,
                        u.list,
                    ),
                });
                i = j;
            }
        }
    }
    out
}
