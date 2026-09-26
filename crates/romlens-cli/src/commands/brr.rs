//! `brr`: a BRR sound sample in the ROM, decoded block by block and value by
//! value (docs/23, A2). It prints numbers and a text waveform, never audio
//! (`12-content-policy.md` rule 11).

use std::path::Path;

use anyhow::{Result, anyhow};
use romlens_core::dsp::brr::{Sample, decode_sample};

use crate::commands::session::{load_rom, rom_offset};

pub struct BrrArgs<'a> {
    pub rom: &'a Path,
    pub at: &'a str,
    /// The loop point, as the sample directory gives it.
    pub loop_at: Option<&'a str>,
    pub blocks: bool,
    pub ascii: bool,
    pub max: usize,
}

pub fn run(a: BrrArgs) -> Result<()> {
    let rom = load_rom(a.rom)?;
    let start = rom_offset(&rom, a.at)?;
    let loop_at = a.loop_at.map(|e| rom_offset(&rom, e)).transpose()?;
    let bytes = rom.bytes();
    if start as usize + 9 > bytes.len() {
        return Err(anyhow!("a block is 9 bytes; the ROM ends first"));
    }
    let s = decode_sample(bytes, start, loop_at, a.max);
    print!("{}", summary(&s, start));
    if a.blocks {
        print!("{}", steps(&s));
    }
    if a.ascii {
        print!("{}", waveform(&s, 96, 15));
    }
    Ok(())
}

fn summary(s: &Sample, start: u32) -> String {
    let n = s.samples().len();
    let mut out = format!(
        "BRR at 0x{start:06X}: {} blocks, {} bytes, {n} samples ({:.1} ms at pitch $1000, 32 kHz)\n",
        s.blocks.len(),
        s.len(),
        n as f64 / 32.0
    );
    out += &if s.unterminated {
        format!("No end flag in the first {} blocks.\n", s.blocks.len())
    } else if s.loops {
        match s.loop_block {
            Some(b) => format!(
                "Ends at block {} and loops to block {b}.\n",
                s.blocks.len() - 1
            ),
            None => {
                "Ends with the loop flag; the loop point is not one of its blocks.\n".to_owned()
            }
        }
    } else {
        format!("Ends at block {} and stops.\n", s.blocks.len() - 1)
    };
    out += "block  offset    header  shift filter  flags        lowest  highest\n";
    for (i, (b, at)) in s.blocks.iter().zip(&s.starts).enumerate() {
        let samples = b.samples();
        let flags = [
            (b.header.loops, "loop"),
            (b.header.end, "end"),
            (s.loop_block == Some(i), "<-loop point"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join(" ");
        out += &format!(
            "{i:>5}  0x{at:06X}  ${:02X}     {:>5} {:>6}  {flags:<12} {:>7} {:>8}\n",
            b.header.byte(),
            b.header.shift,
            b.header.filter,
            samples.iter().min().unwrap(),
            samples.iter().max().unwrap(),
        );
    }
    out
}

fn steps(s: &Sample) -> String {
    let mut out = String::new();
    for (i, b) in s.blocks.iter().enumerate() {
        let h = b.header;
        out += &format!(
            "\nblock {i}: shift {}, filter {} ({}): {}\n",
            h.shift,
            h.filter,
            h.filter_meaning(),
            h.filter_formula()
        );
        out += "   #  nibble  shifted      p1      p2  predict  clamped  result  output\n";
        for (j, st) in b.steps.iter().enumerate() {
            let note = match (st.clipped(), st.wrapped()) {
                (true, true) => "  clipped, wrapped",
                (true, false) => "  clipped",
                (false, true) => "  wrapped",
                _ => "",
            };
            out += &format!(
                "  {j:>2}  {:>6}  {:>7}  {:>6}  {:>6}  {:>7}  {:>7}  {:>6}  {:>6}{note}\n",
                st.nibble,
                st.shifted,
                st.p1,
                st.p2,
                st.prediction,
                st.clamped,
                st.result,
                st.output
            );
        }
    }
    out
}

/// Each column is the lowest and highest sample of its stretch.
fn waveform(s: &Sample, width: usize, rows: usize) -> String {
    let samples = s.samples();
    if samples.is_empty() {
        return String::new();
    }
    let width = width.min(samples.len());
    let per = samples.len().div_ceil(width);
    let row_of = |v: i16| ((32767 - v as i32) as usize * (rows - 1)) / 65535;
    let mut grid = vec![vec![' '; width]; rows];
    for (c, chunk) in samples.chunks(per).enumerate().take(width) {
        let (lo, hi) = (*chunk.iter().min().unwrap(), *chunk.iter().max().unwrap());
        for row in grid.iter_mut().take(row_of(lo) + 1).skip(row_of(hi)) {
            row[c] = '#';
        }
    }
    let mid = row_of(0);
    for cell in grid[mid].iter_mut() {
        if *cell == ' ' {
            *cell = '-';
        }
    }
    let mut out = String::from("\n");
    for row in grid {
        out += &format!("|{}|\n", row.into_iter().collect::<String>());
    }
    if let Some(b) = s.loop_block {
        let col = (b * 16) / per;
        out += &format!(" {}^ loop\n", " ".repeat(col));
    }
    out
}
