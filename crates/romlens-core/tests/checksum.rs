use romlens_core::fixtures;
use romlens_core::rom::checksum::compute_checksum;
use romlens_core::{MappingMode, RomImage};

const MB: usize = 1 << 20;

fn plain(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0u16, |s, &b| s.wrapping_add(b as u16))
}

#[test]
fn power_of_two_images_sum_plainly() {
    for len in [32 * 1024, MB, 4 * MB] {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + i / 3) as u8).collect();
        assert_eq!(compute_checksum(&bytes), plain(&bytes), "{len}");
    }
}

#[test]
fn three_mb_doubles_the_last_megabyte() {
    let bytes: Vec<u8> = (0..3 * MB).map(|i| (i * 31 + (i >> 9)) as u8).collect();
    let expected = plain(&bytes[..2 * MB])
        .wrapping_add(plain(&bytes[2 * MB..]))
        .wrapping_add(plain(&bytes[2 * MB..]));
    assert_eq!(compute_checksum(&bytes), expected);
}

#[test]
fn two_and_a_half_mb_repeats_the_half_megabyte_four_times() {
    let len = 2 * MB + 512 * 1024;
    let bytes: Vec<u8> = (0..len).map(|i| (i * 13 + (i >> 7)) as u8).collect();
    let rest = plain(&bytes[2 * MB..]);
    let expected = plain(&bytes[..2 * MB]).wrapping_add(rest.wrapping_mul(4));
    assert_eq!(compute_checksum(&bytes), expected);
}

#[test]
fn six_mb_doubles_the_last_two_megabytes() {
    let bytes: Vec<u8> = (0..6 * MB).map(|i| (i * 3 + (i >> 11)) as u8).collect();
    let rest = plain(&bytes[4 * MB..]);
    let expected = plain(&bytes[..4 * MB]).wrapping_add(rest.wrapping_mul(2));
    assert_eq!(compute_checksum(&bytes), expected);
}

#[test]
fn fixtures_carry_a_valid_checksum() {
    for mode in MappingMode::all() {
        let rom =
            RomImage::from_bytes(fixtures::for_mapping(mode), fixtures::file_name(mode)).unwrap();
        assert!(rom.header().complement_valid(), "{mode} complement");
        assert!(
            rom.checksum_ok(),
            "{mode} checksum ${:04X} vs computed ${:04X}",
            rom.header().checksum,
            rom.computed_checksum()
        );
    }
}
