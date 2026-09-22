//! `accuracy`: score the classifier against ground truth (checklist 2.8).

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use romlens_core::analysis::accuracy::{Accuracy, GroundTruth, score};
use romlens_core::fixtures;

use crate::commands::session;

/// How many disagreement ranges to print. Ten is enough to see a pattern and
/// few enough to read; the point is that a number that moved is actionable.
const WORST: usize = 10;

pub fn run(
    rom: &Path,
    project: Option<&Path>,
    truth: Option<&PathBuf>,
    fixture: bool,
    json: bool,
) -> Result<()> {
    let s = session::open(rom, project, false)?;
    let truth = match (truth, fixture) {
        (Some(path), false) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| anyhow!("reading {}: {e}", path.display()))?;
            let truth = GroundTruth::parse(&text)?;
            if let Some(sha) = &truth.sha256
                && *sha != s.rom.sha256_hex()
            {
                bail!(
                    "{} describes a different ROM (expected SHA-256 {sha}, found {})",
                    path.display(),
                    s.rom.sha256_hex()
                );
            }
            truth
        }
        (None, true) => {
            let title = s.rom.header().title.clone();
            let Some(ranges) = fixtures::truth_for_title(&title, s.rom.mapping()) else {
                bail!(
                    "no built-in truth for {:?}; --fixture works on the ROMs \
                     `romlens testrom` writes, otherwise pass --truth <file>",
                    title.trim()
                );
            };
            GroundTruth {
                sha256: Some(s.rom.sha256_hex()),
                ranges,
            }
        }
        _ => bail!("pass exactly one of --truth <file> and --fixture"),
    };
    if truth.ranges.is_empty() {
        bail!("the ground truth names no ranges");
    }
    let a = score(&s.snap, &truth, WORST);
    print!(
        "{}",
        if json {
            json_report(&a)
        } else {
            text_report(&a)
        }
    );
    Ok(())
}

fn text_report(a: &Accuracy) -> String {
    let mut out = format!(
        "Labelled:      {} bytes\n\
Correct:       {} bytes ({:.1}% by class, {:.1}% by exact kind)\n\n\
{:<8} {:>10} {:>10} {:>10} {:>10} {:>10}\n",
        a.labelled_bytes,
        a.correct_bytes,
        a.overall() * 100.0,
        if a.labelled_bytes == 0 {
            0.0
        } else {
            a.exact_bytes as f64 * 100.0 / a.labelled_bytes as f64
        },
        "class",
        "truth",
        "claimed",
        "precision",
        "recall",
        "f1"
    );
    for c in &a.classes {
        out.push_str(&format!(
            "{:<8} {:>10} {:>10} {:>10.3} {:>10.3} {:>10.3}\n",
            c.class.name(),
            c.truth_bytes,
            c.claimed_bytes,
            c.precision(),
            c.recall(),
            c.f1()
        ));
    }
    if !a.disagreements.is_empty() {
        out.push_str("\nlargest disagreements:\n");
        for d in &a.disagreements {
            out.push_str(&format!("{d}\n"));
        }
    }
    out
}

fn json_report(a: &Accuracy) -> String {
    let classes: Vec<String> = a
        .classes
        .iter()
        .map(|c| {
            format!(
                "    {{\"class\": \"{}\", \"truthBytes\": {}, \"claimedBytes\": {}, \
\"correctBytes\": {}, \"precision\": {:.4}, \"recall\": {:.4}, \"f1\": {:.4}}}",
                c.class.name(),
                c.truth_bytes,
                c.claimed_bytes,
                c.correct_bytes,
                c.precision(),
                c.recall(),
                c.f1()
            )
        })
        .collect();
    let worst: Vec<String> = a
        .disagreements
        .iter()
        .map(|d| {
            format!(
                "    {{\"start\": {}, \"end\": {}, \"truth\": \"{}\", \"found\": \"{}\"}}",
                d.start,
                d.end,
                d.truth.name(),
                d.found.name()
            )
        })
        .collect();
    let array = |rows: Vec<String>| {
        if rows.is_empty() {
            "[]".to_owned()
        } else {
            format!("[\n{}\n  ]", rows.join(",\n"))
        }
    };
    format!(
        "{{\n  \"labelledBytes\": {},\n  \"correctBytes\": {},\n  \"exactBytes\": {},\n  \
\"overall\": {:.4},\n  \"classes\": {},\n  \"disagreements\": {}\n}}\n",
        a.labelled_bytes,
        a.correct_bytes,
        a.exact_bytes,
        a.overall(),
        array(classes),
        array(worst)
    )
}
