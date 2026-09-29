//! Timelines (`timeline`): a frame from the NMI through vertical blank and
//! the picture's lines, or one scanline's dots and horizontal blank. With a
//! recording's frame, its DMA and register writes are placed on their lines.
//! From `timing` and the recording.

use serde_json::Value;

use romlens_core::timing;

use super::{Drawing, FrameEvent, Placer, Source, fit, list, number, text};
use crate::fonts::{Font, measure};
use crate::svg::{Anchor, Shape, Svg, Text, palette};

const W: f64 = 720.0;
const PAD: f64 = 24.0;
const BAR_X: f64 = 48.0;
const BAR_W: f64 = 624.0;
const BAR_Y: f64 = 104.0;
const BAR_H: f64 = 40.0;

struct Band {
    from: f64,
    to: f64,
    label: &'static str,
    fill: &'static str,
}

struct Scale {
    first: f64,
    count: f64,
}

impl Scale {
    fn x(&self, at: f64) -> f64 {
        BAR_X + (at - self.first) / self.count * BAR_W
    }
}

pub fn draw(spec: &Value, src: &dyn Source) -> Result<Drawing, String> {
    match text(spec, "span").unwrap_or("frame") {
        "frame" => frame(spec, src),
        "line" => line(spec),
        s => Err(format!("span is frame or line, not {s}")),
    }
}

/// The bar, its bands and its axis.
fn bar(svg: &mut Svg, scale: &Scale, bands: &[Band], ticks: &[(f64, String)], placer: &mut Placer) {
    for b in bands {
        let (x0, x1) = (scale.x(b.from), scale.x(b.to + 1.0));
        svg.rect(x0, BAR_Y, x1 - x0, BAR_H, Shape::new(b.fill).radius(0.0));
        let t = Text::new(13.0, Font::Sans);
        if measure(b.label, Font::Sans, 13.0).0 + 12.0 <= x1 - x0 {
            svg.text(
                (x0 + x1) / 2.0,
                t.centred(BAR_Y + BAR_H / 2.0),
                b.label,
                t.middle(),
            );
        } else {
            let w = measure(b.label, Font::Sans, 13.0).0;
            let spots: Vec<(f64, f64)> = (0..30)
                .map(|k| (x0 + k as f64 * 8.0, BAR_Y - 22.0))
                .filter(|(x, _)| x + w < W - PAD)
                .collect();
            if let Some((x, y)) = placer.place(&spots, w, 16.0) {
                svg.text(x, y + 14.0, b.label, t);
            }
        }
    }
    svg.rect(
        BAR_X,
        BAR_Y,
        BAR_W,
        BAR_H,
        Shape::new("none").stroke(palette::INK, 1.25).radius(0.0),
    );
    for (at, label) in ticks {
        let x = scale.x(*at);
        svg.line(x, BAR_Y + BAR_H, x, BAR_Y + BAR_H + 5.0, palette::INK, 1.0);
        svg.text(
            x,
            BAR_Y + BAR_H + 18.0,
            label,
            Text::new(11.0, Font::Mono).colour(palette::MUTED).middle(),
        );
        let w = measure(label, Font::Mono, 11.0).0;
        placer.take(x - w / 2.0, BAR_Y + BAR_H + 6.0, w, 14.0);
    }
}

/// Events under the bar, in lanes, each label as near its moment as the
/// others allow.
fn events(svg: &mut Svg, scale: &Scale, events: &[FrameEvent], placer: &mut Placer) -> f64 {
    let lane0 = BAR_Y + BAR_H + 34.0;
    let mut bottom = lane0;
    // Right to left: each label starts at its own line and runs right, so
    // the lines of the events further left pass nothing placed before them.
    let mut order: Vec<&FrameEvent> = events.iter().collect();
    order.sort_by_key(|e| std::cmp::Reverse(e.from));
    for e in order {
        let (x0, x1) = (scale.x(e.from as f64), scale.x(e.to as f64 + 1.0));
        let label = fit(&e.label, Font::Sans, 12.0, 300.0);
        let w = measure(&label, Font::Sans, 12.0).0;
        let x = if x0 + w > W - PAD {
            (W - PAD - w).max(PAD)
        } else {
            x0
        };
        let spots: Vec<(f64, f64)> = (0..12).map(|k| (x, lane0 + k as f64 * 30.0)).collect();
        let Some((lx, ly)) = placer.place(&spots, w + 6.0, 26.0) else {
            continue;
        };
        let colour = if e.dma { palette::ACCENT } else { palette::INK };
        // Its moment, or its stretch of lines, on the bar's edge.
        if e.to > e.from {
            svg.line(x0, ly + 2.0, x1.max(x0 + 2.0), ly + 2.0, colour, 3.0);
        }
        // A tick on the bar, and a dashed line from under the axis's
        // numbers down to the label.
        svg.line(x0, BAR_Y + BAR_H - 6.0, x0, BAR_Y + BAR_H, colour, 2.0);
        svg.dashed(x0, BAR_Y + BAR_H + 22.0, x0, ly + 2.0, palette::LINE);
        svg.text(
            lx,
            ly + 18.0,
            &label,
            Text::new(12.0, Font::Sans).colour(colour),
        );
        bottom = bottom.max(ly + 26.0);
    }
    bottom
}

fn marks(spec: &Value, at: impl Fn(i64) -> Result<i32, String>) -> Result<Vec<FrameEvent>, String> {
    let mut out = Vec::new();
    for m in list(spec, "marks") {
        let n = number(m, "at").ok_or("each mark needs at: a line (or a dot, for a line)")?;
        let line = at(n)?;
        out.push(FrameEvent {
            from: line,
            to: line,
            label: text(m, "label").unwrap_or("").to_string(),
            dma: false,
        });
    }
    if out.len() > 12 {
        return Err("at most twelve marks".to_string());
    }
    Ok(out)
}

fn frame(spec: &Value, src: &dyn Source) -> Result<Drawing, String> {
    let recorded = match number(spec, "frame") {
        Some(f) if f >= 0 => Some(src.frame(f as u64)?),
        Some(f) => return Err(format!("frame {f} is not a frame")),
        None => None,
    };
    let lines = recorded.as_ref().map_or(timing::LINES_NTSC, |r| r.lines) as i32;
    let visible = timing::VISIBLE as i32;
    let vblank = lines - (visible + 1);
    // The frame in the game's order: the NMI, vertical blank, then the
    // picture. The frame's own numbering puts the blank before line 0 as
    // negative lines.
    let scale = Scale {
        first: -vblank as f64,
        count: lines as f64,
    };
    let hw = |n: i64| -> Result<i32, String> {
        if n < 0 || n >= lines as i64 {
            return Err(format!("a line is 0 to {}, not {n}", lines - 1));
        }
        Ok(if n as i32 > visible {
            n as i32 - lines
        } else {
            n as i32
        })
    };
    let mut evs = marks(spec, hw)?;
    let mut more = 0;
    if let Some(r) = &recorded {
        evs.extend(r.events.iter().cloned());
        more = r.more;
    }
    let bands = [
        Band {
            from: -vblank as f64,
            to: 0.0,
            label: "Vertical blank",
            fill: palette::FILLS[2],
        },
        Band {
            from: 1.0,
            to: visible as f64,
            label: "The PPU draws the picture, line by line",
            fill: palette::FILLS[0],
        },
    ];
    let ticks = [
        (-vblank as f64, format!("{}", visible + 1)),
        (0.0, "0".to_string()),
        (1.0 + (visible / 2) as f64, format!("{}", 1 + visible / 2)),
        (visible as f64, format!("{visible}")),
    ];
    let mut svg = Svg::new(W, 400.0);
    let title = match &recorded {
        Some(r) => format!("Frame {}", r.frame),
        None => "One frame".to_string(),
    };
    svg.text(PAD, 36.0, &title, Text::new(18.0, Font::Bold));
    svg.text(
        PAD,
        58.0,
        &format!(
            "{lines} lines, about 1/{} of a second: the NMI, vertical blank, then the picture",
            if lines == 312 { 50 } else { 60 }
        ),
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    let mut placer = Placer::default();
    // The NMI at the start of vertical blank.
    let nx = scale.x(-vblank as f64);
    svg.text(
        nx,
        BAR_Y - 8.0,
        "NMI",
        Text::new(12.0, Font::Bold).colour(palette::ACCENT),
    );
    placer.take(
        nx,
        BAR_Y - 22.0,
        measure("NMI", Font::Bold, 12.0).0 + 8.0,
        16.0,
    );
    bar(&mut svg, &scale, &bands, &ticks, &mut placer);
    let bottom = events(&mut svg, &scale, &evs, &mut placer);
    let mut h = bottom + 20.0;
    if more > 0 {
        svg.text(
            PAD,
            bottom + 16.0,
            &format!("and {more} more not drawn"),
            Text::new(12.0, Font::Sans).colour(palette::MUTED),
        );
        h += 20.0;
    }
    svg.grow(h - svg.height());
    let mut description = format!(
        "{title}: {lines} lines. The NMI fires at line {} as vertical blank starts; lines {}–{} and 0 are vertical blank, when the CPU may write VRAM, OAM and CGRAM; lines 1–{visible} are the picture.",
        visible + 1,
        visible + 1,
        lines - 1
    );
    for e in &evs {
        let line = |l: i32| if l < 0 { l + lines } else { l };
        description.push_str(&format!(
            "\n- {}{}: {}",
            if e.from == e.to {
                format!("line {}", line(e.from))
            } else {
                format!("lines {}–{}", line(e.from), line(e.to))
            },
            if e.dma { " (DMA)" } else { "" },
            e.label
        ));
    }
    if more > 0 {
        description.push_str(&format!("\n- and {more} more not drawn"));
    }
    Ok(Drawing {
        svg: svg.finish(),
        title,
        description,
    })
}

fn line(spec: &Value) -> Result<Drawing, String> {
    let dots = timing::DOTS as i64;
    let evs = marks(spec, |n| {
        if n < 0 || n >= dots {
            Err(format!("a dot is 0 to {}, not {n}", dots - 1))
        } else {
            Ok(n as i32)
        }
    })?;
    let scale = Scale {
        first: 0.0,
        count: dots as f64,
    };
    let (first, last, hdma) = (
        timing::FIRST_DOT as f64,
        timing::LAST_DOT as f64,
        timing::HDMA_DOT as f64,
    );
    let bands = [
        Band {
            from: 0.0,
            to: first - 1.0,
            label: "Border",
            fill: palette::QUIET_FILL,
        },
        Band {
            from: first,
            to: last,
            label: "The line's 256 pixels",
            fill: palette::FILLS[0],
        },
        Band {
            from: last + 1.0,
            to: dots as f64 - 1.0,
            label: "Horizontal blank",
            fill: palette::FILLS[2],
        },
    ];
    let ticks = [
        (0.0, "0".to_string()),
        (first, format!("{first}")),
        (last, format!("{last}")),
        (dots as f64 - 1.0, format!("{}", dots - 1)),
    ];
    let mut svg = Svg::new(W, 300.0);
    svg.text(PAD, 36.0, "One scanline", Text::new(18.0, Font::Bold));
    svg.text(
        PAD,
        58.0,
        &format!(
            "{dots} dots, {} master cycles, about 63.5 µs; dots are approximate",
            timing::MASTER_PER_LINE
        ),
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    let mut placer = Placer::default();
    let hx = scale.x(hdma);
    svg.text(
        hx,
        BAR_Y - 8.0,
        "HDMA",
        Text::new(12.0, Font::Bold)
            .colour(palette::ACCENT)
            .anchor(Anchor::Start),
    );
    placer.take(hx, BAR_Y - 22.0, 40.0, 16.0);
    bar(&mut svg, &scale, &bands, &ticks, &mut placer);
    let bottom = events(&mut svg, &scale, &evs, &mut placer);
    svg.grow(bottom + 20.0 - svg.height());
    let mut description = format!(
        "One scanline: {dots} dots ({} master cycles). The picture's 256 pixels are drawn in dots {first}–{last}, about; horizontal blank follows, and HDMA runs at its start, about dot {hdma}.",
        timing::MASTER_PER_LINE
    );
    for e in &evs {
        description.push_str(&format!("\n- dot {}: {}", e.from, e.label));
    }
    Ok(Drawing {
        svg: svg.finish(),
        title: "One scanline".to_string(),
        description,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::{FrameEvent, FrameEvents, Source};
    use crate::kinds::draw;
    use crate::kinds::testing::{Fixture, drawn};

    struct Recorded(Fixture);

    impl Source for Recorded {
        fn rom(&self) -> Option<&romlens_core::RomImage> {
            self.0.rom()
        }
        fn resolve(&self, text: &str) -> Result<u32, String> {
            self.0.resolve(text)
        }
        fn name_at(&self, address: u32) -> Option<String> {
            self.0.name_at(address)
        }
        fn frame(&self, frame: u64) -> Result<FrameEvents, String> {
            Ok(FrameEvents {
                frame,
                lines: 262,
                events: vec![
                    FrameEvent {
                        from: -36,
                        to: -36,
                        label: "DMA ch 0: $7E:0200 → OAM, 544 bytes".into(),
                        dma: true,
                    },
                    FrameEvent {
                        from: -30,
                        to: -30,
                        label: "DMA ch 1: $7F:0000 → VRAM $6000, 2 KB".into(),
                        dma: true,
                    },
                    FrameEvent {
                        from: -2,
                        to: -2,
                        label: "INIDISP = $0F: display on".into(),
                        dma: false,
                    },
                    FrameEvent {
                        from: 1,
                        to: 224,
                        label: "BG1HOFS, 224 writes (HDMA)".into(),
                        dma: false,
                    },
                ],
                more: 3,
            })
        }
    }

    #[test]
    fn a_frame_and_a_line() {
        let f = Fixture::new();
        let d = drawn(
            "timeline",
            json!({"span": "frame", "frame": null, "marks": [{"at": 225, "label": "The game's NMI handler runs"}, {"at": 100, "label": "Line 100"}]}),
            &f,
        );
        assert!(
            d.description.contains("The NMI fires at line 225"),
            "{}",
            d.description
        );
        assert!(
            d.description
                .contains("line 225: The game's NMI handler runs"),
            "{}",
            d.description
        );
        let d = drawn(
            "timeline",
            json!({"span": "line", "marks": [{"at": 278, "label": "HDMA writes the scroll"}]}),
            &f,
        );
        assert!(d.description.contains("dots 22–277"), "{}", d.description);
        let e = draw("timeline", &json!({"span": "frame", "frame": 3}), &f).unwrap_err();
        assert!(e.contains("no recording"), "{e}");
        let e = draw(
            "timeline",
            &json!({"span": "frame", "marks": [{"at": 400}]}),
            &f,
        )
        .unwrap_err();
        assert!(e.contains("0 to 261"), "{e}");
    }

    #[test]
    fn a_recorded_frame_places_its_events() {
        let r = Recorded(Fixture::new());
        let d = drawn("timeline", json!({"span": "frame", "frame": 120}), &r);
        assert!(d.description.starts_with("Frame 120"));
        assert!(
            d.description
                .contains("line 226 (DMA): DMA ch 0: $7E:0200 → OAM, 544 bytes"),
            "{}",
            d.description
        );
        assert!(
            d.description.contains("lines 1–224: BG1HOFS"),
            "{}",
            d.description
        );
        assert!(d.description.contains("and 3 more"), "{}", d.description);
    }
}
