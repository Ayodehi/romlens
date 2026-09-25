//! A live session: the recorder script's stream arriving over a local TCP
//! connection while the game runs, held in memory as the latest frames.
//!
//! The script sends exactly what it writes to a `.rlstream` file (`docs/13`,
//! "The Mesen stream"), so one decoder serves both ([`StreamDecoder`]). A
//! [`LiveSource`] is a [`MachineStateSource`] like a `.romrec`, so every view
//! that reads a recording reads a live session unchanged; it keeps a window
//! of recent frames rather than all of them, and numbers frames itself, so a
//! script that reconnects carries on from where the last connection left off.
//!
//! The listener binds the loopback address only. Nothing off the machine can
//! connect, and a stream recorded from another ROM is refused on its header.

use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::recording::delta::{Run, diff, union};
use crate::recording::mesen::pack::StreamDecoder;
use crate::recording::mesen::stream::{Record, StreamReader};
use crate::recording::{
    Layers, MachineState, MachineStateSource, RecordingError, RecordingIdentity, StateRegion,
};
use crate::rom::image::RomImage;

/// The port the recorder script tries when told nothing else.
pub const DEFAULT_PORT: u16 = 7462;

/// Frames a session keeps by default: ten seconds at 60 fps, about 40 MB
/// without WRAM.
pub const DEFAULT_CAPACITY: usize = 600;

/// What a live frame holds. WRAM is left out: nothing live reads it yet, and
/// it would triple the memory a window of frames takes.
pub const LIVE_REGIONS: [StateRegion; 7] = [
    StateRegion::CpuRegisters,
    StateRegion::PpuState,
    StateRegion::IoState,
    StateRegion::Vram,
    StateRegion::Cgram,
    StateRegion::Oam,
    StateRegion::Timing,
];

/// The latest frames of a live session.
pub struct LiveSource {
    identity: RecordingIdentity,
    capacity: usize,
    inner: RwLock<Window>,
}

#[derive(Default)]
struct Window {
    /// Consecutive frames, oldest first.
    frames: VecDeque<MachineState>,
    /// The number of `frames[0]`.
    first: u64,
}

impl LiveSource {
    pub fn new(identity: RecordingIdentity, capacity: usize) -> Self {
        LiveSource {
            identity,
            capacity: capacity.max(2),
            inner: RwLock::new(Window::default()),
        }
    }

    /// Add the next frame, numbered after the last, dropping the oldest when
    /// the window is full. Returns its number.
    pub fn push(&self, mut state: MachineState) -> u64 {
        let mut w = self.inner.write().unwrap();
        let number = w.first + w.frames.len() as u64;
        state.frame = number;
        w.frames.push_back(state);
        if w.frames.len() > self.capacity {
            w.frames.pop_front();
            w.first += 1;
        }
        number
    }

    /// The oldest frame still held.
    pub fn first_frame(&self) -> u64 {
        self.inner.read().unwrap().first
    }

    /// The newest frame, if any has arrived.
    pub fn latest(&self) -> Option<u64> {
        let w = self.inner.read().unwrap();
        (!w.frames.is_empty()).then(|| w.first + w.frames.len() as u64 - 1)
    }

    fn get(&self, frame: u64) -> Result<MachineState, RecordingError> {
        let w = self.inner.read().unwrap();
        let end = w.first + w.frames.len() as u64;
        if frame < w.first {
            return Err(RecordingError::Dropped {
                frame,
                first: w.first,
            });
        }
        w.frames
            .get((frame - w.first) as usize)
            .cloned()
            .ok_or(RecordingError::NoSuchFrame { frame, count: end })
    }
}

impl MachineStateSource for LiveSource {
    fn identity(&self) -> &RecordingIdentity {
        &self.identity
    }

    /// Frames so far: a live session has no end yet, but views need a count
    /// to scrub within, and this one only grows.
    fn frame_count(&self) -> Option<u64> {
        let w = self.inner.read().unwrap();
        Some(w.first + w.frames.len() as u64)
    }

    fn regions(&self) -> Vec<StateRegion> {
        LIVE_REGIONS.to_vec()
    }

    fn state_at(&self, frame: u64) -> Result<MachineState, RecordingError> {
        self.get(frame)
    }

    fn region_at(&self, frame: u64, region: StateRegion) -> Result<Vec<u8>, RecordingError> {
        let w = self.inner.read().unwrap();
        if frame < w.first {
            return Err(RecordingError::Dropped {
                frame,
                first: w.first,
            });
        }
        let count = w.first + w.frames.len() as u64;
        let state = w
            .frames
            .get((frame - w.first) as usize)
            .ok_or(RecordingError::NoSuchFrame { frame, count })?;
        state
            .region(region)
            .map(<[u8]>::to_vec)
            .ok_or(RecordingError::MissingRegion(region.name()))
    }

    fn changes(&self, from: u64, to: u64, region: StateRegion) -> Result<Vec<Run>, RecordingError> {
        let mut out = Vec::new();
        let mut prev = self.region_at(from, region)?;
        for f in from + 1..=to {
            let next = self.region_at(f, region)?;
            out = union(&out, &diff(&prev, &next));
            prev = next;
        }
        Ok(out)
    }

    fn layers(&self) -> Layers {
        Layers::default()
    }
}

/// What a session is doing, for the shell to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStatus {
    /// Waiting for the script on this port.
    Listening { port: u16 },
    /// A script is streaming.
    Connected { producer: String },
    /// The script stopped or the connection dropped; listening again.
    Disconnected { reason: String },
    /// A stream was turned away, e.g. recorded from another ROM.
    Refused { reason: String },
}

/// Receives a session's news, on the session's own thread.
pub trait LiveEvents: Send + Sync {
    /// A frame arrived and is now the latest.
    fn frame(&self, number: u64);
    fn status(&self, status: LiveStatus);
    /// An execution log arrived: the whole log so far on connecting, then
    /// what was recorded since the previous one. Merging each in turn gives
    /// the session's log.
    fn exec_log(&self, _log: Vec<u8>) {}
}

/// A listener on the loopback address, one connection at a time.
pub struct LiveServer {
    port: u16,
    source: Arc<LiveSource>,
    stop: Arc<AtomicBool>,
    /// The connection being read, so `stop` can end a blocked read.
    current: Arc<Mutex<Option<TcpStream>>>,
    thread: Option<JoinHandle<()>>,
}

impl LiveServer {
    /// Listen on `port` (0 for any free port) for streams recorded from `rom`.
    pub fn start(
        rom: &RomImage,
        port: u16,
        capacity: usize,
        events: Arc<dyn LiveEvents>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let source = Arc::new(LiveSource::new(
            RecordingIdentity {
                rom_sha256: *rom.sha256(),
                producer: "Mesen (live)".into(),
                producer_version: "recorder stream".into(),
            },
            capacity,
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let current: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let rom_bytes = rom.bytes().to_vec();
        let thread = {
            let (source, stop, current) =
                (Arc::clone(&source), Arc::clone(&stop), Arc::clone(&current));
            std::thread::Builder::new()
                .name("romlens-live".into())
                .spawn(move || {
                    events.status(LiveStatus::Listening { port });
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let status = serve(stream, &rom_bytes, &source, &*events, &current);
                                if !stop.load(Ordering::Relaxed) {
                                    events.status(status);
                                    events.status(LiveStatus::Listening { port });
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            Err(e) => {
                                events.status(LiveStatus::Disconnected {
                                    reason: e.to_string(),
                                });
                                std::thread::sleep(Duration::from_millis(250));
                            }
                        }
                    }
                })?
        };
        Ok(LiveServer {
            port,
            source,
            stop,
            current,
            thread: Some(thread),
        })
    }

    /// A session fed from `input` instead of a connection, read on its own
    /// thread as a connection would be: for tests and for replaying a saved
    /// `.rlstream` as if it were live. Its port is 0.
    pub fn replay(
        rom: &RomImage,
        capacity: usize,
        events: Arc<dyn LiveEvents>,
        input: impl io::Read + Send + 'static,
    ) -> io::Result<Self> {
        let source = Arc::new(LiveSource::new(
            RecordingIdentity {
                rom_sha256: *rom.sha256(),
                producer: "Mesen (replay)".into(),
                producer_version: "recorder stream".into(),
            },
            capacity,
        ));
        let rom_bytes = rom.bytes().to_vec();
        let thread = {
            let source = Arc::clone(&source);
            std::thread::Builder::new()
                .name("romlens-live-replay".into())
                .spawn(move || {
                    let status = read_stream(input, &rom_bytes, &source, &*events);
                    events.status(status);
                })?
        };
        Ok(LiveServer {
            port: 0,
            source,
            stop: Arc::new(AtomicBool::new(false)),
            current: Arc::new(Mutex::new(None)),
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn source(&self) -> Arc<LiveSource> {
        Arc::clone(&self.source)
    }

    /// Stop listening and close any connection. The frames already held stay
    /// readable through [`source`](Self::source).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(s) = self.current.lock().unwrap().take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Read one connection to its end; what became of it.
fn serve(
    stream: TcpStream,
    rom: &[u8],
    source: &LiveSource,
    events: &dyn LiveEvents,
    current: &Mutex<Option<TcpStream>>,
) -> LiveStatus {
    // The accepted socket inherits nothing useful from the non-blocking
    // listener; reads here block until the script sends or goes away.
    if stream.set_nonblocking(false).is_err() {
        return LiveStatus::Disconnected {
            reason: "could not read the connection".into(),
        };
    }
    let _ = stream.set_nodelay(true);
    *current.lock().unwrap() = stream.try_clone().ok();
    let status = read_stream(stream, rom, source, events);
    *current.lock().unwrap() = None;
    status
}

fn read_stream(
    input: impl io::Read,
    rom: &[u8],
    source: &LiveSource,
    events: &dyn LiveEvents,
) -> LiveStatus {
    let mut reader = match StreamReader::new(io::BufReader::with_capacity(1 << 20, input)) {
        Ok(r) => r,
        Err(e) => {
            return LiveStatus::Refused {
                reason: e.to_string(),
            };
        }
    };
    if let Some(why) = reader.header.rom_mismatch(rom) {
        return LiveStatus::Refused {
            reason: format!(
                "the recorder is running for a different game ({why}). In Mesen, stop the recorder script, load this ROM, and run the script again"
            ),
        };
    }
    events.status(LiveStatus::Connected {
        producer: reader.header.producer.clone(),
    });
    let mut decoder = StreamDecoder::new(&reader.header);
    loop {
        match reader.next_record() {
            Ok(Some(Record::Frame(f))) => {
                let next = source.latest().map_or(source.first_frame(), |l| l + 1);
                let n = source.push(decoder.frame(&f, next, &LIVE_REGIONS));
                events.frame(n);
            }
            Ok(Some(Record::ExecLog(log))) => events.exec_log(log),
            Ok(Some(Record::End { .. })) => {
                return LiveStatus::Disconnected {
                    reason: "the script stopped".into(),
                };
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return LiveStatus::Disconnected {
                    reason: "the connection closed".into(),
                };
            }
            Err(e) => {
                return LiveStatus::Disconnected {
                    reason: e.to_string(),
                };
            }
        }
    }
}
