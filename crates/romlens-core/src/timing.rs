//! The machine's timing, as the diagrams and lessons state it (docs/26).
//! Elsewhere these numbers are still written where they are used.

/// Scanlines in a frame.
pub const LINES_NTSC: u16 = 262;
pub const LINES_PAL: u16 = 312;
/// Dots in a line, 0–340.
pub const DOTS: u16 = 341;
/// Master clock cycles in a line (four per dot, with two longer dots).
pub const MASTER_PER_LINE: u32 = 1364;
/// Master clock frequencies, in Hz.
pub const MASTER_NTSC: f64 = 21_477_270.0;
pub const MASTER_PAL: f64 = 21_281_370.0;
/// The picture's lines: 1–224, or 1–239 with overscan (`SETINI` bit 2).
pub const VISIBLE: u16 = 224;
pub const VISIBLE_OVERSCAN: u16 = 239;
/// The first line of vertical blank, where the NMI fires (without
/// overscan).
pub const VBLANK_START: u16 = VISIBLE + 1;
/// The dots a line's 256 pixels are drawn in, about.
pub const FIRST_DOT: u16 = 22;
pub const LAST_DOT: u16 = 277;
/// Where HDMA runs, at the start of horizontal blank, about.
pub const HDMA_DOT: u16 = 278;
