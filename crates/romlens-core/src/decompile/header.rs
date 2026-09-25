//! `snes.h`: what every decompiled routine includes, so the output is a
//! translation unit a C compiler accepts.
//!
//! Memory goes through `MEM8` and `MEM16`. They are guarded, so a test (or a
//! reader who wants to run the code) can define them over an array before
//! including this; every hardware register is defined through them too.

use crate::model::hardware::all_hardware_registers;

/// The helpers `snes.h` declares, which a label must not shadow.
pub const HELPERS: [&str; 28] = [
    "SET24",
    "SEI",
    "CLI",
    "SED",
    "CLD",
    "native_mode",
    "emulation_mode",
    "push8",
    "push16",
    "pull8",
    "pull16",
    "PHP",
    "PLP",
    "mvn",
    "mvp",
    "mvn8",
    "mvp8",
    "bcd_add",
    "bcd_sub",
    "WAI",
    "STP",
    "BRK",
    "COP",
    "table_overrun",
    "MEM8",
    "MEM16",
    "MEM24",
    "LONG",
];

/// Every other name `snes.h` defines.
pub const GLOBALS: [&str; 20] = [
    "A",
    "X",
    "Y",
    "S",
    "D",
    "DBR",
    "N",
    "V",
    "Z",
    "C",
    "u8",
    "u16",
    "u32",
    "s8",
    "s16",
    "s32",
    "ROMLENS_SNES_H",
    "bool",
    "true",
    "false",
];

pub fn snes_h() -> String {
    let mut s = String::from(
        "/* snes.h: declarations for the pseudo-C Romlens writes.\n\
         *\n\
         * The code is for reading. It is valid C, but it is not the game's\n\
         * source, and it does not rebuild the ROM.\n\
         */\n\
         #ifndef ROMLENS_SNES_H\n\
         #define ROMLENS_SNES_H\n\
         \n\
         #include <stdbool.h>\n\
         #include <stdint.h>\n\
         \n\
         typedef uint8_t u8;\n\
         typedef uint16_t u16;\n\
         typedef uint32_t u32;\n\
         typedef int8_t s8;\n\
         typedef int16_t s16;\n\
         typedef int32_t s32;\n\
         \n\
         /* The CPU. A holds all 16 bits of the accumulator; with 8-bit A only\n\
         * its low byte changes. Flags are 0 or 1. At the full level each\n\
         * routine has these as its own variables (a, x, c, ...), and passes\n\
         * and returns them; the globals carry them only to code that reads\n\
         * the registers themselves. */\n\
         extern u16 A, X, Y, S, D;\n\
         extern u8 DBR;\n\
         extern u8 N, V, Z, C;\n\
         \n\
         /* Memory by 24-bit address. Define MEM8 and MEM16 before including\n\
         * this to run the code over an array. */\n\
         #ifndef MEM8\n\
         #define MEM8(a) (*(volatile u8 *)(uintptr_t)(a))\n\
         #endif\n\
         #ifndef MEM16\n\
         #define MEM16(a) (*(volatile u16 *)(uintptr_t)(a))\n\
         #endif\n\
         #define MEM24(a) ((u32)MEM16(a) | (u32)MEM8((a) + 2) << 16)\n\
         /* Store a 24-bit value: C has no 24-bit type. */\n\
         #define SET24(a, v) (MEM16((uintptr_t)(a)) = (u16)(v), MEM8((uintptr_t)(a) + 2) = (u8)((u32)(v) >> 16))\n\
         /* A 24-bit value held in a three-byte array. */\n\
         #define LONG(p) ((u32)(p)[0] | (u32)(p)[1] << 8 | (u32)(p)[2] << 16)\n\
         \n\
         /* Instructions with no C equivalent. */\n\
         void SEI(void);\n\
         void CLI(void);\n\
         void SED(void);\n\
         void CLD(void);\n\
         void native_mode(void);    /* CLC; XCE */\n\
         void emulation_mode(void); /* SEC; XCE */\n\
         void push8(u8 v);\n\
         void push16(u16 v);\n\
         u8 pull8(void);\n\
         u16 pull16(void);\n\
         void PHP(void);\n\
         void PLP(void);\n\
         void mvn(u8 dst_bank, u8 src_bank); /* copies A + 1 bytes from X to Y */\n\
         void mvp(u8 dst_bank, u8 src_bank);\n\
         void mvn8(u8 dst_bank, u8 src_bank); /* the same with 8-bit X and Y */\n\
         void mvp8(u8 dst_bank, u8 src_bank);\n\
         /* Decimal-mode ADC and SBC of `bits` bits; they set C and V. */\n\
         u16 bcd_add(u16 a, u16 b, int bits);\n\
         u16 bcd_sub(u16 a, u16 b, int bits);\n\
         void WAI(void);\n\
         void STP(void);\n\
         void BRK(u8 n);\n\
         void COP(u8 n);\n\
         /* A jump table's index past the entries Romlens found: */\n\
         /* the game jumps wherever the bytes after the table point. */\n\
         _Noreturn void table_overrun(void);\n\
         \n\
         /* Hardware registers. */\n",
    );
    for r in all_hardware_registers() {
        s.push_str(&format!(
            "#define {} MEM8(0x{:04X}) /* {} */\n",
            r.name, r.address, r.description
        ));
    }
    s.push_str("\n#endif\n");
    s
}
