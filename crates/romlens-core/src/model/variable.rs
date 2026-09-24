//! Variables: a named address with a type, so the code that uses it reads as
//! `STA PlayerX` rather than `STA $7E0094`.
//!
//! A variable is a label (`model::label`) plus a [`VarType`] at the same
//! address; the name lives with the other labels, so export, Find References
//! and the navigator treat it as one. The type says how many bytes the
//! variable spans, which is what lets an access into the middle of it read as
//! `PlayerX+1`.

use crate::error::ProjectError;

/// The width of one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VarWidth {
    Byte,
    Word,
    Long,
}

impl VarWidth {
    pub const fn bytes(self) -> u32 {
        match self {
            VarWidth::Byte => 1,
            VarWidth::Word => 2,
            VarWidth::Long => 3,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            VarWidth::Byte => "byte",
            VarWidth::Word => "word",
            VarWidth::Long => "long",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "byte" | "b" | "db" => Some(VarWidth::Byte),
            "word" | "w" | "dw" => Some(VarWidth::Word),
            "long" | "l" | "dl" => Some(VarWidth::Long),
            _ => None,
        }
    }
}

/// What a variable holds: `count` elements of `width`, one for a scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarType {
    pub width: VarWidth,
    pub count: u16,
}

/// Elements an array may have: a whole bank of bytes.
pub const MAX_COUNT: u16 = 0xFFFF;

impl VarType {
    pub const fn scalar(width: VarWidth) -> Self {
        VarType { width, count: 1 }
    }

    /// Bytes spanned.
    pub const fn len(&self) -> u32 {
        self.width.bytes() * self.count as u32
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `word`, or `byte[16]` for an array.
    pub fn describe(&self) -> String {
        if self.count == 1 {
            self.width.name().to_owned()
        } else {
            format!("{}[{}]", self.width.name(), self.count)
        }
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.count == 0 {
            return Err(ProjectError::InvalidVariable(
                "a variable needs at least one element".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_names() {
        assert_eq!(VarType::scalar(VarWidth::Word).len(), 2);
        let a = VarType {
            width: VarWidth::Long,
            count: 4,
        };
        assert_eq!((a.len(), a.describe()), (12, "long[4]".to_owned()));
        assert_eq!(VarWidth::parse("dw"), Some(VarWidth::Word));
        assert!(
            VarType {
                width: VarWidth::Byte,
                count: 0
            }
            .validate()
            .is_err()
        );
    }
}
