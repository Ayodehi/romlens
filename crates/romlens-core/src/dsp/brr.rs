//! BRR, the S-DSP's sample format (docs/23, A2), from fullsnes and
//! anomie's S-DSP document.
//!
//! A block is nine bytes: a header, then sixteen 4-bit signed values, high
//! nibble first. The header's top four bits are a shift, the next two a
//! filter, then the loop and end flags:
//!
//! ```text
//!   bits 7-4  shift 0-12 (13-15 are not valid: 0, or -2048 for a negative nibble)
//!   bits 3-2  filter 0-3, which predicts each sample from the last two
//!   bit 1     loop: at the end, go to the sample's loop point
//!   bit 0     end: this is the sample's last block
//! ```
//!
//! Each value is shifted, `(nibble << shift) >> 1`, and the filter adds its
//! prediction from the previous two results. The DSP keeps the sum in 15
//! bits: it is clamped to 16 bits and then loses its top bit, so a sum past
//! ±16384 wraps. What plays is that value doubled.

/// A block's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub shift: u8,
    pub filter: u8,
    pub loops: bool,
    pub end: bool,
}

impl Header {
    pub const fn from_byte(b: u8) -> Header {
        Header {
            shift: b >> 4,
            filter: (b >> 2) & 3,
            loops: b & 2 != 0,
            end: b & 1 != 0,
        }
    }

    pub const fn byte(self) -> u8 {
        (self.shift << 4) | (self.filter << 2) | (self.loops as u8) << 1 | self.end as u8
    }

    /// The filter as the arithmetic it does, in the DSP's own terms.
    pub const fn filter_formula(self) -> &'static str {
        match self.filter {
            0 => "s",
            1 => "s + p1 + (-p1 >> 4)",
            2 => "s + 2·p1 + (-3·p1 >> 5) - p2 + (p2 >> 4)",
            _ => "s + 2·p1 + (-13·p1 >> 6) - p2 + (3·p2 >> 4)",
        }
    }

    /// The same, as the fractions it approximates.
    pub const fn filter_meaning(self) -> &'static str {
        match self.filter {
            0 => "no prediction: each value stands alone",
            1 => "s + 15/16 of the last sample",
            2 => "s + 61/32 of the last, less 15/16 of the one before",
            _ => "s + 115/64 of the last, less 13/16 of the one before",
        }
    }
}

/// One sample's decoding, step by step, for teaching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The nibble as a signed value, -8 to 7.
    pub nibble: i8,
    /// After the shift.
    pub shifted: i32,
    /// The previous two results (15-bit), which the filter reads.
    pub p1: i32,
    pub p2: i32,
    /// The filter's prediction, added to `shifted`.
    pub prediction: i32,
    /// `shifted + prediction`, clamped to 16 bits.
    pub clamped: i32,
    /// Kept in 15 bits: the next sample's `p1`.
    pub result: i32,
    /// What plays: `result` doubled, a 16-bit sample.
    pub output: i16,
}

impl Step {
    /// The 15-bit wrap changed the value.
    pub fn wrapped(&self) -> bool {
        self.result != self.clamped
    }

    /// The 16-bit clamp changed the value.
    pub fn clipped(&self) -> bool {
        self.clamped != self.shifted + self.prediction
    }
}

/// A decoded block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: Header,
    pub steps: [Step; 16],
}

impl Block {
    pub fn samples(&self) -> [i16; 16] {
        self.steps.map(|s| s.output)
    }
}

/// The decoder's memory of the last two results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct History {
    pub p1: i32,
    pub p2: i32,
}

fn shifted(nibble: i32, shift: u8) -> i32 {
    if shift <= 12 {
        (nibble << shift) >> 1
    } else if nibble < 0 {
        -2048
    } else {
        0
    }
}

fn prediction(filter: u8, p1: i32, p2: i32) -> i32 {
    match filter {
        0 => 0,
        1 => p1 + ((-p1) >> 4),
        2 => 2 * p1 + ((-3 * p1) >> 5) - p2 + (p2 >> 4),
        _ => 2 * p1 + ((-13 * p1) >> 6) - p2 + ((3 * p2) >> 4),
    }
}

/// Decode one 9-byte block, carrying `history` from the block before.
pub fn decode_block(bytes: &[u8; 9], history: &mut History) -> Block {
    let header = Header::from_byte(bytes[0]);
    let mut steps = [Step {
        nibble: 0,
        shifted: 0,
        p1: 0,
        p2: 0,
        prediction: 0,
        clamped: 0,
        result: 0,
        output: 0,
    }; 16];
    for (i, step) in steps.iter_mut().enumerate() {
        let byte = bytes[1 + i / 2];
        let raw = if i % 2 == 0 { byte >> 4 } else { byte & 0xF };
        let nibble = ((raw << 4) as i8) >> 4;
        let s = shifted(nibble as i32, header.shift);
        let (p1, p2) = (history.p1, history.p2);
        let pred = prediction(header.filter, p1, p2);
        let clamped = (s + pred).clamp(-0x8000, 0x7FFF);
        // Keep 15 bits: the doubled value as 16 bits, halved again.
        let result = (((clamped << 1) as i16) >> 1) as i32;
        *step = Step {
            nibble,
            shifted: s,
            p1,
            p2,
            prediction: pred,
            clamped,
            result,
            output: (result << 1) as i16,
        };
        history.p2 = p1;
        history.p1 = result;
    }
    Block { header, steps }
}

/// A sample walked from its start to its end block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// Where each block starts, in the memory given.
    pub starts: Vec<u32>,
    pub blocks: Vec<Block>,
    /// The block the loop point falls on, when it is one of the sample's.
    pub loop_block: Option<usize>,
    /// The end block's loop flag: at its end the voice goes to the loop
    /// point rather than stopping.
    pub loops: bool,
    /// The walk stopped at `max_blocks` or at the end of memory, not at an
    /// end flag.
    pub unterminated: bool,
}

impl Sample {
    pub fn samples(&self) -> Vec<i16> {
        self.blocks.iter().flat_map(|b| b.samples()).collect()
    }

    /// Bytes from the start to the end of the last block.
    pub fn len(&self) -> u32 {
        self.blocks.len() as u32 * 9
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Walk a sample from `start` in `memory` until a block with the end flag,
/// decoding each. `loop_point` is the directory's loop address, marked when
/// it is the start of one of the blocks. A sample starts from silence.
pub fn decode_sample(
    memory: &[u8],
    start: u32,
    loop_point: Option<u32>,
    max_blocks: usize,
) -> Sample {
    let mut history = History::default();
    let mut out = Sample {
        starts: Vec::new(),
        blocks: Vec::new(),
        loop_block: None,
        loops: false,
        unterminated: true,
    };
    let mut at = start as usize;
    while out.blocks.len() < max_blocks {
        let Some(bytes) = memory.get(at..at + 9) else {
            break;
        };
        let block = decode_block(bytes.try_into().unwrap(), &mut history);
        if loop_point == Some(at as u32) {
            out.loop_block = Some(out.blocks.len());
        }
        out.starts.push(at as u32);
        let end = block.header.end;
        out.loops = block.header.loops;
        out.blocks.push(block);
        if end {
            out.unterminated = false;
            break;
        }
        at += 9;
    }
    out
}

/// Encode sixteen samples as one block with a fixed shift and filter 0,
/// for fixtures and tests: each nibble is the sample's top bits.
pub fn encode_block_plain(samples: &[i16; 16], shift: u8, loops: bool, end: bool) -> [u8; 9] {
    let mut out = [0u8; 9];
    out[0] = Header {
        shift,
        filter: 0,
        loops,
        end,
    }
    .byte();
    for (i, s) in samples.iter().enumerate() {
        // output = ((n << shift) >> 1) << 1, so n = output >> shift.
        let n = ((*s as i32) >> shift.max(1)).clamp(-8, 7) as u8 & 0xF;
        out[1 + i / 2] |= if i % 2 == 0 { n << 4 } else { n };
    }
    out
}
