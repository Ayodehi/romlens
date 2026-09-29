//! The tutor's diagrams (docs/26): `draw_diagram`, one of Romlens's kinds
//! drawn from the core's data, and `draw_svg`, the tutor's own SVG once it
//! passes Romlens's checks. Both give back the picture, with its id for a
//! lesson step, and what it shows in words.

use std::sync::Arc;

use romlens_core::explain;
use romlens_core::provenance::{Transfer, attribute, start_time};
use romlens_draw::kinds::{self, ChainStep, FrameEvent, FrameEvents, Source};
use romlens_tutor::agent::ToolOutput;
use romlens_tutor::provider::ToolSpec;
use romlens_tutor::transcript::{ImageRef, Part};
use serde_json::{Value, json};

use super::png;
use super::tools::addr;
use crate::graphics::RecordingSession;
use crate::workbench::Workbench;

pub const NAMES: &[&str] = &["draw_diagram", "draw_svg"];

pub fn specs() -> Vec<ToolSpec> {
    use super::tools::{choice, spec, string};
    vec![
        spec(
            "draw_diagram",
            "Draw a diagram from Romlens's own data, for an answer or a lesson step's picture. The spec is JSON, with null for what you leave out:
- fields {register (a name or $21xx), value, highlight: [field names]}: a register's bits.
- memory_map {view: bank|banks, bank, marks: [{address, label}], highlight: [{start, end, label}]}: a bank, or every bank.
- blocks {preset: machine, highlight, edges: [{from, to, label, both}]}: the SNES's chips and buses (cart cpu wram ppu smp vram oam cgram aram dsp abus bbus), edges routed along its wires; or {title, nodes: [{id, label, note}], edges, direction: down|right, highlight} for boxes of your own.
- timeline {span: frame|line, frame (a recording's), marks: [{at: a line or a dot, label}]}.
- chain {from: {frame, x, y}}: a recorded pixel's way from the ROM; or {title, steps: [{label, place: an address, a name, or ROM WRAM VRAM OAM CGRAM ARAM screen}]}.",
            &[
                ("kind", choice(kinds::KINDS, "")),
                ("spec", json!({"type": "object"})),
            ],
        ),
        spec(
            "draw_svg",
            "Draw your own SVG when no kind of draw_diagram fits. A viewBox at most 1600 × 1200, shown at most 720 wide (so text is 11 or more there); no scripts, images or outside links. Romlens checks that everything is on the canvas, no text overlaps or leaves its box, and text has contrast, then returns the picture: look at it before you use it.",
            &[("svg", string("")), ("title", string(""))],
        ),
    ]
}

/// The picture as a tool result: the words, then the image, kept by its id.
fn output(png: Vec<u8>, title: &str, text: String) -> ToolOutput {
    let id = png::id(&png).replacen("tool-", "draw-", 1);
    let image = ImageRef {
        id: id.clone(),
        media_type: "image/png".into(),
    };
    ToolOutput {
        summary: title.to_owned(),
        parts: vec![
            Part::Text {
                text: text.replace("{id}", &id),
            },
            Part::Image {
                image: image.clone(),
            },
        ],
        is_error: false,
        pictures: vec![(image, png)],
    }
}

pub struct Draw<'a> {
    pub wb: &'a Arc<Workbench>,
    pub rom: Arc<crate::Rom>,
    pub rec: Option<Arc<RecordingSession>>,
    pub resolve: &'a dyn Fn(&str) -> Result<u32, String>,
}

impl Draw<'_> {
    pub fn run(&self, name: &str, v: &Value) -> Option<ToolOutput> {
        Some(match name {
            "draw_diagram" => self.diagram(v),
            "draw_svg" => svg(v),
            _ => return None,
        })
    }

    fn diagram(&self, v: &Value) -> ToolOutput {
        let kind = v["kind"].as_str().unwrap_or_default();
        let d = match kinds::draw(kind, &v["spec"], self) {
            Ok(d) => d,
            Err(e) => return ToolOutput::error(format!("Romlens could not draw that: {e}")),
        };
        match romlens_draw::render(&d.svg) {
            Ok(p) => output(
                p.png,
                &d.title,
                format!(
                    "{}, drawn by Romlens from its own data; its id is {{id}}, for a lesson step's picture. What it shows:\n{}",
                    d.title, d.description
                ),
            ),
            Err(p) => ToolOutput::error(format!(
                "Romlens drew this badly, which is a fault in Romlens, not in your spec:\n{}",
                romlens_draw::check::describe(&p)
            )),
        }
    }
}

fn svg(v: &Value) -> ToolOutput {
    let svg = v["svg"].as_str().unwrap_or_default();
    let title = v["title"].as_str().unwrap_or("A diagram").trim();
    match romlens_draw::render(svg) {
        Ok(p) => output(
            p.png,
            title,
            format!(
                "{title}: your SVG passed Romlens's checks; its id is {{id}}. Look at the picture before you use it, and draw it again if it does not say what you meant."
            ),
        ),
        Err(p) => ToolOutput::error(format!(
            "The SVG was not drawn:\n{}\nFix these and send it again.",
            romlens_draw::check::describe(&p)
        )),
    }
}

/// Where a DMA channel wrote, by its B-bus register.
fn b_bus_name(b: u8) -> String {
    match b {
        0x04 => "OAM".into(),
        0x18 | 0x19 => "VRAM".into(),
        0x22 => "CGRAM".into(),
        0x80 => "WRAM".into(),
        0x40..=0x43 => "the sound CPU's ports".into(),
        _ => romlens_core::model::hardware_register(0x2100 | b as u16).map_or_else(
            || format!("${:04X}", 0x2100 | b as u16),
            |r| r.name.to_string(),
        ),
    }
}

fn size(bytes: u32) -> String {
    if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

/// At most this many events in a timeline; the rest are counted.
const EVENTS: usize = 12;

fn short(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_owned()
    } else {
        format!(
            "{}…",
            s.chars().take(limit - 1).collect::<String>().trim_end()
        )
    }
}

impl Source for Draw<'_> {
    fn rom(&self) -> Option<&romlens_core::RomImage> {
        Some(&self.rom.image)
    }

    fn resolve(&self, text: &str) -> Result<u32, String> {
        (self.resolve)(text)
    }

    fn name_at(&self, address: u32) -> Option<String> {
        self.wb.label_at(address).map(|l| l.name)
    }

    fn frame(&self, frame: u64) -> Result<FrameEvents, String> {
        let rec = self.rec.as_ref().ok_or(
            "no recording is open in Romlens, so there is no frame to draw; leave frame null",
        )?;
        let info = rec.info();
        let last = info.first_frame + info.frame_count.saturating_sub(1);
        if frame < info.first_frame || frame > last {
            return Err(format!(
                "the recording has frames {} to {last}",
                info.first_frame
            ));
        }
        let src = rec.machine();
        let dma = src.dma_records(frame).map_err(|e| e.to_string())?;
        let writes = src
            .line_writes(frame)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let lines: u16 = if dma.iter().any(|r| r.scanline > 261) {
            312
        } else {
            262
        };
        let mut events = Vec::new();
        for r in &dma {
            let (line, _) = start_time(r, lines as i16, 224);
            for ch in 0..8u8 {
                if r.value & (1 << ch) == 0 {
                    continue;
                }
                let t = Transfer::of(r, ch);
                let (from, to) = (addr(t.source.as_u24()), b_bus_name(t.b_bus));
                let label = if t.mode & 0x80 != 0 {
                    format!("DMA ch {ch}: {to} → {from}, {}", size(t.bytes))
                } else {
                    format!("DMA ch {ch}: {from} → {to}, {}", size(t.bytes))
                };
                events.push(FrameEvent {
                    from: line as i32,
                    to: line as i32,
                    label,
                    dma: true,
                });
            }
        }
        // The CPU's and HDMA's writes, a register at a time: one written on
        // many lines is one stretch, a few writes are each said.
        let by = attribute(&writes, &dma);
        let mut regs: Vec<u8> = Vec::new();
        for (w, b) in writes.iter().zip(&by) {
            if b.is_none() && !regs.contains(&w.reg) {
                regs.push(w.reg);
            }
        }
        for reg in regs {
            let mine: Vec<_> = writes
                .iter()
                .zip(&by)
                .filter(|(w, b)| b.is_none() && w.reg == reg)
                .map(|(w, _)| *w)
                .collect();
            let name = romlens_core::model::hardware_register(0x2100 | reg as u16).map_or_else(
                || format!("${:04X}", 0x2100 | reg as u16),
                |r| r.name.to_string(),
            );
            if mine.len() >= 4 {
                let hdma = mine
                    .iter()
                    .all(|w| (1..=224).contains(&w.line) && w.dot >= 256);
                events.push(FrameEvent {
                    from: mine.iter().map(|w| w.line as i32).min().unwrap_or(0),
                    to: mine.iter().map(|w| w.line as i32).max().unwrap_or(0),
                    label: format!(
                        "{name}, {} writes{}",
                        mine.len(),
                        if hdma { " (HDMA)" } else { "" }
                    ),
                    dma: false,
                });
            } else {
                for w in mine {
                    let label = explain::describe(0x2100 | w.reg as u16, Some(w.value as u32), 1)
                        .map_or_else(|| format!("{name} = ${:02X}", w.value), |d| d.short());
                    events.push(FrameEvent {
                        from: w.line as i32,
                        to: w.line as i32,
                        label: short(&label, 48),
                        dma: false,
                    });
                }
            }
        }
        events.sort_by_key(|e| (e.from, !e.dma));
        let more = events.len().saturating_sub(EVENTS);
        // DMA first when some must go.
        if more > 0 {
            let mut keep: Vec<FrameEvent> = events.iter().filter(|e| e.dma).cloned().collect();
            keep.extend(events.iter().filter(|e| !e.dma).cloned());
            keep.truncate(EVENTS);
            keep.sort_by_key(|e| (e.from, !e.dma));
            events = keep;
        }
        Ok(FrameEvents {
            frame,
            lines,
            events,
            more,
        })
    }

    fn chain(&self, frame: u64, x: u32, y: u32) -> Result<Vec<ChainStep>, String> {
        let rec = self.rec.as_ref().ok_or(
            "no recording is open in Romlens, so no pixel can be traced; give steps instead",
        )?;
        let p = self
            .wb
            .pixel_provenance_blocking(rec.clone(), frame, x, y)
            .ok_or("Romlens could not trace that pixel")?;
        let part = p
            .parts
            .first()
            .ok_or("Romlens found nothing that drew that pixel")?;
        let mut steps = Vec::new();
        if let Some(h) = &part.hop {
            if let Some(placed) = &h.placed_summary {
                steps.push(ChainStep {
                    label: "In the ROM".into(),
                    detail: Some(short(placed, 80)),
                });
            }
            if let Some(c) = h.code.first() {
                steps.push(ChainStep {
                    label: "Written by the code".into(),
                    detail: Some(short(
                        &format!(
                            "{}{}",
                            addr(c.pc),
                            self.name_at(c.pc)
                                .map(|n| format!(" {n}"))
                                .unwrap_or_default()
                        ),
                        80,
                    )),
                });
            }
        }
        for l in part.links.iter().take(2) {
            steps.push(ChainStep {
                label: if l.started_at.is_some() {
                    "Copied by DMA".into()
                } else {
                    "Written".into()
                },
                detail: Some(short(&l.summary, 80)),
            });
        }
        let mut what = part.what.clone();
        if let Some(c) = what.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        steps.push(ChainStep {
            label: short(&what, 40),
            detail: None,
        });
        steps.push(ChainStep {
            label: format!("Pixel {x},{y}"),
            detail: Some(short(&p.summary, 80)),
        });
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutor::tools::RomTools;
    use romlens_tutor::agent::{Event, Mode, ToolContext, Tools};

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

    fn tools() -> RomTools {
        let rom = crate::Rom::from_bytes(
            romlens_core::fixtures::explain_lorom(),
            "explain.sfc".into(),
        )
        .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        RomTools::new(wb)
    }

    fn show(o: &ToolOutput) {
        if std::env::var_os("ROMLENS_SHOW_TOOLS").is_some() {
            println!("{}", text(o));
        }
    }

    #[test]
    fn every_kind_draws_a_picture_with_a_draw_id() {
        let t = tools();
        for (kind, spec) in [
            (
                "fields",
                json!({"register": "INIDISP", "value": "$80", "highlight": ["Forced blank"]}),
            ),
            (
                "memory_map",
                json!({"view": "bank", "bank": "$00", "marks": [{"address": "$00:FFFC", "label": null}], "highlight": null}),
            ),
            ("memory_map", json!({"view": "banks"})),
            (
                "blocks",
                json!({"preset": "machine", "highlight": ["ppu"], "edges": [{"from": "wram", "to": "oam", "label": "DMA", "both": false}]}),
            ),
            (
                "timeline",
                json!({"span": "frame", "frame": null, "marks": [{"at": 225, "label": "NMI handler"}]}),
            ),
            (
                "chain",
                json!({"steps": [{"label": "Code", "place": "$00:8000"}, {"label": "VRAM", "place": "VRAM"}]}),
            ),
        ] {
            let o = call(&t, "draw_diagram", json!({"kind": kind, "spec": spec}));
            show(&o);
            assert!(!o.is_error, "{kind}: {}", text(&o));
            let Part::Image { image } = &o.parts[1] else {
                panic!("{kind}: no picture")
            };
            assert!(image.id.starts_with("draw-"), "{}", image.id);
            assert!(
                text(&o).contains(&image.id),
                "{kind}: the id is in the words"
            );
            assert_eq!(o.pictures[0].0.id, image.id);
            assert_eq!(&o.pictures[0].1[..4], b"\x89PNG");
        }
        // The reset vector, with the fixture's label for where it points.
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "memory_map", "spec": {"bank": "$00", "marks": [{"address": "$00:FFFC"}]}}),
        );
        assert!(
            text(&o).contains("the reset vector: points to $8000"),
            "{}",
            text(&o)
        );
        // A spec as a JSON string reads the same.
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "fields", "spec": "{\"register\": \"BGMODE\"}"}),
        );
        assert!(!o.is_error, "{}", text(&o));
    }

    #[test]
    fn a_bad_spec_or_svg_says_what_is_wrong() {
        let t = tools();
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "fields", "spec": {"register": "INIDISP", "highlight": ["Speed"]}}),
        );
        assert!(
            o.is_error && text(&o).contains("Forced blank"),
            "{}",
            text(&o)
        );
        let o = call(
            &t,
            "draw_svg",
            json!({"svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 200\"><script>x</script></svg>", "title": "x"}),
        );
        assert!(
            o.is_error && text(&o).contains("<script> is not allowed"),
            "{}",
            text(&o)
        );
        let good = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200"><rect x="20" y="20" width="160" height="60" fill="#e8edf5"/><text x="30" y="56" font-size="14" fill="#1d2433">The PPU</text></svg>"##;
        let o = call(&t, "draw_svg", json!({"svg": good, "title": "A box"}));
        assert!(!o.is_error, "{}", text(&o));
        assert!(text(&o).contains("Look at the picture"));
        assert!(matches!(&o.parts[1], Part::Image { image } if image.id.starts_with("draw-")));
    }

    #[test]
    fn a_recording_places_its_frame_and_traces_a_pixel() {
        let rom = crate::Rom::from_bytes(crate::graphics::make_graphics_test_rom(), "g.sfc".into())
            .unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let t = RomTools::new(wb);
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "timeline", "spec": {"span": "frame", "frame": 8}}),
        );
        assert!(
            o.is_error && text(&o).contains("no recording"),
            "{}",
            text(&o)
        );
        t.set_recording(Some(
            RecordingSession::from_bytes(crate::graphics::make_test_recording(40)).unwrap(),
        ));
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "timeline", "spec": {"span": "frame", "frame": 8}}),
        );
        show(&o);
        assert!(!o.is_error, "{}", text(&o));
        assert!(text(&o).contains("Frame 8"), "{}", text(&o));
        let o = call(
            &t,
            "draw_diagram",
            json!({"kind": "chain", "spec": {"from": {"frame": 8, "x": 10, "y": 10}}}),
        );
        show(&o);
        assert!(!o.is_error, "{}", text(&o));
        assert!(text(&o).contains("Pixel 10,10"), "{}", text(&o));
    }
}
