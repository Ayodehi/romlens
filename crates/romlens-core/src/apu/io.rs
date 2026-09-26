//! What the SPC700 sees at `$F0–$FF`: the timers, the four ports each way,
//! CONTROL, the DSP's address and data, and the two spare registers
//! (fullsnes, "SNES APU I/O Ports").

/// One timer: a clock (8 kHz for timers 0 and 1, 64 kHz for timer 2),
/// divided by `divider`, counted in a 4-bit output the driver reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timer {
    /// CONTROL's bit for this timer.
    pub enabled: bool,
    /// `TnDIV`: the clock ticks per count; 0 is 256.
    pub divider: u8,
    /// Ticks since the output last went up.
    pub counter: u8,
    /// `TnOUT`, 0–15; reading it clears it.
    pub output: u8,
    /// Cycles since the clock last ticked.
    pub phase: u8,
}

impl Timer {
    /// One SPC700 cycle; the clock ticks every `period` of them.
    fn cycle(&mut self, period: u8) {
        self.phase += 1;
        if self.phase < period {
            return;
        }
        self.phase = 0;
        if self.enabled {
            self.counter = self.counter.wrapping_add(1);
            if self.counter == self.divider {
                self.counter = 0;
                self.output = (self.output + 1) & 0xF;
            }
        }
    }
}

/// Cycles per clock tick: 1.024 MHz down to 8 kHz, 8 kHz and 64 kHz.
pub const TIMER_PERIODS: [u8; 3] = [128, 128, 16];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Io {
    /// What the S-CPU last wrote to each port: what the SPC700 reads.
    pub from_cpu: [u8; 4],
    /// What the SPC700 last wrote to each port: what the S-CPU reads.
    pub to_cpu: [u8; 4],
    pub aux: [u8; 2],
    pub dspaddr: u8,
    /// CONTROL bit 7: the boot ROM at `$FFC0` (writes still go to RAM).
    pub rom_enabled: bool,
    pub timers: [Timer; 3],
}

impl Default for Io {
    /// As at power on: the boot ROM mapped, the timers off.
    fn default() -> Self {
        Io {
            from_cpu: [0; 4],
            to_cpu: [0; 4],
            aux: [0; 2],
            dspaddr: 0,
            rom_enabled: true,
            timers: [Timer::default(); 3],
        }
    }
}

impl Io {
    pub fn cycle(&mut self) {
        for (t, p) in self.timers.iter_mut().zip(TIMER_PERIODS) {
            t.cycle(p);
        }
    }

    /// CONTROL: a timer switched on starts counting from zero; bits 4 and
    /// 5 clear the ports from the S-CPU.
    pub fn write_control(&mut self, v: u8) {
        for (i, t) in self.timers.iter_mut().enumerate() {
            let on = v & (1 << i) != 0;
            if on && !t.enabled {
                t.counter = 0;
                t.output = 0;
            }
            t.enabled = on;
        }
        if v & 0x10 != 0 {
            self.from_cpu[0] = 0;
            self.from_cpu[1] = 0;
        }
        if v & 0x20 != 0 {
            self.from_cpu[2] = 0;
            self.from_cpu[3] = 0;
        }
        self.rom_enabled = v & 0x80 != 0;
    }
}
