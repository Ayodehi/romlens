//! Memory maps (`memory_map`): one bank as a column of what each address
//! reaches, or the whole 24-bit space as a grid of banks, for this ROM's
//! mapping. From `AddressMap::regions` and the header's vectors.

use serde_json::Value;

use romlens_core::memory::map::BankRegion;
use romlens_core::{MemoryClass, RomImage, SnesAddress};

use super::{Drawing, Placer, Source, addr, fit, list, parse_number, text, wrap};
use crate::fonts::{Font, measure};
use crate::svg::{Anchor, Shape, Svg, Text, palette};

const W: f64 = 720.0;
const PAD: f64 = 24.0;

fn fill(class: MemoryClass, name: &str) -> &'static str {
    match class {
        MemoryClass::Rom => palette::FILLS[0],
        MemoryClass::Wram | MemoryClass::LowRam => palette::FILLS[1],
        MemoryClass::Hardware if name != "Unused" => palette::FILLS[2],
        MemoryClass::Sram => palette::FILLS[3],
        _ => palette::QUIET_FILL,
    }
}

fn size(bytes: u32) -> String {
    if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}

/// What a region holds, in a line.
fn detail(rom: &RomImage, bank: u8, r: &BankRegion) -> Option<String> {
    match r.class {
        MemoryClass::Rom => {
            let a = rom.file_offset_for(SnesAddress::new(bank, r.start))?;
            let b = rom.file_offset_for(SnesAddress::new(bank, r.end))?;
            Some(format!("file 0x{:05X}–0x{:05X}", a.0, b.0))
        }
        MemoryClass::LowRam => Some("the first 8 KB of WRAM, $7E:0000–$1FFF".to_string()),
        MemoryClass::Wram => Some(format!("128 KB in banks $7E–$7F; this is ${bank:02X}")),
        MemoryClass::Sram => Some("the cartridge's battery-backed RAM".to_string()),
        MemoryClass::OpenBus if r.end - r.start >= 0x0FFF => {
            Some("nothing answers here".to_string())
        }
        _ => None,
    }
}

/// The interrupt vector at a bank `$00` address, and where it points.
fn vector(rom: &RomImage, src: &dyn Source, a: u32) -> Option<String> {
    if a >> 16 != 0 && a >> 16 != 0x80 {
        return None;
    }
    let off = a & 0xFFFF;
    for (base, native, v) in [
        (0xFFE4u32, true, rom.native_vectors()),
        (0xFFF4, false, rom.emulation_vectors()),
    ] {
        if (base..base + 12).contains(&off) {
            let i = ((off - base) / 2) as usize;
            let (name, target) = v.named(native)[i];
            if name.contains("unused") {
                return Some(format!(
                    "an unused vector ({})",
                    if native { "native" } else { "emulation" }
                ));
            }
            let mode = if native { "native" } else { "emulation" };
            let to = src
                .name_at(target as u32)
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            let what = if name == "RESET" {
                "the reset vector".to_string()
            } else {
                format!("the {mode} {name} vector")
            };
            return Some(format!("{what}: points to ${target:04X}{to}"));
        }
    }
    None
}

/// An address as written, `$80:FFEA` staying in bank `$80` (resolving
/// would move it to its canonical bank), or a name resolved.
fn at_address(t: &str, src: &dyn Source) -> Result<u32, String> {
    let lit = t.trim().trim_start_matches('$');
    if let Some((b, o)) = lit.split_once(':')
        && let (Ok(b), Ok(o)) = (
            u8::from_str_radix(b, 16),
            u16::from_str_radix(o.trim_start_matches('$'), 16),
        )
    {
        return Ok((b as u32) << 16 | o as u32);
    }
    src.resolve(t)
}

struct Mark {
    address: u32,
    label: String,
}

fn marks(spec: &Value, rom: &RomImage, src: &dyn Source) -> Result<Vec<Mark>, String> {
    let mut out = Vec::new();
    for m in list(spec, "marks") {
        let at = text(m, "address").ok_or("each mark needs an address")?;
        let address = at_address(at, src)?;
        let label = text(m, "label")
            .map(str::to_string)
            .or_else(|| vector(rom, src, address))
            .or_else(|| src.name_at(address))
            .unwrap_or_else(|| addr(address));
        out.push(Mark { address, label });
    }
    for h in list(spec, "highlight") {
        let (Some(a), Some(b)) = (text(h, "start"), text(h, "end")) else {
            return Err("each highlight needs a start and an end".to_string());
        };
        let (a, b) = (at_address(a, src)?, at_address(b, src)?);
        if b < a {
            return Err(format!(
                "the highlight {} to {} ends before it starts",
                addr(a),
                addr(b)
            ));
        }
        let label = text(h, "label").map(str::to_string).unwrap_or_default();
        out.push(Mark {
            address: a,
            label: format!(
                "{label}{}{}–{}",
                if label.is_empty() { "" } else { ": " },
                addr(a),
                addr(b)
            ),
        });
    }
    if out.len() > 10 {
        return Err("at most ten marks and highlights".to_string());
    }
    Ok(out)
}

fn highlights(spec: &Value, src: &dyn Source) -> Vec<(u32, u32)> {
    list(spec, "highlight")
        .iter()
        .filter_map(|h| {
            Some((
                at_address(text(h, "start")?, src).ok()?,
                at_address(text(h, "end")?, src).ok()?,
            ))
        })
        .collect()
}

pub fn draw(spec: &Value, src: &dyn Source) -> Result<Drawing, String> {
    let rom = src.rom().ok_or("no ROM is open")?;
    match text(spec, "view").unwrap_or("bank") {
        "bank" => bank(spec, rom, src),
        "banks" => banks(spec, rom, src),
        v => Err(format!("view is bank or banks, not {v}")),
    }
}

fn bank(spec: &Value, rom: &RomImage, src: &dyn Source) -> Result<Drawing, String> {
    let bank = match text(spec, "bank") {
        Some(b) => {
            let n = parse_number(b.trim_end_matches(':'))
                .or_else(|| parse_number(&format!("${}", b.trim_start_matches('$'))))
                .ok_or_else(|| format!("{b} is not a bank; write it like $00 or $7E"))?;
            if !(0..=0xFF).contains(&n) {
                return Err(format!("{b} is not a bank from $00 to $FF"));
            }
            n as u8
        }
        None => 0,
    };
    let regions = rom.map().regions(bank);
    let marks = marks(spec, rom, src)?;
    for m in &marks {
        if (m.address >> 16) as u8 != bank {
            return Err(format!(
                "{} is not in bank ${bank:02X}; draw bank ${:02X} for it",
                addr(m.address),
                m.address >> 16
            ));
        }
    }
    let lit = highlights(spec, src);

    // Heights by size, not to scale: a 4-byte port still gets room for its
    // name.
    let height = |r: &BankRegion| {
        let bytes = (r.end as u32 - r.start as u32 + 1) as f64;
        (26.0 + 9.0 * (bytes / 256.0).max(1.0).log2()).clamp(26.0, 96.0)
    };
    let (col_x, col_w) = (118.0, 300.0);
    let top = 78.0;
    let total: f64 = regions.iter().map(height).sum();
    let h = (top + total + 40.0).max(260.0);
    let mut svg = Svg::new(W, h);
    svg.text(
        PAD,
        36.0,
        &format!("Bank ${bank:02X}"),
        Text::new(18.0, Font::Bold),
    );
    svg.text(
        PAD,
        58.0,
        &format!(
            "What each address in bank ${bank:02X} reaches, {} mapping (heights not to scale)",
            rom.mapping().name()
        ),
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    let mut description = format!(
        "Bank ${bank:02X} of this {} ROM, from $0000 to $FFFF:",
        rom.mapping().name()
    );
    let mut placer = Placer::default();
    let mut y = top;
    let mut spans = Vec::new();
    for r in &regions {
        let rh = height(r);
        let a = (bank as u32) << 16;
        let on = lit
            .iter()
            .any(|&(s, e)| s <= a | r.end as u32 && a | r.start as u32 <= e);
        let shape = if on {
            Shape::new(fill(r.class, r.name))
                .stroke(palette::ACCENT, 2.5)
                .radius(0.0)
        } else {
            Shape::new(fill(r.class, r.name)).radius(0.0)
        };
        svg.rect(col_x, y, col_w, rh, shape);
        svg.text(
            col_x - 8.0,
            y + 12.0,
            &format!("${:04X}", r.start),
            Text::new(12.0, Font::Mono)
                .colour(palette::MUTED)
                .anchor(Anchor::End),
        );
        placer.take(col_x - 60.0, y, 60.0, 14.0);
        let bytes = r.end as u32 - r.start as u32 + 1;
        let name_colour = if r.name == "Unused" || r.class == MemoryClass::OpenBus {
            palette::MUTED
        } else {
            palette::INK
        };
        let detail = detail(rom, bank, r);
        let two = rh >= 48.0 && detail.is_some();
        let name_y = if two {
            y + rh / 2.0 - 3.0
        } else {
            Text::new(14.0, Font::Sans).centred(y + rh / 2.0)
        };
        svg.text(
            col_x + 10.0,
            name_y,
            r.name,
            Text::new(14.0, Font::Sans).colour(name_colour),
        );
        svg.text(
            col_x + col_w - 10.0,
            name_y,
            &size(bytes),
            Text::new(12.0, Font::Sans)
                .colour(palette::MUTED)
                .anchor(Anchor::End),
        );
        if two && let Some(d) = &detail {
            let font = if r.class == MemoryClass::Rom {
                Font::Mono
            } else {
                Font::Sans
            };
            svg.text(
                col_x + 10.0,
                name_y + 17.0,
                &fit(d, font, 12.0, col_w - 20.0),
                Text::new(12.0, font).colour(palette::MUTED),
            );
        }
        description.push_str(&format!(
            "\n- ${:04X}–${:04X} {} ({}){}",
            r.start,
            r.end,
            r.name,
            size(bytes),
            detail.map(|d| format!(": {d}")).unwrap_or_default()
        ));
        spans.push((r.start, r.end, y, rh));
        y += rh;
    }
    svg.text(
        col_x - 8.0,
        y + 4.0,
        "$FFFF",
        Text::new(12.0, Font::Mono)
            .colour(palette::MUTED)
            .anchor(Anchor::End),
    );
    // Marks, to the right, each as close to its address as the others let
    // it be.
    let label_x = col_x + col_w + 40.0;
    let label_w = W - PAD - label_x;
    let mut marks = marks;
    marks.sort_by_key(|m| m.address);
    for m in &marks {
        let off = (m.address & 0xFFFF) as u16;
        let Some(&(s, e, ry, rh)) = spans.iter().find(|(s, e, _, _)| *s <= off && off <= *e) else {
            continue;
        };
        let frac = (off - s) as f64 / ((e - s) as f64 + 1.0);
        let my = (ry + 5.0 + frac * (rh - 10.0)).clamp(ry + 5.0, ry + rh - 5.0);
        let lines = wrap(&m.label, Font::Sans, 13.0, label_w);
        let lines = &lines[..lines.len().min(2)];
        let box_h = 18.0 * lines.len() as f64 + 16.0;
        let spots: Vec<(f64, f64)> = (0..40)
            .flat_map(|k| [my - 14.0 + k as f64 * 8.0, my - 14.0 - k as f64 * 8.0])
            .filter(|y| *y > 66.0 && *y + box_h < h - 4.0)
            .map(|y| (label_x, y))
            .collect();
        let Some((lx, ly)) = placer.place(&spots, label_w, box_h) else {
            continue;
        };
        for (i, line) in lines.iter().enumerate() {
            let line = if i + 1 == lines.len() {
                fit(line, Font::Sans, 13.0, label_w)
            } else {
                line.clone()
            };
            svg.text(
                lx,
                ly + 13.0 + 18.0 * i as f64,
                &line,
                Text::new(13.0, Font::Sans),
            );
        }
        svg.text(
            lx,
            ly + 13.0 + 18.0 * lines.len() as f64,
            &addr(m.address),
            Text::new(12.0, Font::Mono).colour(palette::MUTED),
        );
        svg.arrow(
            &[(lx - 6.0, ly + 9.0), (col_x + col_w + 4.0, my)],
            palette::ACCENT,
            1.5,
            false,
        );
        description.push_str(&format!("\nMarked: {} at {}", m.label, addr(m.address)));
    }
    Ok(Drawing {
        svg: svg.finish(),
        title: format!("Bank ${bank:02X}"),
        description,
    })
}

/// A bank as runs of one class: first offset, last, class.
type Runs = Vec<(u16, u16, MemoryClass)>;

/// A bank's regions with the hardware window as one: for the grid.
fn coarse(rom: &RomImage, bank: u8) -> Runs {
    let mut out: Runs = Vec::new();
    for r in rom.map().regions(bank) {
        match out.last_mut() {
            Some(last) if last.2 == r.class => last.1 = r.end,
            _ => out.push((r.start, r.end, r.class)),
        }
    }
    out
}

fn short(c: MemoryClass) -> &'static str {
    match c {
        MemoryClass::Rom => "ROM",
        MemoryClass::Wram => "WRAM",
        MemoryClass::LowRam => "Low RAM",
        MemoryClass::Hardware => "Registers",
        MemoryClass::Sram => "SRAM",
        MemoryClass::OpenBus => "Open bus",
    }
}

fn banks(spec: &Value, rom: &RomImage, src: &dyn Source) -> Result<Drawing, String> {
    // Banks with the same layout side by side are one column.
    let mut cols: Vec<(u8, u8, Runs)> = Vec::new();
    for b in 0..=0xFFu8 {
        let c = coarse(rom, b);
        match cols.last_mut() {
            Some(last) if last.2 == c => last.1 = b,
            _ => cols.push((b, b, c)),
        }
    }
    if cols.len() > 12 {
        return Err(
            "this ROM's banks differ too much to draw as one grid; draw a bank at a time"
                .to_string(),
        );
    }
    let mut rows: Vec<u16> = cols.iter().flat_map(|c| c.2.iter().map(|r| r.0)).collect();
    rows.sort_unstable();
    rows.dedup();
    let marks = marks(spec, rom, src)?;
    let (left, top, row_h) = (78.0, 104.0, 54.0);
    let col_w = (W - PAD - left) / cols.len() as f64;
    let h = top + row_h * rows.len() as f64 + 40.0 + 22.0 * marks.len() as f64;
    let mut svg = Svg::new(W, h);
    svg.text(
        PAD,
        36.0,
        "The whole address space",
        Text::new(18.0, Font::Bold),
    );
    svg.text(
        PAD,
        58.0,
        &format!(
            "Banks $00–$FF of this {} ROM ({} KB): what each one holds",
            rom.mapping().name(),
            rom.len() / 1024
        ),
        Text::new(13.0, Font::Sans).colour(palette::MUTED),
    );
    let mut description = format!(
        "The 24-bit address space of this {} ROM, bank by bank:",
        rom.mapping().name()
    );
    let row_of = |off: u16| rows.iter().rposition(|&r| r <= off).unwrap_or(0);
    for (i, &r) in rows.iter().enumerate() {
        svg.text(
            left - 8.0,
            top + i as f64 * row_h + 13.0,
            &format!("${r:04X}"),
            Text::new(12.0, Font::Mono)
                .colour(palette::MUTED)
                .anchor(Anchor::End),
        );
    }
    let mut seen: Vec<(u8, u8, &Runs)> = Vec::new();
    for (k, (a, b, regions)) in cols.iter().enumerate() {
        let x = left + k as f64 * col_w;
        let head = if a == b {
            format!("${a:02X}")
        } else {
            format!("${a:02X}–${b:02X}")
        };
        svg.text(
            x + col_w / 2.0,
            top - 26.0,
            &head,
            Text::new(12.0, Font::Mono).middle(),
        );
        let same = seen.iter().find(|s| s.2 == regions).map(|s| (s.0, s.1));
        if let Some((sa, sb)) = same {
            let note = if sa == sb {
                format!("as ${sa:02X}")
            } else {
                format!("as ${sa:02X}–${sb:02X}")
            };
            if measure(&note, Font::Sans, 11.0).0 <= col_w - 4.0 {
                svg.text(
                    x + col_w / 2.0,
                    top - 10.0,
                    &note,
                    Text::new(11.0, Font::Sans).colour(palette::MUTED).middle(),
                );
            }
        }
        seen.push((*a, *b, regions));
        let mut parts = Vec::new();
        for &(s, e, c) in regions {
            let (r0, r1) = (row_of(s), row_of(e));
            let y = top + r0 as f64 * row_h;
            let hh = (r1 - r0 + 1) as f64 * row_h;
            let lit = marks.iter().any(|m| {
                let mb = (m.address >> 16) as u8;
                let mo = (m.address & 0xFFFF) as u16;
                *a <= mb && mb <= *b && s <= mo && mo <= e
            });
            let shape = Shape::new(fill(c, short(c))).radius(0.0);
            let shape = if lit {
                shape.stroke(palette::ACCENT, 2.5)
            } else {
                shape
            };
            svg.rect(x, y, col_w, hh, shape);
            let colour = if c == MemoryClass::OpenBus {
                palette::MUTED
            } else {
                palette::INK
            };
            svg.text(
                x + col_w / 2.0,
                Text::new(13.0, Font::Sans).centred(y + hh / 2.0),
                &fit(short(c), Font::Sans, 13.0, col_w - 8.0),
                Text::new(13.0, Font::Sans).colour(colour).middle(),
            );
            parts.push(format!("${s:04X}–${e:04X} {}", short(c)));
        }
        description.push_str(&format!(
            "\n- {head}: {}{}",
            parts.join(", "),
            same.map(|(sa, sb)| format!(" (the same as ${sa:02X}–${sb:02X})"))
                .unwrap_or_default()
        ));
    }
    svg.text(
        left - 8.0,
        top + rows.len() as f64 * row_h + 4.0,
        "$FFFF",
        Text::new(12.0, Font::Mono)
            .colour(palette::MUTED)
            .anchor(Anchor::End),
    );
    for (i, m) in marks.iter().enumerate() {
        let y = top + rows.len() as f64 * row_h + 36.0 + i as f64 * 22.0;
        svg.text(
            PAD,
            y,
            &addr(m.address),
            Text::new(12.0, Font::Mono).colour(palette::ACCENT),
        );
        svg.text(
            PAD + 90.0,
            y,
            &fit(&m.label, Font::Sans, 13.0, W - 2.0 * PAD - 90.0),
            Text::new(13.0, Font::Sans),
        );
        description.push_str(&format!("\nMarked: {} at {}", m.label, addr(m.address)));
    }
    Ok(Drawing {
        svg: svg.finish(),
        title: "The address space".to_string(),
        description,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::kinds::draw;
    use crate::kinds::testing::{Fixture, drawn};

    #[test]
    fn bank_00_with_the_reset_vector() {
        let f = Fixture::new();
        let d = drawn(
            "memory_map",
            json!({"view": "bank", "bank": "$00", "marks": [{"address": "$00:FFFC", "label": null}], "highlight": null}),
            &f,
        );
        assert!(
            d.description.contains("$2100–$213F PPU registers"),
            "{}",
            d.description
        );
        assert!(
            d.description
                .contains("$8000–$FFFF ROM (32 KB): file 0x00000–0x07FFF"),
            "{}",
            d.description
        );
        assert!(
            d.description
                .contains("the reset vector: points to $8000 (RESET)"),
            "{}",
            d.description
        );
    }

    #[test]
    fn other_banks_and_highlights() {
        let f = Fixture::new();
        drawn(
            "memory_map",
            json!({"view": "bank", "bank": "$7E", "highlight": [{"start": "$7E:0000", "end": "$7E:00FF", "label": "Direct page"}]}),
            &f,
        );
        drawn(
            "memory_map",
            json!({"bank": "$80", "marks": [{"address": "$80:FFEA", "label": null}, {"address": "$80:2100", "label": "INIDISP"}, {"address": "$80:2105", "label": "BGMODE"}, {"address": "$80:420B", "label": "MDMAEN"}]}),
            &f,
        );
        let e = draw(
            "memory_map",
            &json!({"view": "bank", "bank": "$00", "marks": [{"address": "$7E:0000"}]}),
            &f,
        )
        .unwrap_err();
        assert!(e.contains("draw bank $7E"), "{e}");
    }

    #[test]
    fn the_whole_space() {
        let f = Fixture::new();
        let d = drawn(
            "memory_map",
            json!({"view": "banks", "marks": [{"address": "$7E:0000", "label": "WRAM starts"}]}),
            &f,
        );
        assert!(d.description.contains("$7E"), "{}", d.description);
        assert!(
            d.description.contains("the same as $00–$3F"),
            "{}",
            d.description
        );
    }
}
