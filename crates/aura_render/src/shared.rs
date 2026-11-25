//! One node output read by several consumers.
//!
//! Translation builds each node output once. When more than one consumer reads it, they read
//! through clones of one [`Shared`] (or [`SharedStereo`]) handle, which renders its source once
//! per requested range and hands every consumer asking for that range the same block.
//!
//! The memo is a nonessential cache in deterministic mode: a consumer asking for a different
//! range than the last one simply renders that range. In streaming mode every consumer asks for
//! the same consecutive blocks, so a stateful source advances exactly once per block.

use crate::{Frame, Signal, StereoSignal, BLOCK_FRAMES};
use std::sync::{Arc, Mutex};

struct Memo {
    start: Frame,
    len: usize,
    valid: bool,
}

impl Memo {
    fn new() -> Self {
        Self {
            start: 0,
            len: 0,
            valid: false,
        }
    }

    /// Whether the last render covered exactly this range; otherwise marks it as the new one.
    fn hit(&mut self, start: Frame, len: usize) -> bool {
        if self.valid && self.start == start && self.len == len {
            return true;
        }
        self.start = start;
        self.len = len;
        self.valid = true;
        false
    }
}

struct MonoInner {
    source: Box<dyn Signal>,
    memo: Memo,
    buf: Vec<f32>,
}

/// A mono signal read by several consumers. Clones share the source and its last block.
#[derive(Clone)]
pub struct Shared(Arc<Mutex<MonoInner>>);

impl Shared {
    pub fn new(source: Box<dyn Signal>) -> Self {
        Self(Arc::new(Mutex::new(MonoInner {
            source,
            memo: Memo::new(),
            buf: vec![0.0; BLOCK_FRAMES],
        })))
    }
}

impl Signal for Shared {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let mut inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let inner = &mut *inner;
        let len = out.len();
        if !inner.memo.hit(start, len) {
            inner.source.render(start, &mut inner.buf[..len]);
        }
        out.copy_from_slice(&inner.buf[..len]);
    }
}

struct StereoInner {
    source: Box<dyn StereoSignal>,
    memo: Memo,
    left: Vec<f32>,
    right: Vec<f32>,
}

/// A stereo signal read by several consumers. Clones share the source and its last block.
#[derive(Clone)]
pub struct SharedStereo(Arc<Mutex<StereoInner>>);

impl SharedStereo {
    pub fn new(source: Box<dyn StereoSignal>) -> Self {
        Self(Arc::new(Mutex::new(StereoInner {
            source,
            memo: Memo::new(),
            left: vec![0.0; BLOCK_FRAMES],
            right: vec![0.0; BLOCK_FRAMES],
        })))
    }
}

impl StereoSignal for SharedStereo {
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]) {
        let mut inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let inner = &mut *inner;
        let len = left.len();
        if !inner.memo.hit(start, len) {
            inner
                .source
                .render(start, &mut inner.left[..len], &mut inner.right[..len]);
        }
        left.copy_from_slice(&inner.left[..len]);
        right.copy_from_slice(&inner.right[..len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Ramp(Arc<AtomicUsize>);

    impl Signal for Ramp {
        fn render(&mut self, start: Frame, out: &mut [f32]) {
            self.0.fetch_add(1, Ordering::Relaxed);
            for (k, sample) in out.iter_mut().enumerate() {
                *sample = (start + k as Frame) as f32;
            }
        }
    }

    #[test]
    fn consumers_of_one_range_render_the_source_once() {
        let renders = Arc::new(AtomicUsize::new(0));
        let mut a = Shared::new(Box::new(Ramp(renders.clone())));
        let mut b = a.clone();
        let (mut x, mut y) = ([0.0; 8], [0.0; 8]);
        a.render(16, &mut x);
        b.render(16, &mut y);
        assert_eq!(x, y);
        assert_eq!(renders.load(Ordering::Relaxed), 1);
        b.render(8, &mut y);
        assert_eq!(y[0], 8.0, "another range renders that range");
        assert_eq!(renders.load(Ordering::Relaxed), 2);
    }
}
