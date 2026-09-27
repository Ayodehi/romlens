//! The tutor's `reference` pages (docs/24), made from Romlens's own tables
//! so they say what the listing and the explanations say: the 65816 and
//! SPC700 opcode tables, the register layouts, and the idioms' "why".

use romlens_core::cpu65816::opcodes::OPCODES as CPU;
use romlens_core::explain::{self, sound};
use romlens_core::model::hardware::{Access, all_hardware_registers};
use romlens_core::spc700::opcodes::{Arg, OPCODES as SPC};

pub const TOPICS: &[&str] = &[
    "65816_instruction",
    "65816_opcodes",
    "addressing_modes",
    "register",
    "registers",
    "dsp_register",
    "dsp_registers",
    "spc700_io",
    "spc700_instruction",
    "idioms",
];

/// A page, or why there is none.
pub fn page(topic: &str, detail: Option<&str>) -> Result<String, String> {
    let d = detail.map(str::trim).filter(|d| !d.is_empty());
    match topic {
        "65816_instruction" => cpu_instruction(d.ok_or("name the mnemonic, e.g. LDA")?),
        "65816_opcodes" => Ok(cpu_table()),
        "addressing_modes" => Ok(modes()),
        "register" => register(d.ok_or("give the address, e.g. $2105, or the name, e.g. BGMODE")?),
        "registers" => Ok(registers(d)),
        "dsp_register" => dsp_register(d.ok_or("give the DSP register, e.g. $5D or DIR")?),
        "dsp_registers" => Ok((0..0x80u8)
            .filter(|r| !sound::dsp_register_name(*r).is_empty())
            .map(|r| {
                format!(
                    "${r:02X} {}: {}",
                    sound::dsp_register_name(r),
                    sound::dsp_layout(r).about
                )
            })
            .collect::<Vec<_>>()
            .join("\n")),
        "spc700_io" => Ok((0xF0..=0xFFu16)
            .filter_map(|a| sound::describe_spc_io(a, None))
            .map(write_text)
            .collect::<Vec<_>>()
            .join("\n\n")),
        "spc700_instruction" => spc_instruction(d.ok_or("name the mnemonic, e.g. MOV")?),
        "idioms" => Ok(explain::idioms::WHYS
            .iter()
            .chain(explain::spc::WHYS)
            .map(|(t, w)| format!("## {t}\n{w}"))
            .collect::<Vec<_>>()
            .join("\n\n")),
        other => Err(format!(
            "no page named {other}; the topics are {}",
            TOPICS.join(", ")
        )),
    }
}

fn cpu_instruction(m: &str) -> Result<String, String> {
    let want = m.to_ascii_uppercase();
    let rows: Vec<String> = CPU
        .iter()
        .enumerate()
        .filter(|(_, o)| o.mnemonic.to_string() == want)
        .map(|(op, o)| format!("${op:02X}  {}", o.mode.describe()))
        .collect();
    let first = CPU
        .iter()
        .find(|o| o.mnemonic.to_string() == want)
        .ok_or_else(|| format!("{m} is not a 65816 instruction"))?;
    Ok(format!(
        "{want}: {}\nOpcodes and addressing modes:\n{}",
        first.mnemonic.describe(),
        rows.join("\n")
    ))
}

fn cpu_table() -> String {
    CPU.iter()
        .enumerate()
        .map(|(op, o)| format!("${op:02X} {} {}", o.mnemonic, o.mode.describe()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn modes() -> String {
    let mut seen = Vec::new();
    for o in CPU.iter() {
        let d = o.mode.describe();
        if !seen.contains(&d) {
            seen.push(d);
        }
    }
    let mut s = String::from(
        "The 65816's addressing modes, as Romlens names them. dp is added to D and read in bank 0; abs is read in DBR for data and PBR for jumps; long names the bank; immediate operands are one byte or two depending on M (A) or X (X, Y).\n",
    );
    for d in seen {
        s.push_str("- ");
        s.push_str(d);
        s.push('\n');
    }
    s
}

fn write_text(w: explain::RegisterWrite) -> String {
    let mut s = String::new();
    for p in &w.parts {
        s.push_str(&format!("${:04X} {}: {}", p.address, p.name, p.about));
        if p.twice {
            s.push_str(" (written twice, low byte then high)");
        }
        for f in &p.fields {
            s.push_str(&format!("\n  {}: {}", super::tools::bits(&f.bits), f.name));
        }
        s.push('\n');
    }
    s.trim_end().to_owned()
}

fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('$').trim_start_matches("0x");
    let t = t.rsplit(':').next().unwrap_or(t);
    u32::from_str_radix(t, 16).ok()
}

fn register(d: &str) -> Result<String, String> {
    let regs = all_hardware_registers();
    let address = match parse_hex(d) {
        Some(a) => (a & 0xFFFF) as u16,
        None => regs
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(d))
            .map(|r| r.address)
            .ok_or_else(|| format!("no register named {d}"))?,
    };
    let base = regs.iter().find(|r| r.address == address);
    let mut s = match base {
        Some(r) => format!(
            "${:04X} {} ({}): {}\n",
            r.address,
            r.name,
            access(r.access),
            r.description
        ),
        None => return Err(format!("${address:04X} is not a hardware register")),
    };
    if let Some(w) = explain::describe(address, None, 1) {
        s.push_str(&write_text(w));
    }
    Ok(s.trim_end().to_owned())
}

fn access(a: Access) -> &'static str {
    match a {
        Access::Read => "read",
        Access::Write => "write",
        Access::ReadWrite => "read and write",
    }
}

fn registers(group: Option<&str>) -> String {
    let (lo, hi) = match group.map(|g| g.to_ascii_lowercase()) {
        Some(g) if g == "ppu" => (0x2100, 0x213F),
        Some(g) if g == "apu" => (0x2140, 0x217F),
        Some(g) if g == "dma" => (0x4300, 0x437F),
        Some(g) if g == "cpu" => (0x4000, 0x42FF),
        _ => (0, 0xFFFF),
    };
    all_hardware_registers()
        .into_iter()
        .filter(|r| (lo..=hi).contains(&r.address))
        .map(|r| {
            format!(
                "${:04X} {} ({}): {}",
                r.address,
                r.name,
                access(r.access),
                r.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn dsp_register(d: &str) -> Result<String, String> {
    let reg = match sound::dsp_register_named(d) {
        Some(r) => r,
        None => parse_hex(d)
            .filter(|r| *r < 0x80)
            .ok_or_else(|| format!("no DSP register {d}"))? as u8,
    };
    Ok(write_text(sound::describe_dsp(reg, None)))
}

fn arg(a: Arg) -> String {
    match a {
        Arg::None => String::new(),
        Arg::A => "A".into(),
        Arg::X => "X".into(),
        Arg::Y => "Y".into(),
        Arg::Ya => "YA".into(),
        Arg::Sp => "SP".into(),
        Arg::Psw => "PSW".into(),
        Arg::C => "C".into(),
        Arg::Imm => "#imm".into(),
        Arg::Dp => "dp".into(),
        Arg::DpX => "dp+X".into(),
        Arg::DpY => "dp+Y".into(),
        Arg::Abs => "!abs".into(),
        Arg::AbsX => "!abs+X".into(),
        Arg::AbsY => "!abs+Y".into(),
        Arg::IndX => "(X)".into(),
        Arg::IndXInc => "(X)+".into(),
        Arg::IndY => "(Y)".into(),
        Arg::DpXInd => "[dp+X]".into(),
        Arg::DpIndY => "[dp]+Y".into(),
        Arg::AbsXInd => "[!abs+X]".into(),
        Arg::Rel => "rel".into(),
        Arg::DpBit(b) => format!("dp.{b}"),
        Arg::MemBit => "mem.bit".into(),
        Arg::NotMemBit => "/mem.bit".into(),
        Arg::Upage => "upage".into(),
        Arg::Table(n) => format!("{n}"),
    }
}

fn spc_instruction(m: &str) -> Result<String, String> {
    let want = m.to_ascii_uppercase();
    let rows: Vec<String> = SPC
        .iter()
        .enumerate()
        .filter(|(_, o)| o.mnemonic.to_string().to_ascii_uppercase() == want)
        .map(|(op, o)| {
            let args: Vec<String> = o
                .args
                .iter()
                .map(|a| arg(*a))
                .filter(|a| !a.is_empty())
                .collect();
            format!(
                "${op:02X}  {want} {}  ({} bytes, {} cycles)",
                args.join(","),
                o.len,
                o.cycles
            )
        })
        .collect();
    if rows.is_empty() {
        return Err(format!("{m} is not an SPC700 instruction"));
    }
    Ok(format!(
        "{want} on the SPC700 (Sony syntax, destination first; a branch taken adds 2 cycles):\n{}",
        rows.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_come_from_the_tables() {
        let lda = page("65816_instruction", Some("lda")).unwrap();
        assert!(lda.contains("$A9"), "{lda}");
        let bgmode = page("register", Some("BGMODE")).unwrap();
        assert!(bgmode.starts_with("$2105 BGMODE"), "{bgmode}");
        assert_eq!(page("register", Some("$2105")).unwrap(), bgmode);
        let dir = page("dsp_register", Some("DIR")).unwrap();
        assert!(dir.contains("$005D") || dir.contains("DIR"), "{dir}");
        let mov = page("spc700_instruction", Some("MOV")).unwrap();
        assert!(mov.contains("$E8  MOV A,#imm"), "{mov}");
        assert!(page("idioms", None).unwrap().contains("## DMA"));
        assert!(page("65816_instruction", None).is_err());
        assert!(
            page("nonsense", None)
                .unwrap_err()
                .contains("65816_instruction")
        );
    }
}
