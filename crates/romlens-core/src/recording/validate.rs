//! `rec validate`: everything wrong with a `.romrec`, not just the first
//! thing (`13-recording-format.md`, "Producing a recording").
//!
//! The reader refuses a damaged file at the first problem it meets, which is
//! right for opening one. A producer, though — a bsnes script, a homebrew
//! test harness, a capture rig — needs every problem at once, each with a
//! stable code a test can assert. So this walks the whole file itself,
//! chunk by chunk, and never stops early.
//!
//! The codes, by family:
//!
//! | Family | What it checks |
//! |---|---|
//! | `H` | the header: magic, version, length, region table, strings, fields |
//! | `F` | the footer, the frame counts it and the header give, and both CRC-32s |
//! | `I` | the index: numbering, offsets, lengths and kinds against the chunks |
//! | `T` | the chunk walk from the header to the index: magics and lengths |
//! | `K` | keyframes: the first frame, the declared interval, frame numbering |
//! | `D` | each frame's directory: known regions, order, the small regions |
//! | `R` | runs: bounds, order, whole-region keyframes, lengths |
//! | `P` | payloads: they decompress, to the length the directory gives |
//! | `L` | layers: the header's bits against the chunks, `WLOG` bodies |
//! | `W` | WRAM against the keyframe-only flag |
//! | `M` | the ROM the recording was made from, when one is given |
//! | `S` | sampled frames rebuilt, with `changes` checked against the truth |

use std::collections::BTreeSet;
use std::io::SeekFrom;

use crate::io::crc32::crc32;
use crate::recording::delta::Run;
use crate::recording::format::*;
use crate::recording::reader::ReadSeek;
use crate::recording::{Layers, MachineStateSource, RomrecSource, StateRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Allowed by the format, but not what a careful producer writes.
    Warning,
    /// The file breaks the format.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub frame: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ValidateOptions {
    /// Check the recording was made from this ROM.
    pub rom_sha256: Option<[u8; 32]>,
    /// Rebuild this many frames, spread through the file, and check each
    /// one's `changes` covers every byte that really changed. 0 skips it.
    pub sample: u32,
    /// A file with no footer is in progress rather than broken: say so as a
    /// warning and check the frames that made it to disk.
    pub recover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidateReport {
    pub diagnostics: Vec<Diagnostic>,
    pub frames: u64,
    pub sampled: u32,
}

impl ValidateReport {
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }
    fn count(&self, s: Severity) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == s).count()
    }
    /// The codes found, in order, for tests.
    pub fn codes(&self) -> Vec<&'static str> {
        self.diagnostics.iter().map(|d| d.code).collect()
    }
}

/// A diagnostic stream with a cap per code, so a file broken in every frame
/// says so a few times and then counts rather than printing 18,000 lines.
struct Out {
    report: ValidateReport,
    per_code: std::collections::HashMap<&'static str, u32>,
}

const PER_CODE: u32 = 5;

impl Out {
    fn push(
        &mut self,
        code: &'static str,
        severity: Severity,
        frame: Option<u64>,
        message: String,
    ) {
        let n = self.per_code.entry(code).or_insert(0);
        *n += 1;
        if *n <= PER_CODE {
            self.report.diagnostics.push(Diagnostic {
                code,
                severity,
                frame,
                message,
            });
        }
    }
    fn error(&mut self, code: &'static str, frame: Option<u64>, message: impl Into<String>) {
        self.push(code, Severity::Error, frame, message.into());
    }
    fn warn(&mut self, code: &'static str, frame: Option<u64>, message: impl Into<String>) {
        self.push(code, Severity::Warning, frame, message.into());
    }
    /// A line per code that was capped, saying how many more there were.
    fn finish(mut self) -> ValidateReport {
        let mut capped: Vec<(&'static str, u32)> = self
            .per_code
            .iter()
            .filter(|(_, n)| **n > PER_CODE)
            .map(|(c, n)| (*c, *n))
            .collect();
        capped.sort();
        for (code, n) in capped {
            let severity = self
                .report
                .diagnostics
                .iter()
                .find(|d| d.code == code)
                .map_or(Severity::Error, |d| d.severity);
            self.report.diagnostics.push(Diagnostic {
                code,
                severity,
                frame: None,
                message: format!("{} more like this", n - PER_CODE),
            });
        }
        self.report
    }
}

fn read_at(file: &mut dyn ReadSeek, at: u64, len: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; len];
    file.seek(SeekFrom::Start(at)).ok()?;
    file.read_exact(&mut b).ok()?;
    Some(b)
}

/// What the chunk walk learned about one frame chunk.
struct Seen {
    frame: u64,
    offset: u64,
    len: u32,
    kind: u8,
}

/// Validate a recording.
pub fn validate(mut file: Box<dyn ReadSeek>, options: ValidateOptions) -> ValidateReport {
    let mut out = Out {
        report: ValidateReport::default(),
        per_code: Default::default(),
    };
    let Ok(file_len) = file.seek(SeekFrom::End(0)) else {
        out.error("H1", None, "the file cannot be read");
        return out.finish();
    };

    // H: the header.
    let first = read_at(&mut *file, 0, (file_len as usize).min(16)).unwrap_or_default();
    if first.len() < 8 || &first[..8] != MAGIC {
        out.error("H1", None, "it does not start with ROMREC");
        return out.finish();
    }
    let header_len = match Header::declared_len(&first) {
        Ok(n) if n as u64 <= file_len => n,
        Ok(n) => {
            out.error(
                "H3",
                None,
                format!("the header claims {n} bytes; the file is {file_len}"),
            );
            return out.finish();
        }
        Err(e) => {
            out.error("H3", None, e.to_string());
            return out.finish();
        }
    };
    let hb = read_at(&mut *file, 0, header_len).unwrap_or_default();
    let (major, minor) = (u16_at(&hb, 8), u16_at(&hb, 10));
    if major > VERSION_MAJOR {
        out.error("H2", None, format!("format {major}.{minor} is newer than this Romlens reads ({VERSION_MAJOR}.{VERSION_MINOR})"));
        return out.finish();
    }
    if major == VERSION_MAJOR && minor > VERSION_MINOR {
        out.warn("H2", None, format!("format {major}.{minor}: fields newer than {VERSION_MAJOR}.{VERSION_MINOR} are not checked"));
    }
    let header = match Header::decode(&hb) {
        Ok(h) => h,
        Err(e) => {
            let code = if e.to_string().contains("string") {
                "H5"
            } else {
                "H4"
            };
            out.error(code, None, e.to_string());
            return out.finish();
        }
    };
    let mut ids = BTreeSet::new();
    for r in &header.regions {
        if !ids.insert(r.id()) {
            out.error(
                "H4",
                None,
                format!("the region table lists {} twice", r.name()),
            );
        }
    }
    if header.keyframe_interval == 0 {
        out.error("H6", None, "the keyframe interval is 0");
    }
    if header.compression != COMPRESSION_NONE && header.compression != COMPRESSION_ZSTD {
        out.error(
            "H7",
            None,
            format!("compression {} is not defined", header.compression),
        );
    }
    if header.flags & !FLAG_WRAM_KEYFRAME_ONLY != 0 {
        out.warn(
            "H8",
            None,
            format!(
                "header flags {:#04x} include bits the format does not define",
                header.flags
            ),
        );
    }
    if header.mapping > 2 && header.mapping != 0xFF {
        out.warn(
            "H9",
            None,
            format!(
                "mapping {} is not LoROM (0), HiROM (1), ExHiROM (2) or unknown (255)",
                header.mapping
            ),
        );
    }

    // M: the ROM.
    if let Some(sha) = options.rom_sha256
        && sha != header.rom_sha256
    {
        out.error(
            "M1",
            None,
            "the recording was made from a different ROM (the SHA-256 differs)",
        );
    }

    // F: the footer and the counts.
    let footer = if file_len >= (header_len + FOOTER_LEN) as u64 {
        read_at(&mut *file, file_len - FOOTER_LEN as u64, FOOTER_LEN)
            .and_then(|b| Footer::decode(&b))
    } else {
        None
    };
    let mut index: Option<Vec<IndexEntry>> = None;
    let walk_end = match footer {
        None => {
            if options.recover {
                out.warn("F1", None, "there is no footer: the recording is in progress or was cut short; checking the frames on disk");
            } else {
                out.error("F1", None, "there is no footer: the recording is in progress or was cut short (--recover checks what is on disk)");
            }
            file_len
        }
        Some(f) => {
            if f.header_crc != crc32(&hb) {
                out.error("F4", None, "the header does not match the footer's CRC-32");
            }
            let at_index = f.index_offset >= header_len as u64
                && f.index_offset + 8 <= file_len - FOOTER_LEN as u64;
            let head = at_index
                .then(|| read_at(&mut *file, f.index_offset, 8))
                .flatten();
            match head {
                Some(h) if &h[0..4] == INDEX_MAGIC => {
                    let len = u32_at(&h, 4) as u64;
                    if f.index_offset + 8 + len != file_len - FOOTER_LEN as u64 {
                        out.error("F2", None, "the index does not end where the footer begins");
                    }
                    let body =
                        read_at(&mut *file, f.index_offset + 8, len as usize).unwrap_or_default();
                    if f.index_crc != crc32(&body) {
                        out.error("F5", None, "the index does not match the footer's CRC-32");
                    }
                    if !(len as usize).is_multiple_of(INDEX_ENTRY_LEN) {
                        out.error(
                            "I1",
                            None,
                            format!(
                                "the index is {len} bytes, not a multiple of {INDEX_ENTRY_LEN}"
                            ),
                        );
                    }
                    let entries = decode_index(&body);
                    if header.frame_count == IN_PROGRESS {
                        out.error("F3", None, "the footer is written but the header still says the recording is in progress");
                    } else if header.frame_count != f.frame_count
                        || entries.len() as u64 != f.frame_count
                    {
                        out.error(
                            "F3",
                            None,
                            format!(
                                "the header says {} frames, the footer {}, the index {}",
                                header.frame_count,
                                f.frame_count,
                                entries.len()
                            ),
                        );
                    }
                    index = Some(entries);
                }
                _ => out.error("F2", None, "the footer does not point at the index"),
            }
            f.index_offset.min(file_len)
        }
    };

    // T, K, D, R, P, L, W: walk every chunk from the header to the index.
    let mut seen: Vec<Seen> = Vec::new();
    let mut wlog_chunks = 0u32;
    let mut other_layers = [false; 6];
    let mut at = header_len as u64;
    let interval = header.keyframe_interval.max(1) as u64;
    let whole: Vec<StateRegion> = header.regions.clone();
    while at < walk_end {
        let Some(head) = read_at(&mut *file, at, 8.min((walk_end - at) as usize)) else {
            break;
        };
        if head.len() < 8 {
            out.error(
                "T2",
                None,
                format!("{} stray bytes before the index", walk_end - at),
            );
            break;
        }
        let magic: [u8; 4] = head[0..4].try_into().unwrap();
        if &magic == INDEX_MAGIC {
            // The frames end at the index, footer or not.
            break;
        }
        let body_len = u32_at(&head, 4) as u64;
        if at + 8 + body_len > walk_end {
            if footer.is_none() && options.recover {
                out.warn(
                    "T2",
                    None,
                    format!("the last chunk at {at:#x} was cut short"),
                );
            } else {
                out.error(
                    "T2",
                    None,
                    format!("the chunk at {at:#x} runs past the end of the frames"),
                );
            }
            break;
        }
        let chunk = read_at(&mut *file, at, (8 + body_len) as usize).unwrap_or_default();
        if &magic == FRAME_MAGIC {
            let n = seen.len() as u64;
            check_frame(&mut out, &header, &chunk, n, interval, &whole);
            let frame = if chunk.len() >= 16 {
                u64_at(&chunk, 8)
            } else {
                n
            };
            seen.push(Seen {
                frame,
                offset: at,
                len: chunk.len() as u32,
                kind: chunk.get(16).copied().unwrap_or(0xFF),
            });
        } else if let Some(i) = LAYER_MAGICS.iter().position(|m| **m == magic) {
            if i == 1 {
                wlog_chunks += 1;
                check_wlog(&mut out, &chunk, seen.last().map(|s| s.frame));
            } else {
                if i == 4 {
                    check_lines(&mut out, &chunk, seen.last().map(|s| s.frame));
                }
                if i == 5
                    && chunk.len() >= 8
                    && let Err(e) = crate::recording::apu::ApuEvents::decode(&chunk[8..])
                {
                    out.error("A1", seen.last().map(|s| s.frame), e.to_string());
                }
                other_layers[i] = true;
            }
        } else {
            out.error(
                "T1",
                None,
                format!(
                    "an unknown chunk {:?} at {at:#x}",
                    String::from_utf8_lossy(&magic)
                ),
            );
            break;
        }
        at += 8 + body_len;
    }

    // L: the header's layer bits against what is there.
    let declared = header.layers;
    let present = Layers {
        framebuffer: other_layers[0],
        write_log: wlog_chunks > 0,
        trace: other_layers[2],
        read_log: other_layers[3],
        line_writes: other_layers[4],
        apu_events: other_layers[5],
    };
    for (name, d, p) in [
        ("framebuffer", declared.framebuffer, present.framebuffer),
        ("write log", declared.write_log, present.write_log),
        ("trace", declared.trace, present.trace),
        ("read log", declared.read_log, present.read_log),
        ("line write", declared.line_writes, present.line_writes),
        ("APU event", declared.apu_events, present.apu_events),
    ] {
        if p && !d {
            out.error(
                "L1",
                None,
                format!("there are {name} chunks the header does not declare"),
            );
        }
        if d && !p {
            out.warn(
                "L1",
                None,
                format!("the header declares a {name} layer and there are no chunks for it"),
            );
        }
    }

    // I: the index against the chunks the walk found.
    if let Some(entries) = &index {
        for (i, e) in entries.iter().enumerate() {
            if e.frame != i as u64 {
                out.error(
                    "I1",
                    Some(i as u64),
                    format!("index entry {i} names frame {}", e.frame),
                );
            }
            match seen.iter().find(|s| s.offset == e.offset) {
                None => out.error(
                    "I2",
                    Some(e.frame),
                    format!(
                        "frame {}'s index entry points at {:#x}, where no frame chunk starts",
                        e.frame, e.offset
                    ),
                ),
                Some(s) => {
                    if s.len != e.len {
                        out.error(
                            "I3",
                            Some(e.frame),
                            format!("the index says {} bytes; the chunk is {}", e.len, s.len),
                        );
                    }
                    if s.kind != e.kind {
                        out.error(
                            "I4",
                            Some(e.frame),
                            "the index and the chunk disagree on keyframe or delta",
                        );
                    }
                }
            }
        }
        if seen.len() > entries.len() {
            out.error(
                "I5",
                None,
                format!(
                    "{} frame chunks are not in the index",
                    seen.len() - entries.len()
                ),
            );
        }
    }
    out.report.frames = seen.len() as u64;

    // S: rebuild sampled frames and check `changes` covers the truth.
    if options.sample > 0 && out.report.errors() == 0 && seen.len() > 1 {
        sample(
            &mut out,
            file,
            footer.is_none(),
            seen.len() as u64,
            options.sample,
        );
    }
    out.finish()
}

fn check_frame(
    out: &mut Out,
    header: &Header,
    chunk: &[u8],
    n: u64,
    interval: u64,
    regions: &[StateRegion],
) {
    let f = Some(n);
    let head = match FrameHead::decode(chunk) {
        Ok(h) => h,
        Err(e) => {
            out.error(
                "D1",
                f,
                format!("frame {n}'s directory cannot be read: {e}"),
            );
            return;
        }
    };
    // K: numbering and keyframes.
    if head.frame != n {
        out.error(
            "K3",
            f,
            format!("frame chunk {n} is numbered {}", head.frame),
        );
    }
    if head.kind != KIND_KEY && head.kind != KIND_DELTA {
        out.error(
            "K4",
            f,
            format!(
                "frame {n} has kind {}, neither keyframe nor delta",
                head.kind
            ),
        );
    }
    let key = head.kind == KIND_KEY;
    if n == 0 && !key {
        out.error("K1", f, "the first frame is not a keyframe");
    }
    if n > 0 && key != n.is_multiple_of(interval) {
        out.warn(
            "K2",
            f,
            format!(
                "frame {n} is a {} but the header's interval is {interval}",
                if key { "keyframe" } else { "delta" }
            ),
        );
    }
    // D: the directory.
    let mut last: Option<StateRegion> = None;
    for d in &head.dir {
        if !regions.contains(&d.region) {
            out.error(
                "D2",
                f,
                format!(
                    "frame {n} has {}, which the header's region table does not list",
                    d.region.name()
                ),
            );
        }
        if last.is_some_and(|l| l >= d.region) {
            out.error(
                "D3",
                f,
                format!(
                    "frame {n}'s directory is out of order or lists {} twice",
                    d.region.name()
                ),
            );
        }
        last = Some(d.region);
    }
    for r in regions {
        let present = head.dir.iter().any(|d| d.region == *r);
        if !present && (key || r.always_whole()) {
            out.error(
                "D4",
                f,
                format!(
                    "frame {n} leaves out {}, which a {} must carry",
                    r.name(),
                    if key { "keyframe" } else { "frame" }
                ),
            );
        }
        if present && *r == StateRegion::Wram && !key && header.flags & FLAG_WRAM_KEYFRAME_ONLY != 0
        {
            out.error(
                "W1",
                f,
                format!("delta frame {n} carries WRAM, but the header keeps it in keyframes only"),
            );
        }
    }
    // R and P: runs and payloads.
    let mut at = head.payload_at;
    for d in &head.dir {
        let size = d.region.size() as u32;
        for (what, runs) in [("data", &d.runs), ("change", &d.changes)] {
            let mut prev_end = 0u32;
            for (i, r) in runs.iter().enumerate() {
                if r.len == 0 || r.offset.checked_add(r.len).is_none_or(|e| e > size) {
                    out.error("R1", f, format!("frame {n}: a {what} run of {} ({:#x}+{:#x}) falls outside its {size} bytes", d.region.name(), r.offset, r.len));
                } else if i > 0 && r.offset < prev_end {
                    out.error(
                        "R2",
                        f,
                        format!(
                            "frame {n}: {}'s {what} runs overlap or are out of order",
                            d.region.name()
                        ),
                    );
                }
                prev_end = r.offset.saturating_add(r.len);
            }
        }
        let (whole_region, stored_whole) = (
            key || d.region.always_whole(),
            d.runs
                == [Run {
                    offset: 0,
                    len: size,
                }],
        );
        if whole_region && !stored_whole {
            out.error(
                "R3",
                f,
                format!("frame {n} must store {} whole, as one run", d.region.name()),
            );
        }
        if !whole_region && d.runs != d.changes {
            out.warn(
                "R4",
                f,
                format!(
                    "delta frame {n}: {}'s data runs are not its change runs",
                    d.region.name()
                ),
            );
        }
        let total: u64 = d.runs.iter().map(|r| r.len as u64).sum();
        if total != d.raw_len as u64 {
            out.error(
                "R5",
                f,
                format!(
                    "frame {n}: {}'s runs cover {total} bytes; the directory says {}",
                    d.region.name(),
                    d.raw_len
                ),
            );
        }
        let end = at + d.stored_len as usize;
        if end > chunk.len() {
            out.error(
                "P1",
                f,
                format!(
                    "frame {n}: {}'s payload runs past the chunk",
                    d.region.name()
                ),
            );
            return;
        }
        if let Err(e) = unpack(&chunk[at..end], d.raw_len as usize, header.compression) {
            out.error("P2", f, format!("frame {n}: {}: {e}", d.region.name()));
        }
        at = end;
    }
    if at != chunk.len() {
        out.error(
            "P3",
            f,
            format!(
                "frame {n} has {} bytes after its last payload",
                chunk.len() - at
            ),
        );
    }
}

fn check_wlog(out: &mut Out, chunk: &[u8], after: Option<u64>) {
    /// Kind 1 (a DMA start) and its record length, from docs/13.
    const DMA: u8 = 1;
    const RECORD: usize = 104;
    let body = &chunk[8..];
    if body.len() < 12 {
        out.error(
            "L2",
            after,
            "a write log chunk is shorter than its 12-byte head",
        );
        return;
    }
    let frame = u64_at(body, 0);
    let count = u32_at(body, 8) as usize;
    if Some(frame) != after {
        out.error(
            "L2",
            after,
            format!(
                "a write log chunk names frame {frame} but follows frame {}",
                after.map_or("none".into(), |f| f.to_string())
            ),
        );
    }
    if body.len() != 12 + count * RECORD {
        out.error(
            "L2",
            Some(frame),
            format!(
                "a write log chunk holds {} bytes for {count} records of {RECORD}",
                body.len() - 12
            ),
        );
        return;
    }
    for r in body[12..].chunks(RECORD) {
        if r[0] != DMA && r[0] != crate::recording::wlog::KIND_DMA_CONTEXT {
            out.warn(
                "L3",
                Some(frame),
                format!(
                    "a write log record of kind {} this Romlens does not know",
                    r[0]
                ),
            );
        }
    }
}

/// A `LINE` chunk: it names the frame it follows and decodes.
fn check_lines(out: &mut Out, chunk: &[u8], after: Option<u64>) {
    match crate::recording::lines::decode(&chunk[8..]) {
        Ok((frame, _)) if Some(frame) != after => out.error(
            "L2",
            after,
            format!(
                "a line-write chunk names frame {frame} but follows frame {}",
                after.map_or("none".into(), |f| f.to_string())
            ),
        ),
        Ok(_) => {}
        Err(e) => out.error("L2", after, e),
    }
}

fn sample(out: &mut Out, file: Box<dyn ReadSeek>, recover: bool, frames: u64, want: u32) {
    let source = match RomrecSource::from_boxed(file, recover) {
        Ok(s) => s,
        Err(e) => {
            out.error("S1", None, format!("the recording does not open: {e}"));
            return;
        }
    };
    let n = (want as u64).min(frames - 1).max(1);
    for i in 1..=n {
        let to = i * (frames - 1) / n;
        let from = to - 1;
        let (a, b) = match (source.state_at(from), source.state_at(to)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                out.error("S1", Some(to), format!("frame {to} does not rebuild: {e}"));
                continue;
            }
        };
        for r in source.regions() {
            let (Some(x), Some(y)) = (a.region(r), b.region(r)) else {
                continue;
            };
            let Ok(claimed) = source.changes(from, to, r) else {
                out.error(
                    "S1",
                    Some(to),
                    format!("frame {to}'s {} changes do not read", r.name()),
                );
                continue;
            };
            let mut inside = vec![false; x.len()];
            for c in &claimed {
                for b in inside
                    .iter_mut()
                    .skip(c.offset as usize)
                    .take(c.len as usize)
                {
                    *b = true;
                }
            }
            if let Some(at) = (0..x.len().min(y.len())).find(|&i| x[i] != y[i] && !inside[i]) {
                out.error("S2", Some(to), format!("{} changed at {at:#x} between frames {from} and {to}, outside every change run", r.name()));
            }
        }
        out.report.sampled += 1;
    }
}
