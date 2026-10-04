//! RGBA bitmaps from the core, drawn with square, unsmoothed pixels. Used by
//! the inspector's preview now and by the graphics views later. The core's
//! bitmaps are RGBA8, top row first; cairo wants premultiplied BGRA.

use gtk::cairo;
use gtk::prelude::*;
use romlens_ffi::BitmapInfo;

/// A cairo surface holding `bitmap`, or `None` for an empty or malformed one.
pub fn surface(bitmap: &BitmapInfo) -> Option<cairo::ImageSurface> {
    let (w, h) = (
        i32::try_from(bitmap.width).ok()?,
        i32::try_from(bitmap.height).ok()?,
    );
    if w == 0 || h == 0 || bitmap.rgba.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mut s = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    let stride = s.stride() as usize;
    {
        let mut data = s.data().ok()?;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let src = (y * w as usize + x) * 4;
                let (r, g, b, a) = (
                    u32::from(bitmap.rgba[src]),
                    u32::from(bitmap.rgba[src + 1]),
                    u32::from(bitmap.rgba[src + 2]),
                    u32::from(bitmap.rgba[src + 3]),
                );
                let dst = y * stride + x * 4;
                // Premultiplied, as cairo stores it; BGRA in memory on
                // little-endian machines, which is every target we build for.
                let pm = |c: u32| ((c * a + 127) / 255) as u8;
                data[dst] = pm(b);
                data[dst + 1] = pm(g);
                data[dst + 2] = pm(r);
                data[dst + 3] = a as u8;
            }
        }
    }
    Some(s)
}

/// The bitmap as PNG bytes, scaled up as a model reads it best, for sending.
pub fn png(bitmap: &BitmapInfo) -> Option<Vec<u8>> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return None;
    }
    let scale = romlens_ffi::tutor::png::scale_for_model(bitmap);
    Some(romlens_ffi::tutor::png::encode(bitmap, scale))
}

/// Whole-number scale that fits `fit_width` (so pixels stay square), 1 to 8.
pub fn fit_scale(width: u32, fit_width: u32) -> i32 {
    ((fit_width / width.max(1)).clamp(1, 8)) as i32
}

/// A widget showing the bitmap at `scale`.
pub fn pixel_image(bitmap: &BitmapInfo, scale: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(bitmap.width as i32 * scale)
        .content_height(bitmap.height as i32 * scale)
        .halign(gtk::Align::Start)
        .build();
    let surface = surface(bitmap);
    area.set_draw_func(move |_, cr, _, _| {
        if let Some(s) = &surface {
            cr.scale(f64::from(scale), f64::from(scale));
            let pattern = cairo::SurfacePattern::create(s);
            pattern.set_filter(cairo::Filter::Nearest);
            let _ = cr.set_source(&pattern);
            let _ = cr.paint();
        }
    });
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(w: u32, h: u32, rgba: Vec<u8>) -> BitmapInfo {
        BitmapInfo {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn converts_rgba_to_premultiplied_bgra() {
        let s = surface(&bitmap(2, 1, vec![255, 128, 0, 255, 255, 0, 0, 128])).unwrap();
        let stride = s.stride() as usize;
        let mut s = s;
        let d = s.data().unwrap();
        assert_eq!(&d[0..4], &[0, 128, 255, 255]);
        // Half-transparent red: premultiplied red is about 128.
        assert_eq!(&d[4..8], &[0, 0, 128, 128]);
        let _ = stride;
    }

    #[test]
    fn rejects_empty_and_short_bitmaps() {
        assert!(surface(&bitmap(0, 0, vec![])).is_none());
        assert!(surface(&bitmap(2, 2, vec![0; 8])).is_none());
    }

    #[test]
    fn scale_fits_in_whole_pixels() {
        assert_eq!(fit_scale(8, 256), 8);
        assert_eq!(fit_scale(128, 256), 2);
        assert_eq!(fit_scale(300, 256), 1);
        assert_eq!(fit_scale(0, 256), 8);
    }
}
