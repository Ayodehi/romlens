//! CRC-32 (IEEE 802.3, the zlib polynomial), for the checks other tools write:
//! Mesen2's CDL header records one of the ROM, and the `.romrec` footer
//! carries one of its header and index. Twenty lines is cheaper than a
//! dependency.

const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static TABLE: [u32; 256] = table();

/// Continue a CRC over more bytes; start from `0`.
pub fn update(crc: u32, bytes: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in bytes {
        c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// The CRC-32 of `bytes`.
pub fn crc32(bytes: &[u8]) -> u32 {
    update(0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_check_value() {
        // The check value every CRC-32/ISO-HDLC implementation publishes.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
    }
}
