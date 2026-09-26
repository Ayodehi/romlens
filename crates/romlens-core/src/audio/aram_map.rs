//! What each part of audio RAM holds, from what the hardware says: the
//! DSP's `DIR` points at the sample directory, whose entries point at the
//! samples; `ESA` and `EDL` place the echo buffer; the SPC700's program
//! counter leads to the driver's code. The rest is the driver's own data:
//! songs, instruments, tables, variables.

use crate::dsp::brr::decode_sample;
use crate::model::spc_log::{SpcAccess, SpcLog};
use crate::recording::SpcState;
use crate::spc700::aram;
use crate::spc700::decode_at;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PartKind {
    /// `$0000–$00EF`: the direct page, the driver's variables.
    DirectPage,
    /// `$00F0–$00FF`.
    Io,
    /// `$0100–$01FF`.
    Stack,
    /// Reached from the program counter.
    Code,
    /// The sample directory.
    Directory,
    /// A BRR sample, by directory entry.
    Sample(u8),
    /// The echo's ring buffer.
    Echo,
    /// Bytes the DSP read by itself outside any sample the directory
    /// names (an execution log saw it).
    DspData,
    /// Bytes the driver's code read or wrote (an execution log saw it):
    /// its songs, tables and variables.
    DriverData,
    /// `$FFC0–$FFFF` while the boot ROM is mapped over it.
    Boot,
    /// The driver's data, or nothing.
    Other,
}

impl PartKind {
    pub fn name(self) -> &'static str {
        match self {
            PartKind::DirectPage => "direct page",
            PartKind::Io => "I/O registers",
            PartKind::Stack => "stack",
            PartKind::Code => "driver code",
            PartKind::Directory => "sample directory",
            PartKind::Sample(_) => "sample",
            PartKind::Echo => "echo buffer",
            PartKind::DspData => "data the DSP read",
            PartKind::DriverData => "driver data",
            PartKind::Boot => "boot ROM",
            PartKind::Other => "data",
        }
    }

    /// Which wins where two claim a byte.
    fn rank(self) -> u8 {
        match self {
            PartKind::Io => 9,
            PartKind::Boot => 8,
            PartKind::Echo => 7,
            PartKind::Directory => 6,
            PartKind::Sample(_) => 5,
            PartKind::Code => 4,
            PartKind::DspData => 3,
            PartKind::Stack => 3,
            PartKind::DriverData => 2,
            PartKind::DirectPage => 2,
            PartKind::Other => 0,
        }
    }
}

/// A run of audio RAM with one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AramPart {
    pub start: u16,
    pub len: u32,
    pub kind: PartKind,
    pub label: String,
}

/// A directory entry: where the sample starts and where it loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirEntry {
    pub index: u8,
    pub start: u16,
    pub loop_at: u16,
    /// Blocks to the end flag.
    pub blocks: u32,
    /// The end block loops back rather than stopping.
    pub loops: bool,
}

/// The entries of the directory at `dir << 8` that point at real samples:
/// those `used` names, and the entries from 0 on until one does not start
/// a sample that ends within 4,096 blocks.
pub fn directory(aram: &[u8], dir: u8, used: &[u8]) -> Vec<DirEntry> {
    let base = (dir as usize) << 8;
    let entry = |n: u8| -> Option<DirEntry> {
        let at = base + n as usize * 4;
        let w =
            |i: usize| u16::from_le_bytes([aram[(at + i) & 0xFFFF], aram[(at + i + 1) & 0xFFFF]]);
        let (start, loop_at) = (w(0), w(2));
        if !(0x0200..aram::IPL_START).contains(&start) {
            return None;
        }
        let s = decode_sample(aram, start as u32, Some(loop_at as u32), 4096);
        if s.unterminated || s.blocks.iter().any(|b| b.header.shift > 12) {
            return None;
        }
        Some(DirEntry {
            index: n,
            start,
            loop_at,
            blocks: s.blocks.len() as u32,
            loops: s.loops,
        })
    };
    let mut out: Vec<DirEntry> = Vec::new();
    for n in 0..=255u8 {
        match entry(n) {
            Some(e) => out.push(e),
            None => break,
        }
    }
    for &n in used {
        if !out.iter().any(|e| e.index == n)
            && let Some(e) = entry(n)
        {
            out.push(e);
        }
    }
    out.sort_by_key(|e| e.index);
    out
}

/// The longest run of untouched bytes between two of the driver's data
/// that is taken to be part of the same table.
const DATA_GAP: usize = 16;

/// Audio RAM in parts. `used` are the directory entries the voices name;
/// `entries` are places known to be code, beside the program counter. With
/// the SPC700's execution log, every instruction it ran is code, what the
/// code read or wrote is the driver's data, what the DSP read is sample
/// data, and each sample says whether it was heard.
pub fn aram_map(
    aram: &[u8],
    dsp: &[u8],
    spc: &SpcState,
    used: &[u8],
    entries: &[u16],
    log: Option<&SpcLog>,
) -> Vec<AramPart> {
    let mut owner = vec![PartKind::Other; 0x10000];
    let mut claim = |start: usize, len: usize, kind: PartKind| {
        for o in owner
            .iter_mut()
            .take((start + len).min(0x10000))
            .skip(start)
        {
            if kind.rank() > o.rank() {
                *o = kind;
            }
        }
    };
    claim(0x00, 0xF0, PartKind::DirectPage);
    claim(0xF0, 0x10, PartKind::Io);
    claim(0x100, 0x100, PartKind::Stack);
    if spc.rom_enabled {
        claim(0xFFC0, 0x40, PartKind::Boot);
    }
    let mut starts = vec![spc.pc];
    starts.extend_from_slice(entries);
    if let Some(log) = log {
        starts.extend(log.code_starts());
    }
    let walk = aram::walk(aram, &starts);
    let dsp_read = log
        .map(|l| l.touched(SpcAccess::DspRead))
        .unwrap_or_default();
    match log {
        None => {
            for &a in &walk.code {
                claim(a as usize, 1, PartKind::Code);
            }
        }
        Some(log) => {
            // What ran is code.
            for pc in log.code_starts() {
                let len = decode_at(aram, pc).len() as usize;
                claim(pc as usize, len, PartKind::Code);
            }
            for a in log.touched_by_driver() {
                claim(a as usize, 1, PartKind::DriverData);
            }
            // Below $0200 the DSP reads only through a directory entry not
            // yet set up (a sample at $0000), which says nothing of the page.
            for &a in dsp_read.range(0x0200..) {
                claim(a as usize, 1, PartKind::DspData);
            }
        }
    }
    let dir = dsp.get(0x5D).copied().unwrap_or(0);
    let d = directory(aram, dir, used);
    let entries_len = d.iter().map(|e| e.index as usize + 1).max().unwrap_or(0) * 4;
    claim((dir as usize) << 8, entries_len, PartKind::Directory);
    for e in &d {
        claim(
            e.start as usize,
            e.blocks as usize * 9,
            PartKind::Sample(e.index),
        );
    }
    let edl = dsp.get(0x7D).copied().unwrap_or(0) & 0xF;
    let esa = dsp.get(0x6D).copied().unwrap_or(0) as usize;
    let writes_on = dsp.get(0x6C).copied().unwrap_or(0) & 0x20 == 0;
    if edl > 0 || writes_on {
        let len = if edl == 0 { 4 } else { edl as usize * 2048 };
        claim(esa << 8, len, PartKind::Echo);
    }
    if log.is_some() {
        // The walk goes on past branches the log never saw taken, and from
        // there can wander into data, so the code it finds beyond what ran
        // only fills bytes nothing else claims.
        for &a in &walk.code {
            if owner[a as usize] == PartKind::Other {
                owner[a as usize] = PartKind::Code;
            }
        }
        // A table the driver read only some entries of: short gaps between
        // its data are its data too.
        let mut i = 0usize;
        while i < 0x10000 {
            if owner[i] != PartKind::Other {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < 0x10000 && owner[j] == PartKind::Other {
                j += 1;
            }
            let data = |k: usize| owner.get(k) == Some(&PartKind::DriverData);
            if j - i <= DATA_GAP && i > 0 && data(i - 1) && data(j) {
                owner[i..j].fill(PartKind::DriverData);
            }
            i = j;
        }
    }

    let mut out: Vec<AramPart> = Vec::new();
    let mut i = 0usize;
    while i < 0x10000 {
        let kind = owner[i];
        let mut j = i + 1;
        while j < 0x10000 && owner[j] == kind {
            j += 1;
        }
        let label = match kind {
            PartKind::Sample(n) => {
                let e = d.iter().find(|e| e.index == n).unwrap();
                let lp = if e.loops {
                    format!(", loops at ${:04X}", e.loop_at)
                } else {
                    String::new()
                };
                let heard = match log {
                    Some(_) if dsp_read.contains(&e.start) => ", played",
                    Some(_) => ", not played while logged",
                    None => "",
                };
                format!("sample {n}: {} blocks{lp}{heard}", e.blocks)
            }
            PartKind::Directory => format!(
                "sample directory, {} entr{} (DIR = ${dir:02X})",
                d.len(),
                if d.len() == 1 { "y" } else { "ies" }
            ),
            PartKind::Echo => format!(
                "echo buffer, {} (ESA = ${esa:02X}, EDL = {edl}){}",
                if edl == 0 {
                    "4 bytes and no delay".to_owned()
                } else {
                    format!("{} ms", edl as u32 * 16)
                },
                if writes_on { "" } else { ", writes off" }
            ),
            PartKind::Code if log.is_some() => "driver code".to_owned(),
            PartKind::Code => format!("driver code reached from ${:04X}", spc.pc),
            PartKind::Boot => "boot ROM (mapped over RAM)".to_owned(),
            PartKind::Other if log.is_some() => {
                "data the driver did not touch while logged".to_owned()
            }
            k => k.name().to_owned(),
        };
        out.push(AramPart {
            start: i as u16,
            len: (j - i) as u32,
            kind,
            label,
        });
        i = j;
    }
    out
}
