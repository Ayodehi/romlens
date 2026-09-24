//! Each instruction as IR statements that say everything it does.
//!
//! One match over every mnemonic: the compiler checks it is exhaustive, and
//! `every_opcode_lifts` checks all 256 opcodes in both width states. Flags
//! are set in full here; the data-flow stage drops the ones nothing reads.

use crate::cpu65816::{AddressingMode, Instruction, Mnemonic};
use crate::decompile::cfg::{Cfg, Term};
use crate::decompile::function::{Callee, Function, Transfer};
use crate::decompile::ir::{
    BinOp, CallTarget, Expr, Flag, LiftedBlock, Line, Place, Reg, Stmt, UnOp, Width,
};

/// What the lifter was told to assume.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiftOptions {
    /// The direct page to use where the analysis does not know it.
    pub assume_dp: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Lifted {
    /// Parallel to `Cfg::blocks`; a stub's is empty.
    pub blocks: Vec<LiftedBlock>,
    /// Temporaries `t1..=temps`.
    pub temps: u32,
    pub warnings: Vec<String>,
}

/// The decimal flag, followed through the graph: `ADC` and `SBC` mean
/// different things with it set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decimal {
    Clear,
    Set,
    Unknown,
}

impl Decimal {
    fn join(self, other: Decimal) -> Decimal {
        if self == other {
            self
        } else {
            Decimal::Unknown
        }
    }

    fn after(self, insn: &Instruction) -> Decimal {
        let imm = insn.operand.value() as u8;
        match insn.mnemonic {
            Mnemonic::SED => Decimal::Set,
            Mnemonic::CLD => Decimal::Clear,
            Mnemonic::SEP if imm & 0x08 != 0 => Decimal::Set,
            Mnemonic::REP if imm & 0x08 != 0 => Decimal::Clear,
            Mnemonic::PLP | Mnemonic::RTI => Decimal::Unknown,
            _ => self,
        }
    }
}

pub fn lift(f: &Function, cfg: &Cfg, opts: LiftOptions) -> Lifted {
    let n = cfg.blocks.len();
    // The decimal flag at each block's entry. A routine is assumed entered
    // with it clear, which is how nearly all SNES code runs.
    let mut dec_in: Vec<Option<Decimal>> = vec![None; n];
    dec_in[cfg.entry] = Some(Decimal::Clear);
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &cfg.rpo {
            let Some(mut d) = dec_in[b] else { continue };
            for i in cfg.blocks[b].steps.clone() {
                d = d.after(&f.steps[i].insn);
            }
            for s in cfg.succs(b) {
                let new = match dec_in[s] {
                    None => d,
                    Some(old) => old.join(d),
                };
                if dec_in[s] != Some(new) {
                    dec_in[s] = Some(new);
                    changed = true;
                }
            }
        }
    }

    let mut l = Lifter {
        opts,
        temps: 0,
        warnings: Vec::new(),
        warned_decimal: false,
    };
    let mut blocks = Vec::with_capacity(n);
    for (b, block) in cfg.blocks.iter().enumerate() {
        let mut out = LiftedBlock::default();
        let mut d = dec_in[b].unwrap_or(Decimal::Unknown);
        for i in block.steps.clone() {
            let step = &f.steps[i];
            for stmt in l.insn(&step.insn, &step.transfer, d) {
                out.lines.push(Line { stmt, step: i });
            }
            d = d.after(&step.insn);
        }
        if !block.is_stub() {
            let last = block.steps.end - 1;
            let insn = &f.steps[last].insn;
            out.term_step = Some(last);
            match &block.term {
                Term::Branch { .. } => out.cond = Some(condition(insn.mnemonic)),
                Term::Switch { cases, .. } => {
                    out.switch = Some((
                        Expr::Reg(Reg::X, Width::W16),
                        (0..cases.len() as u32).map(|i| i * 2).collect(),
                    ))
                }
                _ => {}
            }
        }
        blocks.push(out);
    }
    Lifted {
        blocks,
        temps: l.temps,
        warnings: l.warnings,
    }
}

/// When a conditional branch is taken.
pub fn condition(m: Mnemonic) -> Expr {
    use Mnemonic::*;
    let flag = |f| Expr::Flag(f);
    let not = |f| Expr::un(UnOp::LNot, Expr::Flag(f));
    match m {
        BCC => not(Flag::C),
        BCS => flag(Flag::C),
        BEQ => flag(Flag::Z),
        BNE => not(Flag::Z),
        BMI => flag(Flag::N),
        BPL => not(Flag::N),
        BVC => not(Flag::V),
        BVS => flag(Flag::V),
        _ => Expr::Const(1),
    }
}

struct Lifter {
    opts: LiftOptions,
    temps: u32,
    warnings: Vec<String>,
    warned_decimal: bool,
}

fn set(dst: Place, value: Expr) -> Stmt {
    Stmt::Assign { dst, value }
}

fn flag(f: Flag, value: Expr) -> Stmt {
    set(Place::Flag(f), value)
}

fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    Expr::bin(op, a, b)
}

fn c(v: u32) -> Expr {
    Expr::Const(v)
}

/// `N` and `Z` from a result of width `w`.
fn nz(out: &mut Vec<Stmt>, v: Expr, w: Width) {
    out.push(flag(
        Flag::N,
        bin(BinOp::Ne, bin(BinOp::And, v.clone(), c(w.sign())), c(0)),
    ));
    out.push(flag(Flag::Z, bin(BinOp::Eq, Expr::cast(w, v), c(0))));
}

impl Lifter {
    fn temp(&mut self) -> u32 {
        self.temps += 1;
        self.temps
    }

    /// `e` if it can be read twice, else a temporary holding it.
    fn once(&mut self, out: &mut Vec<Stmt>, e: Expr) -> Expr {
        if e.has_effects() {
            let t = self.temp();
            out.push(set(Place::Temp(t), e));
            Expr::Temp(t)
        } else {
            e
        }
    }

    fn dp_base(&self, insn: &Instruction) -> Expr {
        match insn.flags_before.dp.or(self.opts.assume_dp) {
            Some(d) => c(d as u32),
            None => Expr::Reg(Reg::D, Width::W16),
        }
    }

    /// The data bank, shifted into place, or'd onto a 16-bit address.
    fn in_data_bank(&self, insn: &Instruction, e: Expr) -> Expr {
        match insn.flags_before.dbr {
            Some(b) => bin(BinOp::Or, e, c((b as u32) << 16)),
            None => bin(
                BinOp::Or,
                bin(BinOp::Shl, Expr::Reg(Reg::Dbr, Width::W8), c(16)),
                e,
            ),
        }
    }

    /// The 24-bit address a data instruction reads or writes.
    fn address(&self, insn: &Instruction) -> Option<Expr> {
        use AddressingMode::*;
        let op = insn.operand.value();
        let dp = |off: u32| Expr::sum(self.dp_base(insn), c(off));
        let x = Expr::Reg(Reg::X, Width::W16);
        let y = Expr::Reg(Reg::Y, Width::W16);
        let s = Expr::Reg(Reg::S, Width::W16);
        let ptr16 = |at: Expr| self.in_data_bank(insn, Expr::mem(at, Width::W16));
        Some(match insn.mode {
            Direct => dp(op),
            DirectX => Expr::sum(dp(op), x),
            DirectY => Expr::sum(dp(op), y),
            DirectIndirect => ptr16(dp(op)),
            DirectIndexedIndirect => ptr16(Expr::sum(dp(op), x)),
            DirectIndirectIndexed => Expr::sum(ptr16(dp(op)), y),
            DirectIndirectLong => Expr::mem(dp(op), Width::W24),
            DirectIndirectLongIndexed => Expr::sum(Expr::mem(dp(op), Width::W24), y),
            Absolute => self.in_data_bank(insn, c(op)),
            AbsoluteX => Expr::sum(self.in_data_bank(insn, c(op)), x),
            AbsoluteY => Expr::sum(self.in_data_bank(insn, c(op)), y),
            AbsoluteLong => c(op),
            AbsoluteLongX => Expr::sum(c(op), x),
            StackRelative => Expr::sum(s, c(op)),
            StackRelativeIndirectIndexed => Expr::sum(ptr16(Expr::sum(s, c(op))), y),
            _ => return None,
        })
    }

    /// The value an instruction reads: its immediate, the accumulator, or
    /// memory.
    fn source(&self, insn: &Instruction, w: Width) -> Expr {
        match insn.mode {
            AddressingMode::ImmediateM
            | AddressingMode::ImmediateX
            | AddressingMode::Immediate8 => c(insn.operand.value()),
            AddressingMode::Accumulator => Expr::Reg(Reg::A, w),
            _ => match self.address(insn) {
                Some(a) => Expr::mem(a, w),
                None => c(0),
            },
        }
    }

    fn dest(&self, insn: &Instruction, w: Width) -> Place {
        match insn.mode {
            AddressingMode::Accumulator => Place::Reg(Reg::A, w),
            _ => Place::Mem {
                addr: self.address(insn).unwrap_or(c(0)),
                width: w,
            },
        }
    }

    fn decimal_note(&mut self, d: Decimal) {
        if d == Decimal::Unknown && !self.warned_decimal {
            self.warned_decimal = true;
            self.warnings.push(
                "the decimal flag is not known everywhere; ADC and SBC are shown as binary".into(),
            );
        }
    }

    fn insn(&mut self, insn: &Instruction, transfer: &Transfer, dec: Decimal) -> Vec<Stmt> {
        use Mnemonic::*;
        let fl = insn.flags_before;
        let wa = Width::from_flag(fl.eff_m());
        let wx = Width::from_flag(fl.eff_x());
        let a = Expr::Reg(Reg::A, wa);
        let x = Expr::Reg(Reg::X, Width::W16);
        let y = Expr::Reg(Reg::Y, Width::W16);
        let imm = insn.operand.value();
        let mut out = Vec::new();
        let o = &mut out;
        // An index register written at the index width: its high byte is
        // zero while X is set.
        let index_value = |v: Expr| {
            if wx == Width::W8 {
                Expr::cast(Width::W8, v)
            } else {
                v
            }
        };
        match insn.mnemonic {
            LDA => {
                let v = self.source(insn, wa);
                o.push(set(Place::Reg(Reg::A, wa), v));
                nz(o, a, wa);
            }
            LDX | LDY => {
                let r = if insn.mnemonic == LDX { Reg::X } else { Reg::Y };
                let v = self.source(insn, wx);
                o.push(set(Place::Reg(r, Width::W16), v));
                nz(o, Expr::Reg(r, Width::W16), wx);
            }
            STA => o.push(set(self.dest(insn, wa), a)),
            STX => o.push(set(self.dest(insn, wx), x)),
            STY => o.push(set(self.dest(insn, wx), y)),
            STZ => o.push(set(self.dest(insn, wa), c(0))),
            ADC | SBC => {
                let src = self.source(insn, wa);
                let v = self.once(o, src);
                self.decimal_note(dec);
                if dec == Decimal::Set {
                    let helper = if insn.mnemonic == ADC {
                        "bcd_add"
                    } else {
                        "bcd_sub"
                    };
                    o.push(set(
                        Place::Reg(Reg::A, wa),
                        Expr::Call(helper, vec![a.clone(), v, c(wa.bits())]),
                    ));
                } else {
                    let t = self.temp();
                    let tt = Expr::Temp(t);
                    let (sum, overflow) = if insn.mnemonic == ADC {
                        (
                            Expr::sum(Expr::sum(a.clone(), v.clone()), Expr::Flag(Flag::C)),
                            bin(
                                BinOp::And,
                                Expr::un(UnOp::Not, bin(BinOp::Xor, a.clone(), v.clone())),
                                bin(BinOp::Xor, a.clone(), tt.clone()),
                            ),
                        )
                    } else {
                        (
                            bin(
                                BinOp::Sub,
                                bin(BinOp::Sub, a.clone(), v.clone()),
                                Expr::un(UnOp::LNot, Expr::Flag(Flag::C)),
                            ),
                            bin(
                                BinOp::And,
                                bin(BinOp::Xor, a.clone(), v.clone()),
                                bin(BinOp::Xor, a.clone(), tt.clone()),
                            ),
                        )
                    };
                    o.push(set(Place::Temp(t), sum));
                    o.push(flag(
                        Flag::V,
                        bin(BinOp::Ne, bin(BinOp::And, overflow, c(wa.sign())), c(0)),
                    ));
                    o.push(flag(
                        Flag::C,
                        if insn.mnemonic == ADC {
                            bin(BinOp::Gt, tt.clone(), c(wa.mask()))
                        } else {
                            bin(BinOp::Le, tt.clone(), c(wa.mask()))
                        },
                    ));
                    o.push(set(Place::Reg(Reg::A, wa), tt));
                }
                nz(o, a, wa);
            }
            CMP | CPX | CPY => {
                let (r, w) = match insn.mnemonic {
                    CMP => (a.clone(), wa),
                    CPX => (x.clone(), wx),
                    _ => (y.clone(), wx),
                };
                let src = self.source(insn, w);
                let v = self.once(o, src);
                o.push(flag(Flag::C, bin(BinOp::Ge, r.clone(), v.clone())));
                o.push(flag(Flag::Z, bin(BinOp::Eq, r.clone(), v.clone())));
                o.push(flag(
                    Flag::N,
                    bin(
                        BinOp::Ne,
                        bin(BinOp::And, bin(BinOp::Sub, r, v), c(w.sign())),
                        c(0),
                    ),
                ));
            }
            AND | ORA | EOR => {
                let op = match insn.mnemonic {
                    AND => BinOp::And,
                    ORA => BinOp::Or,
                    _ => BinOp::Xor,
                };
                let v = self.source(insn, wa);
                o.push(set(Place::Reg(Reg::A, wa), bin(op, a.clone(), v)));
                nz(o, a, wa);
            }
            BIT => {
                let src = self.source(insn, wa);
                if insn.mode == AddressingMode::ImmediateM {
                    o.push(flag(Flag::Z, bin(BinOp::Eq, bin(BinOp::And, a, src), c(0))));
                } else {
                    let v = self.once(o, src);
                    o.push(flag(
                        Flag::N,
                        bin(BinOp::Ne, bin(BinOp::And, v.clone(), c(wa.sign())), c(0)),
                    ));
                    o.push(flag(
                        Flag::V,
                        bin(
                            BinOp::Ne,
                            bin(BinOp::And, v.clone(), c(wa.sign() >> 1)),
                            c(0),
                        ),
                    ));
                    o.push(flag(Flag::Z, bin(BinOp::Eq, bin(BinOp::And, a, v), c(0))));
                }
            }
            INC | DEC => {
                let op = if insn.mnemonic == INC {
                    BinOp::Add
                } else {
                    BinOp::Sub
                };
                if insn.mode == AddressingMode::Accumulator {
                    o.push(set(Place::Reg(Reg::A, wa), bin(op, a.clone(), c(1))));
                    nz(o, a, wa);
                } else {
                    let t = self.temp();
                    o.push(set(Place::Temp(t), bin(op, self.source(insn, wa), c(1))));
                    o.push(set(self.dest(insn, wa), Expr::Temp(t)));
                    nz(o, Expr::Temp(t), wa);
                }
            }
            INX | INY | DEX | DEY => {
                let r = if matches!(insn.mnemonic, INX | DEX) {
                    Reg::X
                } else {
                    Reg::Y
                };
                let op = if matches!(insn.mnemonic, INX | INY) {
                    BinOp::Add
                } else {
                    BinOp::Sub
                };
                let v = bin(op, Expr::Reg(r, Width::W16), c(1));
                o.push(set(Place::Reg(r, Width::W16), index_value(v)));
                nz(o, Expr::Reg(r, Width::W16), wx);
            }
            ASL | LSR | ROL | ROR => {
                let acc = insn.mode == AddressingMode::Accumulator;
                let v = if acc {
                    a.clone()
                } else {
                    let src = self.source(insn, wa);
                    self.once(o, src)
                };
                let carry_in = Expr::Flag(Flag::C);
                let result = match insn.mnemonic {
                    ASL => bin(BinOp::Shl, v.clone(), c(1)),
                    LSR => bin(BinOp::Shr, v.clone(), c(1)),
                    ROL => bin(BinOp::Or, bin(BinOp::Shl, v.clone(), c(1)), carry_in),
                    _ => bin(
                        BinOp::Or,
                        bin(BinOp::Shr, v.clone(), c(1)),
                        bin(BinOp::Shl, carry_in, c(wa.bits() - 1)),
                    ),
                };
                // The result depends on the old carry, so it is computed
                // before the carry changes.
                let t = self.temp();
                o.push(set(Place::Temp(t), result));
                let carry_out = if matches!(insn.mnemonic, ASL | ROL) {
                    bin(BinOp::Ne, bin(BinOp::And, v, c(wa.sign())), c(0))
                } else {
                    bin(BinOp::And, v, c(1))
                };
                o.push(flag(Flag::C, carry_out));
                o.push(set(self.dest(insn, wa), Expr::Temp(t)));
                nz(o, Expr::Temp(t), wa);
            }
            TSB | TRB => {
                let src = self.source(insn, wa);
                let v = self.once(o, src);
                o.push(flag(
                    Flag::Z,
                    bin(BinOp::Eq, bin(BinOp::And, a.clone(), v.clone()), c(0)),
                ));
                let value = if insn.mnemonic == TSB {
                    bin(BinOp::Or, v, a)
                } else {
                    bin(BinOp::And, v, Expr::un(UnOp::Not, a))
                };
                o.push(set(self.dest(insn, wa), value));
            }
            TAX | TAY => {
                let r = if insn.mnemonic == TAX { Reg::X } else { Reg::Y };
                o.push(set(
                    Place::Reg(r, Width::W16),
                    index_value(Expr::Reg(Reg::A, Width::W16)),
                ));
                nz(o, Expr::Reg(r, Width::W16), wx);
            }
            TXA | TYA => {
                let r = if insn.mnemonic == TXA {
                    x.clone()
                } else {
                    y.clone()
                };
                o.push(set(Place::Reg(Reg::A, wa), r));
                nz(o, a, wa);
            }
            TXY | TYX => {
                let (from, to) = if insn.mnemonic == TXY {
                    (Reg::X, Reg::Y)
                } else {
                    (Reg::Y, Reg::X)
                };
                o.push(set(Place::Reg(to, Width::W16), Expr::Reg(from, Width::W16)));
                nz(o, Expr::Reg(to, Width::W16), wx);
            }
            TCD => {
                o.push(set(
                    Place::Reg(Reg::D, Width::W16),
                    Expr::Reg(Reg::A, Width::W16),
                ));
                nz(o, Expr::Reg(Reg::D, Width::W16), Width::W16);
            }
            TDC | TSC => {
                let from = if insn.mnemonic == TDC { Reg::D } else { Reg::S };
                o.push(set(
                    Place::Reg(Reg::A, Width::W16),
                    Expr::Reg(from, Width::W16),
                ));
                nz(o, Expr::Reg(Reg::A, Width::W16), Width::W16);
            }
            TCS => o.push(set(
                Place::Reg(Reg::S, Width::W16),
                Expr::Reg(Reg::A, Width::W16),
            )),
            TSX => {
                o.push(set(
                    Place::Reg(Reg::X, Width::W16),
                    index_value(Expr::Reg(Reg::S, Width::W16)),
                ));
                nz(o, x, wx);
            }
            TXS => o.push(set(Place::Reg(Reg::S, Width::W16), x)),
            XBA => {
                let a16 = Expr::Reg(Reg::A, Width::W16);
                o.push(set(
                    Place::Reg(Reg::A, Width::W16),
                    bin(
                        BinOp::Or,
                        bin(BinOp::Shr, a16.clone(), c(8)),
                        bin(BinOp::Shl, a16, c(8)),
                    ),
                ));
                nz(o, Expr::Reg(Reg::A, Width::W8), Width::W8);
            }
            CLC => o.push(flag(Flag::C, c(0))),
            SEC => o.push(flag(Flag::C, c(1))),
            CLV => o.push(flag(Flag::V, c(0))),
            SEI => o.push(Stmt::Effect("SEI", vec![])),
            CLI => o.push(Stmt::Effect("CLI", vec![])),
            SED => o.push(Stmt::Effect("SED", vec![])),
            CLD => o.push(Stmt::Effect("CLD", vec![])),
            REP | SEP => {
                let on = insn.mnemonic == SEP;
                let bit = |b: u32| imm & b != 0;
                for (mask, f) in [
                    (0x01, Flag::C),
                    (0x02, Flag::Z),
                    (0x40, Flag::V),
                    (0x80, Flag::N),
                ] {
                    if bit(mask) {
                        o.push(flag(f, c(on as u32)));
                    }
                }
                if bit(0x04) {
                    o.push(Stmt::Effect(if on { "SEI" } else { "CLI" }, vec![]));
                }
                if bit(0x08) {
                    o.push(Stmt::Effect(if on { "SED" } else { "CLD" }, vec![]));
                }
                if bit(0x30) {
                    o.push(Stmt::Note(widths(insn.flags_after)));
                }
            }
            XCE => {
                o.push(flag(Flag::C, c(fl.e as u32)));
                if insn.flags_after.e != fl.e {
                    o.push(Stmt::Effect(
                        if insn.flags_after.e {
                            "emulation_mode"
                        } else {
                            "native_mode"
                        },
                        vec![],
                    ));
                }
                if insn.assumptions & crate::cpu65816::ASSUMED_XCE_CARRY != 0 {
                    o.push(Stmt::Note(
                        "carry unknown here; assumed switching to native mode".into(),
                    ));
                }
            }
            PHA => o.push(push(wa, a)),
            PHX => o.push(push(wx, x)),
            PHY => o.push(push(wx, y)),
            PHB => o.push(push(Width::W8, Expr::Reg(Reg::Dbr, Width::W8))),
            PHD => o.push(push(Width::W16, Expr::Reg(Reg::D, Width::W16))),
            PHK => o.push(push(Width::W8, c(insn.address.bank() as u32))),
            PHP => o.push(Stmt::Effect("PHP", vec![])),
            PEA => o.push(push(Width::W16, c(imm))),
            PEI => o.push(push(
                Width::W16,
                Expr::mem(Expr::sum(self.dp_base(insn), c(imm)), Width::W16),
            )),
            PER => {
                let to = insn.target.map(|t| t.address.offset() as u32).unwrap_or(0);
                o.push(push(Width::W16, c(to)));
            }
            PLA => {
                o.push(set(Place::Reg(Reg::A, wa), pull(wa)));
                nz(o, a, wa);
            }
            PLX | PLY => {
                let r = if insn.mnemonic == PLX { Reg::X } else { Reg::Y };
                o.push(set(Place::Reg(r, Width::W16), pull(wx)));
                nz(o, Expr::Reg(r, Width::W16), wx);
            }
            PLB => {
                o.push(set(Place::Reg(Reg::Dbr, Width::W16), pull(Width::W8)));
                nz(o, Expr::Reg(Reg::Dbr, Width::W16), Width::W8);
            }
            PLD => {
                o.push(set(Place::Reg(Reg::D, Width::W16), pull(Width::W16)));
                nz(o, Expr::Reg(Reg::D, Width::W16), Width::W16);
            }
            PLP => {
                o.push(Stmt::Effect("PLP", vec![]));
                o.push(Stmt::Note(format!(
                    "flags restored; {}",
                    widths(insn.flags_after)
                )));
            }
            JSR | JSL => {
                if let Transfer::Call { callee, .. } = transfer {
                    o.push(Stmt::Call(match callee {
                        Callee::Direct(a) => CallTarget::Direct(*a),
                        Callee::Table { table, targets } => CallTarget::Table {
                            table: *table,
                            targets: targets.clone(),
                            index: x,
                        },
                        Callee::Indirect => CallTarget::Indirect(String::new()),
                    }));
                }
            }
            MVN | MVP => {
                let (src, dst) = match insn.operand {
                    crate::cpu65816::Operand::Move { src, dst } => (src, dst),
                    _ => (0, 0),
                };
                let helper = if insn.mnemonic == MVN { "mvn" } else { "mvp" };
                o.push(Stmt::Effect(helper, vec![c(dst as u32), c(src as u32)]));
                o.push(set(Place::Reg(Reg::Dbr, Width::W16), c(dst as u32)));
            }
            WAI => o.push(Stmt::Effect("wai", vec![])),
            STP => o.push(Stmt::Effect("stp", vec![])),
            BRK => o.push(Stmt::Effect("brk", vec![c(imm)])),
            COP => o.push(Stmt::Effect("cop", vec![c(imm)])),
            NOP | WDM => {}
            // Control transfers are the block's terminator.
            BCC | BCS | BEQ | BMI | BNE | BPL | BVC | BVS | BRA | BRL | JMP | JML | RTS | RTL
            | RTI => {}
        }
        out
    }
}

fn push(w: Width, v: Expr) -> Stmt {
    Stmt::Effect(if w == Width::W8 { "push8" } else { "push16" }, vec![v])
}

fn pull(w: Width) -> Expr {
    Expr::Call(if w == Width::W8 { "pull8" } else { "pull16" }, vec![])
}

/// "8-bit A, 16-bit X and Y".
pub fn widths(f: crate::cpu65816::FlagState) -> String {
    let w = |eight: bool| if eight { "8-bit" } else { "16-bit" };
    if f.eff_m() == f.eff_x() {
        format!("{} A, X and Y", w(f.eff_m()))
    } else {
        format!("{} A, {} X and Y", w(f.eff_m()), w(f.eff_x()))
    }
}
