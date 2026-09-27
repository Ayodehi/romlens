//! What shapes the C beyond labels and variables (docs/24, decision 6):
//! names for a routine's locals, a note on the routine, comments that show
//! in the C, and C versions.
//!
//! The first three change the C Romlens generates: `decompile::annotate`
//! applies them to its text. A C version does not: it is a routine written
//! out again by hand, by the student or the tutor, to explain it another
//! way. It is kept beside the generated C, anchored line by line to the
//! addresses it stands for, and never compiled or checked against the code.

use crate::error::ProjectError;
use crate::memory::address::SnesAddress;

/// Who wrote a C version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Author {
    User,
    Tutor,
}

impl Author {
    pub const fn as_str(self) -> &'static str {
        match self {
            Author::User => "user",
            Author::Tutor => "tutor",
        }
    }

    pub fn parse(s: &str) -> Author {
        if s == "tutor" {
            Author::Tutor
        } else {
            Author::User
        }
    }
}

/// Lines `first..=last` of a version (from 1) stand for the instructions
/// from `start` to `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub first: u32,
    pub last: u32,
    pub start: SnesAddress,
    pub end: SnesAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CVersion {
    pub text: String,
    pub author: Author,
    pub anchors: Vec<Anchor>,
}

/// The longest C version kept: a routine explained, not a program.
pub const MAX_VERSION: usize = 64 * 1024;
/// The longest note or C comment.
pub const MAX_NOTE: usize = 4 * 1024;

const C_KEYWORDS: [&str; 44] = [
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "bool", "true", "false", "u8", "u16", "u32",
    "s8", "s16", "s32", "NULL",
];

/// A name the C can use: a C identifier that is not a keyword or one of
/// `snes.h`'s types.
pub fn validate_c_name(name: &str) -> Result<(), ProjectError> {
    let b = name.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    if !ok {
        return Err(ProjectError::InvalidC(format!(
            "{name:?} is not a C name: use letters, digits and underscores, starting with a letter"
        )));
    }
    if C_KEYWORDS.contains(&name) {
        return Err(ProjectError::InvalidC(format!(
            "{name:?} is a C keyword or type"
        )));
    }
    Ok(())
}

/// A note or comment: some text, not too long, with nothing that ends a C
/// comment early.
pub fn validate_note(text: &str) -> Result<(), ProjectError> {
    if text.len() > MAX_NOTE {
        return Err(ProjectError::InvalidC(format!(
            "the note is {} bytes; keep it under {MAX_NOTE}",
            text.len()
        )));
    }
    if text.contains("*/") {
        return Err(ProjectError::InvalidC("a note cannot contain */".into()));
    }
    Ok(())
}

impl CVersion {
    pub fn validate(&self, name: &str) -> Result<(), ProjectError> {
        if name.trim().is_empty() || name.len() > 64 {
            return Err(ProjectError::InvalidC(
                "a C version needs a short name".into(),
            ));
        }
        if self.text.len() > MAX_VERSION {
            return Err(ProjectError::InvalidC(format!(
                "the C version is {} bytes; keep it under {MAX_VERSION}",
                self.text.len()
            )));
        }
        let lines = self.text.lines().count() as u32;
        for a in &self.anchors {
            if a.first == 0 || a.first > a.last || a.last > lines.max(1) {
                return Err(ProjectError::InvalidC(format!(
                    "an anchor names lines {}–{} of a version with {lines}",
                    a.first, a.last
                )));
            }
            if a.end.as_u24() < a.start.as_u24() {
                return Err(ProjectError::InvalidC(format!(
                    "an anchor runs backwards, {} to {}",
                    a.start, a.end
                )));
            }
        }
        Ok(())
    }

    /// The addresses line `n` (from 1) stands for.
    pub fn anchors_for(&self, n: u32) -> impl Iterator<Item = &Anchor> {
        self.anchors
            .iter()
            .filter(move |a| (a.first..=a.last).contains(&n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_notes_are_checked() {
        assert!(validate_c_name("tileIndex").is_ok());
        assert!(validate_c_name("x_speed2").is_ok());
        assert!(validate_c_name("2fast").is_err());
        assert!(validate_c_name("for").is_err());
        assert!(validate_c_name("u16").is_err());
        assert!(validate_c_name("a-b").is_err());
        assert!(validate_note("moves Samus").is_ok());
        assert!(validate_note("ends */ early").is_err());
    }

    #[test]
    fn anchors_must_fit() {
        let at = |a: u32| SnesAddress::new((a >> 16) as u8, a as u16);
        let v = CVersion {
            text: "a();\nb();\n".into(),
            author: Author::Tutor,
            anchors: vec![Anchor {
                first: 1,
                last: 2,
                start: at(0x80_8000),
                end: at(0x80_8005),
            }],
        };
        assert!(v.validate("Plain").is_ok());
        assert_eq!(v.anchors_for(2).count(), 1);
        assert_eq!(v.anchors_for(3).count(), 0);
        let mut bad = v.clone();
        bad.anchors[0].last = 3;
        assert!(bad.validate("Plain").is_err());
        assert!(v.validate(" ").is_err());
    }
}
