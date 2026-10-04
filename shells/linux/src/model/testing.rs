//! A synchronous `Runtime` for tests: timers fire when the test says so, and
//! analysis either runs inline or waits to be finished by hand, so the
//! "another run asked for while one runs" paths can be driven.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use romlens_ffi::{
    AnalysisStats, Mapping, Rom, RomlensError, Workbench, WorkbenchEvent, WorkbenchListener,
    make_test_rom,
};

use super::Runtime;

type Done = Box<dyn FnOnce(Result<AnalysisStats, RomlensError>)>;
type Timer = (Duration, Box<dyn FnOnce()>);
type Handler = Rc<dyn Fn(WorkbenchEvent)>;
type PostHandler = Rc<dyn Fn(Box<dyn std::any::Any + Send>)>;

#[derive(Default)]
pub struct TestRuntime {
    timers: RefCell<Vec<Timer>>,
    queue: Arc<Mutex<Vec<WorkbenchEvent>>>,
    posts: Arc<Mutex<Vec<Box<dyn std::any::Any + Send>>>>,
    sink: RefCell<Option<PostHandler>>,
    handler: RefCell<Option<Handler>>,
    /// When set, `analyze` parks the run here instead of doing it.
    pub defer: RefCell<bool>,
    parked: RefCell<Vec<(Arc<Workbench>, Done)>>,
    pub runs: RefCell<usize>,
}

struct Sink(Arc<Mutex<Vec<WorkbenchEvent>>>);

impl WorkbenchListener for Sink {
    fn on_event(&self, event: WorkbenchEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl TestRuntime {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Deliver queued core events, as the main loop would.
    pub fn pump(&self) {
        let posted: Vec<_> = std::mem::take(&mut *self.posts.lock().unwrap());
        let sink = self.sink.borrow().clone();
        if let Some(s) = sink {
            for m in posted {
                s(m);
            }
        }
        let events: Vec<_> = std::mem::take(&mut *self.queue.lock().unwrap());
        let handler = self.handler.borrow().clone();
        if let Some(h) = handler {
            for e in events {
                h(e);
            }
        }
    }

    pub fn pending_timers(&self) -> usize {
        self.timers.borrow().len()
    }

    pub fn fire_timers(&self) {
        let timers: Vec<_> = std::mem::take(&mut *self.timers.borrow_mut());
        for (_, f) in timers {
            f();
        }
    }

    /// Finish the oldest parked analysis.
    pub fn finish_parked(&self) {
        let parked = {
            let mut p = self.parked.borrow_mut();
            if p.is_empty() {
                None
            } else {
                Some(p.remove(0))
            }
        };
        if let Some((wb, done)) = parked {
            done(wb.analyze_blocking());
            self.pump();
        }
    }

    pub fn has_parked(&self) -> bool {
        !self.parked.borrow().is_empty()
    }
}

impl Runtime for TestRuntime {
    fn after(&self, delay: Duration, f: Box<dyn FnOnce()>) {
        self.timers.borrow_mut().push((delay, f));
    }

    fn analyze(&self, workbench: Arc<Workbench>, done: Done) {
        *self.runs.borrow_mut() += 1;
        if *self.defer.borrow() {
            self.parked.borrow_mut().push((workbench, done));
        } else {
            done(workbench.analyze_blocking());
        }
    }

    fn spawn(
        &self,
        work: Box<dyn FnOnce() -> Box<dyn std::any::Any + Send> + Send>,
        done: Box<dyn FnOnce(Box<dyn std::any::Any + Send>)>,
    ) {
        done(work());
    }

    fn sink(&self, handler: Rc<dyn Fn(Box<dyn std::any::Any + Send>)>) -> super::runtime::Post {
        *self.sink.borrow_mut() = Some(handler);
        let posts = Arc::clone(&self.posts);
        Arc::new(move |m| posts.lock().unwrap().push(m))
    }

    fn attach_listener(&self, workbench: &Workbench, handler: Rc<dyn Fn(WorkbenchEvent)>) {
        workbench.set_listener(Some(Arc::new(Sink(Arc::clone(&self.queue)))));
        *self.handler.borrow_mut() = Some(handler);
    }
}

pub fn test_rom() -> Arc<Rom> {
    Rom::from_bytes(make_test_rom(Mapping::LoRom), "test.sfc".into()).expect("test rom")
}
