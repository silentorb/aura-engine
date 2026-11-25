//! Real-time audio playback for Aura via CPAL.
//!
//! Deterministic mode plays an [`AudioSignal`] (mono or stereo) on the default output device
//! ([`PlaybackEngine`]); streaming mode pulls blocks from a [`BlockSource`]
//! ([`StreamingOutput`]). Device availability depends on the host environment
//! (WSL/devcontainer may have none).

mod transport;

use aura_render::{AudioSignal, BlockSource};
use aura_sample::SampleRate;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream};
use dasp_sample::Sample;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use transport::RenderAhead;

/// Errors from opening or running a CPAL output stream.
#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("no default audio output device")]
    NoOutputDevice,
    #[error("failed to query default output config: {0}")]
    DefaultConfig(#[source] cpal::Error),
    #[error("unsupported sample format: {0:?}")]
    UnsupportedFormat(SampleFormat),
    #[error("failed to build output stream: {0}")]
    BuildStream(#[source] cpal::Error),
    #[error("failed to start output stream: {0}")]
    PlayStream(#[source] cpal::Error),
}

/// Deterministic transport playback via CPAL: the same frames as an offline render.
///
/// A render thread runs the signal [`PREPARE_AHEAD_SECS`] ahead of the playhead into a ring
/// buffer; the audio callback only copies frames out of it. If the ring runs dry the callback
/// plays silence, counts the missing frames as underruns, and the playhead waits for the
/// renderer (it reports what was heard).
pub struct PlaybackEngine {
    stream: Option<Stream>,
    render: Option<RenderAhead>,
    /// Frames played since the start frame.
    played: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
    start_frame: u64,
    sample_rate_hz: f64,
}

/// How far ahead of the playhead the transport renders. [`PlaybackEngine::start_at`] renders
/// this much before the stream opens.
pub const PREPARE_AHEAD_SECS: f64 = 2.0;

impl std::fmt::Debug for PlaybackEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackEngine")
            .field("playing", &self.is_playing())
            .field("position_secs", &self.position_secs())
            .finish()
    }
}

impl PlaybackEngine {
    pub fn new() -> Self {
        Self {
            stream: None,
            render: None,
            played: Arc::new(AtomicU64::new(0)),
            underruns: Arc::new(AtomicU64::new(0)),
            start_frame: 0,
            sample_rate_hz: 0.0,
        }
    }

    /// Whether an output stream is currently running.
    pub fn is_playing(&self) -> bool {
        self.stream.is_some()
    }

    /// Current playback time in seconds: the start position plus the frames played.
    pub fn position_secs(&self) -> f64 {
        if self.sample_rate_hz <= 0.0 {
            return 0.0;
        }
        (self.start_frame + self.played.load(Ordering::Relaxed)) as f64 / self.sample_rate_hz
    }

    /// Starts playback of `signal` from frame 0 on the default output device.
    ///
    /// The stream uses `sample_rate` so it matches the rate used to translate the
    /// graph. Stops any existing stream first. Fails clearly when no device is available.
    pub fn start(&mut self, signal: AudioSignal, sample_rate: SampleRate) -> Result<(), PlaybackError> {
        self.start_at(signal, sample_rate, 0.0)
    }

    /// Starts playback of `signal` from `start_secs` on the default output device.
    ///
    /// Renders [`PREPARE_AHEAD_SECS`] on this thread first (a plugin far into a piece renders
    /// up to there), then opens the stream. Stops any existing stream first. Fails clearly when
    /// no device is available.
    pub fn start_at(
        &mut self,
        signal: AudioSignal,
        sample_rate: SampleRate,
        start_secs: f64,
    ) -> Result<(), PlaybackError> {
        self.stop();

        let rate = f64::from(sample_rate.get());
        let start_frame = if start_secs <= 0.0 || rate <= 0.0 {
            0
        } else {
            (start_secs * rate).round().max(0.0) as u64
        };
        let ahead_frames = (PREPARE_AHEAD_SECS * rate).ceil() as usize;
        let render = RenderAhead::start(signal, start_frame, ahead_frames);
        let ring = Arc::clone(&render.ring);
        let played = Arc::new(AtomicU64::new(0));
        let underruns = Arc::new(AtomicU64::new(0));
        let (played_in, underruns_in) = (Arc::clone(&played), Arc::clone(&underruns));

        let stream = open_output(sample_rate, move |block: &mut [[f32; 2]]| {
            let count = ring.pop_into(block);
            if count < block.len() {
                block[count..].fill([0.0; 2]);
                underruns_in.fetch_add((block.len() - count) as u64, Ordering::Relaxed);
            }
            played_in.fetch_add(count as u64, Ordering::Relaxed);
        })?;

        self.played = played;
        self.underruns = underruns;
        self.start_frame = start_frame;
        self.sample_rate_hz = rate;
        self.stream = Some(stream);
        self.render = Some(render);
        Ok(())
    }

    /// Frames of this playback that were not rendered in time (played as silence).
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Stops playback if running. No-op when idle.
    pub fn stop(&mut self) {
        // The callback stops before its ring's renderer does.
        self.stream = None;
        self.render = None;
        self.played.store(0, Ordering::Relaxed);
        self.start_frame = 0;
        self.sample_rate_hz = 0.0;
    }
}

impl Default for PlaybackEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Streaming-mode output: a persistent stream pulling consecutive blocks from a
/// [`BlockSource`] (typically a voice mixer fed note events from a control thread).
///
/// Opened once and kept open, so new voices start on the next device block. Runs independently
/// of [`PlaybackEngine`]; both may play at once. Dropping it closes the stream.
pub struct StreamingOutput {
    _stream: Stream,
}

impl std::fmt::Debug for StreamingOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingOutput").finish_non_exhaustive()
    }
}

impl StreamingOutput {
    /// Opens the default output device and starts pulling from `source`.
    pub fn open(
        mut source: Box<dyn BlockSource>,
        sample_rate: SampleRate,
    ) -> Result<Self, PlaybackError> {
        let stream = open_output(sample_rate, move |block: &mut [[f32; 2]]| {
            source.render_block(block);
        })?;
        Ok(Self { _stream: stream })
    }
}

/// Opens and starts an output stream on the default device at `sample_rate`. `fill` writes each
/// device block as stereo frames: a mono device gets their average, a device with more channels
/// gets left and right on its first two and silence on the rest.
fn open_output<F>(sample_rate: SampleRate, fill: F) -> Result<Stream, PlaybackError>
where
    F: FnMut(&mut [[f32; 2]]) + Send + 'static,
{
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(PlaybackError::NoOutputDevice)?;
    let default_config = device
        .default_output_config()
        .map_err(PlaybackError::DefaultConfig)?;

    let mut config: cpal::StreamConfig = default_config.into();
    config.sample_rate = sample_rate.get();

    let stream = match default_config.sample_format() {
        SampleFormat::F32 => build_stream::<f32, F>(&device, config, fill)?,
        SampleFormat::I16 => build_stream::<i16, F>(&device, config, fill)?,
        SampleFormat::U16 => build_stream::<u16, F>(&device, config, fill)?,
        SampleFormat::I32 => build_stream::<i32, F>(&device, config, fill)?,
        SampleFormat::U32 => build_stream::<u32, F>(&device, config, fill)?,
        SampleFormat::I64 => build_stream::<i64, F>(&device, config, fill)?,
        SampleFormat::U64 => build_stream::<u64, F>(&device, config, fill)?,
        SampleFormat::U8 => build_stream::<u8, F>(&device, config, fill)?,
        SampleFormat::I8 => build_stream::<i8, F>(&device, config, fill)?,
        SampleFormat::F64 => build_stream::<f64, F>(&device, config, fill)?,
        format => return Err(PlaybackError::UnsupportedFormat(format)),
    };
    stream.play().map_err(PlaybackError::PlayStream)?;
    Ok(stream)
}

/// Scratch frames reserved up front so typical device blocks never allocate.
const SCRATCH_FRAMES: usize = 8192;

fn build_stream<T, F>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut fill: F,
) -> Result<Stream, PlaybackError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
    F: FnMut(&mut [[f32; 2]]) + Send + 'static,
{
    let channels = (config.channels as usize).max(1);
    let err_fn = |err| eprintln!("aura_io_cpal stream error: {err}");
    let mut frames_buf = Vec::with_capacity(SCRATCH_FRAMES);

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                frames_buf.resize(frames, [0.0; 2]);
                fill(&mut frames_buf);
                for (frame, &[left, right]) in data.chunks_mut(channels).zip(frames_buf.iter()) {
                    write_frame(frame, left, right);
                }
            },
            err_fn,
            None,
        )
        .map_err(PlaybackError::BuildStream)
}

/// Converts an `f32` sample to the target CPAL sample type.
pub fn convert_sample<T>(sample: f32) -> T
where
    T: Sample + dasp_sample::FromSample<f32>,
{
    T::from_sample(sample)
}

/// Returns whether the given CPAL sample format uses floating-point encoding.
pub fn is_float_format(format: SampleFormat) -> bool {
    format.is_float()
}

/// Writes one stereo frame onto a device frame of any channel count.
fn write_frame<T: SizedSample + FromSample<f32>>(frame: &mut [T], left: f32, right: f32) {
    match frame {
        [only] => *only = T::from_sample((left + right) * 0.5),
        [first, second, rest @ ..] => {
            *first = T::from_sample(left);
            *second = T::from_sample(right);
            for out in rest {
                *out = T::from_sample(0.0f32);
            }
        }
        [] => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_converts_to_i16() {
        let sample: i16 = convert_sample(0.5);
        assert!(sample > 0);
    }

    #[test]
    fn f32_converts_to_f32() {
        let sample: f32 = convert_sample(0.5);
        assert!((sample - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn float_format_detection() {
        assert!(is_float_format(SampleFormat::F32));
        assert!(!is_float_format(SampleFormat::I16));
    }

    #[test]
    fn frames_map_onto_device_channels() {
        let mut mono = [0.0f32];
        write_frame(&mut mono, 1.0, 0.0);
        assert_eq!(mono, [0.5]);
        let mut surround = [9.0f32; 4];
        write_frame(&mut surround, 1.0, -1.0);
        assert_eq!(surround, [1.0, -1.0, 0.0, 0.0]);
    }

    #[test]
    fn engine_starts_idle() {
        let engine = PlaybackEngine::new();
        assert!(!engine.is_playing());
        assert_eq!(engine.position_secs(), 0.0);
    }

    #[test]
    fn stop_while_idle_is_noop() {
        let mut engine = PlaybackEngine::new();
        engine.stop();
        assert!(!engine.is_playing());
    }
}
