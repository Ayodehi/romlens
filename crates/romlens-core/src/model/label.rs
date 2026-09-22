//! Labels: names for addresses, from the analyzer, the user, an import or the
//! built-in register table.

use crate::error::ProjectError;
use crate::memory::address::SnesAddress;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LabelSource {
    Auto,
    User,
    /// Imported from a named symbol source (`pjboy`, `asar`).
    Imported(String),
    /// Hardware register name.
    Builtin,
}

impl LabelSource {
    pub fn is_user_or_imported(&self) -> bool {
        matches!(self, LabelSource::User | LabelSource::Imported(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub address: SnesAddress,
    pub name: String,
    pub source: LabelSource,
}

/// Prefixes the analyzer uses; a user name of this shape at another address
/// would be misleading.
pub const AUTO_PREFIXES: [&str; 11] = [
    "RESET", "NMI", "IRQ", "COP", "BRK", "ABORT", "SUB", "CODE", "PTR", "DATA", "LOCAL",
];

/// Check `[A-Za-z_][A-Za-z0-9_]{0,63}` and refuse an auto-style name
/// (`SUB_808423`) whose address part is not `at`.
pub fn validate_label_name(name: &str, at: Option<SnesAddress>) -> Result<(), ProjectError> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return Err(ProjectError::InvalidLabelName("label name is empty".into()));
    }
    if bytes.len() > 64 {
        return Err(ProjectError::InvalidLabelName(
            "label name is longer than 64 characters".into(),
        ));
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return Err(ProjectError::InvalidLabelName(format!(
            "label {name:?} must start with a letter or underscore"
        )));
    }
    if let Some(bad) = bytes
        .iter()
        .find(|b| !(b.is_ascii_alphanumeric() || **b == b'_'))
    {
        return Err(ProjectError::InvalidLabelName(format!(
            "label {name:?} contains {:?}; use letters, digits and underscores",
            *bad as char
        )));
    }
    if let Some((prefix, hex)) = name.rsplit_once('_')
        && AUTO_PREFIXES.contains(&prefix)
        && hex.len() == 6
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())
        && let Ok(v) = u32::from_str_radix(hex, 16)
        && at.is_none_or(|a| a.as_u24() != v)
    {
        return Err(ProjectError::InvalidLabelName(format!(
            "{name:?} looks like an automatic label for {}; choose another name",
            SnesAddress::from_u24(v)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert!(validate_label_name("Boot", None).is_ok());
        assert!(validate_label_name("_x1", None).is_ok());
        assert!(validate_label_name("", None).is_err());
        assert!(validate_label_name("1abc", None).is_err());
        assert!(validate_label_name("a-b", None).is_err());
        assert!(validate_label_name(&"a".repeat(65), None).is_err());
        assert!(validate_label_name("SUB_808423", Some(SnesAddress::from_u24(0x808423))).is_ok());
        assert!(validate_label_name("SUB_808423", Some(SnesAddress::from_u24(0x808424))).is_err());
        assert!(validate_label_name("SUB_808423", None).is_err());
        assert!(
            validate_label_name("SUB_80842", None).is_ok(),
            "five digits is not the auto shape"
        );
    }
}
