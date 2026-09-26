//! The other way: bytes for an instruction, and a small two-pass assembler
//! over the formatter's own syntax, so the tests and the sound fixture are
//! written as SPC700 code rather than as bytes.
//!
//! The assembler reads what [`format_instruction`](super::format) prints,
//! plus labels (`loop:`), equates (`KON = $4C`), `.org`, `.db` and `.dw`,
//! and `;` comments. Numbers are `$hex`, `%binary` or decimal; an operand
//! may be a label, or an I/O register's name (`MOV DSPADDR,#$4C`).

use std::collections::BTreeMap;
use std::fmt;

use crate::spc700::names::io_named;
use crate::spc700::opcodes::{Arg, Mnemonic, OPCODES, OpcodeInfo};

/// Bytes for opcode `opcode` with its operands' values, at `address` (for
/// a branch's offset). Values: an immediate or direct-page byte, an
/// absolute address, a branch's target, a bit's address (the bit is in
/// the opcode or, for `MemBit`, in `bit`), `PCALL`'s target.
pub fn encode(opcode: u8, values: [u32; 2], bit: u8, address: u16) -> Result<Vec<u8>, String> {
    let info = &OPCODES[opcode as usize];
    let next = address.wrapping_add(info.len as u16);
    let mut parts: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for (i, arg) in info.args.iter().enumerate() {
        let v = values[i];
        parts[i] = match *arg {
            Arg::Imm
            | Arg::Dp
            | Arg::DpX
            | Arg::DpY
            | Arg::DpXInd
            | Arg::DpIndY
            | Arg::DpBit(_) => {
                if v > 0xFF {
                    return Err(format!("${v:X} does not fit in a byte"));
                }
                vec![v as u8]
            }
            Arg::Upage => {
                if !(v <= 0xFF || (0xFF00..=0xFFFF).contains(&v)) {
                    return Err(format!("PCALL reaches $FF00-$FFFF, not ${v:X}"));
                }
                vec![v as u8]
            }
            Arg::Abs | Arg::AbsX | Arg::AbsY | Arg::AbsXInd => {
                if v > 0xFFFF {
                    return Err(format!("${v:X} is past $FFFF"));
                }
                (v as u16).to_le_bytes().to_vec()
            }
            Arg::MemBit | Arg::NotMemBit => {
                if v > 0x1FFF || bit > 7 {
                    return Err(format!(
                        "${v:X}.{bit}: a bit address is under $2000 and a bit 0-7"
                    ));
                }
                ((v as u16) | (bit as u16) << 13).to_le_bytes().to_vec()
            }
            Arg::Rel => {
                let off = (v as i32) - (next as i32);
                let off = if off > 0x7FFF {
                    off - 0x10000
                } else if off < -0x8000 {
                    off + 0x10000
                } else {
                    off
                };
                if !(-128..=127).contains(&off) {
                    return Err(format!("${v:04X} is {off} bytes away, too far to branch"));
                }
                vec![off as i8 as u8]
            }
            _ => Vec::new(),
        };
    }
    let mut out = vec![opcode];
    if info.source_first() {
        out.extend(&parts[1]);
        out.extend(&parts[0]);
    } else {
        out.extend(&parts[0]);
        out.extend(&parts[1]);
    }
    Ok(out)
}

/// An assembled program: its bytes where they go, and its labels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    /// Runs of bytes, each at its address, in the order written.
    pub chunks: Vec<(u16, Vec<u8>)>,
    pub labels: BTreeMap<String, u16>,
}

impl Program {
    /// The program laid into a 64 KB image, zero elsewhere.
    pub fn image(&self) -> Vec<u8> {
        let mut image = vec![0u8; 0x10000];
        for (at, bytes) in &self.chunks {
            for (i, b) in bytes.iter().enumerate() {
                image[(*at as usize + i) & 0xFFFF] = *b;
            }
        }
        image
    }

    pub fn label(&self, name: &str) -> Option<u16> {
        self.labels.get(name).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for AsmError {}

/// A number or a name, resolved in the second pass.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    Num(u32),
    Name(String),
}

/// An operand as written, before the opcode is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Form {
    Reg(Arg),
    Imm(Expr),
    /// A bare address: direct page, branch target, `PCALL`, `TCALL`, or a
    /// jump's absolute target.
    Plain(Expr),
    PlainX(Expr),
    PlainY(Expr),
    Abs(Expr),
    AbsX(Expr),
    AbsY(Expr),
    DpXInd(Expr),
    DpIndY(Expr),
    AbsXInd(Expr),
    Bit(Expr, u8),
    NotBit(Expr, u8),
}

impl Form {
    fn expr(&self) -> Option<&Expr> {
        use Form::*;
        match self {
            Reg(_) => None,
            Imm(e)
            | Plain(e)
            | PlainX(e)
            | PlainY(e)
            | Abs(e)
            | AbsX(e)
            | AbsY(e)
            | DpXInd(e)
            | DpIndY(e)
            | AbsXInd(e)
            | Bit(e, _)
            | NotBit(e, _) => Some(e),
        }
    }

    fn bit(&self) -> u8 {
        match self {
            Form::Bit(_, b) | Form::NotBit(_, b) => *b,
            _ => 0,
        }
    }

    /// Whether this written operand can be the opcode's `arg`.
    fn fits(&self, arg: Arg, mnemonic: Mnemonic) -> bool {
        use Form::*;
        let jump = matches!(mnemonic, Mnemonic::Jmp | Mnemonic::Call);
        match (self, arg) {
            (Reg(r), a) => *r == a,
            (Imm(_), Arg::Imm) => true,
            (Plain(_), Arg::Dp | Arg::Rel | Arg::Upage) => true,
            (Plain(_), Arg::Abs) => jump,
            (Plain(Expr::Num(n)), Arg::Table(t)) => *n == t as u32,
            (PlainX(_), Arg::DpX) => true,
            (PlainY(_), Arg::DpY) => true,
            (Abs(_), Arg::Abs) => true,
            (AbsX(_), Arg::AbsX) => true,
            (AbsY(_), Arg::AbsY) => true,
            (DpXInd(_), Arg::DpXInd) => true,
            (DpIndY(_), Arg::DpIndY) => true,
            (AbsXInd(_), Arg::AbsXInd) => true,
            (Bit(_, b), Arg::DpBit(n)) => *b == n,
            (Bit(_, _), Arg::MemBit) => true,
            (NotBit(_, _), Arg::NotMemBit) => true,
            _ => false,
        }
    }
}

fn parse_number(s: &str) -> Option<u32> {
    if let Some(h) = s.strip_prefix('$') {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(b) = s.strip_prefix('%') {
        u32::from_str_radix(b, 2).ok()
    } else if s.chars().next()?.is_ascii_digit() {
        s.parse().ok()
    } else {
        None
    }
}

fn parse_expr(s: &str) -> Result<Expr, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("an operand is missing".into());
    }
    if let Some(n) = parse_number(s) {
        return Ok(Expr::Num(n));
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '@')
        && !s.chars().next().unwrap().is_ascii_digit()
    {
        return Ok(Expr::Name(s.to_owned()));
    }
    Err(format!("`{s}` is not a number or a name"))
}

/// `x.n` with a bit number at the end.
fn split_bit(s: &str) -> Option<(&str, u8)> {
    let (a, b) = s.rsplit_once('.')?;
    let bit: u8 = b.parse().ok()?;
    (bit < 8 && !a.is_empty()).then_some((a, bit))
}

fn parse_form(s: &str) -> Result<Form, String> {
    let s = s.trim();
    let upper = s.to_ascii_uppercase();
    let reg = match upper.as_str() {
        "A" => Some(Arg::A),
        "X" => Some(Arg::X),
        "Y" => Some(Arg::Y),
        "YA" => Some(Arg::Ya),
        "SP" => Some(Arg::Sp),
        "PSW" => Some(Arg::Psw),
        "C" => Some(Arg::C),
        "(X)" => Some(Arg::IndX),
        "(X)+" => Some(Arg::IndXInc),
        "(Y)" => Some(Arg::IndY),
        _ => None,
    };
    if let Some(r) = reg {
        return Ok(Form::Reg(r));
    }
    if let Some(rest) = s.strip_prefix('#') {
        return Ok(Form::Imm(parse_expr(rest)?));
    }
    if let Some(inner) = s.strip_prefix('[') {
        if let Some(e) = upper.strip_suffix("]+Y") {
            return Ok(Form::DpIndY(parse_expr(&inner[..e.len() - 1])?));
        }
        if upper.ends_with("+X]") {
            let e = &inner[..inner.len() - 3];
            return Ok(match e.strip_prefix('!') {
                Some(a) => Form::AbsXInd(parse_expr(a)?),
                None => Form::DpXInd(parse_expr(e)?),
            });
        }
        return Err(format!("`{s}`: expected [d+X], [d]+Y or [!a+X]"));
    }
    if let Some(rest) = s.strip_prefix('/') {
        let (a, bit) = split_bit(rest).ok_or_else(|| format!("`{s}`: expected /address.bit"))?;
        return Ok(Form::NotBit(parse_expr(a)?, bit));
    }
    let (abs, body) = match s.strip_prefix('!') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let upper_body = body.to_ascii_uppercase();
    if let Some(e) = upper_body.strip_suffix("+X") {
        let e = parse_expr(&body[..e.len()])?;
        return Ok(if abs { Form::AbsX(e) } else { Form::PlainX(e) });
    }
    if let Some(e) = upper_body.strip_suffix("+Y") {
        let e = parse_expr(&body[..e.len()])?;
        return Ok(if abs { Form::AbsY(e) } else { Form::PlainY(e) });
    }
    if !abs && let Some((a, bit)) = split_bit(body) {
        return Ok(Form::Bit(parse_expr(a)?, bit));
    }
    let e = parse_expr(body)?;
    Ok(if abs { Form::Abs(e) } else { Form::Plain(e) })
}

/// Split operands at top-level commas.
fn split_operands(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !s[start..].trim().is_empty() {
        out.push(&s[start..]);
    }
    out
}

/// One line's work, kept from the first pass.
enum Item {
    Insn { opcode: u8, forms: Vec<Form> },
    Bytes(Vec<Expr>),
    Words(Vec<Expr>),
}

fn choose(mnemonic: Mnemonic, forms: &[Form]) -> Result<u8, String> {
    let fits = |o: &OpcodeInfo| {
        let n = o.args.iter().filter(|a| **a != Arg::None).count();
        o.mnemonic == mnemonic
            && n == forms.len()
            && forms
                .iter()
                .zip(o.args.iter())
                .all(|(f, a)| f.fits(*a, mnemonic))
    };
    OPCODES
        .iter()
        .position(fits)
        .map(|i| i as u8)
        .ok_or_else(|| format!("{mnemonic} has no form like this"))
}

/// Assemble SPC700 source.
pub fn assemble(source: &str) -> Result<Program, AsmError> {
    let mut labels: BTreeMap<String, u16> = BTreeMap::new();
    let mut equates: BTreeMap<String, u32> = BTreeMap::new();
    let mut items: Vec<(usize, u16, Item)> = Vec::new();
    let mut pc: u16 = 0;
    let err = |line: usize, message: String| AsmError { line, message };

    for (n, raw) in source.lines().enumerate() {
        let line = n + 1;
        let mut text = raw.split(';').next().unwrap_or("").trim();
        while let Some((label, rest)) = text.split_once(':') {
            let label = label.trim();
            if label.is_empty() || label.contains(char::is_whitespace) {
                break;
            }
            if labels.insert(label.to_owned(), pc).is_some() {
                return Err(err(line, format!("`{label}` is defined twice")));
            }
            text = rest.trim();
        }
        if text.is_empty() {
            continue;
        }
        if let Some((name, value)) = text.split_once('=') {
            let v = match parse_expr(value).map_err(|m| err(line, m))? {
                Expr::Num(v) => v,
                Expr::Name(other) => *equates
                    .get(&other)
                    .ok_or_else(|| err(line, format!("`{other}` is not defined yet")))?,
            };
            equates.insert(name.trim().to_owned(), v);
            continue;
        }
        let (word, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let rest = rest.trim();
        match word.to_ascii_lowercase().as_str() {
            ".org" => {
                let v = parse_number(rest)
                    .ok_or_else(|| err(line, format!("`.org {rest}` needs a number")))?;
                pc = v as u16;
                continue;
            }
            ".db" | ".dw" => {
                let exprs = split_operands(rest)
                    .into_iter()
                    .map(parse_expr)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|m| err(line, m))?;
                let words = word.eq_ignore_ascii_case(".dw");
                let len = exprs.len() as u16 * if words { 2 } else { 1 };
                items.push((
                    line,
                    pc,
                    if words {
                        Item::Words(exprs)
                    } else {
                        Item::Bytes(exprs)
                    },
                ));
                pc = pc.wrapping_add(len);
                continue;
            }
            _ => {}
        }
        let mnemonic = Mnemonic::parse(word)
            .ok_or_else(|| err(line, format!("`{word}` is not an SPC700 instruction")))?;
        let forms = split_operands(rest)
            .into_iter()
            .map(parse_form)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|m| err(line, m))?;
        let opcode = choose(mnemonic, &forms).map_err(|m| err(line, m))?;
        items.push((line, pc, Item::Insn { opcode, forms }));
        pc = pc.wrapping_add(OPCODES[opcode as usize].len as u16);
    }

    let resolve = |e: &Expr| -> Result<u32, String> {
        match e {
            Expr::Num(v) => Ok(*v),
            Expr::Name(name) => labels
                .get(name)
                .map(|v| *v as u32)
                .or_else(|| equates.get(name).copied())
                .or_else(|| io_named(name).map(|r| r.address as u32))
                .ok_or_else(|| format!("`{name}` is not defined")),
        }
    };

    let mut chunks: Vec<(u16, Vec<u8>)> = Vec::new();
    for (line, at, item) in &items {
        let bytes = match item {
            Item::Insn { opcode, forms } => {
                let mut values = [0u32; 2];
                let mut bit = 0;
                for (i, f) in forms.iter().enumerate() {
                    if let Some(e) = f.expr() {
                        values[i] = resolve(e).map_err(|m| err(*line, m))?;
                    }
                    if matches!(f, Form::Bit(..) | Form::NotBit(..)) {
                        bit = f.bit();
                    }
                }
                encode(*opcode, values, bit, *at).map_err(|m| err(*line, m))?
            }
            Item::Bytes(exprs) => exprs
                .iter()
                .map(|e| resolve(e).map(|v| v as u8))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|m| err(*line, m))?,
            Item::Words(exprs) => {
                let mut out = Vec::new();
                for e in exprs {
                    let v = resolve(e).map_err(|m| err(*line, m))?;
                    out.extend((v as u16).to_le_bytes());
                }
                out
            }
        };
        match chunks.last_mut() {
            Some((start, run)) if start.wrapping_add(run.len() as u16) == *at => run.extend(bytes),
            _ => chunks.push((*at, bytes)),
        }
    }
    Ok(Program { chunks, labels })
}
