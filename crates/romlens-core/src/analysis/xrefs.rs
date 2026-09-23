//! Cross-reference construction and indexing.

use crate::cpu65816::{AddressingMode, Instruction, Mnemonic, TargetKind};
use crate::memory::address::{FileOffset, SnesAddress};
use crate::model::project::Project;
use crate::model::xref::{XRef, XRefKind};
use crate::rom::image::RomImage;

/// The xref an instruction makes, with the target canonicalised.
pub fn xref_for(rom: &RomImage, insn: &Instruction) -> Option<XRef> {
    let t = insn.target?;
    let kind = match t.kind {
        // A `JMP/JSR (abs,X)` points at a table of addresses, not at one
        // pointer, and the distinction decides the auto label: `JTBL_` reads
        // as a dispatch table, `PTR_` as a single indirect slot.
        TargetKind::Pointer if insn.mode == AddressingMode::AbsoluteIndexedIndirect => {
            XRefKind::JumpTable
        }
        TargetKind::Pointer => XRefKind::Pointer,
        TargetKind::Code => match insn.mnemonic {
            Mnemonic::JSR | Mnemonic::JSL => XRefKind::Call,
            m if m.is_branch() => XRefKind::Branch,
            _ => XRefKind::Jump,
        },
        TargetKind::Data => {
            if insn.mnemonic.writes_memory() {
                XRefKind::Write
            } else if insn.mnemonic.read_modify_writes() {
                XRefKind::ReadWrite
            } else {
                XRefKind::Read
            }
        }
    };
    let to_offset = rom.file_offset_for(t.address);
    Some(XRef {
        from: insn.file_offset,
        to: Project::canonical(rom, t.address),
        to_offset,
        kind,
        certain: t.certain,
        observed: false,
    })
}

/// Sort into the two indexes the snapshot keeps.
pub fn index(mut xrefs: Vec<XRef>) -> (Vec<XRef>, Vec<XRef>) {
    xrefs.sort_by_key(|x| (x.to, x.from, x.kind));
    xrefs.dedup_by_key(|x| (x.to, x.from, x.kind));
    let by_target = xrefs.clone();
    xrefs.sort_by_key(|x| (x.from, x.to, x.kind));
    (by_target, xrefs)
}

/// One of the twelve vector slots that holds a plausible vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorSlot {
    pub name: &'static str,
    pub slot: FileOffset,
    pub target: SnesAddress,
    /// The native RESET and emulation BRK slots are never fetched by the CPU;
    /// they still make xrefs but never name a label.
    pub used: bool,
}

/// Every slot that holds a plausible vector, native then emulation.
pub fn vector_slots(rom: &RomImage) -> Vec<VectorSlot> {
    let h = rom.header();
    let base = rom.header_offset().0;
    let mut out = Vec::with_capacity(12);
    let names_native = ["COP", "BRK", "ABORT", "NMI", "RESET", "IRQ"];
    let names_emu = ["COP", "BRK", "ABORT", "NMI", "RESET", "IRQ"];
    let native = [
        h.native.cop,
        h.native.brk,
        h.native.abort,
        h.native.nmi,
        h.native.reset,
        h.native.irq,
    ];
    let emu = [
        h.emulation.cop,
        h.emulation.brk,
        h.emulation.abort,
        h.emulation.nmi,
        h.emulation.reset,
        h.emulation.irq,
    ];
    for (i, v) in native.iter().enumerate() {
        if *v != 0 && *v != 0xFFFF {
            out.push(VectorSlot {
                name: names_native[i],
                slot: FileOffset(base + 0x24 + 2 * i as u32),
                target: SnesAddress::new(0, *v),
                used: i != 4,
            });
        }
    }
    for (i, v) in emu.iter().enumerate() {
        if *v != 0 && *v != 0xFFFF {
            out.push(VectorSlot {
                name: names_emu[i],
                slot: FileOffset(base + 0x34 + 2 * i as u32),
                target: SnesAddress::new(0, *v),
                used: i != 1,
            });
        }
    }
    out
}
