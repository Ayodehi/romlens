//! The SPC700, one bus cycle at a time.
//!
//! Every cycle of an instruction is a call on the [`SpcBus`]: a read, a
//! write, or an internal cycle that touches nothing. The order and the
//! addresses follow the hardware's, dummy reads included (an instruction
//! with no operand still reads the byte after its opcode), so a timer read
//! or a port write lands on the cycle it does on the console. Written from
//! fullsnes's SPC700 chapters and checked against the single-step suite
//! (`tests/spc700_single_step.rs`).

/// What the CPU is connected to. Each call is one cycle.
pub trait SpcBus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);
    /// A cycle that touches no memory.
    fn idle(&mut self);
}

pub const FLAG_N: u8 = 0x80;
pub const FLAG_V: u8 = 0x40;
/// Selects the direct page: `$00xx` or `$01xx`.
pub const FLAG_P: u8 = 0x20;
pub const FLAG_B: u8 = 0x10;
pub const FLAG_H: u8 = 0x08;
pub const FLAG_I: u8 = 0x04;
pub const FLAG_Z: u8 = 0x02;
pub const FLAG_C: u8 = 0x01;

/// The registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spc700 {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub psw: u8,
    pub pc: u16,
    /// `SLEEP` or `STOP` ran: the CPU does nothing until reset.
    pub halted: bool,
}

/// A register an instruction names.
#[derive(Clone, Copy)]
enum Reg {
    A,
    X,
    Y,
}

impl Spc700 {
    /// After reset: the boot ROM's vector is read by [`Spc700::reset`].
    pub fn reset(&mut self, bus: &mut impl SpcBus) {
        *self = Spc700::default();
        let lo = bus.read(0xFFFE);
        let hi = bus.read(0xFFFF);
        self.pc = u16::from_le_bytes([lo, hi]);
    }

    fn flag(&self, f: u8) -> bool {
        self.psw & f != 0
    }

    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.psw |= f;
        } else {
            self.psw &= !f;
        }
    }

    fn nz(&mut self, v: u8) {
        self.set_flag(FLAG_N, v & 0x80 != 0);
        self.set_flag(FLAG_Z, v == 0);
    }

    fn nz16(&mut self, v: u16) {
        self.set_flag(FLAG_N, v & 0x8000 != 0);
        self.set_flag(FLAG_Z, v == 0);
    }

    pub fn ya(&self) -> u16 {
        u16::from_le_bytes([self.a, self.y])
    }

    fn set_ya(&mut self, v: u16) {
        let [a, y] = v.to_le_bytes();
        self.a = a;
        self.y = y;
    }

    /// The direct page's address of `$xx`.
    fn dp(&self, offset: u8) -> u16 {
        (if self.flag(FLAG_P) { 0x100 } else { 0 }) | offset as u16
    }

    fn fetch(&mut self, bus: &mut impl SpcBus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self, bus: &mut impl SpcBus) -> u16 {
        let lo = self.fetch(bus);
        let hi = self.fetch(bus);
        u16::from_le_bytes([lo, hi])
    }

    /// The read an instruction with no operand makes of the next byte.
    fn dummy(&self, bus: &mut impl SpcBus) {
        bus.read(self.pc);
    }

    fn push(&mut self, bus: &mut impl SpcBus, v: u8) {
        bus.write(0x100 | self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self, bus: &mut impl SpcBus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(0x100 | self.sp as u16)
    }

    fn reg(&mut self, r: Reg) -> &mut u8 {
        match r {
            Reg::A => &mut self.a,
            Reg::X => &mut self.x,
            Reg::Y => &mut self.y,
        }
    }

    /// A 16-bit pointer in the direct page, its high byte wrapping in the page.
    fn dp_word(&self, bus: &mut impl SpcBus, offset: u8) -> u16 {
        let lo = bus.read(self.dp(offset));
        let hi = bus.read(self.dp(offset.wrapping_add(1)));
        u16::from_le_bytes([lo, hi])
    }

    // The operand address of each form, with its cycles.

    fn a_dp(&mut self, bus: &mut impl SpcBus) -> u16 {
        let d = self.fetch(bus);
        self.dp(d)
    }

    fn a_dpx(&mut self, bus: &mut impl SpcBus) -> u16 {
        let d = self.fetch(bus);
        bus.idle();
        self.dp(d.wrapping_add(self.x))
    }

    fn a_dpy(&mut self, bus: &mut impl SpcBus) -> u16 {
        let d = self.fetch(bus);
        bus.idle();
        self.dp(d.wrapping_add(self.y))
    }

    fn a_abs(&mut self, bus: &mut impl SpcBus) -> u16 {
        self.fetch16(bus)
    }

    fn a_absx(&mut self, bus: &mut impl SpcBus) -> u16 {
        let a = self.fetch16(bus);
        bus.idle();
        a.wrapping_add(self.x as u16)
    }

    fn a_absy(&mut self, bus: &mut impl SpcBus) -> u16 {
        let a = self.fetch16(bus);
        bus.idle();
        a.wrapping_add(self.y as u16)
    }

    fn a_indx(&mut self, bus: &mut impl SpcBus) -> u16 {
        self.dummy(bus);
        self.dp(self.x)
    }

    /// `[$12+X]`
    fn a_dpxind(&mut self, bus: &mut impl SpcBus) -> u16 {
        let d = self.fetch(bus);
        bus.idle();
        self.dp_word(bus, d.wrapping_add(self.x))
    }

    /// `[$12]+Y`, for reading: the internal cycle before the pointer.
    fn a_dpindy(&mut self, bus: &mut impl SpcBus) -> u16 {
        let d = self.fetch(bus);
        bus.idle();
        self.dp_word(bus, d).wrapping_add(self.y as u16)
    }

    /// The source operand of an ALU instruction, read.
    fn read_arg(&mut self, bus: &mut impl SpcBus, low: u8) -> u8 {
        // The low three bits of an ALU opcode pick the source form.
        match low {
            4 => {
                let a = self.a_dp(bus);
                bus.read(a)
            }
            5 => {
                let a = self.a_abs(bus);
                bus.read(a)
            }
            6 => {
                let a = self.a_indx(bus);
                bus.read(a)
            }
            7 => {
                let a = self.a_dpxind(bus);
                bus.read(a)
            }
            8 => self.fetch(bus),
            0x14 => {
                let a = self.a_dpx(bus);
                bus.read(a)
            }
            0x15 => {
                let a = self.a_absx(bus);
                bus.read(a)
            }
            0x16 => {
                let a = self.a_absy(bus);
                bus.read(a)
            }
            0x17 => {
                let a = self.a_dpindy(bus);
                bus.read(a)
            }
            _ => unreachable!(),
        }
    }

    // The arithmetic.

    fn adc(&mut self, a: u8, b: u8) -> u8 {
        let c = self.flag(FLAG_C) as u16;
        let r = a as u16 + b as u16 + c;
        let r8 = r as u8;
        self.set_flag(FLAG_C, r > 0xFF);
        self.set_flag(FLAG_H, (a ^ b ^ r8) & 0x10 != 0);
        self.set_flag(FLAG_V, !(a ^ b) & (a ^ r8) & 0x80 != 0);
        self.nz(r8);
        r8
    }

    fn sbc(&mut self, a: u8, b: u8) -> u8 {
        self.adc(a, !b)
    }

    fn cmp(&mut self, a: u8, b: u8) {
        let r = a.wrapping_sub(b);
        self.set_flag(FLAG_C, a >= b);
        self.nz(r);
    }

    /// The ALU operation of an opcode's high three bits, `a op b`; `None`
    /// for `CMP`, which keeps `a`.
    fn alu(&mut self, op: u8, a: u8, b: u8) -> Option<u8> {
        let r = match op {
            0 => a | b,
            1 => a & b,
            2 => a ^ b,
            3 => {
                self.cmp(a, b);
                return None;
            }
            4 => return Some(self.adc(a, b)),
            5 => return Some(self.sbc(a, b)),
            _ => unreachable!(),
        };
        self.nz(r);
        Some(r)
    }

    fn shift(&mut self, op: u8, v: u8) -> u8 {
        let c = self.flag(FLAG_C) as u8;
        let (r, out) = match op {
            0 => (v << 1, v & 0x80 != 0),       // ASL
            1 => (v << 1 | c, v & 0x80 != 0),   // ROL
            2 => (v >> 1, v & 1 != 0),          // LSR
            3 => (v >> 1 | c << 7, v & 1 != 0), // ROR
            _ => unreachable!(),
        };
        self.set_flag(FLAG_C, out);
        self.nz(r);
        r
    }

    fn branch(&mut self, bus: &mut impl SpcBus, offset: u8, taken: bool) {
        if taken {
            bus.idle();
            bus.idle();
            self.pc = self.pc.wrapping_add(offset as i8 as u16);
        }
    }

    /// `$0123.5` operands: 13 bits of address, 3 of bit.
    fn mem_bit(&mut self, bus: &mut impl SpcBus) -> (u16, u8) {
        let w = self.fetch16(bus);
        (w & 0x1FFF, (w >> 13) as u8)
    }

    fn call(&mut self, bus: &mut impl SpcBus, to: u16) {
        let [lo, hi] = self.pc.to_le_bytes();
        self.push(bus, hi);
        self.push(bus, lo);
        self.pc = to;
    }

    /// Run one instruction. A halted CPU spends two cycles going nowhere.
    pub fn step(&mut self, bus: &mut impl SpcBus) {
        if self.halted {
            // The clock stops with the CPU still reading its next byte.
            self.dummy(bus);
            bus.idle();
            return;
        }
        let op = self.fetch(bus);
        let hi = op >> 5;
        match op {
            // OR, AND, EOR, CMP, ADC, SBC in their eight A forms.
            0x04..=0x08
            | 0x14..=0x17
            | 0x24..=0x28
            | 0x34..=0x37
            | 0x44..=0x48
            | 0x54..=0x57
            | 0x64..=0x68
            | 0x74..=0x77
            | 0x84..=0x88
            | 0x94..=0x97
            | 0xA4..=0xA8
            | 0xB4..=0xB7 => {
                let b = self.read_arg(bus, op & 0x1F);
                if let Some(r) = self.alu(hi, self.a, b) {
                    self.a = r;
                }
            }
            // op dp,dp
            0x09 | 0x29 | 0x49 | 0x69 | 0x89 | 0xA9 => {
                let s = self.fetch(bus);
                let b = bus.read(self.dp(s));
                let d = self.a_dp(bus);
                let a = bus.read(d);
                match self.alu(hi, a, b) {
                    Some(r) => bus.write(d, r),
                    None => bus.idle(),
                }
            }
            // op (X),(Y)
            0x19 | 0x39 | 0x59 | 0x79 | 0x99 | 0xB9 => {
                self.dummy(bus);
                let b = bus.read(self.dp(self.y));
                let d = self.dp(self.x);
                let a = bus.read(d);
                match self.alu(hi, a, b) {
                    Some(r) => bus.write(d, r),
                    None => bus.idle(),
                }
            }
            // op dp,#imm
            0x18 | 0x38 | 0x58 | 0x78 | 0x98 | 0xB8 => {
                let b = self.fetch(bus);
                let d = self.a_dp(bus);
                let a = bus.read(d);
                match self.alu(hi, a, b) {
                    Some(r) => bus.write(d, r),
                    None => bus.idle(),
                }
            }
            // CMP X/Y and MOV X/Y loads by form.
            0xC8 => {
                let b = self.fetch(bus);
                self.cmp(self.x, b);
            }
            0x3E => {
                let a = self.a_dp(bus);
                let b = bus.read(a);
                self.cmp(self.x, b);
            }
            0x1E => {
                let a = self.a_abs(bus);
                let b = bus.read(a);
                self.cmp(self.x, b);
            }
            0xAD => {
                let b = self.fetch(bus);
                self.cmp(self.y, b);
            }
            0x7E => {
                let a = self.a_dp(bus);
                let b = bus.read(a);
                self.cmp(self.y, b);
            }
            0x5E => {
                let a = self.a_abs(bus);
                let b = bus.read(a);
                self.cmp(self.y, b);
            }

            // Shifts and rotates: ASL, ROL, LSR, ROR.
            0x0B | 0x2B | 0x4B | 0x6B => {
                let a = self.a_dp(bus);
                let v = bus.read(a);
                let r = self.shift(hi, v);
                bus.write(a, r);
            }
            0x1B | 0x3B | 0x5B | 0x7B => {
                let a = self.a_dpx(bus);
                let v = bus.read(a);
                let r = self.shift(hi, v);
                bus.write(a, r);
            }
            0x0C | 0x2C | 0x4C | 0x6C => {
                let a = self.a_abs(bus);
                let v = bus.read(a);
                let r = self.shift(hi, v);
                bus.write(a, r);
            }
            0x1C | 0x3C | 0x5C | 0x7C => {
                self.dummy(bus);
                self.a = self.shift(hi, self.a);
            }

            // INC and DEC.
            0x8B | 0xAB => {
                let a = self.a_dp(bus);
                let v = bus.read(a);
                let r = if op == 0xAB {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.nz(r);
                bus.write(a, r);
            }
            0x9B | 0xBB => {
                let a = self.a_dpx(bus);
                let v = bus.read(a);
                let r = if op == 0xBB {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.nz(r);
                bus.write(a, r);
            }
            0x8C | 0xAC => {
                let a = self.a_abs(bus);
                let v = bus.read(a);
                let r = if op == 0xAC {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                self.nz(r);
                bus.write(a, r);
            }
            0xBC | 0x9C | 0x3D | 0x1D | 0xFC | 0xDC => {
                self.dummy(bus);
                let r = match op {
                    0xBC | 0x9C => Reg::A,
                    0x3D | 0x1D => Reg::X,
                    _ => Reg::Y,
                };
                let v = *self.reg(r);
                let v = if matches!(op, 0xBC | 0x3D | 0xFC) {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                *self.reg(r) = v;
                self.nz(v);
            }

            // 16-bit.
            0x1A | 0x3A => {
                // DECW, INCW
                let d = self.fetch(bus);
                let lo = bus.read(self.dp(d));
                let w0 = if op == 0x3A {
                    lo.wrapping_add(1)
                } else {
                    lo.wrapping_sub(1)
                };
                bus.write(self.dp(d), w0);
                let hi_ = bus.read(self.dp(d.wrapping_add(1)));
                let v = u16::from_le_bytes([lo, hi_]);
                let r = if op == 0x3A {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                bus.write(self.dp(d.wrapping_add(1)), (r >> 8) as u8);
                self.nz16(r);
            }
            0x5A => {
                // CMPW YA,dp
                let d = self.fetch(bus);
                let w = self.dp_word(bus, d);
                let ya = self.ya();
                self.set_flag(FLAG_C, ya >= w);
                self.nz16(ya.wrapping_sub(w));
            }
            0x7A | 0x9A => {
                // ADDW, SUBW
                let d = self.fetch(bus);
                let lo = bus.read(self.dp(d));
                bus.idle();
                let hi_ = bus.read(self.dp(d.wrapping_add(1)));
                let w = u16::from_le_bytes([lo, hi_]);
                let ya = self.ya();
                let b = if op == 0x7A { w } else { !w };
                let r = ya as u32 + b as u32 + (op == 0x9A) as u32;
                let r16 = r as u16;
                self.set_flag(FLAG_C, r > 0xFFFF);
                self.set_flag(FLAG_H, (ya ^ b ^ r16) & 0x1000 != 0);
                self.set_flag(FLAG_V, !(ya ^ b) & (ya ^ r16) & 0x8000 != 0);
                self.nz16(r16);
                self.set_ya(r16);
            }
            0xBA => {
                // MOVW YA,dp
                let d = self.fetch(bus);
                let lo = bus.read(self.dp(d));
                bus.idle();
                let hi_ = bus.read(self.dp(d.wrapping_add(1)));
                self.set_ya(u16::from_le_bytes([lo, hi_]));
                self.nz16(self.ya());
            }
            0xDA => {
                // MOVW dp,YA
                let d = self.fetch(bus);
                bus.read(self.dp(d));
                bus.write(self.dp(d), self.a);
                bus.write(self.dp(d.wrapping_add(1)), self.y);
            }
            0xCF => {
                // MUL YA
                self.dummy(bus);
                for _ in 0..7 {
                    bus.idle();
                }
                let r = self.y as u16 * self.a as u16;
                self.set_ya(r);
                self.nz(self.y);
            }
            0x9E => {
                // DIV YA,X
                self.dummy(bus);
                for _ in 0..10 {
                    bus.idle();
                }
                self.div();
            }
            0xDF | 0xBE => {
                // DAA, DAS
                self.dummy(bus);
                bus.idle();
                if op == 0xDF {
                    if self.flag(FLAG_C) || self.a > 0x99 {
                        self.a = self.a.wrapping_add(0x60);
                        self.set_flag(FLAG_C, true);
                    }
                    if self.flag(FLAG_H) || self.a & 0xF > 9 {
                        self.a = self.a.wrapping_add(6);
                    }
                } else {
                    if !self.flag(FLAG_C) || self.a > 0x99 {
                        self.a = self.a.wrapping_sub(0x60);
                        self.set_flag(FLAG_C, false);
                    }
                    if !self.flag(FLAG_H) || self.a & 0xF > 9 {
                        self.a = self.a.wrapping_sub(6);
                    }
                }
                self.nz(self.a);
            }
            0x9F => {
                // XCN A
                self.dummy(bus);
                for _ in 0..3 {
                    bus.idle();
                }
                self.a = self.a.rotate_left(4);
                self.nz(self.a);
            }

            // MOV loads into A, X, Y.
            0xE4..=0xE8 | 0xF4..=0xF7 => {
                self.a = self.read_arg(bus, op & 0x1F);
                self.nz(self.a);
            }
            0xBF => {
                // MOV A,(X)+
                self.dummy(bus);
                self.a = bus.read(self.dp(self.x));
                bus.idle();
                self.x = self.x.wrapping_add(1);
                self.nz(self.a);
            }
            0xCD | 0xF8 | 0xF9 | 0xE9 => {
                self.x = match op {
                    0xCD => self.fetch(bus),
                    0xF8 => {
                        let a = self.a_dp(bus);
                        bus.read(a)
                    }
                    0xF9 => {
                        let a = self.a_dpy(bus);
                        bus.read(a)
                    }
                    _ => {
                        let a = self.a_abs(bus);
                        bus.read(a)
                    }
                };
                self.nz(self.x);
            }
            0x8D | 0xEB | 0xFB | 0xEC => {
                self.y = match op {
                    0x8D => self.fetch(bus),
                    0xEB => {
                        let a = self.a_dp(bus);
                        bus.read(a)
                    }
                    0xFB => {
                        let a = self.a_dpx(bus);
                        bus.read(a)
                    }
                    _ => {
                        let a = self.a_abs(bus);
                        bus.read(a)
                    }
                };
                self.nz(self.y);
            }

            // MOV stores: a read of the target first, then the write.
            0xC4 | 0xD4 | 0xC5 | 0xD5 | 0xD6 | 0xC6 | 0xC7 | 0xD7 | 0xD8 | 0xD9 | 0xC9 | 0xCB
            | 0xDB | 0xCC => {
                let a = match op {
                    0xC4 | 0xD8 | 0xCB => self.a_dp(bus),
                    0xD4 | 0xDB => self.a_dpx(bus),
                    0xD9 => self.a_dpy(bus),
                    0xC5 | 0xC9 | 0xCC => self.a_abs(bus),
                    0xD5 => self.a_absx(bus),
                    0xD6 => self.a_absy(bus),
                    0xC6 => self.a_indx(bus),
                    0xC7 => self.a_dpxind(bus),
                    _ => {
                        // [dp]+Y: the internal cycle comes after the pointer.
                        let d = self.fetch(bus);
                        let p = self.dp_word(bus, d);
                        bus.idle();
                        p.wrapping_add(self.y as u16)
                    }
                };
                bus.read(a);
                let v = match op {
                    0xD8 | 0xD9 | 0xC9 => self.x,
                    0xCB | 0xDB | 0xCC => self.y,
                    _ => self.a,
                };
                bus.write(a, v);
            }
            0xAF => {
                // MOV (X)+,A
                self.dummy(bus);
                bus.idle();
                bus.write(self.dp(self.x), self.a);
                self.x = self.x.wrapping_add(1);
            }
            0x8F => {
                // MOV dp,#imm
                let v = self.fetch(bus);
                let a = self.a_dp(bus);
                bus.read(a);
                bus.write(a, v);
            }
            0xFA => {
                // MOV dp,dp
                let s = self.fetch(bus);
                let v = bus.read(self.dp(s));
                let a = self.a_dp(bus);
                bus.write(a, v);
            }
            // Register to register.
            0x7D | 0xDD | 0x5D | 0xFD | 0x9D | 0xBD => {
                self.dummy(bus);
                match op {
                    0x7D => {
                        self.a = self.x;
                        self.nz(self.a);
                    }
                    0xDD => {
                        self.a = self.y;
                        self.nz(self.a);
                    }
                    0x5D => {
                        self.x = self.a;
                        self.nz(self.x);
                    }
                    0xFD => {
                        self.y = self.a;
                        self.nz(self.y);
                    }
                    0x9D => {
                        self.x = self.sp;
                        self.nz(self.x);
                    }
                    _ => self.sp = self.x,
                }
            }

            // Bits.
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x82 | 0x92 | 0xA2 | 0xB2
            | 0xC2 | 0xD2 | 0xE2 | 0xF2 => {
                // SET1 / CLR1 dp.bit
                let a = self.a_dp(bus);
                let v = bus.read(a);
                let bit = 1 << (op >> 5);
                let r = if op & 0x10 == 0 { v | bit } else { v & !bit };
                bus.write(a, r);
            }
            0x03 | 0x13 | 0x23 | 0x33 | 0x43 | 0x53 | 0x63 | 0x73 | 0x83 | 0x93 | 0xA3 | 0xB3
            | 0xC3 | 0xD3 | 0xE3 | 0xF3 => {
                // BBS / BBC dp.bit,rel
                let a = self.a_dp(bus);
                let v = bus.read(a);
                bus.idle();
                let rel = self.fetch(bus);
                let set = v & (1 << (op >> 5)) != 0;
                self.branch(bus, rel, set == (op & 0x10 == 0));
            }
            0x0E | 0x4E => {
                // TSET1 / TCLR1 !abs
                let a = self.a_abs(bus);
                let v = bus.read(a);
                bus.read(a);
                self.nz(self.a.wrapping_sub(v));
                let r = if op == 0x0E { v | self.a } else { v & !self.a };
                bus.write(a, r);
            }
            0x0A | 0x2A | 0x4A | 0x6A | 0x8A | 0xAA => {
                // OR1, AND1, EOR1, MOV1 into C
                let (a, bit) = self.mem_bit(bus);
                let b = bus.read(a) & (1 << bit) != 0;
                let c = self.flag(FLAG_C);
                let r = match op {
                    0x0A => c | b,
                    0x2A => c | !b,
                    0x4A => c & b,
                    0x6A => c & !b,
                    0x8A => c ^ b,
                    _ => b,
                };
                if matches!(op, 0x0A | 0x2A | 0x8A) {
                    bus.idle();
                }
                self.set_flag(FLAG_C, r);
            }
            0xCA => {
                // MOV1 mem.bit,C
                let (a, bit) = self.mem_bit(bus);
                let v = bus.read(a);
                bus.idle();
                let r = if self.flag(FLAG_C) {
                    v | 1 << bit
                } else {
                    v & !(1 << bit)
                };
                bus.write(a, r);
            }
            0xEA => {
                // NOT1 mem.bit
                let (a, bit) = self.mem_bit(bus);
                let v = bus.read(a);
                bus.write(a, v ^ (1 << bit));
            }

            // Flags.
            0x60 | 0x80 | 0x20 | 0x40 | 0xE0 => {
                self.dummy(bus);
                match op {
                    0x60 => self.set_flag(FLAG_C, false),
                    0x80 => self.set_flag(FLAG_C, true),
                    0x20 => self.set_flag(FLAG_P, false),
                    0x40 => self.set_flag(FLAG_P, true),
                    _ => self.psw &= !(FLAG_V | FLAG_H),
                }
            }
            0xED | 0xA0 | 0xC0 => {
                self.dummy(bus);
                bus.idle();
                match op {
                    0xED => self.psw ^= FLAG_C,
                    0xA0 => self.set_flag(FLAG_I, true),
                    _ => self.set_flag(FLAG_I, false),
                }
            }

            // Branches.
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                let rel = self.fetch(bus);
                let (f, set) = match op {
                    0x10 => (FLAG_N, false),
                    0x30 => (FLAG_N, true),
                    0x50 => (FLAG_V, false),
                    0x70 => (FLAG_V, true),
                    0x90 => (FLAG_C, false),
                    0xB0 => (FLAG_C, true),
                    0xD0 => (FLAG_Z, false),
                    _ => (FLAG_Z, true),
                };
                let taken = self.flag(f) == set;
                self.branch(bus, rel, taken);
            }
            0x2F => {
                let rel = self.fetch(bus);
                self.branch(bus, rel, true);
            }
            0x2E | 0xDE => {
                // CBNE dp,rel / CBNE dp+X,rel
                let a = if op == 0x2E {
                    self.a_dp(bus)
                } else {
                    self.a_dpx(bus)
                };
                let v = bus.read(a);
                bus.idle();
                let rel = self.fetch(bus);
                self.branch(bus, rel, self.a != v);
            }
            0x6E => {
                // DBNZ dp,rel
                let a = self.a_dp(bus);
                let v = bus.read(a).wrapping_sub(1);
                bus.write(a, v);
                let rel = self.fetch(bus);
                self.branch(bus, rel, v != 0);
            }
            0xFE => {
                // DBNZ Y,rel
                self.dummy(bus);
                bus.idle();
                let rel = self.fetch(bus);
                self.y = self.y.wrapping_sub(1);
                self.branch(bus, rel, self.y != 0);
            }

            // Jumps and calls.
            0x5F => self.pc = self.fetch16(bus),
            0x1F => {
                let a = self.a_absx(bus);
                let lo = bus.read(a);
                let hi_ = bus.read(a.wrapping_add(1));
                self.pc = u16::from_le_bytes([lo, hi_]);
            }
            0x3F => {
                let to = self.fetch16(bus);
                bus.idle();
                self.call(bus, to);
                bus.idle();
                bus.idle();
            }
            0x4F => {
                let u = self.fetch(bus);
                bus.idle();
                self.call(bus, 0xFF00 | u as u16);
                bus.idle();
            }
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1 | 0xB1
            | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                // TCALL n
                self.dummy(bus);
                bus.idle();
                let [lo, hi_] = self.pc.to_le_bytes();
                self.push(bus, hi_);
                self.push(bus, lo);
                bus.idle();
                let v = 0xFFDE - 2 * (op >> 4) as u16;
                let lo = bus.read(v);
                let hi_ = bus.read(v + 1);
                self.pc = u16::from_le_bytes([lo, hi_]);
            }
            0x0F => {
                // BRK
                self.dummy(bus);
                let [lo, hi_] = self.pc.to_le_bytes();
                self.push(bus, hi_);
                self.push(bus, lo);
                self.push(bus, self.psw);
                bus.idle();
                let lo = bus.read(0xFFDE);
                let hi_ = bus.read(0xFFDF);
                self.pc = u16::from_le_bytes([lo, hi_]);
                self.set_flag(FLAG_B, true);
                self.set_flag(FLAG_I, false);
            }
            0x6F => {
                // RET
                self.dummy(bus);
                bus.idle();
                let lo = self.pop(bus);
                let hi_ = self.pop(bus);
                self.pc = u16::from_le_bytes([lo, hi_]);
            }
            0x7F => {
                // RETI
                self.dummy(bus);
                bus.idle();
                self.psw = self.pop(bus);
                let lo = self.pop(bus);
                let hi_ = self.pop(bus);
                self.pc = u16::from_le_bytes([lo, hi_]);
            }

            // The stack.
            0x2D | 0x4D | 0x6D | 0x0D => {
                self.dummy(bus);
                let v = match op {
                    0x2D => self.a,
                    0x4D => self.x,
                    0x6D => self.y,
                    _ => self.psw,
                };
                self.push(bus, v);
                bus.idle();
            }
            0xAE | 0xCE | 0xEE | 0x8E => {
                self.dummy(bus);
                bus.idle();
                let v = self.pop(bus);
                match op {
                    0xAE => self.a = v,
                    0xCE => self.x = v,
                    0xEE => self.y = v,
                    _ => self.psw = v,
                }
            }

            0x00 => self.dummy(bus),
            0xEF | 0xFF => {
                // SLEEP, STOP
                self.dummy(bus);
                bus.idle();
                self.halted = true;
            }
        }
    }

    /// `DIV YA,X`: the hardware's result, quotient and remainder, also when
    /// the quotient does not fit in 8 bits (anomie's SPC700 notes).
    fn div(&mut self) {
        let ya = self.ya() as u32;
        let x = self.x as u32;
        self.set_flag(FLAG_H, (self.x & 0xF) <= (self.y & 0xF));
        self.set_flag(FLAG_V, self.y as u32 >= x);
        if (self.y as u32) < x << 1 {
            self.a = (ya / x) as u8;
            self.y = (ya % x) as u8;
        } else {
            let rest = ya - (x << 9);
            self.a = (255 - rest / (256 - x)) as u8;
            self.y = (x + rest % (256 - x)) as u8;
        }
        self.nz(self.a);
    }
}
