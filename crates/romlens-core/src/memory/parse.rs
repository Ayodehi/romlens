//! The one address-expression grammar shared by the CLI, the jump sheet and
//! every future shell (docs/08 rule 7).
//!
//! Accepted forms:
//!
//! | Input | Meaning |
//! |---|---|
//! | `$80:841C`, `80:841C` | SNES bank:offset |
//! | `$80841C`, `80841C` | SNES 24-bit; fewer than six digits are bank `$00` |
//! | `0x41C`, `0X41C` | file offset |
//!
//! Whitespace and underscores are ignored.

use crate::error::AddressError;
use crate::memory::address::{FileOffset, SnesAddress};

/// A parsed but not yet resolved address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressExpr {
    Snes(SnesAddress),
    File(FileOffset),
}

fn hex(text: &str, original: &str, max_digits: usize) -> Result<u32, AddressError> {
    if text.is_empty() {
        return Err(AddressError::Empty);
    }
    if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AddressError::InvalidHex(original.to_owned()));
    }
    if text.len() > max_digits {
        return Err(AddressError::OutOfRange(original.to_owned()));
    }
    u32::from_str_radix(text, 16).map_err(|_| AddressError::InvalidHex(original.to_owned()))
}

/// Parse an address expression. See the module docs for the grammar.
pub fn parse_address_expr(input: &str) -> Result<AddressExpr, AddressError> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    if cleaned.is_empty() {
        return Err(AddressError::Empty);
    }
    if let Some(rest) = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        return hex(rest, input, 8).map(|v| AddressExpr::File(FileOffset(v)));
    }
    let body = cleaned.strip_prefix('$').unwrap_or(&cleaned);
    if let Some((bank, off)) = body.split_once(':') {
        let bank = hex(bank, input, 2)?;
        let off = hex(off, input, 4)?;
        return Ok(AddressExpr::Snes(SnesAddress::new(bank as u8, off as u16)));
    }
    hex(body, input, 6).map(|v| AddressExpr::Snes(SnesAddress::from_u24(v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_forms() {
        let snes = AddressExpr::Snes(SnesAddress::new(0x80, 0x841C));
        assert_eq!(parse_address_expr("$80:841C"), Ok(snes));
        assert_eq!(parse_address_expr("80:841C"), Ok(snes));
        assert_eq!(parse_address_expr("$80841C"), Ok(snes));
        assert_eq!(parse_address_expr("80841C"), Ok(snes));
        assert_eq!(parse_address_expr(" $80 : 841c "), Ok(snes));
        assert_eq!(
            parse_address_expr("0x41C"),
            Ok(AddressExpr::File(FileOffset(0x41C)))
        );
        assert_eq!(
            parse_address_expr("0X00041C"),
            Ok(AddressExpr::File(FileOffset(0x41C)))
        );
        assert_eq!(
            parse_address_expr("$841C"),
            Ok(AddressExpr::Snes(SnesAddress::new(0x00, 0x841C)))
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse_address_expr(""), Err(AddressError::Empty));
        assert_eq!(parse_address_expr("$"), Err(AddressError::Empty));
        assert_eq!(parse_address_expr("0x"), Err(AddressError::Empty));
        assert!(matches!(
            parse_address_expr("$8G:0000"),
            Err(AddressError::InvalidHex(_))
        ));
        assert!(matches!(
            parse_address_expr("$100:0000"),
            Err(AddressError::OutOfRange(_))
        ));
        assert!(matches!(
            parse_address_expr("$00:10000"),
            Err(AddressError::OutOfRange(_))
        ));
        assert!(matches!(
            parse_address_expr("$1000000"),
            Err(AddressError::OutOfRange(_))
        ));
        assert!(matches!(
            parse_address_expr("hello"),
            Err(AddressError::InvalidHex(_))
        ));
    }
}
