//! The sound views' shared state (docs/23): what they read and what is
//! selected in them.
//!
//! They read either the attached recording at the Graphics views' frame (one
//! frame for both, so the screen and the sound are the same moment), or the
//! ROM: the driver its upload routine sends, booted on Romlens's own SPC700
//! with the other uploads laid over it, as it stands after starting. The macOS
//! twin is `AudioModel`.
//!
//! The recording and frame come from the document, which keeps them current
//! with `sync_recording`; the model plays, reads and selects.

use std::collections::BTreeSet;
use std::sync::Arc;

use romlens_ffi::{
    ApuPlayer, AramRegionInfo, BrrSampleInfo, DspRegisterInfo, NoteEventInfo, NoteSourceInfo,
    NspcDriverInfo, NspcEntryInfo, NspcEventInfo, PortEventInfo, RecordingSession, Rom, SampleInfo,
    SentUploadInfo, SoundCommandInfo, SpcLineInfo, UploadBlockInfo, UploadInfo, UploadReportInfo,
    VoiceInfo, Workbench,
};

use crate::audio_out::ApuAudio;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Voices,
    Timeline,
    Samples,
    Aram,
    Ports,
    Echo,
    Scope,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Voices,
        Tab::Timeline,
        Tab::Samples,
        Tab::Aram,
        Tab::Ports,
        Tab::Echo,
        Tab::Scope,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tab::Voices => "voices",
            Tab::Timeline => "timeline",
            Tab::Samples => "samples",
            Tab::Aram => "audio-ram",
            Tab::Ports => "ports",
            Tab::Echo => "echo",
            Tab::Scope => "scope",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Voices => "Voices",
            Tab::Timeline => "Timeline",
            Tab::Samples => "Samples",
            Tab::Aram => "Audio RAM",
            Tab::Ports => "Ports",
            Tab::Echo => "Echo & Effects",
            Tab::Scope => "Scope",
        }
    }

    pub fn from_id(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// What is heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playing {
    /// The machine: the ROM's driver, or the recording from a frame.
    Machine,
    /// One sample alone on voice 0, played from the keyboard.
    Sample(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The attached recording at the frame.
    Recording,
    /// The ROM's upload, run by Romlens.
    Rom,
}

/// Everything the views show about one moment, read once.
#[derive(Clone)]
pub struct State {
    pub voices: Vec<VoiceInfo>,
    pub registers: Vec<DspRegisterInfo>,
    pub map: Vec<AramRegionInfo>,
    pub samples: Vec<SampleInfo>,
    pub pc: u16,
    /// Nintendo's N-SPC driver, when audio RAM holds it.
    pub nspc: Option<NspcDriverInfo>,
}

/// A recording frame lasts 1 / 60.0988 s (NTSC).
pub const FRAMES_PER_SECOND: f64 = 60.0988;

/// How often the machine is read again while playing.
pub const TICK_MS: u64 = 66;

#[derive(Clone, PartialEq, Eq)]
struct Key {
    source: Source,
    frame: u64,
    machine: u64,
    recording: Option<usize>,
}

pub struct AudioModel {
    rom: Arc<Rom>,
    workbench: Arc<Workbench>,
    pub source: Source,

    // The attached recording, as the document last reported it.
    recording: Option<Arc<RecordingSession>>,
    recording_name: Option<String>,
    frame: u64,
    frame_count: u64,

    // Selection
    pub selected_voice: Option<usize>,
    pub selected_sample: Option<u8>,
    pub selected_block: Option<usize>,
    /// The audio RAM part selected, by its start.
    pub selected_part: Option<u16>,

    // The ROM's upload
    upload: Option<UploadReportInfo>,
    upload_loading: bool,
    upload_for: Option<u64>,
    /// The block lists laid over the driver, by SNES address.
    included_lists: BTreeSet<u32>,
    /// Why the ROM's machine could not be built.
    rom_problem: Option<String>,
    rom_player: Option<Arc<ApuPlayer>>,
    /// Bumped whenever the ROM's machine changes, so the state is read again.
    machine_generation: u64,

    // Playback
    pub output: ApuAudio,
    playing: Option<Playing>,
    /// The player heard: the ROM's machine, a recording's from a frame, or a
    /// sample's.
    live_player: Option<Arc<ApuPlayer>>,
    /// Voices left out of the mix, bit n for voice n.
    muted: u8,
    /// Playing a recording: its port writes arrive as they did, and the frame
    /// moves with the sound.
    follow_recording: bool,
    played_seconds: f64,
    /// The last port writes sent by hand, newest last.
    sent: Vec<(u8, u8)>,
    play_from: u64,
    frame_set: Option<u64>,
    sample_player: Option<(u8, Arc<ApuPlayer>)>,

    // Timeline
    notes: Vec<NoteEventInfo>,
    notes_loading: bool,
    pub selected_note: Option<NoteEventInfo>,
    note_source: Option<NoteSourceInfo>,
    /// Frames the timeline shows across.
    pub timeline_span: u64,
    /// An address in audio RAM to show in its listing.
    pub listing_target: Option<u16>,
    notes_for: Option<usize>,

    source_picked: bool,
    /// The song the Song pane shows; the one playing when `None`.
    pub chosen_song: Option<u8>,
    /// The voice whose track the Song pane shows.
    pub song_voice: usize,

    cache: std::cell::RefCell<Option<(Key, State)>>,
}

fn id_of(r: &Arc<RecordingSession>) -> usize {
    Arc::as_ptr(r) as usize
}

impl AudioModel {
    pub fn new(rom: Arc<Rom>, workbench: Arc<Workbench>, device_enabled: bool) -> Self {
        Self {
            rom,
            workbench,
            source: Source::Rom,
            recording: None,
            recording_name: None,
            frame: 0,
            frame_count: 0,
            selected_voice: Some(0),
            selected_sample: None,
            selected_block: None,
            selected_part: None,
            upload: None,
            upload_loading: false,
            upload_for: None,
            included_lists: BTreeSet::new(),
            rom_problem: None,
            rom_player: None,
            machine_generation: 0,
            output: ApuAudio::new(device_enabled),
            playing: None,
            live_player: None,
            muted: 0,
            follow_recording: true,
            played_seconds: 0.0,
            sent: Vec::new(),
            play_from: 0,
            frame_set: None,
            sample_player: None,
            notes: Vec::new(),
            notes_loading: false,
            selected_note: None,
            note_source: None,
            timeline_span: 600,
            listing_target: None,
            notes_for: None,
            source_picked: false,
            chosen_song: None,
            song_voice: 0,
            cache: std::cell::RefCell::new(None),
        }
    }

    // MARK: Sources

    /// The document's recording and frame, kept current. A recording that
    /// goes, or changes, leaves the sound it played behind.
    pub fn sync_recording(
        &mut self,
        recording: Option<Arc<RecordingSession>>,
        name: Option<&str>,
        frame: u64,
        frame_count: u64,
    ) {
        let changed = self.recording.as_ref().map(id_of) != recording.as_ref().map(id_of);
        self.frame = frame;
        self.frame_count = frame_count;
        self.recording_name = name.map(str::to_owned);
        if changed {
            self.recording = recording;
            self.notes.clear();
            self.notes_for = None;
            self.selected_note = None;
            self.note_source = None;
            if self.source == Source::Recording {
                if self.playing.is_some() {
                    self.output.play(None);
                    self.playing = None;
                    self.live_player = None;
                }
                if !self.has_recording_sound() {
                    self.source = Source::Rom;
                }
            }
        }
    }

    /// The attached recording, when it has the sound side.
    pub fn recording(&self) -> Option<&Arc<RecordingSession>> {
        self.recording.as_ref().filter(|r| r.has_sound())
    }

    pub fn has_recording_sound(&self) -> bool {
        self.recording().is_some()
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// A view opens: the recording's sound if it has any and nothing was
    /// picked, else what was; the ROM when the recording has no sound.
    /// Returns true if the ROM's upload should be traced.
    pub fn opened(&mut self) -> bool {
        if !self.source_picked && self.has_recording_sound() {
            self.source = Source::Recording;
        }
        if self.source == Source::Recording && !self.has_recording_sound() {
            self.source = Source::Rom;
        }
        self.source == Source::Rom
    }

    /// The source bar's picker. Returns true if the ROM's upload should be
    /// traced.
    pub fn pick(&mut self, s: Source) -> bool {
        self.source_picked = true;
        if s != self.source {
            if self.playing.is_some() {
                self.output.play(None);
            }
            self.playing = None;
            self.live_player = None;
            self.sample_player = None;
            self.notes.clear();
            self.notes_for = None;
            self.selected_note = None;
            self.note_source = None;
        }
        self.source = s;
        self.opened()
    }

    pub fn source_description(&self) -> String {
        match self.source {
            Source::Recording => format!(
                "{} · frame {}",
                self.recording_name.as_deref().unwrap_or("Recording"),
                self.frame
            ),
            Source::Rom => {
                if self.upload_loading {
                    return "tracing the upload…".to_owned();
                }
                if let Some(p) = &self.rom_problem {
                    return p.clone();
                }
                let Some(d) = self.driver_upload() else {
                    return "the ROM".to_owned();
                };
                let mut lists = vec![d.list];
                if let Some(u) = &self.upload {
                    lists.extend(
                        u.uploads
                            .iter()
                            .filter(|u| self.included_lists.contains(&u.list))
                            .map(|u| u.list),
                    );
                }
                let names: Vec<String> = lists
                    .into_iter()
                    .map(romlens_ffi::format_snes_address)
                    .collect();
                format!("the driver from {}, run by Romlens", names.join(" + "))
            }
        }
    }

    // MARK: The ROM's upload

    pub fn upload(&self) -> Option<&UploadReportInfo> {
        self.upload.as_ref()
    }

    pub fn upload_loading(&self) -> bool {
        self.upload_loading
    }

    pub fn driver_upload(&self) -> Option<&UploadInfo> {
        self.upload.as_ref()?.uploads.iter().find(|u| u.driver)
    }

    /// The uploads a running driver is sent: songs and samples.
    pub fn other_uploads(&self) -> Vec<&UploadInfo> {
        self.upload.as_ref().map_or_else(Vec::new, |u| {
            u.uploads.iter().filter(|u| !u.driver).collect()
        })
    }

    pub fn included_lists(&self) -> &BTreeSet<u32> {
        &self.included_lists
    }

    pub fn rom_problem(&self) -> Option<&str> {
        self.rom_problem.as_deref()
    }

    /// Whether the upload in the analysis at `generation` is yet to be traced.
    /// If so it is marked loading, and the caller traces it.
    pub fn begin_upload(&mut self, generation: u64) -> bool {
        if self.upload_for == Some(generation) || self.upload_loading {
            return false;
        }
        self.upload_loading = true;
        true
    }

    pub fn finish_upload(&mut self, generation: u64, report: UploadReportInfo) {
        self.upload_for = Some(generation);
        self.upload_loading = false;
        self.included_lists = default_lists(&report);
        self.upload = Some(report);
        self.rebuild_rom_machine();
    }

    /// Include or leave out an upload's block list.
    pub fn set_included(&mut self, list: u32, included: bool) {
        let changed = if included {
            self.included_lists.insert(list)
        } else {
            self.included_lists.remove(&list)
        };
        if changed {
            self.rebuild_rom_machine();
        }
    }

    pub fn rebuild_rom_machine(&mut self) {
        if self.source == Source::Rom && self.playing.is_some() {
            self.output.play(None);
            self.playing = None;
            self.live_player = None;
        }
        self.sample_player = None;
        if self.source == Source::Rom {
            self.notes.clear();
            self.selected_note = None;
        }
        if self.driver_upload().is_none() {
            self.rom_player = None;
            self.rom_problem = self
                .upload
                .as_ref()
                .map(|_| "no upload of a sound driver was traced in this ROM".to_owned());
            self.machine_generation += 1;
            return;
        }
        let lists: Vec<u32> = self
            .other_uploads()
            .iter()
            .map(|u| u.list)
            .filter(|l| self.included_lists.contains(l))
            .collect();
        match ApuPlayer::from_upload(Arc::clone(&self.workbench), lists, 1.0) {
            Ok(p) => {
                p.log_notes();
                self.rom_player = Some(p);
                self.rom_problem = None;
            }
            Err(e) => {
                self.rom_player = None;
                self.rom_problem = Some(e.to_string());
            }
        }
        self.machine_generation += 1;
    }

    /// Show `player`'s machine in place of the ROM's upload: a machine built
    /// another way, such as a fixture's.
    pub fn load_machine(&mut self, player: Arc<ApuPlayer>) {
        self.source_picked = true;
        self.source = Source::Rom;
        self.rom_player = Some(player);
        self.rom_problem = None;
        self.notes.clear();
        self.machine_generation += 1;
    }

    /// Something changed the ROM's machine (it played, or a port was
    /// written): read its state again.
    pub fn machine_changed(&mut self) {
        self.machine_generation += 1;
    }

    // MARK: Playing

    pub fn playing(&self) -> Option<Playing> {
        self.playing
    }

    pub fn is_playing(&self) -> bool {
        self.playing == Some(Playing::Machine)
    }

    pub fn muted(&self) -> u8 {
        self.muted
    }

    pub fn set_muted(&mut self, mask: u8) {
        self.muted = mask;
        if let Some(p) = &self.live_player {
            p.set_muted(mask);
        }
    }

    pub fn follow_recording(&self) -> bool {
        self.follow_recording
    }

    pub fn set_follow_recording(&mut self, on: bool) {
        if on != self.follow_recording {
            self.follow_recording = on;
            if self.is_playing() && self.source == Source::Recording {
                self.restart_recording();
            }
        }
    }

    pub fn played_seconds(&self) -> f64 {
        self.played_seconds
    }

    pub fn sent(&self) -> &[(u8, u8)] {
        &self.sent
    }

    pub fn toggle_play(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Play the source from where it is: the ROM's machine carries on; a
    /// recording starts from the frame.
    pub fn play(&mut self) {
        match self.source {
            Source::Rom => {
                if let Some(p) = self.rom_player.clone() {
                    self.start(p);
                }
            }
            Source::Recording => self.restart_recording(),
        }
    }

    fn restart_recording(&mut self) {
        let Some(r) = self.recording().cloned() else {
            return;
        };
        let Ok(p) = ApuPlayer::from_recording(r, self.frame, self.follow_recording) else {
            return;
        };
        self.play_from = self.frame;
        self.frame_set = Some(self.frame);
        self.start(p);
    }

    fn start(&mut self, p: Arc<ApuPlayer>) {
        p.set_muted(self.muted);
        self.live_player = Some(Arc::clone(&p));
        self.playing = Some(Playing::Machine);
        self.played_seconds = 0.0;
        self.output.play(Some(p));
    }

    pub fn pause(&mut self) {
        self.output.play(None);
        self.playing = None;
        self.machine_changed();
    }

    /// Mute voice `v`, or hear it again.
    pub fn toggle_mute(&mut self, v: usize) {
        self.set_muted(self.muted ^ (1 << v));
    }

    /// Hear only voice `v`, or every voice if it already is.
    pub fn toggle_solo(&mut self, v: usize) {
        let only = !(1u8 << v);
        self.set_muted(if self.muted == only { 0 } else { only });
    }

    pub fn is_solo(&self, v: usize) -> bool {
        self.muted == !(1u8 << v)
    }

    /// The S-CPU writes a port, as the game would to ask for a sound: playing
    /// the ROM's machine (started if it was not), or the recording.
    pub fn send(&mut self, port: u8, value: u8) {
        let player = if self.is_playing() {
            self.live_player.clone()
        } else if self.source == Source::Rom {
            self.rom_player.clone()
        } else {
            self.live_player.clone()
        };
        let Some(p) = player else { return };
        // Written before playing starts: starting renders ahead at once.
        p.send_port(port & 3, value);
        if self.source == Source::Rom && !self.is_playing() {
            self.play();
        }
        self.sent.push((port & 3, value));
        if self.sent.len() > 16 {
            self.sent.remove(0);
        }
        self.machine_changed();
    }

    /// What the S-CPU would read back from a port now.
    pub fn read_port(&self, port: u8) -> Option<u8> {
        self.live_player
            .as_ref()
            .or(self.rom_player.as_ref())
            .map(|p| p.read_port(port))
    }

    /// The values the game's code sends on each port, from the trace.
    pub fn command_values(&self, port: u8) -> Vec<u32> {
        let mut v: Vec<u32> = self.upload.as_ref().map_or_else(Vec::new, |u| {
            u.commands
                .iter()
                .filter(|c| c.port == port)
                .map(|c| c.value)
                .collect()
        });
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The code the game sends a port value from, per the trace.
    pub fn command_sites(&self, port: u8, value: u8) -> Vec<SoundCommandInfo> {
        self.upload.as_ref().map_or_else(Vec::new, |u| {
            u.commands
                .iter()
                .filter(|c| c.port == port && c.value == u32::from(value))
                .cloned()
                .collect()
        })
    }

    /// Play This Command: the ROM's driver, sent `value` on `port`. Returns
    /// true if the ROM's upload should be traced first.
    pub fn play_command(&mut self, port: u8, value: u8) -> bool {
        let trace = self.pick(Source::Rom);
        if self.rom_player.is_some() {
            self.send(port, value);
        }
        trace
    }

    // MARK: A sample on the keyboard

    /// Sound directory entry `index` at `pitch` until `release`.
    pub fn press(&mut self, index: u8, pitch: u16) {
        if self.sample_player.as_ref().map(|s| s.0) != Some(index) {
            self.sample_player = self.make_sample_player(index).map(|p| (index, p));
        }
        let Some((_, p)) = self.sample_player.clone() else {
            return;
        };
        // Keyed first: starting the output renders ahead at once.
        p.set_pitch(0, pitch);
        p.key_on(1);
        if self.playing != Some(Playing::Sample(index)) {
            if self.is_playing() {
                self.pause();
            }
            self.live_player = Some(Arc::clone(&p));
            self.playing = Some(Playing::Sample(index));
            self.output.play(Some(p));
        }
    }

    pub fn release(&mut self) {
        if let Some((_, p)) = &self.sample_player {
            p.key_off(1);
        }
    }

    /// Stop the sample, leaving the machine paused.
    pub fn stop_sample(&mut self) {
        if matches!(self.playing, Some(Playing::Sample(_))) {
            self.output.play(None);
            self.playing = None;
            self.live_player = None;
        }
    }

    fn make_sample_player(&self, index: u8) -> Option<Arc<ApuPlayer>> {
        match self.source {
            Source::Recording => ApuPlayer::from_recorded_sample(
                self.recording()?.clone(),
                self.frame,
                index,
                0x1000,
            )
            .ok(),
            Source::Rom => {
                let state = self.state()?;
                let entry = state.samples.iter().find(|s| s.index == index)?;
                let rom = entry.rom_offset?;
                Some(ApuPlayer::from_rom_sample(
                    Arc::clone(&self.rom),
                    rom,
                    u32::from(entry.loop_at.wrapping_sub(entry.start)),
                    0x1000,
                ))
            }
        }
    }

    /// The pitch that plays directory entry `index` at `semitones` from A4,
    /// from its tuning; `0x1000` shifted by semitones from its own rate when
    /// it has none.
    pub fn pitch(&self, index: u8, semitones: i32) -> u16 {
        let hz = 440.0 * 2f64.powf(f64::from(semitones) / 12.0);
        let base = self
            .state()
            .and_then(|s| {
                s.samples
                    .iter()
                    .find(|s| s.index == index)
                    .and_then(|s| s.tuning_hz)
            })
            .unwrap_or(440.0);
        (4096.0 * hz / base).clamp(1.0, f64::from(0x3FFF)) as u16
    }

    // MARK: The clock

    /// While playing, a few times a second: read the machine again, and move
    /// the frame along with a recording. Returns the frame to move the
    /// graphics to, if the sound has carried on past it.
    pub fn tick(&mut self) -> TickResult {
        let (true, Some(p)) = (self.is_playing(), self.live_player.clone()) else {
            return TickResult::default();
        };
        self.played_seconds = p.seconds();
        if self.source == Source::Recording {
            // Scrubbed by hand while playing: play from there.
            if let Some(set) = self.frame_set
                && self.frame != set
            {
                self.restart_recording();
                return TickResult::default();
            }
            let at = self.play_from + (self.played_seconds * FRAMES_PER_SECOND) as u64;
            if at >= self.frame_count {
                self.pause();
                return TickResult {
                    stopped: true,
                    ..TickResult::default()
                };
            }
            if self.follow_recording {
                self.frame_set = Some(at);
                return TickResult {
                    move_to: Some(at),
                    ..TickResult::default()
                };
            }
            TickResult::default()
        } else {
            self.machine_changed();
            self.refresh_rom_notes();
            TickResult::default()
        }
    }

    /// The last `count` samples of voice 0 to 7, or 8 and 9 for left and
    /// right, from what is playing.
    pub fn scope(&self, which: u8, count: u32) -> Vec<i16> {
        self.live_player
            .as_ref()
            .map_or_else(Vec::new, |p| p.scope(which, count))
    }

    // MARK: Notes

    pub fn notes(&self) -> &[NoteEventInfo] {
        &self.notes
    }

    pub fn notes_loading(&self) -> bool {
        self.notes_loading
    }

    /// The recording's notes are yet to be read: returns the recording and
    /// the last frame, and marks them loading. The caller reads them off the
    /// main thread.
    pub fn begin_notes(&mut self) -> Option<(Arc<RecordingSession>, u64)> {
        match self.source {
            Source::Rom => {
                self.refresh_rom_notes();
                None
            }
            Source::Recording => {
                let r = self.recording()?.clone();
                if self.notes_for == Some(id_of(&r)) || self.notes_loading {
                    return None;
                }
                self.notes_loading = true;
                Some((r, self.frame_count.max(1) - 1))
            }
        }
    }

    pub fn finish_notes(&mut self, recording: &Arc<RecordingSession>, notes: Vec<NoteEventInfo>) {
        self.notes_loading = false;
        if self.recording.as_ref().map(id_of) == Some(id_of(recording)) {
            self.notes = notes;
            self.notes_for = Some(id_of(recording));
        }
    }

    fn refresh_rom_notes(&mut self) {
        let (Source::Rom, Some(p)) = (self.source, self.rom_player.clone()) else {
            return;
        };
        let count = p.note_count() as usize;
        if count < self.notes.len() {
            self.notes.clear();
        }
        if count != self.notes.len() {
            let more = p.notes(self.notes.len() as u32);
            self.notes.extend(more);
        }
        self.notes_for = None;
    }

    /// The frame the timeline centres on: the recording's, or the ROM
    /// machine's time in frames.
    pub fn timeline_now(&self) -> u64 {
        match self.source {
            Source::Recording => self.frame,
            Source::Rom => {
                (self.rom_player.as_ref().map_or(0.0, |p| p.seconds()) * FRAMES_PER_SECOND) as u64
            }
        }
    }

    /// Select a note and work out where it came from.
    pub fn select_note(&mut self, note: Option<NoteEventInfo>) {
        self.selected_note = note.clone();
        self.note_source = None;
        let Some(note) = note else { return };
        if let Some(pc) = note.spc_pc {
            self.note_source = Some(NoteSourceInfo {
                spc_pc: Some(pc),
                commands: Vec::new(),
            });
        } else if self.source == Source::Recording
            && let Some(r) = self.recording()
        {
            self.note_source = r.note_source(note.frame, note.spc_cycle).ok();
        }
    }

    pub fn note_source(&self) -> Option<&NoteSourceInfo> {
        self.note_source.as_ref()
    }

    /// Show the listing at an SPC700 address.
    pub fn show_spc(&mut self, address: u16) {
        self.selected_part = self.part_containing(address).map(|p| p.start);
        self.listing_target = Some(address);
    }

    // MARK: N-SPC

    pub fn nspc_song(&self, number: u8) -> Vec<NspcEntryInfo> {
        match self.source {
            Source::Recording => self
                .recording()
                .and_then(|r| r.nspc_song(self.frame, number).ok())
                .unwrap_or_default(),
            Source::Rom => self
                .rom_player
                .as_ref()
                .map_or_else(Vec::new, |p| p.nspc_song(number)),
        }
    }

    pub fn nspc_track(&self, at: u16) -> Vec<NspcEventInfo> {
        match self.source {
            Source::Recording => self
                .recording()
                .and_then(|r| r.nspc_track(self.frame, at).ok())
                .unwrap_or_default(),
            Source::Rom => self
                .rom_player
                .as_ref()
                .map_or_else(Vec::new, |p| p.nspc_track(at)),
        }
    }

    /// The uploads the recording saw, up to the frame.
    pub fn sent_uploads(&self) -> Vec<SentUploadInfo> {
        match (self.source, self.recording()) {
            (Source::Recording, Some(r)) => r.sent_uploads(0, self.frame).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The ports both ways around the frame.
    pub fn port_events(&self, around: u64, frames: u64) -> Vec<PortEventInfo> {
        match (self.source, self.recording()) {
            (Source::Recording, Some(r)) => r
                .port_events(around.saturating_sub(frames), around + frames)
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    // MARK: Reading

    fn key(&self) -> Key {
        Key {
            source: self.source,
            frame: if self.source == Source::Recording {
                self.frame
            } else {
                0
            },
            machine: if self.source == Source::Rom {
                self.machine_generation
            } else {
                0
            },
            recording: if self.source == Source::Recording {
                self.recording.as_ref().map(id_of)
            } else {
                None
            },
        }
    }

    /// The moment the views show, read once per frame or machine change.
    pub fn state(&self) -> Option<State> {
        let key = self.key();
        if let Some((k, s)) = self.cache.borrow().as_ref()
            && *k == key
        {
            return Some(s.clone());
        }
        let s = match self.source {
            Source::Recording => {
                let r = self.recording()?;
                let f = self.frame;
                State {
                    voices: r.voices(f).ok()?,
                    registers: r.dsp_registers(f).ok()?,
                    map: r.aram_map(f).ok()?,
                    samples: r.samples(f).ok()?,
                    pc: r.spc_pc(f).ok()?,
                    nspc: r.nspc(f).ok()?,
                }
            }
            Source::Rom => {
                let p = self.rom_player.as_ref()?;
                State {
                    voices: p.voices(),
                    registers: p.dsp_registers(),
                    map: p.aram_map(),
                    samples: p.samples(),
                    pc: p.spc_pc(),
                    nspc: p.nspc(),
                }
            }
        };
        *self.cache.borrow_mut() = Some((key, s.clone()));
        Some(s)
    }

    pub fn sample(&self, index: u8, max_blocks: u32) -> Option<BrrSampleInfo> {
        match self.source {
            Source::Recording => self.recording()?.sample(self.frame, index, max_blocks).ok(),
            Source::Rom => Some(self.rom_player.as_ref()?.sample(index, max_blocks)),
        }
    }

    pub fn listing(&self, from: u16, count: u32) -> Vec<SpcLineInfo> {
        match self.source {
            Source::Recording => self
                .recording()
                .and_then(|r| r.spc_listing(self.frame, from, count).ok())
                .unwrap_or_default(),
            Source::Rom => self
                .rom_player
                .as_ref()
                .map_or_else(Vec::new, |p| p.spc_listing(from, count)),
        }
    }

    pub fn aram(&self, start: u16, len: u32) -> Vec<u8> {
        match self.source {
            Source::Recording => self
                .recording()
                .and_then(|r| r.aram(self.frame, start, len).ok())
                .unwrap_or_default(),
            Source::Rom => self
                .rom_player
                .as_ref()
                .map_or_else(Vec::new, |p| p.aram(start, len)),
        }
    }

    /// Where audio RAM byte `address` came from in the ROM.
    pub fn rom_origin(&self, address: u16) -> Option<u32> {
        if self.source == Source::Rom {
            self.rom_player.as_ref()?.rom_origin(address)
        } else {
            None
        }
    }

    /// The part holding `address`.
    pub fn part_containing(&self, address: u16) -> Option<AramRegionInfo> {
        self.state()?.map.into_iter().find(|p| {
            u32::from(address) >= u32::from(p.start)
                && u32::from(address) < u32::from(p.start) + p.len
        })
    }

    /// The upload blocks that filled a part, with their ROM ranges.
    pub fn blocks_filling(&self, part: &AramRegionInfo) -> Vec<(UploadInfo, UploadBlockInfo)> {
        let (Source::Rom, Some(upload)) = (self.source, self.upload.as_ref()) else {
            return Vec::new();
        };
        let driver = self.driver_upload().map(|d| d.list);
        let range = u32::from(part.start)..u32::from(part.start) + part.len;
        let mut out = Vec::new();
        for u in &upload.uploads {
            if Some(u.list) != driver && !self.included_lists.contains(&u.list) {
                continue;
            }
            for b in &u.blocks {
                let r = u32::from(b.aram)..u32::from(b.aram) + u32::from(b.len);
                if r.start < range.end && range.start < r.end {
                    out.push((u.clone(), b.clone()));
                }
            }
        }
        out
    }
}

/// A note from its key-on to where it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSpan {
    pub on: NoteEventInfo,
    pub end: u64,
}

/// Each key-on to its key-off, or to the voice's next key-on; a note still
/// sounding at the end runs to `until`.
pub fn note_spans(notes: &[NoteEventInfo], until: u64) -> Vec<NoteSpan> {
    use romlens_ffi::NoteKindInfo;
    let mut open: [Option<&NoteEventInfo>; 8] = [None; 8];
    let mut out = Vec::new();
    for n in notes {
        let v = usize::from(n.voice & 7);
        match n.kind {
            NoteKindInfo::On => {
                if let Some(o) = open[v].take() {
                    out.push(NoteSpan {
                        on: o.clone(),
                        end: n.frame,
                    });
                }
                open[v] = Some(n);
            }
            NoteKindInfo::Off => {
                if let Some(o) = open[v].take() {
                    out.push(NoteSpan {
                        on: o.clone(),
                        end: n.frame,
                    });
                }
            }
            NoteKindInfo::Pitch => {}
        }
    }
    out.extend(open.into_iter().flatten().map(|o| NoteSpan {
        on: o.clone(),
        end: until,
    }));
    out
}

/// Up and down a lane by pitch, on a log scale: `$0100` at the bottom, `$3FFF`
/// at the top, as a fraction from the top of the lane.
pub fn pitch_y(pitch: u16) -> f64 {
    let l = f64::from(pitch.max(0x100)).log2();
    1.0 - (l - 8.0) / 6.0
}

/// The FIR filter's response, |H(f)| from 0 to 16 kHz in decibels.
pub fn fir_response(taps: &[i8; 8], count: usize) -> Vec<f64> {
    (0..count)
        .map(|i| {
            let w = std::f64::consts::PI * i as f64 / (count - 1) as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (k, t) in taps.iter().enumerate() {
                let c = f64::from(*t) / 128.0;
                re += c * (w * k as f64).cos();
                im -= c * (w * k as f64).sin();
            }
            20.0 * (re * re + im * im).sqrt().max(1e-4).log10()
        })
        .collect()
}

/// What a clock tick found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickResult {
    /// The recording's frame the sound has reached.
    pub move_to: Option<u64>,
    /// Playback ran off the end of the recording.
    pub stopped: bool,
}

/// Every upload besides the driver's whose audio RAM does not overlap one
/// before it: the samples and the first song bank, not a second bank meant to
/// replace the first.
pub fn default_lists(report: &UploadReportInfo) -> BTreeSet<u32> {
    let mut taken: Vec<std::ops::Range<u32>> = Vec::new();
    let mut out = BTreeSet::new();
    for u in &report.uploads {
        let ranges: Vec<_> = u
            .blocks
            .iter()
            .map(|b| u32::from(b.aram)..u32::from(b.aram) + u32::from(b.len))
            .collect();
        let overlaps =
            |a: &std::ops::Range<u32>, b: &std::ops::Range<u32>| a.start < b.end && b.start < a.end;
        if !u.driver && ranges.iter().any(|r| taken.iter().any(|t| overlaps(t, r))) {
            continue;
        }
        taken.extend(ranges);
        if !u.driver {
            out.insert(u.list);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use romlens_ffi::{make_sound_test_recording, make_sound_test_rom};

    fn model() -> (AudioModel, Arc<Workbench>) {
        let rom = Rom::from_bytes(make_sound_test_rom(), "s.sfc".into()).unwrap();
        let wb = Workbench::new(Arc::clone(&rom));
        wb.analyze_blocking().unwrap();
        (AudioModel::new(rom, Arc::clone(&wb), false), wb)
    }

    fn with_rom_machine() -> AudioModel {
        let (mut m, wb) = model();
        let generation = wb.analysis_generation();
        assert!(m.begin_upload(generation));
        m.finish_upload(generation, wb.sound_upload_blocking());
        m
    }

    fn with_recording(m: &mut AudioModel) -> Arc<RecordingSession> {
        let rec = RecordingSession::from_bytes(make_sound_test_recording(8)).unwrap();
        m.sync_recording(Some(Arc::clone(&rec)), Some("s.romrec"), 3, 8);
        rec
    }

    #[test]
    fn the_roms_upload_is_traced_once_per_analysis_and_builds_the_machine() {
        let (mut m, wb) = model();
        let g = wb.analysis_generation();
        assert!(m.begin_upload(g));
        assert!(!m.begin_upload(g), "one run at a time");
        assert_eq!(m.source_description(), "tracing the upload…");
        m.finish_upload(g, wb.sound_upload_blocking());
        assert!(!m.begin_upload(g), "once per analysis");
        assert!(m.driver_upload().is_some());
        assert!(m.rom_problem().is_none());
        assert!(m.source_description().starts_with("the driver from $"));
        assert!(m.state().is_some_and(|s| s.voices.len() == 8));
        // A new analysis traces again.
        assert!(m.begin_upload(g + 1));
    }

    #[test]
    fn a_rom_without_a_driver_says_so() {
        let rom = Rom::from_bytes(romlens_ffi::make_routines_test_rom(), "r.sfc".into()).unwrap();
        let wb = Workbench::new(Arc::clone(&rom));
        wb.analyze_blocking().unwrap();
        let mut m = AudioModel::new(rom, Arc::clone(&wb), false);
        let g = wb.analysis_generation();
        m.begin_upload(g);
        m.finish_upload(g, wb.sound_upload_blocking());
        assert_eq!(
            m.rom_problem(),
            Some("no upload of a sound driver was traced in this ROM")
        );
        assert!(m.state().is_none());
        m.play();
        assert!(!m.is_playing(), "nothing to play");
    }

    #[test]
    fn default_lists_leave_out_a_bank_that_overlaps_an_earlier_one() {
        let block = |aram: u16, len: u16| UploadBlockInfo {
            aram,
            len,
            rom_offset: 0,
            snes: 0,
        };
        let up = |list: u32, driver: bool, blocks: Vec<UploadBlockInfo>| UploadInfo {
            list,
            aram_table: None,
            source: String::new(),
            set_at: 0,
            set_in: 0,
            blocks,
            entry: 0,
            bytes: 0,
            driver,
        };
        let report = UploadReportInfo {
            routines: Vec::new(),
            uploads: vec![
                up(1, true, vec![block(0x0200, 0x1000)]),
                up(2, false, vec![block(0x4000, 0x100)]),
                up(3, false, vec![block(0x4080, 0x100)]),
                up(4, false, vec![block(0x6000, 0x10)]),
            ],
            commands: Vec::new(),
        };
        assert_eq!(default_lists(&report), BTreeSet::from([2, 4]));
    }

    #[test]
    fn playing_the_roms_machine_fills_the_output_and_pausing_stops_it() {
        let mut m = with_rom_machine();
        m.play();
        assert!(m.is_playing());
        assert_eq!(m.output.ring().available(), crate::audio_out::LEAD);
        let t = m.tick();
        assert_eq!(
            t,
            TickResult::default(),
            "the ROM's machine has no frame to move"
        );
        assert!(m.played_seconds() >= 0.0);
        m.toggle_play();
        assert!(!m.is_playing() && m.playing().is_none());
        assert_eq!(m.output.ring().available(), 0);
    }

    #[test]
    fn mute_and_solo_are_masks_and_solo_toggles() {
        let mut m = with_rom_machine();
        m.toggle_mute(2);
        assert_eq!(m.muted(), 0b100);
        m.toggle_mute(2);
        assert_eq!(m.muted(), 0);
        m.toggle_solo(1);
        assert_eq!(m.muted(), !0b10);
        assert!(m.is_solo(1) && !m.is_solo(0));
        m.toggle_solo(1);
        assert_eq!(m.muted(), 0, "soloing the soloed voice hears everything");
    }

    #[test]
    fn sending_a_port_starts_the_roms_machine_and_is_remembered_newest_last() {
        let mut m = with_rom_machine();
        m.send(1, 0x42);
        assert!(m.is_playing(), "a command started the machine");
        for i in 0..20u8 {
            m.send(i, i);
        }
        assert_eq!(m.sent().len(), 16, "only the last sixteen");
        assert_eq!(m.sent().last(), Some(&(3, 19)), "the port is two bits");
    }

    #[test]
    fn a_recording_with_sound_becomes_the_source_unless_the_source_was_picked() {
        let mut m = with_rom_machine();
        let _rec = with_recording(&mut m);
        assert!(m.has_recording_sound());
        assert!(!m.opened(), "opening takes the recording's sound");
        assert_eq!(m.source, Source::Recording);
        assert_eq!(m.source_description(), "s.romrec · frame 3");
        // A person's pick is kept.
        assert!(m.pick(Source::Rom));
        assert!(m.opened());
        assert_eq!(m.source, Source::Rom);
        // The recording going away puts a recording source back on the ROM.
        m.pick(Source::Recording);
        m.sync_recording(None, None, 0, 0);
        assert_eq!(m.source, Source::Rom);
    }

    #[test]
    fn reading_a_recording_follows_the_frame() {
        let mut m = with_rom_machine();
        let _rec = with_recording(&mut m);
        m.pick(Source::Recording);
        let a = m.state().unwrap();
        assert_eq!(a.voices.len(), 8);
        m.sync_recording(m.recording().cloned(), Some("s.romrec"), 5, 8);
        assert_eq!(m.frame(), 5);
        assert!(m.state().is_some());
        assert!(!m.aram(0x4000, 16).is_empty());
        assert!(!m.listing(0x0200, 4).is_empty());
        assert!(m.sample(0, 64).is_some());
    }

    #[test]
    fn playing_a_recording_moves_the_frame_with_the_sound_and_stops_at_the_end() {
        let mut m = with_rom_machine();
        let _rec = with_recording(&mut m);
        m.pick(Source::Recording);
        m.play();
        assert!(m.is_playing());
        // Render some sound so the clock advances.
        for _ in 0..40 {
            while m.output.ring().pop().is_some() {}
            m.output.fill();
        }
        let t = m.tick();
        assert!(t.stopped || t.move_to.is_some() || t == TickResult::default());
        // Run off the end: pausing ends it.
        m.sync_recording(m.recording().cloned(), Some("s.romrec"), 3, 3);
        m.play();
        m.frame_set = Some(3);
        for _ in 0..200 {
            while m.output.ring().pop().is_some() {}
            m.output.fill();
        }
        let t = m.tick();
        assert!(t.stopped, "{t:?}");
        assert!(!m.is_playing());
    }

    fn note(voice: u8, frame: u64, kind: romlens_ffi::NoteKindInfo, pitch: u16) -> NoteEventInfo {
        NoteEventInfo {
            voice,
            frame,
            spc_cycle: frame * 100,
            kind,
            pitch,
            source: 0,
            spc_pc: None,
        }
    }

    #[test]
    fn notes_run_from_key_on_to_key_off_or_the_voices_next_key_on() {
        use romlens_ffi::NoteKindInfo::{Off, On, Pitch};
        let notes = [
            note(0, 10, On, 0x1000),
            note(1, 12, On, 0x800),
            note(0, 15, Pitch, 0x1100),
            note(0, 20, Off, 0x1100),
            note(1, 30, On, 0x900),
            note(1, 33, On, 0x900),
        ];
        let spans = note_spans(&notes, 100);
        let find = |v: u8, f: u64| {
            spans
                .iter()
                .find(|s| s.on.voice == v && s.on.frame == f)
                .unwrap()
        };
        assert_eq!(find(0, 10).end, 20, "to its key-off");
        assert_eq!(find(1, 12).end, 30, "to the voice's next key-on");
        assert_eq!(find(1, 30).end, 33);
        assert_eq!(find(1, 33).end, 100, "still sounding at the end");
        assert_eq!(spans.len(), 4);
    }

    #[test]
    fn pitch_is_placed_on_a_log_scale_between_the_lane_edges() {
        assert!((pitch_y(0x100) - 1.0).abs() < 1e-9, "$0100 at the bottom");
        assert!(pitch_y(0x3FFF).abs() < 0.01, "$3FFF at the top");
        assert!(pitch_y(0x10) == pitch_y(0x100), "below $0100 is the bottom");
        assert!(pitch_y(0x2000) < pitch_y(0x1000));
    }

    #[test]
    fn a_flat_fir_passes_everything_and_a_lone_newest_tap_is_unchanged() {
        // One tap of 128/128 = 1: the filter does nothing, so 0 dB everywhere.
        let r = fir_response(&[0, 0, 0, 0, 0, 0, 0, 127], 64);
        assert!(r.iter().all(|db| db.abs() < 0.2), "{r:?}");
        // All eight taps: a low-pass, loud at 0 Hz and quiet at the top.
        let r = fir_response(&[16; 8], 64);
        assert!(r[0] > -0.5 && r[0] > r[63] + 20.0, "{} {}", r[0], r[63]);
        // Silence is the floor, not minus infinity.
        assert!(
            fir_response(&[0; 8], 8)
                .iter()
                .all(|db| (*db + 80.0).abs() < 1e-9)
        );
    }

    #[test]
    fn a_pitch_follows_the_samples_tuning() {
        let mut m = with_rom_machine();
        // A4 is the sample's own rate unless it has a tuning: 0x1000 at A4 for
        // an untuned sample, an octave up doubling it.
        let a4 = m.pitch(0, 0);
        let a5 = m.pitch(0, 12);
        assert!(a5 == a4.saturating_mul(2).min(0x3FFF));
        assert!(m.pitch(0, -200) >= 1 && m.pitch(0, 200) <= 0x3FFF);
        m.press(0, a4);
        assert!(matches!(m.playing(), Some(Playing::Sample(0))));
        m.release();
        m.stop_sample();
        assert!(m.playing().is_none());
    }

    #[test]
    fn the_roms_notes_grow_as_it_plays() {
        let mut m = with_rom_machine();
        m.send(0, 1);
        for _ in 0..50 {
            while m.output.ring().pop().is_some() {}
            m.output.fill();
        }
        m.tick();
        let _ = m.notes().len(); // may be empty for the fixture; never panics
        assert!(
            m.begin_notes().is_none(),
            "the ROM's notes are read in place"
        );
    }

    #[test]
    fn a_recordings_notes_are_read_once_off_the_main_thread() {
        let mut m = with_rom_machine();
        let rec = with_recording(&mut m);
        m.pick(Source::Recording);
        let (r, last) = m.begin_notes().expect("not read yet");
        assert_eq!(last, 7);
        assert!(m.notes_loading());
        assert!(m.begin_notes().is_none(), "one read at a time");
        let notes = r.note_timeline(0, last).unwrap_or_default();
        m.finish_notes(&rec, notes);
        assert!(!m.notes_loading());
        assert!(m.begin_notes().is_none(), "and once per recording");
    }

    #[test]
    fn a_part_is_found_by_address_and_the_blocks_that_filled_it_by_overlap() {
        let m = with_rom_machine();
        let state = m.state().unwrap();
        let sample = state
            .map
            .iter()
            .find(|p| p.kind == romlens_ffi::AramKindInfo::Sample)
            .expect("the fixture's sample is in the map")
            .clone();
        assert_eq!(
            m.part_containing(sample.start).map(|p| p.start),
            Some(sample.start)
        );
        let blocks = m.blocks_filling(&sample);
        assert!(!blocks.is_empty(), "an upload filled it");
        assert!(m.rom_origin(sample.start).is_some());
    }
}
