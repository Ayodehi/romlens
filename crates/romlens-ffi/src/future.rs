//! A oneshot future completed by a plain thread: UniFFI drives it from the
//! foreign side, so no async runtime is needed. Dropping the future (a
//! cancelled Swift task) raises the cancel flag the worker watches.

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::RomlensError;

struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

pub struct ThreadFuture<T> {
    slot: Arc<Mutex<Slot<T>>>,
    cancel: Arc<AtomicBool>,
    done: bool,
}

/// What a future gives when its work panicked: without one the future
/// would never complete and the Swift `await` would wait forever.
pub trait Panicked {
    fn panicked(why: String) -> Self;
}

impl<T> Panicked for Result<T, RomlensError> {
    fn panicked(why: String) -> Self {
        Err(RomlensError::Io {
            msg: format!("Romlens stopped on an internal error: {why}"),
        })
    }
}

impl<T> Panicked for Option<T> {
    fn panicked(_: String) -> Self {
        None
    }
}

/// A panic's message, when it has one.
fn message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned())
}

/// Run `work` on a new thread and hand back a future for its result, or
/// for [`Panicked::panicked`] if it panics.
pub fn spawn<T: Panicked + Send + 'static>(
    cancel: Arc<AtomicBool>,
    work: impl FnOnce() -> T + Send + 'static,
) -> ThreadFuture<T> {
    let slot = Arc::new(Mutex::new(Slot {
        value: None,
        waker: None,
    }));
    let producer = Arc::clone(&slot);
    std::thread::Builder::new()
        .name("romlens-analysis".into())
        .spawn(move || {
            // The work owns everything it touches and nothing outlives a
            // panic but the message, so unwinding past it is safe.
            let value = std::panic::catch_unwind(AssertUnwindSafe(work))
                .unwrap_or_else(|p| T::panicked(message(&*p)));
            let waker = {
                let mut s = producer.lock().unwrap_or_else(|e| e.into_inner());
                s.value = Some(value);
                s.waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        })
        .expect("spawn analysis thread");
    ThreadFuture {
        slot,
        cancel,
        done: false,
    }
}

impl<T> Future for ThreadFuture<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut s = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = s.value.take() {
            drop(s);
            self.done = true;
            return Poll::Ready(v);
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Drop for ThreadFuture<T> {
    fn drop(&mut self) {
        if !self.done {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}
