//! The sound side as a machine (docs/23, A7): the SPC700, its 64 KB of
//! audio RAM, its I/O and timers, and the boot program, cycle by cycle.
//!
//! The DSP runs beside it on the same clock (A8): a sample every 32
//! cycles, from the registers the SPC700 writes through `$F2`/`$F3`.

pub mod cpu;
pub mod io;
pub mod ipl;
pub mod render;
pub mod replay;

pub use cpu::{Spc700, SpcBus};
pub use io::{Io, TIMER_PERIODS, Timer};
pub use ipl::{UploadBlock, boot_upload, ipl};

use crate::recording::SpcState;

/// A write the SPC700 made to `$F0–$FF`, on its own clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoWrite {
    pub cycle: u64,
    /// `$F0` + this.
    pub register: u8,
    pub value: u8,
}

/// Audio RAM, the I/O and the DSP's registers, as the SPC700's bus.
#[derive(Debug, Clone)]
pub struct ApuBus {
    pub aram: Vec<u8>,
    /// The DSP's 128 registers, as the SPC700 reads and writes them.
    pub dsp: [u8; 128],
    /// The DSP's inside: its voices, envelopes and echo.
    pub chip: crate::dsp::Dsp,
    /// Cycles into the DSP's current sample, 0-31.
    pub dsp_clock: u8,
    /// The samples made, when kept.
    pub output: Option<Vec<crate::dsp::Frame>>,
    pub io: Io,
    /// SPC700 cycles run, at 1.024 MHz.
    pub cycle: u64,
    /// Every write to `$F0–$FF`, when kept.
    pub io_writes: Option<Vec<IoWrite>>,
    /// What each write replaced, when kept, so an instruction can be undone.
    pub undo: Option<Vec<Undo>>,
    /// Writes the S-CPU makes to the ports, each on the SPC700 cycle it
    /// lands on, in order.
    pub port_writes: std::collections::VecDeque<PortWrite>,
}

/// The S-CPU writing a port at a given SPC700 cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortWrite {
    pub cycle: u64,
    pub port: u8,
    pub value: u8,
}

/// A byte a write replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    Aram(u16, u8),
    Dsp(u8, u8),
}

impl ApuBus {
    /// One cycle: the S-CPU's writes up to now land first, so an access
    /// in this cycle sees them.
    fn tick(&mut self) {
        while let Some(w) = self.port_writes.front().copied() {
            if w.cycle >= self.cycle {
                break;
            }
            self.io.from_cpu[w.port as usize & 3] = w.value;
            self.port_writes.pop_front();
        }
        self.cycle += 1;
        self.io.cycle();
        if let Some(f) = self
            .chip
            .step(self.dsp_clock, &mut self.dsp, &mut self.aram)
            && let Some(out) = &mut self.output
        {
            out.push(f);
        }
        self.dsp_clock = (self.dsp_clock + 1) & 31;
    }

    /// Put the DSP where it is `cycle` SPC700 cycles after power on.
    pub fn set_dsp_clock(&mut self, cycle: u64) {
        let samples = cycle / 32;
        self.dsp_clock = (cycle % 32) as u8;
        self.chip.counter = crate::dsp::chip::counter_after(samples);
        self.chip.even = samples % 2 == 1;
    }

    /// A byte as the SPC700 would read it, without the read's side effects.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0xF0..=0xFF => self.peek_io(addr as u8 & 0xF),
            0xFFC0.. if self.io.rom_enabled => ipl()[addr as usize - 0xFFC0],
            _ => self.aram[addr as usize],
        }
    }

    fn peek_io(&self, r: u8) -> u8 {
        match r {
            2 => self.io.dspaddr,
            3 => self.dsp[self.io.dspaddr as usize & 0x7F],
            4..=7 => self.io.from_cpu[r as usize - 4],
            8 | 9 => self.io.aux[r as usize - 8],
            0xD..=0xF => self.io.timers[r as usize - 0xD].output,
            // TEST, CONTROL and the dividers cannot be read back.
            _ => 0,
        }
    }
}

impl SpcBus for ApuBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.tick();
        let v = self.peek(addr);
        if let 0xFD..=0xFF = addr {
            self.io.timers[addr as usize - 0xFD].output = 0;
        }
        v
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.tick();
        if let Some(u) = &mut self.undo {
            u.push(Undo::Aram(addr, self.aram[addr as usize]));
            if addr == 0xF3 && self.io.dspaddr < 0x80 {
                u.push(Undo::Dsp(
                    self.io.dspaddr,
                    self.dsp[self.io.dspaddr as usize],
                ));
            }
        }
        // Writes to the I/O registers and under the boot ROM reach RAM too.
        self.aram[addr as usize] = value;
        if !(0xF0..=0xFF).contains(&addr) {
            return;
        }
        let r = addr as u8 & 0xF;
        if let Some(log) = &mut self.io_writes {
            log.push(IoWrite {
                cycle: self.cycle,
                register: r,
                value,
            });
        }
        match r {
            1 => self.io.write_control(value),
            2 => self.io.dspaddr = value,
            3 => {
                // $80–$FF are read-only mirrors.
                let d = self.io.dspaddr;
                if d < 0x80 {
                    self.dsp[d as usize] = value;
                    match d {
                        // Writing ENDX clears it.
                        0x7C => self.dsp[0x7C] = 0,
                        0x4C => self.chip.write_kon(value),
                        _ => {}
                    }
                }
            }
            4..=7 => self.io.to_cpu[r as usize - 4] = value,
            8 | 9 => self.io.aux[r as usize - 8] = value,
            0xA..=0xC => self.io.timers[r as usize - 0xA].divider = value,
            // TEST, and the outputs, which cannot be written.
            _ => {}
        }
    }

    fn idle(&mut self) {
        self.tick();
    }
}

/// The SPC700 and everything on its bus.
#[derive(Debug, Clone)]
pub struct Apu {
    pub cpu: Spc700,
    pub bus: ApuBus,
}

impl Default for Apu {
    fn default() -> Self {
        Apu::new()
    }
}

impl Apu {
    /// Power on: RAM zeroed, the SPC700 at the start of our boot program.
    pub fn new() -> Apu {
        let mut apu = Apu {
            cpu: Spc700::default(),
            bus: ApuBus {
                aram: vec![0; 0x10000],
                dsp: [0; 128],
                chip: crate::dsp::Dsp::default(),
                dsp_clock: 0,
                output: None,
                io: Io::default(),
                cycle: 0,
                io_writes: None,
                undo: None,
                port_writes: Default::default(),
            },
        };
        // The DSP comes up keyed off, muted and not writing echo, as if FLG
        // held $E0 (anomie).
        apu.bus.dsp[0x6C] = 0xE0;
        apu.cpu.reset(&mut apu.bus);
        apu.bus.cycle = 0;
        apu.bus.dsp_clock = 0;
        apu
    }

    /// The machine as a recording saw it at a frame's end.
    pub fn from_snapshot(aram: &[u8], dsp: &[u8], s: &SpcState) -> Apu {
        let mut apu = Apu::new();
        apu.bus.aram.copy_from_slice(&aram[..0x10000]);
        apu.bus.dsp.copy_from_slice(&dsp[..128]);
        apu.cpu = Spc700 {
            a: s.a,
            x: s.x,
            y: s.y,
            sp: s.sp,
            psw: s.psw,
            pc: s.pc,
            halted: false,
        };
        let io = &mut apu.bus.io;
        io.from_cpu = s.from_cpu;
        io.to_cpu = s.to_cpu;
        io.aux = s.aux;
        io.dspaddr = s.dspaddr;
        io.rom_enabled = s.rom_enabled;
        for i in 0..3 {
            io.timers[i] = Timer {
                enabled: s.timers_on[i],
                divider: s.dividers[i],
                counter: s.timer_counter[i],
                output: s.counts[i],
                phase: s.timer_phase[i],
            };
        }
        apu.bus.cycle = s.cycle;
        // The DSP's clock runs from power on, as the SPC700's cycle count
        // does: the samples since then place its step within a sample, its
        // global counter and which sample polls KON.
        apu.bus.set_dsp_clock(s.cycle);
        apu
    }

    /// Run whole instructions until at least `cycle`.
    pub fn run_until(&mut self, cycle: u64) {
        while self.bus.cycle < cycle {
            self.cpu.step(&mut self.bus);
        }
    }

    /// The S-CPU writes a port (`$2140` + `port`), now.
    pub fn write_port(&mut self, port: usize, value: u8) {
        self.bus.io.from_cpu[port & 3] = value;
    }

    /// The S-CPU will write a port when the SPC700's clock reaches `cycle`.
    /// Writes must be queued in order.
    pub fn queue_port(&mut self, cycle: u64, port: u8, value: u8) {
        self.bus
            .port_writes
            .push_back(PortWrite { cycle, port, value });
    }

    /// What the S-CPU reads from a port.
    pub fn read_port(&self, port: usize) -> u8 {
        self.bus.io.to_cpu[port & 3]
    }
}
