//! The synthetic recording `romlens testrec` writes: the graphics fixture's
//! machine, animated. The recording twin of `testrom`, so every recording
//! golden and bug report works with no ROM and no committed binary.

use sha2::{Digest, Sha256};

use crate::fixtures::graphics::{graphics_lorom, machine};
use crate::recording::{CpuRegisters, MachineState, RecordingIdentity, StateRegion};

pub const PRODUCER: &str = "romlens testrec";

/// The identity: the graphics fixture ROM.
pub fn identity() -> RecordingIdentity {
    RecordingIdentity {
        rom_sha256: Sha256::digest(graphics_lorom()).into(),
        producer: PRODUCER.to_owned(),
        producer_version: crate::API_VERSION.to_owned(),
    }
}

/// `frames` consecutive states. Each frame moves sprite 0 right by one pixel
/// and scrolls BG1 by one; every 8 frames sprite 1 changes palette; at frame
/// 30 tile 5 is rewritten in VRAM, the way a DMA would; CGRAM entry 17
/// cycles. WRAM holds the frame counter at `$7E:0000`.
pub fn frames(frames: u32) -> Vec<MachineState> {
    let (vram, cgram, oam, ppu) = machine();
    let mut out = Vec::with_capacity(frames as usize);
    for f in 0..frames {
        let mut state = MachineState {
            frame: f as u64,
            ..MachineState::default()
        };
        // PB:PC = $80:800A, the fixture's spin loop, with M and X set.
        let cpu = CpuRegisters {
            pb: 0x80,
            pc: 0x800A,
            p: 0x30,
            s: 0x1FFF,
            ..CpuRegisters::default()
        }
        .encode()
        .to_vec();
        let mut ppu = ppu.clone();
        ppu.set_scroll(1, false, f as u16);
        let mut oam = oam.clone();
        oam[0] = oam[0].wrapping_add(f as u8);
        oam[7] = (oam[7] & !0x0E) | (((f / 8) as u8 % 8) << 1);
        let mut cgram = cgram.clone();
        let c = (0x001Fu16 >> (f % 5)).to_le_bytes();
        cgram[34..36].copy_from_slice(&c);
        let mut vram = vram.clone();
        if f >= 30 {
            // Tile 5 (bytes $A0–$BF) becomes a solid block of index 15.
            vram[0xA0..0xC0].fill(0xFF);
        }
        let mut wram = vec![0u8; StateRegion::Wram.size()];
        wram[0..4].copy_from_slice(&f.to_le_bytes());
        let mut timing = vec![0u8; 16];
        timing[0..8].copy_from_slice(&(f as u64).to_le_bytes());
        state.regions.insert(StateRegion::CpuRegisters, cpu);
        state
            .regions
            .insert(StateRegion::PpuState, ppu.bytes.to_vec());
        state.regions.insert(StateRegion::IoState, vec![0u8; 128]);
        state.regions.insert(StateRegion::Wram, wram);
        state.regions.insert(StateRegion::Vram, vram);
        state.regions.insert(StateRegion::Cgram, cgram);
        state.regions.insert(StateRegion::Oam, oam);
        state.regions.insert(StateRegion::Timing, timing);
        out.push(state);
    }
    out
}
