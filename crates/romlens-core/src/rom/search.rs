//! Byte-pattern search with `??` wildcards.

use crate::error::AddressError;

/// One pattern element: a byte or a wildcard.
pub type Pattern = Vec<Option<u8>>;

/// A pattern from text rather than hex.
///
/// `ignore_case` matches both cases of every ASCII letter by turning that
/// position into a two-way choice, which the byte matcher cannot express — so
/// it is expanded to a wildcard *and* checked afterwards. Wildcards are the
/// only thing the matcher understands, and a case-insensitive letter is not a
/// wildcard; the mask is what keeps "find `reset`" from matching `RESE\x00`.
pub fn pattern_from_text(
    text: &str,
    ignore_case: bool,
) -> Result<(Pattern, TextMask), AddressError> {
    if text.is_empty() {
        return Err(AddressError::Empty);
    }
    let mut pattern = Pattern::with_capacity(text.len());
    let mut mask = Vec::with_capacity(text.len());
    for b in text.as_bytes() {
        if ignore_case && b.is_ascii_alphabetic() {
            pattern.push(None);
            mask.push(Some(b.to_ascii_lowercase()));
        } else {
            pattern.push(Some(*b));
            mask.push(None);
        }
    }
    Ok((pattern, mask))
}

/// Per-position: `Some(lowercase letter)` where the match must be that letter
/// in either case, `None` where the pattern already decided.
pub type TextMask = Vec<Option<u8>>;

/// Whether a match at `offset` satisfies a case-insensitive text mask.
pub fn mask_matches(bytes: &[u8], offset: u32, mask: &TextMask) -> bool {
    mask.iter().enumerate().all(|(i, m)| match m {
        None => true,
        Some(letter) => bytes
            .get(offset as usize + i)
            .is_some_and(|b| b.to_ascii_lowercase() == *letter),
    })
}

/// [`search_bytes`] with a case-insensitive mask applied as it goes.
///
/// The mask has to be inside the loop, not a filter over the results: a
/// case-insensitive letter is a wildcard to the matcher, so an all-letters
/// pattern matches every offset, and `max` would be spent on rejects before
/// the first real hit.
pub fn search_masked(
    bytes: &[u8],
    pattern: &Pattern,
    mask: &TextMask,
    start: u32,
    len: u32,
    max: u32,
) -> Vec<u32> {
    search_impl(bytes, pattern, Some(mask), start, len, max)
}

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
    search_impl(bytes, pattern, None, start, len, max)
}

fn search_impl(
    bytes: &[u8],
    pattern: &Pattern,
    mask: Option<&TextMask>,
    start: u32,
    len: u32,
    max: u32,
) -> Vec<u32> {
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
            && mask.is_none_or(|m| mask_matches(bytes, i as u32, m))
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
    fn text_patterns() {
        let bytes = b"Reset\x00RESET\x00reset";
        let (p, m) = pattern_from_text("reset", false).unwrap();
        assert!(m.iter().all(Option::is_none), "no mask when case matters");
        assert_eq!(search_bytes(bytes, &p, 0, 100, 10), vec![12]);

        let (p, m) = pattern_from_text("reset", true).unwrap();
        assert_eq!(search_masked(bytes, &p, &m, 0, 100, 10), vec![0, 6, 12]);
        // Without the mask the all-wildcard pattern matches everywhere, which
        // is exactly the bug it exists to prevent — and why `max` has to be
        // spent on real hits rather than on rejects.
        assert!(search_bytes(bytes, &p, 0, 100, 100).len() > 3);
        assert_eq!(
            search_masked(bytes, &p, &m, 0, 100, 2),
            vec![0, 6],
            "the cap counts matches, not candidates"
        );
        assert!(pattern_from_text("", false).is_err());

        // Digits and punctuation are not letters, so they stay exact.
        let (p, m) = pattern_from_text("v1.0", true).unwrap();
        assert_eq!(p[1], Some(b'1'));
        assert_eq!(m[0], Some(b'v'));
    }

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
