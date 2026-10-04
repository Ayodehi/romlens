//! The inspector's sections for the selected item. The macOS twin is
//! `InspectorView.swift`'s section views.

use std::rc::Rc;

use gtk::prelude::*;
use romlens_ffi::{
    Access, ByteInterpretation, CommentKind, EvidenceInfo, EvidenceKind, FlagOverride, FlagState,
    InstructionInfo, PreviewInfo, PreviewView, RegionInfo, RegionKind, Severity,
};

use super::ui::*;
use crate::model::{Details, Document};
use crate::pixels;
use crate::style::chip;

fn region_class(region: &RegionInfo) -> &'static str {
    match region.kind {
        RegionKind::Code => "chip-blue",
        RegionKind::Data => "chip-orange",
        RegionKind::Unknown => "chip-gray",
    }
}

fn percent(v: f32) -> i32 {
    (v * 100.0).round() as i32
}

// MARK: Header

pub fn selection_header(doc: &Rc<Document>, d: &Details) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let top = hbox(8);
    let address = doc
        .selected_address()
        .map_or_else(|| "unmapped".to_owned(), romlens_ffi::format_snes_address);
    let title = mono(&address);
    title.add_css_class("title-3");
    title.set_hexpand(true);
    top.append(&title);
    if let Some(region) = &d.region {
        top.append(&region_chip(doc, region));
    }
    b.append(&top);
    if let Some(offset) = doc.selected() {
        let l = note(&format!("file {}", romlens_ffi::format_file_offset(offset)));
        l.add_css_class("monospace");
        b.append(&l);
    }
    if let Some(range) = doc.highlighted_range()
        && range.len() > 1
    {
        b.append(&note(&format!("{} bytes selected", range.len())));
    }
    b.upcast()
}

// MARK: Region chip and evidence

/// Strongest first: a region can carry several pieces of evidence and the
/// one that decided it should lead. Ties go observation, then the user, then
/// inference.
pub fn sorted_evidence(region: &RegionInfo) -> Vec<EvidenceInfo> {
    let rank = |k: EvidenceKind| match k {
        EvidenceKind::User => 0,
        EvidenceKind::Trace => 1,
        EvidenceKind::Uploaded => 2,
        EvidenceKind::Imported => 3,
        EvidenceKind::VectorReach => 4,
        EvidenceKind::Heuristic => 5,
    };
    let mut e = region.evidence.clone();
    e.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(rank(a.kind).cmp(&rank(b.kind)))
    });
    e
}

/// The `$bb:aaaa` a jump table's evidence names, as a SNES address. Parsing
/// the detail string is not elegant, but a typed back-reference on every
/// evidence variant is a lot of surface for one link, and the string is ours.
pub fn dispatcher_in(detail: &str) -> Option<u32> {
    let hex_upper =
        |s: &str, n: usize| s.len() == n && s.chars().all(|c| matches!(c, '0'..='9' | 'A'..='F'));
    detail.match_indices('$').find_map(|(i, _)| {
        let candidate = detail.get(i + 1..i + 8)?;
        let (bank, offset) = candidate.split_once(':')?;
        if !(hex_upper(bank, 2) && hex_upper(offset, 4)) {
            return None;
        }
        let bank = u32::from_str_radix(bank, 16).ok()?;
        let offset = u32::from_str_radix(offset, 16).ok()?;
        Some(bank << 16 | offset)
    })
}

fn evidence_name(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::VectorReach => "reached from a vector",
        EvidenceKind::Heuristic => "heuristic",
        EvidenceKind::User => "you",
        EvidenceKind::Imported => "import",
        EvidenceKind::Trace => "trace",
        EvidenceKind::Uploaded => "sound upload",
    }
}

fn region_chip(doc: &Rc<Document>, region: &RegionInfo) -> gtk::MenuButton {
    let pop = gtk::Popover::new();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_margin_start(12);
    col.set_margin_end(12);
    col.set_margin_top(10);
    col.set_margin_bottom(10);
    col.set_width_request(260);
    col.append(&heading(&format!("Why {}?", region.name)));
    if region.evidence.is_empty() {
        col.append(&note("Nothing reached these bytes."));
    }
    for e in sorted_evidence(region) {
        let row = hbox(6);
        let kind = note(evidence_name(e.kind));
        row.append(&kind);
        let detail = gtk::Label::new(Some(&e.detail));
        detail.set_wrap(true);
        detail.set_xalign(0.0);
        detail.set_hexpand(true);
        row.append(&detail);
        if e.score > 0.0 && e.score < 1.0 {
            row.append(&note(&format!("{}%", percent(e.score))));
        }
        if let Some(target) = dispatcher_in(&e.detail) {
            let go = small_button("Go");
            go.set_tooltip_text(Some("Go to the instruction this evidence names"));
            let (d, pop) = (Rc::clone(doc), pop.clone());
            go.connect_clicked(move |_| {
                pop.popdown();
                d.jump_to_snes(target);
            });
            row.append(&go);
        }
        col.append(&row);
    }
    let range = note(&format!(
        "{} · {} bytes",
        romlens_ffi::format_file_offset(region.start),
        region.len
    ));
    range.add_css_class("monospace");
    col.append(&range);
    pop.set_child(Some(&col));

    let label = chip(
        &format!("{} {}%", region.name, percent(region.confidence)),
        region_class(region),
    );
    let button = gtk::MenuButton::builder()
        .popover(&pop)
        .child(&label)
        .build();
    button.add_css_class("flat");
    button
}

// MARK: Instruction

fn flag_chips(title: &str, f: &FlagState) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("romlens-flags-title");
    t.set_xalign(0.0);
    b.append(&t);
    let row = hbox(3);
    let bit = |n: &str, set: bool, on: &'static str, off: &'static str| {
        chip(&format!("{n}{}", u8::from(set)), if set { on } else { off })
    };
    row.append(&bit("M", f.m, "chip-orange", "chip-blue"));
    row.append(&bit("X", f.x, "chip-orange", "chip-blue"));
    row.append(&bit("E", f.e, "chip-red", "chip-gray"));
    row.append(&chip(
        &f.dbr
            .map_or("DBR ?".into(), |v| format!("DBR ${}", hex(u32::from(v), 2))),
        "chip-gray",
    ));
    row.append(&chip(
        &f.dp
            .map_or("DP ?".into(), |v| format!("DP ${}", hex(u32::from(v), 4))),
        "chip-gray",
    ));
    for c in children_of(&row) {
        c.add_css_class("monospace");
    }
    b.append(&row);
    b
}

fn children_of(b: &gtk::Box) -> Vec<gtk::Widget> {
    let mut v = Vec::new();
    let mut c = b.first_child();
    while let Some(w) = c {
        c = w.next_sibling();
        v.push(w);
    }
    v
}

pub fn instruction(doc: &Rc<Document>, d: &Details, insn: &InstructionInfo) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let text = mono(&insn.text);
    text.add_css_class("heading");
    b.append(&text);
    let desc = gtk::Label::new(Some(&insn.description));
    desc.add_css_class("dim-label");
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    b.append(&desc);

    let mut items = vec![
        ("Mode", insn.mode.clone()),
        (
            "Bytes",
            insn.bytes
                .iter()
                .map(|x| hex(u32::from(*x), 2))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    ];
    if let Some(t) = insn.target {
        let mut s = romlens_ffi::format_snes_address(t);
        if !insn.target_certain {
            s.push_str("  (uncertain)");
        }
        items.push(("Target", s));
    }
    if let Some(r) = &insn.hardware_register {
        let access = match r.access {
            Access::Read => "R",
            Access::Write => "W",
            Access::ReadWrite => "RW",
        };
        items.push((
            "Register",
            format!("{} ({access})\n{}", r.name, r.description),
        ));
    }
    let grid = rows(&items);
    b.append(&grid);
    if insn.target_file_offset.is_some() {
        let go = small_button("Go to target");
        go.set_halign(gtk::Align::Start);
        let d = Rc::clone(doc);
        go.connect_clicked(move |_| d.follow_reference());
        b.append(&go);
    }

    let flags = hbox(8);
    flags.append(&flag_chips("before", &insn.flags_before));
    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
    arrow.add_css_class("dim-label");
    arrow.set_valign(gtk::Align::End);
    flags.append(&arrow);
    flags.append(&flag_chips("after", &insn.flags_after));
    b.append(&flags);

    for a in &insn.assumptions {
        let l = note(&format!("? {a}"));
        l.add_css_class("warning");
        b.append(&l);
    }
    // Sorted by severity so a gap in the map is not lost among the notes
    // about decisions the analyzer made on purpose.
    let mut warnings = d.warnings.clone();
    warnings.sort_by_key(|w| w.severity != Severity::Warning);
    for w in warnings {
        let l = note(&w.text);
        if w.severity == Severity::Warning {
            l.add_css_class("warning");
            l.set_label(&format!("⚠ {}", w.text));
        } else {
            l.set_label(&format!("ⓘ {}", w.text));
        }
        b.append(&l);
    }
    b.upcast()
}

// MARK: Region, marks and flags

fn flag_text(f: &FlagOverride) -> String {
    let mut parts = Vec::new();
    if let Some(m) = f.m {
        parts.push(format!("M={}", u8::from(m)));
    }
    if let Some(x) = f.x {
        parts.push(format!("X={}", u8::from(x)));
    }
    if let Some(e) = f.e {
        parts.push(format!("E={}", u8::from(e)));
    }
    if let Some(d) = f.dbr {
        parts.push(format!("DBR=${}", hex(u32::from(d), 2)));
    }
    if let Some(d) = f.dp {
        parts.push(format!("DP=${}", hex(u32::from(d), 4)));
    }
    parts.join(" ")
}

pub fn region(doc: &Rc<Document>, d: &Details) -> gtk::Widget {
    let b = section("Region");
    if let Some(r) = &d.region {
        b.append(&rows(&[
            ("Kind", r.name.clone()),
            ("Confidence", format!("{}%", percent(r.confidence))),
            (
                "Range",
                format!("{} + {}", romlens_ffi::format_file_offset(r.start), r.len),
            ),
            (
                "Source",
                r.evidence
                    .first()
                    .map_or("none".to_owned(), |e| e.detail.clone()),
            ),
        ]));
    }
    let mark = hbox(8);
    let l = gtk::Label::new(Some("Mark"));
    l.add_css_class("dim-label");
    mark.append(&l);
    let group = hbox(0);
    group.add_css_class("linked");
    for (label, action) in [
        ("Code", "win.mark-code"),
        ("Data", "win.mark-data"),
        ("Unknown", "win.mark-unknown"),
        ("Clear", "win.clear-mark"),
    ] {
        group.append(
            &gtk::Button::builder()
                .label(label)
                .action_name(action)
                .build(),
        );
    }
    mark.append(&group);
    b.append(&mark);

    let flags = hbox(8);
    let l = gtk::Label::new(Some("Flags"));
    l.add_css_class("dim-label");
    flags.append(&l);
    match &d.flag_override {
        Some(f) => {
            let t = mono(&flag_text(f));
            t.add_css_class("caption");
            flags.append(&t);
        }
        None => {
            let n = note("from analysis");
            n.set_wrap(false);
            flags.append(&n);
        }
    }
    flags.append(
        &gtk::Button::builder()
            .label("Set Flags…")
            .action_name("win.set-flags")
            .build(),
    );
    b.append(&flags);
    let _ = doc;
    b.upcast()
}

// MARK: Preview

pub fn preview(doc: &Rc<Document>, p: &PreviewInfo) -> gtk::Widget {
    let b = section("Preview");
    if let Some(bitmap) = &p.bitmap {
        b.append(&pixels::pixel_image(
            bitmap,
            pixels::fit_scale(bitmap.width, 256),
        ));
    }
    b.append(&note(&p.summary));
    let view = match p.view {
        PreviewView::TileDecoder => "Tile Decoder",
        PreviewView::Palette => "Palette",
        PreviewView::Tilemap => "Tilemap",
    };
    let title = if p.kind == "compressed" {
        format!("Decompress and Open in {view}")
    } else {
        format!("Open in {view}")
    };
    let row = hbox(8);
    // The graphics views arrive in L3; until then the way in is disabled.
    let open = gtk::Button::with_label(&title);
    open.set_sensitive(false);
    row.append(&open);
    let options = gtk::Button::with_label("Options…");
    options.set_sensitive(false);
    options.set_tooltip_text(Some(if doc.marked_range().is_none() {
        "Mark the range first; preview options belong to a mark"
    } else {
        "Palette, tiles across, and a tilemap's size and tiles"
    }));
    row.append(&options);
    b.append(&row);
    b.upcast()
}

// MARK: Label and comments

pub fn label(doc: &Rc<Document>, d: &Details) -> gtk::Widget {
    let b = section("Label");
    let auto = d
        .label
        .as_ref()
        .filter(|l| l.source == romlens_ffi::LabelSource::Auto);
    let entry = gtk::Entry::builder()
        .placeholder_text(auto.map_or("name", |l| l.name.as_str()))
        .hexpand(true)
        .build();
    entry.add_css_class("monospace");
    if let Some(l) = d
        .label
        .as_ref()
        .filter(|l| l.source == romlens_ffi::LabelSource::User)
    {
        entry.set_text(&l.name);
    }
    let error = note("");
    error.add_css_class("error");
    error.set_visible(false);
    entry.connect_activate({
        let (doc, error) = (Rc::clone(doc), error.clone());
        move |e| {
            let name = e.text().trim().to_owned();
            if name.is_empty() {
                return;
            }
            // The core's message is shown as it is, so a bad name says why.
            match doc.set_label(Some(name)) {
                Ok(()) => error.set_visible(false),
                Err(err) => {
                    error.set_text(&err.to_string());
                    error.set_visible(true);
                }
            }
        }
    });
    let row = hbox(6);
    row.append(&entry);
    if doc.can_remove_label() {
        let remove = small_button("Remove");
        remove.set_action_name(Some("win.remove-label"));
        row.append(&remove);
    }
    b.append(&row);
    b.append(&error);
    if let Some(l) = &d.label
        && l.source != romlens_ffi::LabelSource::User
    {
        b.append(&note(&if l.source == romlens_ffi::LabelSource::Auto {
            "automatic name".to_owned()
        } else {
            format!("imported from {}", l.origin)
        }));
    }
    b.upcast()
}

pub fn comments(doc: &Rc<Document>, d: &Details) -> gtk::Widget {
    let b = section("Comments");
    let line = gtk::Entry::builder()
        .placeholder_text("Line comment")
        .build();
    if let Some(c) = &d.line_comment {
        line.set_text(&c.text);
    }
    line.connect_activate({
        let doc = Rc::clone(doc);
        move |e| {
            let t = e.text().trim().to_owned();
            let _ = doc.set_comment(CommentKind::Line, (!t.is_empty()).then_some(t));
        }
    });
    b.append(&line);

    let block = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(4)
        .bottom_margin(4)
        .left_margin(6)
        .right_margin(6)
        .build();
    if let Some(c) = &d.block_comment {
        block.buffer().set_text(&c.text);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(48)
        .max_content_height(96)
        .child(&block)
        .build();
    scroll.add_css_class("card");
    b.append(&scroll);
    let foot = hbox(6);
    let hint = note("Block comment above the line");
    hint.set_hexpand(true);
    foot.append(&hint);
    let apply = small_button("Apply");
    apply.connect_clicked({
        let (doc, block) = (Rc::clone(doc), block.clone());
        move |_| {
            let buf = block.buffer();
            let t = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .trim()
                .to_owned();
            let _ = doc.set_comment(CommentKind::Block, (!t.is_empty()).then_some(t));
        }
    });
    foot.append(&apply);
    b.append(&foot);
    b.upcast()
}

// MARK: References

pub fn xrefs(doc: &Rc<Document>, d: &Details) -> gtk::Widget {
    let b = section("References");
    if d.xrefs_to.is_empty() && d.xrefs_from.is_empty() {
        let n = gtk::Label::new(Some("None"));
        n.add_css_class("dim-label");
        n.set_xalign(0.0);
        b.append(&n);
    }
    let link = |text: String, tail: &str, tooltip: Option<&str>, target: Option<u32>| {
        let row = hbox(8);
        let t = mono(&text);
        t.set_selectable(false);
        row.append(&t);
        row.append(&note(tail));
        let button = gtk::Button::builder()
            .child(&row)
            .sensitive(target.is_some())
            .build();
        button.add_css_class("flat");
        if let Some(tip) = tooltip {
            button.set_tooltip_text(Some(tip));
        }
        if let Some(offset) = target {
            let d = Rc::clone(doc);
            button.connect_clicked(move |_| d.jump_to(offset));
        }
        button
    };
    if !d.xrefs_to.is_empty() {
        b.append(&note("Referenced by"));
        for x in d.xrefs_to.iter().take(50) {
            let from = x.from_address.map_or_else(
                || romlens_ffi::format_file_offset(x.from_offset),
                romlens_ffi::format_snes_address,
            );
            let tail = if x.observed {
                format!("{}  seen", x.kind_name)
            } else if !x.certain {
                format!("{}  ?", x.kind_name)
            } else {
                x.kind_name.clone()
            };
            let tip = x
                .observed
                .then_some("An execution log saw the game do this");
            b.append(&link(from, &tail, tip, Some(x.from_offset)));
        }
        if d.xrefs_to.len() > 50 {
            let all = small_button(&format!("Show all {}", d.xrefs_to.len()));
            all.set_action_name(Some("win.find-references"));
            all.set_halign(gtk::Align::Start);
            all.set_tooltip_text(Some(
                "List every reference in the results pane (Ctrl+Shift+F)",
            ));
            b.append(&all);
        }
    }
    if !d.xrefs_from.is_empty() {
        b.append(&note("References"));
        for x in &d.xrefs_from {
            b.append(&link(
                romlens_ffi::format_snes_address(x.to_address),
                &x.kind_name,
                None,
                x.to_offset,
            ));
        }
    }
    b.upcast()
}

// MARK: Byte readings

pub fn byte_readings(doc: &Rc<Document>, byte: &ByteInterpretation, open: bool) -> gtk::Widget {
    let expander = gtk::Expander::new(None);
    let title = heading("Byte readings");
    expander.set_label_widget(Some(&title));
    expander.set_expanded(open);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_top(6);
    if let Some(name) = &byte.span_name {
        let n = gtk::Label::new(Some(name));
        n.add_css_class("heading");
        n.set_xalign(0.0);
        b.append(&n);
        if let Some(v) = &byte.span_value {
            b.append(&note(v));
        }
    }
    let dash = || "—".to_owned();
    let mut items = vec![(
        "File offset",
        romlens_ffi::format_file_offset(byte.file_offset),
    )];
    if byte.disk_offset != byte.file_offset {
        items.push(("On disk", romlens_ffi::format_file_offset(byte.disk_offset)));
    }
    items.push((
        "SNES address",
        byte.snes_address
            .map_or("unmapped".into(), romlens_ffi::format_snes_address),
    ));
    items.push((
        "Mirrors",
        byte.mirrors
            .iter()
            .map(|a| romlens_ffi::format_snes_address(*a))
            .collect::<Vec<_>>()
            .join(", "),
    ));
    items.push((
        "u8",
        format!("{}  (${})", byte.value_u8, hex(u32::from(byte.value_u8), 2)),
    ));
    items.push(("i8", byte.value_i8.to_string()));
    items.push((
        "u16 LE",
        byte.value_u16_le
            .map_or_else(dash, |v| format!("{v}  (${})", hex(u32::from(v), 4))),
    ));
    items.push((
        "i16 LE",
        byte.value_i16_le.map_or_else(dash, |v| v.to_string()),
    ));
    items.push((
        "u24 LE",
        byte.value_u24_le
            .map_or_else(dash, |v| format!("{v}  (${})", hex(v, 6))),
    ));
    items.push((
        "ASCII",
        byte.ascii
            .as_ref()
            .map_or_else(dash, |a| format!("\"{a}\"")),
    ));
    items.push((
        "u16 in bank",
        byte.u16_as_address_in_bank
            .map_or_else(dash, romlens_ffi::format_snes_address),
    ));
    items.push((
        "u24 as address",
        byte.u24_as_snes_address.map_or_else(dash, |a| {
            let mut s = romlens_ffi::format_snes_address(a);
            if let Some(r) = &byte.pointer_target_region {
                s.push_str(&format!("  {r}"));
            }
            s
        }),
    ));
    b.append(&rows(&items));
    for (label, address) in [
        ("Go to u16 in bank", byte.u16_as_address_in_bank),
        ("Go to u24 address", byte.u24_as_snes_address),
    ] {
        if let Some(a) = address.filter(|a| doc.rom.file_offset_for(*a).is_some()) {
            let go = small_button(label);
            go.set_halign(gtk::Align::Start);
            let d = Rc::clone(doc);
            go.connect_clicked(move |_| d.jump_to_snes(a));
            b.append(&go);
        }
    }
    if let Some(target) = byte.pointer_target_file_offset {
        let go = small_button(&format!(
            "Points to {}  Go",
            romlens_ffi::format_file_offset(target)
        ));
        go.set_halign(gtk::Align::Start);
        let d = Rc::clone(doc);
        go.connect_clicked(move |_| d.jump_to(target));
        b.append(&go);
    }
    expander.set_child(Some(&b));
    expander.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::EvidenceInfo;

    fn ev(kind: EvidenceKind, detail: &str, score: f32) -> EvidenceInfo {
        EvidenceInfo {
            kind,
            detail: detail.into(),
            score,
        }
    }

    #[test]
    fn dispatcher_addresses_are_found_in_evidence_text() {
        assert_eq!(
            dispatcher_in("jump table at $80:8423 reads it"),
            Some(0x80_8423)
        );
        assert_eq!(dispatcher_in("dispatch via $7E:00AB"), Some(0x7E_00AB));
        assert_eq!(dispatcher_in("no address here"), None);
        assert_eq!(dispatcher_in("lower case $80:84ab"), None);
        assert_eq!(dispatcher_in("short $80:84"), None);
    }

    #[test]
    fn evidence_leads_with_the_strongest_then_by_trust() {
        let region = RegionInfo {
            start: 0,
            len: 1,
            kind: RegionKind::Data,
            data_kind: None,
            bpp: None,
            stride: None,
            elem: None,
            bank: None,
            confidence: 0.9,
            kind_code: 2,
            name: "byte".into(),
            evidence: vec![
                ev(EvidenceKind::Heuristic, "h", 0.5),
                ev(EvidenceKind::User, "u", 0.5),
                ev(EvidenceKind::Trace, "t", 0.9),
            ],
        };
        let order: Vec<_> = sorted_evidence(&region)
            .iter()
            .map(|e| e.detail.clone())
            .collect();
        assert_eq!(order, ["t", "u", "h"]);
    }

    #[test]
    fn flag_overrides_read_back() {
        let f = FlagOverride {
            m: Some(true),
            x: None,
            e: Some(false),
            dbr: Some(0x7e),
            dp: Some(0x300),
        };
        assert_eq!(flag_text(&f), "M=1 E=0 DBR=$7E DP=$0300");
        assert_eq!(flag_text(&FlagOverride::default()), "");
    }
}
