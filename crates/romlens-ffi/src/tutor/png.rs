//! PNG for the pictures a tool shows the model (docs/24). Written from the
//! PNG specification: a palette when the picture has 256 colours or fewer
//! (an SNES screen always does without colour math), else RGBA; the image
//! data in stored deflate blocks, which every decoder reads. Never written
//! to a file for the student (`12-content-policy.md` rules 5 and 8).

use crate::graphics::BitmapInfo;

/// Small pictures are scaled up by whole pixels so a model sees each one;
/// the long side stays at or under 768.
pub fn scale_for_model(b: &BitmapInfo) -> u32 {
    let long = b.width.max(b.height).max(1);
    (512 / long).clamp(1, 8).min((768 / long).max(1))
}

pub fn encode(b: &BitmapInfo, scale: u32) -> Vec<u8> {
    let s = scale.max(1);
    let (w, h) = (b.width * s, b.height * s);
    let px = |x: u32, y: u32| {
        let i = (((y / s) * b.width + x / s) * 4) as usize;
        [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2], b.rgba[i + 3]]
    };
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut indexed = true;
    'scan: for y in 0..b.height {
        for x in 0..b.width {
            let c = px(x * s, y * s);
            if !palette.contains(&c) {
                if palette.len() == 256 {
                    indexed = false;
                    break 'scan;
                }
                palette.push(c);
            }
        }
    }
    let mut raw = Vec::with_capacity(((w * if indexed { 1 } else { 4 } + 1) * h) as usize);
    for y in 0..h {
        raw.push(0); // no filter
        for x in 0..w {
            let c = px(x, y);
            if indexed {
                raw.push(palette.iter().position(|p| *p == c).unwrap() as u8);
            } else {
                raw.extend_from_slice(&c);
            }
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, if indexed { 3 } else { 6 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if indexed {
        let plte: Vec<u8> = palette.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
        chunk(&mut out, b"PLTE", &plte);
        if palette.iter().any(|c| c[3] != 255) {
            let trns: Vec<u8> = palette.iter().map(|c| c[3]).collect();
            chunk(&mut out, b"tRNS", &trns);
        }
    }
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let mut blocks = data.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(b) = blocks.next() {
        z.push(if blocks.peek().is_none() { 1 } else { 0 });
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut bb) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65_521;
        bb = (bb + a) % 65_521;
    }
    z.extend_from_slice(&((bb << 16) | a).to_be_bytes());
    z
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A short stable id for a picture's bytes.
pub fn id(bytes: &[u8]) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("tool-{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_adler_match_the_references() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        let z = zlib_stored(b"Wikipedia");
        assert_eq!(&z[z.len() - 4..], &0x11E6_0398u32.to_be_bytes());
    }

    #[test]
    fn a_small_picture_is_indexed_and_scaled() {
        let mut rgba = Vec::new();
        for i in 0..16u8 {
            rgba.extend_from_slice(&[i * 16, 0, 255 - i * 16, if i == 0 { 0 } else { 255 }]);
        }
        let b = BitmapInfo {
            width: 4,
            height: 4,
            rgba,
        };
        let s = scale_for_model(&b);
        assert_eq!(s, 8);
        let png = encode(&b, s);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..24], &[0, 0, 0, 32, 0, 0, 0, 32]);
        assert_eq!(png[25], 3, "indexed");
        assert!(png.windows(4).any(|w| w == b"tRNS"));
        // macOS's own decoder reads it where one is at hand.
        let path = std::env::temp_dir().join("romlens-tutor-png-test.png");
        std::fs::write(&path, &png).unwrap();
        if let Ok(o) = std::process::Command::new("sips")
            .args(["-g", "pixelWidth"])
            .arg(&path)
            .output()
        {
            let text = String::from_utf8_lossy(&o.stdout);
            assert!(text.contains("pixelWidth: 32"), "{text}");
        }
        let _ = std::fs::remove_file(path);
    }
}
