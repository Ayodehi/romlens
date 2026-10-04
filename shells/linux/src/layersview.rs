//! The frame's layers apart (checklist 3.13): each background the mode has
//! and the sprites, alone where they show on the main screen, in a grid two
//! wide (three when a mode has four backgrounds), with the mode's order front
//! to back. The macOS twin is `LayersView`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use romlens_ffi::FrameLayersInfo;

use crate::canvas::with_alpha;
use crate::gfxdraw::*;
use crate::model::graphics as gfx;
use crate::model::{Change, Document};
use crate::pixels;

const SPACING: f64 = 16.0;
const HEADER: f64 = 22.0;

struct Panel {
    title: String,
    detail: String,
    surface: Option<cairo::ImageSurface>,
}

struct View {
    doc: Rc<Document>,
    area: gtk::DrawingArea,
    scroll: gtk::ScrolledWindow,
    fit: gtk::ToggleButton,
    actual: gtk::ToggleButton,
    colour_math: gtk::CheckButton,
    legend: gtk::Box,
    panels: RefCell<Vec<Panel>>,
    /// The size of one layer's image.
    size: Cell<(u32, u32)>,
    key: RefCell<String>,
    syncing: Cell<bool>,
}

pub fn build(doc: &Rc<Document>) -> gtk::Widget {
    let (root, view) = View::build(doc);
    root.connect_destroy(move |_| {
        let _ = &view;
    });
    root
}

fn columns_for(n: usize) -> usize {
    if n > 4 { 3 } else { 2 }
}

impl View {
    fn build(doc: &Rc<Document>) -> (gtk::Widget, Rc<Self>) {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&area)
            .build();
        let fit = gtk::ToggleButton::with_label("Fit");
        let actual = gtk::ToggleButton::with_label("1:1");
        actual.set_group(Some(&fit));
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        group.add_css_class("linked");
        group.append(&fit);
        group.append(&actual);
        let colour_math = gtk::CheckButton::with_label("Colour math");
        colour_math.set_tooltip_text(Some(
            "Show each layer with its colour math done, as on screen: a gradient made by \
             adding or subtracting a colour (CGADSUB, COLDATA) shows. Off: the layer's own colours.",
        ));
        let controls = gtk::Box::builder()
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        controls.append(&group);
        controls.append(&colour_math);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&controls);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&scroll);
        let legend = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .build();
        let legend_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(240)
            .child(&legend)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&left);
        root.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        root.append(&legend_scroll);

        let view = Rc::new(Self {
            doc: Rc::clone(doc),
            area,
            scroll,
            fit,
            actual,
            colour_math,
            legend,
            panels: RefCell::new(Vec::new()),
            size: Cell::new((256, 224)),
            key: RefCell::new(String::new()),
            syncing: Cell::new(false),
        });
        view.wire();
        view.refresh();
        (root.upcast(), view)
    }

    fn wire(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.area.set_draw_func(move |a, cr, w, h| {
            if let Some(v) = this.upgrade() {
                v.draw(a, cr, f64::from(w), f64::from(h));
            }
        });
        for (b, fit) in [(&self.fit, true), (&self.actual, false)] {
            let this = Rc::downgrade(self);
            b.connect_clicked(move |b| {
                if b.is_active()
                    && let Some(v) = this.upgrade()
                    && !v.syncing.get()
                {
                    v.doc.edit_graphics(|g| {
                        g.layers_fit = fit;
                        None
                    });
                }
            });
        }
        let this = Rc::downgrade(self);
        self.colour_math.connect_toggled(move |b| {
            if let Some(v) = this.upgrade()
                && !v.syncing.get()
            {
                let on = b.is_active();
                v.doc.edit_graphics(|g| {
                    g.layers_colour_math = on;
                    None
                });
            }
        });
        let this = Rc::downgrade(self);
        self.doc.subscribe(move |c| {
            if matches!(c, Change::Graphics | Change::Layout)
                && let Some(v) = this.upgrade()
            {
                v.refresh();
            }
        });
    }

    fn refresh(&self) {
        if self.doc.graphics_tab() != Some(gfx::Tab::Layers) {
            return;
        }
        let (frame, math, fit, has) = {
            let g = self.doc.graphics();
            (
                g.frame(),
                g.layers_colour_math,
                g.layers_fit,
                g.has_recording(),
            )
        };
        self.syncing.set(true);
        self.colour_math.set_active(math);
        self.fit.set_active(fit);
        self.actual.set_active(!fit);
        self.syncing.set(false);
        let key = format!(
            "{frame}|{math}|{has}|{}",
            self.doc.graphics().source_description()
        );
        if *self.key.borrow() != key {
            *self.key.borrow_mut() = key;
            self.rebuild();
        }
        self.scroll.set_policy(
            if fit {
                gtk::PolicyType::Never
            } else {
                gtk::PolicyType::Automatic
            },
            if fit {
                gtk::PolicyType::Never
            } else {
                gtk::PolicyType::Automatic
            },
        );
        self.resize();
        self.area.queue_draw();
    }

    fn rebuild(&self) {
        let g = self.doc.graphics();
        let info = g.frame_layers();
        let mut panels = Vec::new();
        let mut size = None;
        if let Some(info) = &info {
            for l in &info.layers {
                let bitmap = g.frame_layer(l.layer);
                if size.is_none()
                    && let Some(b) = &bitmap
                {
                    size = Some((b.width, b.height));
                }
                panels.push(Panel {
                    title: if l.layer == 5 {
                        "Sprites".to_owned()
                    } else {
                        format!("BG{}", l.layer)
                    },
                    detail: l.detail.clone(),
                    surface: bitmap.as_ref().and_then(pixels::surface),
                });
            }
        }
        drop(g);
        self.size.set(size.unwrap_or((256, 224)));
        *self.panels.borrow_mut() = panels;
        self.fill_legend(info.as_ref());
    }

    fn fill_legend(&self, info: Option<&FrameLayersInfo>) {
        while let Some(c) = self.legend.first_child() {
            self.legend.remove(&c);
        }
        let head = gtk::Label::builder()
            .label("Front to back")
            .xalign(0.0)
            .build();
        head.add_css_class("heading");
        self.legend.append(&head);
        let spans = info.map_or(&[][..], |i| &i.spans[..]);
        let wrapped = |text: &str| {
            let l = caption(text);
            l.set_wrap(true);
            l
        };
        if spans.len() > 1 {
            self.legend.append(&wrapped(
                "The mode changes part way down the screen; each part puts its layers in its own order, and each pixel shows the first that draws there.",
            ));
        }
        for span in spans {
            if spans.len() > 1 {
                let t = gtk::Label::builder()
                    .label(format!(
                        "Lines {}-{}: Mode {}",
                        span.first_line, span.last_line, span.mode
                    ))
                    .xalign(0.0)
                    .margin_top(4)
                    .build();
                t.add_css_class("heading");
                self.legend.append(&t);
            } else {
                self.legend.append(&wrapped(&format!(
                    "Mode {} puts the layers in this order; each pixel shows the first that draws there.",
                    span.mode
                )));
            }
            for (i, name) in span.order.iter().enumerate() {
                let l = gtk::Label::builder()
                    .label(format!("{}. {name}", i + 1))
                    .xalign(0.0)
                    .build();
                self.legend.append(&l);
            }
        }
        self.legend
            .append(&wrapped("then the backdrop, CGRAM colour 0"));
    }

    /// The scale a layer is drawn at: fitted to the area, or 1:1.
    fn scale(&self, w: f64, h: f64) -> f64 {
        let n = self.panels.borrow().len().max(1);
        let (cols, rows) = (columns_for(n), n.div_ceil(columns_for(n)));
        let (iw, ih) = self.size.get();
        if !self.doc.graphics().layers_fit {
            return 1.0;
        }
        let across = (w - SPACING * (cols as f64 + 1.0)) / cols as f64 / f64::from(iw);
        let down = (h - SPACING * (rows as f64 + 1.0) - HEADER * rows as f64)
            / rows as f64
            / f64::from(ih);
        across.min(down).max(0.25)
    }

    fn resize(&self) {
        if self.doc.graphics().layers_fit {
            self.area.set_content_width(0);
            self.area.set_content_height(0);
            return;
        }
        let n = self.panels.borrow().len().max(1);
        let (cols, rows) = (columns_for(n), n.div_ceil(columns_for(n)));
        let (iw, ih) = self.size.get();
        self.area
            .set_content_width((cols as f64 * (f64::from(iw) + SPACING) + SPACING) as i32);
        self.area.set_content_height(
            (rows as f64 * (f64::from(ih) + SPACING + HEADER) + SPACING) as i32,
        );
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
        let panels = self.panels.borrow();
        let n = panels.len();
        let cols = columns_for(n);
        let scale = self.scale(w, h);
        let (iw, ih) = self.size.get();
        let (pw, ph) = (f64::from(iw) * scale, f64::from(ih) * scale);
        let fg = a.color();
        for (i, p) in panels.iter().enumerate() {
            let (col, row) = (i % cols, i / cols);
            let x = SPACING + col as f64 * (pw + SPACING);
            let y = SPACING + row as f64 * (ph + SPACING + HEADER);
            let title = mono_layout(a, 10.5, &p.title);
            text(cr, &title, x, y, &fg);
            let (tw, _) = title.pixel_size();
            let detail = mono_layout(a, 8.5, &p.detail);
            text(
                cr,
                &detail,
                x + f64::from(tw) + 8.0,
                y + 3.0,
                &with_alpha(&fg, 0.6),
            );
            cr.save().ok();
            cr.translate(x, y + HEADER - 4.0);
            checker(cr, pw, ph, &fg);
            if let Some(s) = &p.surface {
                paint_surface(cr, s, scale);
            }
            cr.restore().ok();
        }
    }
}
