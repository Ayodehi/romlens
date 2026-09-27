//! The tutor's tools over a `Workbench` (docs/24, "The tools"): what the
//! app shows, as text a model reads. The app and the CLI both use these,
//! so the tutor sees what the student sees.
//!
//! Every schema is strict: all properties required, none extra, optional
//! values nullable, so every protocol's strict mode takes it.

use std::sync::Arc;

use romlens_tutor::agent::{ToolContext, ToolKind, ToolOutput, Tools};
use romlens_tutor::provider::ToolSpec;
use serde_json::{Value, json};

use super::reference;
use crate::graphics::RecordingSession;
use crate::records::{
    AddressStyle, DecompileLevel, FlagState, RegionKind, ResolvedAny, SearchQuery,
};
use crate::workbench::Workbench;

/// `$BB:AAAA`.
pub fn addr(a: u32) -> String {
    format!("${:02X}:{:04X}", (a >> 16) & 0xFF, a & 0xFFFF)
}

pub struct RomTools {
    wb: Arc<Workbench>,
    /// The recording open in the main window, which the recording tools read.
    recording: std::sync::Mutex<Option<Arc<RecordingSession>>>,
}

impl RomTools {
    pub fn new(wb: Arc<Workbench>) -> RomTools {
        RomTools {
            wb,
            recording: std::sync::Mutex::new(None),
        }
    }

    pub fn set_recording(&self, r: Option<Arc<RecordingSession>>) {
        *self.recording.lock().unwrap_or_else(|e| e.into_inner()) = r;
    }

    /// An address the model wrote: `$80:8000`, `808000`, `0x1234` (a file
    /// offset), a register such as `$2105`, or a label's name.
    fn place(&self, text: &str) -> Result<ResolvedAny, String> {
        let t = text.trim().trim_matches('`');
        if let Ok(r) = self.wb.resolve_any(t.to_owned()) {
            return Ok(r);
        }
        let labels = self.wb.labels();
        match labels.iter().find(|l| l.name.eq_ignore_ascii_case(t)) {
            Some(l) => self
                .wb
                .resolve_any(format!("${:06X}", l.address))
                .map_err(|e| e.to_string()),
            None => Err(format!(
                "`{t}` is neither an address (like $80:8000, or 0x1234 for a file offset) nor a label"
            )),
        }
    }

    fn rom_place(&self, text: &str) -> Result<(u32, u32), String> {
        let r = self.place(text)?;
        match r.file_offset {
            Some(off) => Ok((r.snes_address, off)),
            None => Err(format!("{} is not in the ROM", addr(r.snes_address))),
        }
    }

    fn label(&self, a: u32) -> String {
        self.wb
            .label_at(a)
            .map(|l| format!(" ({})", l.name))
            .unwrap_or_default()
    }
}

// Schema pieces, also for `media`.
pub(crate) fn string(about: &str) -> Value {
    json!({"type": "string", "description": about})
}
pub(crate) fn integer(about: &str) -> Value {
    json!({"type": "integer", "description": about})
}
pub(crate) fn boolean(about: &str) -> Value {
    json!({"type": "boolean", "description": about})
}
pub(crate) fn choice(values: &[&str], about: &str) -> Value {
    json!({"type": "string", "enum": values, "description": about})
}
pub(crate) fn nullable(v: Value) -> Value {
    let about = v["description"].clone();
    let mut inner = v;
    inner.as_object_mut().unwrap().remove("description");
    json!({"anyOf": [inner, {"type": "null"}], "description": about})
}
pub(crate) fn object(props: &[(&str, Value)]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in props {
        p.insert((*k).into(), v.clone());
    }
    json!({
        "type": "object",
        "properties": p,
        "required": props.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        "additionalProperties": false,
    })
}

const ADDRESS: &str =
    "A CPU address such as $80:8000, a file offset such as 0x1234, or a label's name";

pub(crate) fn spec(name: &str, description: &str, props: &[(&str, Value)]) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        schema: object(props),
    }
}

pub fn specs() -> Vec<ToolSpec> {
    let mut v = code_specs();
    v.extend(super::media::specs());
    v.extend(super::edits::specs());
    v
}

fn code_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "rom_info",
            "The ROM's header, mapping, size, vectors and checksum, and how much of it the analysis has classified.",
            &[],
        ),
        spec(
            "resolve",
            "Where an address is: its CPU address and file offset, what memory it is, its mirrors, the hardware register there if any, and its region and label.",
            &[("address", string(ADDRESS))],
        ),
        spec(
            "read_bytes",
            "Raw bytes as hex rows. At most 4096 bytes a call.",
            &[
                ("address", string(ADDRESS)),
                ("length", integer("How many bytes, 1 to 4096")),
            ],
        ),
        spec(
            "listing",
            "Romlens's listing from an address: the instructions as the analysis decoded them (with its M/X flags), data rows, labels and comments. Start here to read code.",
            &[
                ("address", string(ADDRESS)),
                ("lines", integer("How many lines, 1 to 300")),
            ],
        ),
        spec(
            "decode_as",
            "A what-if: decode instructions straight on from an address with the flags you give, ignoring the analysis. Use it to test whether a different M, X or E would make the code sensible.",
            &[
                ("address", string(ADDRESS)),
                ("count", integer("How many instructions, 1 to 200")),
                ("m", boolean("M=1: A and memory are 8-bit")),
                ("x", boolean("X=1: X and Y are 8-bit")),
                ("e", boolean("E=1: emulation mode")),
            ],
        ),
        spec(
            "region_at",
            "The region an address is in: code, data (and what kind) or unknown, its extent, the analysis's confidence and evidence, and any warnings there.",
            &[("address", string(ADDRESS))],
        ),
        spec(
            "regions",
            "The regions in a range of the ROM, at most 100.",
            &[
                ("address", string(ADDRESS)),
                ("length", integer("Bytes to cover")),
                (
                    "kind",
                    choice(
                        &["any", "code", "data", "unknown"],
                        "Only regions of this kind",
                    ),
                ),
            ],
        ),
        spec(
            "labels",
            "The labels (names) in a range of the ROM, at most 200.",
            &[
                ("address", string(ADDRESS)),
                ("length", integer("Bytes to cover")),
            ],
        ),
        spec(
            "find_label",
            "Labels whose name contains some text, ignoring case, at most 50: how to find a routine or variable by name.",
            &[("name", string("Part of a name"))],
        ),
        spec(
            "variables",
            "The typed variables the project defines in RAM.",
            &[],
        ),
        spec(
            "xrefs",
            "Cross-references: what calls, jumps to, reads or writes an address (to), or what the instruction at an address refers to (from).",
            &[
                ("address", string(ADDRESS)),
                (
                    "direction",
                    choice(
                        &["to", "from"],
                        "to: references to the address; from: references the instruction there makes",
                    ),
                ),
            ],
        ),
        spec(
            "search",
            "Search the ROM for bytes (hex with ?? wildcards, such as `A9 ?? 8D 00 21`) or text. At most 50 hits.",
            &[
                ("pattern", string("Hex bytes with ?? for any byte, or text")),
                ("text", boolean("Search for text rather than bytes")),
            ],
        ),
        spec(
            "decompile",
            "The routine at an address as C, as Romlens rebuilds it. lift stays close to the instructions; clean folds them into statements; full adds loops, parameters and typed variables.",
            &[
                (
                    "address",
                    string("The routine's entry, or any address in it"),
                ),
                (
                    "level",
                    choice(&["lift", "clean", "full"], "How far to rebuild"),
                ),
            ],
        ),
        spec(
            "routine_graph",
            "The routine's control flow: its basic blocks, how each ends, the edges between them and its loops.",
            &[(
                "address",
                string("The routine's entry, or any address in it"),
            )],
        ),
        spec(
            "calls",
            "Who calls the routine and what it calls, with each call site.",
            &[(
                "address",
                string("The routine's entry, or any address in it"),
            )],
        ),
        spec(
            "explain_at",
            "What the instruction at an address does to the hardware (a register write decoded field by field, with the value when known) and the idioms it belongs to (waiting for vertical blank, a DMA, a sound upload…) with why games do that.",
            &[("address", string(ADDRESS))],
        ),
        spec(
            "describe_register",
            "A hardware register's fields, decoded for a value if you give one. For registers at $2100-$21FF and $4200-$437F.",
            &[
                (
                    "register",
                    string("The register's address, such as $2105, or its name, such as BGMODE"),
                ),
                (
                    "value",
                    nullable(integer("The value written, or null for the layout only")),
                ),
            ],
        ),
        spec(
            "screen_at",
            "What the screen is set up to be at an instruction: the mode, each layer's tilemap and tiles, scroll, sprites, colour math, as the code has set the registers by then.",
            &[("address", string(ADDRESS))],
        ),
        spec(
            "reference",
            "Romlens's reference pages: 65816_instruction (detail: a mnemonic), 65816_opcodes, addressing_modes, register (detail: an address or name), registers (detail: ppu, cpu, dma, apu or null), dsp_register (detail: $5D or DIR), dsp_registers, spc700_io, spc700_instruction (detail: a mnemonic), idioms.",
            &[
                ("topic", choice(reference::TOPICS, "Which page")),
                ("detail", nullable(string("What the page needs, or null"))),
            ],
        ),
    ]
}

fn int(v: &Value, k: &str, lo: u64, hi: u64) -> Result<u32, String> {
    let n = v[k]
        .as_u64()
        .ok_or_else(|| format!("{k} must be a whole number"))?;
    Ok(n.clamp(lo, hi) as u32)
}

fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str().ok_or_else(|| format!("{k} must be text"))
}

impl Tools for RomTools {
    fn specs(&self) -> Vec<ToolSpec> {
        specs()
    }

    fn kind(&self, name: &str) -> ToolKind {
        if super::edits::NAMES.contains(&name) {
            ToolKind::Edit
        } else {
            ToolKind::Read
        }
    }

    fn run(&self, id: &str, name: &str, input: &Value, cx: &ToolContext) -> ToolOutput {
        let place = |t: &str| self.rom_place(t);
        if super::edits::NAMES.contains(&name) {
            let edits = super::edits::Edits {
                wb: &self.wb,
                place: &place,
            };
            return edits.run(id, name, input, cx);
        }
        let media = super::media::Media {
            wb: &self.wb,
            rec: self
                .recording
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            place: &place,
        };
        if let Some(out) = media.run(name, input) {
            return out;
        }
        match self.dispatch(name, input) {
            Ok(t) => ToolOutput::text(t),
            Err(e) => ToolOutput::error(e),
        }
    }
}

impl RomTools {
    fn dispatch(&self, name: &str, v: &Value) -> Result<String, String> {
        match name {
            "rom_info" => Ok(self.rom_info()),
            "resolve" => self.resolve(text(v, "address")?),
            "read_bytes" => self.read_bytes(text(v, "address")?, int(v, "length", 1, 4096)?),
            "listing" => self.listing(text(v, "address")?, int(v, "lines", 1, 300)?),
            "decode_as" => self.decode_as(v),
            "region_at" => self.region_at(text(v, "address")?),
            "regions" => self.regions(
                text(v, "address")?,
                int(v, "length", 1, u32::MAX as u64)?,
                text(v, "kind")?,
            ),
            "labels" => self.labels(text(v, "address")?, int(v, "length", 1, u32::MAX as u64)?),
            "find_label" => Ok(self.find_label(text(v, "name")?)),
            "variables" => Ok(self.variables()),
            "c_versions" => self.c_versions(v["routine"].as_str()),
            "xrefs" => self.xrefs(text(v, "address")?, text(v, "direction")?),
            "search" => self.search(text(v, "pattern")?, v["text"].as_bool().unwrap_or(false)),
            "decompile" => self.decompile(text(v, "address")?, text(v, "level")?),
            "routine_graph" => self.routine_graph(text(v, "address")?),
            "calls" => self.calls(text(v, "address")?),
            "explain_at" => self.explain_at(text(v, "address")?),
            "describe_register" => describe_register(text(v, "register")?, v["value"].as_u64()),
            "screen_at" => self.screen_at(text(v, "address")?),
            "reference" => reference::page(text(v, "topic")?, v["detail"].as_str()),
            other => Err(format!("there is no tool named {other}")),
        }
    }

    fn rom_info(&self) -> String {
        let i = self.wb.rom().info();
        let s = self.wb.stats();
        let v = |x: u16| format!("$00:{x:04X}");
        format!(
            "Title: {}\nMapping: {}{}\nSize: {} bytes ({} declared){}\nCartridge: {}\nRegion: {}\nVersion: {}\nChecksum: ${:04X} ({})\nNative vectors: NMI {}, IRQ {}, BRK {}, COP {}\nEmulation vectors: RESET {}, NMI {}, IRQ/BRK {}\nSHA-256: {}\nAnalysis: {} code bytes, {} data, {} unknown; {} instructions, {} labels, {} cross-references, {} warnings",
            i.title.trim(),
            i.mapping_name,
            if i.fast_rom { ", FastROM" } else { "" },
            i.byte_len,
            i.declared_rom_size,
            if i.has_copier_header {
                " (with a copier header)"
            } else {
                ""
            },
            i.cartridge_type_name,
            i.region_name,
            i.version,
            i.checksum,
            if i.checksum_ok {
                "matches"
            } else {
                "does not match the ROM"
            },
            v(i.native.nmi),
            v(i.native.irq),
            v(i.native.brk),
            v(i.native.cop),
            v(i.emulation.reset),
            v(i.emulation.nmi),
            v(i.emulation.irq),
            i.sha256,
            s.code_bytes,
            s.data_bytes,
            s.unknown_bytes,
            s.instructions,
            s.labels,
            s.xrefs,
            s.warnings,
        )
    }

    fn resolve(&self, t: &str) -> Result<String, String> {
        let r = self.place(t)?;
        let mut s = format!("{}{}", addr(r.snes_address), self.label(r.snes_address));
        match r.file_offset {
            Some(o) => s.push_str(&format!(", file offset 0x{o:X}")),
            None => s.push_str(", not in the ROM"),
        }
        s.push_str(&format!("\nMemory: {:?}", r.memory_class));
        if let Some(reg) = &r.register {
            s.push_str(&format!(
                "\nRegister: ${:04X} {}: {}",
                reg.address, reg.name, reg.description
            ));
        }
        if !r.mirrors.is_empty() {
            let m: Vec<String> = r.mirrors.iter().take(8).map(|a| addr(*a)).collect();
            s.push_str(&format!("\nMirrors: {}", m.join(", ")));
        }
        if let Some(o) = r.file_offset
            && let Some(region) = self.wb.region_at(o)
        {
            s.push_str(&format!("\nRegion: {}", region.name));
        }
        Ok(s)
    }

    fn read_bytes(&self, t: &str, len: u32) -> Result<String, String> {
        let (a, off) = self.rom_place(t)?;
        let bytes = self.wb.rom().bytes(off, len);
        let mut s = String::new();
        for (i, row) in bytes.chunks(16).enumerate() {
            let at = off + i as u32 * 16;
            let cpu = self
                .wb
                .rom()
                .snes_address_for(at)
                .map(addr)
                .unwrap_or_else(|| addr(a));
            let hex: Vec<String> = row.iter().map(|b| format!("{b:02X}")).collect();
            s.push_str(&format!("{cpu}  0x{at:06X}  {}\n", hex.join(" ")));
        }
        Ok(s)
    }

    fn listing(&self, t: &str, lines: u32) -> Result<String, String> {
        let (_, off) = self.rom_place(t)?;
        let line = self
            .wb
            .line_for_offset(off)
            .ok_or("that address has no line in the listing")?;
        Ok(self.wb.asm_lines_text(line, lines, AddressStyle::Snes))
    }

    fn decode_as(&self, v: &Value) -> Result<String, String> {
        let (_, off) = self.rom_place(text(v, "address")?)?;
        let count = int(v, "count", 1, 200)?;
        let flags = FlagState {
            m: v["m"].as_bool().unwrap_or(true),
            x: v["x"].as_bool().unwrap_or(true),
            e: v["e"].as_bool().unwrap_or(false),
            dbr: None,
            dp: None,
        };
        let mut s = format!(
            "Decoded straight on with M={} X={} E={}, ignoring the analysis:\n",
            flags.m as u8, flags.x as u8, flags.e as u8
        );
        for i in self.wb.disassemble(off, count, Some(flags)) {
            let bytes: Vec<String> = i.bytes.iter().map(|b| format!("{b:02X}")).collect();
            s.push_str(&format!(
                "{}  {:<12} {}\n",
                addr(i.snes_address),
                bytes.join(" "),
                i.text
            ));
        }
        Ok(s)
    }

    fn region_at(&self, t: &str) -> Result<String, String> {
        let (a, off) = self.rom_place(t)?;
        let r = self.wb.region_at(off).ok_or("no region there")?;
        let start = self.wb.rom().snes_address_for(r.start).unwrap_or(a);
        let mut s = format!(
            "{} from {} (file 0x{:X}), {} bytes, confidence {:.0}%",
            r.name,
            addr(start),
            r.start,
            r.len,
            r.confidence * 100.0
        );
        for e in &r.evidence {
            s.push_str(&format!("\n- {:?}: {}", e.kind, e.detail));
        }
        for w in self.wb.warnings_at(off) {
            s.push_str(&format!("\nWarning ({}): {}", w.kind_name, w.text));
        }
        if let Some(f) = self.wb.flag_override_at(off) {
            s.push_str(&format!("\nThe student set the flags here: {f:?}"));
        }
        Ok(s)
    }

    fn regions(&self, t: &str, len: u32, kind: &str) -> Result<String, String> {
        let (_, off) = self.rom_place(t)?;
        let want = match kind {
            "code" => Some(RegionKind::Code),
            "data" => Some(RegionKind::Data),
            "unknown" => Some(RegionKind::Unknown),
            _ => None,
        };
        let rs: Vec<String> = self
            .wb
            .regions(off, len)
            .into_iter()
            .filter(|r| want.is_none_or(|k| r.kind == k))
            .take(100)
            .map(|r| {
                let a = self
                    .wb
                    .rom()
                    .snes_address_for(r.start)
                    .map(addr)
                    .unwrap_or_default();
                format!(
                    "{a}  {} bytes  {}  ({:.0}%)",
                    r.len,
                    r.name,
                    r.confidence * 100.0
                )
            })
            .collect();
        Ok(if rs.is_empty() {
            "none".into()
        } else {
            rs.join("\n")
        })
    }

    fn labels(&self, t: &str, len: u32) -> Result<String, String> {
        let (_, off) = self.rom_place(t)?;
        let ls: Vec<String> = self
            .wb
            .labels_in(off, len)
            .into_iter()
            .take(200)
            .map(|l| format!("{}  {}  ({:?})", addr(l.address), l.name, l.source))
            .collect();
        Ok(if ls.is_empty() {
            "none".into()
        } else {
            ls.join("\n")
        })
    }

    fn find_label(&self, name: &str) -> String {
        let want = name.to_ascii_lowercase();
        let ls: Vec<String> = self
            .wb
            .labels()
            .into_iter()
            .filter(|l| l.name.to_ascii_lowercase().contains(&want))
            .take(50)
            .map(|l| format!("{}  {}  ({:?})", addr(l.address), l.name, l.source))
            .collect();
        if ls.is_empty() {
            format!("no label contains `{name}`")
        } else {
            ls.join("\n")
        }
    }

    fn variables(&self) -> String {
        let vs: Vec<String> = self
            .wb
            .variables()
            .into_iter()
            .map(|v| format!("{}  {}  {}", addr(v.address), v.name, v.description))
            .collect();
        if vs.is_empty() {
            "the project defines no variables yet".into()
        } else {
            vs.join("\n")
        }
    }

    fn c_versions(&self, routine: Option<&str>) -> Result<String, String> {
        let r = routine.map(|t| self.routine(t)).transpose()?;
        let vs = self.wb.c_versions(r);
        if vs.is_empty() {
            return Ok("no C versions saved".into());
        }
        Ok(vs
            .iter()
            .map(|v| {
                let anchors: Vec<String> = v
                    .version
                    .anchors
                    .iter()
                    .map(|a| {
                        format!(
                            "lines {}–{} = {}–{}",
                            a.first,
                            a.last,
                            addr(a.start),
                            addr(a.end)
                        )
                    })
                    .collect();
                format!(
                    "\"{}\" of {} by the {:?}{}\n```c\n{}\n```",
                    v.name,
                    addr(v.routine),
                    v.version.author,
                    if anchors.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", anchors.join(", "))
                    },
                    v.version.text.trim_end()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    fn xrefs(&self, t: &str, direction: &str) -> Result<String, String> {
        let r = self.place(t)?;
        let xs = if direction == "from" {
            let off = r
                .file_offset
                .ok_or("only an instruction in the ROM refers to things")?;
            self.wb.xrefs_from(off)
        } else {
            self.wb.xrefs_to(r.snes_address)
        };
        let rows: Vec<String> = xs
            .iter()
            .take(200)
            .map(|x| {
                let from = x
                    .from_address
                    .map(addr)
                    .unwrap_or_else(|| format!("0x{:X}", x.from_offset));
                format!(
                    "{from}{} → {}{}  {}{}{}",
                    x.from_address.map(|a| self.label(a)).unwrap_or_default(),
                    addr(x.to_address),
                    self.label(x.to_address),
                    x.kind_name,
                    if x.certain { "" } else { ", inferred" },
                    if x.observed { ", seen running" } else { "" },
                )
            })
            .collect();
        Ok(if rows.is_empty() {
            "none found".into()
        } else {
            let more = if xs.len() > 200 {
                format!("\n({} more)", xs.len() - 200)
            } else {
                String::new()
            };
            rows.join("\n") + &more
        })
    }

    fn search(&self, pattern: &str, is_text: bool) -> Result<String, String> {
        let hits = self
            .wb
            .search(SearchQuery {
                pattern: pattern.into(),
                text: is_text,
                ignore_case: is_text,
                start: 0,
                len: self.wb.rom().info().byte_len,
                max: 50,
                context: 0,
            })
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = hits
            .iter()
            .map(|h| {
                format!(
                    "{}  file 0x{:X}  in {}",
                    h.snes_address.clone().unwrap_or_default(),
                    h.file_offset,
                    h.region_kind
                )
            })
            .collect();
        Ok(if rows.is_empty() {
            "no match".into()
        } else {
            rows.join("\n")
        })
    }

    fn routine(&self, t: &str) -> Result<u32, String> {
        let (a, off) = self.rom_place(t)?;
        Ok(self.wb.function_containing(off).unwrap_or(a))
    }

    fn decompile(&self, t: &str, level: &str) -> Result<String, String> {
        let entry = self.routine(t)?;
        let level = match level {
            "lift" => DecompileLevel::Lift,
            "clean" => DecompileLevel::Clean,
            _ => DecompileLevel::Full,
        };
        let d = self
            .wb
            .decompile_blocking(entry, level)
            .map_err(|e| e.to_string())?;
        let mut lines: Vec<&str> = d.text.lines().collect();
        let cut = lines.len() > 600;
        lines.truncate(600);
        let mut s = format!(
            "{} at {}\n```c\n{}\n```",
            d.name,
            addr(d.entry),
            lines.join("\n")
        );
        if cut {
            s.push_str("\n(cut at 600 lines)");
        }
        for w in &d.warnings {
            s.push_str(&format!("\nWarning: {w}"));
        }
        Ok(s)
    }

    fn routine_graph(&self, t: &str) -> Result<String, String> {
        let entry = self.routine(t)?;
        let g = self
            .wb
            .routine_graph_blocking(entry)
            .map_err(|e| e.to_string())?;
        let mut s = format!("{} at {}: {} blocks", g.name, addr(g.entry), g.blocks.len());
        if g.irreducible {
            s.push_str(", irreducible (not plain loops)");
        }
        if g.truncated {
            s.push_str(", cut short");
        }
        for (i, b) in g.blocks.iter().enumerate() {
            let at = b.address.map(addr).unwrap_or_else(|| "(outside)".into());
            let outs: Vec<String> = g
                .edges
                .iter()
                .filter(|e| e.from == i as u32)
                .map(|e| {
                    format!(
                        "{:?}→B{}{}",
                        e.kind,
                        e.to,
                        e.count.map(|c| format!(" ×{c}")).unwrap_or_default()
                    )
                })
                .collect();
            let exit = if b.exit_text.is_empty() {
                format!("{:?}", b.exit).to_lowercase()
            } else {
                b.exit_text.clone()
            };
            s.push_str(&format!(
                "\nB{i} {at}, {} lines, ends: {exit}{}{}{}",
                b.line_count,
                if b.loop_header { ", loop header" } else { "" },
                b.runs.map(|r| format!(", ran {r}×")).unwrap_or_default(),
                if outs.is_empty() {
                    String::new()
                } else {
                    format!("; {}", outs.join(", "))
                },
            ));
        }
        for l in &g.loops {
            s.push_str(&format!("\nLoop: header B{}, body {:?}", l.header, l.body));
        }
        Ok(s)
    }

    fn calls(&self, t: &str) -> Result<String, String> {
        let entry = self.routine(t)?;
        let n = self
            .wb
            .call_neighbourhood_blocking(entry)
            .map_err(|e| e.to_string())?;
        let link = |l: &crate::graphs::CallLinkInfo| {
            let sites: Vec<String> = l
                .sites
                .iter()
                .map(|c| {
                    format!(
                        "{} ({:?}{})",
                        addr(c.address),
                        c.how,
                        c.count.map(|n| format!(", {n}×")).unwrap_or_default()
                    )
                })
                .collect();
            format!("{} {} from {}", addr(l.entry), l.name, sites.join(", "))
        };
        let mut s = format!("{} at {}\nCalled by:", n.name, addr(n.entry));
        if n.callers.is_empty() {
            s.push_str(" nothing the analysis found (an interrupt vector, a table, or unreached)");
        }
        for c in &n.callers {
            s.push_str(&format!("\n- {}", link(c)));
        }
        s.push_str("\nCalls:");
        if n.callees.is_empty() {
            s.push_str(" nothing");
        }
        for c in &n.callees {
            s.push_str(&format!("\n- {}", link(c)));
        }
        Ok(s)
    }

    fn explain_at(&self, t: &str) -> Result<String, String> {
        let (a, off) = self.rom_place(t)?;
        let e = self.wb.explain_at(off);
        let mut s = String::new();
        if let Some(r) = &e.register {
            s.push_str(&format!(
                "{} {}: {}",
                if r.store { "Writes" } else { "Reads" },
                addr(a),
                r.short
            ));
            if let Some(src) = &r.source_name {
                s.push_str(&format!(" (the value comes from {src})"));
            }
            for p in &r.parts {
                s.push_str(&format!("\n${:04X} {}: {}", p.address, p.name, p.about));
                if let Some(v) = p.value {
                    s.push_str(&format!(" Value ${v:X}."));
                }
                for f in &p.fields {
                    if let Some(m) = &f.meaning {
                        s.push_str(&format!("\n  {} {}: {m}", bits(&f.bits), f.name));
                    }
                }
            }
        }
        for i in &e.idioms {
            if !s.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str(&format!(
                "Idiom: {}\n{}\nWhy: {}",
                i.title, i.summary, i.why
            ));
            if !i.offsets.is_empty() {
                let at: Vec<String> = i
                    .offsets
                    .iter()
                    .filter_map(|o| self.wb.rom().snes_address_for(*o))
                    .map(addr)
                    .collect();
                s.push_str(&format!("\nIts instructions: {}", at.join(", ")));
            }
        }
        Ok(if s.is_empty() {
            "nothing to explain there: no hardware access and no idiom Romlens knows".into()
        } else {
            s
        })
    }

    fn screen_at(&self, t: &str) -> Result<String, String> {
        let (_, off) = self.rom_place(t)?;
        let setup = self
            .wb
            .screen_at_blocking(off)
            .ok_or("Romlens could not work out the screen there (no PPU setup reaches it)")?;
        let mut s = format!(
            "The screen as the code has set it up by then (routine {}):",
            addr(setup.routine)
        );
        for sec in &setup.sections {
            s.push_str(&format!("\n## {}", sec.title));
            for r in &sec.rows {
                s.push_str(&format!("\n{}: {}", r.label, r.text));
                if !r.set_at.is_empty() {
                    let at: Vec<String> = r
                        .set_at
                        .iter()
                        .filter_map(|o| self.wb.rom().snes_address_for(*o))
                        .map(addr)
                        .collect();
                    s.push_str(&format!(" (set at {})", at.join(", ")));
                }
            }
        }
        Ok(s)
    }
}

/// `bit 7` or `bits 0–3`.
pub fn bits(b: &str) -> String {
    if b.contains('–') {
        format!("bits {b}")
    } else {
        format!("bit {b}")
    }
}

fn describe_register(reg: &str, value: Option<u64>) -> Result<String, String> {
    let page = reference::page("register", Some(reg))?;
    let Some(v) = value else {
        return Ok(page);
    };
    let address = romlens_core::model::hardware::all_hardware_registers()
        .into_iter()
        .find(|r| page.starts_with(&format!("${:04X}", r.address)))
        .map(|r| r.address)
        .ok_or("no such register")?;
    let w = romlens_core::explain::describe(address, Some(v as u32), if v > 0xFF { 2 } else { 1 })
        .ok_or("Romlens has no field layout for that register")?;
    let mut s = w.short();
    for p in &w.parts {
        for f in &p.fields {
            if let Some(m) = &f.meaning {
                s.push_str(&format!("\n{} {} {}: {m}", p.name, bits(&f.bits), f.name));
            }
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_strict() {
        for t in specs() {
            let s = &t.schema;
            assert_eq!(s["additionalProperties"], false, "{}", t.name);
            let props: Vec<&String> = s["properties"].as_object().unwrap().keys().collect();
            let req: Vec<&str> = s["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(props.len(), req.len(), "{}", t.name);
            for p in props {
                assert!(req.contains(&p.as_str()), "{} {p}", t.name);
            }
        }
    }

    fn fixture() -> RomTools {
        let bytes = romlens_core::fixtures::explain_lorom();
        let rom = crate::Rom::from_bytes(bytes, "explain.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        RomTools::new(wb)
    }

    fn call(t: &RomTools, name: &str, input: Value) -> ToolOutput {
        let on = |_: romlens_tutor::agent::Event| {};
        let cancel = romlens_tutor::http::Cancel::new();
        let cx = ToolContext {
            mode: romlens_tutor::agent::Mode::ReadOnly,
            conversation: "c",
            turn: 1,
            events: &on,
            approver: &romlens_tutor::agent::AcceptAll,
            cancel: &cancel,
        };
        t.run("call", name, &input, &cx)
    }

    #[test]
    fn every_tool_answers_on_a_fixture() {
        let t = fixture();
        let reset = format!("$00:{:04X}", t.wb.rom().info().emulation.reset);
        let cases = [
            ("rom_info", json!({})),
            ("resolve", json!({"address": reset})),
            ("resolve", json!({"address": "$2105"})),
            ("read_bytes", json!({"address": reset, "length": 40})),
            ("listing", json!({"address": reset, "lines": 30})),
            (
                "decode_as",
                json!({"address": reset, "count": 8, "m": true, "x": true, "e": true}),
            ),
            ("region_at", json!({"address": reset})),
            (
                "regions",
                json!({"address": "0x0", "length": 65536, "kind": "code"}),
            ),
            ("labels", json!({"address": "0x0", "length": 65536})),
            ("find_label", json!({"name": "reset"})),
            ("variables", json!({})),
            ("xrefs", json!({"address": reset, "direction": "to"})),
            ("xrefs", json!({"address": reset, "direction": "from"})),
            ("search", json!({"pattern": "78 18 FB", "text": false})),
            ("decompile", json!({"address": reset, "level": "full"})),
            ("routine_graph", json!({"address": reset})),
            ("calls", json!({"address": reset})),
            ("explain_at", json!({"address": reset})),
            (
                "describe_register",
                json!({"register": "$2100", "value": 0x80}),
            ),
            (
                "reference",
                json!({"topic": "register", "detail": "INIDISP"}),
            ),
        ];
        let mut all = String::new();
        for (name, input) in cases {
            let out = call(&t, name, input.clone());
            let text = match &out.parts[0] {
                romlens_tutor::transcript::Part::Text { text } => text.clone(),
                _ => String::new(),
            };
            assert!(!out.is_error, "{name} {input}: {text}");
            assert!(!text.is_empty(), "{name}");
            all.push_str(&format!("=== {name} {input}\n{text}\n"));
        }
        if std::env::var_os("ROMLENS_SHOW_TOOLS").is_some() {
            println!("{all}");
        }
        assert!(all.contains("SEI"), "{all}");
        let bad = call(&t, "listing", json!({"address": "nowhere", "lines": 3}));
        assert!(bad.is_error);
        assert!(call(&t, "no_such_tool", json!({})).is_error);
    }

    #[test]
    fn a_register_is_described_for_a_value() {
        let s = describe_register("BGMODE", Some(0x09)).unwrap();
        assert!(s.to_lowercase().contains("mode 1"), "{s}");
    }
}

#[cfg(test)]
mod size {
    #[test]
    fn the_prefix_is_modest() {
        let tools = serde_json::to_string(&super::specs()).unwrap().len();
        let prompt = romlens_tutor::prompt::system().len();
        // About four characters a token: the fixed prefix stays near 10k
        // tokens, well above every provider's cache minimum.
        assert!(tools + prompt < 48_000, "{tools} + {prompt}");
        assert!(tools + prompt > 8_000);
    }
}
