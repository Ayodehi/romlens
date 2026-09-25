//! Printing C: names, expressions, statements and the block layout.
//!
//! Names are chosen in this order: a variable (typed from its `VarType`), a
//! hardware register, a label, then a plain `MEM8(0x7E0094)`. Every name the
//! routine uses is declared at the top, so the result is a translation unit
//! a compiler accepts against `snes.h`. The writer records which instructions
//! each line came from and a token for every name and number, which is how
//! the app keeps the C and the disassembly in step.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::snapshot::AnalysisSnapshot;
use crate::cpu65816::format_instruction;
use crate::decompile::cfg::{BlockId, Cfg, Term};
use crate::decompile::function::Function;
use crate::decompile::header::{GLOBALS, HELPERS};
use crate::decompile::ir::{
    BinOp, CallTarget, Expr, LiftedBlock, Place, Stmt, UnOp, VarDecl, Width,
};
use crate::decompile::signature::Abi;
use crate::memory::address::SnesAddress;
use crate::memory::map::MemoryClass;
use crate::model::hardware::{all_hardware_registers, hardware_register};
use crate::model::project::Project;
use crate::model::region::RegionKind;
use crate::model::symbols::Symbols;
use crate::model::variable::{VarType, VarWidth};
use crate::rom::image::RomImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CTokenKind {
    Keyword,
    Type,
    Number,
    Comment,
    /// A routine; `address` is its entry.
    Function,
    /// A variable the project defines.
    Variable,
    Register,
    /// A labelled address used as data.
    Label,
    /// A `snes.h` helper.
    Helper,
    /// A CPU register, flag or temporary.
    Local,
    /// A goto label.
    GotoLabel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CToken {
    /// Byte offset into the text.
    pub start: u32,
    pub len: u32,
    pub kind: CTokenKind,
    pub address: Option<SnesAddress>,
}

#[derive(Debug, Default)]
pub struct Writer {
    pub text: String,
    pub tokens: Vec<CToken>,
    /// For each line, the steps it came from.
    pub lines: Vec<Vec<usize>>,
    open: bool,
    pub indent: usize,
}

impl Writer {
    fn open(&mut self) {
        if !self.open {
            self.open = true;
            for _ in 0..self.indent {
                self.text.push_str("    ");
            }
        }
    }

    pub fn w(&mut self, s: &str) {
        self.open();
        self.text.push_str(s);
    }

    pub fn tok(&mut self, s: &str, kind: CTokenKind, address: Option<SnesAddress>) {
        self.open();
        self.tokens.push(CToken {
            start: self.text.len() as u32,
            len: s.len() as u32,
            kind,
            address,
        });
        self.text.push_str(s);
    }

    pub fn end(&mut self, steps: &[usize]) {
        self.text.push('\n');
        self.lines.push(steps.to_vec());
        self.open = false;
    }

    pub fn blank(&mut self) {
        self.end(&[]);
    }

    pub fn comment_line(&mut self, text: &str, steps: &[usize]) {
        self.tok(
            &format!("/* {} */", safe_comment(text)),
            CTokenKind::Comment,
            None,
        );
        self.end(steps);
    }
}

/// Text that cannot end the comment it goes in.
pub fn safe_comment(s: &str) -> String {
    s.replace("*/", "* /")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Function,
    Object,
    Table,
}

/// The shape of a named object in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// One byte or word: `u8 Name;`.
    Scalar(u32),
    /// Bytes or words: `u16 Name[4];`.
    Array(u32),
    /// Plain bytes: a long variable, or a label's `u8 Name[]`.
    Bytes,
}

pub struct Namer<'a> {
    rom: &'a RomImage,
    project: &'a Project,
    snap: &'a AnalysisSnapshot,
    pub use_names: bool,
    taken: BTreeMap<String, (SnesAddress, Kind)>,
    /// Name to declaration, functions first.
    decls: BTreeMap<(u8, String), String>,
    reserved: BTreeSet<&'static str>,
    /// What each routine reads and returns, for its declaration.
    pub summaries: Option<&'a BTreeMap<SnesAddress, crate::decompile::signature::Summary>>,
    /// RAM nothing names yet, by address: its width in bytes, or `None`
    /// where it is only ever indexed (an array). Printed as `ADDR_7E0000`
    /// until a variable names it.
    pub placeholders: BTreeMap<SnesAddress, Option<u32>>,
    /// Each routine's C signature, at the `full` level.
    pub abis: Option<&'a BTreeMap<SnesAddress, Abi>>,
}

/// The `full` level's variable names, which a label must not take.
const LOCALS: [&str; 22] = [
    "a", "x", "y", "c", "n", "v", "z", "a8", "a16", "x8", "x16", "y8", "y16", "a_out", "x_out",
    "y_out", "c_out", "n_out", "v_out", "z_out", "i", "j",
];

const KEYWORDS: [&str; 44] = [
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Bool",
    "_Complex",
    "_Imaginary",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Generic",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
];

impl<'a> Namer<'a> {
    pub fn new(
        rom: &'a RomImage,
        project: &'a Project,
        snap: &'a AnalysisSnapshot,
        use_names: bool,
    ) -> Self {
        let mut reserved: BTreeSet<&'static str> = BTreeSet::new();
        reserved.extend(KEYWORDS);
        reserved.extend(HELPERS);
        reserved.extend(GLOBALS);
        reserved.extend(all_hardware_registers().iter().map(|r| r.name));
        reserved.extend(["uintptr_t", "main"]);
        reserved.extend(LOCALS);
        Self {
            rom,
            project,
            snap,
            use_names,
            taken: BTreeMap::new(),
            decls: BTreeMap::new(),
            reserved,
            summaries: None,
            placeholders: BTreeMap::new(),
            abis: None,
        }
    }

    fn canonical(&self, a: u32) -> SnesAddress {
        Project::canonical(self.rom, SnesAddress::from_u24(a & 0xFF_FFFF))
    }

    fn label(&self, at: SnesAddress) -> Option<&'a str> {
        self.project
            .labels
            .get(&at)
            .or_else(|| self.snap.auto_labels.get(&at))
            .map(|l| l.name.as_str())
    }

    /// A C identifier for `raw` naming `at`, unique in this unit.
    fn claim(&mut self, raw: &str, at: SnesAddress, kind: Kind) -> String {
        let mut name: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
            name.insert(0, '_');
        }
        if self.reserved.contains(name.as_str()) {
            name.push('_');
        }
        match self.taken.get(&name) {
            Some(&(a, k)) if a == at && k == kind => name,
            Some(_) => {
                let alt = format!("{name}_{:06X}", at.as_u24());
                self.taken.insert(alt.clone(), (at, kind));
                alt
            }
            None => {
                self.taken.insert(name.clone(), (at, kind));
                name
            }
        }
    }

    pub fn function(&mut self, at: SnesAddress) -> String {
        let at = Project::canonical(self.rom, at);
        let raw = self
            .label(at)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("sub_{:06X}", at.as_u24()));
        let name = self.claim(&raw, at, Kind::Function);
        let note = self
            .summaries
            .and_then(|s| s.get(&at))
            .map(|s| format!(" /* {} */", crate::decompile::signature::describe(s)))
            .unwrap_or_default();
        let sig = self
            .abis
            .and_then(|m| m.get(&at))
            .map(|abi| abi.c_signature(&name))
            .unwrap_or_else(|| format!("void {name}(void)"));
        self.decls
            .insert((0, name.clone()), format!("{sig};{note}"));
        name
    }

    /// Every routine and table the unit calls, by name.
    pub fn callees(&self) -> Vec<(String, SnesAddress, bool)> {
        self.taken
            .iter()
            .filter(|(n, (_, k))| {
                matches!(k, Kind::Function | Kind::Table)
                    && self
                        .decls
                        .contains_key(&(if *k == Kind::Function { 0 } else { 1 }, (*n).clone()))
            })
            .map(|(n, (a, k))| (n.clone(), *a, *k == Kind::Table))
            .collect()
    }

    /// The name of the routine being printed: declared by its definition.
    pub fn own(&mut self, at: SnesAddress) -> String {
        let name = self.function(at);
        self.decls.remove(&(0, name.clone()));
        name
    }

    pub fn table(&mut self, at: SnesAddress) -> String {
        let at = Project::canonical(self.rom, at);
        let raw = self
            .label(at)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("table_{:06X}", at.as_u24()));
        let name = self.claim(&raw, at, Kind::Table);
        self.decls.insert(
            (1, name.clone()),
            format!("extern void (*const {name}[])(void);"),
        );
        name
    }

    /// A named object whose bytes include `c`: its name, shape, and how far
    /// in `c` is.
    fn object(&mut self, c: SnesAddress) -> Option<(String, Shape, u32, SnesAddress)> {
        if let Some((start, ty)) = self.project.variable_containing(c)
            && let Some(l) = self.project.labels.get(&start)
        {
            let name = self.claim(&l.name.clone(), start, Kind::Object);
            let (shape, decl) = var_decl(&name, ty);
            self.decls.insert((2, name.clone()), decl);
            return Some((name, shape, (c.offset() - start.offset()) as u32, start));
        }
        let Some(raw) = self.label(c).map(str::to_owned) else {
            return self.placeholder(c);
        };
        // A label on code is a routine, not data.
        if let Some(off) = self.rom.file_offset_for(c)
            && self
                .snap
                .region_at(off)
                .is_some_and(|r| r.kind == RegionKind::Code)
        {
            return None;
        }
        let name = self.claim(&raw, c, Kind::Object);
        self.decls
            .insert((3, name.clone()), format!("extern u8 {name}[];"));
        Some((name, Shape::Bytes, 0, c))
    }

    /// `ADDR_7E0000`: RAM the code uses that nothing names yet, typed by
    /// how it is used, covering `c`.
    fn placeholder(&mut self, c: SnesAddress) -> Option<(String, Shape, u32, SnesAddress)> {
        let (&start, &len) = self.placeholders.range(..=c).next_back()?;
        if start.bank() != c.bank() {
            return None;
        }
        let k = (c.offset() - start.offset()) as u32;
        let (shape, ty) = match len {
            Some(n) if k < n => (Shape::Scalar(n), n),
            None if k == 0 => (Shape::Bytes, 0),
            _ => return None,
        };
        let name = self.claim(&format!("ADDR_{:06X}", start.as_u24()), start, Kind::Object);
        let decl = match ty {
            0 => format!("extern u8 {name}[];"),
            1 => format!("extern u8 {name};"),
            2 => format!("extern u16 {name};"),
            _ => format!("extern u32 {name}; /* 24-bit */"),
        };
        self.decls.insert((3, name.clone()), decl);
        Some((name, shape, k, start))
    }

    /// Work out the placeholders a routine needs from the addresses it
    /// uses: each exact address at its widest access, an indexed base as an
    /// array, and an address inside a wider one left to that one.
    pub fn plan_placeholders(&mut self, uses: &[(u32, Width, bool)]) {
        if !self.use_names {
            return;
        }
        let mut exact: BTreeMap<SnesAddress, u32> = BTreeMap::new();
        let mut indexed: BTreeSet<SnesAddress> = BTreeSet::new();
        for &(a, w, is_indexed) in uses {
            let c = self.canonical(a);
            if !matches!(
                self.rom.map().classify(c),
                MemoryClass::Wram | MemoryClass::LowRam | MemoryClass::Sram
            ) || self.project.variable_containing(c).is_some()
                || self.label(c).is_some()
            {
                continue;
            }
            if is_indexed {
                indexed.insert(c);
            } else {
                let e = exact.entry(c).or_insert(0);
                *e = (*e).max(w.bytes().min(3));
            }
        }
        let mut out: BTreeMap<SnesAddress, Option<u32>> = BTreeMap::new();
        let mut covered_to: Option<(u8, u16)> = None;
        for (a, n) in exact {
            if let Some((bank, end)) = covered_to
                && bank == a.bank()
                && a.offset() < end
            {
                continue;
            }
            out.insert(a, Some(n));
            covered_to = Some((a.bank(), a.offset().saturating_add(n as u16)));
        }
        for a in indexed {
            let inside = out.range(..=a).next_back().is_some_and(|(s, n)| {
                s.bank() == a.bank() && n.is_some_and(|n| (a.offset() - s.offset()) < n as u16)
            });
            if !inside {
                out.entry(a).or_insert(None);
            }
        }
        self.placeholders = out;
    }

    /// Names for bytes of RAM: each object that holds some of them once,
    /// `MEM8(0x7E0000)` for a byte nothing names.
    pub fn ram_names(&mut self, bytes: &BTreeSet<u32>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for &b in bytes {
            let c = SnesAddress::from_u24(b);
            let name = match self.use_names.then(|| self.object(c)).flatten() {
                Some((name, ..)) => name,
                None => format!("MEM8(0x{b:06X})"),
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    /// A goto label for code at `at`: the listing's label for it (a
    /// `LOOP_`, a `SKIP_`, the user's name) as an identifier, else `L_`
    /// and the address. Labels have their own namespace in C.
    pub fn goto_label(&self, at: SnesAddress) -> String {
        let at = Project::canonical(self.rom, at);
        let Some(raw) = self.label(at).filter(|_| self.use_names) else {
            return format!("L_{:06X}", at.as_u24());
        };
        let mut name: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
            name.insert(0, '_');
        }
        if KEYWORDS.contains(&name.as_str()) {
            name.push('_');
        }
        name
    }

    pub fn declarations(&self) -> Vec<&str> {
        self.decls.values().map(String::as_str).collect()
    }

    fn is_hardware(&self, c: SnesAddress) -> Option<&'static str> {
        (self.rom.map().classify(c) == MemoryClass::Hardware)
            .then(|| hardware_register(c.offset()).map(|r| r.name))
            .flatten()
    }
}

fn var_decl(name: &str, ty: VarType) -> (Shape, String) {
    let ew = ty.width.bytes();
    match (ty.width, ty.count) {
        // C has no 24-bit type: a u32 holds it.
        (VarWidth::Long, 1) => (Shape::Scalar(3), format!("extern u32 {name}; /* 24-bit */")),
        (VarWidth::Long, n) => (
            Shape::Bytes,
            format!(
                "extern u8 {name}[{}]; /* {} */",
                3 * n as u32,
                ty.describe()
            ),
        ),
        (w, 1) => (
            Shape::Scalar(ew),
            format!(
                "extern {} {name};",
                if w == VarWidth::Byte { "u8" } else { "u16" }
            ),
        ),
        (w, n) => (
            Shape::Array(ew),
            format!(
                "extern {} {name}[{n}];",
                if w == VarWidth::Byte { "u8" } else { "u16" }
            ),
        ),
    }
}

/// C precedence of an expression's outermost operator.
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Bin(op, ..) => op.precedence(),
        Expr::Un(..) | Expr::Cast(..) | Expr::Signed(..) | Expr::Step(..) => 14,
        _ => 16,
    }
}

fn bitwise(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Shl | BinOp::Shr | BinOp::LAnd | BinOp::LOr
    )
}

fn comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

/// Whether a child of `parent` needs parentheses: when C's precedence says
/// so, and wherever bitwise and other operators mix, which C readers (and
/// `-Wparentheses`) do not like to leave to precedence.
fn needs_parens(parent: BinOp, child: &Expr, right: bool) -> bool {
    let Expr::Bin(op, ..) = child else {
        return prec(child) < parent.precedence();
    };
    let (p, c) = (parent.precedence(), op.precedence());
    if c < p {
        return true;
    }
    // `a == 0 || b == 0`: a test inside `&&` or `||` reads plainly.
    if matches!(parent, BinOp::LAnd | BinOp::LOr) && comparison(*op) {
        return false;
    }
    if *op != parent && (bitwise(parent) || bitwise(*op)) {
        return true;
    }
    c == p && right && !(*op == parent && parent.commutative())
}

/// `v` in the style asked for.
pub fn number_in(v: u32, style: super::NumberStyle) -> String {
    use super::NumberStyle::*;
    match style {
        Auto => number(v),
        Decimal => v.to_string(),
        Hex => {
            if v <= 0xFF {
                format!("0x{v:02X}")
            } else if v <= 0xFFFF {
                format!("0x{v:04X}")
            } else {
                format!("0x{v:06X}")
            }
        }
        Binary => {
            let bits = if v <= 0xFF {
                8
            } else if v <= 0xFFFF {
                16
            } else {
                24
            };
            format!("0b{v:0bits$b}")
        }
    }
}

pub fn number(v: u32) -> String {
    if v < 10 {
        v.to_string()
    } else if v <= 0xFF {
        format!("0x{v:02X}")
    } else if v <= 0xFFFF {
        format!("0x{v:04X}")
    } else {
        format!("0x{v:06X}")
    }
}

/// Everything the printer needs about one routine.
pub struct Emitter<'a, 'n> {
    pub w: Writer,
    pub names: &'n mut Namer<'a>,
    pub stats: Stats,
    /// The `full` level's variables.
    pub vars: Vec<VarDecl>,
    /// Print as C is written (`x++`, `a |= 4`), not statement by statement.
    pub modern: bool,
    /// The RAM each call's caller passes, by the call's step.
    pub mem_args: BTreeMap<usize, BTreeSet<u32>>,
    /// How numbers print.
    pub numbers: super::NumberStyle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub instructions: u32,
    /// Statements printed, from the instructions' own lines.
    pub statements: u32,
    pub blocks: u32,
    pub gotos: u32,
    pub asm_comments: u32,
}

impl<'a, 'n> Emitter<'a, 'n> {
    pub fn new(names: &'n mut Namer<'a>) -> Self {
        Self {
            w: Writer::default(),
            names,
            stats: Stats::default(),
            vars: Vec::new(),
            modern: false,
            mem_args: BTreeMap::new(),
            numbers: super::NumberStyle::Auto,
        }
    }

    fn num(&mut self, v: u32) {
        self.w
            .tok(&number_in(v, self.numbers), CTokenKind::Number, None);
    }

    fn kw(&mut self, s: &str) {
        self.w.tok(s, CTokenKind::Keyword, None);
    }

    fn ty(&mut self, s: &str) {
        self.w.tok(s, CTokenKind::Type, None);
    }

    fn local(&mut self, s: &str) {
        self.w.tok(s, CTokenKind::Local, None);
    }

    pub fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Const(v) => self.num(*v),
            Expr::Reg(r, w) => {
                if *w == Width::W8 {
                    self.w.w("(");
                    self.ty("u8");
                    self.w.w(")");
                }
                self.local(r.name());
            }
            Expr::Flag(f) => self.local(f.name()),
            Expr::Temp(t) => self.local(&format!("t{t}")),
            Expr::Mem { addr, width } => self.mem(addr, *width),
            Expr::Un(op, inner) => {
                self.w.w(match op {
                    UnOp::Not => "~",
                    UnOp::LNot => "!",
                });
                self.paren_if(inner, prec(inner) < 14);
            }
            Expr::Bin(op, a, b) => {
                self.paren_if(a, needs_parens(*op, a, false));
                self.w.w(" ");
                self.w.w(op.symbol());
                self.w.w(" ");
                match (op, &**b) {
                    // A shift count reads better in decimal.
                    (BinOp::Shl | BinOp::Shr, Expr::Const(n)) => {
                        self.w.tok(&n.to_string(), CTokenKind::Number, None)
                    }
                    _ => self.paren_if(b, needs_parens(*op, b, true)),
                }
            }
            Expr::Cast(w, inner) => {
                self.w.w("(");
                self.ty(w.c_type());
                self.w.w(")");
                self.paren_if(inner, prec(inner) < 14);
            }
            Expr::Signed(w, inner) => {
                self.w.w("(");
                self.ty(w.c_signed());
                self.w.w(")");
                self.paren_if(inner, prec(inner) < 14);
            }
            Expr::Call(name, args) => {
                self.w.tok(name, CTokenKind::Helper, None);
                self.args(args);
            }
            Expr::Var(v) => {
                let name = self.var_name(*v);
                self.local(&name);
            }
            Expr::Global(k) => self.local(k.global()),
            Expr::Step(op, v) => {
                self.w.w(if *op == BinOp::Sub { "--" } else { "++" });
                let name = self.var_name(*v);
                self.local(&name);
            }
        }
    }

    fn var_name(&self, v: u32) -> String {
        self.vars
            .get(v as usize)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("v{v}"))
    }

    /// Whether a 24-bit store to `addr` can be a plain assignment: a u32
    /// variable or placeholder starts there.
    fn names_24(&mut self, addr: &Expr) -> bool {
        if !self.names.use_names {
            return false;
        }
        let Expr::Const(a) = addr else {
            return false;
        };
        let c = self.names.canonical(*a);
        self.names.is_hardware(c).is_none()
            && matches!(self.names.object(c), Some((_, Shape::Scalar(3), 0, _)))
    }

    /// An address to pass as one: a named object's own name where one
    /// starts there, else the number.
    fn address_of(&mut self, addr: &Expr) {
        if self.names.use_names
            && let Expr::Const(a) = addr
        {
            let c = self.names.canonical(*a);
            if self.names.is_hardware(c).is_none()
                && let Some((name, shape, 0, start)) = self.names.object(c)
            {
                let kind = if self.names.project.variables.contains_key(&start) {
                    CTokenKind::Variable
                } else {
                    CTokenKind::Label
                };
                if !matches!(shape, Shape::Bytes) {
                    self.w.w("&");
                }
                self.w.tok(&name, kind, Some(start));
                return;
            }
        }
        self.address(addr);
    }

    /// An address: a base and an index read base first, `0x7F8002 + X`;
    /// a constant as an address, `0x0000` rather than `0`.
    fn address(&mut self, addr: &Expr) {
        if let Expr::Const(a) = addr {
            let text = if *a > 0xFFFF {
                format!("0x{a:06X}")
            } else {
                format!("0x{a:04X}")
            };
            self.w.tok(&text, CTokenKind::Number, None);
            return;
        }
        if let Expr::Bin(BinOp::Add, i, base) = addr
            && let Expr::Const(b) = **base
            && b > 0xFF
        {
            self.w.tok(&number(b), CTokenKind::Number, None);
            self.w.w(" + ");
            self.paren_if(i, needs_parens(BinOp::Add, i, true));
        } else {
            self.expr(addr);
        }
    }

    fn args(&mut self, args: &[Expr]) {
        self.w.w("(");
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.w.w(", ");
            }
            self.expr(a);
        }
        self.w.w(")");
    }

    fn paren_if(&mut self, e: &Expr, yes: bool) {
        if yes {
            self.w.w("(");
            self.expr(e);
            self.w.w(")");
        } else {
            self.expr(e);
        }
    }

    /// Memory at `addr`: by name where it has one.
    fn mem(&mut self, addr: &Expr, width: Width) {
        if self.names.use_names {
            match addr {
                Expr::Const(a) => {
                    if self.named(*a, None, width) {
                        return;
                    }
                }
                Expr::Bin(BinOp::Add, x, y) => {
                    let split = match (&**x, &**y) {
                        (i, Expr::Const(a)) | (Expr::Const(a), i) => Some((*a, i)),
                        _ => None,
                    };
                    if let Some((a, i)) = split
                        && self.named(a, Some(i), width)
                    {
                        return;
                    }
                }
                _ => {}
            }
        }
        let helper = match width {
            Width::W8 => "MEM8",
            Width::W16 => "MEM16",
            // Memory is never read as 32 bits.
            Width::W24 | Width::W32 => "MEM24",
        };
        self.w.tok(helper, CTokenKind::Helper, None);
        self.w.w("(");
        self.address(addr);
        self.w.w(")");
    }

    /// Memory at `a` (plus `index`) by name; `false` when nothing names it.
    fn named(&mut self, a: u32, index: Option<&Expr>, width: Width) -> bool {
        let c = self.names.canonical(a);
        if let Some(reg) = self.names.is_hardware(c) {
            let r = SnesAddress::new(0, c.offset());
            match (index, width) {
                (None, Width::W8) => self.w.tok(reg, CTokenKind::Register, Some(r)),
                (None, Width::W16) => {
                    self.w.w("*(");
                    self.ty("u16");
                    self.w.w(" *)&");
                    self.w.tok(reg, CTokenKind::Register, Some(r));
                }
                (Some(i), Width::W8) => {
                    self.w.w("(&");
                    self.w.tok(reg, CTokenKind::Register, Some(r));
                    self.w.w(")[");
                    self.expr(i);
                    self.w.w("]");
                }
                (Some(i), Width::W16) => {
                    self.w.w("*(");
                    self.ty("u16");
                    self.w.w(" *)(&");
                    self.w.tok(reg, CTokenKind::Register, Some(r));
                    self.w.w(" + ");
                    self.paren_if(i, needs_parens(BinOp::Add, i, true));
                    self.w.w(")");
                }
                _ => return false,
            }
            return true;
        }
        let Some((name, shape, k, start)) = self.names.object(c) else {
            return false;
        };
        let kind = if self.names.project.variables.contains_key(&start) {
            CTokenKind::Variable
        } else {
            CTokenKind::Label
        };
        let n = width.bytes();
        let name_tok = |e: &mut Self| e.w.tok(&name, kind, Some(start));
        // The element itself, where the access is exactly one.
        match (shape, index) {
            (Shape::Scalar(ew), None) if k == 0 && n == ew => {
                name_tok(self);
                return true;
            }
            (Shape::Array(ew), None) if k % ew == 0 && n == ew => {
                name_tok(self);
                self.w.w("[");
                self.num(k / ew);
                self.w.w("]");
                return true;
            }
            (Shape::Array(1) | Shape::Bytes, _) if n == 1 => {
                name_tok(self);
                self.w.w("[");
                self.offset(k, index);
                self.w.w("]");
                return true;
            }
            (Shape::Array(1) | Shape::Bytes, _) if n == 2 => {
                self.w.w("*(");
                self.ty("u16");
                self.w.w(" *)&");
                name_tok(self);
                self.w.w("[");
                self.offset(k, index);
                self.w.w("]");
                return true;
            }
            _ => {}
        }
        // Otherwise through a byte pointer to the object.
        let base = |e: &mut Self| {
            e.w.w("(");
            e.ty("u8");
            e.w.w(" *)");
            if matches!(shape, Shape::Scalar(_)) {
                e.w.w("&");
            }
            name_tok(e);
        };
        let has_offset = k != 0 || index.is_some();
        match width {
            Width::W8 => {
                self.w.w("(");
                base(self);
                self.w.w(")[");
                self.offset(k, index);
                self.w.w("]");
            }
            Width::W16 => {
                self.w.w("*(");
                self.ty("u16");
                self.w.w(" *)");
                if has_offset {
                    self.w.w("(");
                    base(self);
                    self.w.w(" + ");
                    self.offset(k, index);
                    self.w.w(")");
                } else {
                    base(self);
                }
            }
            Width::W24 | Width::W32 => {
                self.w.tok("LONG", CTokenKind::Helper, None);
                self.w.w("(");
                base(self);
                if has_offset {
                    self.w.w(" + ");
                    self.offset(k, index);
                }
                self.w.w(")");
            }
        }
        true
    }

    fn offset(&mut self, k: u32, index: Option<&Expr>) {
        match index {
            None => self.num(k),
            Some(i) if k == 0 => self.expr(i),
            Some(i) => {
                self.paren_if(i, needs_parens(BinOp::Add, i, false));
                self.w.w(" + ");
                self.num(k);
            }
        }
    }

    fn place(&mut self, p: &Place) {
        match p {
            Place::Reg(r, _) => self.local(r.name()),
            Place::Flag(f) => self.local(f.name()),
            Place::Temp(t) => self.local(&format!("t{t}")),
            Place::Mem { addr, width } => self.mem(addr, *width),
            Place::Var(v) => {
                let name = self.var_name(*v);
                self.local(&name);
            }
            Place::Global(k) => self.local(k.global()),
            Place::Out(k) => {
                self.w.w("*");
                self.local(&format!("{}_out", k.name()));
            }
        }
    }

    /// One statement on its own line.
    pub fn stmt(&mut self, s: &Stmt, steps: &[usize], asm: &dyn Fn(usize) -> String) {
        if !matches!(s, Stmt::Note(_)) {
            self.stats.statements += 1;
        }
        match s {
            // A 24-bit store with no 24-bit name to assign to: SET24.
            Stmt::Assign {
                dst:
                    Place::Mem {
                        addr,
                        width: Width::W24,
                    },
                value,
            } if !self.names_24(addr) => {
                self.w.tok("SET24", CTokenKind::Helper, None);
                self.w.w("(");
                self.address_of(addr);
                self.w.w(", ");
                self.expr(value);
                self.w.w(");");
                self.w.end(steps);
            }
            Stmt::Assign { dst, value } if self.modern && self.compound(dst, value).is_some() => {
                let (op, rhs) = self.compound(dst, value).unwrap();
                self.place(dst);
                // `++` binds tighter than a `*` or `&` a memory place
                // starts with, so memory takes `+= 1`.
                let var = matches!(dst, Place::Var(_));
                match (op, rhs.as_const()) {
                    (BinOp::Add, Some(1)) if var => self.w.w("++"),
                    (BinOp::Sub, Some(1)) if var => self.w.w("--"),
                    _ => {
                        self.w.w(" ");
                        self.w.w(op.symbol());
                        self.w.w("= ");
                        self.expr(&rhs);
                    }
                }
                self.w.w(";");
                self.w.end(steps);
            }
            Stmt::Assign { dst, value } => {
                self.place(dst);
                self.w.w(" = ");
                if let Place::Reg(crate::decompile::ir::Reg::A, Width::W8) = dst {
                    // Only the low byte changes.
                    self.w.w("(");
                    self.local("A");
                    self.w.w(" & ");
                    self.num(0xFF00);
                    self.w.w(") | ");
                    self.byte(value);
                } else {
                    self.expr(value);
                }
                self.w.w(";");
                self.w.end(steps);
            }
            Stmt::Call(target) => {
                match target {
                    CallTarget::Direct(a) => {
                        let name = self.names.function(*a);
                        self.w.tok(&name, CTokenKind::Function, Some(*a));
                        self.w.w("(");
                        self.passed_in_memory(steps, true);
                        self.w.w(");");
                    }
                    CallTarget::Table { table, index, .. } => {
                        let name = self.names.table(*table);
                        self.w.tok(&name, CTokenKind::Label, Some(*table));
                        self.w.w("[");
                        self.paren_if(index, false);
                        self.w.w(" >> 1]();");
                    }
                    CallTarget::Indirect(_) => {
                        self.stats.asm_comments += 1;
                        let text = steps.first().map(|&i| asm(i)).unwrap_or_default();
                        self.w.tok(
                            &format!(
                                "/* asm: {}: a call through a pointer nothing resolved */",
                                safe_comment(&text)
                            ),
                            CTokenKind::Comment,
                            None,
                        );
                    }
                }
                self.w.end(steps);
            }
            Stmt::Effect(name, args) => {
                self.w.tok(name, CTokenKind::Helper, None);
                self.args(args);
                self.w.w(";");
                self.w.end(steps);
            }
            Stmt::Asm { text, note } => {
                self.stats.asm_comments += 1;
                self.w.comment_line(&format!("asm: {text}: {note}"), steps);
            }
            Stmt::Note(n) => self.w.comment_line(n, steps),
            Stmt::Eval(e) => {
                self.w.w("(");
                self.ty("void");
                self.w.w(")");
                self.expr(e);
                self.w.w(";");
                self.w.end(steps);
            }
            Stmt::Invoke {
                target,
                args,
                ret,
                outs,
            } => {
                if let Some(r) = ret {
                    self.place(r);
                    self.w.w(" = ");
                }
                let name = self.names.function(*target);
                self.w.tok(&name, CTokenKind::Function, Some(*target));
                self.w.w("(");
                let mut first = true;
                for a in args {
                    if !first {
                        self.w.w(", ");
                    }
                    first = false;
                    self.expr(a);
                }
                for o in outs {
                    if !first {
                        self.w.w(", ");
                    }
                    first = false;
                    self.w.w("&");
                    self.place(o);
                }
                self.passed_in_memory(steps, first);
                self.w.w(");");
                self.w.end(steps);
            }
        }
    }

    /// The RAM this call's caller stored for it, as a comment among the
    /// arguments: `SUB_8079(/* ADDR_7E0000 */)`.
    fn passed_in_memory(&mut self, steps: &[usize], first: bool) {
        let Some(bytes) = steps.first().and_then(|s| self.mem_args.get(s)).cloned() else {
            return;
        };
        let names = self.names.ram_names(&bytes);
        if names.is_empty() {
            return;
        }
        if !first {
            self.w.w(" ");
        }
        self.w.tok(
            &format!("/* {} */", safe_comment(&names.join(", "))),
            CTokenKind::Comment,
            None,
        );
    }

    /// `x = x + 1` as `x++`, `ADDR = ADDR | 4` as `ADDR |= 4`: the operator
    /// and right-hand side, where the left-hand side is the destination.
    fn compound(&self, dst: &Place, value: &Expr) -> Option<(BinOp, Expr)> {
        let Expr::Bin(
            op @ (BinOp::Add
            | BinOp::Sub
            | BinOp::And
            | BinOp::Or
            | BinOp::Xor
            | BinOp::Shl
            | BinOp::Shr),
            a,
            b,
        ) = value
        else {
            return None;
        };
        let same = match (dst, &**a) {
            (Place::Var(v), Expr::Var(w)) => v == w,
            (
                Place::Mem { addr, width },
                Expr::Mem {
                    addr: a2,
                    width: w2,
                },
            ) => addr == &**a2 && width == w2,
            _ => false,
        };
        same.then(|| (*op, (**b).clone()))
    }

    /// A value as a byte.
    fn byte(&mut self, e: &Expr) {
        let eight = match e {
            Expr::Const(v) => *v <= 0xFF,
            Expr::Reg(_, Width::W8) | Expr::Flag(_) | Expr::Cast(Width::W8, _) => true,
            Expr::Mem { width, .. } => *width == Width::W8,
            Expr::Call(name, _) => *name == "pull8",
            _ => false,
        };
        if eight {
            self.expr(e);
        } else {
            self.expr(&Expr::Cast(Width::W8, Box::new(e.clone())));
        }
    }
}

/// Print a function in goto form: every block in address order (the entry
/// first), a label where something jumps, and a `goto` for each edge that
/// is not the block below.
pub struct GotoLayout<'f> {
    pub f: &'f Function,
    pub cfg: &'f Cfg,
    pub blocks: &'f [LiftedBlock],
}

impl GotoLayout<'_> {
    fn order(&self) -> Vec<BlockId> {
        let mut real: Vec<BlockId> = (0..self.cfg.blocks.len())
            .filter(|&b| !self.cfg.blocks[b].is_stub() && b != self.cfg.entry)
            .collect();
        real.sort_by_key(|&b| self.cfg.blocks[b].steps.start);
        let mut order = vec![self.cfg.entry];
        order.extend(real);
        order
    }

    pub fn label(&self, e: &Emitter, b: BlockId) -> String {
        let s = &self.f.steps[self.cfg.blocks[b].steps.start].insn;
        e.names.goto_label(s.address)
    }

    pub fn print(&self, e: &mut Emitter, asm: &dyn Fn(usize) -> String) {
        let order = self.order();
        let next_of: BTreeMap<BlockId, Option<BlockId>> = order
            .iter()
            .enumerate()
            .map(|(i, &b)| (b, order.get(i + 1).copied()))
            .collect();
        // Which blocks a goto names.
        let mut targeted: BTreeSet<BlockId> = BTreeSet::new();
        for &b in &order {
            let next = next_of[&b];
            let mut edge = |t: BlockId, fallthrough: bool| {
                if !self.cfg.blocks[t].is_stub() && !(fallthrough && Some(t) == next) {
                    targeted.insert(t);
                }
            };
            match &self.cfg.blocks[b].term {
                Term::Fall(t) | Term::Goto(t) => edge(*t, true),
                Term::Branch { taken, fall } => {
                    edge(*taken, false);
                    edge(*fall, true);
                }
                Term::Switch { cases, .. } => cases.iter().for_each(|c| edge(*c, false)),
                _ => {}
            }
        }
        for (i, &b) in order.iter().enumerate() {
            let block = &self.cfg.blocks[b];
            let lb = &self.blocks[b];
            e.stats.blocks += 1;
            if targeted.contains(&b) {
                if i > 0 {
                    e.w.blank();
                }
                let indent = e.w.indent;
                e.w.indent = 0;
                let l = self.label(e, b);
                e.w.tok(&l, CTokenKind::GotoLabel, None);
                e.w.w(":");
                e.w.end(&[block.steps.start]);
                e.w.indent = indent;
            }
            for line in &lb.lines {
                e.stmt(&line.stmt, &line.steps(), asm);
            }
            let ts: Vec<usize> = lb
                .term_step
                .into_iter()
                .chain(lb.term_merged.iter().copied())
                .collect();
            let next = next_of[&b];
            match &block.term {
                Term::Fall(t) | Term::Goto(t) => self.go(e, *t, next, &ts, b, asm),
                Term::Branch { taken, fall } => {
                    e.kw("if");
                    e.w.w(" (");
                    e.expr(lb.cond.as_ref().unwrap_or(&Expr::Const(1)));
                    e.w.w(")");
                    if self.cfg.blocks[*taken].is_stub() {
                        e.w.w(" {");
                        e.w.end(&ts);
                        e.w.indent += 1;
                        self.stub(e, *taken, &ts, b, asm);
                        e.w.indent -= 1;
                        e.w.w("}");
                        e.w.end(&ts);
                    } else {
                        e.w.w(" ");
                        e.kw("goto");
                        e.w.w(" ");
                        let l = self.label(e, *taken);
                        e.w.tok(&l, CTokenKind::GotoLabel, None);
                        e.w.w(";");
                        e.stats.gotos += 1;
                        e.w.end(&ts);
                    }
                    self.go(e, *fall, next, &ts, b, asm);
                }
                Term::Switch { cases, .. } => {
                    let (index, values) = lb
                        .switch
                        .clone()
                        .unwrap_or((Expr::Const(0), (0..cases.len() as u32).collect()));
                    e.kw("switch");
                    e.w.w(" (");
                    e.expr(&index);
                    e.w.w(") {");
                    e.w.end(&ts);
                    for (c, v) in cases.iter().zip(values) {
                        e.kw("case");
                        e.w.w(" ");
                        e.num(v);
                        e.w.w(":");
                        if self.cfg.blocks[*c].is_stub() {
                            e.w.end(&ts);
                            e.w.indent += 1;
                            self.stub(e, *c, &ts, b, asm);
                            e.w.indent -= 1;
                        } else {
                            e.w.w(" ");
                            e.kw("goto");
                            e.w.w(" ");
                            let l = self.label(e, *c);
                            e.w.tok(&l, CTokenKind::GotoLabel, None);
                            e.w.w(";");
                            e.stats.gotos += 1;
                            e.w.end(&ts);
                        }
                    }
                    e.kw("default");
                    e.w.w(":");
                    e.w.end(&ts);
                    e.w.indent += 1;
                    e.w.comment_line("not an entry in the table", &ts);
                    e.kw("return");
                    e.w.w(";");
                    e.w.end(&ts);
                    e.w.indent -= 1;
                    e.w.w("}");
                    e.w.end(&ts);
                }
                Term::Return | Term::Halt | Term::Tail(_) | Term::Unknown(_) => {
                    self.stub(e, b, &ts, b, asm)
                }
            }
        }
    }

    /// Control to `t` from the end of `from`.
    fn go(
        &self,
        e: &mut Emitter,
        t: BlockId,
        next: Option<BlockId>,
        ts: &[usize],
        from: BlockId,
        asm: &dyn Fn(usize) -> String,
    ) {
        if self.cfg.blocks[t].is_stub() {
            self.stub(e, t, ts, from, asm);
        } else if Some(t) != next {
            e.kw("goto");
            e.w.w(" ");
            let l = self.label(e, t);
            e.w.tok(&l, CTokenKind::GotoLabel, None);
            e.w.w(";");
            e.stats.gotos += 1;
            e.w.end(ts);
        }
    }

    fn stub(
        &self,
        e: &mut Emitter,
        b: BlockId,
        ts: &[usize],
        from: BlockId,
        asm: &dyn Fn(usize) -> String,
    ) {
        exit(self.cfg, self.blocks, e, b, ts, from, asm)
    }
}

/// What a block that leaves the function does: return, tail-call, or say
/// what could not be followed; at the `full` level, with its results.
fn exit(
    cfg: &Cfg,
    blocks: &[LiftedBlock],
    e: &mut Emitter,
    b: BlockId,
    ts: &[usize],
    from: BlockId,
    asm: &dyn Fn(usize) -> String,
) {
    let value = blocks[b].exit.as_ref();
    if let Some(x) = value {
        for s in &x.before {
            e.stmt(s, ts, asm);
        }
    }
    match &cfg.blocks[b].term {
        Term::Tail(a) if !value.is_some_and(|x| x.call_done) => {
            let name = e.names.function(*a);
            e.w.tok(&name, CTokenKind::Function, Some(*a));
            e.w.w("();");
            e.w.end(ts);
        }
        Term::Unknown(why) => {
            e.stats.asm_comments += 1;
            let text = blocks[from].term_step.map(asm).unwrap_or_default();
            e.w.comment_line(&format!("asm: {text}: {why}"), ts);
        }
        _ => {}
    }
    if let Some(x) = value {
        for s in &x.after {
            e.stmt(s, ts, asm);
        }
        for (k, v) in &x.outs {
            e.place(&Place::Out(*k));
            e.w.w(" = ");
            e.expr(v);
            e.w.w(";");
            e.w.end(ts);
        }
    }
    e.kw("return");
    if let Some(v) = value.and_then(|x| x.ret.as_ref()) {
        e.w.w(" ");
        e.expr(v);
    }
    e.w.w(";");
    e.w.end(ts);
}

/// The instruction text for asm comments.
pub fn asm_text(
    rom: &RomImage,
    project: &Project,
    snap: &AnalysisSnapshot,
    f: &Function,
) -> impl Fn(usize) -> String {
    let symbols = Symbols::new(rom, project, &snap.auto_labels);
    let texts: Vec<String> = f
        .steps
        .iter()
        .map(|s| format_instruction(&s.insn, &symbols).text)
        .collect();
    move |i| texts.get(i).cloned().unwrap_or_default()
}

/// Print the structured tree (`structure`).
pub struct TreeLayout<'f> {
    pub f: &'f Function,
    pub cfg: &'f Cfg,
    pub blocks: &'f [LiftedBlock],
    pub gotos: &'f BTreeSet<BlockId>,
}

impl TreeLayout<'_> {
    fn label(&self, e: &Emitter, b: BlockId) -> String {
        let s = &self.f.steps[self.cfg.blocks[b].steps.start].insn;
        e.names.goto_label(s.address)
    }

    fn ts(&self, b: BlockId) -> Vec<usize> {
        let lb = &self.blocks[b];
        lb.term_step
            .into_iter()
            .chain(lb.term_merged.iter().copied())
            .collect()
    }

    /// A block's label and its first `n` statements.
    fn lines(&self, e: &mut Emitter, b: BlockId, n: usize, asm: &dyn Fn(usize) -> String) {
        e.stats.blocks += 1;
        let lines = &self.blocks[b].lines[..n];
        // A label needs a statement after it: comments are not one.
        let empty = lines.iter().all(|l| matches!(l.stmt, Stmt::Note(_)));
        self.put_label(e, b, empty);
        for line in lines {
            e.stmt(&line.stmt, &line.steps(), asm);
        }
    }

    fn put_label(&self, e: &mut Emitter, b: BlockId, empty: bool) {
        if !self.gotos.contains(&b) {
            return;
        }
        let indent = e.w.indent;
        e.w.indent = 0;
        let l = self.label(e, b);
        e.w.tok(&l, CTokenKind::GotoLabel, None);
        e.w.w(if empty { ": ;" } else { ":" });
        e.w.end(&[self.cfg.blocks[b].steps.start]);
        e.w.indent = indent;
    }

    pub fn print(
        &self,
        e: &mut Emitter,
        nodes: &[crate::decompile::structure::Node],
        asm: &dyn Fn(usize) -> String,
    ) {
        for n in nodes {
            self.node(e, n, asm);
        }
    }

    fn body(
        &self,
        e: &mut Emitter,
        nodes: &[crate::decompile::structure::Node],
        asm: &dyn Fn(usize) -> String,
    ) {
        e.w.indent += 1;
        self.print(e, nodes, asm);
        e.w.indent -= 1;
    }

    fn node(
        &self,
        e: &mut Emitter,
        n: &crate::decompile::structure::Node,
        asm: &dyn Fn(usize) -> String,
    ) {
        use crate::decompile::structure::{LoopKind, Node};
        match n {
            Node::Block(b) => self.lines(e, *b, self.blocks[*b].lines.len(), asm),
            Node::Lines(b, n) => self.lines(e, *b, *n, asm),
            Node::If {
                cond,
                then,
                els,
                at,
                steps,
                ..
            } => {
                let mut ts = self.ts(*at);
                ts.extend(steps.iter().copied());
                e.kw("if");
                e.w.w(" (");
                e.expr(cond);
                e.w.w(") {");
                e.w.end(&ts);
                self.body(e, then, asm);
                let mut els = els;
                loop {
                    match els.as_slice() {
                        [] => {
                            e.w.w("}");
                            e.w.end(&ts);
                            break;
                        }
                        // else if, when the else is only another if.
                        [
                            Node::If {
                                cond,
                                then,
                                els: next,
                                at,
                                steps,
                                ..
                            },
                        ] => {
                            let mut ts = self.ts(*at);
                            ts.extend(steps.iter().copied());
                            e.w.w("} ");
                            e.kw("else");
                            e.w.w(" ");
                            e.kw("if");
                            e.w.w(" (");
                            e.expr(cond);
                            e.w.w(") {");
                            e.w.end(&ts);
                            self.body(e, then, asm);
                            els = next;
                        }
                        _ => {
                            e.w.w("} ");
                            e.kw("else");
                            e.w.w(" {");
                            e.w.end(&ts);
                            self.body(e, els, asm);
                            e.w.w("}");
                            e.w.end(&ts);
                            break;
                        }
                    }
                }
            }
            Node::Loop {
                kind,
                body,
                head,
                at,
            } => {
                let ts = self.ts(*at);
                match kind {
                    LoopKind::While(cond) => {
                        self.put_label(e, *head, false);
                        e.kw("while");
                        e.w.w(" (");
                        e.expr(cond);
                        e.w.w(") {");
                        e.w.end(&ts);
                        self.body(e, body, asm);
                        e.w.w("}");
                        e.w.end(&ts);
                    }
                    LoopKind::DoWhile(cond) => {
                        e.kw("do");
                        e.w.w(" {");
                        e.w.end(&[]);
                        self.body(e, body, asm);
                        e.w.w("} ");
                        e.kw("while");
                        e.w.w(" (");
                        e.expr(cond);
                        e.w.w(");");
                        e.w.end(&ts);
                    }
                    LoopKind::Forever => {
                        e.kw("for");
                        e.w.w(" (;;) {");
                        e.w.end(&[]);
                        self.body(e, body, asm);
                        e.w.w("}");
                        e.w.end(&[]);
                    }
                    LoopKind::For(c) => {
                        let mut ts = ts.clone();
                        ts.extend(c.steps.iter().copied());
                        let name = e.var_name(c.var);
                        e.kw("for");
                        e.w.w(" (");
                        if c.declare {
                            e.ty("int");
                            e.w.w(" ");
                        }
                        e.local(&name);
                        e.w.w(" = ");
                        e.num(c.init);
                        e.w.w("; ");
                        e.expr(&c.cond);
                        e.w.w("; ");
                        e.local(&name);
                        match (c.op, c.step) {
                            (BinOp::Add, 1) => e.w.w("++"),
                            (BinOp::Sub, 1) => e.w.w("--"),
                            (op, k) => {
                                e.w.w(" ");
                                e.w.w(op.symbol());
                                e.w.w("= ");
                                e.num(k);
                            }
                        }
                        e.w.w(") {");
                        e.w.end(&ts);
                        self.body(e, body, asm);
                        e.w.w("}");
                        e.w.end(&ts);
                    }
                }
            }
            Node::Switch { index, cases, at } => {
                let ts = self.ts(*at);
                e.kw("switch");
                e.w.w(" (");
                e.expr(index);
                e.w.w(") {");
                e.w.end(&ts);
                for (values, body) in cases {
                    for v in values {
                        e.kw("case");
                        e.w.w(" ");
                        e.num(*v);
                        e.w.w(":");
                        e.w.end(&ts);
                    }
                    self.body(e, body, asm);
                    let ends = matches!(
                        body.last(),
                        Some(Node::Exit(..) | Node::Goto(..) | Node::Continue(_) | Node::Break(_))
                    );
                    if !ends {
                        e.w.indent += 1;
                        e.kw("break");
                        e.w.w(";");
                        e.w.end(&ts);
                        e.w.indent -= 1;
                    }
                }
                e.kw("default");
                e.w.w(":");
                e.w.end(&ts);
                e.w.indent += 1;
                e.w.comment_line("not an entry in the table", &ts);
                e.kw("return");
                e.w.w(";");
                e.w.end(&ts);
                e.w.indent -= 1;
                e.w.w("}");
                e.w.end(&ts);
            }
            Node::Goto(t, from) => {
                e.kw("goto");
                e.w.w(" ");
                let l = self.label(e, *t);
                e.w.tok(&l, CTokenKind::GotoLabel, None);
                e.w.w(";");
                e.stats.gotos += 1;
                e.w.end(&self.ts(*from));
            }
            Node::Break(from) => {
                e.kw("break");
                e.w.w(";");
                e.w.end(&self.ts(*from));
            }
            Node::Continue(from) => {
                e.kw("continue");
                e.w.w(";");
                e.w.end(&self.ts(*from));
            }
            Node::Exit(b, from) => {
                let ts = self.ts(*from);
                exit(self.cfg, self.blocks, e, *b, &ts, *from, asm);
            }
        }
    }
}
