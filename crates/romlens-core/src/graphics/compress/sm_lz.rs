//! Super Metroid's LZ format — "LZ5" in SnesLab's numbering, which Super
//! Mario Kart shares.
//!
//! Verified against the game's own routine (`$80:B119`, read from a local
//! copy of the bank logs, never redistributed — `12-content-policy.md`
//! rule 6) and SnesLab's LZ5 page. A stream is chunks, each a header byte
//! and a parameter, ending at a header of `$FF`:
//!
//! ```text
//! CCCLLLLL                length L + 1 (1–32), command C
//! 111CCCLL LLLLLLLL       the long form: length L + 1 (1–1024)
//!
//! 0  direct copy          L+1 literal bytes follow
//! 1  byte fill            one byte, written L+1 times
//! 2  word fill            two bytes, alternated for L+1 bytes (an odd
//!                         length ends on the first)
//! 3  incrementing fill    one byte, written L+1 times, +1 each (wraps at 8 bits)
//! 4  dictionary copy      u16 offset from the start of the output
//! 5  inverted dictionary  the same, each byte XOR $FF
//! 6  sliding copy         one byte n: copy from (current position − n)
//! 7  inverted sliding     the same, each byte XOR $FF
//! ```
//!
//! Command 7 is reachable only in the long form, since a short `111xxxxx`
//! header *is* the long form; and `$FF` is always the terminator, never a
//! long command. Copies read one byte at a time, so a copy may overlap its
//! own output — that is how a run repeats.

/// The decompressor refuses to produce more than this. Super Metroid
/// decompresses into buffers of at most 64 KB, and the dictionary offset is
/// sixteen bits, so a longer output cannot be a real stream — and the
/// heuristic that calls this on arbitrary bytes needs it to fail fast.
pub const MAX_OUTPUT: usize = 0x10000;

/// Why a decompression stopped short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmLzError {
    /// The input ran out at this offset before a terminator.
    Truncated { at: usize },
    /// A copy at input offset `at` reaches before the start of the output.
    BadReference { at: usize },
    /// The output would exceed [`MAX_OUTPUT`].
    TooLarge { at: usize },
}

impl std::fmt::Display for SmLzError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmLzError::Truncated { at } => {
                write!(
                    f,
                    "the stream ends at input byte {at:#x} without a $FF terminator"
                )
            }
            SmLzError::BadReference { at } => write!(
                f,
                "the copy at input byte {at:#x} reaches before the start of the output"
            ),
            SmLzError::TooLarge { at } => write!(
                f,
                "the output passes {MAX_OUTPUT} bytes at input byte {at:#x}; this is not a Super Metroid stream"
            ),
        }
    }
}

/// The eight command names, by number.
pub const COMMAND_NAMES: [&str; 8] = [
    "direct copy",
    "byte fill",
    "word fill",
    "incrementing fill",
    "dictionary copy",
    "inverted dictionary copy",
    "sliding copy",
    "inverted sliding copy",
];

/// What a successful decompression did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decompressed {
    pub output: Vec<u8>,
    /// Input bytes consumed, including the terminator.
    pub consumed: usize,
    /// Chunks per command.
    pub commands: [u32; 8],
    /// Chunks that used the long header.
    pub long_headers: u32,
}

/// Decompress from the start of `bytes`: the output and the number of input
/// bytes consumed, or why it failed. This is the signature track 2A's
/// compressed-data heuristic was promised.
pub fn try_decompress(bytes: &[u8]) -> Result<(Vec<u8>, usize), SmLzError> {
    decompress(bytes).map(|d| (d.output, d.consumed))
}

/// [`try_decompress`] with the per-command statistics `--stats` prints.
pub fn decompress(bytes: &[u8]) -> Result<Decompressed, SmLzError> {
    let mut out: Vec<u8> = Vec::new();
    let mut commands = [0u32; 8];
    let mut long_headers = 0u32;
    let mut pos = 0usize;
    let next = |pos: &mut usize| -> Result<u8, SmLzError> {
        let b = *bytes.get(*pos).ok_or(SmLzError::Truncated { at: *pos })?;
        *pos += 1;
        Ok(b)
    };
    loop {
        let at = pos;
        let header = next(&mut pos)?;
        if header == 0xFF {
            break;
        }
        let (command, len) = if header >> 5 == 7 {
            long_headers += 1;
            let low = next(&mut pos)?;
            (
                (header >> 2) & 7,
                (((header as usize) & 3) << 8 | low as usize) + 1,
            )
        } else {
            (header >> 5, (header as usize & 0x1F) + 1)
        };
        if out.len() + len > MAX_OUTPUT {
            return Err(SmLzError::TooLarge { at });
        }
        commands[command as usize] += 1;
        match command {
            0 => {
                for _ in 0..len {
                    let b = next(&mut pos)?;
                    out.push(b);
                }
            }
            1 => {
                let b = next(&mut pos)?;
                out.extend(std::iter::repeat_n(b, len));
            }
            2 => {
                let pair = [next(&mut pos)?, next(&mut pos)?];
                out.extend((0..len).map(|i| pair[i % 2]));
            }
            3 => {
                let b = next(&mut pos)?;
                out.extend((0..len).map(|i| b.wrapping_add(i as u8)));
            }
            _ => {
                let invert = if command & 1 != 0 { 0xFF } else { 0x00 };
                let from = if command < 6 {
                    u16::from_le_bytes([next(&mut pos)?, next(&mut pos)?]) as usize
                } else {
                    let back = next(&mut pos)? as usize;
                    out.len()
                        .checked_sub(back)
                        .ok_or(SmLzError::BadReference { at })?
                };
                for i in 0..len {
                    // Byte by byte, so an overlapping copy repeats itself.
                    let b = *out.get(from + i).ok_or(SmLzError::BadReference { at })?;
                    out.push(b ^ invert);
                }
            }
        }
    }
    Ok(Decompressed {
        output: out,
        consumed: pos,
        commands,
        long_headers,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A compressor, in the test module only: the round trip is the test, and
    /// there is no shippable ground truth to compare against. Greedy, and it
    /// uses every command, so a decoder bug in any of them shows up.
    pub(crate) fn compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut literal: Vec<u8> = Vec::new();
        let mut i = 0;
        fn header(out: &mut Vec<u8>, command: u8, len: usize) {
            let l = len - 1;
            if l < 32 && command != 7 {
                out.push(command << 5 | l as u8);
            } else {
                out.push(0xE0 | command << 2 | (l >> 8) as u8);
                out.push(l as u8);
            }
        }
        fn flush(out: &mut Vec<u8>, literal: &mut Vec<u8>) {
            for chunk in literal.chunks(1024) {
                header(out, 0, chunk.len());
                out.extend_from_slice(chunk);
            }
            literal.clear();
        }
        // The best candidate at `i`: (command, length, parameter bytes).
        let candidates = |i: usize| -> Vec<(u8, usize, Vec<u8>)> {
            let max = (data.len() - i).min(1024);
            let run =
                |f: &dyn Fn(usize) -> u8| (0..max).take_while(|&k| data[i + k] == f(k)).count();
            let mut c = vec![
                (1, run(&|_| data[i]), vec![data[i]]),
                (3, run(&|k| data[i].wrapping_add(k as u8)), vec![data[i]]),
            ];
            if i + 1 < data.len() {
                let pair = [data[i], data[i + 1]];
                c.push((2, run(&|k| pair[k % 2]), pair.to_vec()));
            }
            for from in 0..i {
                for invert in [0u8, 0xFF] {
                    let len = (0..max)
                        .take_while(|&k| data[i + k] == data[from + k] ^ invert)
                        .count();
                    if len == 0 {
                        continue;
                    }
                    let back = i - from;
                    if back < 256 {
                        c.push((6 | (invert & 1), len, vec![back as u8]));
                    } else {
                        c.push((4 | (invert & 1), len, (from as u16).to_le_bytes().to_vec()));
                    }
                }
            }
            c
        };
        while i < data.len() {
            let best = candidates(i)
                .into_iter()
                .filter(|(_, len, p)| *len > p.len() + 1)
                .max_by_key(|(_, len, p)| *len as isize - p.len() as isize);
            match best {
                Some((command, len, param)) => {
                    flush(&mut out, &mut literal);
                    header(&mut out, command, len);
                    out.extend(param);
                    i += len;
                }
                None => {
                    literal.push(data[i]);
                    i += 1;
                }
            }
        }
        flush(&mut out, &mut literal);
        out.push(0xFF);
        out
    }

    #[test]
    fn each_command_by_hand() {
        // Direct copy of three, byte fill of four, word fill of five,
        // incrementing fill of three from $FE, then the terminator.
        let s = [0x02, 1, 2, 3, 0x23, 9, 0x44, 0xA, 0xB, 0x62, 0xFE, 0xFF];
        let d = decompress(&s).unwrap();
        assert_eq!(
            d.output,
            vec![
                1, 2, 3, 9, 9, 9, 9, 0xA, 0xB, 0xA, 0xB, 0xA, 0xFE, 0xFF, 0x00
            ]
        );
        assert_eq!(d.consumed, s.len());
        assert_eq!(&d.commands[..4], &[1, 1, 1, 1]);
    }

    #[test]
    fn copies_read_their_own_output() {
        // "AB", then a dictionary copy of six from offset 0 (overlapping),
        // an inverted one of two, a sliding copy of three from two back and
        // an inverted sliding copy of one from one back, in the long form.
        let s = [
            0x01, b'A', b'B', //
            0x85, 0x00, 0x00, //
            0xA1, 0x00, 0x00, //
            0xC2, 0x02, //
            0xFC, 0x00, 0x01, //
            0xFF,
        ];
        let d = decompress(&s).unwrap();
        assert_eq!(
            d.output,
            b"ABABABAB\xBE\xBD\xBE\xBD\xBEA".to_vec(),
            "the sliding copy repeats the inverted pair; the inverted slide undoes one"
        );
        assert_eq!(d.long_headers, 1);
        assert_eq!(d.commands[7], 1);
    }

    #[test]
    fn malformed_streams_fail_with_a_position() {
        assert_eq!(decompress(&[0x02, 1]), Err(SmLzError::Truncated { at: 2 }));
        assert_eq!(decompress(&[]), Err(SmLzError::Truncated { at: 0 }));
        assert_eq!(
            decompress(&[0xC0, 0x01, 0xFF]),
            Err(SmLzError::BadReference { at: 0 })
        );
        assert_eq!(
            decompress(&[0x80, 0x05, 0x00, 0xFF]),
            Err(SmLzError::BadReference { at: 0 })
        );
        // 65 long byte fills of 1024 pass the cap.
        let mut s = Vec::new();
        for _ in 0..65 {
            s.extend([0xE7, 0xFF, 0x00]);
        }
        s.push(0xFF);
        assert!(matches!(decompress(&s), Err(SmLzError::TooLarge { .. })));
    }

    #[test]
    fn round_trips_generated_data() {
        let mut seed = 0x1234_5678u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as u8
        };
        let mut data = Vec::new();
        for _ in 0..40 {
            let kind = rand() % 6;
            let n = 1 + rand() as usize % 300;
            match kind {
                0 => data.extend((0..n).map(|_| rand())),
                1 => data.extend(std::iter::repeat_n(rand(), n)),
                2 => {
                    let p = [rand(), rand()];
                    data.extend((0..n).map(|k| p[k % 2]));
                }
                3 => {
                    let b = rand();
                    data.extend((0..n).map(|k| b.wrapping_add(k as u8)));
                }
                _ if !data.is_empty() => {
                    let from = rand() as usize * data.len() / 256;
                    let invert = if kind == 5 { 0xFF } else { 0 };
                    let copy: Vec<u8> = data[from..].iter().take(n).map(|b| b ^ invert).collect();
                    data.extend(copy);
                }
                _ => {}
            }
        }
        let packed = compress(&data);
        let d = decompress(&packed).unwrap();
        assert_eq!(d.output, data);
        assert_eq!(d.consumed, packed.len());
        assert!(
            packed.len() < data.len(),
            "{} vs {}",
            packed.len(),
            data.len()
        );
        assert!(
            d.commands.iter().all(|c| *c > 0),
            "every command exercised: {:?}",
            d.commands
        );
        // Trailing bytes after the terminator are not consumed.
        let mut tail = packed.clone();
        tail.extend([1, 2, 3]);
        assert_eq!(try_decompress(&tail).unwrap().1, packed.len());
    }
}
