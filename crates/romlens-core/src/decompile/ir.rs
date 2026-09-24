//! The intermediate form each instruction is lifted to: a few statements
//! that say everything it does, with every width explicit.
//!
//! Registers are 16-bit storage. Writing a register at 8 bits changes only
//! its low byte, which is what the accumulator does with M set; the lifter
//! writes X and Y whole (their high byte is zero while X is set), so that
//! rule never hides a change. Flags are separate values, 0 or 1. Memory is
//! addressed by a 24-bit expression. Address arithmetic does not wrap at
//! bank or page ends; the hardware does, and a routine that relies on it is
//! rare enough to read about in the listing.

use crate::memory::address::SnesAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reg {
    /// The accumulator, all 16 bits (`C` in the manuals).
    A,
    X,
    Y,
    S,
    D,
    Dbr,
}

impl Reg {
    pub const ALL: [Reg; 6] = [Reg::A, Reg::X, Reg::Y, Reg::S, Reg::D, Reg::Dbr];

    /// The global in `snes.h`.
    pub const fn name(self) -> &'static str {
        match self {
            Reg::A => "A",
            Reg::X => "X",
            Reg::Y => "Y",
            Reg::S => "S",
            Reg::D => "D",
            Reg::Dbr => "DBR",
        }
    }
}

/// The status flags the IR models. I and D are side effects (`SEI()`,
/// `SED()`), and M, X and E are static: the decoder knows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Flag {
    N,
    V,
    Z,
    C,
}

impl Flag {
    pub const ALL: [Flag; 4] = [Flag::N, Flag::V, Flag::Z, Flag::C];

    /// The global in `snes.h`.
    pub const fn name(self) -> &'static str {
        match self {
            Flag::N => "N",
            Flag::V => "V",
            Flag::Z => "Z",
            Flag::C => "C",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Width {
    W8,
    W16,
    W24,
}

impl Width {
    pub const fn bytes(self) -> u32 {
        match self {
            Width::W8 => 1,
            Width::W16 => 2,
            Width::W24 => 3,
        }
    }

    pub const fn mask(self) -> u32 {
        match self {
            Width::W8 => 0xFF,
            Width::W16 => 0xFFFF,
            Width::W24 => 0xFF_FFFF,
        }
    }

    pub const fn sign(self) -> u32 {
        match self {
            Width::W8 => 0x80,
            Width::W16 => 0x8000,
            Width::W24 => 0x80_0000,
        }
    }

    pub const fn bits(self) -> u32 {
        self.bytes() * 8
    }

    /// `true` for 8 bits: the sense of M and X.
    pub const fn from_flag(eight: bool) -> Width {
        if eight { Width::W8 } else { Width::W16 }
    }

    /// The unsigned C type.
    pub const fn c_type(self) -> &'static str {
        match self {
            Width::W8 => "u8",
            Width::W16 => "u16",
            Width::W24 => "u32",
        }
    }

    pub const fn c_signed(self) -> &'static str {
        match self {
            Width::W8 => "s8",
            Width::W16 => "s16",
            Width::W24 => "s32",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// `~`
    Not,
    /// `!`
    LNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    /// Unsigned.
    Lt,
    Le,
    Gt,
    Ge,
    LAnd,
    LOr,
}

impl BinOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::LAnd => "&&",
            BinOp::LOr => "||",
        }
    }

    /// C precedence, higher binds tighter.
    pub const fn precedence(self) -> u8 {
        match self {
            BinOp::LOr => 3,
            BinOp::LAnd => 4,
            BinOp::Or => 5,
            BinOp::Xor => 6,
            BinOp::And => 7,
            BinOp::Eq | BinOp::Ne => 8,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 9,
            BinOp::Shl | BinOp::Shr => 10,
            BinOp::Add | BinOp::Sub => 11,
        }
    }

    pub const fn commutative(self) -> bool {
        matches!(
            self,
            BinOp::Add
                | BinOp::And
                | BinOp::Or
                | BinOp::Xor
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::LAnd
                | BinOp::LOr
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Const(u32),
    /// The low `width` of a register.
    Reg(Reg, Width),
    Flag(Flag),
    Temp(u32),
    Mem {
        addr: Box<Expr>,
        width: Width,
    },
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// Truncate to `width`, unsigned.
    Cast(Width, Box<Expr>),
    /// Reinterpret the low `width` as signed.
    Signed(Width, Box<Expr>),
    /// A helper from `snes.h` that returns a value (`pull8()`).
    Call(&'static str, Vec<Expr>),
}

impl Expr {
    pub fn reg(r: Reg, w: Width) -> Expr {
        Expr::Reg(r, w)
    }

    pub fn mem(addr: Expr, width: Width) -> Expr {
        Expr::Mem {
            addr: Box::new(addr),
            width,
        }
    }

    pub fn un(op: UnOp, e: Expr) -> Expr {
        match (op, e) {
            (UnOp::LNot, Expr::Const(v)) => Expr::Const((v == 0) as u32),
            (UnOp::Not, Expr::Const(v)) => Expr::Const(!v),
            (UnOp::LNot, Expr::Un(UnOp::LNot, inner)) if inner.is_boolean() => *inner,
            (op, e) => Expr::Un(op, Box::new(e)),
        }
    }

    /// Always 0 or 1.
    pub fn is_boolean(&self) -> bool {
        match self {
            Expr::Flag(_) | Expr::Un(UnOp::LNot, _) => true,
            Expr::Const(v) => *v <= 1,
            Expr::Bin(op, ..) => matches!(
                op,
                BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
                    | BinOp::LAnd
                    | BinOp::LOr
            ),
            _ => false,
        }
    }

    pub fn cast(w: Width, e: Expr) -> Expr {
        match e {
            Expr::Const(v) => Expr::Const(v & w.mask()),
            Expr::Reg(r, rw) if rw <= w => Expr::Reg(r, rw),
            Expr::Reg(r, _) => Expr::Reg(r, w),
            Expr::Mem { width, .. } if width <= w => e,
            Expr::Flag(_) => e,
            Expr::Cast(inner, e) if inner <= w => Expr::Cast(inner, e),
            e => Expr::Cast(w, Box::new(e)),
        }
    }

    /// A binary operation, folding constants.
    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
        use BinOp::*;
        match (op, &a, &b) {
            (_, Expr::Const(x), Expr::Const(y)) => {
                let (x, y) = (*x, *y);
                let v = match op {
                    Add => Some(x.wrapping_add(y)),
                    Sub => Some(x.wrapping_sub(y)),
                    And => Some(x & y),
                    Or => Some(x | y),
                    Xor => Some(x ^ y),
                    Shl => Some(x.checked_shl(y).unwrap_or(0)),
                    Shr => Some(x.checked_shr(y).unwrap_or(0)),
                    Eq => Some((x == y) as u32),
                    Ne => Some((x != y) as u32),
                    Lt => Some((x < y) as u32),
                    Le => Some((x <= y) as u32),
                    Gt => Some((x > y) as u32),
                    Ge => Some((x >= y) as u32),
                    LAnd => Some((x != 0 && y != 0) as u32),
                    LOr => Some((x != 0 || y != 0) as u32),
                };
                Expr::Const(v.unwrap())
            }
            (Add | Sub | Or | Xor | Shl | Shr, _, Expr::Const(0)) => a,
            (Add | Or | Xor, Expr::Const(0), _) => b,
            // (x ± c1) ± c2
            (Add | Sub, Expr::Bin(inner_op @ (Add | Sub), inner, c1), Expr::Const(c2))
                if matches!(**c1, Expr::Const(_)) =>
            {
                let Expr::Const(c1) = **c1 else {
                    unreachable!()
                };
                let signed = |op: BinOp, v: u32| if op == Add { v as i64 } else { -(v as i64) };
                let total = signed(*inner_op, c1) + signed(op, *c2);
                match total {
                    0 => (**inner).clone(),
                    t if t > 0 => Expr::Bin(Add, inner.clone(), Box::new(Expr::Const(t as u32))),
                    t => Expr::Bin(Sub, inner.clone(), Box::new(Expr::Const((-t) as u32))),
                }
            }
            // A constant on the right reads better.
            (_, Expr::Const(_), _) if op.commutative() => Expr::Bin(op, Box::new(b), Box::new(a)),
            _ => Expr::Bin(op, Box::new(a), Box::new(b)),
        }
    }

    pub fn sum(a: Expr, b: Expr) -> Expr {
        Expr::bin(BinOp::Add, a, b)
    }

    pub fn is_const(&self) -> bool {
        matches!(self, Expr::Const(_))
    }

    pub fn as_const(&self) -> Option<u32> {
        match self {
            Expr::Const(v) => Some(*v),
            _ => None,
        }
    }

    /// Reads memory or calls a helper, so evaluating it twice is not the
    /// same as evaluating it once (a hardware register, `pull8()`).
    pub fn has_effects(&self) -> bool {
        match self {
            Expr::Mem { .. } | Expr::Call(..) => true,
            Expr::Un(_, e) | Expr::Cast(_, e) | Expr::Signed(_, e) => e.has_effects(),
            Expr::Bin(_, a, b) => a.has_effects() || b.has_effects(),
            _ => false,
        }
    }

    /// Visit this expression and every one inside it.
    pub fn walk(&self, f: &mut impl FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Mem { addr, .. } => addr.walk(f),
            Expr::Un(_, e) | Expr::Cast(_, e) | Expr::Signed(_, e) => e.walk(f),
            Expr::Bin(_, a, b) => {
                a.walk(f);
                b.walk(f);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| a.walk(f)),
            _ => {}
        }
    }
}

/// Where a statement stores.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Place {
    /// At `W8`, only the low byte changes.
    Reg(Reg, Width),
    Flag(Flag),
    Temp(u32),
    Mem {
        addr: Expr,
        width: Width,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    Direct(SnesAddress),
    /// `JSR (abs,X)`: the entry the index selects.
    Table {
        table: SnesAddress,
        targets: Vec<SnesAddress>,
        index: Expr,
    },
    /// Through a pointer nothing resolved; the instruction, for the comment.
    Indirect(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Assign {
        dst: Place,
        value: Expr,
    },
    Call(CallTarget),
    /// A helper from `snes.h` called for its effect (`SEI()`, `push8(A)`).
    Effect(&'static str, Vec<Expr>),
    /// An instruction kept as a comment, and why.
    Asm {
        text: String,
        note: String,
    },
    /// A remark for the reader (`16-bit A`).
    Note(String),
}

/// One statement and the instruction it came from (an index into
/// `Function::steps`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub stmt: Stmt,
    pub step: usize,
}

/// A block's lifted statements and how it ends.
#[derive(Debug, Clone, Default)]
pub struct LiftedBlock {
    pub lines: Vec<Line>,
    /// For a `Term::Branch`: taken when this is true.
    pub cond: Option<Expr>,
    /// For a `Term::Switch`: the index, and each case's value.
    pub switch: Option<(Expr, Vec<u32>)>,
    /// The step the terminator came from, for the line map.
    pub term_step: Option<usize>,
}
