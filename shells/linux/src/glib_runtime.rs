//! The model's `Runtime` on the GLib main loop.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use romlens_ffi::{AnalysisStats, RomlensError, Workbench, WorkbenchEvent, WorkbenchListener};

use crate::model::Runtime;

pub struct GlibRuntime;

/// Receives core events on any thread and queues them for the main loop.
struct Bridge {
    tx: async_channel::Sender<WorkbenchEvent>,
}

impl WorkbenchListener for Bridge {
    fn on_event(&self, event: WorkbenchEvent) {
        // Unbounded: a closed receiver just means the document is gone.
        let _ = self.tx.try_send(event);
    }
}

impl Runtime for GlibRuntime {
    fn after(&self, delay: Duration, f: Box<dyn FnOnce()>) {
        glib::timeout_add_local_once(delay, f);
    }

    fn analyze(
        &self,
        workbench: Arc<Workbench>,
        done: Box<dyn FnOnce(Result<AnalysisStats, RomlensError>)>,
    ) {
        // The core's future runs the analysis on its own thread, and
        // dropping it cancels the run.
        glib::spawn_future_local(async move {
            let result = workbench.analyze().await;
            done(result);
        });
    }

    fn spawn(
        &self,
        work: Box<dyn FnOnce() -> Box<dyn std::any::Any + Send> + Send>,
        done: Box<dyn FnOnce(Box<dyn std::any::Any + Send>)>,
    ) {
        glib::spawn_future_local(async move {
            // A panic in the work is the caller's bug; dropping `done` is the
            // right answer to a result that never arrives.
            if let Ok(result) = gtk::gio::spawn_blocking(work).await {
                done(result);
            }
        });
    }

    fn sink(
        &self,
        handler: Rc<dyn Fn(Box<dyn std::any::Any + Send>)>,
    ) -> crate::model::runtime::Post {
        let (tx, rx) = async_channel::unbounded::<Box<dyn std::any::Any + Send>>();
        glib::spawn_future_local(async move {
            while let Ok(message) = rx.recv().await {
                handler(message);
            }
        });
        Arc::new(move |message| {
            // A closed receiver means the document is gone.
            let _ = tx.try_send(message);
        })
    }

    fn attach_listener(&self, workbench: &Workbench, handler: Rc<dyn Fn(WorkbenchEvent)>) {
        let (tx, rx) = async_channel::unbounded();
        workbench.set_listener(Some(Arc::new(Bridge { tx })));
        glib::spawn_future_local(async move {
            while let Ok(first) = rx.recv().await {
                // Progress is coalesced: only the latest report matters, and
                // the core can raise them far faster than a frame.
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                let last_progress = batch
                    .iter()
                    .rposition(|e| matches!(e, WorkbenchEvent::AnalysisProgress { .. }));
                for (i, event) in batch.into_iter().enumerate() {
                    if matches!(event, WorkbenchEvent::AnalysisProgress { .. })
                        && Some(i) != last_progress
                    {
                        continue;
                    }
                    handler(event);
                }
            }
        });
    }
}
