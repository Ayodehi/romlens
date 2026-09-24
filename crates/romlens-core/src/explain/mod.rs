//! Explanations for a student (docs/20): what a write to a hardware
//! register does, field by field, and the common SNES sequences named where
//! they appear.

pub mod fields;

pub use fields::{FieldRow, Part, RegisterWrite, describe};
