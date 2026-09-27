//! The sound side (docs/23, A10): a recording's voices, DSP registers,
//! audio RAM, samples, notes and ports; the upload traced in the ROM; BRR
//! decoded step by step; and [`ApuPlayer`], Romlens's SPC700 and DSP run
//! for the app to hear. The app plays what a player makes and never writes
//! it out (`12-content-policy.md` rule 11).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use romlens_core::apu::player::{Player, Start};
use romlens_core::audio::upload::{self, UploadReport};
use romlens_core::audio::{
    self as core_audio, AramPart, NoteKind, PartKind, aram_map, directory, note_name,
    port_messages, sample_tuning, timeline,
};
use romlens_core::dsp::brr::{self, decode_sample};
use romlens_core::explain::sound::{describe_dsp, dsp_layout, dsp_register_name};
use romlens_core::model::spc_log::{SpcAccess, SpcLog};
use romlens_core::recording::{SpcState, StateRegion};

use crate::graphics::RecordingSession;
use crate::workbench::Workbench;
use crate::{Rom, RomlensError};

fn recording_error(msg: impl Into<String>) -> RomlensError {
    RomlensError::Recording { msg: msg.into() }
}

// ---------------------------------------------------------------- records

/// One voice as the DSP's registers set it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct VoiceInfo {
    pub index: u8,
    pub volume_left: i8,
    pub volume_right: i8,
    /// 14 bits: `$1000` plays the sample at 32 kHz.
    pub pitch: u16,
    /// The rate and the semitones from `$1000`, in words.
    pub pitch_words: String,
    /// The sample directory entry.
    pub source: u8,
    /// The directory's start and loop for it.
    pub sample_start: Option<u16>,
    pub sample_loop: Option<u16>,
    /// The note it plays, estimated from its sample's loop.
    pub note: Option<String>,
    pub frequency: Option<f64>,
    /// ADSR, else GAIN.
    pub adsr: bool,
    pub adsr1: u8,
    pub adsr2: u8,
    pub gain: u8,
    /// Its envelope's settings in words.
    pub envelope_words: String,
    /// The envelope now, 0–127.
    pub envx: u8,
    /// The sample now, after the envelope: its high byte.
    pub outx: i8,
    pub echo: bool,
    pub noise: bool,
    /// Its pitch follows the voice before's wave.
    pub modulated: bool,
    /// Its sample reached an end block.
    pub ended: bool,
    pub keyed_off: bool,
    pub sounding: bool,
}

/// One of the DSP's 128 registers, explained.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DspRegisterInfo {
    pub register: u8,
    /// `V0PITCHL`, `KON`.
    pub name: String,
    pub value: u8,
    /// Voice 0–7, or none for a global register.
    pub voice: Option<u8>,
    /// Not used by the hardware.
    pub unused: bool,
    /// `KON = $01: voice 0`.
    pub short: String,
    pub parts: Vec<crate::explain::RegisterPartInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AramKindInfo {
    DirectPage,
    Io,
    Stack,
    Code,
    Directory,
    Sample,
    Echo,
    DspData,
    DriverData,
    Boot,
    Other,
}

impl From<PartKind> for AramKindInfo {
    fn from(k: PartKind) -> Self {
        match k {
            PartKind::DirectPage => AramKindInfo::DirectPage,
            PartKind::Io => AramKindInfo::Io,
            PartKind::Stack => AramKindInfo::Stack,
            PartKind::Code => AramKindInfo::Code,
            PartKind::Directory => AramKindInfo::Directory,
            PartKind::Sample(_) => AramKindInfo::Sample,
            PartKind::Echo => AramKindInfo::Echo,
            PartKind::DspData => AramKindInfo::DspData,
            PartKind::DriverData => AramKindInfo::DriverData,
            PartKind::Boot => AramKindInfo::Boot,
            PartKind::Other => AramKindInfo::Other,
        }
    }
}

/// A run of audio RAM holding one kind of thing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AramRegionInfo {
    pub start: u16,
    pub len: u32,
    pub kind: AramKindInfo,
    /// The directory entry, for a sample.
    pub sample: Option<u8>,
    pub kind_name: String,
    pub label: String,
    /// Where its first byte came from in the ROM, when the upload is known.
    pub rom_offset: Option<u32>,
}

/// A sample directory entry.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SampleInfo {
    pub index: u8,
    pub start: u16,
    pub loop_at: u16,
    pub blocks: u32,
    pub loops: bool,
    /// Whether the DSP read it while an execution log was kept; none
    /// without a log.
    pub played: Option<bool>,
    /// The frequency it makes at pitch `$1000`, estimated from its loop.
    pub tuning_hz: Option<f64>,
    pub tuning_note: Option<String>,
    /// Where it came from in the ROM, when the upload is known.
    pub rom_offset: Option<u32>,
}

/// One value of a BRR block, decoded step by step.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BrrStepInfo {
    /// The 4-bit value, signed.
    pub nibble: i8,
    /// After the shift.
    pub shifted: i32,
    /// The two results before.
    pub p1: i32,
    pub p2: i32,
    /// The filter's prediction from them.
    pub prediction: i32,
    /// Clamped to 16 bits.
    pub clamped: i32,
    /// Wrapped to 15 bits: the value the DSP keeps.
    pub result: i32,
    /// Doubled: what the voice plays.
    pub output: i16,
    pub wrapped: bool,
    pub clipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BrrBlockInfo {
    /// In the memory decoded: a file offset or an audio RAM address.
    pub offset: u32,
    pub header: u8,
    pub shift: u8,
    pub filter: u8,
    pub loops: bool,
    pub end: bool,
    /// `s + p1 × 15/16`.
    pub filter_formula: String,
    pub filter_meaning: String,
    pub steps: Vec<BrrStepInfo>,
}

/// A sample decoded from its start to its end block.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BrrSampleInfo {
    pub start: u32,
    pub blocks: Vec<BrrBlockInfo>,
    /// The block the loop point is on.
    pub loop_block: Option<u32>,
    pub loops: bool,
    /// The walk stopped without an end block.
    pub unterminated: bool,
    /// Every value, as the voice plays them.
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NoteKindInfo {
    On,
    Pitch,
    Off,
}

impl From<NoteKind> for NoteKindInfo {
    fn from(k: NoteKind) -> Self {
        match k {
            NoteKind::On => NoteKindInfo::On,
            NoteKind::Pitch => NoteKindInfo::Pitch,
            NoteKind::Off => NoteKindInfo::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteEventInfo {
    pub voice: u8,
    pub frame: u64,
    pub spc_cycle: u64,
    pub kind: NoteKindInfo,
    pub pitch: u16,
    pub source: u8,
    /// The SPC700 instruction whose DSP write made it, when known: a
    /// player logs it as it runs; for a recording ask `note_source`.
    pub spc_pc: Option<u16>,
}

fn note_info(e: &romlens_core::audio::NoteEvent, spc_pc: Option<u16>) -> NoteEventInfo {
    NoteEventInfo {
        voice: e.voice,
        frame: e.frame,
        spc_cycle: e.spc_cycle,
        kind: e.kind.into(),
        pitch: e.pitch,
        source: e.source,
        spc_pc,
    }
}

/// Where a recorded note came from: the SPC700 instruction that wrote it,
/// and what the S-CPU last asked on each port before it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteSourceInfo {
    pub spc_pc: Option<u16>,
    /// The last byte other than zero the S-CPU wrote to each port, within
    /// ten seconds before the note, newest first.
    pub commands: Vec<PortEventInfo>,
}

/// An upload a recording saw sent through the ports.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SentUploadInfo {
    /// Each block's audio RAM address and length.
    pub blocks: Vec<SentBlockInfo>,
    /// Where the SPC700 was sent when it ended; none if the frames end
    /// first.
    pub entry: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SentBlockInfo {
    pub aram: u16,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PortEventInfo {
    pub frame: u64,
    pub spc_cycle: u64,
    /// The S-CPU wrote it; else the SPC700 did.
    pub from_cpu: bool,
    pub port: u8,
    pub value: u8,
}

/// An idiom in the SPC700's code: a wait on a timer or a port.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SpcIdiomInfo {
    pub kind: String,
    pub start: u16,
    pub end: u16,
    pub title: String,
    pub summary: String,
    pub why: String,
}

/// One line of the SPC700's listing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SpcLineInfo {
    pub address: u16,
    pub bytes: Vec<u8>,
    pub text: String,
    pub label: Option<String>,
    /// Reached from the driver's entry points.
    pub code: bool,
    /// What it writes, when that is a DSP or I/O register.
    pub comment: Option<String>,
    /// The write decoded field by field.
    pub write: Vec<crate::explain::RegisterPartInfo>,
    pub write_to_dsp: bool,
    pub idiom: Option<SpcIdiomInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UploadRoutineInfo {
    /// SNES address.
    pub entry: u32,
    /// The direct page pointer it reads the block list through.
    pub pointer: u16,
    /// File offsets of the instructions that show what it is.
    pub evidence: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UploadBlockInfo {
    pub aram: u16,
    pub len: u16,
    pub rom_offset: u32,
    pub snes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UploadInfo {
    /// The block list, SNES address.
    pub list: u32,
    /// The instruction that sets the pointer, file offset, and its routine.
    pub set_at: u32,
    pub set_in: u32,
    pub blocks: Vec<UploadBlockInfo>,
    /// Where the SPC700 goes when it ends.
    pub entry: u16,
    pub bytes: u32,
    /// The upload that brings the driver; the rest go to a running one.
    pub driver: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SoundCommandInfo {
    /// File offset of the store.
    pub at: u32,
    pub routine: u32,
    pub port: u8,
    pub width: u8,
    pub value: u32,
    /// The RAM byte it is set in, which other code copies to the port.
    pub via: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UploadReportInfo {
    pub routines: Vec<UploadRoutineInfo>,
    pub uploads: Vec<UploadInfo>,
    pub commands: Vec<SoundCommandInfo>,
}

impl From<&UploadReport> for UploadReportInfo {
    fn from(r: &UploadReport) -> Self {
        let driver = upload::driver(&r.uploads).map(|d| d.list);
        UploadReportInfo {
            routines: r
                .routines
                .iter()
                .map(|u| UploadRoutineInfo {
                    entry: u.entry.as_u24(),
                    pointer: u.pointer,
                    evidence: u.evidence.iter().map(|o| o.0).collect(),
                })
                .collect(),
            uploads: r
                .uploads
                .iter()
                .map(|u| UploadInfo {
                    list: u.list.as_u24(),
                    set_at: u.set_at.0,
                    set_in: u.set_in.as_u24(),
                    blocks: u
                        .blocks
                        .iter()
                        .map(|b| UploadBlockInfo {
                            aram: b.aram,
                            len: b.len,
                            rom_offset: b.rom.0,
                            snes: b.from.as_u24(),
                        })
                        .collect(),
                    entry: u.entry,
                    bytes: u.bytes(),
                    driver: driver == Some(u.list),
                })
                .collect(),
            commands: r
                .commands
                .iter()
                .map(|c| SoundCommandInfo {
                    at: c.at.0,
                    routine: c.routine.as_u24(),
                    port: c.port,
                    width: c.width,
                    value: c.value,
                    via: c.via.map(|a| a.as_u24()),
                })
                .collect(),
        }
    }
}

/// Nintendo's N-SPC driver, recognised in audio RAM (docs/23, A14).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NspcDriverInfo {
    /// The format's version in words.
    pub dialect: String,
    /// The older version (Super Mario World, Pilotwings).
    pub old: bool,
    /// Where its table of command lengths is.
    pub lengths_at: u16,
    pub song_table: Option<u16>,
    /// Each song's list: song number n is entry n - 1.
    pub songs: Vec<u16>,
    pub playing: Option<NspcPlayingInfo>,
}

/// What the driver is playing, from its direct page.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NspcPlayingInfo {
    pub song: Option<u8>,
    pub list: u16,
    pub block: u16,
    pub entry: u16,
    /// Each voice's track in the block, 0 for none.
    pub tracks: Vec<u16>,
    /// The next byte each voice reads.
    pub positions: Vec<Option<u16>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NspcEntryKind {
    Block,
    Repeat,
    Jump,
    End,
}

/// One entry of a song's list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NspcEntryInfo {
    pub at: u16,
    pub kind: NspcEntryKind,
    /// A block's address and its eight tracks.
    pub block: u16,
    pub tracks: Vec<u16>,
    /// A repeat's count, and where a repeat or jump goes.
    pub count: u8,
    pub to: u16,
}

/// One event of a track.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NspcEventInfo {
    pub at: u16,
    pub bytes: Vec<u8>,
    /// `length`, `note`, `tie`, `rest`, `percussion`, `command`, `end` or
    /// `invalid`.
    pub kind: String,
    pub text: String,
    /// A call's target.
    pub calls: Option<u16>,
}

/// The driver found last, so the next frame's is read without a search.
static NSPC_KNOWN: Mutex<Option<romlens_core::audio::nspc::Driver>> = Mutex::new(None);

fn nspc_driver(aram: &[u8]) -> Option<romlens_core::audio::nspc::Driver> {
    use romlens_core::audio::nspc::{recognise, refresh};
    let mut known = NSPC_KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let d = match known.as_ref() {
        Some(k) => refresh(aram, k),
        None => recognise(aram),
    };
    if d.is_some() {
        *known = d.clone();
    }
    d
}

fn nspc_info(aram: &[u8]) -> Option<NspcDriverInfo> {
    use romlens_core::audio::nspc::{Dialect, playing};
    let d = nspc_driver(aram)?;
    let p = playing(aram, &d);
    Some(NspcDriverInfo {
        dialect: d.dialect.name().to_owned(),
        old: d.dialect == Dialect::Old,
        lengths_at: d.lengths_at,
        song_table: d.song_table,
        songs: d.songs.clone(),
        playing: p.map(|p| NspcPlayingInfo {
            song: p.song,
            list: p.list,
            block: p.block,
            entry: p.entry,
            tracks: p.tracks.to_vec(),
            positions: p.positions.to_vec(),
        }),
    })
}

fn nspc_song(aram: &[u8], number: u8) -> Vec<NspcEntryInfo> {
    use romlens_core::audio::nspc::{ListEntry, block_tracks, song_list};
    let Some(d) = nspc_driver(aram) else {
        return Vec::new();
    };
    let Some(&at) = d.songs.get((number as usize).wrapping_sub(1)) else {
        return Vec::new();
    };
    song_list(aram, d.dialect, at)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let base = NspcEntryInfo {
                at: e.at(),
                kind: NspcEntryKind::End,
                block: 0,
                tracks: Vec::new(),
                count: 0,
                to: 0,
            };
            match e {
                ListEntry::Block { block, .. } => NspcEntryInfo {
                    kind: NspcEntryKind::Block,
                    block,
                    tracks: block_tracks(aram, block).to_vec(),
                    ..base
                },
                ListEntry::Repeat { count, to, .. } => NspcEntryInfo {
                    kind: NspcEntryKind::Repeat,
                    count,
                    to,
                    ..base
                },
                ListEntry::Jump { to, .. } => NspcEntryInfo {
                    kind: NspcEntryKind::Jump,
                    to,
                    ..base
                },
                ListEntry::End { .. } => base,
            }
        })
        .collect()
}

fn nspc_track(aram: &[u8], at: u16) -> Vec<NspcEventInfo> {
    use romlens_core::audio::nspc::{EventKind, track};
    let Some(d) = nspc_driver(aram) else {
        return Vec::new();
    };
    track(aram, d.dialect, at)
        .events
        .iter()
        .map(|e| {
            let (kind, calls) = match &e.kind {
                EventKind::Length { .. } => ("length", None),
                EventKind::Note { .. } => ("note", None),
                EventKind::Tie => ("tie", None),
                EventKind::Rest => ("rest", None),
                EventKind::Percussion(_) => ("percussion", None),
                EventKind::Command { command, params } => (
                    "command",
                    (command.name == "call" && params.len() >= 2)
                        .then(|| u16::from_le_bytes([params[0], params[1]])),
                ),
                EventKind::End => ("end", None),
                EventKind::Invalid => ("invalid", None),
            };
            NspcEventInfo {
                at: e.at,
                bytes: e.bytes.clone(),
                kind: kind.to_owned(),
                text: e.text(),
                calls,
            }
        })
        .collect()
}

// ----------------------------------------------------- shared over a state

/// Audio RAM, the DSP's registers and the SPC700 at one moment, and where
/// each audio RAM byte came from in the ROM when an upload put it there.
struct Machine<'a> {
    aram: &'a [u8],
    dsp: &'a [u8],
    spc: SpcState,
    log: Option<&'a SpcLog>,
    origins: &'a [(u16, u16, u32)],
    entries: Vec<u16>,
}

type Tunings = HashMap<(u16, u16), Option<f64>>;

impl Machine<'_> {
    fn origin(&self, at: u16) -> Option<u32> {
        self.origins.iter().rev().find_map(|&(a, len, rom)| {
            let off = at.wrapping_sub(a);
            (off < len).then_some(rom + off as u32)
        })
    }

    fn used(&self) -> Vec<u8> {
        core_audio::voices(self.dsp, None)
            .iter()
            .map(|v| v.source)
            .collect()
    }

    fn tuning(&self, cache: &mut Tunings, start: u16, loop_at: u16) -> Option<f64> {
        *cache
            .entry((start, loop_at))
            .or_insert_with(|| sample_tuning(self.aram, start, loop_at))
    }

    fn voices(&self, cache: &mut Tunings) -> Vec<VoiceInfo> {
        core_audio::voices(self.dsp, Some(self.aram))
            .into_iter()
            .map(|v| {
                let hz = v
                    .sample
                    .filter(|_| v.pitch != 0)
                    .and_then(|(s, l)| self.tuning(cache, s, l))
                    .map(|t| t * v.pitch as f64 / 4096.0);
                VoiceInfo {
                    index: v.index,
                    volume_left: v.volume.0,
                    volume_right: v.volume.1,
                    pitch: v.pitch,
                    pitch_words: v.pitch_words(),
                    source: v.source,
                    sample_start: v.sample.map(|s| s.0),
                    sample_loop: v.sample.map(|s| s.1),
                    note: hz.map(note_name),
                    frequency: hz,
                    adsr: v.adsr1 & 0x80 != 0,
                    adsr1: v.adsr1,
                    adsr2: v.adsr2,
                    gain: v.gain,
                    envelope_words: v.envelope(),
                    envx: v.envx,
                    outx: v.outx,
                    echo: v.echo,
                    noise: v.noise,
                    modulated: v.modulated,
                    ended: v.ended,
                    keyed_off: v.keyed_off,
                    sounding: v.sounding(),
                }
            })
            .collect()
    }

    fn map(&self) -> Vec<AramRegionInfo> {
        aram_map(
            self.aram,
            self.dsp,
            &self.spc,
            &self.used(),
            &self.entries,
            self.log,
        )
        .into_iter()
        .map(|p: AramPart| AramRegionInfo {
            start: p.start,
            len: p.len,
            kind: p.kind.into(),
            sample: match p.kind {
                PartKind::Sample(n) => Some(n),
                _ => None,
            },
            kind_name: p.kind.name().to_owned(),
            label: p.label,
            rom_offset: self.origin(p.start),
        })
        .collect()
    }

    fn samples(&self, cache: &mut Tunings) -> Vec<SampleInfo> {
        let played = self.log.map(|l| l.touched(SpcAccess::DspRead));
        directory(self.aram, self.dsp[0x5D], &self.used())
            .into_iter()
            .map(|e| {
                let hz = self.tuning(cache, e.start, e.loop_at);
                SampleInfo {
                    index: e.index,
                    start: e.start,
                    loop_at: e.loop_at,
                    blocks: e.blocks,
                    loops: e.loops,
                    played: played.as_ref().map(|p| p.contains(&e.start)),
                    tuning_hz: hz,
                    tuning_note: hz.map(note_name),
                    rom_offset: self.origin(e.start),
                }
            })
            .collect()
    }

    /// Directory entry `index`'s sample, decoded.
    fn sample(&self, index: u8, max_blocks: u32) -> BrrSampleInfo {
        let at = ((self.dsp[0x5D] as usize) << 8) + index as usize * 4;
        let w = |i: usize| {
            u16::from_le_bytes([
                self.aram[(at + i) & 0xFFFF],
                self.aram[(at + i + 1) & 0xFFFF],
            ])
        };
        brr_info(self.aram, w(0) as u32, Some(w(2) as u32), max_blocks)
    }

    fn listing(&self, from: u16, count: u32) -> Vec<SpcLineInfo> {
        let mut image = self.aram.to_vec();
        if self.spc.rom_enabled {
            image[0xFFC0..].copy_from_slice(romlens_core::apu::ipl());
        }
        let mut entries = self.entries.clone();
        entries.push(self.spc.pc);
        if let Some(log) = self.log {
            entries.extend(log.code_starts());
        }
        romlens_core::explain::spc::listing(&image, &entries, from, count.min(4096) as usize)
            .into_iter()
            .map(line_info)
            .collect()
    }
}

fn line_info(l: romlens_core::explain::spc::SpcLine) -> SpcLineInfo {
    let comment = match (&l.write, l.register) {
        (Some(w), _) => Some(w.write.short()),
        (None, Some(r)) => Some(r.to_owned()),
        _ => None,
    };
    SpcLineInfo {
        address: l.address,
        bytes: l.bytes,
        text: l.text,
        label: l.label,
        code: l.code,
        comment,
        write: l
            .write
            .as_ref()
            .map(|w| crate::explain::parts(&w.write))
            .unwrap_or_default(),
        write_to_dsp: l.write.as_ref().is_some_and(|w| w.dsp),
        idiom: l.idiom.map(|i| SpcIdiomInfo {
            kind: i.kind.name().to_owned(),
            start: i.start,
            end: i.end,
            title: i.title,
            summary: i.summary,
            why: i.why.to_owned(),
        }),
    }
}

fn dsp_registers(dsp: &[u8]) -> Vec<DspRegisterInfo> {
    (0..0x80u8)
        .map(|r| {
            let value = dsp[r as usize];
            let w = describe_dsp(r, Some(value));
            let l = dsp_layout(r);
            DspRegisterInfo {
                register: r,
                name: dsp_register_name(r),
                value,
                voice: (r & 0x0F < 0x0C).then_some(r >> 4),
                unused: l.data && l.about.starts_with("Not used"),
                short: w.parts[0].short(),
                parts: crate::explain::parts(&w),
            }
        })
        .collect()
}

fn brr_info(memory: &[u8], start: u32, loop_point: Option<u32>, max_blocks: u32) -> BrrSampleInfo {
    let s = decode_sample(
        memory,
        start,
        loop_point,
        max_blocks.clamp(1, 4096) as usize,
    );
    BrrSampleInfo {
        start,
        samples: s.samples(),
        blocks: s
            .blocks
            .iter()
            .zip(&s.starts)
            .map(|(b, &offset)| block_info(b, offset))
            .collect(),
        loop_block: s.loop_block.map(|b| b as u32),
        loops: s.loops,
        unterminated: s.unterminated,
    }
}

fn block_info(b: &brr::Block, offset: u32) -> BrrBlockInfo {
    BrrBlockInfo {
        offset,
        header: b.header.byte(),
        shift: b.header.shift,
        filter: b.header.filter,
        loops: b.header.loops,
        end: b.header.end,
        filter_formula: b.header.filter_formula().to_owned(),
        filter_meaning: b.header.filter_meaning().to_owned(),
        steps: b
            .steps
            .iter()
            .map(|s| BrrStepInfo {
                nibble: s.nibble,
                shifted: s.shifted,
                p1: s.p1,
                p2: s.p2,
                prediction: s.prediction,
                clamped: s.clamped,
                result: s.result,
                output: s.output,
                wrapped: s.wrapped(),
                clipped: s.clipped(),
            })
            .collect(),
    }
}

/// The envelope a voice with these settings goes through, from the DSP
/// itself: keyed on for `hold_ms`, then off for `release_ms`, its level
/// (0–2047) every `stride` samples at 32 kHz.
#[uniffi::export]
pub fn envelope_curve(
    adsr1: u8,
    adsr2: u8,
    gain: u8,
    hold_ms: u32,
    release_ms: u32,
    stride: u32,
) -> Vec<u16> {
    let samples = |ms: u32| (ms.min(60_000) as usize) * 32;
    romlens_core::dsp::chip::envelope_curve(
        adsr1,
        adsr2,
        gain,
        samples(hold_ms),
        samples(release_ms),
        stride as usize,
    )
}

/// A frequency as a note and cents: `A4 +3`.
#[uniffi::export]
pub fn note_for_frequency(hz: f64) -> String {
    note_name(hz)
}

/// The sound test ROM (`romlens testrom --fixture sound`): an upload
/// routine of Romlens's own with a driver, directory and sample.
#[uniffi::export]
pub fn make_sound_test_rom() -> Vec<u8> {
    romlens_core::fixtures::sound::sound_upload_lorom()
}

/// The recorder stream [`make_sound_test_recording`] is packed from: what
/// Mesen's recorder would write for [`make_sound_test_rom`], for shell
/// tests of opening a stream.
#[uniffi::export]
pub fn make_sound_test_stream(frames: u32) -> Vec<u8> {
    romlens_core::recording::mesen::stream::encode::fixture_run_by_apu(
        &make_sound_test_rom(),
        frames.max(2),
    )
}

/// A player over [`romlens_core::fixtures::sound::nspc_aram`]: audio RAM
/// laid out as the N-SPC driver keeps it, song 1 playing, for shell
/// tests of the song views.
#[uniffi::export]
pub fn make_nspc_test_player() -> Arc<ApuPlayer> {
    let aram = romlens_core::fixtures::sound::nspc_aram();
    ApuPlayer::wrap(Player::image(&aram, 0x0300), Vec::new(), None)
}

/// A recording of [`make_sound_test_rom`]'s driver run by Romlens's own
/// SPC700 and DSP: in frame 1 the S-CPU sends `$01` and the driver plays
/// a note on voice 0.
#[uniffi::export]
pub fn make_sound_test_recording(frames: u32) -> Vec<u8> {
    use romlens_core::recording::mesen::pack::{PackOptions, pack};
    use romlens_core::recording::mesen::stream::encode::fixture_run_by_apu;
    let bytes = make_sound_test_rom();
    let rom = romlens_core::RomImage::from_bytes(bytes.clone(), "sound.sfc")
        .expect("the sound fixture is a ROM");
    let stream = fixture_run_by_apu(&bytes, frames.max(2));
    let mut out = std::io::Cursor::new(Vec::new());
    pack(stream.as_slice(), &rom, &mut out, PackOptions::default())
        .expect("the fixture stream packs");
    out.into_inner()
}

// ------------------------------------------------------ a recording frame

impl RecordingSession {
    fn sound_frame(&self, frame: u64) -> Result<(Vec<u8>, Vec<u8>, SpcState), RomlensError> {
        let src = self.machine();
        if !src.regions().contains(&StateRegion::Aram) {
            return Err(recording_error(
                "the recording has no sound side: record it again with this Romlens's recorder",
            ));
        }
        let s = src.state_at(frame)?;
        let get = |r: StateRegion| {
            s.region(r)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| recording_error(format!("frame {frame} has no {}", r.name())))
        };
        Ok((
            get(StateRegion::Aram)?,
            get(StateRegion::DspRegisters)?,
            SpcState::decode(&get(StateRegion::SpcState)?),
        ))
    }

    /// The SPC700's execution log beside the recording, when there is one.
    fn spc_log(&self) -> Option<SpcLog> {
        let path = Path::new(self.path()?).with_extension("spc.mxlog");
        let bytes = std::fs::read(path).ok()?;
        romlens_core::io::import::spc_log::read(&bytes, None).ok()
    }

    fn with_machine<R>(
        &self,
        frame: u64,
        log: bool,
        f: impl FnOnce(&Machine) -> R,
    ) -> Result<R, RomlensError> {
        let (aram, dsp, spc) = self.sound_frame(frame)?;
        let log = if log { self.spc_log() } else { None };
        Ok(f(&Machine {
            aram: &aram,
            dsp: &dsp,
            spc,
            log: log.as_ref(),
            origins: &[],
            entries: Vec::new(),
        }))
    }
}

#[uniffi::export]
impl RecordingSession {
    /// The recording has the sound side (docs/23).
    pub fn has_sound(&self) -> bool {
        self.machine().regions().contains(&StateRegion::Aram)
    }

    /// The eight voices at `frame`'s end.
    pub fn voices(&self, frame: u64) -> Result<Vec<VoiceInfo>, RomlensError> {
        self.with_machine(frame, false, |m| m.voices(&mut Tunings::new()))
    }

    /// The DSP's 128 registers at `frame`'s end, explained.
    pub fn dsp_registers(&self, frame: u64) -> Result<Vec<DspRegisterInfo>, RomlensError> {
        let (_, dsp, _) = self.sound_frame(frame)?;
        Ok(dsp_registers(&dsp))
    }

    /// Audio RAM at `frame`'s end in parts, with the SPC700's execution
    /// log beside the recording when there is one.
    pub fn aram_map(&self, frame: u64) -> Result<Vec<AramRegionInfo>, RomlensError> {
        self.with_machine(frame, true, |m| m.map())
    }

    /// The sample directory at `frame`'s end.
    pub fn samples(&self, frame: u64) -> Result<Vec<SampleInfo>, RomlensError> {
        self.with_machine(frame, true, |m| m.samples(&mut Tunings::new()))
    }

    /// Directory entry `index`'s sample at `frame`'s end, decoded.
    pub fn sample(
        &self,
        frame: u64,
        index: u8,
        max_blocks: u32,
    ) -> Result<BrrSampleInfo, RomlensError> {
        self.with_machine(frame, false, |m| m.sample(index, max_blocks))
    }

    /// `len` bytes of audio RAM from `start` at `frame`'s end.
    pub fn aram(&self, frame: u64, start: u16, len: u32) -> Result<Vec<u8>, RomlensError> {
        let (aram, _, _) = self.sound_frame(frame)?;
        let s = start as usize;
        Ok(aram[s..(s + len as usize).min(0x10000)].to_vec())
    }

    /// The SPC700's listing at `frame`'s end: `count` instructions from
    /// `from`, the code the driver reaches named and explained.
    pub fn spc_listing(
        &self,
        frame: u64,
        from: u16,
        count: u32,
    ) -> Result<Vec<SpcLineInfo>, RomlensError> {
        self.with_machine(frame, true, |m| m.listing(from, count))
    }

    /// Where the SPC700 was at `frame`'s end.
    pub fn spc_pc(&self, frame: u64) -> Result<u16, RomlensError> {
        Ok(self.sound_frame(frame)?.2.pc)
    }

    /// Every note keyed on, bent and keyed off in frames `from..=to`.
    pub fn note_timeline(&self, from: u64, to: u64) -> Result<Vec<NoteEventInfo>, RomlensError> {
        let to = to.min(self.machine().frame_count().unwrap_or(0).saturating_sub(1));
        Ok(timeline(self.machine(), from, to)?
            .iter()
            .map(|e| note_info(e, None))
            .collect())
    }

    /// Where the note written at `spc_cycle` in `frame` came from: the
    /// frame run again by Romlens's SPC700 to find the instruction, and the
    /// S-CPU's last requests before it.
    pub fn note_source(&self, frame: u64, spc_cycle: u64) -> Result<NoteSourceInfo, RomlensError> {
        let src = self.machine();
        // The replay's write on the note's cycle; within a cycle or two
        // where Mesen's clock and ours place it apart.
        let spc_pc = romlens_core::apu::replay::frame_writers(src, frame)?.and_then(|w| {
            w.iter()
                .filter(|(w, _)| w.register == 3)
                .min_by_key(|(w, _)| w.cycle.abs_diff(spc_cycle))
                .filter(|(w, _)| w.cycle.abs_diff(spc_cycle) <= 2)
                .map(|(_, pc)| *pc)
        });
        // Each port's last request before the note, walking back: the last
        // byte other than zero, since a driver's ports rest at zero between
        // commands (a game writes 0 each frame it asks for nothing).
        let mut commands: Vec<PortEventInfo> = Vec::new();
        let mut seen = [false; 4];
        let first = frame.saturating_sub(600);
        let mut f = frame + 1;
        while f > first && seen.iter().any(|s| !s) {
            f -= 1;
            let Some(e) = src.apu_events(f)? else {
                continue;
            };
            for m in port_messages(&e).into_iter().rev() {
                let p = m.port as usize & 3;
                if !m.from_cpu || seen[p] || m.value == 0 || m.spc_cycle > spc_cycle {
                    continue;
                }
                seen[p] = true;
                commands.push(PortEventInfo {
                    frame: m.frame,
                    spc_cycle: m.spc_cycle,
                    from_cpu: true,
                    port: m.port,
                    value: m.value,
                });
            }
        }
        commands.sort_by_key(|c| std::cmp::Reverse(c.spc_cycle));
        Ok(NoteSourceInfo { spc_pc, commands })
    }

    /// The uploads the recording saw sent through the ports in frames
    /// `from..=to`, block by block.
    pub fn sent_uploads(&self, from: u64, to: u64) -> Result<Vec<SentUploadInfo>, RomlensError> {
        let src = self.machine();
        let to = to.min(src.frame_count().unwrap_or(0).saturating_sub(1));
        let mut events = Vec::new();
        for f in from..=to {
            if let Some(e) = src.apu_events(f)? {
                events.extend(e.events);
            }
        }
        Ok(upload::from_ports(&events)
            .into_iter()
            .map(|u| SentUploadInfo {
                blocks: u
                    .blocks
                    .iter()
                    .map(|(aram, bytes)| SentBlockInfo {
                        aram: *aram,
                        len: bytes.len() as u32,
                    })
                    .collect(),
                entry: u.entry,
            })
            .collect())
    }

    /// The N-SPC driver at `frame`'s end, if audio RAM holds one.
    pub fn nspc(&self, frame: u64) -> Result<Option<NspcDriverInfo>, RomlensError> {
        Ok(nspc_info(&self.sound_frame(frame)?.0))
    }

    /// Song `number`'s list at `frame`'s end.
    pub fn nspc_song(&self, frame: u64, number: u8) -> Result<Vec<NspcEntryInfo>, RomlensError> {
        Ok(nspc_song(&self.sound_frame(frame)?.0, number))
    }

    /// The track at `at` at `frame`'s end, decoded.
    pub fn nspc_track(&self, frame: u64, at: u16) -> Result<Vec<NspcEventInfo>, RomlensError> {
        Ok(nspc_track(&self.sound_frame(frame)?.0, at))
    }

    /// The ports both ways in frames `from..=to`.
    pub fn port_events(&self, from: u64, to: u64) -> Result<Vec<PortEventInfo>, RomlensError> {
        let src = self.machine();
        let to = to.min(src.frame_count().unwrap_or(0).saturating_sub(1));
        let mut out = Vec::new();
        for f in from..=to {
            if let Some(e) = src.apu_events(f)? {
                out.extend(port_messages(&e).into_iter().map(|m| PortEventInfo {
                    frame: m.frame,
                    spc_cycle: m.spc_cycle,
                    from_cpu: m.from_cpu,
                    port: m.port,
                    value: m.value,
                }));
            }
        }
        Ok(out)
    }
}

// --------------------------------------------------------------- the ROM

#[uniffi::export]
impl Workbench {
    /// The upload to the sound CPU traced in the analysed code (docs/23,
    /// A9): the routine, each block list and the sound commands.
    pub fn sound_upload_blocking(&self) -> UploadReportInfo {
        let (rom, snap) = self.rom_and_snapshot();
        UploadReportInfo::from(&upload::trace(&rom.image, &snap))
    }

    /// The same off the calling thread.
    pub async fn sound_upload(&self) -> UploadReportInfo {
        let (rom, snap) = self.rom_and_snapshot();
        crate::future::spawn(
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            move || UploadReportInfo::from(&upload::trace(&rom.image, &snap)),
        )
        .await
    }
}

/// A BRR sample in the ROM at `offset`, decoded to its end block.
#[uniffi::export]
pub fn brr_at(
    rom: Arc<Rom>,
    offset: u32,
    loop_offset: Option<u32>,
    max_blocks: u32,
) -> BrrSampleInfo {
    brr_info(rom.image.bytes(), offset, loop_offset, max_blocks)
}

// ------------------------------------------------------------ the player

struct PlayerState {
    player: Player,
    /// `(aram, len, rom offset)` of each block an upload put there.
    origins: Vec<(u16, u16, u32)>,
    log: Option<SpcLog>,
    entries: Vec<u16>,
    tunings: Tunings,
}

impl PlayerState {
    fn machine(&self) -> Machine<'_> {
        let a = &self.player.apu;
        Machine {
            aram: &a.bus.aram,
            dsp: &a.bus.dsp,
            spc: SpcState {
                pc: a.cpu.pc,
                rom_enabled: a.bus.io.rom_enabled,
                ..SpcState::default()
            },
            log: self.log.as_ref(),
            origins: &self.origins,
            entries: self.entries.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlayerSourceInfo {
    Recording { frame: u64, follow: bool },
    Upload,
    Sample,
}

/// Romlens's SPC700 and DSP, playing. The app renders a batch at a time on
/// a background queue into a ring the audio thread reads; nothing calls
/// into Rust from the audio thread.
#[derive(uniffi::Object)]
pub struct ApuPlayer {
    state: Mutex<PlayerState>,
}

impl ApuPlayer {
    fn lock(&self) -> MutexGuard<'_, PlayerState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wrap(player: Player, origins: Vec<(u16, u16, u32)>, log: Option<SpcLog>) -> Arc<Self> {
        let entries = match player.start {
            Start::Upload => vec![player.apu.cpu.pc],
            _ => Vec::new(),
        };
        Arc::new(ApuPlayer {
            state: Mutex::new(PlayerState {
                player,
                origins,
                log,
                entries,
                tunings: Tunings::new(),
            }),
        })
    }
}

#[uniffi::export]
impl ApuPlayer {
    /// The machine as `recording` saw it at `frame`'s end. With `follow`,
    /// the S-CPU's port writes after it arrive as they did.
    #[uniffi::constructor]
    pub fn from_recording(
        recording: Arc<RecordingSession>,
        frame: u64,
        follow: bool,
    ) -> Result<Arc<Self>, RomlensError> {
        if !recording.has_sound() {
            return Err(recording_error(
                "the recording has no sound side: record it again with this Romlens's recorder",
            ));
        }
        let p = Player::from_recording(recording.machine(), frame, follow)?;
        Ok(Self::wrap(p, Vec::new(), recording.spc_log()))
    }

    /// The driver the ROM uploads, booted straight into, with the uploads
    /// whose block lists are at `with` (SNES addresses) laid over it, run
    /// `settle_seconds` before it is heard.
    #[uniffi::constructor]
    pub fn from_upload(
        workbench: Arc<Workbench>,
        with: Vec<u32>,
        settle_seconds: f64,
    ) -> Result<Arc<Self>, RomlensError> {
        let (rom, snap) = workbench.rom_and_snapshot();
        // The full trace, so every list `sound_upload` names is here.
        let uploads = upload::trace(&rom.image, &snap).uploads;
        let d = upload::driver(&uploads).ok_or_else(|| RomlensError::Project {
            msg: "no upload of a sound driver was traced in this ROM".to_owned(),
        })?;
        let mut more = Vec::new();
        for w in with {
            let u = uploads
                .iter()
                .find(|u| u.list.as_u24() == w)
                .ok_or_else(|| RomlensError::BadAddress {
                    msg: format!("${w:06X} is not a traced upload"),
                })?;
            more.push(u);
        }
        let origins = std::iter::once(d)
            .chain(more.iter().copied())
            .flat_map(|u| u.blocks.iter().map(|b| (b.aram, b.len, b.rom.0)))
            .collect();
        let p = Player::from_upload(&rom.image, d, &more, settle_seconds.clamp(0.0, 10.0));
        Ok(Self::wrap(p, origins, None))
    }

    /// A BRR sample in the ROM at `offset` alone on voice 0 at `pitch`,
    /// looping at `loop_offset` bytes in. Keyed with `key_on`.
    #[uniffi::constructor]
    pub fn from_rom_sample(rom: Arc<Rom>, offset: u32, loop_offset: u32, pitch: u16) -> Arc<Self> {
        let s = decode_sample(rom.image.bytes(), offset, None, 4096);
        let bytes = &rom.image.bytes()[offset as usize..(offset + s.len()) as usize];
        let origins = vec![(
            romlens_core::apu::player::SAMPLE_AT,
            s.len().min(0xFFFF) as u16,
            offset,
        )];
        Self::wrap(
            Player::sample(bytes, loop_offset as usize, pitch),
            origins,
            None,
        )
    }

    /// Directory entry `index`'s sample at a recording's `frame` alone on
    /// voice 0 at `pitch`.
    #[uniffi::constructor]
    pub fn from_recorded_sample(
        recording: Arc<RecordingSession>,
        frame: u64,
        index: u8,
        pitch: u16,
    ) -> Result<Arc<Self>, RomlensError> {
        let (aram, dsp, _) = recording.sound_frame(frame)?;
        let at = ((dsp[0x5D] as usize) << 8) + index as usize * 4;
        let w =
            |i: usize| u16::from_le_bytes([aram[(at + i) & 0xFFFF], aram[(at + i + 1) & 0xFFFF]]);
        let (start, loop_at) = (w(0), w(2));
        let s = decode_sample(&aram, start as u32, None, 4096);
        let bytes = &aram[start as usize..start as usize + s.len() as usize];
        Ok(Self::wrap(
            Player::sample(bytes, loop_at.wrapping_sub(start) as usize, pitch),
            Vec::new(),
            None,
        ))
    }

    pub fn source(&self) -> PlayerSourceInfo {
        match self.lock().player.start {
            Start::Recording { frame, follow } => PlayerSourceInfo::Recording { frame, follow },
            Start::Upload => PlayerSourceInfo::Upload,
            Start::Sample => PlayerSourceInfo::Sample,
        }
    }

    /// The next `samples` stereo samples at 32 kHz, left and right
    /// interleaved.
    pub fn render(&self, samples: u32) -> Vec<i16> {
        let frames = self.lock().player.render(samples.min(32_000) as usize);
        let mut out = Vec::with_capacity(frames.len() * 2);
        for f in frames {
            out.push(f.left);
            out.push(f.right);
        }
        out
    }

    /// The last `count` samples of voice 0–7, or 8 for the left output and
    /// 9 for the right, oldest first.
    pub fn scope(&self, which: u8, count: u32) -> Vec<i16> {
        self.lock().player.scope(which as usize, count as usize)
    }

    /// Seconds played.
    pub fn seconds(&self) -> f64 {
        self.lock().player.seconds()
    }

    /// The S-CPU writes `value` to port `port` (0–3) now.
    pub fn send_port(&self, port: u8, value: u8) {
        self.lock().player.send_port(port & 3, value);
    }

    /// What the S-CPU would read from port `port`.
    pub fn read_port(&self, port: u8) -> u8 {
        self.lock().player.apu.read_port(port as usize & 3)
    }

    /// Voices left out of the mix, bit n for voice n.
    pub fn set_muted(&self, mask: u8) {
        self.lock().player.set_muted(mask);
    }

    pub fn muted(&self) -> u8 {
        self.lock().player.muted()
    }

    /// Only `voice` in the mix, or every voice with none.
    pub fn solo(&self, voice: Option<u8>) {
        let mask = voice.map_or(0, |v| !(1u8 << (v & 7)));
        self.lock().player.set_muted(mask);
    }

    pub fn key_on(&self, mask: u8) {
        self.lock().player.key_on(mask);
    }

    pub fn key_off(&self, mask: u8) {
        self.lock().player.key_off(mask);
    }

    pub fn set_pitch(&self, voice: u8, pitch: u16) {
        self.lock().player.set_pitch(voice, pitch);
    }

    /// The eight voices now.
    pub fn voices(&self) -> Vec<VoiceInfo> {
        let mut s = self.lock();
        let mut cache = std::mem::take(&mut s.tunings);
        let out = s.machine().voices(&mut cache);
        s.tunings = cache;
        out
    }

    pub fn dsp_registers(&self) -> Vec<DspRegisterInfo> {
        dsp_registers(&self.lock().player.apu.bus.dsp)
    }

    pub fn aram_map(&self) -> Vec<AramRegionInfo> {
        self.lock().machine().map()
    }

    pub fn samples(&self) -> Vec<SampleInfo> {
        let mut s = self.lock();
        let mut cache = std::mem::take(&mut s.tunings);
        let out = s.machine().samples(&mut cache);
        s.tunings = cache;
        out
    }

    pub fn sample(&self, index: u8, max_blocks: u32) -> BrrSampleInfo {
        self.lock().machine().sample(index, max_blocks)
    }

    pub fn aram(&self, start: u16, len: u32) -> Vec<u8> {
        let s = start as usize;
        self.lock().player.apu.bus.aram[s..(s + len as usize).min(0x10000)].to_vec()
    }

    pub fn spc_listing(&self, from: u16, count: u32) -> Vec<SpcLineInfo> {
        self.lock().machine().listing(from, count)
    }

    pub fn spc_pc(&self) -> u16 {
        self.lock().player.apu.cpu.pc
    }

    /// The N-SPC driver now, if audio RAM holds one.
    pub fn nspc(&self) -> Option<NspcDriverInfo> {
        nspc_info(&self.lock().player.apu.bus.aram)
    }

    pub fn nspc_song(&self, number: u8) -> Vec<NspcEntryInfo> {
        nspc_song(&self.lock().player.apu.bus.aram, number)
    }

    pub fn nspc_track(&self, at: u16) -> Vec<NspcEventInfo> {
        nspc_track(&self.lock().player.apu.bus.aram, at)
    }

    /// Log the notes the driver plays from now, each with the instruction
    /// behind it.
    pub fn log_notes(&self) {
        self.lock().player.log_notes();
    }

    /// The notes logged from index `since` on.
    pub fn notes(&self, since: u32) -> Vec<NoteEventInfo> {
        let s = self.lock();
        let n = s.player.notes();
        n.iter()
            .skip(since as usize)
            .map(|(e, pc)| note_info(e, Some(*pc)))
            .collect()
    }

    /// How many notes are logged.
    pub fn note_count(&self) -> u32 {
        self.lock().player.notes().len() as u32
    }

    /// Where audio RAM byte `address` came from in the ROM, when an upload
    /// put it there.
    pub fn rom_origin(&self, address: u16) -> Option<u32> {
        self.lock().machine().origin(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> Arc<RecordingSession> {
        RecordingSession::from_bytes(make_sound_test_recording(5)).unwrap()
    }

    #[test]
    fn a_recording_frame_reads_as_voices_registers_parts_and_samples() {
        let rec = recording();
        assert!(rec.has_sound());
        let v = rec.voices(3).unwrap();
        assert_eq!(v.len(), 8);
        assert!(v[0].sounding, "the driver keyed voice 0 on in frame 1");
        assert_eq!((v[0].pitch, v[0].sample_start), (0x1000, Some(0x4000)));
        assert!(v[0].adsr && v[0].envelope_words.starts_with("ADSR"));
        let regs = rec.dsp_registers(3).unwrap();
        assert_eq!(regs.len(), 128);
        assert_eq!(regs[0x4C].name, "KON");
        assert_eq!(regs[0x5D].value, 0x3C);
        assert_eq!(regs[0x02].voice, Some(0));
        let map = rec.aram_map(3).unwrap();
        for k in [
            AramKindInfo::Code,
            AramKindInfo::Directory,
            AramKindInfo::Sample,
        ] {
            assert!(map.iter().any(|p| p.kind == k), "{k:?} in {map:?}");
        }
        let samples = rec.samples(3).unwrap();
        assert_eq!((samples[0].start, samples[0].loop_at), (0x4000, 0x4012));
        let s = rec.sample(3, 0, 64).unwrap();
        assert_eq!(s.blocks.len(), 4);
        assert_eq!(s.loop_block, Some(2));
        assert_eq!(s.samples.len(), 64);
        assert_eq!(s.blocks[1].filter, 1);
        let lines = rec.spc_listing(3, 0x0200, 40).unwrap();
        assert_eq!(lines[0].text, "MOV X,#$EF");
        // Walked from where the SPC700 is: its main loop, not the start.
        let pc = rec.spc_pc(3).unwrap();
        assert!(lines.iter().any(|l| l.address == pc && l.code));
        assert!(!lines[0].code);
        // The start is not walked, so its DIR write is only named.
        assert!(lines[2].comment.is_some(), "{:?}", lines[2]);
        assert!(lines.iter().any(|l| l.idiom.is_some()), "the port wait");
        let notes = rec.note_timeline(0, 4).unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.voice == 0 && n.kind == NoteKindInfo::On)
        );
        let ports = rec.port_events(1, 1).unwrap();
        assert!(ports.iter().any(|p| p.from_cpu && p.value == 1));
        assert!(ports.iter().any(|p| !p.from_cpu && p.value == 1));
        assert_eq!(rec.aram(3, 0x3C00, 4).unwrap(), [0x00, 0x40, 0x12, 0x40]);
    }

    #[test]
    fn a_player_from_a_frame_plays_and_keeps_scopes() {
        let rec = recording();
        let p = ApuPlayer::from_recording(rec, 0, true).unwrap();
        let out = p.render(3200);
        assert_eq!(out.len(), 6400);
        assert!(
            out.iter().any(|v| *v != 0),
            "the note from frame 1's command"
        );
        assert_eq!(p.scope(0, 256).len(), 256);
        assert!(p.scope(0, 256).iter().any(|v| *v != 0));
        assert!((p.seconds() - 0.1).abs() < 1e-9);
        // Muted, the mix is silent and the voice still runs.
        p.solo(Some(3));
        let out = p.render(320);
        assert!(out.iter().all(|v| *v == 0));
        assert!(p.scope(0, 64).iter().any(|v| *v != 0));
        assert!(p.voices()[0].sounding);
    }

    #[test]
    fn the_rom_upload_boots_and_answers_a_command() {
        let rom = Rom::from_bytes(make_sound_test_rom(), "sound.sfc".to_owned()).unwrap();
        let wb = Workbench::new(rom.clone());
        wb.analyze_blocking().unwrap();
        let r = wb.sound_upload_blocking();
        assert_eq!(r.routines.len(), 1);
        assert_eq!(r.uploads.len(), 1);
        let u = &r.uploads[0];
        assert!(u.driver);
        assert_eq!((u.entry, u.blocks.len()), (0x0200, 3));
        let p = ApuPlayer::from_upload(wb, Vec::new(), 0.05).unwrap();
        assert_eq!(p.source(), PlayerSourceInfo::Upload);
        assert!(p.render(1600).iter().all(|v| *v == 0), "nothing asked yet");
        p.send_port(0, 1);
        assert!(p.render(3200).iter().any(|v| *v != 0));
        assert_eq!(p.read_port(0), 1, "the driver echoes the command");
        let map = p.aram_map();
        let dir = map
            .iter()
            .find(|m| m.kind == AramKindInfo::Directory)
            .unwrap();
        assert_eq!(dir.rom_offset, Some(u.blocks[1].rom_offset));
        assert_eq!(p.rom_origin(0x4000), Some(u.blocks[2].rom_offset));
        assert_eq!(p.samples()[0].rom_offset, Some(u.blocks[2].rom_offset));
    }

    #[test]
    fn a_sample_plays_alone_when_keyed() {
        use romlens_core::fixtures::sound::{SAMPLE_LOOP, SAMPLE_OFFSET};
        let rom = Rom::from_bytes(make_sound_test_rom(), "sound.sfc".to_owned()).unwrap();
        let s = brr_at(rom.clone(), SAMPLE_OFFSET as u32, None, 16);
        assert_eq!(s.blocks.len(), 4);
        assert!(!s.unterminated && s.loops);
        assert_eq!(s.blocks[0].steps[0].nibble, -8);
        let p = ApuPlayer::from_rom_sample(rom, SAMPLE_OFFSET as u32, SAMPLE_LOOP as u32, 0x1000);
        assert!(p.render(320).iter().all(|v| *v == 0));
        p.key_on(1);
        let out = p.render(3200);
        assert!(out.iter().any(|v| *v != 0));
        p.key_off(1);
        // Release takes 8 a sample from at most $7FF: gone in 256.
        p.render(600);
        assert!(p.render(64).iter().all(|v| *v == 0));
        let rec = recording();
        let p = ApuPlayer::from_recorded_sample(rec, 3, 0, 0x0800).unwrap();
        p.key_on(1);
        assert!(p.render(3200).iter().any(|v| *v != 0));
    }

    #[test]
    fn a_note_leads_back_to_its_instruction_and_its_command() {
        use romlens_core::recording::mesen::stream::encode::FIXTURE_PLAYER;
        let program = romlens_core::spc700::assemble(FIXTURE_PLAYER).unwrap();
        // The setup loop's DSPDATA write keys the voice on.
        let setup = program.label("setup").unwrap();
        let rec = recording();
        let on = rec
            .note_timeline(0, 4)
            .unwrap()
            .into_iter()
            .find(|n| n.kind == NoteKindInfo::On)
            .unwrap();
        let src = rec.note_source(on.frame, on.spc_cycle).unwrap();
        assert_eq!(src.spc_pc, Some(setup + 9), "MOV DSPDATA,A in the loop");
        assert_eq!((src.commands[0].port, src.commands[0].value), (0, 1));
        // The same from the ROM, logged as it plays.
        let rom = Rom::from_bytes(make_sound_test_rom(), "sound.sfc".to_owned()).unwrap();
        let wb = Workbench::new(rom);
        wb.analyze_blocking().unwrap();
        let p = ApuPlayer::from_upload(wb, Vec::new(), 0.05).unwrap();
        p.log_notes();
        p.send_port(0, 1);
        p.render(3200);
        let notes = p.notes(0);
        let on = notes.iter().find(|n| n.kind == NoteKindInfo::On).unwrap();
        assert_eq!(
            (on.voice, on.pitch, on.spc_pc),
            (0, 0x1000, Some(setup + 9))
        );
        assert_eq!(p.note_count() as usize, notes.len());
    }

    #[test]
    fn a_driver_that_is_not_n_spc_has_no_songs() {
        // The fixture's driver is Romlens's own.
        let rec = recording();
        assert_eq!(rec.nspc(3).unwrap(), None);
        assert!(rec.nspc_song(3, 1).unwrap().is_empty());
        // One that is.
        let p = make_nspc_test_player();
        let d = p.nspc().unwrap();
        assert!(d.old && d.songs == [0x1400, 0x1420]);
        assert_eq!(d.playing.as_ref().unwrap().song, Some(1));
        let list = p.nspc_song(1);
        assert_eq!(list[2].kind, NspcEntryKind::Repeat);
        let t = p.nspc_track(0x2000);
        assert_eq!(t[6].calls, Some(0x2300));
        assert_eq!(t[2].text, "note C3");
    }

    #[test]
    fn a_recorder_stream_packs_into_a_recording_with_sound() {
        use romlens_core::recording::mesen::stream::encode::fixture_run_by_apu;
        let dir = std::env::temp_dir().join(format!("romlens-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = make_sound_test_rom();
        let stream = dir.join("s.rlstream");
        std::fs::write(&stream, fixture_run_by_apu(&bytes, 5)).unwrap();
        let out = dir.join("s.romrec");
        let rom = Rom::from_bytes(bytes, "sound.sfc".to_owned()).unwrap();
        let summary = crate::graphics::pack_recorder_stream(
            rom,
            stream.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        )
        .unwrap();
        assert_eq!(summary.frames, 5);
        assert!(summary.sound_events > 0 && !summary.truncated && !summary.spc_log);
        let rec = RecordingSession::open(out.to_string_lossy().into_owned(), false).unwrap();
        assert!(rec.has_sound());
        assert!(!dir.join("s.romrec.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_envelope_curve_is_the_dsps() {
        // Attack 15 adds 1024 a sample to $7FF, then decay 0 and sustain
        // level 7 hold it high; release takes 8 a sample.
        let c = envelope_curve(0x8F, 0xE0, 0, 10, 10, 1);
        assert_eq!(c.len(), 640);
        assert_eq!(&c[..3], &[0, 1024, 0x7FF]);
        assert_eq!(*c.last().unwrap(), 0);
        assert_eq!(note_for_frequency(440.0), "A4");
    }
}
