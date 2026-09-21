//! Address translation tables per mapping, from docs/04 and public references.

use romlens_core::{AddressMap, FileOffset, MappingMode, Region, SnesAddress, mirror_offset};

const MB: u32 = 1 << 20;

fn lorom(len: u32, fast: bool) -> AddressMap {
    AddressMap::new(MappingMode::LoRom, len, fast)
}
fn hirom(len: u32, fast: bool) -> AddressMap {
    AddressMap::new(MappingMode::HiRom, len, fast)
}
fn exhirom(len: u32) -> AddressMap {
    AddressMap::new(MappingMode::ExHiRom, len, true)
}

fn a(bank: u8, off: u16) -> SnesAddress {
    SnesAddress::new(bank, off)
}

#[test]
fn lorom_file_offsets() {
    let m = lorom(4 * MB, true);
    let table = [
        (a(0x80, 0x841C), Some(0x41C)),
        (a(0x00, 0x841C), Some(0x41C)),
        (a(0x00, 0x8000), Some(0)),
        (a(0x3F, 0xFFFF), Some(0x1F_FFFF)),
        (a(0xFF, 0xFFFF), Some(0x3F_FFFF)),
        (a(0x7E, 0x8000), None),
        (a(0x7F, 0xFFFF), None),
        (a(0x00, 0x7FFF), None),
        (a(0x00, 0x0000), None),
    ];
    for (addr, expected) in table {
        assert_eq!(m.file_offset(addr), expected.map(FileOffset), "{addr}");
    }
    // 3 MB: the top megabyte mirrors the third.
    assert_eq!(
        lorom(3 * MB, true).file_offset(a(0xFF, 0xFFFF)),
        Some(FileOffset(0x2F_FFFF))
    );
    assert_eq!(
        lorom(3 * MB, true).file_offset(a(0xE0, 0x8000)),
        Some(FileOffset(0x20_0000))
    );
}

#[test]
fn hirom_file_offsets() {
    let m = hirom(4 * MB, true);
    let table = [
        (a(0xC0, 0x0000), Some(0)),
        (a(0x40, 0x0000), Some(0)),
        (a(0x00, 0xFFC0), Some(0xFFC0)),
        (a(0x80, 0xFFC0), Some(0xFFC0)),
        (a(0xC0, 0xFFC0), Some(0xFFC0)),
        (a(0xFF, 0xFFFF), Some(0x3F_FFFF)),
        (a(0x7D, 0xFFFF), Some(0x3D_FFFF)),
        (a(0x00, 0x0000), None),
        (a(0x00, 0x7FFF), None),
        (a(0x7E, 0x0000), None),
        (a(0x7F, 0x8000), None),
    ];
    for (addr, expected) in table {
        assert_eq!(m.file_offset(addr), expected.map(FileOffset), "{addr}");
    }
}

#[test]
fn exhirom_file_offsets() {
    let m = exhirom(8 * MB);
    let table = [
        (a(0xC0, 0x0000), Some(0)),
        (a(0xFF, 0xFFFF), Some(0x3F_FFFF)),
        (a(0x40, 0x0000), Some(0x40_0000)),
        (a(0x00, 0xFFC0), Some(0x40_FFC0)),
        (a(0x80, 0xFFC0), Some(0xFFC0)),
        (a(0x3F, 0x8000), Some(0x7F_8000)),
        (a(0x7D, 0xFFFF), Some(0x7D_FFFF)),
        (a(0x00, 0x7FFF), None),
        (a(0x7E, 0x0000), None),
    ];
    for (addr, expected) in table {
        assert_eq!(m.file_offset(addr), expected.map(FileOffset), "{addr}");
    }
}

#[test]
fn canonical_prefers_fast_bank_when_header_says_so() {
    assert_eq!(
        lorom(3 * MB, true).canonical(FileOffset(0x41C)),
        Some(a(0x80, 0x841C))
    );
    assert_eq!(
        lorom(3 * MB, false).canonical(FileOffset(0x41C)),
        Some(a(0x00, 0x841C))
    );
    // Slots that would be $7E/$7F only exist as FastROM banks.
    assert_eq!(
        lorom(4 * MB, false).canonical(FileOffset(0x3F_0000)),
        Some(a(0xFE, 0x8000))
    );
    assert_eq!(
        hirom(64 * 1024, false).canonical(FileOffset(0xFFC0)),
        Some(a(0x40, 0xFFC0))
    );
    assert_eq!(
        hirom(64 * 1024, true).canonical(FileOffset(0xFFC0)),
        Some(a(0xC0, 0xFFC0))
    );
    assert_eq!(
        exhirom(8 * MB).canonical(FileOffset(0x40_FFC0)),
        Some(a(0x40, 0xFFC0))
    );
    assert_eq!(
        exhirom(8 * MB).canonical(FileOffset(0x0000)),
        Some(a(0xC0, 0x0000))
    );
    assert_eq!(
        exhirom(8 * MB).canonical(FileOffset(0x7E_8000)),
        Some(a(0x3E, 0x8000))
    );
    assert_eq!(exhirom(8 * MB).canonical(FileOffset(0x7E_0000)), None);
    assert_eq!(lorom(3 * MB, true).canonical(FileOffset(0x30_0000)), None);
}

#[test]
fn mirrors_list_every_reader_of_a_byte() {
    let m = lorom(3 * MB, true);
    assert_eq!(
        m.mirrors(FileOffset(0x41C)),
        vec![a(0x00, 0x841C), a(0x80, 0x841C)]
    );
    // Size folding is not listed: the last byte of a 3 MB LoROM has two
    // direct readers even though $7F/$FF also fold onto it.
    assert_eq!(
        m.mirrors(FileOffset(0x2F_FFFF)),
        vec![a(0x5F, 0xFFFF), a(0xDF, 0xFFFF)]
    );
    // A 32 KB image lists only its own bank pair, not all 128 folds.
    assert_eq!(
        lorom(0x8000, false).mirrors(FileOffset(0x10)),
        vec![a(0x00, 0x8010), a(0x80, 0x8010)]
    );
    let h = hirom(64 * 1024, true);
    assert_eq!(
        h.mirrors(FileOffset(0xFFC0)),
        vec![
            a(0x00, 0xFFC0),
            a(0x40, 0xFFC0),
            a(0x80, 0xFFC0),
            a(0xC0, 0xFFC0)
        ]
    );
    assert_eq!(
        h.mirrors(FileOffset(0x0100)),
        vec![a(0x40, 0x0100), a(0xC0, 0x0100)]
    );
    assert!(m.mirrors(FileOffset(3 * MB)).is_empty());
}

#[test]
fn round_trips_every_page() {
    for (map, len) in [
        (lorom(3 * MB, true), 3 * MB),
        (lorom(4 * MB, false), 4 * MB),
        (hirom(4 * MB, true), 4 * MB),
        (hirom(2 * MB + 512 * 1024, false), 2 * MB + 512 * 1024),
        (exhirom(6 * MB), 6 * MB),
    ] {
        for off in (0..len).step_by(0x1000) {
            let off = FileOffset(off);
            let Some(addr) = map.canonical(off) else {
                continue;
            };
            assert_eq!(
                map.file_offset(addr),
                Some(off),
                "{:?} {off} via {addr}",
                map.mode
            );
            assert!(
                map.mirrors(off).contains(&addr),
                "{off} mirrors include {addr}"
            );
        }
    }
}

#[test]
fn mirror_offset_folds_non_power_of_two_images() {
    assert_eq!(mirror_offset(0x1F_FFFF, MB), 0x0F_FFFF);
    assert_eq!(mirror_offset(0x3F_FFFF, 2 * MB), 0x1F_FFFF);
    assert_eq!(mirror_offset(0x3F_FFFF, 4 * MB), 0x3F_FFFF);
    assert_eq!(mirror_offset(0x7F_FFFF, 4 * MB), 0x3F_FFFF);
    // 3 MB = 2 MB + 1 MB; the megabyte repeats above 2 MB.
    assert_eq!(mirror_offset(0x2F_FFFF, 3 * MB), 0x2F_FFFF);
    assert_eq!(mirror_offset(0x3F_FFFF, 3 * MB), 0x2F_FFFF);
    assert_eq!(mirror_offset(0x30_0000, 3 * MB), 0x20_0000);
    assert_eq!(mirror_offset(0x40_0000, 3 * MB), 0x00_0000);
    // 2.5 MB = 2 MB + 512 KB; the half megabyte repeats four times.
    let len = 2 * MB + 512 * 1024;
    assert_eq!(mirror_offset(0x28_0000, len), 0x20_0000);
    assert_eq!(mirror_offset(0x3F_FFFF, len), 0x27_FFFF);
    // 6 MB = 4 MB + 2 MB.
    assert_eq!(mirror_offset(0x60_0000, 6 * MB), 0x40_0000);
    assert_eq!(mirror_offset(0x7F_FFFF, 6 * MB), 0x5F_FFFF);
    // 3.5 MB = 2 MB + (1 MB + 512 KB): recursion two deep.
    let len = 3 * MB + 512 * 1024;
    assert_eq!(mirror_offset(0x38_0000, len), 0x30_0000);
    assert_eq!(mirror_offset(0x3F_FFFF, len), 0x37_FFFF);
}

#[test]
fn classify_regions() {
    let m = lorom(3 * MB, true);
    assert_eq!(m.classify(a(0x80, 0x841C)), Region::Rom);
    assert_eq!(m.classify(a(0x7E, 0x0000)), Region::Wram);
    assert_eq!(m.classify(a(0x00, 0x0086)), Region::LowRam);
    assert_eq!(m.classify(a(0x00, 0x2100)), Region::Hardware);
    assert_eq!(m.classify(a(0x80, 0x420D)), Region::Hardware);
    assert_eq!(m.classify(a(0x70, 0x0000)), Region::Sram);
    assert_eq!(m.classify(a(0x40, 0x0000)), Region::OpenBus);
    assert_eq!(m.classify(a(0x00, 0x6000)), Region::OpenBus);
    let h = hirom(4 * MB, true);
    assert_eq!(h.classify(a(0x20, 0x6000)), Region::Sram);
    assert_eq!(h.classify(a(0x40, 0x0000)), Region::Rom);
    assert_eq!(h.classify(a(0x00, 0x7FFF)), Region::OpenBus);
}

#[test]
fn header_offsets_per_mode() {
    assert_eq!(MappingMode::LoRom.header_offset(), FileOffset(0x7FC0));
    assert_eq!(MappingMode::HiRom.header_offset(), FileOffset(0xFFC0));
    assert_eq!(MappingMode::ExHiRom.header_offset(), FileOffset(0x40_FFC0));
    assert_eq!(
        MappingMode::from_map_mode_nibble(0x30),
        Some(MappingMode::LoRom)
    );
    assert_eq!(
        MappingMode::from_map_mode_nibble(0x31),
        Some(MappingMode::HiRom)
    );
    assert_eq!(
        MappingMode::from_map_mode_nibble(0x25),
        Some(MappingMode::ExHiRom)
    );
    assert_eq!(MappingMode::from_map_mode_nibble(0x23), None);
}
