//! The boot program at `$FFC0`: our own, not Nintendo's.
//!
//! The console's 64-byte boot ROM is Nintendo's code and Romlens never
//! ships it (docs/23). This one is written from the upload protocol
//! fullsnes describes ("SNES APU Main CPU Communication Port"), which is all
//! a game relies on:
//!
//! 1. The SPC700 puts `$AA` on port 0 and `$BB` on port 1: ready.
//! 2. The S-CPU puts an address on ports 2 and 3, a non-zero byte on port 1
//!    and `$CC` on port 0. The SPC700 echoes port 0.
//! 3. For each byte: the S-CPU puts the byte on port 1 and its index (from
//!    0, wrapping) on port 0, then waits for the echo of the index.
//! 4. A new block: a new address on ports 2 and 3, port 1 non-zero, and on
//!    port 0 a value past the last index (usually the index plus 2). Port 1
//!    zero instead means jump to the address.
//!
//! While it waits for byte `Y`, port 0 holds the last value echoed (the
//! command's, then `Y - 1`), which X keeps; anything else that is not `Y`
//! is a new command. Port 0 is checked against X first: once it leaves the
//! last echo it stays put until answered, so the second read sees the same
//! value and a byte cannot be taken for a command.
//!
//! [`boot_upload`] does the same work without running it, for booting a
//! driver from a known block list.

use std::sync::OnceLock;

use super::Apu;

/// Our boot program, assembled with [`crate::spc700::assemble`].
pub const IPL_SOURCE: &str = "
    .org $FFC0
    reset:  MOV X,#$EF          ; the stack at $01EF
            MOV SP,X
            MOV A,#$00
    clear:  MOV (X),A           ; zero the direct page, $01-$EF
            DEC X
            BNE clear
            MOV CPUIO0,#$AA     ; ready
            MOV CPUIO1,#$BB
    kick:   CMP CPUIO0,#$CC     ; wait for the first command
            BNE kick
    command:
            MOVW YA,CPUIO2      ; the address, from ports 2 and 3
            MOVW $00,YA
            MOVW YA,CPUIO0      ; A: the command's port 0, Y: port 1
            MOV CPUIO0,A        ; echo it
            MOV X,A             ; X: the last echo
            MOV A,Y
            BEQ jump            ; port 1 zero: run what was sent
            MOV Y,#$00
    wait:   CMP X,CPUIO0        ; still the last echo: not yet
            BEQ wait
            CMP Y,CPUIO0        ; anything but byte Y: a new command
            BNE command
            MOV A,CPUIO1
            MOV CPUIO0,Y        ; echo the index
            MOV [$00]+Y,A
            MOV A,Y
            MOV X,A
            INC Y
            BNE wait
            INC $01             ; 256 bytes on: the next page
            BRA wait
    jump:   MOV X,A             ; A is 0
            JMP [!$0000+X]
    .org $FFFE
            .dw reset
";

/// The program's 64 bytes, `$FFC0–$FFFF`.
pub fn ipl() -> &'static [u8; 64] {
    static IPL: OnceLock<[u8; 64]> = OnceLock::new();
    IPL.get_or_init(|| {
        let image = crate::spc700::assemble(IPL_SOURCE)
            .expect("the boot program assembles")
            .image();
        image[0xFFC0..].try_into().unwrap()
    })
}

/// A block the boot program would receive: bytes for audio RAM at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadBlock {
    pub at: u16,
    pub bytes: Vec<u8>,
}

/// Boot straight into a driver: the blocks laid into audio RAM and the
/// SPC700 at `entry`, as the boot program leaves it once the S-CPU says
/// jump. The registers are those the program leaves: the stack at `$EF`,
/// X zero, the boot ROM still mapped.
pub fn boot_upload(apu: &mut Apu, blocks: &[UploadBlock], entry: u16) {
    for b in blocks {
        for (i, v) in b.bytes.iter().enumerate() {
            apu.bus.aram[(b.at as usize + i) & 0xFFFF] = *v;
        }
    }
    apu.cpu.sp = 0xEF;
    apu.cpu.x = 0;
    apu.cpu.pc = entry;
    apu.cpu.halted = false;
    apu.bus.io.rom_enabled = true;
}
