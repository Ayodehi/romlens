//! The fixtures the suite assembles itself.

mod common;

use romlens_core::fixtures::{self, BOOT_CODE};
use romlens_core::rom::checksum::compute_checksum;
use romlens_core::rom::scorer::{SCORE_FLOOR, detect_mapping};
use romlens_core::{FileOffset, MappingMode, RomError, RomImage, SnesAddress};

#[test]
fn fixtures_load_with_their_mapping() {
    for (mode, len, header) in [
        (MappingMode::LoRom, 0x8000, 0x7FC0),
        (MappingMode::HiRom, 0x1_0000, 0xFFC0),
        (MappingMode::ExHiRom, 0x41_0000, 0x40_FFC0),
    ] {
        let bytes = fixtures::for_mapping(mode);
        assert_eq!(bytes.len(), len);
        let rom = RomImage::from_bytes(bytes, fixtures::file_name(mode)).unwrap();
        assert_eq!(rom.mapping(), mode, "{mode}");
        assert_eq!(rom.header_offset(), FileOffset(header));
        assert_eq!(rom.header().title, fixtures::FIXTURE_TITLE);
        assert!(!rom.header().is_fast_rom());
        assert_eq!(rom.header().region, 0x01);
        assert_eq!(rom.native_vectors().nmi, 0x800E);
        assert_eq!(rom.native_vectors().irq, 0x800E);
        assert_eq!(rom.emulation_vectors().reset, 0x8000);
        assert_eq!(rom.emulation_vectors().irq, 0x800E);
        assert!(rom.checksum_ok(), "{mode}");
        assert!(!rom.has_copier_header());
        // The reset vector lands on the boot prologue.
        let reset = rom.resolve("$00:8000").unwrap();
        let at = reset.file_offset.as_usize();
        assert_eq!(&rom.bytes()[at..at + BOOT_CODE.len()], &BOOT_CODE, "{mode}");
        // The canonical address may be another bank ($40:8000 for HiROM); it round-trips.
        let canonical = rom.snes_address_for(reset.file_offset).unwrap();
        assert_eq!(rom.file_offset_for(canonical), Some(reset.file_offset));
        assert!(
            rom.mirrors(reset.file_offset)
                .contains(&SnesAddress::new(0x00, 0x8000)),
            "{mode}"
        );
    }
}

#[test]
fn lorom_fixture_row_zero_is_the_boot_code() {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    assert_eq!(&rom.bytes()[..3], &[0x78, 0x18, 0xFB]);
    assert_eq!(rom.resolve("$00:8000").unwrap().row, 0);
    assert_eq!(rom.row_count(), 0x800);
}

#[test]
fn random_bytes_have_no_header() {
    let bytes = common::pseudo_random_bytes(0x8000, 0xC0FFEE);
    match RomImage::from_bytes(bytes, "noise.sfc") {
        Err(RomError::NoValidHeader) => {}
        other => panic!("expected NoValidHeader, got {other:?}"),
    }
    let big = common::pseudo_random_bytes(0x10_0000, 0xBEEF);
    assert!(matches!(
        RomImage::from_bytes(big, "noise.sfc"),
        Err(RomError::NoValidHeader)
    ));
}

#[test]
fn size_limits() {
    assert!(matches!(
        RomImage::from_bytes(vec![0; 0x7FFF], "s.sfc"),
        Err(RomError::TooSmall { .. })
    ));
    assert!(matches!(
        RomImage::from_bytes(vec![0; 0x80_0000 + 1024], "l.sfc"),
        Err(RomError::TooLarge { .. })
    ));
    // 512 + 32 KB: a copier header on a minimal image is stripped, not rejected.
    let mut smc = vec![0u8; 512];
    smc.extend(fixtures::minimal_lorom());
    let rom = RomImage::from_bytes(smc, "t.smc").unwrap();
    assert!(rom.has_copier_header());
    assert_eq!(rom.len(), 0x8000);
    assert_eq!(rom.mapping(), MappingMode::LoRom);
}

/// Both slots carry a complete, complement-valid header; only the map-mode
/// byte differs, and it decides.
#[test]
fn ambiguous_slots_are_decided_by_the_map_mode_byte() {
    for (mode_byte, expected) in [(0x20u8, MappingMode::LoRom), (0x21u8, MappingMode::HiRom)] {
        let mut rom = fixtures::minimal_hirom();
        // Copy the HiROM header block onto the LoROM slot, then set both mode bytes.
        let block: Vec<u8> = rom[0xFFC0..0x1_0000].to_vec();
        rom[0x7FC0..0x8000].copy_from_slice(&block);
        rom[0x7FC0 + 0x15] = mode_byte;
        rom[0xFFC0 + 0x15] = mode_byte;
        // Recompute the shared checksum pair in both slots.
        for h in [0x7FC0usize, 0xFFC0] {
            rom[h + 0x1C..h + 0x1E].copy_from_slice(&0u16.to_le_bytes());
            rom[h + 0x1E..h + 0x20].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        let sum = compute_checksum(&rom);
        for h in [0x7FC0usize, 0xFFC0] {
            rom[h + 0x1C..h + 0x1E].copy_from_slice(&(!sum).to_le_bytes());
            rom[h + 0x1E..h + 0x20].copy_from_slice(&sum.to_le_bytes());
        }
        let best = detect_mapping(&rom).unwrap();
        assert_eq!(best.mode, expected, "mode byte ${mode_byte:02X}");
        assert!(best.score >= SCORE_FLOOR);
        assert!(best.header.complement_valid());
    }
}

#[test]
fn unsupported_map_mode_is_reported() {
    let mut rom = fixtures::minimal_lorom();
    rom[0x7FC0 + 0x15] = 0x23; // SA-1
    match RomImage::from_bytes(rom, "sa1.sfc") {
        Err(RomError::UnsupportedMapping(0x23)) => {}
        other => panic!("expected UnsupportedMapping, got {other:?}"),
    }
}

#[test]
fn fixture_without_valid_checksum_still_loads() {
    let mut rom = fixtures::minimal_lorom();
    rom[0x7FC0 + 0x1E] ^= 0x55;
    let rom = RomImage::from_bytes(rom, "bad-sum.sfc").unwrap();
    assert!(!rom.checksum_ok());
    assert_eq!(rom.mapping(), MappingMode::LoRom);
}
