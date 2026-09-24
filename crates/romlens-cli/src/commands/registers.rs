//! `registers`: the built-in hardware register table, and what a register's
//! bits mean (docs/20).

use std::fmt::Write as _;

use anyhow::{Result, anyhow};
use romlens_core::explain::{RegisterWrite, describe};
use romlens_core::model::{all_hardware_registers, hardware_register};
use romlens_core::{AddressExpr, parse_address_expr};

pub fn run(address: Option<&str>, value: Option<&str>) -> Result<()> {
    match address {
        Some(text) => {
            let a = match parse_address_expr(text)? {
                AddressExpr::Snes(a) => a.offset(),
                AddressExpr::File(_) => return Err(anyhow!("give a CPU address such as $420D")),
            };
            let r = hardware_register(a).ok_or_else(|| anyhow!("no register at ${a:04X}"))?;
            println!(
                "${:04X}  {:<9} {:<3} {}",
                r.address,
                r.name,
                r.access.as_str(),
                r.description
            );
            let value = value.map(super::rec::number).transpose()?;
            if value.is_some_and(|v| v > 0xFFFF) {
                return Err(anyhow!("a register takes one or two bytes"));
            }
            let width = if value.is_some_and(|v| v > 0xFF) {
                2
            } else {
                1
            };
            if let Some(w) = describe(a, value, width) {
                print!("{}", write_text(&w, "  "));
            }
        }
        None => {
            if value.is_some() {
                return Err(anyhow!("--value needs a register address"));
            }
            for r in all_hardware_registers() {
                println!(
                    "${:04X}  {:<9} {:<3} {}",
                    r.address,
                    r.name,
                    r.access.as_str(),
                    r.description
                );
            }
        }
    }
    Ok(())
}

/// A write, register by register: what each is for, its fields as a table,
/// and the short form. Each line starts with `indent`.
pub fn write_text(w: &RegisterWrite, indent: &str) -> String {
    let mut out = String::new();
    for p in &w.parts {
        out.push('\n');
        if w.parts.len() > 1 {
            let _ = writeln!(out, "{indent}${:04X} {}", p.address, p.name);
        }
        for line in wrap(p.about, 72) {
            let _ = writeln!(out, "{indent}{line}");
        }
        if p.twice {
            let _ = writeln!(
                out,
                "{indent}(Written twice in a row: low byte, then high.)"
            );
        }
        if !p.fields.is_empty() {
            out.push('\n');
            let name_w = p
                .fields
                .iter()
                .map(|f| f.name.chars().count())
                .max()
                .unwrap_or(0);
            let bits_w = p
                .fields
                .iter()
                .map(|f| f.bits.chars().count())
                .max()
                .unwrap_or(0)
                .max(4);
            let _ = writeln!(
                out,
                "{indent}{:<bits_w$}  {:<name_w$}{}",
                "Bits",
                "Field",
                if p.value.is_some() {
                    "  Value  Meaning"
                } else {
                    ""
                }
            );
            for f in &p.fields {
                let pad_bits = bits_w - f.bits.chars().count();
                let pad_name = name_w - f.name.chars().count();
                let mut line = format!(
                    "{indent}{}{}  {}{}",
                    f.bits,
                    " ".repeat(pad_bits),
                    f.name,
                    " ".repeat(pad_name)
                );
                if let (Some(raw), Some(m)) = (f.raw, &f.meaning) {
                    // Wide fields read better in hex, as the code writes them.
                    let raw = if raw > 9 && f.bits.contains('–') && wide(&f.bits) {
                        format!("${raw:X}")
                    } else {
                        format!("{raw}")
                    };
                    let _ = write!(line, "  {raw:<5}  {m}");
                }
                let _ = writeln!(out, "{}", line.trim_end());
            }
        }
    }
    if w.value.is_some() {
        out.push('\n');
        for p in &w.parts {
            let _ = writeln!(out, "{indent}{}", p.short());
        }
    }
    out
}

/// Whether a `lo–hi` bit range is five bits or more.
fn wide(bits: &str) -> bool {
    let mut it = bits.split('–').filter_map(|b| b.parse::<u32>().ok());
    match (it.next(), it.next()) {
        (Some(lo), Some(hi)) => hi - lo >= 4,
        _ => false,
    }
}

/// `text` in lines of at most `width` characters, broken at spaces.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
