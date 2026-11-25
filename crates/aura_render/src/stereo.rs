//! Audio layouts: mono and stereo samplers, and the adapters between them.
//!
//! Design: `docs/design/audio-layouts.md`.

use crate::Sampler;
use std::sync::Arc;

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

/// Pure function of time in seconds to a stereo frame `[left, right]`.
pub trait StereoSampler: Send + Sync {
    fn frame(&self, time_secs: f64) -> [f32; 2];

    /// One channel (`LEFT` or `RIGHT`). Implementations that compute channels independently
    /// override this so that reading one channel does not compute the other.
    fn channel(&self, channel: usize, time_secs: f64) -> f32 {
        self.frame(time_secs)[channel]
    }

    /// See [`Sampler::prepare`].
    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        let _ = (start_secs, ahead_secs);
    }

    /// See [`Sampler::underruns`].
    fn underruns(&self) -> u64 {
        0
    }
}

/// A translated audio signal of either layout.
pub enum AudioSampler {
    Mono(Box<dyn Sampler>),
    Stereo(Box<dyn StereoSampler>),
}

impl AudioSampler {
    pub fn layout(&self) -> Layout {
        match self {
            AudioSampler::Mono(_) => Layout::Mono,
            AudioSampler::Stereo(_) => Layout::Stereo,
        }
    }

    /// A stereo frame; a mono signal is the same on both channels.
    pub fn frame(&self, time_secs: f64) -> [f32; 2] {
        match self {
            AudioSampler::Mono(sampler) => {
                let value = sampler.at(time_secs);
                [value, value]
            }
            AudioSampler::Stereo(sampler) => sampler.frame(time_secs),
        }
    }

    pub fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        match self {
            AudioSampler::Mono(sampler) => sampler.prepare(start_secs, ahead_secs),
            AudioSampler::Stereo(sampler) => sampler.prepare(start_secs, ahead_secs),
        }
    }

    pub fn underruns(&self) -> u64 {
        match self {
            AudioSampler::Mono(sampler) => sampler.underruns(),
            AudioSampler::Stereo(sampler) => sampler.underruns(),
        }
    }

    /// The mono sampler, or `None` for a stereo signal.
    pub fn into_mono(self) -> Option<Box<dyn Sampler>> {
        match self {
            AudioSampler::Mono(sampler) => Some(sampler),
            AudioSampler::Stereo(_) => None,
        }
    }
}

impl From<Box<dyn Sampler>> for AudioSampler {
    fn from(sampler: Box<dyn Sampler>) -> Self {
        AudioSampler::Mono(sampler)
    }
}

/// One channel of a shared stereo signal, read without computing the other channel when the
/// source computes channels independently.
pub struct ChannelView {
    source: Arc<dyn StereoSampler>,
    channel: usize,
}

impl ChannelView {
    pub fn new(source: Arc<dyn StereoSampler>, channel: usize) -> Self {
        Self { source, channel }
    }
}

impl Sampler for ChannelView {
    fn at(&self, time_secs: f64) -> f32 {
        self.source.channel(self.channel, time_secs)
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        self.source.prepare(start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        self.source.underruns()
    }
}

/// A stereo signal made of two independent mono signals (for example dual mono).
pub struct StereoPair {
    left: Box<dyn Sampler>,
    right: Box<dyn Sampler>,
}

impl StereoPair {
    pub fn new(left: Box<dyn Sampler>, right: Box<dyn Sampler>) -> Self {
        Self { left, right }
    }
}

impl StereoSampler for StereoPair {
    fn frame(&self, time_secs: f64) -> [f32; 2] {
        [self.left.at(time_secs), self.right.at(time_secs)]
    }

    fn channel(&self, channel: usize, time_secs: f64) -> f32 {
        if channel == LEFT {
            self.left.at(time_secs)
        } else {
            self.right.at(time_secs)
        }
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        self.left.prepare(start_secs, ahead_secs);
        self.right.prepare(start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        self.left.underruns() + self.right.underruns()
    }
}

/// A mono sampler shared by several consumers.
pub struct SharedSampler(pub Arc<dyn Sampler>);

impl Sampler for SharedSampler {
    fn at(&self, time_secs: f64) -> f32 {
        self.0.at(time_secs)
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        self.0.prepare(start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        self.0.underruns()
    }
}

/// A stereo sampler shared by several consumers.
pub struct SharedStereoSampler(pub Arc<dyn StereoSampler>);

impl StereoSampler for SharedStereoSampler {
    fn frame(&self, time_secs: f64) -> [f32; 2] {
        self.0.frame(time_secs)
    }

    fn channel(&self, channel: usize, time_secs: f64) -> f32 {
        self.0.channel(channel, time_secs)
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        self.0.prepare(start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        self.0.underruns()
    }
}

/// Samples `sampler` over the render interval as interleaved frames of its layout.
pub fn sample_offline_audio(
    spec: crate::RenderSpec,
    sampler: &AudioSampler,
) -> Result<Vec<f32>, crate::RenderError> {
    match sampler {
        AudioSampler::Mono(mono) => crate::sample_offline(spec, mono.as_ref()),
        AudioSampler::Stereo(stereo) => {
            let frame_count = spec.expected_frame_count();
            let rate = spec.sample_rate.get() as f64;
            let mut out = Vec::with_capacity(frame_count * 2);
            for frame in 0..frame_count {
                let [left, right] = stereo.frame(frame as f64 / rate);
                out.push(left);
                out.push(right);
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>, f32);

    impl Sampler for Counting {
        fn at(&self, _t: f64) -> f32 {
            self.0.fetch_add(1, Ordering::Relaxed);
            self.1
        }
    }

    #[test]
    fn channel_view_reads_one_side_of_a_pair() {
        let left_reads = Arc::new(AtomicUsize::new(0));
        let right_reads = Arc::new(AtomicUsize::new(0));
        let pair: Arc<dyn StereoSampler> = Arc::new(StereoPair::new(
            Box::new(Counting(left_reads.clone(), 1.0)),
            Box::new(Counting(right_reads.clone(), -1.0)),
        ));
        let left = ChannelView::new(pair.clone(), LEFT);
        assert_eq!(left.at(0.0), 1.0);
        assert_eq!(left_reads.load(Ordering::Relaxed), 1);
        assert_eq!(right_reads.load(Ordering::Relaxed), 0);
        assert_eq!(ChannelView::new(pair, RIGHT).at(0.0), -1.0);
    }

    #[test]
    fn mono_frames_duplicate_the_sample() {
        let mono = AudioSampler::Mono(Box::new(Counting(Arc::new(AtomicUsize::new(0)), 0.5)));
        assert_eq!(mono.frame(0.0), [0.5, 0.5]);
        assert_eq!(mono.layout(), Layout::Mono);
    }
}
