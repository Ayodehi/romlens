//! How bytes reach the screen (`chain`): steps left to right, from the ROM
//! through the code or DMA that moved them to the PPU's memories and the
//! pixel. With a recording, a pixel's own chain from `provenance`; without
//! one, the tutor's steps, each place checked.

use serde_json::Value;

use super::{ChainStep, Drawing, Source, addr, list, number, text, wrap};
use crate::fonts::Font;
use crate::svg::{Shape, Svg, Text, palette};

const W: f64 = 720.0;
const PAD: f64 = 24.0;
const PER_ROW: usize = 4;
const GAP: f64 = 36.0;

/// Memories a step can name instead of an address.
const MEMORIES: &[(&str, &str)] = &[
    ("rom", "ROM: the cartridge's"),
    ("sram", "SRAM: the cartridge's"),
    ("wram", "WRAM: $7E:0000–$7F:FFFF"),
    ("vram", "VRAM: the PPU's 64 KB"),
    ("oam", "OAM: the PPU's sprite table"),
    ("cgram", "CGRAM: the PPU's 256 colours"),
    ("aram", "ARAM: the sound CPU's 64 KB"),
    ("screen", "the picture on the TV"),
];

fn steps(spec: &Value, src: &dyn Source) -> Result<(String, Vec<ChainStep>), String> {
    if let Some(from) = spec.get("from").filter(|v| v.is_object()) {
        let (Some(f), Some(x), Some(y)) =
            (number(from, "frame"), number(from, "x"), number(from, "y"))
        else {
            return Err("from needs frame, x and y".to_string());
        };
        if !(0..256).contains(&x) || !(0..240).contains(&y) || f < 0 {
            return Err(format!(
                "the pixel {x},{y} is not on the screen (x 0–255, y 0–239)"
            ));
        }
        let steps = src.chain(f as u64, x as u32, y as u32)?;
        return Ok((format!("How pixel {x},{y} of frame {f} got there"), steps));
    }
    let mut out = Vec::new();
    for s in list(spec, "steps") {
        let label = text(s, "label")
            .ok_or("each step needs a label")?
            .to_string();
        let detail = match text(s, "place") {
            None => None,
            Some(p) => match MEMORIES.iter().find(|(m, _)| m.eq_ignore_ascii_case(p)) {
                Some((_, about)) => Some(about.to_string()),
                None => {
                    let a = src.resolve(p)?;
                    Some(match src.name_at(a) {
                        Some(n) => format!("{} {n}", addr(a)),
                        None => addr(a),
                    })
                }
            },
        };
        out.push(ChainStep { label, detail });
    }
    if out.is_empty() {
        return Err("give steps (each a label, and a place: an address, a label or a memory such as VRAM), or from a pixel of a recording".to_string());
    }
    let title = text(spec, "title")
        .unwrap_or("From the ROM to the screen")
        .to_string();
    Ok((title, out))
}

pub fn draw(spec: &Value, src: &dyn Source) -> Result<Drawing, String> {
    let (title, steps) = steps(spec, src)?;
    if steps.len() > 8 {
        return Err("at most eight steps; group some".to_string());
    }
    let box_w = (W - 2.0 * PAD - GAP * (PER_ROW - 1) as f64) / PER_ROW as f64;
    let set: Vec<(Vec<String>, Vec<String>)> = steps
        .iter()
        .map(|s| {
            let l = wrap(&s.label, Font::Bold, 13.0, box_w - 16.0);
            let d = s
                .detail
                .as_deref()
                .map(|d| wrap(d, Font::Sans, 12.0, box_w - 16.0))
                .unwrap_or_default();
            (l, d)
        })
        .collect();
    for (i, (l, d)) in set.iter().enumerate() {
        if l.len() > 3 || d.len() > 4 {
            return Err(format!(
                "step {} says too much for its box; keep a step to a few words",
                i + 1
            ));
        }
    }
    let box_h = |r: &[(Vec<String>, Vec<String>)]| {
        r.iter()
            .map(|(l, d)| {
                20.0 + 17.0 * l.len() as f64
                    + 15.0 * d.len() as f64
                    + if d.is_empty() { 0.0 } else { 6.0 }
            })
            .fold(52.0, f64::max)
    };
    let rows: Vec<&[(Vec<String>, Vec<String>)]> = set.chunks(PER_ROW).collect();
    let heights: Vec<f64> = rows.iter().map(|r| box_h(r)).collect();
    let top = 78.0;
    let row_gap = 44.0;
    let h = top + heights.iter().sum::<f64>() + row_gap * (rows.len() - 1) as f64 + PAD;
    let mut svg = Svg::new(W, h);
    svg.text(PAD, 36.0, &title, Text::new(18.0, Font::Bold));
    svg.text(
        PAD,
        58.0,
        "Each step moves or turns the bytes on the way to the picture",
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    let mut y = top;
    let mut prev: Option<(f64, f64, f64)> = None;
    let mut description = format!("{title}:");
    for (r, row) in rows.iter().enumerate() {
        let bh = heights[r];
        for (k, (l, d)) in row.iter().enumerate() {
            let i = r * PER_ROW + k;
            let x = PAD + k as f64 * (box_w + GAP);
            let fill = palette::FILLS[i % palette::FILLS.len()];
            svg.rect(x, y, box_w, bh, Shape::new(fill).radius(6.0));
            let mut ty = y + 22.0;
            for line in l {
                svg.text(x + 8.0, ty, line, Text::new(13.0, Font::Bold));
                ty += 17.0;
            }
            ty += 4.0;
            for line in d {
                svg.text(
                    x + 8.0,
                    ty,
                    line,
                    Text::new(12.0, Font::Sans).colour(palette::MUTED),
                );
                ty += 15.0;
            }
            // The arrow from the step before: across a row, or down and back
            // to the start of the next.
            if let Some((px, py, ph)) = prev {
                if k == 0 {
                    let mid = py + ph + row_gap / 2.0;
                    svg.arrow(
                        &[
                            (px + box_w / 2.0, py + ph),
                            (px + box_w / 2.0, mid),
                            (x + box_w / 2.0, mid),
                            (x + box_w / 2.0, y),
                        ],
                        palette::ACCENT,
                        1.75,
                        false,
                    );
                } else {
                    let ay = y + bh / 2.0;
                    svg.arrow(
                        &[(px + box_w + 3.0, ay), (x - 3.0, ay)],
                        palette::ACCENT,
                        1.75,
                        false,
                    );
                }
            }
            prev = Some((x, y, bh));
            description.push_str(&format!(
                "\n{}. {}{}",
                i + 1,
                steps[i].label,
                steps[i]
                    .detail
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
        }
        y += bh + row_gap;
    }
    Ok(Drawing {
        svg: svg.finish(),
        title,
        description,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::kinds::draw;
    use crate::kinds::testing::{Fixture, drawn};

    #[test]
    fn steps_are_checked_and_drawn() {
        let f = Fixture::new();
        let d = drawn(
            "chain",
            json!({"from": null, "title": null, "steps": [
                {"label": "Tiles in the ROM", "place": "$00:8000"},
                {"label": "The CPU unpacks them", "place": "RESET"},
                {"label": "In WRAM", "place": "WRAM"},
                {"label": "DMA in vertical blank", "place": "$00:420B"},
                {"label": "In VRAM", "place": "vram"},
                {"label": "The PPU draws them", "place": "screen"}
            ]}),
            &f,
        );
        assert!(
            d.description
                .contains("2. The CPU unpacks them ($00:8000 RESET)"),
            "{}",
            d.description
        );
        assert!(
            d.description.contains("5. In VRAM (VRAM: the PPU's 64 KB)"),
            "{}",
            d.description
        );
        let e = draw(
            "chain",
            &json!({"steps": [{"label": "x", "place": "NOWHERE"}]}),
            &f,
        )
        .unwrap_err();
        assert!(!e.is_empty());
        let e = draw("chain", &json!({"from": {"frame": 1, "x": 3, "y": 4}}), &f).unwrap_err();
        assert!(e.contains("no recording"), "{e}");
    }
}
