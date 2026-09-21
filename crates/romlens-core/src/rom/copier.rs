//! The optional 512-byte header some copiers prepend to a dump (`.smc`).

/// Length of a copier header when present.
pub const COPIER_HEADER_LEN: usize = 512;

/// Split a copier header off the front of a dump. The rule is the usual one:
/// SNES images are multiples of 1024 bytes, so a length that is 512 modulo
/// 1024 has a header.
pub fn split_copier_header(bytes: &[u8]) -> (Option<&[u8]>, &[u8]) {
    if bytes.len() > COPIER_HEADER_LEN && bytes.len() % 1024 == COPIER_HEADER_LEN {
        let (head, payload) = bytes.split_at(COPIER_HEADER_LEN);
        (Some(head), payload)
    } else {
        (None, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_length() {
        let plain = vec![0u8; 0x8000];
        assert!(split_copier_header(&plain).0.is_none());
        let headered = vec![0u8; 0x8000 + 512];
        let (head, payload) = split_copier_header(&headered);
        assert_eq!(head.map(<[u8]>::len), Some(512));
        assert_eq!(payload.len(), 0x8000);
        // A stray 512 bytes with nothing behind it is not a header.
        assert!(split_copier_header(&[0u8; 512]).0.is_none());
    }
}
