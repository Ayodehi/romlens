//! The SPC700's I/O registers, `$F0–$FF` of its address space.
//!
//! Names follow fullsnes. The SPC700 reaches them as direct-page bytes
//! while P is clear (page 0), which is how every driver runs, or with an
//! absolute address.

/// Whether the SPC700 reads a register, writes it, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoRegister {
    pub address: u16,
    pub name: &'static str,
    pub access: Access,
    pub description: &'static str,
}

const fn reg(
    address: u16,
    name: &'static str,
    access: Access,
    description: &'static str,
) -> IoRegister {
    IoRegister {
        address,
        name,
        access,
        description,
    }
}

use Access::*;

pub static IO_REGISTERS: [IoRegister; 16] = [
    reg(
        0xF0,
        "TEST",
        Write,
        "Test settings; drivers leave it at $0A",
    ),
    reg(
        0xF1,
        "CONTROL",
        Write,
        "Timer enables, port clears, boot ROM at $FFC0",
    ),
    reg(
        0xF2,
        "DSPADDR",
        ReadWrite,
        "Which DSP register DSPDATA reads or writes",
    ),
    reg(
        0xF3,
        "DSPDATA",
        ReadWrite,
        "The DSP register DSPADDR selects",
    ),
    reg(
        0xF4,
        "CPUIO0",
        ReadWrite,
        "Port 0: read the S-CPU's byte, write one back",
    ),
    reg(
        0xF5,
        "CPUIO1",
        ReadWrite,
        "Port 1: read the S-CPU's byte, write one back",
    ),
    reg(
        0xF6,
        "CPUIO2",
        ReadWrite,
        "Port 2: read the S-CPU's byte, write one back",
    ),
    reg(
        0xF7,
        "CPUIO3",
        ReadWrite,
        "Port 3: read the S-CPU's byte, write one back",
    ),
    reg(
        0xF8,
        "AUXIO4",
        ReadWrite,
        "A spare byte, not connected on the SNES",
    ),
    reg(
        0xF9,
        "AUXIO5",
        ReadWrite,
        "A spare byte, not connected on the SNES",
    ),
    reg(
        0xFA,
        "T0TARGET",
        Write,
        "Timer 0's period, in 8 kHz ticks (0 means 256)",
    ),
    reg(
        0xFB,
        "T1TARGET",
        Write,
        "Timer 1's period, in 8 kHz ticks (0 means 256)",
    ),
    reg(
        0xFC,
        "T2TARGET",
        Write,
        "Timer 2's period, in 64 kHz ticks (0 means 256)",
    ),
    reg(
        0xFD,
        "T0OUT",
        Read,
        "Timer 0's count of periods since it was last read",
    ),
    reg(
        0xFE,
        "T1OUT",
        Read,
        "Timer 1's count of periods since it was last read",
    ),
    reg(
        0xFF,
        "T2OUT",
        Read,
        "Timer 2's count of periods since it was last read",
    ),
];

/// The I/O register at an SPC700 address.
pub fn io_register(address: u16) -> Option<&'static IoRegister> {
    (0xF0..=0xFF)
        .contains(&address)
        .then(|| &IO_REGISTERS[(address - 0xF0) as usize])
}

/// The register at a name, for the assembler.
pub fn io_named(name: &str) -> Option<&'static IoRegister> {
    IO_REGISTERS
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
}
