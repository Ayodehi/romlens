//! `truth`: turn a trace into a ground-truth file for `romlens accuracy`.
//!
//! The output is never committed: it describes a commercial ROM and is
//! therefore derived from it (`12-content-policy.md`). It is written beside
//! the developer's own recording, for their own measurement.

use std::path::Path;

use anyhow::{Context, Result, bail};
use romlens_core::analysis::accuracy::{GroundTruth, TruthRange};
use romlens_core::io::import::read_trace;
use romlens_core::model::RegionKind;

use crate::commands::session::load_rom;

pub fn from_cdl(rom: &Path, file: &Path, out: &Path) -> Result<()> {
    let rom = load_rom(rom)?;
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let (format, coverage) = read_trace(&bytes, &rom, None)?;
    if coverage.is_empty() {
        bail!("{} records nothing for this ROM", file.display());
    }

    // A byte an emulator executed is code; a byte it only ever read is data.
    // A byte it never touched is *unlabelled*, not "unknown": the player did
    // not go there, which says nothing about what is in it. Truth that claimed
    // otherwise would punish the classifier for being right about code the
    // session never reached.
    let mut ranges: Vec<TruthRange> = Vec::new();
    let mut push = |start: u32, end: u32, kind: RegionKind| {
        if let Some(last) = ranges.last_mut()
            && last.end == start
            && last.kind == kind
        {
            last.end = end;
        } else {
            ranges.push(TruthRange { start, end, kind });
        }
    };
    for off in 0..coverage.len {
        if coverage.executed.get(off) {
            push(off, off + 1, RegionKind::Code);
        } else if coverage.read.get(off) {
            push(
                off,
                off + 1,
                RegionKind::Data(romlens_core::model::DataKind::Byte),
            );
        }
    }
    let truth = GroundTruth {
        sha256: Some(rom.sha256_hex()),
        ranges,
    };
    let labelled = truth.labelled_bytes();
    std::fs::write(out, truth.to_tsv()).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} from {} ({}): {} ranges, {labelled} labelled bytes ({:.1}% of the image)",
        out.display(),
        file.display(),
        format.name(),
        truth.ranges.len(),
        labelled as f64 * 100.0 / rom.len() as f64
    );
    Ok(())
}
