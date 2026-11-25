//! Audio rendering for Aura: signals rendered a block of frames at a time ([`Signal`],
//! [`StereoSignal`]), offline and real-time sinks over them ([`Renderer`]), and streaming voices
//! ([`BlockSource`]).
//!
//! Design: `docs/design/imp-execution.md`.
//!
//! A signal renders any requested range of frames into a buffer. Signals may keep state, under
//! a different rule per mode:
//!
//! - **Deterministic** — state is nonessential. Rendering a range gives the same output whether
//!   the signal is fresh or used, and whatever ranges were rendered before, in any order.
//! - **Streaming** — ranges arrive contiguously and only move forward, so state may carry
//!   history (filter memory, oscillator phase).

mod shared;
mod stereo;
mod streaming;

pub use shared::{Shared, SharedStereo};
pub use stereo::{
    AudioSignal, Layout, MonoToStereo, StereoChannel, StereoPair, StereoToMono, LEFT, RIGHT,
};

pub use streaming::{
    voice_channel, BlockSource, StreamVoice, VoiceCommand, VoiceMixer, VoiceSendError,
    VoiceSender, MAX_SLOTS, MAX_VOICES,
};

use aura_sample::{duration_to_frame_count, SampleRate};
use thiserror::Error;

/// A frame index from the start of a render, at the render's sample rate.
pub type Frame = u64;

/// Most frames a [`Signal`] or [`StereoSignal`] is asked for at once. Signals size their scratch
/// buffers to it; [`render_mono`] and [`Renderer`] split longer ranges.
pub const BLOCK_FRAMES: usize = 256;

/// Frame `frame` in seconds. Every signal computes time from frames this way, so the same frame
/// has the same time everywhere.
#[inline]
pub fn frame_secs(frame: Frame, rate: f64) -> f64 {
    frame as f64 / rate
}

/// A mono signal rendered a range of frames at a time.
pub trait Signal: Send {
    /// Writes frames `start .. start + out.len()` into `out`. `out.len()` is at most
    /// [`BLOCK_FRAMES`].
    fn render(&mut self, start: Frame, out: &mut [f32]);
}

/// A stereo signal rendered a range of frames at a time, one buffer per channel.
pub trait StereoSignal: Send {
    /// Writes frames `start .. start + left.len()` into `left` and `right` (equal lengths, at
    /// most [`BLOCK_FRAMES`]).
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]);
}

/// A signal input with its own block buffer: what a node reads its inputs through.
pub struct Input {
    signal: Box<dyn Signal>,
    buf: Vec<f32>,
}

impl Input {
    pub fn new(signal: Box<dyn Signal>) -> Self {
        Self {
            signal,
            buf: vec![0.0; BLOCK_FRAMES],
        }
    }

    /// Frames `start .. start + len` of the input (`len` at most [`BLOCK_FRAMES`]).
    pub fn read(&mut self, start: Frame, len: usize) -> &[f32] {
        let buf = &mut self.buf[..len];
        self.signal.render(start, buf);
        buf
    }
}

impl From<Box<dyn Signal>> for Input {
    fn from(signal: Box<dyn Signal>) -> Self {
        Self::new(signal)
    }
}

/// A stereo input with its own block buffers.
pub struct StereoInput {
    signal: Box<dyn StereoSignal>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl StereoInput {
    pub fn new(signal: Box<dyn StereoSignal>) -> Self {
        Self {
            signal,
            left: vec![0.0; BLOCK_FRAMES],
            right: vec![0.0; BLOCK_FRAMES],
        }
    }

    /// Both channels of frames `start .. start + len` (`len` at most [`BLOCK_FRAMES`]).
    pub fn read(&mut self, start: Frame, len: usize) -> (&[f32], &[f32]) {
        let (left, right) = (&mut self.left[..len], &mut self.right[..len]);
        self.signal.render(start, left, right);
        (left, right)
    }
}

impl From<Box<dyn StereoSignal>> for StereoInput {
    fn from(signal: Box<dyn StereoSignal>) -> Self {
        Self::new(signal)
    }
}

/// The same value on every frame.
pub struct Constant(pub f32);

impl Signal for Constant {
    fn render(&mut self, _start: Frame, out: &mut [f32]) {
        out.fill(self.0);
    }
}

/// Renders `out.len()` frames from `start`, of any length, in blocks of at most
/// [`BLOCK_FRAMES`].
pub fn render_mono(signal: &mut dyn Signal, start: Frame, out: &mut [f32]) {
    for (index, chunk) in out.chunks_mut(BLOCK_FRAMES).enumerate() {
        signal.render(start + (index * BLOCK_FRAMES) as Frame, chunk);
    }
}

/// Renders an [`AudioSignal`] as stereo frames for sinks: block buffers held once, ranges of any
/// length, a mono signal on both channels. Never allocates after construction.
pub struct Renderer {
    signal: AudioSignal,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Renderer {
    pub fn new(signal: AudioSignal) -> Self {
        Self {
            signal,
            left: vec![0.0; BLOCK_FRAMES],
            right: vec![0.0; BLOCK_FRAMES],
        }
    }

    pub fn layout(&self) -> Layout {
        self.signal.layout()
    }

    /// One block of frames `start .. start + len` (`len` at most [`BLOCK_FRAMES`]) as
    /// `(left, right)`; a mono signal returns its one channel twice.
    pub fn block(&mut self, start: Frame, len: usize) -> (&[f32], &[f32]) {
        match &mut self.signal {
            AudioSignal::Mono(signal) => {
                let out = &mut self.left[..len];
                signal.render(start, out);
                (out, out)
            }
            AudioSignal::Stereo(signal) => {
                let (left, right) = (&mut self.left[..len], &mut self.right[..len]);
                signal.render(start, left, right);
                (left, right)
            }
        }
    }

    /// Writes frames `start .. start + out.len()` as `[left, right]`.
    pub fn render_frames(&mut self, start: Frame, out: &mut [[f32; 2]]) {
        for (index, chunk) in out.chunks_mut(BLOCK_FRAMES).enumerate() {
            let (left, right) = self.block(start + (index * BLOCK_FRAMES) as Frame, chunk.len());
            for ((frame, &l), &r) in chunk.iter_mut().zip(left).zip(right) {
                *frame = [l, r];
            }
        }
    }

    /// Appends `frames` frames from `start` to `out`, interleaved by the signal's layout.
    pub fn render_interleaved(&mut self, start: Frame, frames: usize, out: &mut Vec<f32>) {
        let stereo = self.layout() == Layout::Stereo;
        let mut done = 0;
        while done < frames {
            let len = (frames - done).min(BLOCK_FRAMES);
            let (left, right) = self.block(start + done as Frame, len);
            if stereo {
                for (&l, &r) in left.iter().zip(right) {
                    out.push(l);
                    out.push(r);
                }
            } else {
                out.extend_from_slice(left);
            }
            done += len;
        }
    }

    pub fn into_signal(self) -> AudioSignal {
        self.signal
    }
}

/// Parameters for an offline render pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSpec {
    pub sample_rate: SampleRate,
    pub duration_secs: f64,
    pub channels: u16,
}

impl Default for RenderSpec {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::default(),
            duration_secs: 10.0,
            channels: 1,
        }
    }
}

impl RenderSpec {
    pub fn expected_frame_count(&self) -> usize {
        duration_to_frame_count(self.sample_rate, self.duration_secs)
    }
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("rendered frame count {actual} does not match expected {expected}")]
    FrameCountMismatch { expected: usize, actual: usize },
}

/// Renders a mono signal over the render interval.
pub fn sample_offline(spec: RenderSpec, signal: &mut dyn Signal) -> Result<Vec<f32>, RenderError> {
    let mut buffer = vec![0.0; spec.expected_frame_count()];
    render_mono(signal, 0, &mut buffer);
    Ok(buffer)
}

/// Renders `renderer`'s signal over the render interval as interleaved frames of its layout.
pub fn sample_offline_audio(
    spec: RenderSpec,
    renderer: &mut Renderer,
) -> Result<Vec<f32>, RenderError> {
    let frame_count = spec.expected_frame_count();
    let expected = frame_count * renderer.layout().channels();
    let mut out = Vec::with_capacity(expected);
    renderer.render_interleaved(0, frame_count, &mut out);
    if out.len() != expected {
        return Err(RenderError::FrameCountMismatch {
            expected,
            actual: out.len(),
        });
    }
    Ok(out)
}

/// Test and tooling helper: frames `start .. start + len` of `signal` as stereo frames, rendered
/// in one-block requests starting at `start`.
pub fn render_range(signal: &mut AudioSignal, start: Frame, len: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0; 2]; len];
    let mut left = vec![0.0; BLOCK_FRAMES];
    let mut right = vec![0.0; BLOCK_FRAMES];
    for (index, chunk) in out.chunks_mut(BLOCK_FRAMES).enumerate() {
        let at = start + (index * BLOCK_FRAMES) as Frame;
        let (l, r) = (&mut left[..chunk.len()], &mut right[..chunk.len()]);
        match signal {
            AudioSignal::Mono(mono) => {
                mono.render(at, l);
                r.copy_from_slice(l);
            }
            AudioSignal::Stereo(stereo) => stereo.render(at, l, r),
        }
        for ((frame, &a), &b) in chunk.iter_mut().zip(l.iter()).zip(r.iter()) {
            *frame = [a, b];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_dsp::sine;

    struct Sine440 {
        rate: f64,
    }

    impl Signal for Sine440 {
        fn render(&mut self, start: Frame, out: &mut [f32]) {
            for (k, sample) in out.iter_mut().enumerate() {
                *sample = sine(440.0, frame_secs(start + k as Frame, self.rate));
            }
        }
    }

    #[test]
    fn sample_offline_produces_expected_frame_count() {
        let spec = RenderSpec {
            sample_rate: SampleRate::RATE_44100,
            duration_secs: 1.0,
            channels: 1,
        };

        let buffer =
            sample_offline(spec, &mut Sine440 { rate: 44_100.0 }).expect("sample should succeed");
        assert_eq!(buffer.len(), 44_100);
    }

    #[test]
    fn ranges_of_any_length_split_into_blocks() {
        let mut whole = vec![0.0; 1000];
        render_mono(&mut Sine440 { rate: 44_100.0 }, 7, &mut whole);
        let mut renderer = Renderer::new(AudioSignal::Mono(Box::new(Sine440 { rate: 44_100.0 })));
        let mut frames = vec![[0.0; 2]; 1000];
        renderer.render_frames(7, &mut frames);
        for (frame, &value) in frames.iter().zip(&whole) {
            assert_eq!(*frame, [value, value]);
        }
    }
}
