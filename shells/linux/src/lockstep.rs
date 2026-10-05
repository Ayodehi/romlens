//! The Both tab: hex on the left, disassembly on the right, scrolled in
//! lockstep, with a bracket joining the highlighted bytes to their line. The
//! macOS twin is `LockstepEditorView`.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::asmview::{self, AsmPane};
use crate::canvas::{set_source, with_alpha};
use crate::hex::BYTES_PER_ROW;
use crate::hexview::{self, HexPane};
use crate::model::workspace::Id;
use crate::model::{Change, Document};

/// The listing line that should be at the top when hex row `row` is. Rows
/// outside the image (overscroll) are clamped.
pub fn asm_line_for_top_row(doc: &Document, row: i64) -> Option<u32> {
    let rows = i64::from(doc.row_count());
    if rows == 0 {
        return None;
    }
    let row = row.clamp(0, rows - 1) as u32;
    doc.line_for_offset(row * BYTES_PER_ROW as u32)
}

/// The hex row that should be at the top when listing line `line` is, with
/// the same clamping.
pub fn hex_row_for_top_line(doc: &Document, line: i64) -> Option<u32> {
    let lines = i64::from(doc.asm_line_count());
    if lines == 0 {
        return None;
    }
    let line = line.clamp(0, lines - 1) as u32;
    doc.workbench()
        .offset_for_line(line)
        .map(|o| o / BYTES_PER_ROW as u32)
}

pub fn build(doc: &Rc<Document>, item: Option<Id>) -> gtk::Widget {
    let hex = Rc::new(hexview::build(doc, item));
    let asm = Rc::new(asmview::build(doc, item));

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&hex.widget)
        .end_child(&asm.widget)
        .resize_start_child(true)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .wide_handle(false)
        .build();
    // Hex gets 55% the first time there is room to say so.
    let placed = Rc::new(Cell::new(false));
    paned.connect_notify_local(Some("max-position"), {
        let placed = Rc::clone(&placed);
        move |p, _| {
            if !placed.get() && p.width() > 400 {
                placed.set(true);
                p.set_position((f64::from(p.width()) * 0.55) as i32);
            }
        }
    });

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&paned));
    let bracket = gtk::DrawingArea::builder().can_target(false).build();
    overlay.add_overlay(&bracket);
    {
        let (doc, hex, asm) = (Rc::clone(doc), Rc::clone(&hex), Rc::clone(&asm));
        bracket.set_draw_func(move |a, cr, _, _| draw_bracket(&doc, &hex, &asm, a, cr));
    }

    // Keep the two panes aligned. A dead band stops them nudging each other
    // forever, and the flag stops a sync from syncing back.
    let syncing = Rc::new(Cell::new(false));
    hex.scroll.adjustment.connect_value_changed({
        let (doc, asm, syncing, bracket) = (
            Rc::clone(doc),
            Rc::clone(&asm),
            Rc::clone(&syncing),
            bracket.clone(),
        );
        move |adj| {
            if !syncing.get()
                && let Some(line) = asm_line_for_top_row(&doc, adj.value().floor() as i64)
            {
                follow(&asm.scroll.adjustment, f64::from(line), &syncing);
            }
            bracket.queue_draw();
        }
    });
    asm.scroll.adjustment.connect_value_changed({
        let (doc, hex, syncing, bracket) = (
            Rc::clone(doc),
            Rc::clone(&hex),
            Rc::clone(&syncing),
            bracket.clone(),
        );
        move |adj| {
            if !syncing.get()
                && let Some(row) = hex_row_for_top_line(&doc, adj.value().floor() as i64)
            {
                follow(&hex.scroll.adjustment, f64::from(row), &syncing);
            }
            bracket.queue_draw();
        }
    });
    asm.scroll.adjustment.connect_changed({
        let bracket = bracket.clone();
        move |_| bracket.queue_draw()
    });
    for area in [&hex.area, &asm.area] {
        let bracket = bracket.clone();
        area.connect_resize(move |_, _, _| bracket.queue_draw());
    }
    doc.subscribe({
        let bracket = bracket.clone();
        move |c| {
            if matches!(
                c,
                Change::Selection | Change::Rows | Change::AddressStyle | Change::Scroll
            ) {
                bracket.queue_draw();
            }
        }
    });
    overlay.upcast()
}

fn follow(adj: &gtk::Adjustment, target: f64, syncing: &Cell<bool>) {
    let max = (adj.upper() - adj.page_size()).max(0.0);
    let target = target.clamp(0.0, max);
    if (adj.value() - target).abs() <= 0.5 {
        return;
    }
    syncing.set(true);
    adj.set_value(target);
    syncing.set(false);
}

/// Anchors: the highlighted rows' right edge in the hex pane and the
/// selected line's left edge in the listing, converted into the overlay and
/// clipped to their panes.
fn draw_bracket(
    doc: &Document,
    hex: &HexPane,
    asm: &AsmPane,
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
) {
    let Some(range) = doc.highlighted_range().filter(|r| !r.is_empty()) else {
        return;
    };
    let Some(line) = doc.line_for_offset(range.start) else {
        return;
    };
    let row_bytes = BYTES_PER_ROW as u32;
    let (first_row, last_row) = (range.start / row_bytes, (range.end - 1) / row_bytes);

    let to_overlay = |w: &gtk::Widget, x: f64, y: f64| -> Option<(f64, f64)> {
        w.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))
            .map(|p| (f64::from(p.x()), f64::from(p.y())))
    };
    let hex_rh = hex.row_height();
    let asm_rh = asm.row_height();
    let x_right = hex.hex_right_edge();
    let hex_top_y = (f64::from(first_row) - hex.scroll.top()) * hex_rh;
    let hex_bottom_y = (f64::from(last_row) + 1.0 - hex.scroll.top()) * hex_rh;
    let asm_top_y = (f64::from(line) - asm.scroll.top()) * asm_rh;
    let asm_bottom_y = asm_top_y + asm_rh;

    let hex_w: gtk::Widget = hex.area.clone().upcast();
    let asm_w: gtk::Widget = asm.area.clone().upcast();
    let (Some(ht), Some(hb), Some(at), Some(ab)) = (
        to_overlay(&hex_w, x_right, hex_top_y),
        to_overlay(&hex_w, x_right, hex_bottom_y),
        to_overlay(&asm_w, 0.0, asm_top_y),
        to_overlay(&asm_w, 0.0, asm_bottom_y),
    ) else {
        return;
    };
    // Clip to each pane's frame, in overlay coordinates.
    let frame = |w: &gtk::Widget| -> Option<(f64, f64)> {
        let top = to_overlay(w, 0.0, 0.0)?.1;
        Some((top, top + f64::from(w.height())))
    };
    let (Some((hy0, hy1)), Some((ay0, ay1))) = (frame(&hex_w), frame(&asm_w)) else {
        return;
    };
    let (ht, hb) = ((ht.0, ht.1.clamp(hy0, hy1)), (hb.0, hb.1.clamp(hy0, hy1)));
    let (at, ab) = ((at.0, at.1.clamp(ay0, ay1)), (ab.0, ab.1.clamp(ay0, ay1)));

    let accent = adw::StyleManager::default().accent_color_rgba();
    set_source(cr, &with_alpha(&accent, 0.8));
    cr.set_line_width(1.5);
    // Hex side: a bracket along the highlighted rows' right edge.
    cr.move_to(ht.0 - 4.0, ht.1);
    cr.line_to(ht.0 + 4.0, ht.1);
    cr.line_to(hb.0 + 4.0, hb.1);
    cr.line_to(hb.0 - 4.0, hb.1);
    // The join across the divider to the line's left edge.
    let hex_mid = (ht.0 + 4.0, (ht.1 + hb.1) / 2.0);
    let asm_mid = (at.0 + 6.0, (at.1 + ab.1) / 2.0);
    cr.move_to(hex_mid.0, hex_mid.1);
    let mid_x = (hex_mid.0 + asm_mid.0) / 2.0;
    cr.curve_to(mid_x, hex_mid.1, mid_x, asm_mid.1, asm_mid.0, asm_mid.1);
    // Listing side: a bracket along the selected line's left edge.
    cr.move_to(at.0 + 12.0, at.1);
    cr.line_to(at.0 + 6.0, at.1);
    cr.line_to(ab.0 + 6.0, ab.1);
    cr.line_to(ab.0 + 12.0, ab.1);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::{TestRuntime, test_rom};

    fn doc() -> Rc<Document> {
        let rt = TestRuntime::new();
        let d = Document::new(test_rom(), rt.clone());
        d.start_analysis();
        rt.pump();
        d
    }

    #[test]
    fn a_hex_row_maps_to_the_line_at_its_first_byte() {
        let d = doc();
        assert_eq!(asm_line_for_top_row(&d, 0), d.line_for_offset(0));
        // Overscroll clamps to the first and last rows.
        assert_eq!(asm_line_for_top_row(&d, -5), d.line_for_offset(0));
        let last = i64::from(d.row_count()) - 1;
        assert_eq!(
            asm_line_for_top_row(&d, last + 40),
            asm_line_for_top_row(&d, last)
        );
    }

    #[test]
    fn a_line_maps_back_to_its_hex_row() {
        let d = doc();
        for line in [0i64, 3, 40] {
            let row = hex_row_for_top_line(&d, line).unwrap();
            let offset = d.workbench().offset_for_line(line as u32).unwrap();
            assert_eq!(row, offset / 16);
        }
        assert_eq!(hex_row_for_top_line(&d, -3), hex_row_for_top_line(&d, 0));
        let beyond = i64::from(d.asm_line_count()) + 100;
        assert_eq!(
            hex_row_for_top_line(&d, beyond),
            hex_row_for_top_line(&d, i64::from(d.asm_line_count()) - 1)
        );
    }
}
