//! Facts pinned to the development ROM (docs/04). Skipped without
//! `ROMLENS_ROM_DIR`.

mod common;

use romlens_core::{FileOffset, MappingMode, RomImage, SnesAddress, header_spans};

const SHA256: &str = "12b77c4bc9c1832cee8881244659065ee1d84c70c3d29e6eaf92e6798cc2ca72";

#[test]
fn identifies_the_image() {
    let Some(rom) = common::dev_rom() else { return };
    assert_eq!(rom.len(), 3_145_728);
    assert_eq!(rom.row_count(), 196_608);
    assert!(!rom.has_copier_header());
    assert_eq!(rom.mapping(), MappingMode::LoRom);
    assert_eq!(rom.header_offset(), FileOffset(0x7FC0));
    assert_eq!(rom.sha256_hex(), SHA256);
    assert_eq!(rom.source_name(), common::DEV_ROM);
}

#[test]
fn header_fields() {
    let Some(rom) = common::dev_rom() else { return };
    let h = rom.header();
    assert_eq!(h.title, "Super Metroid");
    assert_eq!(h.map_mode, 0x30);
    assert!(h.is_fast_rom());
    assert_eq!(h.mapping(), Some(MappingMode::LoRom));
    assert_eq!(h.cartridge_type, 0x02);
    assert_eq!(h.rom_size_code, 0x0C);
    assert_eq!(h.declared_rom_size(), 4 << 20);
    assert_eq!(h.ram_size_code, 0x03);
    assert_eq!(h.declared_ram_size(), 8 * 1024);
    assert_eq!(h.region, 0x00);
    assert_eq!(h.developer_id, 0x01);
    assert_eq!(h.version, 0x00);
    assert_eq!(h.complement, 0x0720);
    assert_eq!(h.checksum, 0xF8DF);
    assert!(h.complement_valid());
    assert!(h.extended.is_none());
}

#[test]
fn checksum_needs_the_mirrored_sum() {
    let Some(rom) = common::dev_rom() else { return };
    assert_eq!(rom.computed_checksum(), 0xF8DF);
    assert!(rom.checksum_ok());
    let plain = rom
        .bytes()
        .iter()
        .fold(0u16, |s, &b| s.wrapping_add(b as u16));
    assert_ne!(plain, 0xF8DF, "a plain sum must not validate a 3 MB image");
}

#[test]
fn vectors() {
    let Some(rom) = common::dev_rom() else { return };
    let n = rom.native_vectors();
    assert_eq!(
        (n.cop, n.brk, n.abort, n.nmi, n.irq),
        (0x8573, 0x8573, 0x8573, 0x9583, 0x986A)
    );
    let e = rom.emulation_vectors();
    assert_eq!(e.reset, 0x841C);
    assert_eq!(
        (e.cop, e.abort, e.nmi, e.irq),
        (0x8573, 0x8573, 0x8573, 0x8573)
    );
}

#[test]
fn boot_bytes_and_resolution() {
    let Some(rom) = common::dev_rom() else { return };
    assert_eq!(
        &rom.bytes()[0x41C..0x423],
        &[0x78, 0x18, 0xFB, 0x5C, 0x23, 0x84, 0x80]
    );
    let r = rom.resolve("$80:841C").unwrap();
    assert_eq!(r.file_offset, FileOffset(0x41C));
    assert_eq!(r.snes_address, Some(SnesAddress::new(0x80, 0x841C)));
    assert_eq!(r.row, 0x41);
    assert_eq!(
        rom.resolve("0x41C").unwrap().snes_address,
        Some(SnesAddress::new(0x80, 0x841C))
    );
    assert_eq!(
        rom.mirrors(FileOffset(0x41C)),
        vec![
            SnesAddress::new(0x00, 0x841C),
            SnesAddress::new(0x80, 0x841C)
        ]
    );
    // The JML at 0x41F carries the next instruction's address in bank $80 as its operand.
    let interp = romlens_core::interpret(&rom, &[], FileOffset(0x420)).unwrap();
    assert_eq!(interp.u24_le, Some(0x80_8423));
    assert_eq!(
        interp.u24_as_snes_address,
        Some(SnesAddress::new(0x80, 0x8423))
    );
    assert_eq!(interp.pointer_target_file_offset, Some(FileOffset(0x423)));
}

#[test]
fn spans_cover_the_header_block() {
    let Some(rom) = common::dev_rom() else { return };
    let spans = header_spans(&rom);
    assert!(spans.len() >= 22, "{} spans", spans.len());
    let mut covered = [false; 0x40];
    for s in &spans {
        for off in s.start.0..s.end().0 {
            assert!((0x7FC0..0x8000).contains(&off), "{} outside header", s.name);
            covered[(off - 0x7FC0) as usize] = true;
        }
    }
    // The two unused 4-byte gaps (+0x20, +0x30) are the only holes.
    let holes: Vec<usize> = covered
        .iter()
        .enumerate()
        .filter(|(_, c)| !**c)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(holes, vec![0x20, 0x21, 0x22, 0x23, 0x30, 0x31, 0x32, 0x33]);
    let reset = spans.iter().find(|s| s.name == "Emulation RESET").unwrap();
    assert_eq!(reset.value_text, "$841C");
    assert_eq!(reset.target, Some(SnesAddress::new(0x80, 0x841C)));
    assert_eq!(reset.start, FileOffset(0x7FFC));
}

#[test]
fn smc_variant_parses_identically() {
    let Some(rom) = common::dev_rom() else { return };
    let mut smc = vec![0u8; 512];
    smc.extend_from_slice(rom.bytes());
    let headered = RomImage::from_bytes(smc, "SuperMetroid.F8DF.smc").unwrap();
    assert!(headered.has_copier_header());
    assert_eq!(headered.len(), rom.len());
    assert_eq!(
        headered.sha256_hex(),
        SHA256,
        "identity is over the payload"
    );
    assert_eq!(headered.header(), rom.header());
    assert_eq!(headered.computed_checksum(), rom.computed_checksum());
    assert_eq!(headered.disk_offset_delta(), 512);
    let interp = romlens_core::interpret(&headered, &[], FileOffset(0x41C)).unwrap();
    assert_eq!(interp.disk_offset, 0x41C + 512);
}
