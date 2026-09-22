//! Scored guesses about what unclassified bytes are.
//!
//! Two rules decide everything about how these behave, and both are asserted
//! in `tests/heuristics.rs`:
//!
//! 1. **A heuristic only fills bytes nothing else claimed.** It never argues
//!    with the disassembler, an imported trace or the user. That is the
//!    difference between a map that is mostly right and a map that fights you.
//! 2. **A heuristic emits spans, never per-byte verdicts.** Regions are cut
//!    wherever the classification changes, and a per-byte score would shatter
//!    the development ROM's few hundred blocks into millions of fragments.
//!    Hits are window-aligned, then coalesced here before anything is painted.
//!
//! Confidence is quantized to 5% steps for the same reason: two neighbouring
//! windows that agree to within a rounding error have to merge, or the region
//! list grows without telling anyone anything.

pub mod ascii;
pub mod entropy;
pub mod graphics;
pub mod palette;
pub mod pointers;

use crate::model::region::RegionKind;
use crate::rom::image::RomImage;

pub use entropy::EntropyProfile;

/// Bytes per window. Small enough to find a compressed blob inside a bank,
/// large enough that a 3 MB ROM is twelve thousand windows rather than a
/// million.
pub const WINDOW: u32 = 256;

/// The ceiling on any heuristic's confidence, as painted. A guess is never
/// allowed to look as certain as the linear sweep's successor stages, let
/// alone as the disassembler (`16-phase2-plan.md` 2A.2).
pub const MAX_CONFIDENCE: u8 = 55;

/// A span a heuristic recognises.
#[derive(Debug, Clone, PartialEq)]
pub struct HeuristicHit {
    pub start: u32,
    pub len: u32,
    pub kind: RegionKind,
    /// 0.0..=1.0, already quantized.
    pub score: f32,
    /// Stable identifier, shown in the evidence popover and `romlens
    /// heuristics --kind`.
    pub name: &'static str,
    /// What was measured, for a reader: "7.4 bits per byte".
    pub detail: String,
    /// Whether this hit may set a region's kind. `false` means it contributes
    /// evidence and an overview tint only — see `graphics`.
    pub classifies: bool,
}

impl HeuristicHit {
    pub fn end(&self) -> u32 {
        self.start + self.len
    }

    /// The painted confidence: the score as a percentage, capped.
    pub fn confidence(&self) -> u8 {
        ((self.score * 100.0) as u8).min(MAX_CONFIDENCE)
    }
}

/// Round a score to 5% steps so neighbouring windows that agree can merge.
pub fn quantize(score: f32) -> f32 {
    (score.clamp(0.0, 1.0) * 20.0).round() / 20.0
}

/// A window's verdict before coalescing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowHit {
    pub kind: RegionKind,
    pub score: f32,
    pub detail_value: f32,
}

/// What a heuristic implements: look at one window, say what it thinks.
pub trait Heuristic {
    /// Stable identifier.
    fn name(&self) -> &'static str;

    /// Whether hits may set a region's kind.
    fn classifies(&self) -> bool {
        true
    }

    /// How to phrase `WindowHit::detail_value` for a coalesced span.
    fn detail(&self, value: f32) -> String;

    /// `None` when this window says nothing.
    fn window(
        &self,
        rom: &RomImage,
        profile: &EntropyProfile,
        start: u32,
        len: u32,
    ) -> Option<WindowHit>;
}

/// Run every heuristic over the ROM and return coalesced hits, strongest
/// first. Ties keep the order the heuristics are listed in, so the output is
/// stable across runs.
pub fn run(rom: &RomImage, profile: &EntropyProfile) -> Vec<HeuristicHit> {
    let heuristics: Vec<Box<dyn Heuristic>> = vec![
        Box::new(palette::Palette),
        Box::new(ascii::Ascii),
        Box::new(pointers::Pointers),
        Box::new(entropy::Entropy),
        Box::new(graphics::Graphics),
    ];
    let mut hits = Vec::new();
    for h in &heuristics {
        hits.extend(coalesce(h.as_ref(), rom, profile));
    }
    // Strongest first, then by position: `analyze` paints in this order and
    // the first hit on a byte wins, so the sort *is* the conflict rule.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.start.cmp(&b.start))
    });
    hits
}

/// Walk the windows of one heuristic and merge adjacent agreeing ones.
fn coalesce(h: &dyn Heuristic, rom: &RomImage, profile: &EntropyProfile) -> Vec<HeuristicHit> {
    let n = rom.len() as u32;
    let mut out: Vec<HeuristicHit> = Vec::new();
    let mut run: Option<(RegionKind, f32, u32, u32, f32, u32)> = None;
    let mut start = 0u32;
    while start < n {
        let len = WINDOW.min(n - start);
        let hit = h
            .window(rom, profile, start, len)
            .map(|w| (w.kind, quantize(w.score), w.detail_value));
        match (&mut run, hit) {
            // Extend a run whose kind and quantized score match.
            (Some((kind, score, _, end, sum, count)), Some((k, s, v)))
                if *kind == k && (*score - s).abs() < f32::EPSILON =>
            {
                *end = start + len;
                *sum += v;
                *count += 1;
            }
            (open, hit) => {
                if let Some((kind, score, from, end, sum, count)) = open.take() {
                    out.push(HeuristicHit {
                        start: from,
                        len: end - from,
                        kind,
                        score,
                        name: h.name(),
                        detail: h.detail(sum / count as f32),
                        classifies: h.classifies(),
                    });
                }
                if let Some((k, s, v)) = hit {
                    *open = Some((k, s, start, start + len, v, 1));
                }
            }
        }
        start += len;
    }
    if let Some((kind, score, from, end, sum, count)) = run {
        out.push(HeuristicHit {
            start: from,
            len: end - from,
            kind,
            score,
            name: h.name(),
            detail: h.detail(sum / count as f32),
            classifies: h.classifies(),
        });
    }
    out
}
