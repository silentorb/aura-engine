//! Audio layouts: mono and stereo signals, and the adapters between them.
//!
//! Design: `docs/design/audio-layouts.md`.

use crate::{Frame, Input, Signal, StereoSignal, BLOCK_FRAMES};

/// Channel layout of an audio signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Layout {
    #[default]
    Mono,
    Stereo,
}

impl Layout {
    pub fn channels(self) -> usize {
        match self {
            Layout::Mono => 1,
            Layout::Stereo => 2,
        }
    }
}

/// Left channel index.
pub const LEFT: usize = 0;
/// Right channel index.
pub const RIGHT: usize = 1;

/// A translated audio signal of either layout.
pub enum AudioSignal {
    Mono(Box<dyn Signal>),
    Stereo(Box<dyn StereoSignal>),
}

impl AudioSignal {
    pub fn layout(&self) -> Layout {
        match self {
            AudioSignal::Mono(_) => Layout::Mono,
            AudioSignal::Stereo(_) => Layout::Stereo,
        }
    }

    /// The mono signal, or `None` for a stereo signal.
    pub fn into_mono(self) -> Option<Box<dyn Signal>> {
        match self {
            AudioSignal::Mono(signal) => Some(signal),
            AudioSignal::Stereo(_) => None,
        }
    }
}

impl From<Box<dyn Signal>> for AudioSignal {
    fn from(signal: Box<dyn Signal>) -> Self {
        AudioSignal::Mono(signal)
    }
}

/// One channel of a stereo signal. Both channels are rendered (into a scratch buffer for the
/// other one); share the source ([`crate::SharedStereo`]) so both views render it once.
pub struct StereoChannel {
    source: Box<dyn StereoSignal>,
    channel: usize,
    other: Vec<f32>,
}

impl StereoChannel {
    pub fn new(source: Box<dyn StereoSignal>, channel: usize) -> Self {
        Self {
            source,
            channel,
            other: vec![0.0; BLOCK_FRAMES],
        }
    }
}

impl Signal for StereoChannel {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let other = &mut self.other[..out.len()];
        if self.channel == crate::LEFT {
            self.source.render(start, out, other);
        } else {
            self.source.render(start, other, out);
        }
    }
}

/// A stereo signal made of two mono signals (for example dual mono).
pub struct StereoPair {
    left: Box<dyn Signal>,
    right: Box<dyn Signal>,
}

impl StereoPair {
    pub fn new(left: Box<dyn Signal>, right: Box<dyn Signal>) -> Self {
        Self { left, right }
    }
}

impl StereoSignal for StereoPair {
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]) {
        self.left.render(start, left);
        self.right.render(start, right);
    }
}

/// The average of a stereo signal's channels.
pub struct StereoToMono(crate::StereoInput);

impl StereoToMono {
    pub fn new(source: Box<dyn StereoSignal>) -> Self {
        Self(crate::StereoInput::new(source))
    }
}

impl Signal for StereoToMono {
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        let (left, right) = self.0.read(start, out.len());
        for ((sample, &l), &r) in out.iter_mut().zip(left).zip(right) {
            *sample = (l + r) * 0.5;
        }
    }
}

/// A mono signal on both channels.
pub struct MonoToStereo(Input);

impl MonoToStereo {
    pub fn new(source: Box<dyn Signal>) -> Self {
        Self(Input::new(source))
    }
}

impl StereoSignal for MonoToStereo {
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]) {
        let value = self.0.read(start, left.len());
        left.copy_from_slice(value);
        right.copy_from_slice(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Constant, SharedStereo};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Counting(Arc<AtomicUsize>, f32);

    impl Signal for Counting {
        fn render(&mut self, _start: Frame, out: &mut [f32]) {
            self.0.fetch_add(1, Ordering::Relaxed);
            out.fill(self.1);
        }
    }

    #[test]
    fn channel_views_of_a_shared_pair_render_it_once_per_block() {
        let left_reads = Arc::new(AtomicUsize::new(0));
        let pair = SharedStereo::new(Box::new(StereoPair::new(
            Box::new(Counting(left_reads.clone(), 1.0)),
            Box::new(Constant(-1.0)),
        )));
        let mut left = StereoChannel::new(Box::new(pair.clone()), LEFT);
        let mut right = StereoChannel::new(Box::new(pair), RIGHT);
        let mut out = [0.0; 4];
        left.render(0, &mut out);
        assert_eq!(out, [1.0; 4]);
        right.render(0, &mut out);
        assert_eq!(out, [-1.0; 4]);
        assert_eq!(left_reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut mono = StereoToMono::new(Box::new(StereoPair::new(
            Box::new(Constant(1.0)),
            Box::new(Constant(0.0)),
        )));
        let mut out = [0.0; 3];
        mono.render(5, &mut out);
        assert_eq!(out, [0.5; 3]);
    }
}
