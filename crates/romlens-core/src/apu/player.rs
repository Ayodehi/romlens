//! A machine to listen to (docs/23, A10): the SPC700 and the DSP started
//! from a recording's frame, from the upload traced in the ROM, or holding
//! one sample on voice 0, run a batch of samples at a time. The app plays
//! what it makes and draws the last of it as scopes; nothing here writes
//! sound out (`12-content-policy.md` rule 11).

use super::render::SAMPLE_RATE;
use super::{Apu, UploadBlock, boot_upload};
use crate::audio::upload::Upload;
use crate::dsp::Frame;
use crate::recording::apu::ApuEventKind;
use crate::recording::{MachineStateSource, RecordingError, SpcState, StateRegion};
use crate::rom::image::RomImage;

/// Samples each scope keeps: 64 ms.
pub const SCOPE_LEN: usize = 2048;

/// How far a player following a recording queues its port writes: ten
/// minutes of frames.
pub const FOLLOW_FRAMES: u64 = 36_000;

/// Where [`Player::sample`] puts its sample, directory and idle loop.
pub const SAMPLE_AT: u16 = 0x1000;
const DIRECTORY_AT: u16 = 0x0200;
const IDLE_AT: u16 = 0x0300;

/// What a player was started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// A recording's frame, its port writes after it followed or not.
    Recording { frame: u64, follow: bool },
    /// The driver the ROM uploads, booted straight into.
    Upload,
    /// One sample on voice 0, keyed by hand.
    Sample,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub apu: Apu,
    pub start: Start,
    /// Samples made so far.
    pub made: u64,
    /// The last [`SCOPE_LEN`] samples of each voice, then left and right,
    /// oldest first from `scope_head`.
    scopes: Vec<[i16; SCOPE_LEN]>,
    scope_head: usize,
}

impl Player {
    fn new(apu: Apu, start: Start) -> Player {
        Player {
            apu,
            start,
            made: 0,
            scopes: vec![[0; SCOPE_LEN]; 10],
            scope_head: 0,
        }
    }

    /// The machine as the recording saw it at `frame`'s end. With
    /// `follow`, the S-CPU's port writes after it land on their cycles, up
    /// to [`FOLLOW_FRAMES`] on. The DSP starts from rest, as in
    /// [`super::render::render`].
    pub fn from_recording(
        src: &dyn MachineStateSource,
        frame: u64,
        follow: bool,
    ) -> Result<Player, RecordingError> {
        let s = src.state_at(frame)?;
        let get = |r: StateRegion| s.region(r).ok_or(RecordingError::MissingRegion(r.name()));
        let spc = SpcState::decode(get(StateRegion::SpcState)?);
        let mut apu = Apu::from_snapshot(
            get(StateRegion::Aram)?,
            get(StateRegion::DspRegisters)?,
            &spc,
        );
        if follow {
            let last = src.frame_count().unwrap_or(0).min(frame + FOLLOW_FRAMES);
            for f in frame..last {
                if let Some(e) = src.apu_events(f)? {
                    for e in e.events.iter().filter(|e| e.kind == ApuEventKind::CpuPort) {
                        if e.spc_cycle >= spc.cycle {
                            apu.queue_port(e.spc_cycle, e.address, e.value);
                        }
                    }
                }
            }
        }
        Ok(Player::new(apu, Start::Recording { frame, follow }))
    }

    /// The driver `driver` uploads, booted straight into, with `more`
    /// uploads laid over it (the songs and samples a game sends a running
    /// driver), run `settle` seconds so it sets the DSP up before a
    /// command. Those seconds are not heard.
    pub fn from_upload(rom: &RomImage, driver: &Upload, more: &[&Upload], settle: f64) -> Player {
        let mut apu = Apu::new();
        boot_upload(&mut apu, &upload_blocks(rom, driver, more), driver.entry);
        let until = apu.bus.cycle + (settle.max(0.0) * 1_024_000.0) as u64;
        apu.run_until(until);
        Player::new(apu, Start::Upload)
    }

    /// `brr` alone on voice 0: the sample at [`SAMPLE_AT`], its directory
    /// entry looping to `loop_offset` bytes in (a whole block), played at
    /// `pitch` with the envelope at full by direct gain, nothing else
    /// sounding, and the SPC700 idling. Keyed with [`Player::key_on`].
    pub fn sample(brr: &[u8], loop_offset: usize, pitch: u16) -> Player {
        let mut apu = Apu::new();
        let room = 0x10000 - SAMPLE_AT as usize;
        let len = (brr.len() / 9 * 9).min(room / 9 * 9);
        let aram = &mut apu.bus.aram;
        aram[SAMPLE_AT as usize..SAMPLE_AT as usize + len].copy_from_slice(&brr[..len]);
        let loop_at = SAMPLE_AT.wrapping_add((loop_offset.min(len) / 9 * 9) as u16);
        let d = DIRECTORY_AT as usize;
        aram[d..d + 2].copy_from_slice(&SAMPLE_AT.to_le_bytes());
        aram[d + 2..d + 4].copy_from_slice(&loop_at.to_le_bytes());
        // BRA to itself.
        aram[IDLE_AT as usize] = 0x2F;
        aram[IDLE_AT as usize + 1] = 0xFE;
        apu.cpu.pc = IDLE_AT;
        apu.bus.io.rom_enabled = false;
        let r = &mut apu.bus.dsp;
        r[0x5D] = (DIRECTORY_AT >> 8) as u8;
        r[0x00] = 0x7F;
        r[0x01] = 0x7F;
        r[0x04] = 0;
        r[0x05] = 0;
        r[0x07] = 0x7F;
        r[0x0C] = 0x7F;
        r[0x1C] = 0x7F;
        // Echo writes off; not muted, not reset.
        r[0x6C] = 0x20;
        let mut p = Player::new(apu, Start::Sample);
        p.set_pitch(0, pitch);
        p
    }

    /// Key voices on, as a write of `mask` to KON.
    pub fn key_on(&mut self, mask: u8) {
        let dsp = &mut self.apu.bus.dsp;
        dsp[0x5C] &= !mask;
        dsp[0x4C] = mask;
        self.apu.bus.chip.write_kon(mask);
    }

    /// Key voices off, as a write of `mask` to KOFF.
    pub fn key_off(&mut self, mask: u8) {
        self.apu.bus.dsp[0x5C] |= mask;
    }

    pub fn set_pitch(&mut self, voice: u8, pitch: u16) {
        let b = (voice as usize & 7) << 4;
        let pitch = pitch & 0x3FFF;
        self.apu.bus.dsp[b | 2] = pitch as u8;
        self.apu.bus.dsp[b | 3] = (pitch >> 8) as u8;
    }

    /// The S-CPU writes a port now.
    pub fn send_port(&mut self, port: u8, value: u8) {
        self.apu.write_port(port as usize, value);
    }

    /// Voices left out of the mix, bit *n* for voice *n*. They still run,
    /// and their scopes still show them.
    pub fn set_muted(&mut self, mask: u8) {
        self.apu.bus.chip.muted = mask;
    }

    pub fn muted(&self) -> u8 {
        self.apu.bus.chip.muted
    }

    /// The next `n` samples.
    pub fn render(&mut self, n: usize) -> Vec<Frame> {
        let mut out = self.apu.bus.output.take().unwrap_or_default();
        out.clear();
        out.reserve(n);
        self.apu.bus.output = Some(out);
        while self.apu.bus.output.as_ref().is_some_and(|o| o.len() < n) {
            self.apu.cpu.step(&mut self.apu.bus);
        }
        let mut out = self.apu.bus.output.take().unwrap_or_default();
        // An instruction can run past the last sample asked for: keep it
        // for the next batch.
        let rest = out.split_off(n.min(out.len()));
        self.apu.bus.output = Some(rest);
        for f in &out {
            let h = self.scope_head;
            for v in 0..8 {
                self.scopes[v][h] = f.voices[v];
            }
            self.scopes[8][h] = f.left;
            self.scopes[9][h] = f.right;
            self.scope_head = (h + 1) % SCOPE_LEN;
        }
        self.made += out.len() as u64;
        out
    }

    /// The last `n` samples (at most [`SCOPE_LEN`]) of voice 0–7, or 8 for
    /// the left output and 9 for the right, oldest first.
    pub fn scope(&self, which: usize, n: usize) -> Vec<i16> {
        let Some(s) = self.scopes.get(which) else {
            return Vec::new();
        };
        let n = n.min(SCOPE_LEN);
        (0..n)
            .map(|i| s[(self.scope_head + SCOPE_LEN - n + i) % SCOPE_LEN])
            .collect()
    }

    /// Seconds played.
    pub fn seconds(&self) -> f64 {
        self.made as f64 / SAMPLE_RATE as f64
    }

    /// The voices as the DSP's registers set them now.
    pub fn voices(&self) -> Vec<crate::audio::Voice> {
        crate::audio::voices(&self.apu.bus.dsp, Some(&self.apu.bus.aram))
    }
}

/// The blocks [`Player::from_upload`] would boot, for callers that boot
/// an [`Apu`] of their own.
pub fn upload_blocks(rom: &RomImage, driver: &Upload, more: &[&Upload]) -> Vec<UploadBlock> {
    let mut blocks = driver.apu_blocks(rom);
    for u in more {
        blocks.extend(u.apu_blocks(rom));
    }
    blocks
}
