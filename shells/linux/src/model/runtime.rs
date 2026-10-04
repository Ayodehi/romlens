//! What the model needs from the event loop: timers, background analysis and
//! delivery of core events to the main thread. The GTK shell implements it on
//! GLib; tests implement it synchronously.

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use romlens_ffi::{AnalysisStats, RomlensError, Workbench, WorkbenchEvent};

pub trait Runtime {
    /// Run `f` once on the main thread after `delay`.
    fn after(&self, delay: Duration, f: Box<dyn FnOnce()>);

    /// Run the analysis off the main thread and call `done` on the main
    /// thread with the result. The workbench's own `cancel_analysis` ends it.
    fn analyze(
        &self,
        workbench: Arc<Workbench>,
        done: Box<dyn FnOnce(Result<AnalysisStats, RomlensError>)>,
    );

    /// Run `work` on a worker thread and call `done` with its result on the
    /// main thread. Use [`background`] rather than this untyped form.
    fn spawn(
        &self,
        work: Box<dyn FnOnce() -> Box<dyn Any + Send> + Send>,
        done: Box<dyn FnOnce(Box<dyn Any + Send>)>,
    );

    /// A way for any thread to hand a message to `handler`, which runs on the
    /// main thread: how a live session's frames, status and execution logs
    /// reach the model. Messages are delivered in order.
    fn sink(&self, handler: Rc<dyn Fn(Box<dyn Any + Send>)>) -> Post;

    /// Deliver the workbench's events (raised on any thread) to `handler` on
    /// the main thread.
    fn attach_listener(&self, workbench: &Workbench, handler: Rc<dyn Fn(WorkbenchEvent)>);
}

/// Posts a message to the main thread from any thread.
pub type Post = std::sync::Arc<dyn Fn(Box<dyn Any + Send>) + Send + Sync>;

/// Typed wrapper over [`Runtime::spawn`]: slow reads (the navigator's lists,
/// a decompile) run off the main thread and land back on it.
pub fn background<T: Send + 'static>(
    runtime: &dyn Runtime,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    runtime.spawn(
        Box::new(move || Box::new(work())),
        Box::new(move |result| {
            if let Ok(value) = result.downcast::<T>() {
                done(*value);
            }
        }),
    );
}
