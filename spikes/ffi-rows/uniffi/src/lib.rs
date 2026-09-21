use std::sync::Arc;
uniffi::setup_scaffolding!();

#[derive(uniffi::Object)]
pub struct Rom { bytes: Vec<u8> }

#[derive(uniffi::Record)]
pub struct HexRow { pub file_offset: u32, pub bytes: Vec<u8>, pub ascii: String }

#[uniffi::export]
impl Rom {
    #[uniffi::constructor]
    pub fn open(path: String) -> Arc<Self> { Arc::new(Rom { bytes: std::fs::read(path).expect("rom") }) }
    pub fn size(&self) -> u64 { self.bytes.len() as u64 }
    /// Record per row: what a naive generated API looks like.
    pub fn hex_rows(&self, start_row: u32, count: u32) -> Vec<HexRow> {
        let total_rows = (self.bytes.len() + 15) / 16;
        let n = (count as usize).min(total_rows.saturating_sub(start_row as usize));
        (0..n).map(|i| {
            let off = (start_row as usize + i) * 16;
            let src = &self.bytes[off..(off + 16).min(self.bytes.len())];
            HexRow { file_offset: off as u32, bytes: src.to_vec(),
                     ascii: src.iter().map(|b| if (32..127).contains(b) { *b as char } else { '.' }).collect() }
        }).collect()
    }
    /// One flat blob per batch: 36 bytes per row, the cheap generated shape.
    pub fn hex_rows_blob(&self, start_row: u32, count: u32) -> Vec<u8> {
        let total_rows = (self.bytes.len() + 15) / 16;
        let n = (count as usize).min(total_rows.saturating_sub(start_row as usize));
        let mut out = Vec::with_capacity(n * 36);
        for i in 0..n {
            let off = (start_row as usize + i) * 16;
            let src = &self.bytes[off..(off + 16).min(self.bytes.len())];
            out.extend_from_slice(&(off as u32).to_le_bytes());
            let mut b = [0u8; 16]; b[..src.len()].copy_from_slice(src); out.extend_from_slice(&b);
            let mut a = [b'.'; 16]; for (k, c) in src.iter().enumerate() { if (32..127).contains(c) { a[k] = *c; } } out.extend_from_slice(&a);
        }
        out
    }
    /// Pre-formatted text per batch.
    pub fn hex_rows_text(&self, start_row: u32, count: u32) -> String {
        use std::fmt::Write;
        let total_rows = (self.bytes.len() + 15) / 16;
        let n = (count as usize).min(total_rows.saturating_sub(start_row as usize));
        let mut s = String::with_capacity(n * 80);
        for i in 0..n {
            let off = (start_row as usize + i) * 16;
            let src = &self.bytes[off..(off + 16).min(self.bytes.len())];
            let _ = write!(s, "{:06X}  ", off);
            for b in src { let _ = write!(s, "{:02X} ", b); }
            s.push(' ');
            for b in src { s.push(if (32..127).contains(b) { *b as char } else { '.' }); }
            s.push('\n');
        }
        s
    }
}
