//! The core's data the diagrams draw (docs/26): registers by name, their
//! layouts, and a bank's regions.

use romlens_core::explain::fields::layout_named;
use romlens_core::model::hardware_register_named;
use romlens_core::{AddressMap, MappingMode, MemoryClass};

#[test]
fn registers_are_found_by_name_before_hex() {
    assert_eq!(hardware_register_named("inidisp").unwrap().address, 0x2100);
    // BBAD0 is also hex; the name wins.
    assert_eq!(hardware_register_named("BBAD0").unwrap().address, 0x4301);
    assert!(hardware_register_named("NOPE").is_none());
}

#[test]
fn a_layout_by_its_register_or_its_pair() {
    let (name, l) = layout_named("INIDISP").unwrap();
    assert_eq!((name.as_str(), l.address), ("INIDISP", 0x2100));
    assert_eq!(l.fields[0].name, "Forced blank");
    assert_eq!((l.fields[0].hi, l.fields[0].lo), (7, 7));
    let (name, l) = layout_named("vmadd").unwrap();
    assert_eq!((name.as_str(), l.address), ("VMADD", 0x2116));
    // Either half of a pair is the pair.
    let (name, _) = layout_named("VMADDH").unwrap();
    assert_eq!(name, "VMADD");
    let (name, l) = layout_named("A1T3").unwrap();
    assert_eq!((name.as_str(), l.address), ("A1T3", 0x4332));
    let (name, l) = layout_named("A1T3L").unwrap();
    assert_eq!((name.as_str(), l.address), ("A1T3", 0x4332));
    let (name, l) = layout_named("DMAP2").unwrap();
    assert_eq!((name.as_str(), l.address), ("DMAP2", 0x4320));
}

#[test]
fn a_system_bank_in_regions() {
    let map = AddressMap {
        mode: MappingMode::LoRom,
        rom_len: 0x80000,
        fast_rom: false,
    };
    let r = map.regions(0x00);
    let names: Vec<(u16, u16, &str)> = r.iter().map(|r| (r.start, r.end, r.name)).collect();
    assert_eq!(names.first(), Some(&(0x0000, 0x1FFF, "Low RAM")));
    assert!(names.contains(&(0x2100, 0x213F, "PPU registers")));
    assert!(names.contains(&(0x4200, 0x421F, "CPU registers")));
    assert!(names.contains(&(0x4300, 0x437F, "DMA registers")));
    assert_eq!(names.last(), Some(&(0x8000, 0xFFFF, "ROM")));
    // Every offset once, in order.
    for w in r.windows(2) {
        assert_eq!(w[0].end + 1, w[1].start);
    }
    let wram = map.regions(0x7E);
    assert_eq!(wram.len(), 1);
    assert_eq!(wram[0].class, MemoryClass::Wram);
}
