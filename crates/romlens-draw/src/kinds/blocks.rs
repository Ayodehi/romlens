//! Boxes and arrows (`blocks`). The `machine` preset is Romlens's own
//! drawing of the SNES and its buses, which the tutor can only highlight
//! and add arrows to; free boxes are the tutor's, laid out by
//! `graph::layout` so nothing overlaps.

use serde_json::Value;

use romlens_core::graph::layout::{EdgeSpec, LayoutOptions, NodeSize, layout};

use super::{Drawing, Placer, fit, list, text, wrap};
use crate::fonts::{Font, measure};
use crate::svg::{Shape, Svg, Text, palette};

const W: f64 = 720.0;
const PAD: f64 = 24.0;

/// A box: where it is, its name and a line about it.
#[derive(Clone)]
struct Block {
    id: String,
    label: String,
    note: Vec<String>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    fill: &'static str,
}

impl Block {
    fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

fn b(id: &str, label: &str, note: &str, x: f64, y: f64, w: f64, fill: &'static str) -> Block {
    Block {
        id: id.to_string(),
        label: label.to_string(),
        note: wrap(note, Font::Sans, 12.0, w - 16.0),
        x,
        y,
        w,
        h: 64.0,
        fill,
    }
}

const A_BUS: f64 = 190.0;
const B_BUS: f64 = 250.0;

/// The SNES as its chips and buses: what the CPU reaches on the A bus, the
/// registers it reaches on the B bus, and the memories behind the PPU and
/// the sound CPU that only they can see.
fn machine() -> Vec<Block> {
    let [f0, f1, f2, f3, ..] = palette::FILLS;
    vec![
        b(
            "cart",
            "Cartridge",
            "the game's ROM, and SRAM if it has any",
            24.0,
            80.0,
            190.0,
            f0,
        ),
        b(
            "cpu",
            "S-CPU",
            "a 65816 core, with DMA, timers and multiply",
            265.0,
            80.0,
            190.0,
            f0,
        ),
        b("wram", "WRAM", "128 KB of work RAM", 506.0, 80.0, 190.0, f1),
        b(
            "ppu",
            "PPU1 and PPU2",
            "draw the picture for the TV, line by line",
            24.0,
            290.0,
            300.0,
            f2,
        ),
        b(
            "smp",
            "S-SMP",
            "the sound CPU, an SPC700, with its own program",
            396.0,
            290.0,
            300.0,
            f3,
        ),
        b(
            "vram",
            "VRAM",
            "64 KB: tiles, tilemaps",
            24.0,
            400.0,
            92.0,
            f2,
        ),
        b("oam", "OAM", "544 bytes: sprites", 128.0, 400.0, 92.0, f2),
        b(
            "cgram",
            "CGRAM",
            "512 bytes: colours",
            232.0,
            400.0,
            92.0,
            f2,
        ),
        b(
            "aram",
            "ARAM",
            "64 KB: code, samples",
            396.0,
            400.0,
            140.0,
            f3,
        ),
        b(
            "dsp",
            "S-DSP",
            "mixes 8 voices of sound",
            556.0,
            400.0,
            140.0,
            f3,
        ),
    ]
}

/// The machine's wires: each joins two parts (a box, or a bus `abus` or
/// `bbus`) at an x, from one y to another.
const WIRES: &[(&str, &str, f64, f64, f64)] = &[
    ("cart", "abus", 119.0, 144.0, A_BUS),
    ("cpu", "abus", 360.0, 144.0, A_BUS),
    ("wram", "abus", 601.0, 144.0, A_BUS),
    // The CPU, and its DMA, drive the B bus too; WRAM has a port on it.
    ("abus", "bbus", 360.0, A_BUS, B_BUS),
    ("wram", "bbus", 666.0, 144.0, B_BUS),
    ("ppu", "bbus", 174.0, 290.0, B_BUS),
    ("smp", "bbus", 546.0, 290.0, B_BUS),
    ("ppu", "vram", 70.0, 354.0, 400.0),
    ("ppu", "oam", 174.0, 354.0, 400.0),
    ("ppu", "cgram", 278.0, 354.0, 400.0),
    ("smp", "aram", 466.0, 354.0, 400.0),
    ("smp", "dsp", 626.0, 354.0, 400.0),
];

/// Each bus's line, its label and where the label goes: between two of its
/// wires, above the line.
const BUSES: &[(&str, f64, f64, f64, &str, f64)] = &[
    (
        "abus",
        119.0,
        601.0,
        A_BUS,
        "A bus: 24-bit addresses",
        132.0,
    ),
    (
        "bbus",
        174.0,
        666.0,
        B_BUS,
        "B bus: registers $2100–$21FF",
        184.0,
    ),
];

/// The parts a route may pass through on its way.
const THROUGH: &[&str] = &["abus", "bbus", "ppu", "smp"];

/// The wires from one part to another, fewest first, passing only through
/// the buses and the chips that own a memory.
fn route(from: &str, to: &str) -> Option<Vec<usize>> {
    let mut seen = vec![from.to_string()];
    let mut queue: std::collections::VecDeque<(String, Vec<usize>)> =
        [(from.to_string(), vec![])].into();
    while let Some((at, path)) = queue.pop_front() {
        if at == to {
            return Some(path);
        }
        if at != from && !THROUGH.contains(&at.as_str()) {
            continue;
        }
        for (i, w) in WIRES.iter().enumerate() {
            // WRAM's port on the B bus is for the CPU reading it a byte at
            // a time; DMA from WRAM reads it over the A bus.
            if w.0 == "wram"
                && w.1 == "bbus"
                && !(from == "wram" && to == "bbus" || from == "bbus" && to == "wram")
            {
                continue;
            }
            let next = if w.0 == at {
                w.1
            } else if w.1 == at {
                w.0
            } else {
                continue;
            };
            if seen.iter().any(|s| s == next) {
                continue;
            }
            seen.push(next.to_string());
            let mut p = path.clone();
            p.push(i);
            queue.push_back((next.to_string(), p));
        }
    }
    None
}

/// A route as lines to draw: up and down each wire, and along a bus from
/// one wire to the next. A chip passed through breaks the line, so nothing
/// is drawn over its words.
fn route_lines(from: &str, wires: &[usize]) -> Vec<Vec<(f64, f64)>> {
    let mut lines: Vec<Vec<(f64, f64)>> = vec![vec![]];
    let mut at = from.to_string();
    for &i in wires {
        let (a, b, x, ya, yb) = WIRES[i];
        let (y0, y1, next) = if a == at { (ya, yb, b) } else { (yb, ya, a) };
        let on_bus = at == "abus" || at == "bbus";
        let cur = lines.last_mut().unwrap();
        if on_bus
            && let Some(&(px, py)) = cur.last()
            && (px - x).abs() > 0.1
        {
            // Along the bus to this wire.
            cur.push((x, py));
        }
        let cur = lines.last_mut().unwrap();
        if cur.last() != Some(&(x, y0)) {
            // Through a chip: the line stops at one side and starts again
            // at the other.
            if !cur.is_empty() {
                lines.push(vec![]);
            }
            lines.last_mut().unwrap().push((x, y0));
        }
        lines.last_mut().unwrap().push((x, y1));
        at = next.to_string();
    }
    lines.retain(|l| l.len() > 1);
    lines
}

fn machine_wires(svg: &mut Svg, highlight: &[String]) {
    for &(_, _, x, y0, y1) in WIRES {
        svg.line(x, y0, x, y1, palette::LINE, 2.0);
    }
    for &(id, x0, x1, y, label, lx) in BUSES {
        let lit = highlight.iter().any(|h| h == id);
        let (colour, width) = if lit {
            (palette::ACCENT, 4.0)
        } else {
            (palette::INK, 3.0)
        };
        svg.line(x0, y, x1, y, colour, width);
        let font = if lit { Font::Bold } else { Font::Sans };
        svg.text(
            lx,
            y - 8.0,
            label,
            Text::new(12.0, font).colour(if lit { palette::ACCENT } else { palette::INK }),
        );
    }
    svg.text(
        556.0,
        274.0,
        "ports $2140–$2143",
        Text::new(12.0, Font::Mono).colour(palette::MUTED),
    );
}

fn draw_block(svg: &mut Svg, b: &Block, lit: bool) {
    let shape = if lit {
        Shape::new(b.fill).stroke(palette::ACCENT, 2.5).radius(6.0)
    } else {
        Shape::new(b.fill).radius(6.0)
    };
    svg.rect(b.x, b.y, b.w, b.h, shape);
    let (cx, _) = b.centre();
    let lines = 1 + b.note.len();
    let block_h = 18.0 + 16.0 * (lines - 1) as f64;
    let mut y = b.y + (b.h - block_h) / 2.0 + 13.0;
    let font = Font::Bold;
    svg.text(
        cx,
        y,
        &fit(&b.label, font, 14.0, b.w - 12.0),
        Text::new(14.0, font)
            .colour(if lit { palette::ACCENT } else { palette::INK })
            .middle(),
    );
    for n in &b.note {
        y += 16.0;
        svg.text(
            cx,
            y,
            n,
            Text::new(12.0, Font::Sans).colour(palette::MUTED).middle(),
        );
    }
}

struct Arrow {
    from: String,
    to: String,
    label: Option<String>,
    both: bool,
}

fn arrows(spec: &Value, ids: &[String]) -> Result<Vec<Arrow>, String> {
    let mut out = Vec::new();
    for e in list(spec, "edges") {
        let (Some(from), Some(to)) = (text(e, "from"), text(e, "to")) else {
            return Err("each edge needs from and to".to_string());
        };
        for id in [from, to] {
            if !ids.iter().any(|i| i == id) {
                return Err(format!(
                    "no box with the id {id}; the ids are {}",
                    ids.join(", ")
                ));
            }
        }
        out.push(Arrow {
            from: from.to_string(),
            to: to.to_string(),
            label: text(e, "label").map(str::to_string),
            both: e.get("both").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    if out.len() > 24 {
        return Err("at most 24 edges".to_string());
    }
    Ok(out)
}

/// An edge's label on the paper, near the middle of its line and clear of
/// everything else, or in the list under the picture if there is no room.
fn place_label(
    svg: &mut Svg,
    placer: &mut Placer,
    points: &[(f64, f64)],
    label: &str,
    legend: &mut Vec<String>,
) {
    let t = Text::new(12.0, Font::Sans);
    let (w, h) = (measure(label, Font::Sans, 12.0).0 + 8.0, 18.0);
    let mut spots = Vec::new();
    for seg in points.windows(2) {
        for f in [0.5, 0.35, 0.65, 0.2, 0.8] {
            let (x, y) = (
                seg[0].0 + (seg[1].0 - seg[0].0) * f,
                seg[0].1 + (seg[1].1 - seg[0].1) * f,
            );
            for (dx, dy) in [
                (0.0, 0.0),
                (0.0, -12.0),
                (0.0, 12.0),
                (w / 2.0 + 6.0, 0.0),
                (-w / 2.0 - 6.0, 0.0),
            ] {
                spots.push((x - w / 2.0 + dx, y - h / 2.0 + dy));
            }
        }
    }
    spots.retain(|&(x, y)| x > 2.0 && x + w < W - 2.0 && y > 2.0);
    match placer.place(&spots, w, h) {
        Some((x, y)) => {
            svg.rect(
                x,
                y,
                w,
                h,
                Shape::new(palette::PAPER)
                    .stroke(palette::PAPER, 0.0)
                    .radius(3.0),
            );
            svg.text(x + w / 2.0, y + 13.0, label, t.middle());
        }
        None => legend.push(label.to_string()),
    }
}

pub fn draw(spec: &Value) -> Result<Drawing, String> {
    let highlight: Vec<String> = list(spec, "highlight")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    match text(spec, "preset") {
        Some("machine") => draw_machine(spec, &highlight),
        Some(p) => Err(format!("the only preset is machine, not {p}")),
        None => draw_free(spec, &highlight),
    }
}

fn draw_machine(spec: &Value, highlight: &[String]) -> Result<Drawing, String> {
    let blocks = machine();
    let mut ids: Vec<String> = blocks.iter().map(|b| b.id.clone()).collect();
    ids.extend(BUSES.iter().map(|b| b.0.to_string()));
    for h in highlight {
        if !ids.contains(h) {
            return Err(format!(
                "the machine has no part {h}; its parts are {}",
                ids.join(", ")
            ));
        }
    }
    if !list(spec, "nodes").is_empty() {
        return Err("the machine preset takes no nodes; draw edges between its parts".to_string());
    }
    let arrows = arrows(spec, &ids)?;
    if arrows.len() > 6 {
        return Err("at most six edges on the machine".to_string());
    }
    let body = 490.0;
    let legend: Vec<&Arrow> = arrows.iter().collect();
    let h = body
        + if legend.is_empty() {
            0.0
        } else {
            8.0 + 24.0 * legend.len() as f64
        };
    let mut svg = Svg::new(W, h);
    svg.text(PAD, 36.0, "Inside the SNES", Text::new(18.0, Font::Bold));
    svg.text(
        PAD,
        58.0,
        "The S-CPU runs the game and reaches everything else over two buses",
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    machine_wires(&mut svg, highlight);
    let mut description = "The SNES: the S-CPU (a 65816 with DMA) reaches the cartridge's ROM and SRAM and the 128 KB of WRAM over the A bus (24-bit addresses), and the PPUs, the sound CPU's four ports ($2140–$2143) and a port into WRAM over the B bus (registers $2100–$21FF). VRAM (64 KB), OAM (544 bytes) and CGRAM (512 bytes) belong to the PPUs, which draw the picture; the S-SMP (an SPC700) runs from its own 64 KB of ARAM, and the S-DSP mixes eight voices.".to_string();
    if !highlight.is_empty() {
        description.push_str(&format!(" Highlighted: {}.", highlight.join(", ")));
    }
    let name = |id: &str| {
        blocks
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.label.clone())
            .or_else(|| {
                BUSES
                    .iter()
                    .find(|b| b.0 == id)
                    .map(|b| b.4.split(':').next().unwrap_or(b.4).to_string())
            })
            .unwrap_or_default()
    };
    // Each arrow along the wires it would travel, numbered at its end.
    let mut ends = Vec::new();
    for (n, a) in arrows.iter().enumerate() {
        let wires = route(&a.from, &a.to)
            .ok_or_else(|| format!("nothing joins {} and {}", a.from, a.to))?;
        let lines = route_lines(&a.from, &wires);
        let k = lines.len();
        for (i, l) in lines.iter().enumerate() {
            if i + 1 == k {
                svg.arrow(l, palette::ACCENT, 3.0, false);
            } else {
                svg.arrow_less(l, palette::ACCENT, 3.0);
            }
        }
        if a.both
            && let Some(first) = lines.first()
        {
            let rev: Vec<(f64, f64)> = first.iter().rev().copied().collect();
            svg.arrow(&rev, palette::ACCENT, 3.0, false);
        }
        if let Some(&end) = lines.last().and_then(|l| l.last()) {
            ends.push((n, end));
        }
        description.push_str(&format!(
            "
{}. {} {} {}{}",
            n + 1,
            name(&a.from),
            if a.both { "↔" } else { "→" },
            name(&a.to),
            a.label
                .as_ref()
                .map(|l| format!(": {l}"))
                .unwrap_or_default()
        ));
    }
    for bl in &blocks {
        draw_block(&mut svg, bl, highlight.contains(&bl.id));
    }
    let mut placer = Placer::default();
    for bl in &blocks {
        placer.take(bl.x, bl.y, bl.w, bl.h);
    }
    for &(_, _, _, y, label, lx) in BUSES {
        placer.take(lx, y - 22.0, measure(label, Font::Bold, 12.0).0, 16.0);
    }
    placer.take(552.0, 261.0, 140.0, 18.0);
    for (n, (x, y)) in ends {
        let spots: Vec<(f64, f64)> = [
            (10.0, -30.0),
            (10.0, -52.0),
            (-30.0, -30.0),
            (10.0, -8.0),
            (-30.0, -52.0),
            (10.0, 6.0),
        ]
        .iter()
        .map(|(dx, dy)| (x + dx, y + dy))
        .collect();
        if let Some((bx, by)) = placer.place(&spots, 20.0, 20.0) {
            badge(&mut svg, bx + 10.0, by + 10.0, n + 1);
        }
    }
    for (i, a) in legend.iter().enumerate() {
        let y = body + 16.0 + i as f64 * 24.0;
        badge(&mut svg, PAD + 10.0, y, i + 1);
        let text = format!(
            "{} {} {}{}",
            name(&a.from),
            if a.both { "↔" } else { "→" },
            name(&a.to),
            a.label
                .as_ref()
                .map(|l| format!(": {l}"))
                .unwrap_or_default()
        );
        svg.text(
            PAD + 28.0,
            y + 5.0,
            &fit(&text, Font::Sans, 13.0, W - 2.0 * PAD - 28.0),
            Text::new(13.0, Font::Sans),
        );
    }
    Ok(Drawing {
        svg: svg.finish(),
        title: "Inside the SNES".to_string(),
        description,
    })
}

/// A number in a small accent disc.
fn badge(svg: &mut Svg, cx: f64, cy: f64, n: usize) {
    svg.circle(cx, cy, 10.0, palette::ACCENT);
    svg.text(
        cx,
        cy + 4.0,
        &n.to_string(),
        Text::new(12.0, Font::Bold)
            .colour(palette::ON_ACCENT)
            .middle(),
    );
}

/// Labels that found no room by their lines, listed under the picture.
fn with_legend(mut svg: Svg, legend: &[String]) -> String {
    let base = svg.height();
    svg.grow(if legend.is_empty() {
        0.0
    } else {
        12.0 + 20.0 * legend.len() as f64
    });
    for (i, l) in legend.iter().enumerate() {
        svg.text(
            PAD,
            base + 18.0 + i as f64 * 20.0,
            &fit(l, Font::Sans, 12.0, W - 2.0 * PAD),
            Text::new(12.0, Font::Sans),
        );
    }
    svg.finish()
}

fn draw_free(spec: &Value, highlight: &[String]) -> Result<Drawing, String> {
    let nodes = list(spec, "nodes");
    if nodes.is_empty() {
        return Err("give nodes (each an id and a label), or preset machine".to_string());
    }
    if nodes.len() > 16 {
        return Err("at most 16 boxes".to_string());
    }
    let mut blocks: Vec<Block> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let id = text(n, "id").ok_or("each node needs an id")?.to_string();
        if blocks.iter().any(|b| b.id == id) {
            return Err(format!("two nodes have the id {id}"));
        }
        let label = text(n, "label").unwrap_or(&id).to_string();
        let note = text(n, "note")
            .map(|t| wrap(t, Font::Sans, 12.0, 168.0))
            .unwrap_or_default();
        if note.len() > 4 {
            return Err(format!(
                "{id}'s note is too long for its box; keep it to a line or two"
            ));
        }
        let lw = measure(&label, Font::Bold, 14.0).0.min(260.0);
        let nw = note
            .iter()
            .map(|l| measure(l, Font::Sans, 12.0).0)
            .fold(0.0, f64::max);
        let w = (lw.max(nw) + 28.0).max(96.0);
        let h = 30.0 + 16.0 * note.len() as f64 + if note.is_empty() { 8.0 } else { 6.0 };
        blocks.push(Block {
            id,
            label,
            note,
            x: 0.0,
            y: 0.0,
            w,
            h,
            fill: palette::FILLS[i % palette::FILLS.len()],
        });
    }
    let ids: Vec<String> = blocks.iter().map(|b| b.id.clone()).collect();
    for h in highlight {
        if !ids.contains(h) {
            return Err(format!("no box with the id {h} to highlight"));
        }
    }
    let arrows = arrows(spec, &ids)?;
    let asked_right = text(spec, "direction") == Some("right");
    let edges: Vec<EdgeSpec> = arrows
        .iter()
        .map(|a| EdgeSpec {
            from: ids.iter().position(|i| *i == a.from).unwrap(),
            to: ids.iter().position(|i| *i == a.to).unwrap(),
            back: false,
        })
        .collect();
    let opts = LayoutOptions {
        node_gap: 28.0,
        layer_gap: 48.0,
        edge_gap: 14.0,
        sweeps: 16,
    };
    let lay = |right: bool| {
        let sizes: Vec<NodeSize> = blocks
            .iter()
            .map(|b| {
                if right {
                    NodeSize {
                        width: b.h,
                        height: b.w,
                    }
                } else {
                    NodeSize {
                        width: b.w,
                        height: b.h,
                    }
                }
            })
            .collect();
        let l = layout(&sizes, &edges, &opts);
        let across = if right { l.height } else { l.width };
        (l, across)
    };
    // The direction asked for, or the other when it is too wide.
    let room = W - 2.0 * PAD;
    let (mut right, (mut l, mut across)) = (asked_right, lay(asked_right));
    let mut turned = false;
    if across > room {
        let other = lay(!asked_right);
        if other.1 <= room {
            (right, l, across, turned) = (!asked_right, other.0, other.1, true);
        }
    }
    if across > room {
        return Err(format!(
            "the boxes need {across:.0} points side by side and the picture has {room:.0}; use fewer boxes in a row or shorter notes"
        ));
    }
    let flip = |x: f64, y: f64| if right { (y, x) } else { (x, y) };
    let (lw, lh) = flip(l.width, l.height);
    let title = text(spec, "title").unwrap_or("");
    let top = if title.is_empty() { PAD } else { 64.0 };
    let x0 = (W - lw) / 2.0;
    for (b, p) in blocks.iter_mut().zip(&l.nodes) {
        let (x, y) = flip(p.x, p.y);
        b.x = x0 + x;
        b.y = top + y;
    }
    let h = top + lh + PAD;
    let mut svg = Svg::new(W, h.max(120.0));
    if !title.is_empty() {
        svg.text(
            PAD,
            38.0,
            &fit(title, Font::Bold, 18.0, W - 2.0 * PAD),
            Text::new(18.0, Font::Bold),
        );
    }
    let mut placer = Placer::default();
    for b in &blocks {
        placer.take(b.x, b.y, b.w, b.h);
    }
    let mut lines = Vec::new();
    for (a, e) in arrows.iter().zip(&l.edges) {
        let pts: Vec<(f64, f64)> = e
            .points
            .iter()
            .map(|p| {
                let (x, y) = flip(p.x, p.y);
                (x0 + x, top + y)
            })
            .collect();
        let lit = highlight.contains(&a.from) && highlight.contains(&a.to);
        svg.arrow(
            &pts,
            if lit { palette::ACCENT } else { palette::LINE },
            1.75,
            a.both,
        );
        lines.push(pts);
    }
    for b in &blocks {
        draw_block(&mut svg, b, highlight.contains(&b.id));
    }
    let mut legend = Vec::new();
    let mut description = if title.is_empty() {
        "Boxes and arrows".to_string()
    } else {
        title.to_string()
    };
    if turned {
        description.push_str(if right {
            " (drawn across: too wide downwards)"
        } else {
            " (drawn downwards: too wide across)"
        });
    }
    description.push(':');
    for b in &blocks {
        description.push_str(&format!(
            "\n- {}{}",
            b.label,
            if b.note.is_empty() {
                String::new()
            } else {
                format!(": {}", b.note.join(" "))
            }
        ));
    }
    for (a, pts) in arrows.iter().zip(&lines) {
        if let Some(label) = &a.label {
            place_label(&mut svg, &mut placer, pts, label, &mut legend);
        }
        let name = |id: &str| {
            blocks
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.label.clone())
                .unwrap_or_default()
        };
        description.push_str(&format!(
            "\n{} {} {}{}",
            name(&a.from),
            if a.both { "↔" } else { "→" },
            name(&a.to),
            a.label
                .as_ref()
                .map(|l| format!(": {l}"))
                .unwrap_or_default()
        ));
    }
    Ok(Drawing {
        svg: with_legend(svg, &legend),
        title: if title.is_empty() {
            "Boxes and arrows".to_string()
        } else {
            title.to_string()
        },
        description,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::kinds::draw;
    use crate::kinds::testing::{Fixture, drawn};

    #[test]
    fn the_machine_has_its_parts() {
        let f = Fixture::new();
        let d = drawn(
            "blocks",
            json!({"preset": "machine", "highlight": ["ppu", "bbus"], "edges": null}),
            &f,
        );
        for part in [
            "S-CPU",
            "WRAM",
            "PPU1 and PPU2",
            "VRAM",
            "OAM",
            "CGRAM",
            "S-SMP",
            "ARAM",
            "S-DSP",
            "A bus",
            "B bus",
        ] {
            assert!(d.svg.contains(part), "{part}");
        }
        assert!(d.description.contains("$2140–$2143"));
        // Arrows between parts, with labels.
        let d = drawn(
            "blocks",
            json!({"preset": "machine", "edges": [
                {"from": "wram", "to": "oam", "label": "DMA copies the sprite table", "both": false},
                {"from": "cart", "to": "vram", "label": "DMA copies tiles", "both": false},
                {"from": "cpu", "to": "smp", "label": "the upload", "both": true}
            ]}),
            &f,
        );
        assert!(
            d.description
                .contains("1. WRAM → OAM: DMA copies the sprite table")
        );
        let e = draw(
            "blocks",
            &json!({"preset": "machine", "highlight": ["gpu"]}),
            &f,
        )
        .unwrap_err();
        assert!(e.contains("ppu"), "{e}");
    }

    #[test]
    fn free_boxes_are_laid_out() {
        let f = Fixture::new();
        let spec = json!({
            "title": "From the cartridge to the screen",
            "nodes": [
                {"id": "rom", "label": "ROM", "note": "compressed tiles"},
                {"id": "wram", "label": "WRAM", "note": "unpacked by the CPU"},
                {"id": "vram", "label": "VRAM", "note": null},
                {"id": "ppu", "label": "PPU", "note": "reads VRAM every line"},
                {"id": "tv", "label": "The screen", "note": null}
            ],
            "edges": [
                {"from": "rom", "to": "wram", "label": "decompress"},
                {"from": "wram", "to": "vram", "label": "DMA in vblank"},
                {"from": "vram", "to": "ppu", "label": null},
                {"from": "ppu", "to": "tv", "label": null}
            ],
            "direction": "right",
            "highlight": ["vram"]
        });
        let d = drawn("blocks", spec.clone(), &f);
        assert!(
            d.description.contains("WRAM → VRAM: DMA in vblank"),
            "{}",
            d.description
        );
        let mut down = spec;
        down["direction"] = json!("down");
        drawn("blocks", down, &f);
        let e = draw(
            "blocks",
            &json!({"nodes": [{"id": "a"}], "edges": [{"from": "a", "to": "b"}]}),
            &f,
        )
        .unwrap_err();
        assert!(e.contains("no box with the id b"), "{e}");
    }
}
