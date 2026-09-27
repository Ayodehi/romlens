//! Nintendo's N-SPC sound driver, read (docs/23, A14): its songs as lists
//! of blocks, each block eight tracks, each track notes, lengths and
//! commands. The one driver-specific part of the sound track; every other
//! view reads the hardware and works for any driver.
//!
//! Written from two public descriptions of the format, the Super
//! Famicom Development Wiki's "Nintendo Music Format (N-SPC)" (the
//! standard version, commands `$E0–$FA`) and "Super Mario World Music
//! Format" (the older version Super Mario World and Pilotwings use,
//! commands `$DA–$F2`), and checked against Super Mario World's audio RAM.
//!
//! The driver is recognised by the table of its commands' parameter
//! lengths, which it keeps in audio RAM to step over each command:
//! Super Mario World's holds each length plus one (the command byte too)
//! at `$0FC2`. The songs are found as a table of pointers to valid song
//! lists; which song is playing, and where each voice is in it, comes
//! from the driver's direct page: eight track pointers at `$30–$3F` and
//! the song list pointer at `$40`.

/// Which version of the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Super Mario World, Pilotwings: commands `$DA–$F2`.
    Old,
    /// Zelda, Super Metroid and most later games: commands `$E0–$FA`.
    Standard,
}

/// A command: its byte, name, parameter count and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub op: u8,
    pub name: &'static str,
    pub params: u8,
    pub about: &'static str,
}

const fn c(op: u8, name: &'static str, params: u8, about: &'static str) -> Command {
    Command {
        op,
        name,
        params,
        about,
    }
}

/// The older version's commands (Super Mario World Music Format).
pub const OLD_COMMANDS: [Command; 25] = [
    c(
        0xDA,
        "instrument",
        1,
        "the instrument (patch) the voice plays",
    ),
    c(
        0xDB,
        "pan",
        1,
        "where the voice sits, $00 right to $14 left",
    ),
    c(
        0xDC,
        "pan fade",
        2,
        "move the pan over a time to a position",
    ),
    c(
        0xDD,
        "pitch slide",
        3,
        "after a delay, slide over a duration to a note",
    ),
    c(
        0xDE,
        "vibrato",
        3,
        "vibrato after a delay, at a rate and depth",
    ),
    c(0xDF, "vibrato off", 0, "stop the vibrato"),
    c(0xE0, "master volume", 1, "the song's volume"),
    c(
        0xE1,
        "master volume fade",
        2,
        "fade the song's volume over a time",
    ),
    c(0xE2, "tempo", 1, "the speed the ticks go at"),
    c(0xE3, "tempo fade", 2, "change the tempo over a time"),
    c(
        0xE4,
        "transpose",
        1,
        "shift every voice's notes by semitones",
    ),
    c(
        0xE5,
        "tremolo",
        3,
        "a pulsing volume after a delay, at a rate and depth",
    ),
    c(0xE6, "tremolo off", 0, "stop the tremolo"),
    c(0xE7, "volume", 1, "the voice's volume"),
    c(
        0xE8,
        "volume fade",
        2,
        "fade the voice's volume over a time",
    ),
    c(
        0xE9,
        "call",
        3,
        "play a phrase elsewhere, a number of times, and come back",
    ),
    c(0xEA, "vibrato fade", 1, "bring the vibrato in over a time"),
    c(
        0xEB,
        "pitch envelope to",
        3,
        "each note bends away after a delay",
    ),
    c(
        0xEC,
        "pitch envelope from",
        3,
        "each note starts bent and glides back",
    ),
    c(
        0xED,
        "pitch envelope off",
        0,
        "undefined in Super Mario World",
    ),
    c(0xEE, "tuning", 1, "fine pitch, in 1/256 of a semitone"),
    c(
        0xEF,
        "echo on",
        3,
        "which voices echo (EON), and the echo's volume left and right",
    ),
    c(0xF0, "echo off", 0, "turn the echo off"),
    c(
        0xF1,
        "echo setup",
        3,
        "the echo's delay (EDL), feedback (EFB) and FIR filter",
    ),
    c(
        0xF2,
        "echo volume fade",
        3,
        "fade the echo's volume over a time",
    ),
];

/// The standard version's commands (Nintendo Music Format (N-SPC)).
pub const STANDARD_COMMANDS: [Command; 27] = [
    c(0xE0, "instrument", 1, "the instrument the voice plays"),
    c(0xE1, "pan", 1, "where the voice sits, 0 to 20"),
    c(
        0xE2,
        "pan fade",
        2,
        "move the pan over a time to a position",
    ),
    c(
        0xE3,
        "vibrato",
        3,
        "vibrato after a delay, at a rate and depth",
    ),
    c(0xE4, "vibrato off", 0, "stop the vibrato"),
    c(0xE5, "master volume", 1, "the song's volume"),
    c(
        0xE6,
        "master volume fade",
        2,
        "fade the song's volume over a time",
    ),
    c(0xE7, "tempo", 1, "the speed the ticks go at"),
    c(0xE8, "tempo fade", 2, "change the tempo over a time"),
    c(
        0xE9,
        "transpose",
        1,
        "shift every voice's notes by semitones",
    ),
    c(
        0xEA,
        "voice transpose",
        1,
        "shift this voice's notes by semitones",
    ),
    c(
        0xEB,
        "tremolo",
        3,
        "a pulsing volume after a delay, at a rate and depth",
    ),
    c(0xEC, "tremolo off", 0, "stop the tremolo"),
    c(0xED, "volume", 1, "the voice's volume"),
    c(
        0xEE,
        "volume fade",
        2,
        "fade the voice's volume over a time",
    ),
    c(
        0xEF,
        "call",
        3,
        "play a phrase elsewhere, a number of times, and come back",
    ),
    c(0xF0, "vibrato fade", 1, "bring the vibrato in over a time"),
    c(
        0xF1,
        "pitch envelope to",
        3,
        "each note bends up after a delay",
    ),
    c(
        0xF2,
        "pitch envelope from",
        3,
        "each note starts bent and glides back",
    ),
    c(0xF3, "pitch envelope off", 0, "stop the pitch envelope"),
    c(0xF4, "tuning", 1, "fine pitch, in 1/256 of a semitone"),
    c(
        0xF5,
        "echo on",
        3,
        "which voices echo (EON), and the echo's volume left and right",
    ),
    c(0xF6, "echo off", 0, "turn the echo off"),
    c(
        0xF7,
        "echo setup",
        3,
        "the echo's delay (EDL), feedback (EFB) and FIR filter",
    ),
    c(
        0xF8,
        "echo volume fade",
        3,
        "fade the echo's volume over a time",
    ),
    c(
        0xF9,
        "pitch slide",
        3,
        "after a delay, slide over a duration to a note",
    ),
    c(
        0xFA,
        "percussion base",
        1,
        "the instrument percussion notes start from",
    ),
];

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Old => "N-SPC, the older version (Super Mario World, Pilotwings)",
            Dialect::Standard => "N-SPC",
        }
    }

    pub fn commands(self) -> &'static [Command] {
        match self {
            Dialect::Old => &OLD_COMMANDS,
            Dialect::Standard => &STANDARD_COMMANDS,
        }
    }

    pub fn command(self, op: u8) -> Option<&'static Command> {
        self.commands().iter().find(|c| c.op == op)
    }

    /// Notes, tie, rest and percussion.
    fn notes(
        self,
    ) -> (
        std::ops::RangeInclusive<u8>,
        u8,
        u8,
        std::ops::RangeInclusive<u8>,
    ) {
        match self {
            // $C8–$CF are unused and act as rests.
            Dialect::Old => (0x80..=0xC5, 0xC6, 0xC7, 0xD0..=0xD9),
            Dialect::Standard => (0x80..=0xC7, 0xC8, 0xC9, 0xCA..=0xDF),
        }
    }

    /// The octave of `$80`: C1 in the standard version's description,
    /// C0 in Super Mario World's.
    fn first_octave(self) -> i32 {
        match self {
            Dialect::Old => 0,
            Dialect::Standard => 1,
        }
    }
}

/// The recognised driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driver {
    pub dialect: Dialect,
    /// Where its table of command lengths is.
    pub lengths_at: u16,
    /// The song table, when found, and each song's list address. Song
    /// number *n* (the command a game sends) is entry *n* − 1.
    pub song_table: Option<u16>,
    pub songs: Vec<u16>,
}

fn word(aram: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([aram[at & 0xFFFF], aram[(at + 1) & 0xFFFF]])
}

/// Find the driver in audio RAM by its command lengths table: stored as
/// each command's parameter count, or with the command byte counted too.
pub fn recognise(aram: &[u8]) -> Option<Driver> {
    for dialect in [Dialect::Old, Dialect::Standard] {
        let plain: Vec<u8> = dialect.commands().iter().map(|c| c.params).collect();
        let plus: Vec<u8> = plain.iter().map(|p| p + 1).collect();
        for table in [&plus, &plain] {
            if let Some(at) = aram[..0xFFC0]
                .windows(table.len())
                .position(|w| w == &table[..])
            {
                let mut d = Driver {
                    dialect,
                    lengths_at: at as u16,
                    song_table: None,
                    songs: Vec::new(),
                };
                if let Some((t, songs)) = find_song_table(aram, dialect) {
                    d.song_table = Some(t);
                    d.songs = songs;
                }
                return Some(d);
            }
        }
    }
    None
}

/// The driver found before, if it is still there: its length table where
/// it was and its song table still leading to songs, read again without
/// searching. Else a search, as [`recognise`].
pub fn refresh(aram: &[u8], known: &Driver) -> Option<Driver> {
    let plain: Vec<u8> = known.dialect.commands().iter().map(|c| c.params).collect();
    let at = known.lengths_at as usize;
    let here = &aram[at..at + plain.len()];
    let same = here == &plain[..] || here.iter().zip(&plain).all(|(a, b)| *a == b + 1);
    if let (true, Some(t)) = (same, known.song_table) {
        let songs: Vec<u16> = (0..known.songs.len().max(1) + 8)
            .map(|i| word(aram, t as usize + i * 2))
            .take_while(|w| song_list(aram, known.dialect, *w).is_some())
            .collect();
        if !songs.is_empty() {
            return Some(Driver {
                songs,
                ..known.clone()
            });
        }
    }
    recognise(aram)
}

/// In RAM, past the direct page and stack, below the boot ROM.
fn in_ram(a: u16) -> bool {
    (0x0200..0xFFC0).contains(&a)
}

/// One entry of a song list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListEntry {
    /// Play the block at `block`: eight track pointers.
    Block {
        at: u16,
        block: u16,
    },
    /// Go back to `to` `count` more times (`count` 1–$7F).
    Repeat {
        at: u16,
        count: u8,
        to: u16,
    },
    /// Go to `to` for ever ($80 and up).
    Jump {
        at: u16,
        to: u16,
    },
    End {
        at: u16,
    },
}

impl ListEntry {
    pub fn at(&self) -> u16 {
        match *self {
            ListEntry::Block { at, .. }
            | ListEntry::Repeat { at, .. }
            | ListEntry::Jump { at, .. }
            | ListEntry::End { at } => at,
        }
    }
}

/// A block's eight track pointers, 0 for a silent voice.
pub fn block_tracks(aram: &[u8], block: u16) -> [u16; 8] {
    std::array::from_fn(|i| word(aram, block as usize + i * 2))
}

/// The song list at `at`, to its end or its jump, if it reads as one:
/// every block's tracks in RAM and each track parsing to its end.
pub fn song_list(aram: &[u8], dialect: Dialect, at: u16) -> Option<Vec<ListEntry>> {
    if !in_ram(at) {
        return None;
    }
    let mut out = Vec::new();
    let mut p = at;
    for _ in 0..256 {
        let w = word(aram, p as usize);
        if w == 0 {
            out.push(ListEntry::End { at: p });
            break;
        }
        if w < 0x0100 {
            let to = word(aram, p as usize + 2);
            // A repeat or a jump goes back into the list.
            if !(at..=p).contains(&to) {
                return None;
            }
            let count = w as u8;
            if count >= 0x80 {
                out.push(ListEntry::Jump { at: p, to });
                break;
            }
            out.push(ListEntry::Repeat { at: p, count, to });
            p = p.wrapping_add(4);
            continue;
        }
        let tracks = block_tracks(aram, w);
        if !in_ram(w) || tracks.iter().all(|t| *t == 0) {
            return None;
        }
        for t in tracks.iter().filter(|t| **t != 0) {
            if !in_ram(*t) || !track(aram, dialect, *t).ends() {
                return None;
            }
        }
        out.push(ListEntry::Block { at: p, block: w });
        p = p.wrapping_add(2);
    }
    out.iter()
        .any(|e| matches!(e, ListEntry::Block { .. }))
        .then_some(out)
}

/// The longest run of pointers to song lists: the song table.
fn find_song_table(aram: &[u8], dialect: Dialect) -> Option<(u16, Vec<u16>)> {
    // Every address that starts a song list, remembered.
    let mut valid = std::collections::HashMap::new();
    let mut is_list = |a: u16| {
        *valid
            .entry(a)
            .or_insert_with(|| song_list(aram, dialect, a).is_some())
    };
    let mut best: Option<(u16, Vec<u16>)> = None;
    let mut at = 0x0200usize;
    while at + 2 < 0xFFC0 {
        let mut songs = Vec::new();
        let mut p = at;
        while p + 2 < 0xFFC0 {
            let w = word(aram, p);
            if !is_list(w) {
                break;
            }
            songs.push(w);
            p += 2;
        }
        if songs.len() > best.as_ref().map_or(1, |b| b.1.len()) {
            best = Some((at as u16, songs));
        }
        // A run found is not searched again from inside it.
        at = if p > at { p } else { at + 1 };
    }
    best
}

/// What a byte of a track is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// How long the notes after it last, in ticks (48 a quarter note),
    /// with the quantize (how much of it sounds, 0–7) and velocity (0–15)
    /// indices when a second byte gives them.
    Length {
        ticks: u8,
        quantize: Option<u8>,
        velocity: Option<u8>,
    },
    /// A note: semitones up from `$80`.
    Note {
        semitone: u8,
        name: String,
    },
    Tie,
    Rest,
    Percussion(u8),
    Command {
        command: &'static Command,
        params: Vec<u8>,
    },
    /// Not a byte this version uses.
    Invalid,
    /// The end of the track, or of a called phrase.
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackEvent {
    pub at: u16,
    pub bytes: Vec<u8>,
    pub kind: EventKind,
}

impl TrackEvent {
    /// The event in words.
    pub fn text(&self) -> String {
        match &self.kind {
            EventKind::Length {
                ticks,
                quantize,
                velocity,
            } => {
                let mut s = format!("length {ticks} ticks{}", beat_words(*ticks));
                if let (Some(q), Some(v)) = (quantize, velocity) {
                    s += &format!(", quantize {q}, velocity {v}");
                }
                s
            }
            EventKind::Note { name, .. } => format!("note {name}"),
            EventKind::Tie => "tie: the note before goes on".to_owned(),
            EventKind::Rest => "rest".to_owned(),
            EventKind::Percussion(n) => format!("percussion {n}"),
            EventKind::Command { command, params } => {
                let p: Vec<String> = params.iter().map(|b| format!("${b:02X}")).collect();
                let at = |i: usize| params.get(i).copied().unwrap_or(0) as u16;
                match command.name {
                    "call" => format!(
                        "call ${:04X}, {} time{}",
                        at(0) | at(1) << 8,
                        at(2),
                        if at(2) == 1 { "" } else { "s" }
                    ),
                    _ if p.is_empty() => format!("{}: {}", command.name, command.about),
                    _ => format!("{} {}: {}", command.name, p.join(" "), command.about),
                }
            }
            EventKind::Invalid => "not a byte of this format".to_owned(),
            EventKind::End => "end".to_owned(),
        }
    }
}

fn beat_words(ticks: u8) -> &'static str {
    match ticks {
        192 => " (a whole note)",
        96 => " (a half note)",
        72 => " (a dotted quarter)",
        48 => " (a quarter note)",
        36 => " (a dotted eighth)",
        32 => " (a quarter-note triplet)",
        24 => " (an eighth)",
        16 => " (an eighth-note triplet)",
        12 => " (a sixteenth)",
        6 => " (a thirty-second)",
        _ => "",
    }
}

/// A track decoded from `at` to its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub at: u16,
    pub events: Vec<TrackEvent>,
}

impl Track {
    /// It reached its end byte cleanly.
    pub fn ends(&self) -> bool {
        matches!(self.events.last().map(|e| &e.kind), Some(EventKind::End))
    }

    /// The event holding `address`.
    pub fn event_at(&self, address: u16) -> Option<usize> {
        self.events
            .iter()
            .position(|e| (e.at..e.at.wrapping_add(e.bytes.len() as u16)).contains(&address))
    }
}

/// Names a note: `$80` is C of the dialect's first octave.
pub fn note_name(dialect: Dialect, semitone: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!(
        "{}{}",
        NAMES[semitone as usize % 12],
        dialect.first_octave() + semitone as i32 / 12
    )
}

/// Decode the track at `at` to its end byte, at most 4,096 bytes; an
/// invalid byte stops it.
pub fn track(aram: &[u8], dialect: Dialect, at: u16) -> Track {
    let (notes, tie, rest, drums) = dialect.notes();
    let mut events = Vec::new();
    let mut p = at as usize;
    let end = at as usize + 4096;
    while p < end && p < 0xFFC0 {
        let b = aram[p];
        let (kind, len) = match b {
            0x00 => (EventKind::End, 1),
            0x01..=0x7F => {
                let next = aram[(p + 1) & 0xFFFF];
                if next < 0x80 && next != 0 {
                    (
                        EventKind::Length {
                            ticks: b,
                            quantize: Some(next >> 4 & 7),
                            velocity: Some(next & 0xF),
                        },
                        2,
                    )
                } else {
                    (
                        EventKind::Length {
                            ticks: b,
                            quantize: None,
                            velocity: None,
                        },
                        1,
                    )
                }
            }
            n if notes.contains(&n) => (
                EventKind::Note {
                    semitone: n - 0x80,
                    name: note_name(dialect, n - 0x80),
                },
                1,
            ),
            n if n == tie => (EventKind::Tie, 1),
            n if n == rest => (EventKind::Rest, 1),
            0xC8..=0xCF if dialect == Dialect::Old => (EventKind::Rest, 1),
            n if drums.contains(&n) => (EventKind::Percussion(n - drums.start()), 1),
            n => match dialect.command(n) {
                Some(command) => {
                    let params = (1..=command.params as usize)
                        .map(|i| aram[(p + i) & 0xFFFF])
                        .collect();
                    (
                        EventKind::Command { command, params },
                        1 + command.params as usize,
                    )
                }
                None => (EventKind::Invalid, 1),
            },
        };
        let stop = matches!(kind, EventKind::End | EventKind::Invalid);
        events.push(TrackEvent {
            at: p as u16,
            bytes: (0..len).map(|i| aram[(p + i) & 0xFFFF]).collect(),
            kind,
        });
        if stop {
            break;
        }
        p += len;
    }
    Track { at, events }
}

/// Where the driver is now: the song, the block it is in, and each
/// voice's place in its track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playing {
    /// The song's number, as a game sends it (its table entry plus one).
    pub song: Option<u8>,
    pub list: u16,
    /// The block being played, and its entry in the list.
    pub block: u16,
    pub entry: u16,
    /// Each voice's track, and the next byte it will read.
    pub tracks: [u16; 8],
    pub positions: [Option<u16>; 8],
}

/// Read the driver's direct page: the song list pointer at `$40` (the
/// entry after the block being played) and the track pointers at
/// `$30–$3F`. `None` when they do not point into a song this driver has.
pub fn playing(aram: &[u8], driver: &Driver) -> Option<Playing> {
    let next = word(aram, 0x40);
    let (number, list, entries) = driver.songs.iter().enumerate().find_map(|(i, &s)| {
        let l = song_list(aram, driver.dialect, s)?;
        let last = l.last()?.at();
        (s..=last.wrapping_add(2))
            .contains(&next)
            .then_some((i, s, l))
    })?;
    // The block entry just before the pointer.
    let entry = entries
        .iter()
        .rev()
        .find(|e| matches!(e, ListEntry::Block { .. }) && e.at() < next)
        .or_else(|| {
            entries
                .iter()
                .find(|e| matches!(e, ListEntry::Block { .. }))
        })?;
    let ListEntry::Block { at, block } = *entry else {
        return None;
    };
    let tracks = block_tracks(aram, block);
    let positions: [Option<u16>; 8] = std::array::from_fn(|v| {
        let p = word(aram, 0x30 + v * 2);
        (tracks[v] != 0 && p != 0).then_some(p)
    });
    Some(Playing {
        song: u8::try_from(number + 1).ok(),
        list,
        block,
        entry: at,
        tracks,
        positions,
    })
}
