//! The overview strip's columns, decoded from the core's flat batch
//! (`viewmodel::region_summary::encode_summary`). The macOS twin is
//! `RegionStrip`.

const HEADER_LEN: usize = 8;
const RECORD_LEN: usize = 16;
const VERSION: u8 = 1;

/// One column of the overview strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripColumn {
    pub start: u32,
    pub len: u32,
    pub kind_code: u8,
    pub confidence: f64,
    pub share: f64,
    pub entropy: f64,
    pub executed: f64,
}

impl StripColumn {
    /// A column that blends kinds, which is hatched rather than painted flat:
    /// "this bank is mostly code" must not read as "this bank is code".
    pub fn mixed(&self) -> bool {
        self.share < 0.9
    }
}

pub fn decode(bytes: &[u8]) -> Vec<StripColumn> {
    if bytes.len() < HEADER_LEN || bytes[0] != VERSION {
        return Vec::new();
    }
    let count = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    if bytes.len() < HEADER_LEN + count * RECORD_LEN {
        return Vec::new();
    }
    (0..count)
        .map(|i| {
            let at = HEADER_LEN + i * RECORD_LEN;
            let u32_at = |o: usize| {
                u32::from_le_bytes([
                    bytes[at + o],
                    bytes[at + o + 1],
                    bytes[at + o + 2],
                    bytes[at + o + 3],
                ])
            };
            StripColumn {
                start: u32_at(0),
                len: u32_at(4),
                kind_code: bytes[at + 8],
                confidence: f64::from(bytes[at + 9]) / 255.0,
                share: f64::from(bytes[at + 10]) / 255.0,
                entropy: f64::from(bytes[at + 11]) / 32.0,
                executed: f64::from(bytes[at + 12]) / 255.0,
            }
        })
        .collect()
}

pub fn kind_name(code: u8) -> &'static str {
    match code {
        0 => "unknown",
        1 => "code",
        2 => "byte",
        3 => "word",
        4 => "long",
        5 => "pointer",
        6 => "table",
        7 => "string",
        8 => "graphics",
        9 => "tilemap",
        10 => "palette",
        11 => "compressed",
        13 => "sample",
        _ => "struct",
    }
}

/// The column under a horizontal position in a strip `width` wide.
pub fn column_at(x: f64, width: f64, columns: usize) -> Option<usize> {
    if width <= 0.0 || columns == 0 {
        return None;
    }
    Some(((x / width * columns as f64).floor().max(0.0) as usize).min(columns - 1))
}

/// The file offset a scrub at `x` should jump to.
pub fn offset_at(x: f64, width: f64, rom_len: u32) -> Option<u32> {
    if width <= 0.0 || rom_len == 0 {
        return None;
    }
    let fraction = (x / width).clamp(0.0, 1.0);
    Some((f64::from(rom_len - 1) * fraction) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(columns: &[(u32, u32, u8)]) -> Vec<u8> {
        let mut d = vec![0u8; HEADER_LEN];
        d[0] = VERSION;
        d[2..4].copy_from_slice(&(columns.len() as u16).to_le_bytes());
        for (start, len, kind) in columns {
            let mut r = [0u8; RECORD_LEN];
            r[0..4].copy_from_slice(&start.to_le_bytes());
            r[4..8].copy_from_slice(&len.to_le_bytes());
            r[8] = *kind;
            r[9] = 255;
            r[10] = 128;
            r[11] = 64;
            r[12] = 51;
            d.extend_from_slice(&r);
        }
        d
    }

    #[test]
    fn decodes_columns() {
        let cols = decode(&batch(&[(0, 100, 1), (100, 50, 8)]));
        assert_eq!(cols.len(), 2);
        assert_eq!(
            (cols[1].start, cols[1].len, cols[1].kind_code),
            (100, 50, 8)
        );
        assert!((cols[0].confidence - 1.0).abs() < 1e-9);
        assert!((cols[0].entropy - 2.0).abs() < 1e-9);
        assert!((cols[0].executed - 0.2).abs() < 1e-9);
        assert!(cols[0].mixed());
    }

    #[test]
    fn rejects_malformed_batches() {
        assert!(decode(&[]).is_empty());
        let mut b = batch(&[(0, 1, 1)]);
        b[0] = 2;
        assert!(decode(&b).is_empty());
        let mut b = batch(&[(0, 1, 1)]);
        b.truncate(HEADER_LEN + 4);
        assert!(decode(&b).is_empty());
    }

    #[test]
    fn hit_testing_clamps() {
        assert_eq!(column_at(0.0, 100.0, 10), Some(0));
        assert_eq!(column_at(55.0, 100.0, 10), Some(5));
        assert_eq!(column_at(500.0, 100.0, 10), Some(9));
        assert_eq!(column_at(-5.0, 100.0, 10), Some(0));
        assert_eq!(column_at(5.0, 0.0, 10), None);
        assert_eq!(offset_at(0.0, 100.0, 1000), Some(0));
        assert_eq!(offset_at(100.0, 100.0, 1000), Some(999));
        assert_eq!(offset_at(250.0, 100.0, 1000), Some(999));
        assert_eq!(offset_at(1.0, 100.0, 0), None);
    }

    #[test]
    fn the_core_batch_decodes() {
        let rom = crate::model::testing::test_rom();
        let wb = romlens_ffi::Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let cols = decode(&wb.region_map(64));
        assert_eq!(cols.len(), 64);
        // The columns tile the image.
        assert_eq!(cols[0].start, 0);
        assert_eq!(cols.iter().map(|c| c.len).sum::<u32>(), 32768);
        assert!(cols.windows(2).all(|w| w[0].start + w[0].len == w[1].start));
    }
}
