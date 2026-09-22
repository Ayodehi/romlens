//! Linear sweep of the gaps directly after code, accepted only when the
//! bytes look unmistakably like a routine. Never seeds entries or labels.

use crate::analysis::control::{AnalysisControl, AnalysisPhase, Cancelled};
use crate::analysis::descent::{KIND_NONE, KIND_OPCODE, KIND_OPERAND, OVR_NONE, Walk};
use crate::analysis::snapshot::InsnRecord;
use crate::cpu65816::{Mnemonic, decode};
use crate::memory::address::FileOffset;
use crate::memory::map::MappingMode;

pub const MIN_GAP: u32 = 4;
pub const SWEEP_CONFIDENCE: f32 = 0.3;

/// Records the sweep accepted (kept apart so regions can rate them 0.3).
pub fn sweep(walk: &mut Walk<'_>, control: &AnalysisControl) -> Result<Vec<InsnRecord>, Cancelled> {
    let rom = walk.rom;
    let bytes = rom.bytes();
    let n = bytes.len() as u32;
    let bank = match rom.mapping() {
        MappingMode::LoRom => 0x8000u32,
        _ => 0x1_0000,
    };
    // Descent records sorted so the flags at a run's end can be found.
    let mut sorted: Vec<InsnRecord> = walk.records.iter().map(|(r, _)| *r).collect();
    sorted.sort_by_key(|r| r.offset);
    let mut accepted = Vec::new();
    let mut off = 0u32;
    let mut gaps = 0u64;
    while off < n {
        // Find the end of the current code run.
        if walk.kind[off as usize] == KIND_NONE {
            off += 1;
            continue;
        }
        while off < n && walk.kind[off as usize] != KIND_NONE {
            off += 1;
        }
        let run_end = off;
        // The gap: until the next decoded byte, override, bank edge or end.
        let bank_end = (run_end / bank + 1) * bank;
        let mut gap_end = run_end;
        while gap_end < n
            && gap_end < bank_end
            && walk.kind[gap_end as usize] == KIND_NONE
            && walk.ovr[gap_end as usize] == OVR_NONE
        {
            gap_end += 1;
        }
        if gap_end - run_end < MIN_GAP {
            off = gap_end;
            continue;
        }
        gaps += 1;
        if gaps.is_multiple_of(256) {
            control.check()?;
            control.report(AnalysisPhase::Sweep, run_end as u64, n as u64);
        }
        // Flags after the last instruction of the run.
        let i = sorted.partition_point(|r| r.offset < run_end);
        let Some(last) = i
            .checked_sub(1)
            .map(|i| sorted[i])
            .filter(|r| r.end() == run_end)
        else {
            off = gap_end;
            continue;
        };
        let Some(last_insn) = walk_decode(walk, &last) else {
            off = gap_end;
            continue;
        };
        let mut flags = last_insn.flags_after;
        let mut pos = run_end;
        let mut run: Vec<InsnRecord> = Vec::new();
        let mut ok = false;
        while pos < gap_end {
            let Some(addr) = rom.snes_address_for(FileOffset(pos)) else {
                break;
            };
            let Some(insn) = decode(&bytes[pos as usize..], addr, FileOffset(pos), flags) else {
                break;
            };
            let end = pos + insn.len as u32;
            if end > gap_end {
                break;
            }
            if matches!(
                insn.mnemonic,
                Mnemonic::BRK | Mnemonic::WDM | Mnemonic::STP | Mnemonic::COP
            ) {
                break;
            }
            if let Some(t) = insn.target
                && (insn.mnemonic.is_branch() || insn.mnemonic.is_jump() || insn.mnemonic.is_call())
                && rom.file_offset_for(t.address).is_none()
            {
                break;
            }
            run.push(InsnRecord::from_instruction(&insn));
            flags = insn.flags_after;
            pos = end;
            if insn.mnemonic.is_block_end() {
                ok = true;
                break;
            }
        }
        if ok && run.len() >= 2 {
            for r in &run {
                walk.kind[r.offset as usize] = KIND_OPCODE;
                walk.wkey[r.offset as usize] = r.flags_before().width_key() + 1;
                for b in r.offset + 1..r.end() {
                    walk.kind[b as usize] = KIND_OPERAND;
                }
            }
            let end = run.last().map(|r| r.end()).unwrap_or(run_end);
            let i = sorted.partition_point(|r| r.offset < run_end);
            sorted.splice(i..i, run.iter().copied());
            accepted.extend(run);
            // Continue directly after the accepted run: it is code now.
            off = end;
        } else {
            off = gap_end;
        }
    }
    control.report(AnalysisPhase::Sweep, n as u64, n as u64);
    Ok(accepted)
}

fn walk_decode(walk: &Walk<'_>, rec: &InsnRecord) -> Option<crate::cpu65816::Instruction> {
    let off = FileOffset(rec.offset);
    let addr = walk.rom.snes_address_for(off)?;
    decode(
        &walk.rom.bytes()[rec.offset as usize..],
        addr,
        off,
        rec.flags_before(),
    )
}
