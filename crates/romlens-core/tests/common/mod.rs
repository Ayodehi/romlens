#![allow(dead_code)]

use std::path::PathBuf;

use romlens_core::RomImage;

pub const DEV_ROM: &str = "SuperMetroid.F8DF.sfc";

/// The development ROM, when `ROMLENS_ROM_DIR` points at it. Tests that need
/// it print "skipped" and pass otherwise, so CI never needs a commercial ROM.
pub fn dev_rom() -> Option<RomImage> {
    let dir = std::env::var_os("ROMLENS_ROM_DIR")?;
    let path = PathBuf::from(dir).join(DEV_ROM);
    if !path.exists() {
        eprintln!("skipped: {} not found in ROMLENS_ROM_DIR", DEV_ROM);
        return None;
    }
    Some(RomImage::load(&path).expect("development ROM loads"))
}

/// A tiny deterministic PRNG (xorshift32) so "random bytes" tests are stable.
pub fn pseudo_random_bytes(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.max(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}
