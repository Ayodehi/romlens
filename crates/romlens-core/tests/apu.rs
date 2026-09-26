//! The sound CPU as a machine (docs/23, A7): its timers, ports and
//! CONTROL, our boot program's upload protocol, and the boot straight into
//! a driver. The instructions themselves are checked against the
//! single-step suite (`spc700_single_step.rs`).

use romlens_core::apu::{Apu, SpcBus, UploadBlock, boot_upload, ipl};
use romlens_core::spc700::assemble;

/// An APU running `source` from its first `.org`, with the boot ROM off.
fn running(source: &str) -> Apu {
    let program = assemble(source).unwrap();
    let mut apu = Apu::new();
    apu.bus.aram.copy_from_slice(&program.image());
    apu.bus.io.rom_enabled = false;
    apu.cpu.pc = program.chunks[0].0;
    apu.cpu.sp = 0xEF;
    apu
}

fn run_cycles(apu: &mut Apu, n: u64) {
    let until = apu.bus.cycle + n;
    apu.run_until(until);
}

mod timers {
    use super::*;

    #[test]
    fn timer_0_counts_its_divider_of_8_khz_ticks() {
        // Divider 16: one count every 16 × 128 = 2,048 cycles.
        let mut apu = running(
            "
            .org $0200
                MOV T0DIV,#$10
                MOV CONTROL,#$01
            loop:
                BRA loop
            ",
        );
        apu.run_until(8);
        let enabled_at = apu.bus.cycle;
        let t = apu.bus.io.timers[0];
        assert!(t.enabled && t.counter == 0 && t.output == 0);
        // The clock ticks when its phase comes round, so the first count
        // lands between 15 and 16 periods on.
        run_cycles(&mut apu, 15 * 128 - 8);
        assert_eq!(apu.bus.io.timers[0].output, 0);
        apu.run_until(enabled_at + 16 * 128 + 4);
        assert_eq!(apu.bus.io.timers[0].output, 1);
        apu.run_until(enabled_at + 16 * 128 * 16 + 4);
        assert_eq!(
            apu.bus.io.timers[0].output, 0,
            "4 bits: the 16th count wraps"
        );
    }

    #[test]
    fn timer_2_ticks_eight_times_faster_and_divider_0_is_256() {
        let mut apu = running(
            "
            .org $0200
                MOV T2DIV,#$00
                MOV CONTROL,#$04
            loop:
                BRA loop
            ",
        );
        apu.run_until(8);
        let from = apu.bus.cycle;
        apu.run_until(from + 255 * 16 - 16);
        assert_eq!(apu.bus.io.timers[2].output, 0);
        apu.run_until(from + 256 * 16 + 4);
        assert_eq!(apu.bus.io.timers[2].output, 1);
    }

    #[test]
    fn reading_an_output_clears_it_and_re_enabling_restarts_it() {
        let mut apu = running(
            "
            .org $0200
                MOV T1DIV,#$01
                MOV CONTROL,#$02
            wait:
                MOV A,T1OUT
                BEQ wait
                MOV X,T1OUT      ; read straight after: cleared
                MOV CONTROL,#$00
                MOV CONTROL,#$02 ; off and on again: counts from zero
            loop:
                BRA loop
            ",
        );
        run_cycles(&mut apu, 400);
        assert_eq!(apu.cpu.a, 1);
        assert_eq!(apu.cpu.x, 0);
        assert_eq!(apu.bus.io.timers[1].counter, 0);
    }
}

mod io {
    use super::*;

    #[test]
    fn the_ports_go_both_ways_and_control_clears_the_inputs() {
        let mut apu = running(
            "
            .org $0200
                MOV A,CPUIO0
                MOV CPUIO1,A       ; answer on port 1
                MOV CONTROL,#$30   ; clear what the S-CPU wrote
                MOV X,CPUIO0
                MOV Y,CPUIO3
            loop:
                BRA loop
            ",
        );
        apu.write_port(0, 0x5A);
        apu.write_port(3, 0xA5);
        run_cycles(&mut apu, 40);
        assert_eq!(apu.read_port(1), 0x5A);
        assert_eq!((apu.cpu.x, apu.cpu.y), (0, 0));
        assert!(!apu.bus.io.rom_enabled, "CONTROL bit 7 clear: RAM at $FFC0");
    }

    #[test]
    fn a_port_write_lands_on_its_cycle() {
        // The byte is there from the cycle it is queued for, even in the
        // middle of an instruction.
        let mut apu = running(
            "
            .org $0200
            wait:
                MOV A,CPUIO0
                BEQ wait
            ",
        );
        let first = apu.bus.cycle;
        apu.queue_port(first + 10, 0, 0x42);
        apu.run_until(first + 40);
        assert_eq!(apu.cpu.a, 0x42);
        assert!(apu.bus.port_writes.is_empty());
    }

    #[test]
    fn the_dsp_registers_through_f2_and_f3() {
        let mut apu = running(
            "
            .org $0200
                MOV DSPADDR,#$5D
                MOV DSPDATA,#$3C      ; DIR
                MOV DSPADDR,#$DD      ; a read-only mirror of $5D
                MOV DSPDATA,#$00      ; ignored
                MOV A,DSPDATA
                MOV DSPADDR,#$7C
                MOV DSPDATA,#$FF      ; writing ENDX clears it
            loop:
                BRA loop
            ",
        );
        apu.bus.dsp[0x7C] = 0x81;
        run_cycles(&mut apu, 60);
        assert_eq!(apu.bus.dsp[0x5D], 0x3C);
        assert_eq!(apu.cpu.a, 0x3C, "the mirror reads $5D");
        assert_eq!(apu.bus.dsp[0x7C], 0);
    }

    #[test]
    fn writes_to_io_and_under_the_boot_rom_reach_ram() {
        let mut apu = running(
            "
            .org $0200
                MOV CPUIO2,#$77
                MOV A,#$99
                MOV !$FFC0,A
            loop:
                BRA loop
            ",
        );
        apu.bus.io.rom_enabled = true;
        run_cycles(&mut apu, 30);
        assert_eq!(apu.bus.aram[0xF6], 0x77);
        assert_eq!(apu.bus.aram[0xFFC0], 0x99);
        assert_eq!(
            apu.bus.peek(0xFFC0),
            ipl()[0],
            "the boot ROM still reads over it"
        );
    }
}

mod boot {
    use super::*;

    /// The S-CPU's side of the upload protocol, as a game runs it: each
    /// step waits for the SPC700's answer before the next.
    fn upload(apu: &mut Apu, blocks: &[UploadBlock], entry: u16) {
        let wait = |apu: &mut Apu, port: usize, value: u8| {
            let give_up = apu.bus.cycle + 100_000;
            while apu.read_port(port) != value {
                assert!(
                    apu.bus.cycle < give_up,
                    "no answer {value:#04x} on port {port}"
                );
                apu.cpu.step(&mut apu.bus);
            }
        };
        wait(apu, 0, 0xAA);
        wait(apu, 1, 0xBB);
        let mut kick = 0xCCu8;
        for b in blocks {
            let [lo, hi] = b.at.to_le_bytes();
            apu.write_port(2, lo);
            apu.write_port(3, hi);
            apu.write_port(1, 1);
            apu.write_port(0, kick);
            wait(apu, 0, kick);
            for (i, v) in b.bytes.iter().enumerate() {
                apu.write_port(1, *v);
                apu.write_port(0, i as u8);
                wait(apu, 0, i as u8);
            }
            // The next command is past the last index, and never zero.
            kick = (b.bytes.len() as u8).wrapping_add(1).max(1);
        }
        let [lo, hi] = entry.to_le_bytes();
        apu.write_port(2, lo);
        apu.write_port(3, hi);
        apu.write_port(1, 0);
        apu.write_port(0, kick);
        wait(apu, 0, kick);
    }

    fn blocks() -> Vec<UploadBlock> {
        // A driver that says it is running on port 3; and a block of 300
        // bytes, so the index wraps and the page goes on.
        let driver = assemble(
            "
            .org $0400
                MOV CPUIO3,#$5A
            loop:
                BRA loop
            ",
        )
        .unwrap();
        vec![
            UploadBlock {
                at: 0x0400,
                bytes: driver.chunks[0].1.clone(),
            },
            UploadBlock {
                at: 0x3000,
                bytes: (0..300u32).map(|i| (i * 7) as u8).collect(),
            },
        ]
    }

    #[test]
    fn our_boot_program_is_64_bytes_that_start_at_ffc0() {
        let rom = ipl();
        assert_eq!(u16::from_le_bytes([rom[62], rom[63]]), 0xFFC0);
        let apu = Apu::new();
        assert_eq!(apu.cpu.pc, 0xFFC0);
        assert!(apu.bus.io.rom_enabled);
    }

    #[test]
    fn it_takes_an_upload_and_runs_it() {
        let mut apu = Apu::new();
        let blocks = blocks();
        upload(&mut apu, &blocks, 0x0400);
        for b in &blocks {
            let at = b.at as usize;
            assert_eq!(
                &apu.bus.aram[at..at + b.bytes.len()],
                &b.bytes[..],
                "block at ${at:04X}"
            );
        }
        let until = apu.bus.cycle + 200;
        apu.run_until(until);
        assert_eq!(apu.read_port(3), 0x5A, "the driver runs");
        // It cleared the direct page before the upload.
        assert!(apu.bus.aram[0x10..0xEF].iter().all(|&v| v == 0));
    }

    #[test]
    fn booting_straight_into_the_driver_leaves_the_same_machine() {
        let blocks = blocks();
        let mut hle = Apu::new();
        boot_upload(&mut hle, &blocks, 0x0400);
        assert_eq!((hle.cpu.pc, hle.cpu.sp), (0x0400, 0xEF));
        let until = hle.bus.cycle + 200;
        hle.run_until(until);
        assert_eq!(hle.read_port(3), 0x5A);
        let mut real = Apu::new();
        upload(&mut real, &blocks, 0x0400);
        for b in &blocks {
            let r = b.at as usize..b.at as usize + b.bytes.len();
            assert_eq!(hle.bus.aram[r.clone()], real.bus.aram[r]);
        }
    }
}

#[test]
fn every_cycle_is_one_bus_call() {
    // NOP is two cycles: the opcode and a dummy read of the next byte.
    let mut apu = running(".org $0200\n NOP\n NOP\n");
    let from = apu.bus.cycle;
    apu.cpu.step(&mut apu.bus);
    assert_eq!(apu.bus.cycle - from, 2);
    // A bus is anything that answers reads and writes: a flat one works.
    struct Count(u32);
    impl SpcBus for Count {
        fn read(&mut self, _: u16) -> u8 {
            self.0 += 1;
            0
        }
        fn write(&mut self, _: u16, _: u8) {
            self.0 += 1;
        }
        fn idle(&mut self) {
            self.0 += 1;
        }
    }
    let mut bus = Count(0);
    let mut cpu = romlens_core::apu::Spc700::default();
    cpu.step(&mut bus);
    assert_eq!(bus.0, 2);
}
