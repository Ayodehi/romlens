//! Cancellation and progress for a run of the analyzer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisPhase {
    Descent,
    /// Resolving jump tables between descent passes.
    Tables,
    Sweep,
    /// Scoring whatever the walk and the sweep left unclassified.
    Heuristics,
    Labels,
    /// Building the line index (done by the caller after `analyze`).
    Lines,
}

impl AnalysisPhase {
    pub const fn name(self) -> &'static str {
        match self {
            AnalysisPhase::Descent => "descent",
            AnalysisPhase::Tables => "tables",
            AnalysisPhase::Heuristics => "heuristics",
            AnalysisPhase::Sweep => "sweep",
            AnalysisPhase::Labels => "labels",
            AnalysisPhase::Lines => "lines",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: AnalysisPhase,
    pub done: u64,
    pub total: u64,
}

/// The run was cancelled through its flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("analysis cancelled")]
pub struct Cancelled;

pub struct AnalysisControl {
    pub cancel: Arc<AtomicBool>,
    pub progress: Box<dyn Fn(Progress) + Send + Sync>,
}

impl AnalysisControl {
    /// Never cancels, reports nothing.
    pub fn silent() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Box::new(|_| {}),
        }
    }

    pub fn with_progress(f: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Box::new(f),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn report(&self, phase: AnalysisPhase, done: u64, total: u64) {
        (self.progress)(Progress { phase, done, total });
    }
}
