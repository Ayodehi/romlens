//! Colouring any C text with the C view's token kinds: a C version, or C in
//! the tutor's answers. The generated C carries its own tokens; this reads
//! text nobody generated, so it knows only what the text shows: keywords,
//! types, numbers, comments, hardware registers by name, the project's
//! labels, and calls.

use crate::decompile::emit::{CToken, CTokenKind};
use crate::memory::address::SnesAddress;
use crate::model::hardware::all_hardware_registers;

const KEYWORDS: &[&str] = &[
    "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto",
    "if", "inline", "return", "sizeof", "static", "struct", "switch", "typedef", "union",
    "volatile", "while", "true", "false", "NULL",
];
const TYPES: &[&str] = &[
    "void", "bool", "char", "int", "long", "short", "signed", "unsigned", "u8", "u16", "u32", "s8",
    "s16", "s32", "uint8_t", "uint16_t", "uint32_t", "int8_t", "int16_t", "int32_t",
];

/// Tokens of `text`. `names` finds a label's address by its name.
pub fn lex(text: &str, names: &dyn Fn(&str) -> Option<SnesAddress>) -> Vec<CToken> {
    let b = text.as_bytes();
    let regs = all_hardware_registers();
    let mut out = Vec::new();
    let mut i = 0usize;
    let tok = |start: usize, end: usize, kind: CTokenKind, address: Option<SnesAddress>| CToken {
        start: start as u32,
        len: (end - start) as u32,
        kind,
        address,
    };
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..]
                .find("*/")
                .map(|e| i + 4 + e)
                .unwrap_or(b.len());
            out.push(tok(i, end, CTokenKind::Comment, None));
            i = end;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = text[i..].find('\n').map(|e| i + e).unwrap_or(b.len());
            out.push(tok(i, end, CTokenKind::Comment, None));
            i = end;
        } else if c == b'#' && (i == 0 || b[i - 1] == b'\n') {
            let end = text[i..].find('\n').map(|e| i + e).unwrap_or(b.len());
            out.push(tok(i, end, CTokenKind::Keyword, None));
            i = end;
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            out.push(tok(i, end, CTokenKind::Number, None));
            i = end;
        } else if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            out.push(tok(i, j, CTokenKind::Number, None));
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let word = &text[i..j];
            let rest = text[j..].trim_start();
            let kind_addr = if KEYWORDS.contains(&word) {
                Some((CTokenKind::Keyword, None))
            } else if TYPES.contains(&word) {
                Some((CTokenKind::Type, None))
            } else if regs.iter().any(|r| r.name == word) {
                Some((CTokenKind::Register, None))
            } else if let Some(a) = names(word) {
                let k = if rest.starts_with('(') {
                    CTokenKind::Function
                } else {
                    CTokenKind::Label
                };
                Some((k, Some(a)))
            } else if rest.starts_with('(') {
                Some((CTokenKind::Helper, None))
            } else {
                None
            };
            if let Some((k, a)) = kind_addr {
                out.push(tok(i, j, k, a));
            }
            i = j;
        } else {
            // A multi-byte character is skipped whole.
            i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_colours_c_it_did_not_write() {
        let text = "#include \"snes.h\"\n/* wait */ void Wait(void) {\n  while ((HVBJOY & 0x80) == 0) {} // é\n  UpdateSamus(x, 3);\n}\n";
        let samus = SnesAddress::new(0x90, 0x8000);
        let t = lex(text, &|n| (n == "UpdateSamus").then_some(samus));
        let word = |k: &CToken| &text[k.start as usize..(k.start + k.len) as usize];
        let kinds: Vec<(&str, CTokenKind)> = t.iter().map(|k| (word(k), k.kind)).collect();
        assert!(kinds.contains(&("#include \"snes.h\"", CTokenKind::Keyword)));
        assert!(kinds.contains(&("/* wait */", CTokenKind::Comment)));
        assert!(kinds.contains(&("void", CTokenKind::Type)));
        assert!(kinds.contains(&("while", CTokenKind::Keyword)));
        assert!(kinds.contains(&("HVBJOY", CTokenKind::Register)));
        assert!(kinds.contains(&("0x80", CTokenKind::Number)));
        assert!(kinds.contains(&("// é", CTokenKind::Comment)));
        assert!(kinds.contains(&("Wait", CTokenKind::Helper)));
        let call = t.iter().find(|k| word(k) == "UpdateSamus").unwrap();
        assert_eq!(
            (call.kind, call.address),
            (CTokenKind::Function, Some(samus))
        );
        assert!(!kinds.iter().any(|(w, _)| *w == "x"));
    }
}
