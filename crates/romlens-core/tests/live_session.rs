//! A live session end to end: the recorder's stream over a real loopback
//! connection, read back through the same trait a `.romrec` is.

use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use romlens_core::RomImage;
use romlens_core::fixtures;
use romlens_core::recording::live::{LiveEvents, LiveServer, LiveStatus};
use romlens_core::recording::mesen::stream::{Record, StreamReader, encode};
use romlens_core::recording::{MachineStateSource, RecordingError, StateRegion};

#[derive(Default)]
struct Log {
    frames: Mutex<Vec<u64>>,
    statuses: Mutex<Vec<LiveStatus>>,
    logs: Mutex<Vec<Vec<u8>>>,
}

impl LiveEvents for Log {
    fn frame(&self, n: u64) {
        self.frames.lock().unwrap().push(n);
    }
    fn status(&self, s: LiveStatus) {
        self.statuses.lock().unwrap().push(s);
    }
    fn exec_log(&self, log: Vec<u8>) {
        self.logs.lock().unwrap().push(log);
    }
}

fn rom() -> RomImage {
    RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap()
}

fn wait(what: &str, until: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !until() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(port: u16, bytes: &[u8]) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(bytes).unwrap();
}

#[test]
fn frames_arrive_and_read_like_a_recording() {
    let rom = rom();
    let log = Arc::new(Log::default());
    let mut server = LiveServer::start(&rom, 0, 600, log.clone()).unwrap();
    let port = server.port();
    let source = server.source();

    // Five frames, then the clean end.
    send(port, &encode::fixture(rom.bytes(), 5, true));
    wait("five frames", || log.frames.lock().unwrap().len() == 5);
    assert_eq!(*log.frames.lock().unwrap(), vec![0, 1, 2, 3, 4]);
    assert_eq!(source.frame_count(), Some(5));

    // Frame n wrote VRAM block n with n + 1 (`encode::fixture`).
    let vram = source.region_at(3, StateRegion::Vram).unwrap();
    assert_eq!(vram[3 * 256], 4);
    assert_eq!(vram[4 * 256], 0, "frame 4's write is not in frame 3");
    let changed = source.changes(2, 3, StateRegion::Vram).unwrap();
    assert_eq!(changed.first().map(|r| r.offset), Some(3 * 256));
    wait("the end", || {
        log.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| matches!(s, LiveStatus::Disconnected { reason } if reason.contains("stopped")))
    });

    // The script reconnects: numbering carries on after the last frame.
    send(port, &encode::fixture(rom.bytes(), 2, false));
    wait("two more frames", || log.frames.lock().unwrap().len() == 7);
    assert_eq!(source.latest(), Some(6));
    server.stop();
    assert!(
        matches!(log.statuses.lock().unwrap()[0], LiveStatus::Listening { port: p } if p == port)
    );
}

#[test]
fn old_frames_fall_out_of_the_window() {
    let rom = rom();
    let log = Arc::new(Log::default());
    let mut server = LiveServer::start(&rom, 0, 3, log.clone()).unwrap();
    send(server.port(), &encode::fixture(rom.bytes(), 5, true));
    wait("five frames", || log.frames.lock().unwrap().len() == 5);
    let source = server.source();
    assert_eq!(source.first_frame(), 2);
    assert!(matches!(
        source.state_at(1),
        Err(RecordingError::Dropped { frame: 1, first: 2 })
    ));
    assert!(source.state_at(4).is_ok());
    server.stop();
}

#[test]
fn a_stream_from_another_rom_is_refused() {
    let rom = rom();
    let other = RomImage::from_bytes(fixtures::dispatch_lorom(), "o.sfc").unwrap();
    let log = Arc::new(Log::default());
    let mut server = LiveServer::start(&rom, 0, 600, log.clone()).unwrap();
    send(server.port(), &encode::fixture(other.bytes(), 3, true));
    wait("the refusal", || {
        log.statuses
            .lock()
            .unwrap()
            .iter()
            .any(|s| matches!(s, LiveStatus::Refused { .. }))
    });
    assert!(log.frames.lock().unwrap().is_empty());
    server.stop();
}

#[test]
fn execution_logs_arrive_between_frames() {
    let rom = rom();
    let log = Arc::new(Log::default());
    let mut server = LiveServer::start(&rom, 0, 600, log.clone()).unwrap();
    // The fixture's frames with two log records, as the script sends them:
    // the whole log after the first frame, a delta after the third.
    let plain = encode::fixture(rom.bytes(), 4, true);
    let mut reader = StreamReader::new(std::io::Cursor::new(&plain)).unwrap();
    let mut bytes = encode::header(&reader.header);
    let mut frames = 0;
    while let Some(r) = reader.next_record().unwrap() {
        let frame = matches!(r, Record::Frame(_));
        bytes.extend(encode::record(&r));
        if frame {
            frames += 1;
            if frames == 1 || frames == 3 {
                bytes.extend(encode::record(&Record::ExecLog(vec![frames as u8; 5])));
            }
        }
    }
    send(server.port(), &bytes);
    wait("both logs", || log.logs.lock().unwrap().len() == 2);
    assert_eq!(*log.logs.lock().unwrap(), vec![vec![1u8; 5], vec![3u8; 5]]);
    wait("all frames", || log.frames.lock().unwrap().len() == 4);
    server.stop();
}
