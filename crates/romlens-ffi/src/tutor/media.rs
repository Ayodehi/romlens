//! The tutor's tools over pictures, sound and recordings (docs/24, "The
//! tools"): tiles from the ROM, a recording's frames, layers and sprites
//! as pictures, the PPU's state, a pixel's provenance, what changed and who
//! wrote it, and the sound side, recorded or traced from the ROM.
//!
//! The recording tools use the recording open in the main window; with
//! none open they say so. They are declared either way, so the tool list
//! stays the same all conversation.

use std::sync::Arc;

use romlens_tutor::agent::ToolOutput;
use romlens_tutor::provider::ToolSpec;
use romlens_tutor::transcript::{ImageRef, Part};
use serde_json::Value;

use super::png;
use super::tools::{addr, bits};
use crate::graphics::{
    BitmapInfo, PaletteSource, RecordingSession, StateRegion, TileFormat, tile_sheet,
};
use crate::workbench::Workbench;

pub fn specs() -> Vec<ToolSpec> {
    use super::tools::{boolean, choice, integer, nullable, spec, string};
    vec![
        spec(
            "preview_at",
            "The data at an address as the analysis sees it: graphics, palette, tilemap or table, as a picture where it is one.",
            &[("address", string(""))],
        ),
        spec(
            "decode_tiles",
            "Draw ROM bytes as tiles, to check whether and how they are graphics.",
            &[
                ("address", string("")),
                ("format", choice(&["2bpp", "4bpp", "8bpp", "mode7"], "")),
                ("count", integer("1–512")),
                (
                    "palette",
                    nullable(string("BGR15 colours in the ROM, or null for grey")),
                ),
            ],
        ),
        spec(
            "recording_info",
            "The open recording: frames, contents, source.",
            &[],
        ),
        spec(
            "ppu_state",
            "The PPU at a frame: mode, layers, scroll, screens, brightness, priority order, CPU position.",
            &[("frame", integer(""))],
        ),
        spec(
            "render_frame",
            "A frame drawn from its PPU state (no colour math, windows or mosaic).",
            &[("frame", integer(""))],
        ),
        spec(
            "render_layer",
            "One layer's whole tilemap at a frame, drawn.",
            &[("frame", integer("")), ("layer", integer("1 to 4"))],
        ),
        spec(
            "render_sprite",
            "One sprite at a frame, drawn.",
            &[("frame", integer("")), ("index", integer("0 to 127"))],
        ),
        spec(
            "pixel_provenance",
            "Where a pixel came from: layer or sprite, tilemap entry, VRAM, the DMA, the WRAM buffer and its code, the ROM bytes.",
            &[
                ("frame", integer("")),
                ("x", integer("0 to 255")),
                ("y", integer("0 to 223")),
            ],
        ),
        spec(
            "changes",
            "Byte ranges of a memory that changed between two frames.",
            &[
                ("from", integer("")),
                ("to", integer("")),
                (
                    "memory",
                    choice(&["wram", "vram", "cgram", "oam", "aram"], ""),
                ),
            ],
        ),
        spec(
            "history",
            "When a memory range last changed before a frame, and next after.",
            &[
                (
                    "memory",
                    choice(&["wram", "vram", "cgram", "oam", "aram"], ""),
                ),
                ("offset", integer("WRAM $7E:0000 is 0")),
                ("length", integer("Bytes")),
                ("frame", integer("")),
            ],
        ),
        spec(
            "who_writes",
            "Instructions the execution log saw write a WRAM address or PPU register, with counts.",
            &[("address", string(""))],
        ),
        spec(
            "sound_upload",
            "The sound uploads traced from the ROM: routines, blocks (ARAM address, length, ROM source), entry, and sound commands.",
            &[],
        ),
        spec(
            "voices",
            "The eight voices at a frame: sample, pitch, note, volume, envelope.",
            &[("frame", integer(""))],
        ),
        spec(
            "dsp_registers",
            "DSP registers at a frame, explained.",
            &[
                ("frame", integer("")),
                (
                    "voice",
                    nullable(integer("0–7, or null for the global ones")),
                ),
            ],
        ),
        spec(
            "aram_map",
            "What audio RAM holds at a frame (driver, directory, samples, echo), with ROM sources.",
            &[("frame", integer(""))],
        ),
        spec(
            "samples",
            "The directory's samples at a frame: start, loop, length, tuning, played.",
            &[("frame", integer(""))],
        ),
        spec(
            "spc_listing",
            "SPC700 code in audio RAM at a frame, disassembled.",
            &[
                ("frame", integer("")),
                ("address", nullable(integer("null: where the SPC700 was"))),
                ("count", integer("1–200")),
            ],
        ),
        spec(
            "note_timeline",
            "Key ons, key offs and pitch changes per voice over frames, with their SPC700 instructions.",
            &[("from", integer("")), ("to", integer(""))],
        ),
        spec(
            "port_events",
            "Bytes written to the four APU ports over frames, and by which CPU.",
            &[("from", integer("")), ("to", integer(""))],
        ),
        spec(
            "brr_sample",
            "A BRR sample in the ROM, block by block.",
            &[
                ("address", string("")),
                ("blocks", integer("1–512")),
                ("with_steps", boolean("The first block's arithmetic too")),
            ],
        ),
    ]
}

fn picture(b: &BitmapInfo, about: String) -> ToolOutput {
    if b.width == 0 || b.height == 0 {
        return ToolOutput::error("there is nothing to draw");
    }
    let s = png::scale_for_model(b);
    let bytes = png::encode(b, s);
    let image = ImageRef {
        id: png::id(&bytes),
        media_type: "image/png".into(),
    };
    let text = if s > 1 {
        format!(
            "{about}\n({}×{} pixels, shown {s}× larger)",
            b.width, b.height
        )
    } else {
        format!("{about}\n({}×{} pixels)", b.width, b.height)
    };
    ToolOutput {
        summary: about.lines().next().unwrap_or_default().to_owned(),
        parts: vec![
            Part::Text { text },
            Part::Image {
                image: image.clone(),
            },
        ],
        is_error: false,
        pictures: vec![(image, bytes)],
    }
}

fn int(v: &Value, k: &str) -> Result<u64, String> {
    v[k].as_u64()
        .ok_or_else(|| format!("{k} must be a whole number"))
}

fn memory(m: &str) -> StateRegion {
    match m {
        "vram" => StateRegion::Vram,
        "cgram" => StateRegion::Cgram,
        "oam" => StateRegion::Oam,
        "aram" => StateRegion::Aram,
        _ => StateRegion::Wram,
    }
}

/// Offsets as ranges: `$0010–$001F, $0200`.
fn ranges(offsets: &[u32], limit: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < offsets.len() && out.len() < limit {
        let start = offsets[i];
        let mut end = start;
        while i + 1 < offsets.len() && offsets[i + 1] == end + 1 {
            i += 1;
            end = offsets[i];
        }
        out.push(if end == start {
            format!("${start:04X}")
        } else {
            format!("${start:04X}–${end:04X}")
        });
        i += 1;
    }
    let mut s = out.join(", ");
    if i < offsets.len() {
        s.push_str(", …");
    }
    s
}

/// Resolves an address the model wrote to its CPU address and file offset.
pub type Place<'a> = &'a dyn Fn(&str) -> Result<(u32, u32), String>;

pub struct Media<'a> {
    pub wb: &'a Arc<Workbench>,
    pub rec: Option<Arc<RecordingSession>>,
    pub place: Place<'a>,
}

impl Media<'_> {
    fn rec(&self) -> Result<&Arc<RecordingSession>, String> {
        self.rec
            .as_ref()
            .ok_or_else(|| "no recording is open in Romlens: the student can open one, or record one with Mesen".to_owned())
    }

    fn frame(&self, v: &Value) -> Result<u64, String> {
        let f = int(v, "frame")?;
        let info = self.rec()?.info();
        let last = info.first_frame + info.frame_count.saturating_sub(1);
        if f < info.first_frame || f > last {
            return Err(format!(
                "the recording has frames {} to {last}",
                info.first_frame
            ));
        }
        Ok(f)
    }

    /// `None` when the tool is not one of these.
    pub fn run(&self, name: &str, v: &Value) -> Option<ToolOutput> {
        let r = match name {
            "preview_at" => self.preview_at(v),
            "decode_tiles" => self.decode_tiles(v),
            "render_frame" => self.render_frame(v),
            "render_layer" => self.render_layer(v),
            "render_sprite" => self.render_sprite(v),
            _ => {
                let t = match name {
                    "recording_info" => self.recording_info(),
                    "ppu_state" => self.ppu_state(v),
                    "pixel_provenance" => self.pixel_provenance(v),
                    "changes" => self.changes(v),
                    "history" => self.history(v),
                    "who_writes" => self.who_writes(v),
                    "sound_upload" => Ok(self.sound_upload()),
                    "voices" => self.voices(v),
                    "dsp_registers" => self.dsp_registers(v),
                    "aram_map" => self.aram_map(v),
                    "samples" => self.samples(v),
                    "spc_listing" => self.spc_listing(v),
                    "note_timeline" => self.note_timeline(v),
                    "port_events" => self.port_events(v),
                    "brr_sample" => self.brr_sample(v),
                    _ => return None,
                };
                t.map(ToolOutput::text)
            }
        };
        Some(r.unwrap_or_else(ToolOutput::error))
    }

    fn preview_at(&self, v: &Value) -> Result<ToolOutput, String> {
        let (_, off) = (self.place)(v["address"].as_str().unwrap_or_default())?;
        let p = self
            .wb
            .preview_at(off)
            .ok_or("the analysis has no preview for the data there")?;
        let about = format!("{}: {}", p.kind, p.summary);
        Ok(match &p.bitmap {
            Some(b) => picture(b, about),
            None => ToolOutput::text(about),
        })
    }

    fn decode_tiles(&self, v: &Value) -> Result<ToolOutput, String> {
        let (a, off) = (self.place)(v["address"].as_str().unwrap_or_default())?;
        let (format, len, name) = match v["format"].as_str() {
            Some("2bpp") => (TileFormat::Bpp2, 16, "2 bpp"),
            Some("8bpp") => (TileFormat::Bpp8, 64, "8 bpp"),
            Some("mode7") => (TileFormat::Mode7, 64, "mode 7"),
            _ => (TileFormat::Bpp4, 32, "4 bpp"),
        };
        let count = int(v, "count")?.clamp(1, 512) as u32;
        let rom = self.wb.rom();
        let bytes = rom.bytes(off, count * len);
        let count = bytes.len() as u32 / len;
        let palette = match v["palette"].as_str() {
            Some(p) => {
                let (_, po) = (self.place)(p)?;
                PaletteSource::Colours {
                    bytes: rom.bytes(po, 512),
                }
            }
            None => PaletteSource::Grayscale,
        };
        let b = tile_sheet(bytes, format, count, 16, palette);
        Ok(picture(
            &b,
            format!("{count} tiles at {} as {name}", addr(a)),
        ))
    }

    fn recording_info(&self) -> Result<String, String> {
        let i = self.rec()?.info();
        Ok(format!(
            "Frames {} to {}{}, a keyframe every {}\nFrom {} {}\nHolds: {:?}\nSound: {}",
            i.first_frame,
            i.first_frame + i.frame_count.saturating_sub(1),
            if i.live {
                " (live, still arriving)"
            } else {
                ""
            },
            i.keyframe_interval,
            i.producer,
            i.producer_version,
            i.regions,
            if self.rec()?.has_sound() { "yes" } else { "no" },
        ))
    }

    fn ppu_state(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let rec = self.rec()?;
        let p = rec.ppu_summary(f).map_err(|e| e.to_string())?;
        let mut s = format!(
            "Frame {f}: mode {}, INIDISP ${:02X}{}, main screen ${:02X}, OBSEL ${:02X}; the CPU was at {}",
            p.bg_mode,
            p.inidisp,
            if p.inidisp & 0x80 != 0 {
                " (forced blank)"
            } else {
                ""
            },
            p.main_screen,
            p.obsel,
            addr(p.pc)
        );
        for l in &p.layers {
            s.push_str(&format!(
                "\nBG{}: {}, tilemap at VRAM word ${:04X} ({:?}), tiles at word ${:04X}{}, scroll {},{}",
                l.bg,
                l.format.map(|f| format!("{f:?}")).unwrap_or_else(|| "off in this mode".into()),
                l.map_word,
                l.size,
                l.char_word,
                if l.tile16 { ", 16×16 tiles" } else { "" },
                l.hscroll,
                l.vscroll
            ));
        }
        if let Ok(order) = rec.priority_order(f) {
            s.push_str(&format!("\nPriority, front to back: {}", order.join(", ")));
        }
        Ok(s)
    }

    fn render_frame(&self, v: &Value) -> Result<ToolOutput, String> {
        let f = self.frame(v)?;
        let img = self.rec()?.render_frame(f).map_err(|e| e.to_string())?;
        let mut about = format!("Frame {f}, mode {}, drawn from the PPU state", img.bg_mode);
        if img.per_line {
            about.push_str("; registers change down the screen, and those changes are drawn");
        }
        if !img.unsupported.is_empty() {
            about.push_str(&format!("; not drawn: {}", img.unsupported.join(", ")));
        }
        Ok(picture(&img.image, about))
    }

    fn render_layer(&self, v: &Value) -> Result<ToolOutput, String> {
        let f = self.frame(v)?;
        let bg = int(v, "layer")?.clamp(1, 4) as u8;
        let b = self.rec()?.render_bg(f, bg).map_err(|e| e.to_string())?;
        Ok(picture(
            &b,
            format!("BG{bg} at frame {f}, its whole tilemap"),
        ))
    }

    fn render_sprite(&self, v: &Value) -> Result<ToolOutput, String> {
        let f = self.frame(v)?;
        let i = int(v, "index")?.min(127) as u8;
        let b = self.rec()?.render_sprite(f, i).map_err(|e| e.to_string())?;
        Ok(picture(&b, format!("Sprite {i} at frame {f}")))
    }

    fn pixel_provenance(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let (x, y) = (int(v, "x")?.min(255) as u32, int(v, "y")?.min(238) as u32);
        let p = self
            .wb
            .pixel_provenance_blocking(self.rec()?.clone(), f, x, y)
            .ok_or("Romlens could not trace that pixel")?;
        let rom = self.wb.rom();
        let mut s = format!("Pixel {x},{y} at frame {f}: {}", p.summary);
        for part in &p.parts {
            s.push_str(&format!("\n## {}", part.what));
            for l in &part.links {
                s.push_str(&format!("\n- {}", l.summary));
                if let Some(pc) = l.started_at {
                    s.push_str(&format!(" (started by the store at {})", addr(pc)));
                }
                if let Some(o) = l.rom_start
                    && let Some(a) = rom.snes_address_for(o)
                {
                    s.push_str(&format!(" (from ROM {}, {} bytes)", addr(a), l.rom_len));
                }
            }
            if let Some(h) = &part.hop {
                s.push_str(&format!("\n{}", h.code_summary));
                for c in h.code.iter().take(8) {
                    s.push_str(&format!(
                        "\n- {} wrote it {}×{}",
                        addr(c.pc),
                        c.count,
                        if c.clears { " (clearing memory)" } else { "" }
                    ));
                }
                if let Some(ps) = &h.placed_summary {
                    s.push_str(&format!("\n{ps}"));
                }
            }
        }
        Ok(s)
    }

    fn changes(&self, v: &Value) -> Result<String, String> {
        let rec = self.rec()?;
        let (from, to) = (int(v, "from")?, int(v, "to")?);
        let m = v["memory"].as_str().unwrap_or("wram");
        let offs = rec
            .changes(from, to, memory(m))
            .map_err(|e| e.to_string())?;
        Ok(if offs.is_empty() {
            format!("nothing in {m} changed from frame {from} to {to}")
        } else {
            format!(
                "{} bytes of {m} changed from frame {from} to {to}: {}",
                offs.len(),
                ranges(&offs, 80)
            )
        })
    }

    fn history(&self, v: &Value) -> Result<String, String> {
        let rec = self.rec()?;
        let m = v["memory"].as_str().unwrap_or("wram");
        let h = rec
            .history(
                memory(m),
                int(v, "offset")? as u32,
                int(v, "length")?.max(1) as u32,
                int(v, "frame")?,
            )
            .map_err(|e| e.to_string())?;
        if !h.indexed {
            return Ok("the recording is live, so it has no change index yet".into());
        }
        Ok(format!(
            "Last changed: {}\nNext changes: {}",
            h.last
                .map(|f| format!("frame {f}"))
                .unwrap_or_else(|| "not before".into()),
            h.next
                .map(|f| format!("frame {f}"))
                .unwrap_or_else(|| "not after".into())
        ))
    }

    fn who_writes(&self, v: &Value) -> Result<String, String> {
        use romlens_core::provenance::source::{Written, code_writing};
        let text = v["address"].as_str().unwrap_or_default();
        let r = self
            .wb
            .resolve_any(text.to_owned())
            .map_err(|e| e.to_string())?;
        let a = r.snes_address;
        let what = match (a >> 16, a & 0xFFFF) {
            (0x7E | 0x7F, _) => Written::Wram(a - 0x7E_0000),
            (b, lo) if lo < 0x2000 && (b < 0x40 || (0x80..0xC0).contains(&b)) => Written::Wram(lo),
            (_, lo) if (0x2100..0x2200).contains(&lo) => Written::Port(lo as u16),
            _ => return Err(format!("{} is neither WRAM nor a PPU register", addr(a))),
        };
        let writers = self
            .wb
            .with_project(|p| p.exec_log.as_ref().map(|l| code_writing(l, what)))
            .ok_or(
                "the project has no execution log: play the game under the Mesen fork to make one",
            )?;
        if writers.is_empty() {
            return Ok(format!("the execution log saw nothing write {}", addr(a)));
        }
        let rows: Vec<String> = writers
            .iter()
            .take(20)
            .map(|w| {
                format!(
                    "{}{}  {}×, writing {} bytes around it",
                    addr(w.pc),
                    self.wb
                        .label_at(w.pc)
                        .map(|l| format!(" ({})", l.name))
                        .unwrap_or_default(),
                    w.count,
                    w.span
                )
            })
            .collect();
        Ok(format!(
            "Writers of {}, narrowest first:\n{}",
            addr(a),
            rows.join("\n")
        ))
    }

    fn sound_upload(&self) -> String {
        let r = self.wb.sound_upload_blocking();
        let rom = self.wb.rom();
        let at = |o: u32| {
            rom.snes_address_for(o)
                .map(addr)
                .unwrap_or_else(|| format!("0x{o:X}"))
        };
        let mut s = String::new();
        for u in &r.routines {
            s.push_str(&format!(
                "Upload routine at {}, reading its list through direct page ${:02X}\n",
                addr(u.entry),
                u.pointer
            ));
        }
        for u in &r.uploads {
            s.push_str(&format!(
                "\n{}{}: {} bytes in {} blocks, then the SPC700 runs from ${:04X} (set at {})",
                u.source,
                if u.driver { ", the driver" } else { "" },
                u.bytes,
                u.blocks.len(),
                u.entry,
                at(u.set_at)
            ));
            for b in u.blocks.iter().take(24) {
                s.push_str(&format!(
                    "\n- ${:04X}, {} bytes, from ROM {}",
                    b.aram,
                    b.len,
                    addr(b.snes)
                ));
            }
        }
        if !r.commands.is_empty() {
            s.push_str("\n\nSound commands:");
            for c in r.commands.iter().take(60) {
                s.push_str(&format!(
                    "\n- ${:0w$X} to port {} at {}{}",
                    c.value,
                    c.port,
                    at(c.at),
                    c.via
                        .map(|v| format!(" (through {})", addr(v)))
                        .unwrap_or_default(),
                    w = c.width as usize * 2
                ));
            }
        }
        if s.is_empty() {
            "Romlens found no sound upload it could trace in the ROM; a recording shows what was sent".into()
        } else {
            s.trim_start().to_owned()
        }
    }

    fn sound(&self) -> Result<&Arc<RecordingSession>, String> {
        let r = self.rec()?;
        if !r.has_sound() {
            return Err("the recording has no sound layer: record it again with this Romlens's recorder script".into());
        }
        Ok(r)
    }

    fn voices(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let vs = self.sound()?.voices(f).map_err(|e| e.to_string())?;
        let rows: Vec<String> = vs
            .iter()
            .map(|x| {
                format!(
                    "Voice {}: {}; sample {}{}; pitch ${:04X} ({}){}; volume {}/{}; {}; ENVX {}{}{}{}{}{}",
                    x.index,
                    if x.sounding { "sounding" } else { "silent" },
                    x.source,
                    x.sample_start.map(|s| format!(" at ${s:04X}")).unwrap_or_default(),
                    x.pitch,
                    x.pitch_words,
                    x.note.as_ref().map(|n| format!(", about {n}")).unwrap_or_default(),
                    x.volume_left,
                    x.volume_right,
                    x.envelope_words,
                    x.envx,
                    if x.echo { ", echo" } else { "" },
                    if x.noise { ", noise" } else { "" },
                    if x.modulated { ", pitch modulated" } else { "" },
                    if x.keyed_off { ", keyed off" } else { "" },
                    if x.ended { ", sample ended" } else { "" },
                )
            })
            .collect();
        Ok(format!("Frame {f}:\n{}", rows.join("\n")))
    }

    fn dsp_registers(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let voice = v["voice"].as_u64().map(|n| n.min(7) as u8);
        let rs = self.sound()?.dsp_registers(f).map_err(|e| e.to_string())?;
        let rows: Vec<String> = rs
            .iter()
            .filter(|r| r.voice == voice && !r.unused)
            .map(|r| {
                let mut s = format!("${:02X} {}", r.register, r.short);
                for p in &r.parts {
                    for fr in &p.fields {
                        if let Some(m) = &fr.meaning {
                            s.push_str(&format!("\n  {} {}: {m}", bits(&fr.bits), fr.name));
                        }
                    }
                }
                s
            })
            .collect();
        Ok(rows.join("\n"))
    }

    fn aram_map(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let m = self.sound()?.aram_map(f).map_err(|e| e.to_string())?;
        let rom = self.wb.rom();
        let rows: Vec<String> = m
            .iter()
            .map(|r| {
                format!(
                    "${:04X}–${:04X}  {}{}",
                    r.start,
                    (r.start as u32 + r.len.max(1) - 1).min(0xFFFF),
                    r.label,
                    r.rom_offset
                        .and_then(|o| rom.snes_address_for(o))
                        .map(|a| format!(" (from ROM {})", addr(a)))
                        .unwrap_or_default()
                )
            })
            .collect();
        Ok(rows.join("\n"))
    }

    fn samples(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let ss = self.sound()?.samples(f).map_err(|e| e.to_string())?;
        let rom = self.wb.rom();
        let rows: Vec<String> = ss
            .iter()
            .map(|x| {
                format!(
                    "Sample {}: ${:04X}, {} blocks{}{}{}{}",
                    x.index,
                    x.start,
                    x.blocks,
                    if x.loops {
                        format!(", loops at ${:04X}", x.loop_at)
                    } else {
                        ", no loop".into()
                    },
                    x.tuning_note
                        .as_ref()
                        .map(|n| format!(", tuned about {n}"))
                        .unwrap_or_default(),
                    match x.played {
                        Some(true) => ", played",
                        Some(false) => ", not played while logged",
                        None => "",
                    },
                    x.rom_offset
                        .and_then(|o| rom.snes_address_for(o))
                        .map(|a| format!(", from ROM {}", addr(a)))
                        .unwrap_or_default(),
                )
            })
            .collect();
        Ok(if rows.is_empty() {
            "no samples in the directory".into()
        } else {
            rows.join("\n")
        })
    }

    fn spc_listing(&self, v: &Value) -> Result<String, String> {
        let f = self.frame(v)?;
        let rec = self.sound()?;
        let from = match v["address"].as_u64() {
            Some(a) => a.min(0xFFFF) as u16,
            None => rec.spc_pc(f).map_err(|e| e.to_string())?,
        };
        let count = int(v, "count")?.clamp(1, 200) as u32;
        let lines = rec.spc_listing(f, from, count).map_err(|e| e.to_string())?;
        let rows: Vec<String> = lines
            .iter()
            .map(|l| {
                let bytes: Vec<String> = l.bytes.iter().map(|b| format!("{b:02X}")).collect();
                let mut s = format!(
                    "{}${:04X}  {:<9} {}",
                    l.label
                        .as_ref()
                        .map(|n| format!("{n}:\n"))
                        .unwrap_or_default(),
                    l.address,
                    bytes.join(" "),
                    l.text
                );
                if let Some(c) = &l.comment {
                    s.push_str(&format!("  ; {c}"));
                }
                if let Some(i) = &l.idiom {
                    s.push_str(&format!("  ; {}", i.title));
                }
                s
            })
            .collect();
        Ok(format!("Audio RAM at frame {f}:\n{}", rows.join("\n")))
    }

    fn note_timeline(&self, v: &Value) -> Result<String, String> {
        let (from, to) = (int(v, "from")?, int(v, "to")?);
        let notes = self
            .sound()?
            .note_timeline(from, to)
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = notes
            .iter()
            .take(300)
            .map(|n| {
                format!(
                    "frame {}  voice {}  {:?}  sample {}  pitch ${:04X}{}",
                    n.frame,
                    n.voice,
                    n.kind,
                    n.source,
                    n.pitch,
                    n.spc_pc
                        .map(|p| format!("  (SPC700 ${p:04X})"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let more = notes.len().saturating_sub(300);
        Ok(if rows.is_empty() {
            "no notes in those frames".into()
        } else if more > 0 {
            format!("{}\n({more} more)", rows.join("\n"))
        } else {
            rows.join("\n")
        })
    }

    fn port_events(&self, v: &Value) -> Result<String, String> {
        let (from, to) = (int(v, "from")?, int(v, "to")?);
        let ev = self
            .sound()?
            .port_events(from, to)
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = ev
            .iter()
            .take(300)
            .map(|e| {
                format!(
                    "frame {}  {} port {} = ${:02X}",
                    e.frame,
                    if e.from_cpu {
                        "S-CPU →"
                    } else {
                        "SPC700 →"
                    },
                    e.port,
                    e.value
                )
            })
            .collect();
        Ok(if rows.is_empty() {
            "the ports were quiet".into()
        } else {
            rows.join("\n")
        })
    }

    fn brr_sample(&self, v: &Value) -> Result<String, String> {
        let (a, off) = (self.place)(v["address"].as_str().unwrap_or_default())?;
        let max = int(v, "blocks")?.clamp(1, 512) as u32;
        let s = crate::audio::brr_at(self.wb.rom(), off, None, max);
        let mut out = format!(
            "BRR at {}: {} blocks{}{}",
            addr(a),
            s.blocks.len(),
            if s.loops { ", loops" } else { ", ends" },
            if s.unterminated {
                " (no end block within the limit: maybe not a sample)"
            } else {
                ""
            }
        );
        for (i, b) in s.blocks.iter().enumerate().take(64) {
            out.push_str(&format!(
                "\nblock {i}: header ${:02X}, shift {}, filter {} ({}){}{}",
                b.header,
                b.shift,
                b.filter,
                b.filter_meaning,
                if b.loops { ", loop" } else { "" },
                if b.end { ", end" } else { "" }
            ));
        }
        if v["with_steps"].as_bool() == Some(true)
            && let Some(b) = s.blocks.first()
        {
            out.push_str(&format!(
                "\nThe first block's filter: {}\n{:?}",
                b.filter_formula, b.steps
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutor::tools::RomTools;
    use romlens_tutor::agent::{Event, Mode, ToolContext, Tools};
    use serde_json::json;

    fn call(t: &RomTools, name: &str, input: Value) -> ToolOutput {
        let on = |_: Event| {};
        let cancel = romlens_tutor::http::Cancel::new();
        let cx = ToolContext {
            mode: Mode::ReadOnly,
            conversation: "c",
            turn: 1,
            events: &on,
            approver: &romlens_tutor::agent::AcceptAll,
            cancel: &cancel,
        };
        t.run("call", name, &input, &cx)
    }

    fn text(o: &ToolOutput) -> String {
        match &o.parts[0] {
            Part::Text { text } => text.clone(),
            _ => String::new(),
        }
    }

    #[test]
    fn pictures_and_the_ppu_from_a_recording() {
        let rom = crate::Rom::from_bytes(crate::graphics::make_graphics_test_rom(), "g.sfc".into())
            .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let t = RomTools::new(wb);
        let none = call(&t, "render_frame", json!({"frame": 8}));
        assert!(none.is_error && text(&none).contains("no recording"));
        t.set_recording(Some(
            RecordingSession::from_bytes(crate::graphics::make_test_recording(40)).unwrap(),
        ));

        let f = call(&t, "render_frame", json!({"frame": 8}));
        assert!(!f.is_error, "{}", text(&f));
        assert_eq!(f.pictures.len(), 1);
        assert!(matches!(&f.parts[1], Part::Image { image } if image.media_type == "image/png"));
        assert_eq!(&f.pictures[0].1[..4], b"\x89PNG");
        assert!(text(&f).contains("256×224"), "{}", text(&f));

        let out_of_range = call(&t, "ppu_state", json!({"frame": 4000}));
        assert!(out_of_range.is_error, "{}", text(&out_of_range));
        let p = call(&t, "ppu_state", json!({"frame": 8}));
        assert!(text(&p).contains("mode 1"), "{}", text(&p));
        for (name, input) in [
            ("render_layer", json!({"frame": 8, "layer": 1})),
            ("render_sprite", json!({"frame": 8, "index": 3})),
            ("pixel_provenance", json!({"frame": 8, "x": 150, "y": 55})),
            ("changes", json!({"from": 0, "to": 20, "memory": "vram"})),
            (
                "history",
                json!({"memory": "wram", "offset": 0, "length": 16, "frame": 10}),
            ),
            ("recording_info", json!({})),
            (
                "decode_tiles",
                json!({"address": "0x0", "count": 32, "format": "4bpp", "palette": null}),
            ),
        ] {
            let o = call(&t, name, input.clone());
            assert!(!o.is_error, "{name} {input}: {}", text(&o));
        }
        let who = call(&t, "who_writes", json!({"address": "$7E:0AF6"}));
        assert!(
            who.is_error && text(&who).contains("execution log"),
            "{}",
            text(&who)
        );
    }

    #[test]
    fn the_sound_side_from_a_recording_and_the_rom() {
        let rom =
            crate::Rom::from_bytes(crate::audio::make_sound_test_rom(), "s.sfc".into()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let t = RomTools::new(wb);
        let up = call(&t, "sound_upload", json!({}));
        assert!(
            text(&up).contains("the SPC700 runs from $0200"),
            "{}",
            text(&up)
        );
        t.set_recording(Some(
            RecordingSession::from_bytes(crate::audio::make_sound_test_recording(8)).unwrap(),
        ));
        let v = call(&t, "voices", json!({"frame": 3}));
        assert!(text(&v).contains("Voice 0: sounding"), "{}", text(&v));
        for (name, input) in [
            ("dsp_registers", json!({"frame": 3, "voice": 0})),
            ("dsp_registers", json!({"frame": 3, "voice": null})),
            ("aram_map", json!({"frame": 3})),
            ("samples", json!({"frame": 3})),
            (
                "spc_listing",
                json!({"frame": 3, "address": null, "count": 12}),
            ),
            ("note_timeline", json!({"from": 0, "to": 7})),
            ("port_events", json!({"from": 0, "to": 7})),
        ] {
            let o = call(&t, name, input.clone());
            assert!(!o.is_error, "{name} {input}: {}", text(&o));
            if std::env::var_os("ROMLENS_SHOW_TOOLS").is_some() {
                println!("=== {name}\n{}", text(&o));
            }
        }
    }

    #[test]
    fn offsets_read_as_ranges() {
        assert_eq!(
            ranges(&[1, 2, 3, 7, 9, 10], 10),
            "$0001–$0003, $0007, $0009–$000A"
        );
        assert_eq!(ranges(&[1, 3, 5], 2), "$0001, $0003, …");
    }
}
