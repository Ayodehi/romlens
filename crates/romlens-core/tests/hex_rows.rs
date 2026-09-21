mod common;

use romlens_core::fixtures;
use romlens_core::viewmodel::hex_rows::{FLAG_HAS_SPAN, FLAG_LAST_ROW};
use romlens_core::{
    AddressStyle, BATCH_HEADER_LEN, FileOffset, ROW_STRIDE, ROW_VERSION, RomImage, SnesAddress,
    SpanIndex, SpanKind, encode_rows, format_rows_text, header_spans, interpret,
};

fn lorom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn u32_at(buf: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(buf[i..i + 4].try_into().unwrap())
}

#[test]
fn batch_header_and_record_layout() {
    let rom = lorom();
    let spans = header_spans(&rom);
    let index = SpanIndex::new(&spans);
    let batch = encode_rows(&rom, &index, 0, 4);
    assert_eq!(batch.len(), BATCH_HEADER_LEN + 4 * ROW_STRIDE as usize);
    assert_eq!(u16::from_le_bytes([batch[0], batch[1]]), ROW_VERSION);
    assert_eq!(u16::from_le_bytes([batch[2], batch[3]]), ROW_STRIDE);
    assert_eq!(u32_at(&batch, 4), 4);
    let rec = &batch[BATCH_HEADER_LEN..BATCH_HEADER_LEN + 64];
    assert_eq!(u32_at(rec, 0), 0);
    assert_eq!(u32_at(rec, 4), SnesAddress::new(0x00, 0x8000).as_u24());
    assert_eq!(rec[8], 16);
    assert_eq!(rec[9], 0);
    assert_eq!(&rec[12..15], &[0x78, 0x18, 0xFB]);
    assert!(rec[28..44].iter().all(|&id| id == 0));
    assert_eq!(rec[44], b'x');
    assert_eq!(rec[45], b'.');
    // Second record starts 16 bytes on.
    let rec2 = &batch[BATCH_HEADER_LEN + 64..BATCH_HEADER_LEN + 128];
    assert_eq!(u32_at(rec2, 0), 16);
    assert_eq!(u32_at(rec2, 4), 0x8010);
}

#[test]
fn span_ids_cover_exactly_the_header_bytes() {
    let rom = lorom();
    let spans = header_spans(&rom);
    let index = SpanIndex::new(&spans);
    let header_row = 0x7FC0 / 16;
    let batch = encode_rows(&rom, &index, header_row - 1, 5);
    assert_eq!(u32_at(&batch, 4), 5);
    let rec = |i: usize| &batch[BATCH_HEADER_LEN + i * 64..BATCH_HEADER_LEN + (i + 1) * 64];
    assert_eq!(
        rec(0)[9] & FLAG_HAS_SPAN,
        0,
        "row before the header carries no span"
    );
    assert!(rec(0)[28..44].iter().all(|&id| id == 0));
    let title = spans.iter().find(|s| s.kind == SpanKind::Title).unwrap();
    assert!(rec(1)[28..44].iter().all(|&id| id as u32 == title.id));
    assert_ne!(rec(1)[9] & FLAG_HAS_SPAN, 0);
    // Row 0x7FD0: bytes 0..5 are title, then one field per byte.
    let r = rec(2);
    assert!(r[28..33].iter().all(|&id| id as u32 == title.id));
    let by_kind = |k: SpanKind| spans.iter().find(|s| s.kind == k).unwrap().id as u8;
    assert_eq!(r[33], by_kind(SpanKind::MapMode));
    assert_eq!(r[34], by_kind(SpanKind::CartridgeType));
    assert_eq!(r[35], by_kind(SpanKind::RomSize));
    assert_eq!(r[36], by_kind(SpanKind::RamSize));
    assert_eq!(r[37], by_kind(SpanKind::Region));
    assert_eq!(r[38], by_kind(SpanKind::DeveloperId));
    assert_eq!(r[39], by_kind(SpanKind::Version));
    assert_eq!(r[40], by_kind(SpanKind::ChecksumComplement));
    assert_eq!(r[41], by_kind(SpanKind::ChecksumComplement));
    assert_eq!(r[42], by_kind(SpanKind::Checksum));
    assert_eq!(r[43], by_kind(SpanKind::Checksum));
    // Last row of the image is the emulation vectors and is flagged last.
    let last = rec(4);
    assert_ne!(last[9] & FLAG_LAST_ROW, 0);
    assert_eq!(last[8], 16);
    assert!(
        last[28..32].iter().all(|&id| id == 0),
        "+0x30 gap is unspanned"
    );
    let emu_ids: Vec<u8> = spans
        .iter()
        .filter(|s| s.kind == SpanKind::EmulationVector)
        .map(|s| s.id as u8)
        .collect();
    assert_eq!(emu_ids.len(), 6);
    for (i, id) in emu_ids.iter().enumerate() {
        assert_eq!(&last[32 + 2 * i..34 + 2 * i], &[*id, *id]);
    }
}

#[test]
fn partial_last_row_and_clipping() {
    let mut bytes = fixtures::minimal_lorom();
    bytes.extend_from_slice(&[0xAA; 5]); // 32 KB + 5: still LoROM, last row has 5 bytes
    let rom = RomImage::from_bytes(bytes, "t.sfc").unwrap();
    assert_eq!(rom.row_count(), 0x801);
    let index = SpanIndex::default();
    let batch = encode_rows(&rom, &index, 0x7FF, 10);
    assert_eq!(u32_at(&batch, 4), 2, "clipped to the two remaining rows");
    let last = &batch[BATCH_HEADER_LEN + 64..];
    assert_eq!(last[8], 5);
    assert_eq!(last[9] & FLAG_LAST_ROW, FLAG_LAST_ROW);
    assert_eq!(&last[12..17], &[0xAA; 5]);
    assert_eq!(&last[17..28], &[0u8; 11]);
    assert_eq!(&last[49..60], &[0u8; 11], "ascii padding is zero");
    assert_eq!(u32_at(last, 4), SnesAddress::new(0x01, 0x8000).as_u24());
}

#[test]
fn text_lines_have_the_documented_shape() {
    let rom = lorom();
    let spans = header_spans(&rom);
    let index = SpanIndex::new(&spans);
    let text = format_rows_text(&rom, &index, 0, 1, AddressStyle::Both);
    assert_eq!(
        text,
        "0x000000  $00:8000  78 18 FB E2 30 A9 80 8D  00 21 80 FE EA EA 40 00   |x...0....!....@.|\n"
    );
    let snes = format_rows_text(&rom, &index, 0, 1, AddressStyle::Snes);
    assert!(snes.starts_with("$00:8000  78 18"));
    let file = format_rows_text(&rom, &index, 0, 1, AddressStyle::File);
    assert!(file.starts_with("0x000000  78 18"));
    let header = format_rows_text(&rom, &index, 0x7FC, 1, AddressStyle::Both);
    assert!(header.ends_with("  *|ROMLENS TEST    |\n"), "{header:?}");
}

#[test]
fn inspector_readings() {
    let rom = lorom();
    let spans = header_spans(&rom);
    let i = interpret(&rom, &spans, FileOffset(0x7FFC)).unwrap();
    assert_eq!(i.u8, 0x00);
    assert_eq!(i.u16_le, Some(0x8000));
    assert_eq!(i.u24_le, Some(0x0E_8000));
    assert_eq!(i.snes_address, Some(SnesAddress::new(0x00, 0xFFFC)));
    assert_eq!(
        i.u16_as_address_in_bank,
        Some(SnesAddress::new(0x00, 0x8000))
    );
    assert_eq!(i.span_name.as_deref(), Some("Emulation RESET"));
    assert_eq!(
        i.mirrors,
        vec![
            SnesAddress::new(0x00, 0xFFFC),
            SnesAddress::new(0x80, 0xFFFC)
        ]
    );
    let end = interpret(&rom, &spans, FileOffset(0x7FFF)).unwrap();
    assert_eq!(end.u16_le, None);
    assert_eq!(end.u24_le, None);
    assert!(interpret(&rom, &spans, FileOffset(0x8000)).is_none());
    let boot = interpret(&rom, &spans, FileOffset(5)).unwrap();
    assert_eq!(boot.u8, 0xA9);
    assert_eq!(boot.i8, -87);
    assert_eq!(boot.ascii, None);
    assert_eq!(boot.span_name, None);
}
