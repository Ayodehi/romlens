//! `inspect`: everything known about one address.

use std::path::Path;

use anyhow::Result;
use romlens_core::cpu65816::{NoSymbols, assumption_names, format_instruction};
use romlens_core::model::{CommentKind, Evidence, Symbols};
use romlens_core::{FileOffset, MemoryClass, header_spans, interpret};

use crate::commands::session;

pub fn run(rom: &Path, expr: &str, project: Option<&Path>) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let r = s.rom.resolve_any(expr)?;
    println!("Address:       {}", r.snes_address);
    match r.file_offset {
        Some(off) => println!("File offset:   {}", off),
        None => println!("File offset:   (not in ROM)"),
    }
    println!(
        "Memory:        {}",
        match r.memory_class {
            MemoryClass::Rom => "ROM",
            MemoryClass::Wram => "WRAM",
            MemoryClass::LowRam => "low RAM (WRAM mirror)",
            MemoryClass::Hardware => "hardware registers",
            MemoryClass::Sram => "SRAM",
            MemoryClass::OpenBus => "open bus",
        }
    );
    if let Some(reg) = r.register {
        println!(
            "Register:      {} ({}) {}",
            reg.name,
            reg.access.as_str(),
            reg.description
        );
    }
    if !r.mirrors.is_empty() {
        let m: Vec<String> = r.mirrors.iter().map(ToString::to_string).collect();
        println!("Mirrors:       {}", m.join(", "));
    }
    let symbols = Symbols::new(&s.rom, &s.project, &s.snap.auto_labels);
    if let Some(l) = symbols.label_at(r.snes_address) {
        println!("Label:         {} ({:?})", l.name, l.source);
    }
    for kind in [CommentKind::Line, CommentKind::Block] {
        if let Some(c) = s.project.comment_at(r.snes_address, kind) {
            println!(
                "Comment ({}): {}",
                kind.as_str(),
                c.text.replace('\n', " | ")
            );
        }
    }
    let to = s.snap.xrefs_to(r.snes_address);
    if !to.is_empty() {
        println!("Referenced by: {}", to.len());
        for x in to {
            println!(
                "  {}  {}{}",
                x.from,
                x.kind.name(),
                if x.certain { "" } else { " (uncertain)" }
            );
        }
    }
    let Some(off) = r.file_offset else {
        return Ok(());
    };
    let spans = header_spans(&s.rom);
    if let Some(b) = interpret(&s.rom, &spans, off) {
        println!("Bytes:         u8 {} (${:02X}), i8 {}", b.u8, b.u8, b.i8);
        if let Some(v) = b.u16_le {
            println!(
                "               u16 LE {v} (${v:04X}), i16 {}",
                b.i16_le.unwrap_or(0)
            );
        }
        if let Some(v) = b.u24_le {
            println!("               u24 LE {v} (${v:06X})");
        }
        if let Some(a) = b.u24_as_snes_address {
            println!("               as SNES address {a}");
        }
        if let Some(c) = b.ascii {
            println!("               ASCII {c:?}");
        }
        if let Some(name) = b.span_name {
            println!(
                "Header field:  {} = {}",
                name,
                b.span_value.unwrap_or_default()
            );
        }
    }
    if let Some(region) = s.snap.region_at(off) {
        let evidence: Vec<String> = region
            .evidence
            .iter()
            .map(|e| match e {
                Evidence::VectorReach { depth } => {
                    format!("reached from a vector through {depth} calls")
                }
                Evidence::Heuristic { name, score } => format!("{name} ({score:.1})"),
                Evidence::User => "marked by the user".to_owned(),
                Evidence::Imported(s) => format!("imported from {s}"),
                Evidence::Trace { file, hits } => format!("trace {file} ({hits} hits)"),
            })
            .collect();
        println!(
            "Region:        {} {}..{} ({} bytes), confidence {:.0}%{}",
            region.kind.name(),
            region.start,
            FileOffset(region.end()),
            region.len,
            region.confidence * 100.0,
            if evidence.is_empty() {
                String::new()
            } else {
                format!(": {}", evidence.join("; "))
            }
        );
    }
    if let Some(rec) = s.snap.instruction_at(off)
        && let Some(insn) = s.snap.decode_at(&s.rom, rec)
    {
        let f = format_instruction(&insn, &symbols);
        println!(
            "Instruction:   {} at {} ({} bytes: {})",
            f.text,
            insn.address,
            insn.len,
            romlens_core::cpu65816::format_bytes(insn.bytes())
        );
        println!(
            "               {} ({})",
            insn.mnemonic.describe(),
            insn.mode.describe()
        );
        println!("Flags before:  {}", insn.flags_before);
        println!("Flags after:   {}", insn.flags_after);
        if let Some(t) = insn.target {
            println!(
                "Target:        {} ({:?}{}){}",
                t.address,
                t.kind,
                if t.certain { "" } else { ", uncertain" },
                s.rom
                    .file_offset_for(t.address)
                    .map_or(String::new(), |o| format!(" = {o}"))
            );
        }
        for a in assumption_names(insn.assumptions) {
            println!("Assumption:    {a}");
        }
        let plain = format_instruction(&insn, &NoSymbols);
        if plain.text != f.text {
            println!("Raw operand:   {}", plain.text);
        }
        let from = s.snap.xrefs_from(off);
        for x in from {
            println!("References:    {} {}", x.kind.name(), x.to);
        }
    }
    for w in s.snap.warnings_at(off) {
        println!("Warning:       {} — {}", w.kind.name(), w.text);
    }
    if let Some(o) = s.project.region_override_at(off) {
        println!(
            "User mark:     {} {}..{}",
            o.kind.name(),
            o.start,
            FileOffset(o.end())
        );
    }
    if let Some(f) = s.project.flag_overrides.get(&off) {
        println!("Flag override: {f:?}");
    }
    Ok(())
}
