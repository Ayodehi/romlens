//! A live session's news (docs/22), handed from the session's thread to the
//! main loop. The session raises a frame sixty times a second; the document
//! tells the views at most thirty times, with only the newest. The macOS twin
//! is `LiveBridge`.

use std::sync::Arc;

use romlens_ffi::Workbench;
use romlens_ffi::live::{LiveListener, LiveStatus};

use super::runtime::Post;

/// What the session's thread posts to the main loop.
pub enum LiveMessage {
    Frame(u64),
    Status(LiveStatus),
    /// The execution log merged into the project, adding this many
    /// instructions.
    Merged(u64),
}

/// How often the views are told of a new frame: at most this per second.
pub const MAX_FRAME_RATE: u64 = 30;

/// Receives the session's events on its own thread. The execution log merges
/// here, since the workbench is thread safe and a long session's log is
/// large; only the bookkeeping and the re-analysis come back to the main
/// loop.
pub struct LiveBridge {
    post: Post,
    workbench: Arc<Workbench>,
}

impl LiveBridge {
    pub fn new(post: Post, workbench: Arc<Workbench>) -> Arc<Self> {
        Arc::new(Self { post, workbench })
    }
}

impl LiveListener for LiveBridge {
    fn on_frame(&self, frame: u64) {
        (self.post)(Box::new(LiveMessage::Frame(frame)));
    }

    fn on_status(&self, status: LiveStatus) {
        (self.post)(Box::new(LiveMessage::Status(status)));
    }

    fn on_exec_log(&self, log: Vec<u8>) {
        if let Ok(added) = self.workbench.merge_live_log("live session".into(), log) {
            (self.post)(Box::new(LiveMessage::Merged(added)));
        }
    }
}

/// What a status says, in words, given what the session was doing.
pub fn status_text(previous: Option<&str>, status: &LiveStatus) -> Option<String> {
    Some(match status {
        LiveStatus::Listening { port } => {
            // A stream already connected keeps its word.
            if previous.is_some_and(|p| !p.starts_with("streaming")) && previous.is_some() {
                return None;
            }
            format!("waiting for Mesen on port {port}")
        }
        LiveStatus::Connected { .. } => "streaming".to_owned(),
        LiveStatus::Disconnected { reason } => format!("{reason}; waiting for Mesen"),
        LiveStatus::Refused { reason } => format!("refused: {reason}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_words_follow_the_session() {
        let listening = LiveStatus::Listening { port: 7462 };
        assert_eq!(
            status_text(None, &listening).as_deref(),
            Some("waiting for Mesen on port 7462")
        );
        // Listening again after a disconnect keeps the disconnect's reason.
        assert_eq!(
            status_text(Some("lost it; waiting for Mesen"), &listening),
            None
        );
        assert_eq!(
            status_text(Some("streaming"), &listening).as_deref(),
            Some("waiting for Mesen on port 7462")
        );
        assert_eq!(
            status_text(
                None,
                &LiveStatus::Connected {
                    producer: "m".into()
                }
            )
            .as_deref(),
            Some("streaming")
        );
        assert_eq!(
            status_text(
                None,
                &LiveStatus::Disconnected {
                    reason: "closed".into()
                }
            )
            .as_deref(),
            Some("closed; waiting for Mesen")
        );
        assert_eq!(
            status_text(
                None,
                &LiveStatus::Refused {
                    reason: "other ROM".into()
                }
            )
            .as_deref(),
            Some("refused: other ROM")
        );
    }
}
