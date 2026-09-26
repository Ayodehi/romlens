//! `registers`: the built-in hardware register table, and what a register's
//! bits mean (docs/20).

use std::fmt::Write as _;

use anyhow::{Result, anyhow};
use romlens_core::explain::{RegisterWrite, describe};
use romlens_core::model::{all_hardware_registers, hardware_register};
use romlens_core::{AddressExpr, parse_address_expr};

/// Which register file: the 65816's, the S-DSP's or the SPC700's I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bank {
    Cpu,
    Dsp,
    Spc,
}

pub fn run(bank: Bank, address: Option<&str>, value: Option<&str>) -> Result<()> {
    if bank != Bank::Cpu {
        return sound(bank, address, value);
    }
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

/// The DSP's registers (`$00–$7F`, or by name) and the SPC700's
/// (`$F0–$FF`), as the 65816's above.
fn sound(bank: Bank, address: Option<&str>, value: Option<&str>) -> Result<()> {
    use romlens_core::explain::sound::{
        describe_dsp, describe_spc_io, dsp_layout, dsp_register_name, dsp_register_named,
    };
    use romlens_core::spc700::{IO_REGISTERS, io_named};
    let value = value.map(super::rec::number).transpose()?;
    if value.is_some_and(|v| v > 0xFF) {
        return Err(anyhow!("a sound register takes one byte"));
    }
    let value = value.map(|v| v as u8);
    let Some(text) = address else {
        if value.is_some() {
            return Err(anyhow!("--value needs a register"));
        }
        match bank {
            Bank::Dsp => {
                for r in 0..0x80u8 {
                    let l = dsp_layout(r);
                    if l.data && l.about.starts_with("Not used") {
                        continue;
                    }
                    println!(
                        "${r:02X}  {:<9} {}",
                        dsp_register_name(r),
                        first_sentence(l.about)
                    );
                }
            }
            _ => {
                for r in &IO_REGISTERS {
                    println!("${:02X}  {:<9} {}", r.address, r.name, r.description);
                }
            }
        }
        return Ok(());
    };
    let w = match bank {
        Bank::Dsp => {
            let reg = match dsp_register_named(text) {
                Some(r) => r,
                None => {
                    let v = super::rec::number(text)?;
                    u8::try_from(v)
                        .ok()
                        .filter(|v| *v < 0x80)
                        .ok_or_else(|| anyhow!("DSP registers are $00-$7F"))?
                }
            };
            describe_dsp(reg, value)
        }
        _ => {
            let a = match io_named(text) {
                Some(r) => r.address,
                None => {
                    let v = super::rec::number(text)?;
                    u16::try_from(v).map_err(|_| anyhow!("the SPC700's I/O is $F0-$FF"))?
                }
            };
            describe_spc_io(a, value).ok_or_else(|| anyhow!("the SPC700's I/O is $F0-$FF"))?
        }
    };
    let p = &w.parts[0];
    println!("${:02X}  {}", p.address, p.name);
    print!("{}", write_text(&w, "  "));
    Ok(())
}

fn first_sentence(s: &str) -> &str {
    s.split_once(". ")
        .map_or(s, |(a, _)| a)
        .trim_end_matches('.')
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
