//! The Atlas tab's state and geometry (docs/22, A2): the whole ROM as a map,
//! one bank a row, zoomed continuously from all of it down to single
//! instructions. The geometry is here, without a widget, so the zoom and
//! hit-testing rules are tested. The macOS twin is `AtlasModel` and the
//! geometry half of `AtlasCanvasView`.

/// What a column's colour says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Kind,
    Confidence,
    Entropy,
    Coverage,
}

impl Overlay {
    pub const ALL: [Overlay; 4] = [Self::Kind, Self::Confidence, Self::Entropy, Self::Coverage];

    pub fn title(self) -> &'static str {
        match self {
            Self::Kind => "Kind",
            Self::Confidence => "Confidence",
            Self::Entropy => "Entropy",
            Self::Coverage => "Coverage",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::Kind => {
                "What the analysis says each part is: code, graphics, a table… Faint where it is a guess, hatched where a column blends kinds."
            }
            Self::Confidence => "How sure the analysis is, red for a guess to green for certain.",
            Self::Entropy => {
                "How random the bytes look, dark blue for repetitive to yellow for random: compressed data is the yellow."
            }
            Self::Coverage => "What an imported execution log or trace saw run.",
        }
    }
}

pub const GUTTER: f64 = 44.0;
pub const PAD: f64 = 8.0;
pub const GAP: f64 = 2.0;
pub const MAX_ROW_HEIGHT: f64 = 96.0;
/// Items (instructions, data rows) show from here.
pub const ITEM_ZOOM: f64 = 4.0;
pub const MAX_ZOOM: f64 = 48.0;

/// Where the canvas is looking. `ppb` is pixels a byte across, fitted to the
/// width when zoomed out; rows grow taller as it zooms in, up to a limit, so a
/// deep zoom is a long strip of one bank rather than a wall.
#[derive(Debug, Clone)]
pub struct Geometry {
    pub rom_length: u32,
    pub bank_size: u32,
    pub width: f64,
    pub height: f64,
    pub ppb: f64,
    pub at_fit: bool,
    /// The top-left of the view in the map's coordinates.
    pub origin: (f64, f64),
}

impl Geometry {
    pub fn new(rom_length: u32, bank_size: u32) -> Self {
        Self {
            rom_length: rom_length.max(1),
            bank_size,
            width: 0.0,
            height: 0.0,
            ppb: 0.0,
            at_fit: true,
            origin: (0.0, 0.0),
        }
    }

    pub fn rows(&self) -> usize {
        self.rom_length.div_ceil(self.bank_size) as usize
    }

    pub fn fit_ppb(&self) -> f64 {
        (self.width - GUTTER - PAD).max(1.0) / f64::from(self.bank_size)
    }

    fn fit_row_height(&self) -> f64 {
        (((self.height - 2.0 * PAD) / self.rows().max(1) as f64) - GAP).clamp(4.0, 28.0)
    }

    pub fn row_height(&self) -> f64 {
        let fit = self.fit_ppb();
        if fit <= 0.0 {
            return self.fit_row_height();
        }
        (self.fit_row_height() * self.ppb / fit).clamp(
            self.fit_row_height(),
            MAX_ROW_HEIGHT.max(self.fit_row_height()),
        )
    }

    pub fn pitch(&self) -> f64 {
        self.row_height() + GAP
    }

    pub fn content_size(&self) -> (f64, f64) {
        (
            f64::from(self.bank_size) * self.ppb + PAD,
            2.0 * PAD + self.rows() as f64 * self.pitch(),
        )
    }

    pub fn map_width(&self) -> f64 {
        (self.width - GUTTER).max(1.0)
    }

    pub fn clamp_origin(&mut self) {
        let (cw, ch) = self.content_size();
        self.origin.0 = self.origin.0.clamp(0.0, (cw - self.map_width()).max(0.0));
        self.origin.1 = self.origin.1.clamp(0.0, (ch - self.height).max(0.0));
    }

    pub fn row_y(&self, row: usize) -> f64 {
        PAD + row as f64 * self.pitch() - self.origin.1
    }

    pub fn x_of_byte(&self, b: u32) -> f64 {
        GUTTER + f64::from(b) * self.ppb - self.origin.0
    }

    /// The byte under a point in the view, if any.
    pub fn offset_at(&self, p: (f64, f64)) -> Option<u32> {
        if p.0 < GUTTER || self.ppb <= 0.0 {
            return None;
        }
        let y = p.1 + self.origin.1 - PAD;
        let row = (y / self.pitch()).floor();
        if row < 0.0 || row as usize >= self.rows() || y - row * self.pitch() > self.row_height() {
            return None;
        }
        let b = (p.0 - GUTTER + self.origin.0) / self.ppb;
        if b < 0.0 || b >= f64::from(self.bank_size) {
            return None;
        }
        let off = row as u32 * self.bank_size + b as u32;
        (off < self.rom_length).then_some(off)
    }

    /// Where a byte sits: the middle of its column, its row's middle.
    pub fn point_of(&self, off: u32) -> (f64, f64) {
        let row = (off / self.bank_size) as usize;
        (
            self.x_of_byte(off % self.bank_size) + self.ppb.max(1.0) / 2.0,
            self.row_y(row) + self.row_height() / 2.0,
        )
    }

    /// The canvas was resized.
    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width;
        self.height = height;
        if self.at_fit || self.ppb == 0.0 {
            self.ppb = self.fit_ppb();
            self.origin = (0.0, 0.0);
        }
        self.ppb = self.ppb.max(self.fit_ppb());
        self.clamp_origin();
    }

    pub fn fit(&mut self) {
        self.at_fit = true;
        self.ppb = self.fit_ppb();
        self.origin = (0.0, 0.0);
    }

    /// Zoom by `factor` keeping the byte under `anchor` (the view's centre
    /// when `None`) where it is. Returns whether anything changed.
    pub fn zoom_by(&mut self, factor: f64, anchor: Option<(f64, f64)>) -> bool {
        if self.ppb <= 0.0 {
            return false;
        }
        let a = anchor.unwrap_or((GUTTER + self.map_width() / 2.0, self.height / 2.0));
        let byte_x = (a.0 - GUTTER + self.origin.0) / self.ppb;
        let row_f = (a.1 + self.origin.1 - PAD) / self.pitch();
        let next = (self.ppb * factor).max(self.fit_ppb()).min(MAX_ZOOM);
        if (next - self.ppb).abs() < f64::EPSILON {
            return false;
        }
        self.ppb = next;
        self.at_fit = (self.ppb - self.fit_ppb()).abs() < 1e-9;
        self.origin.0 = byte_x * self.ppb - (a.0 - GUTTER);
        self.origin.1 = PAD + row_f * self.pitch() - a.1;
        self.clamp_origin();
        true
    }

    /// Scroll so `off` is in view, unless it is already.
    pub fn reveal(&mut self, off: u32) {
        if off >= self.rom_length || self.width <= 0.0 {
            return;
        }
        let p = self.point_of(off);
        let inside = p.0 >= GUTTER && p.0 <= self.width && p.1 >= 0.0 && p.1 <= self.height;
        if inside {
            return;
        }
        self.origin.0 += p.0 - (GUTTER + self.map_width() / 2.0);
        self.origin.1 += p.1 - self.height / 2.0;
        self.clamp_origin();
    }

    pub fn scroll_by(&mut self, dx: f64, dy: f64) {
        self.origin.0 += dx;
        self.origin.1 += dy;
        self.clamp_origin();
    }

    /// The byte range each visible row shows: `(row, rows' first offset,
    /// first byte in view, one past the last)`.
    pub fn visible_rows(&self) -> Vec<(usize, u32, u32, u32)> {
        if self.ppb <= 0.0 {
            return Vec::new();
        }
        let b0 = (self.origin.0 / self.ppb).floor().max(0.0) as u32;
        (0..self.rows())
            .filter_map(|row| {
                let y = self.row_y(row);
                if y + self.row_height() < 0.0 || y > self.height {
                    return None;
                }
                let start = row as u32 * self.bank_size;
                let len = self.bank_size.min(self.rom_length - start);
                let end = len.min(((self.origin.0 + self.map_width()) / self.ppb).ceil() as u32);
                (b0 < end).then_some((row, start, b0, end))
            })
            .collect()
    }
}

/// The state the menu and header share with the canvas.
#[derive(Debug, Clone, Copy)]
pub struct AtlasState {
    pub overlay: Overlay,
}

impl Default for AtlasState {
    fn default() -> Self {
        Self {
            overlay: Overlay::Kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1 MB LoROM: 32 banks of 32 KB, in a 900 by 700 view.
    fn geo() -> Geometry {
        let mut g = Geometry::new(0x10_0000, 0x8000);
        g.resize(900.0, 700.0);
        g
    }

    #[test]
    fn it_starts_fitted_with_one_bank_across_the_width() {
        let g = geo();
        assert_eq!(g.rows(), 32);
        assert!(g.at_fit);
        let expected = (900.0 - GUTTER - PAD) / 32768.0;
        assert!((g.ppb - expected).abs() < 1e-9);
        // The last byte of a bank lands at the right edge of the map.
        let right = g.x_of_byte(0x7FFF);
        assert!(right <= 900.0 && right > 900.0 - 2.0 * PAD);
    }

    #[test]
    fn a_point_hits_the_byte_under_it_and_points_come_back() {
        let g = geo();
        assert_eq!(
            g.offset_at((GUTTER - 1.0, 100.0)),
            None,
            "the gutter is not the map"
        );
        let p = g.point_of(0x1_2345);
        assert_eq!(g.offset_at(p).map(|o| o / 32), Some(0x1_2345 / 32));
        // A pixel is about forty bytes at this zoom.
        assert!(g.offset_at((GUTTER + 1.0, g.row_y(0) + 1.0)).unwrap() < 64);
        // The gap between rows is not a byte.
        assert_eq!(g.offset_at((GUTTER + 5.0, g.row_y(1) - 1.0)), None);
        assert_eq!(g.offset_at((GUTTER + 5.0, 5000.0)), None);
    }

    #[test]
    fn the_last_row_is_short_when_the_rom_is_not_a_whole_bank() {
        let mut g = Geometry::new(0x8000 + 100, 0x8000);
        g.resize(900.0, 300.0);
        assert_eq!(g.rows(), 2);
        let second = g.visible_rows().into_iter().find(|r| r.0 == 1).unwrap();
        assert_eq!((second.1, second.2), (0x8000, 0));
        assert!(second.3 <= 100);
        let near_the_end = 0x8000 + 40;
        assert_eq!(
            g.offset_at(g.point_of(near_the_end)).map(|o| o / 64),
            Some(near_the_end / 64)
        );
        assert_eq!(g.offset_at((GUTTER + 600.0, g.row_y(1) + 1.0)), None);
    }

    #[test]
    fn zooming_keeps_the_byte_under_the_pointer_where_it_is() {
        let mut g = geo();
        let anchor = (400.0, 300.0);
        let before = g.offset_at(anchor).unwrap();
        assert!(g.zoom_by(8.0, Some(anchor)));
        assert!(!g.at_fit);
        let after = g.offset_at(anchor).unwrap();
        // The same bank, and within the width of one pixel of the same byte.
        assert_eq!(before / 0x8000, after / 0x8000);
        assert!(
            f64::from(before.abs_diff(after)) <= 2.0 / g.ppb + 1.0,
            "{before} {after}"
        );
        g.fit();
        assert!(g.at_fit && g.origin == (0.0, 0.0));
    }

    #[test]
    fn zoom_is_bounded_and_rows_grow_with_it() {
        let mut g = geo();
        let fit_row = g.row_height();
        for _ in 0..40 {
            g.zoom_by(1.5, None);
        }
        assert!((g.ppb - MAX_ZOOM).abs() < 1e-9);
        assert!(g.row_height() > fit_row && g.row_height() <= MAX_ROW_HEIGHT);
        // Zooming out never goes past the fit.
        for _ in 0..80 {
            g.zoom_by(1.0 / 1.5, None);
        }
        assert!((g.ppb - g.fit_ppb()).abs() < 1e-9 && g.at_fit);
        assert!(!g.zoom_by(0.5, None), "nothing to do at the fit");
    }

    #[test]
    fn the_origin_stays_inside_the_content() {
        let mut g = geo();
        g.zoom_by(20.0, None);
        g.scroll_by(-1e9, -1e9);
        assert_eq!(g.origin, (0.0, 0.0));
        g.scroll_by(1e9, 1e9);
        let (cw, ch) = g.content_size();
        assert!((g.origin.0 - (cw - g.map_width())).abs() < 1e-6);
        assert!((g.origin.1 - (ch - g.height).max(0.0)).abs() < 1e-6);
    }

    #[test]
    fn reveal_scrolls_only_when_the_byte_is_out_of_view() {
        let mut g = geo();
        g.zoom_by(20.0, None);
        let off = 0xF_1000;
        g.reveal(off);
        let p = g.point_of(off);
        assert!(
            p.0 >= GUTTER && p.0 <= g.width && p.1 >= 0.0 && p.1 <= g.height,
            "{p:?}"
        );
        let origin = g.origin;
        g.reveal(off);
        assert_eq!(g.origin, origin, "already in view");
        g.reveal(u32::MAX);
        assert_eq!(g.origin, origin, "a byte outside the ROM is ignored");
    }

    #[test]
    fn only_the_visible_rows_and_columns_are_asked_for() {
        let mut g = geo();
        let all = g.visible_rows();
        assert_eq!(all.len(), 32);
        assert!(all.iter().all(|r| r.2 == 0 && r.3 == 0x8000));
        g.zoom_by(16.0, None);
        let some = g.visible_rows();
        assert!(some.len() < 32);
        assert!(some.iter().all(|r| r.2 > 0 || r.3 < 0x8000));
    }

    #[test]
    fn the_overlays_are_named() {
        assert_eq!(Overlay::ALL.len(), 4);
        for o in Overlay::ALL {
            assert!(!o.title().is_empty() && !o.help().is_empty());
        }
    }
}
