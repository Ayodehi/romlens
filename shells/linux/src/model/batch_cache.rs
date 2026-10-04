//! LRU cache of decoded batches keyed by their first row. 64 batches of 256
//! hex rows is about 1 MB; a miss costs about 10 µs per batch (docs/10).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// A batch of consecutive rows (or lines) decoded from one flat buffer.
pub trait RowBatch: Sized {
    const ROWS_PER_BATCH: u32;
    fn decode(start_row: u32, data: &[u8]) -> Result<Self, String>;
}

type Fetch = Box<dyn Fn(u32, u32) -> Vec<u8>>;

pub struct BatchCache<B: RowBatch> {
    capacity: usize,
    fetch: Fetch,
    batches: RefCell<HashMap<u32, Rc<B>>>,
    /// Least recently used first.
    order: RefCell<VecDeque<u32>>,
    misses: Cell<usize>,
}

impl<B: RowBatch> BatchCache<B> {
    pub fn new(capacity: usize, fetch: impl Fn(u32, u32) -> Vec<u8> + 'static) -> Self {
        Self {
            capacity: capacity.max(1),
            fetch: Box::new(fetch),
            batches: RefCell::new(HashMap::new()),
            order: RefCell::new(VecDeque::new()),
            misses: Cell::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.batches.borrow().len()
    }

    pub fn miss_count(&self) -> usize {
        self.misses.get()
    }

    fn start_of(row: u32) -> u32 {
        row - row % B::ROWS_PER_BATCH
    }

    pub fn is_cached(&self, row: u32) -> bool {
        self.batches.borrow().contains_key(&Self::start_of(row))
    }

    /// The batch holding `row`, fetched on a miss. `None` only when the core
    /// returns a buffer that does not decode, which is a build skew between
    /// the shell and the library, not a runtime condition.
    pub fn batch(&self, row: u32) -> Option<Rc<B>> {
        let start = Self::start_of(row);
        if let Some(hit) = self.batches.borrow().get(&start) {
            let hit = Rc::clone(hit);
            self.touch(start);
            return Some(hit);
        }
        self.misses.set(self.misses.get() + 1);
        let data = (self.fetch)(start, B::ROWS_PER_BATCH);
        let batch = Rc::new(B::decode(start, &data).ok()?);
        self.batches.borrow_mut().insert(start, Rc::clone(&batch));
        let mut order = self.order.borrow_mut();
        order.push_back(start);
        while self.batches.borrow().len() > self.capacity {
            let Some(victim) = order.pop_front() else {
                break;
            };
            self.batches.borrow_mut().remove(&victim);
        }
        Some(batch)
    }

    /// Drop everything (the snapshot or labels changed).
    pub fn invalidate_all(&self) {
        self.batches.borrow_mut().clear();
        self.order.borrow_mut().clear();
    }

    fn touch(&self, start: u32) {
        let mut order = self.order.borrow_mut();
        if let Some(i) = order.iter().position(|&s| s == start) {
            order.remove(i);
            order.push_back(start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        start: u32,
    }

    impl RowBatch for Fake {
        const ROWS_PER_BATCH: u32 = 4;
        fn decode(start_row: u32, data: &[u8]) -> Result<Self, String> {
            if data.is_empty() {
                return Err("empty".into());
            }
            Ok(Self { start: start_row })
        }
    }

    fn cache(capacity: usize) -> BatchCache<Fake> {
        BatchCache::new(capacity, |_, _| vec![1])
    }

    #[test]
    fn one_fetch_per_batch() {
        let c = cache(4);
        for row in 0..4 {
            assert_eq!(c.batch(row).unwrap().start, 0);
        }
        assert_eq!(c.miss_count(), 1);
        assert_eq!(c.batch(5).unwrap().start, 4);
        assert_eq!(c.miss_count(), 2);
    }

    #[test]
    fn least_recently_used_batch_is_evicted() {
        let c = cache(2);
        c.batch(0);
        c.batch(4);
        c.batch(0); // 0 is now the most recent
        c.batch(8); // evicts 4
        assert!(c.is_cached(0) && c.is_cached(8) && !c.is_cached(4));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn invalidate_drops_everything() {
        let c = cache(2);
        c.batch(0);
        c.invalidate_all();
        assert_eq!(c.len(), 0);
        c.batch(0);
        assert_eq!(c.miss_count(), 2);
    }

    #[test]
    fn a_batch_that_does_not_decode_is_none() {
        let c: BatchCache<Fake> = BatchCache::new(2, |_, _| Vec::new());
        assert!(c.batch(0).is_none());
        assert_eq!(c.len(), 0);
    }
}
