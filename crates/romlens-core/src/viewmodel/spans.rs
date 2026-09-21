//! Named, typed byte ranges the hex view overlays: header fields and vectors
//! in Phase 0, regions later.

use crate::memory::address::{FileOffset, SnesAddress};
use crate::rom::header::{EXTENDED_HEADER_LEN, TITLE_LEN};
use crate::rom::image::RomImage;

/// Semantic kind of a span; shells map kinds to colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpanKind {
    Title,
    MapMode,
    CartridgeType,
    RomSize,
    RamSize,
    Region,
    DeveloperId,
    Version,
    ChecksumComplement,
    Checksum,
    NativeVector,
    EmulationVector,
    ExtendedHeader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// 1-based id; the hex-row encoder writes it per byte (0 = no span).
    pub id: u32,
    pub start: FileOffset,
    pub len: u32,
    pub kind: SpanKind,
    pub name: String,
    /// Human-readable decoded value.
    pub value_text: String,
    /// Where a vector or pointer points, when it points somewhere.
    pub target: Option<SnesAddress>,
}

impl Span {
    pub fn end(&self) -> FileOffset {
        FileOffset(self.start.0 + self.len)
    }

    pub fn contains(&self, off: FileOffset) -> bool {
        off >= self.start && off < self.end()
    }
}

/// Per-byte span lookup over the range the spans cover.
#[derive(Debug, Clone, Default)]
pub struct SpanIndex {
    start: u32,
    ids: Vec<u8>,
}

impl SpanIndex {
    pub fn new(spans: &[Span]) -> Self {
        let Some(start) = spans.iter().map(|s| s.start.0).min() else {
            return Self::default();
        };
        let end = spans.iter().map(|s| s.end().0).max().unwrap_or(start);
        let mut ids = vec![0u8; (end - start) as usize];
        for span in spans {
            let id = u8::try_from(span.id).unwrap_or(u8::MAX);
            for b in span.start.0..span.end().0 {
                ids[(b - start) as usize] = id;
            }
        }
        Self { start, ids }
    }

    /// Span id covering `off`, 0 when none.
    pub fn id_at(&self, off: u32) -> u8 {
        off.checked_sub(self.start)
            .and_then(|i| self.ids.get(i as usize))
            .copied()
            .unwrap_or(0)
    }

    /// Whether any byte of `[off, off + len)` carries a span.
    pub fn any_in(&self, off: u32, len: u32) -> bool {
        (off..off + len).any(|o| self.id_at(o) != 0)
    }
}

/// Spans for the internal header and vector table of an image.
pub fn header_spans(rom: &RomImage) -> Vec<Span> {
    let h = rom.header();
    let base = rom.header_offset().0;
    // Vectors are 16-bit and fetched from bank $00; show the canonical bank.
    let vector_target = |value: u16| {
        rom.file_offset_for(SnesAddress::new(0x00, value))
            .and_then(|off| rom.snes_address_for(off))
    };
    let mut spans = Vec::with_capacity(24);
    let mut push = |start: u32,
                    len: u32,
                    kind: SpanKind,
                    name: &str,
                    value: String,
                    target: Option<SnesAddress>| {
        spans.push(Span {
            id: spans.len() as u32 + 1,
            start: FileOffset(start),
            len,
            kind,
            name: name.to_owned(),
            value_text: value,
            target,
        });
    };

    if let Some(ext) = &h.extended {
        push(
            base - EXTENDED_HEADER_LEN as u32,
            EXTENDED_HEADER_LEN as u32,
            SpanKind::ExtendedHeader,
            "Extended header",
            format!("maker {:?}, game {:?}", ext.maker_code, ext.game_code),
            None,
        );
    }
    push(
        base,
        TITLE_LEN as u32,
        SpanKind::Title,
        "Title",
        format!("{:?}", h.title),
        None,
    );
    push(
        base + 0x15,
        1,
        SpanKind::MapMode,
        "Map mode",
        format!(
            "${:02X} = {}{}",
            h.map_mode,
            h.mapping().map_or("unknown mapping", |m| m.name()),
            if h.is_fast_rom() {
                ", FastROM"
            } else {
                ", SlowROM"
            }
        ),
        None,
    );
    push(
        base + 0x16,
        1,
        SpanKind::CartridgeType,
        "Cartridge type",
        format!("${:02X} = {}", h.cartridge_type, h.cartridge_type_name()),
        None,
    );
    push(
        base + 0x17,
        1,
        SpanKind::RomSize,
        "ROM size",
        format!(
            "${:02X} = {} KB declared",
            h.rom_size_code,
            h.declared_rom_size() / 1024
        ),
        None,
    );
    push(
        base + 0x18,
        1,
        SpanKind::RamSize,
        "RAM size",
        format!(
            "${:02X} = {} KB",
            h.ram_size_code,
            h.declared_ram_size() / 1024
        ),
        None,
    );
    push(
        base + 0x19,
        1,
        SpanKind::Region,
        "Region",
        format!("${:02X} = {}", h.region, h.region_name()),
        None,
    );
    push(
        base + 0x1A,
        1,
        SpanKind::DeveloperId,
        "Developer id",
        format!("${:02X}", h.developer_id),
        None,
    );
    push(
        base + 0x1B,
        1,
        SpanKind::Version,
        "Version",
        format!("1.{}", h.version),
        None,
    );
    push(
        base + 0x1C,
        2,
        SpanKind::ChecksumComplement,
        "Checksum complement",
        format!("${:04X}", h.complement),
        None,
    );
    push(
        base + 0x1E,
        2,
        SpanKind::Checksum,
        "Checksum",
        format!(
            "${:04X} ({})",
            h.checksum,
            if rom.checksum_ok() {
                "valid"
            } else {
                "mismatch"
            }
        ),
        None,
    );
    for (i, (name, value)) in h.native.named(true).into_iter().enumerate() {
        push(
            base + 0x24 + 2 * i as u32,
            2,
            SpanKind::NativeVector,
            &format!("Native {name}"),
            format!("${value:04X}"),
            vector_target(value),
        );
    }
    for (i, (name, value)) in h.emulation.named(false).into_iter().enumerate() {
        push(
            base + 0x34 + 2 * i as u32,
            2,
            SpanKind::EmulationVector,
            &format!("Emulation {name}"),
            format!("${value:04X}"),
            vector_target(value),
        );
    }
    spans
}
