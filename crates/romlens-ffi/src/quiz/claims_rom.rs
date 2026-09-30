//! The claims about the open ROM and its analysis (docs/28): its mapping,
//! vectors and labels, the instructions and their widths, the values the
//! code writes, the idioms and the DMA it does.

use romlens_core::cpu65816::{Instruction, format_instruction};
use romlens_core::explain::{DmaDest, DmaTransfer, IdiomKind};
use romlens_core::memory::address::{FileOffset, SnesAddress};
use romlens_core::model::symbols::Symbols;
use romlens_core::timing::{LINES_NTSC, MASTER_PER_LINE, VBLANK_START};
use romlens_tutor::quiz::Claim;

use super::World;
use super::claims::{field, register, same};

/// `$00:8123`.
pub fn cpu(a: u32) -> String {
    format!("${:02X}:{:04X}", (a >> 16) & 0xFF, a & 0xFFFF)
}

pub fn offset(w: &World, a: u32) -> Option<FileOffset> {
    w.rom.file_offset_for(SnesAddress::from_u24(a))
}

/// The instruction that starts at `a`.
pub fn insn(w: &World, a: u32) -> Option<Instruction> {
    let off = offset(w, a)?;
    let rec = w.snap.instruction_at(off).filter(|r| r.offset == off.0)?;
    w.snap.decode_at(w.rom, rec)
}

/// How the listing writes an instruction: `STA $2100`, `JSR SUB_008020`.
pub fn text(w: &World, i: &Instruction) -> String {
    let symbols = Symbols::new(w.rom, w.project, &w.snap.auto_labels);
    format_instruction(i, &symbols).text
}

/// What an idiom does, in a few words; `None` for one not worth asking.
pub fn idiom_title(k: IdiomKind) -> Option<&'static str> {
    Some(match k {
        IdiomKind::Wait => "Waits for the PPU or an interrupt",
        IdiomKind::Dma => "Copies bytes with DMA",
        IdiomKind::Hdma => "Sets up HDMA",
        IdiomKind::Multiply => "Multiplies with the hardware",
        IdiomKind::Divide => "Divides with the hardware",
        IdiomKind::ClearMemory => "Fills memory with one value",
        IdiomKind::BlockMove => "Moves a block of memory",
        IdiomKind::ApuHandshake => "Waits for the sound CPU",
        IdiomKind::Decimal => "Adds or subtracts in decimal",
        IdiomKind::ShadowRegister => "Writes a register and keeps a copy in RAM",
        IdiomKind::DataBank => "Sets the data bank",
        IdiomKind::ApuUpload => "Uploads data to the sound CPU",
        IdiomKind::SharedEntry => return None,
    })
}

/// Where a DMA copies to, as a word.
pub fn dest_name(d: DmaDest) -> String {
    match d {
        DmaDest::Vram(_) => "VRAM".into(),
        DmaDest::Cgram(_) => "CGRAM".into(),
        DmaDest::Oam => "OAM".into(),
        DmaDest::Wram(_) => "WRAM".into(),
        DmaDest::Other(b) => format!("$21{b:02X}"),
        DmaDest::Unknown => "somewhere Romlens can't tell".into(),
    }
}

/// Master cycles one vertical blank gives, NTSC.
pub const VBLANK_MASTER: u32 = (LINES_NTSC - VBLANK_START) as u32 * MASTER_PER_LINE;

/// Master cycles DMA takes for `bytes`, at 8 a byte.
pub fn dma_cycles(bytes: u32) -> u32 {
    bytes * 8
}

pub fn transfer_at(w: &World, at: u32) -> Option<DmaTransfer> {
    let off = offset(w, at)?;
    w.explain
        .idioms()
        .iter()
        .flat_map(|i| i.transfers.iter())
        .find(|t| t.at == off)
        .copied()
}

/// The part of the store at `a` that writes `register`, with its value.
fn stored(w: &World, a: u32, register: &str) -> Result<u32, String> {
    let off = offset(w, a).ok_or_else(|| format!("{} is not in the ROM", cpu(a)))?;
    let e = w.explain.write_at(off).ok_or_else(|| {
        format!(
            "the instruction at {} is not a store to the hardware Romlens explains",
            cpu(a)
        )
    })?;
    let p = e
        .write
        .parts
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(register))
        .ok_or_else(|| {
            format!(
                "the store at {} writes {}, not {register}",
                cpu(a),
                e.write
                    .parts
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            )
        })?;
    p.value.ok_or_else(|| {
        format!(
            "Romlens doesn't know the value the store at {} writes",
            cpu(a)
        )
    })
}

fn same_place(w: &World, a: u32, b: u32) -> bool {
    a == b || matches!((offset(w, a), offset(w, b)), (Some(x), Some(y)) if x == y)
}

pub fn check(w: &World, c: &Claim) -> Result<(), String> {
    match c {
        Claim::Mapping { expect } => {
            let m = w.rom.mapping().name();
            if same(m, expect) {
                Ok(())
            } else {
                Err(format!("this game is {m}, not {expect}"))
            }
        }
        Claim::AddressOf { what, expect } => {
            let (kind, name) = what.split_once(':').unwrap_or((what.as_str(), ""));
            let at: Vec<u32> = match kind {
                "vector" => {
                    let h = w.rom.header();
                    let v = match name {
                        "reset" => h.emulation.reset,
                        "nmi" => h.native.nmi,
                        "irq" => h.native.irq,
                        "brk" => h.native.brk,
                        "cop" => h.native.cop,
                        _ => return Err("the vectors are reset, nmi, irq, brk and cop".into()),
                    };
                    vec![SnesAddress::new(0, v).as_u24()]
                }
                "label" => w
                    .project
                    .labels
                    .iter()
                    .map(|(a, l)| (*a, l.name.as_str()))
                    .chain(
                        w.snap
                            .auto_labels
                            .iter()
                            .map(|(a, l)| (*a, l.name.as_str())),
                    )
                    .filter(|(_, n)| n.eq_ignore_ascii_case(name))
                    .map(|(a, _)| a.as_u24())
                    .collect(),
                "idiom" => w
                    .explain
                    .idioms()
                    .iter()
                    .filter(|i| {
                        i.kind.as_str() == name
                            || idiom_title(i.kind).is_some_and(|t| same(t, name))
                    })
                    .filter_map(|i| w.rom.snes_address_for(i.first()).map(|a| a.as_u24()))
                    .collect(),
                _ => return Err("name a vector:, label: or idiom:".into()),
            };
            if at.is_empty() {
                return Err(format!("Romlens finds no {what} in this game"));
            }
            if at.iter().any(|a| same_place(w, *a, *expect)) {
                Ok(())
            } else {
                Err(format!(
                    "{what} is at {}, not {}",
                    at.iter().map(|a| cpu(*a)).collect::<Vec<_>>().join(", "),
                    cpu(*expect)
                ))
            }
        }
        Claim::InstructionAt {
            address,
            mnemonic,
            operand,
        } => {
            let i = insn(w, *address)
                .ok_or_else(|| format!("no instruction starts at {}", cpu(*address)))?;
            let t = text(w, &i);
            let (m, rest) = t.split_once(' ').unwrap_or((t.as_str(), ""));
            if !m.eq_ignore_ascii_case(mnemonic)
                || operand.as_deref().is_some_and(|o| !same(o, rest))
            {
                return Err(format!("the instruction at {} is `{t}`", cpu(*address)));
            }
            Ok(())
        }
        Claim::BytesAt { address, bytes } => {
            if bytes.is_empty() || bytes.len() > 4 {
                return Err("name one to four bytes".into());
            }
            let off = offset(w, *address)
                .ok_or_else(|| format!("{} is not in the ROM", cpu(*address)))?;
            let rom = w.rom.bytes();
            let at = off.0 as usize;
            match rom.get(at..at + bytes.len()) {
                Some(b) if b == bytes.as_slice() => Ok(()),
                Some(b) => Err(format!(
                    "the bytes at {} are {}",
                    cpu(*address),
                    b.iter()
                        .map(|x| format!("{x:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                None => Err("past the end of the ROM".into()),
            }
        }
        Claim::ValueReaching {
            address,
            register,
            value,
        } => {
            let v = stored(w, *address, register)?;
            if v == *value {
                Ok(())
            } else {
                Err(format!(
                    "the store at {} writes ${v:02X} to {register}",
                    cpu(*address)
                ))
            }
        }
        Claim::WriteMeans {
            address,
            register: r,
            field: f,
            expect,
        } => {
            let v = stored(w, *address, r)?;
            let reg = register(r).ok_or_else(|| format!("{r} is not a register Romlens knows"))?;
            let fl = field(&reg, f).ok_or_else(|| format!("{} has no field “{f}”", reg.name))?;
            let m = fl.meaning(v);
            if same(&m, expect) {
                Ok(())
            } else {
                Err(format!(
                    "the store at {} writes ${v:02X}: {} is {m}",
                    cpu(*address),
                    fl.name
                ))
            }
        }
        Claim::Dma { at, field, expect } => {
            let t = transfer_at(w, *at)
                .ok_or_else(|| format!("Romlens finds no DMA started at {}", cpu(*at)))?;
            let bytes = || t.bytes.ok_or("Romlens doesn't know that DMA's size");
            let got =
                match field.as_str() {
                    "channel" => t.channel.to_string(),
                    "destination" => dest_name(t.dest),
                    "source" => t
                        .source
                        .map(|s| cpu(s.as_u24()))
                        .ok_or("Romlens doesn't know that DMA's source")?,
                    "size" => bytes()?.to_string(),
                    "cycles" => dma_cycles(bytes()?).to_string(),
                    "fits" => if dma_cycles(bytes()?) <= VBLANK_MASTER {
                        "yes"
                    } else {
                        "no"
                    }
                    .into(),
                    _ => return Err(
                        "a DMA's fields are channel, destination, source, size, cycles and fits"
                            .into(),
                    ),
                };
            if same(&got, expect) {
                Ok(())
            } else {
                Err(format!("the DMA at {}: its {field} is {got}", cpu(*at)))
            }
        }
        Claim::Width {
            address,
            register,
            bits,
        } => {
            let i = insn(w, *address)
                .ok_or_else(|| format!("no instruction starts at {}", cpu(*address)))?;
            if i.assumptions != 0 {
                return Err(format!(
                    "Romlens isn't sure of the widths at {}",
                    cpu(*address)
                ));
            }
            let eight = match register.to_ascii_lowercase().as_str() {
                "a" | "m" => i.flags_before.m || i.flags_before.e,
                "x" | "y" => i.flags_before.x || i.flags_before.e,
                _ => return Err("the register is a or x".into()),
            };
            let got = if eight { 8 } else { 16 };
            if got == *bits {
                Ok(())
            } else {
                Err(format!("at {} it is {got} bits", cpu(*address)))
            }
        }
        Claim::Length { address, expect } => {
            let i = insn(w, *address)
                .ok_or_else(|| format!("no instruction starts at {}", cpu(*address)))?;
            if i.len == *expect {
                Ok(())
            } else {
                Err(format!(
                    "`{}` at {} is {} bytes",
                    text(w, &i),
                    cpu(*address),
                    i.len
                ))
            }
        }
        Claim::IdiomAt { address, expect } => {
            let off = offset(w, *address)
                .ok_or_else(|| format!("{} is not in the ROM", cpu(*address)))?;
            let kinds: Vec<&str> = w
                .explain
                .idioms_at(off)
                .iter()
                .filter_map(|i| idiom_title(i.kind))
                .collect();
            if kinds.iter().any(|k| same(k, expect)) {
                Ok(())
            } else if kinds.is_empty() {
                Err(format!("Romlens finds no idiom at {}", cpu(*address)))
            } else {
                Err(format!(
                    "the code at {}: {}",
                    cpu(*address),
                    kinds.join("; ")
                ))
            }
        }
        _ => Err("Romlens can't check that claim".into()),
    }
}
