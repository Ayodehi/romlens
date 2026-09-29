//! A register's bits (`fields`): bit 7 (or 15) to 0 as boxes coloured by
//! field, a key under each field, and a row per field saying its bits, its
//! name and, with a value, what that value sets. From `explain::fields`.

use serde_json::Value;

use romlens_core::explain::fields::{Kind, Layout, layout_named};
use romlens_core::model::hardware_register;

use super::{Drawing, first_sentence, fit, list, number, parse_number, text, wrap};
use crate::fonts::{Font, measure};
use crate::svg::{Shape, Svg, Text, palette};

const W: f64 = 720.0;
const PAD: f64 = 24.0;

fn find(register: &str) -> Result<(String, Layout), String> {
    if let Some(found) = layout_named(register) {
        return Ok(found);
    }
    let a = parse_number(register.rsplit(':').next().unwrap_or(register))
        .ok_or_else(|| format!("no register named {register}"))? as u16;
    let r = hardware_register(a).ok_or_else(|| format!("${a:04X} is not a hardware register"))?;
    layout_named(r.name).ok_or_else(|| format!("{} has no fields: {}", r.name, r.description))
}

fn possible(f: &romlens_core::explain::fields::Field) -> String {
    match f.kind {
        Kind::Flag { on, off, .. } => format!("1: {on}; 0: {off}"),
        Kind::Choice { names, .. } => names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{i}: {n}"))
            .collect::<Vec<_>>()
            .join("; "),
        Kind::Number(_) => format!("a number, 0 to {}", (1u32 << (f.hi - f.lo + 1)) - 1),
    }
}

pub fn draw(spec: &Value) -> Result<Drawing, String> {
    let register = text(spec, "register").ok_or("say which register, by name or address")?;
    let (name, l) = find(register)?;
    let whole = l.fields.len() == 1
        && l.fields[0].lo == 0
        && l.fields[0].hi + 1 >= if l.pair.is_some() { 16 } else { 8 };
    if whole {
        return Err(format!(
            "{name} is one number ({}), so there are no fields to draw: {}",
            l.fields[0].name,
            first_sentence(l.about)
        ));
    }
    if l.fields.is_empty() || l.data {
        return Err(format!(
            "{name}'s bytes are data, not fields, so there are no bits to draw: {}",
            first_sentence(l.about)
        ));
    }
    let bits: u8 = if l.pair.is_some() { 16 } else { 8 };
    let value = match number(spec, "value") {
        Some(v) if v < 0 || v >= 1 << bits => {
            return Err(format!("{name} is {bits} bits; {v} does not fit"));
        }
        v => v.map(|v| v as u32),
    };
    let wanted: Vec<String> = list(spec, "highlight")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_ascii_lowercase)
        .collect();
    for w in &wanted {
        if !l.fields.iter().any(|f| f.name.to_ascii_lowercase() == *w) {
            return Err(format!(
                "{name} has no field “{w}”; its fields are {}",
                l.fields
                    .iter()
                    .map(|f| f.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let lit = |i: usize| wanted.contains(&l.fields[i].name.to_ascii_lowercase());
    // Fields left to right, highest bits first.
    let mut order: Vec<usize> = (0..l.fields.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(l.fields[i].hi));
    let key = |i: usize| (b'A' + order.iter().position(|&o| o == i).unwrap_or(0) as u8) as char;

    let about = wrap(first_sentence(l.about), Font::Sans, 13.0, W - 2.0 * PAD);
    let row_h = 26.0;
    let top = 60.0 + about.len() as f64 * 19.0 + 8.0;
    let cell = ((W - 2.0 * PAD) / bits as f64).min(64.0);
    let x0 = (W - cell * bits as f64) / 2.0;
    let cells_y = top + 20.0;
    let names_y = cells_y + 44.0 + 10.0;
    let rows_y = names_y + 40.0;
    let h = rows_y + row_h * l.fields.len() as f64 + 40.0;
    let mut svg = Svg::new(W, h);

    svg.text(
        PAD,
        36.0,
        &format!("{name}  ${:04X}", l.address),
        Text::new(18.0, Font::Bold),
    );
    if let Some(v) = value {
        let digits = if bits == 16 { 4 } else { 2 };
        svg.text(
            W - PAD,
            36.0,
            &format!("= ${v:0digits$X}"),
            Text::new(18.0, Font::Mono).anchor(crate::svg::Anchor::End),
        );
    }
    for (i, line) in about.iter().enumerate() {
        svg.text(
            PAD,
            62.0 + i as f64 * 19.0,
            line,
            Text::new(13.0, Font::Sans).colour(palette::MUTED),
        );
    }

    let field_of = |b: u8| l.fields.iter().position(|f| f.lo <= b && b <= f.hi);
    let col_x = |b: u8| x0 + (bits - 1 - b) as f64 * cell;
    for b in 0..bits {
        let x = col_x(b);
        svg.text(
            x + cell / 2.0,
            top + 12.0,
            &b.to_string(),
            Text::new(12.0, Font::Mono).colour(palette::MUTED).middle(),
        );
        let fill = field_of(b)
            .map(|i| {
                palette::FILLS
                    [order.iter().position(|&o| o == i).unwrap_or(0) % palette::FILLS.len()]
            })
            .unwrap_or(palette::QUIET_FILL);
        let shape = Shape::new(fill).radius(0.0);
        svg.rect(x, cells_y, cell, 44.0, shape);
        match (value, field_of(b)) {
            (Some(v), _) => svg.text(
                x + cell / 2.0,
                cells_y + 29.0,
                &((v >> b) & 1).to_string(),
                Text::new(18.0, Font::Mono).middle(),
            ),
            (None, None) => svg.text(
                x + cell / 2.0,
                cells_y + 28.0,
                "–",
                Text::new(14.0, Font::Sans).colour(palette::MUTED).middle(),
            ),
            _ => {}
        }
    }
    // Each field's outline, its bracket and its key or name.
    for &i in &order {
        let f = &l.fields[i];
        let (left, right) = (col_x(f.hi), col_x(f.lo) + cell);
        let outline = if lit(i) {
            Shape::new("none").stroke(palette::ACCENT, 2.5).radius(0.0)
        } else {
            Shape::new("none").stroke(palette::INK, 1.25).radius(0.0)
        };
        svg.rect(left, cells_y, right - left, 44.0, outline);
        let colour = if lit(i) {
            palette::ACCENT
        } else {
            palette::LINE
        };
        svg.line(left + 3.0, names_y, right - 3.0, names_y, colour, 1.5);
        svg.line(left + 3.0, names_y - 5.0, left + 3.0, names_y, colour, 1.5);
        svg.line(
            right - 3.0,
            names_y - 5.0,
            right - 3.0,
            names_y,
            colour,
            1.5,
        );
        let label_font = if lit(i) { Font::Bold } else { Font::Sans };
        let span = right - left - 6.0;
        let label = if measure(f.name, label_font, 13.0).0 <= span {
            f.name.to_string()
        } else {
            key(i).to_string()
        };
        svg.text(
            (left + right) / 2.0,
            names_y + 19.0,
            &label,
            Text::new(13.0, label_font)
                .colour(if lit(i) {
                    palette::ACCENT
                } else {
                    palette::INK
                })
                .middle(),
        );
    }
    // A row per field.
    let mut description = format!("{name} (${:04X}): {}", l.address, first_sentence(l.about));
    if let Some(v) = value {
        let digits = if bits == 16 { 4 } else { 2 };
        description.push_str(&format!(" Drawn with the value ${v:0digits$X}."));
    }
    for (k, &i) in order.iter().enumerate() {
        let f = &l.fields[i];
        let y = rows_y + k as f64 * row_h;
        let fill = palette::FILLS[k % palette::FILLS.len()];
        svg.rect(PAD, y - 13.0, 18.0, 18.0, Shape::new(fill).radius(3.0));
        svg.text(
            PAD + 9.0,
            y + 1.0,
            &key(i).to_string(),
            Text::new(11.0, Font::Bold).middle(),
        );
        let bits_text = if f.hi == f.lo {
            format!("bit {}", f.lo)
        } else {
            format!("bits {}", f.bits())
        };
        svg.text(
            PAD + 30.0,
            y,
            &bits_text,
            Text::new(13.0, Font::Mono).colour(palette::MUTED),
        );
        let name_font = if lit(i) { Font::Bold } else { Font::Sans };
        let name_w = 190.0;
        svg.text(
            PAD + 118.0,
            y,
            &fit(f.name, name_font, 14.0, name_w),
            Text::new(14.0, name_font).colour(if lit(i) {
                palette::ACCENT
            } else {
                palette::INK
            }),
        );
        let meaning = match value {
            Some(v) => format!("= {}: {}", f.raw(v), f.meaning(v)),
            None => possible(f),
        };
        let mx = PAD + 118.0 + name_w + 12.0;
        svg.text(
            mx,
            y,
            &fit(&meaning, Font::Sans, 14.0, W - PAD - mx),
            Text::new(14.0, Font::Sans).colour(if value.is_some() {
                palette::INK
            } else {
                palette::MUTED
            }),
        );
        description.push_str(&format!(
            "\n- {bits_text}, {}: {}",
            f.name,
            match value {
                Some(v) => format!("{} ({})", f.raw(v), f.meaning(v)),
                None => possible(f),
            }
        ));
    }
    let unused: Vec<u8> = (0..bits).filter(|&b| field_of(b).is_none()).collect();
    if !unused.is_empty() {
        let y = rows_y + l.fields.len() as f64 * row_h;
        let list = unused
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        svg.text(
            PAD,
            y,
            &format!("Unused bits: {list}"),
            Text::new(13.0, Font::Sans).colour(palette::MUTED),
        );
        description.push_str(&format!("\n- unused bits: {list}"));
    }
    Ok(Drawing {
        svg: svg.finish(),
        title: format!("{name}'s bits"),
        description,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::kinds::draw;
    use crate::kinds::testing::{Fixture, drawn};

    #[test]
    fn inidisp_bit_7_is_forced_blank() {
        let f = Fixture::new();
        let d = drawn(
            "fields",
            json!({"register": "INIDISP", "value": "$8F", "highlight": ["Forced blank"]}),
            &f,
        );
        assert!(d.svg.contains("INIDISP  $2100"));
        assert!(
            d.description
                .contains("bit 7, Forced blank: 1 (forced blank)"),
            "{}",
            d.description
        );
        assert!(
            d.description.contains("bits 0–3, Brightness: 15"),
            "{}",
            d.description
        );
        assert!(
            d.description.contains("unused bits: 4, 5, 6"),
            "{}",
            d.description
        );
        // Without a value, what each field can be.
        let d = drawn(
            "fields",
            json!({"register": "$2100", "value": null, "highlight": null}),
            &f,
        );
        assert!(
            d.description.contains("1: forced blank; 0: display on"),
            "{}",
            d.description
        );
    }

    #[test]
    fn pairs_and_many_fields() {
        let f = Fixture::new();
        let d = drawn(
            "fields",
            json!({"register": "OAMADD", "value": "$8123"}),
            &f,
        );
        assert!(d.svg.contains("OAMADD  $2102"));
        assert!(
            d.description.contains("bit 15, Priority rotation: 1"),
            "{}",
            d.description
        );
        for r in [
            "BGMODE", "NMITIMEN", "OBSEL", "VMAIN", "DMAP0", "TM", "CGWSEL", "SETINI", "W12SEL",
        ] {
            drawn("fields", json!({"register": r, "value": "$FF"}), &f);
            drawn("fields", json!({"register": r}), &f);
        }
    }

    #[test]
    fn a_register_without_fields_or_a_wrong_field_says_so() {
        let f = Fixture::new();
        let e = draw("fields", &json!({"register": "VMDATAL"}), &f).unwrap_err();
        assert!(e.contains("no fields"), "{e}");
        let e = draw("fields", &json!({"register": "VMADD"}), &f).unwrap_err();
        assert!(e.contains("one number"), "{e}");
        let e = draw(
            "fields",
            &json!({"register": "INIDISP", "highlight": ["Speed"]}),
            &f,
        )
        .unwrap_err();
        assert!(e.contains("Forced blank"), "{e}");
        let e = draw("fields", &json!({"register": "INIDISP", "value": 300}), &f).unwrap_err();
        assert!(e.contains("8 bits"), "{e}");
        assert!(draw("fields", &json!({"register": "NOPE"}), &f).is_err());
    }
}
