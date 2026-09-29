//! Diagrams Romlens draws for the tutor and its lessons (docs/26): an SVG
//! writer, the checks every picture passes before it is drawn, and the
//! drawing itself with `resvg` and the fonts Romlens ships.

pub mod check;
pub mod fonts;
pub mod render;
pub mod svg;

pub use check::Problem;
pub use render::{Picture, render};
