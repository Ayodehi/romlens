//! Live sessions for the shells: a loopback listener for the recorder
//! script's stream, whose frames read through an ordinary
//! [`RecordingSession`] (`romlens_core::recording::live`).

use std::sync::{Arc, Mutex};

use romlens_core::recording::live::{self, LiveEvents, LiveServer};

use crate::graphics::RecordingSession;
use crate::{Rom, RomlensError};

/// What a live session is doing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LiveStatus {
    Listening { port: u16 },
    Connected { producer: String },
    Disconnected { reason: String },
    Refused { reason: String },
}

impl From<live::LiveStatus> for LiveStatus {
    fn from(s: live::LiveStatus) -> Self {
        match s {
            live::LiveStatus::Listening { port } => LiveStatus::Listening { port },
            live::LiveStatus::Connected { producer } => LiveStatus::Connected { producer },
            live::LiveStatus::Disconnected { reason } => LiveStatus::Disconnected { reason },
            live::LiveStatus::Refused { reason } => LiveStatus::Refused { reason },
        }
    }
}

/// Receives a live session's news, on the session's thread: a frame for every
/// frame that arrives, so a shell should coalesce before touching its views.
#[uniffi::export(with_foreign)]
pub trait LiveListener: Send + Sync {
    fn on_frame(&self, frame: u64);
    fn on_status(&self, status: LiveStatus);
}

/// Forwards events, remembering the last status: a shell attaches after the
/// session starts, and a stream may already have connected by then.
struct Forward {
    listener: Arc<dyn LiveListener>,
    last: Arc<Mutex<Option<LiveStatus>>>,
}

impl LiveEvents for Forward {
    fn frame(&self, number: u64) {
        self.listener.on_frame(number);
    }
    fn status(&self, status: live::LiveStatus) {
        let status: LiveStatus = status.into();
        *self.last.lock().unwrap() = Some(status.clone());
        self.listener.on_status(status);
    }
}

/// The port the recorder script tries by default.
#[uniffi::export]
pub fn live_default_port() -> u16 {
    live::DEFAULT_PORT
}

#[derive(uniffi::Object)]
pub struct LiveSession {
    server: Mutex<Option<LiveServer>>,
    port: u16,
    recording: Arc<RecordingSession>,
    source: Arc<live::LiveSource>,
    last: Arc<Mutex<Option<LiveStatus>>>,
}

impl LiveSession {
    fn new(server: LiveServer, last: Arc<Mutex<Option<LiveStatus>>>) -> Arc<Self> {
        Arc::new(LiveSession {
            port: server.port(),
            recording: RecordingSession::live(server.source()),
            source: server.source(),
            server: Mutex::new(Some(server)),
            last,
        })
    }
}

#[uniffi::export]
impl LiveSession {
    /// Listen on the loopback address at `port` (0 for any free port) for
    /// streams recorded from `rom`.
    #[uniffi::constructor]
    pub fn start(
        rom: Arc<Rom>,
        port: u16,
        listener: Arc<dyn LiveListener>,
    ) -> Result<Arc<Self>, RomlensError> {
        let last = Arc::new(Mutex::new(None));
        let server = LiveServer::start(
            &rom.image,
            port,
            live::DEFAULT_CAPACITY,
            Arc::new(Forward {
                listener,
                last: Arc::clone(&last),
            }),
        )
        .map_err(|e| RomlensError::Io {
            msg: format!("could not listen on port {port}: {e}"),
        })?;
        Ok(LiveSession::new(server, last))
    }

    /// A session fed from a saved stream's bytes rather than a connection,
    /// read exactly as a connection would be: for tests, and for replaying an
    /// `.rlstream` as if it were live.
    #[uniffi::constructor]
    pub fn replay(
        rom: Arc<Rom>,
        stream: Vec<u8>,
        listener: Arc<dyn LiveListener>,
    ) -> Result<Arc<Self>, RomlensError> {
        let last = Arc::new(Mutex::new(None));
        let server = LiveServer::replay(
            &rom.image,
            live::DEFAULT_CAPACITY,
            Arc::new(Forward {
                listener,
                last: Arc::clone(&last),
            }),
            std::io::Cursor::new(stream),
        )
        .map_err(|e| RomlensError::Io { msg: e.to_string() })?;
        Ok(LiveSession::new(server, last))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The frames so far, read like any recording.
    pub fn recording(&self) -> Arc<RecordingSession> {
        Arc::clone(&self.recording)
    }

    /// Stop listening. The frames already held stay readable.
    pub fn stop(&self) {
        if let Some(mut server) = self.server.lock().unwrap().take() {
            server.stop();
        }
    }

    /// The last status reported, for a shell attaching after the start.
    pub fn status(&self) -> Option<LiveStatus> {
        self.last.lock().unwrap().clone()
    }

    /// The newest frame received, if any.
    pub fn latest_frame(&self) -> Option<u64> {
        self.source.latest()
    }

    pub fn is_listening(&self) -> bool {
        self.server.lock().unwrap().is_some()
    }
}

/// A short recorder stream of `rom`, as the script sends it, for shell
/// tests: frame n writes VRAM block n, and the stream ends cleanly.
#[uniffi::export]
pub fn make_test_stream(rom: Arc<Rom>, frames: u32) -> Vec<u8> {
    romlens_core::recording::mesen::stream::encode::fixture(rom.image.bytes(), frames, true)
}
