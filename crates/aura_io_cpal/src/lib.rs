//! Real-time audio playback for Aura via CPAL.
//!
//! Streams a pure [`Sampler`] through the default output device. Device
//! availability depends on the host environment (WSL/devcontainer may have none).

use aura_render::Sampler;
use aura_sample::SampleRate;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream};
use dasp_sample::Sample;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;

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

/// Real-time playback via CPAL, sampling the same `Sampler::at(t)` path as offline render.
pub struct PlaybackEngine {
    stream: Option<Stream>,
    sample_clock: Arc<AtomicU64>,
    sample_rate_hz: f64,
}

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
            sample_clock: Arc::new(AtomicU64::new(0)),
            sample_rate_hz: 0.0,
        }
    }

    /// Whether an output stream is currently running.
    pub fn is_playing(&self) -> bool {
        self.stream.is_some()
    }

    /// Current playback time in seconds (from the shared sample clock).
    pub fn position_secs(&self) -> f64 {
        if self.sample_rate_hz <= 0.0 {
            return 0.0;
        }
        self.sample_clock.load(Ordering::Relaxed) as f64 / self.sample_rate_hz
    }

    /// Starts playback of `sampler` from `t = 0` on the default output device.
    ///
    /// The stream uses `sample_rate` so it matches the rate used to translate the
    /// graph. Stops any existing stream first. Fails clearly when no device is available.
    pub fn start(
        &mut self,
        sampler: Arc<dyn Sampler>,
        sample_rate: SampleRate,
    ) -> Result<(), PlaybackError> {
        self.start_at(sampler, sample_rate, 0.0)
    }

    /// Starts playback of `sampler` from `start_secs` on the default output device.
    ///
    /// Stops any existing stream first. Fails clearly when no device is available.
    pub fn start_at(
        &mut self,
        sampler: Arc<dyn Sampler>,
        sample_rate: SampleRate,
        start_secs: f64,
    ) -> Result<(), PlaybackError> {
        self.stop();

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(PlaybackError::NoOutputDevice)?;
        let default_config = device
            .default_output_config()
            .map_err(PlaybackError::DefaultConfig)?;

        let mut config: cpal::StreamConfig = default_config.clone().into();
        config.sample_rate = sample_rate.get();

        let rate = f64::from(sample_rate.get());
        let start_sample = if start_secs <= 0.0 || rate <= 0.0 {
            0
        } else {
            (start_secs * rate).round().max(0.0) as u64
        };
        let sample_clock = Arc::new(AtomicU64::new(start_sample));

        let stream = match default_config.sample_format() {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::I32 => {
                build_stream::<i32>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::U32 => {
                build_stream::<u32>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::I64 => {
                build_stream::<i64>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::U64 => {
                build_stream::<u64>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::U8 => {
                build_stream::<u8>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::I8 => {
                build_stream::<i8>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            SampleFormat::F64 => {
                build_stream::<f64>(&device, config, sampler, Arc::clone(&sample_clock))?
            }
            format => return Err(PlaybackError::UnsupportedFormat(format)),
        };

        stream.play().map_err(PlaybackError::PlayStream)?;
        self.sample_clock = sample_clock;
        self.sample_rate_hz = rate;
        self.stream = Some(stream);
        Ok(())
    }

    /// Stops playback if running. No-op when idle.
    pub fn stop(&mut self) {
        self.stream = None;
        self.sample_clock.store(0, Ordering::Relaxed);
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

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sampler: Arc<dyn Sampler>,
    sample_clock: Arc<AtomicU64>,
) -> Result<Stream, PlaybackError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let rate = f64::from(config.sample_rate);
    let err_fn = |err| eprintln!("aura_io_cpal stream error: {err}");

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    let clock = sample_clock.fetch_add(1, Ordering::Relaxed);
                    let t = clock as f64 / rate;
                    let mono = sampler.at(t);
                    let sample = T::from_sample(mono);
                    for out in frame.iter_mut() {
                        *out = sample;
                    }
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
