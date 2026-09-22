//! Byte-pattern search with `??` wildcards.

use crate::error::AddressError;

/// One pattern element: a byte or a wildcard.
pub type Pattern = Vec<Option<u8>>;

/// Parse `78 18 ?? 5C` (spaces optional between pairs; `?`/`??` wildcards).
pub fn parse_pattern(text: &str) -> Result<Pattern, AddressError> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err(AddressError::Empty);
    }
    if !cleaned.len().is_multiple_of(2) {
        return Err(AddressError::InvalidHex(text.to_owned()));
    }
    let bytes = cleaned.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        if pair == b"??" {
            out.push(None);
        } else if pair.iter().all(u8::is_ascii_hexdigit) {
            out.push(Some(
                u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap(),
            ));
        } else {
            return Err(AddressError::InvalidHex(text.to_owned()));
        }
    }
    Ok(out)
}

/// Offsets in `[start, start + len)` where the pattern matches, at most `max`.
pub fn search_bytes(bytes: &[u8], pattern: &Pattern, start: u32, len: u32, max: u32) -> Vec<u32> {
    let mut out = Vec::new();
    if pattern.is_empty() || max == 0 {
        return out;
    }
    let end = (start as usize)
        .saturating_add(len as usize)
        .min(bytes.len());
    let start = (start as usize).min(end);
    let n = pattern.len();
    if end - start < n {
        return out;
    }
    let first = pattern[0];
    let mut i = start;
    while i + n <= end {
        if let Some(f) = first
            && bytes[i] != f
        {
            i += 1;
            continue;
        }
        if pattern
            .iter()
            .zip(&bytes[i..i + n])
            .all(|(p, b)| p.is_none_or(|p| p == *b))
        {
            out.push(i as u32);
            if out.len() as u32 >= max {
                break;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(
            parse_pattern("78 18 ?? 5C").unwrap(),
            vec![Some(0x78), Some(0x18), None, Some(0x5C)]
        );
        assert_eq!(parse_pattern("7818").unwrap(), vec![Some(0x78), Some(0x18)]);
        assert!(parse_pattern("7").is_err());
        assert!(parse_pattern("zz").is_err());
        assert!(parse_pattern("").is_err());
        let bytes = [0x78, 0x18, 0xFB, 0x5C, 0x78, 0x18, 0x00, 0x5C];
        assert_eq!(
            search_bytes(&bytes, &parse_pattern("78 18 ?? 5C").unwrap(), 0, 8, 10),
            vec![0, 4]
        );
        assert_eq!(
            search_bytes(&bytes, &parse_pattern("78 18 ?? 5C").unwrap(), 0, 8, 1),
            vec![0]
        );
        assert_eq!(
            search_bytes(&bytes, &parse_pattern("78 18 ?? 5C").unwrap(), 1, 8, 10),
            vec![4]
        );
        assert_eq!(
            search_bytes(&bytes, &parse_pattern("?? ??").unwrap(), 6, 100, 10),
            vec![6]
        );
    }
}
