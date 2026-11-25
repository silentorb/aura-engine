//! A plugin as a seekable sampler: render forward once, keep the result.

use crate::{sort_events, Event, LayoutStage, NoteEventSource, PluginHostError, Tail, BLOCK_FRAMES};
use aura_render::{is_realtime_reader, AudioSampler, Layout, Sampler, StereoSampler};
use aura_sample::SampleRate;
use aura_sequencing::LiveNote;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

/// Where a [`RenderCache`]'s note events come from.
pub enum EventSource {
    /// No notes (an effect).
    None,
    /// A fully known score (deterministic mode).
    Score(NoteEventSource),
    /// One live note sounding from `t = 0` until its release (streaming per-voice fallback).
    Live(Arc<LiveNote>),
}

/// After its last note (and with no audio input), a plugin is done once it has been this quiet
/// for [`QUIET_SECS`], or after [`MAX_TAIL_SECS`] regardless.
const QUIET_LEVEL: f32 = 1e-6;
const QUIET_SECS: f64 = 0.25;
const MAX_TAIL_SECS: f64 = 30.0;

/// Samples per output chunk, and the most chunks a cache holds: about 3.4 hours of mono (1.7 of
/// stereo) at 44.1 kHz. Rendering stops (and the rest is silence) past that.
const CHUNK_FRAMES: usize = 1 << 16;
const MAX_CHUNKS: usize = 8192;
/// Blocks the render-ahead worker renders before letting go of the plugin, so inline readers
/// are not shut out for long.
const WORKER_BATCH_BLOCKS: u64 = 16;
/// How long the render-ahead worker waits when it is far enough ahead.
const WORKER_IDLE: Duration = Duration::from_millis(5);

/// Rendered output, readable without waiting on the renderer: append-only chunks of atomics.
/// One renderer at a time appends (under the cache's render lock); `len` publishes frames to
/// readers.
struct Output {
    chunks: Box<[OnceLock<Box<[AtomicU32]>>]>,
    len: AtomicUsize,
    finished: AtomicBool,
}

impl Output {
    fn new() -> Self {
        Self {
            chunks: (0..MAX_CHUNKS).map(|_| OnceLock::new()).collect(),
            len: AtomicUsize::new(0),
            finished: AtomicBool::new(false),
        }
    }

    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }

    fn get(&self, index: usize) -> Option<f32> {
        if index >= self.len() {
            return None;
        }
        let chunk = self.chunks[index / CHUNK_FRAMES].get()?;
        Some(f32::from_bits(chunk[index % CHUNK_FRAMES].load(Ordering::Relaxed)))
    }

    /// Appends `samples`. Returns false (writing nothing) once the cache is full.
    fn push(&self, samples: &[f32]) -> bool {
        let start = self.len.load(Ordering::Relaxed);
        if start + samples.len() > MAX_CHUNKS * CHUNK_FRAMES {
            return false;
        }
        for (offset, sample) in samples.iter().enumerate() {
            let index = start + offset;
            let chunk = self.chunks[index / CHUNK_FRAMES]
                .get_or_init(|| (0..CHUNK_FRAMES).map(|_| AtomicU32::new(0)).collect());
            chunk[index % CHUNK_FRAMES].store(sample.to_bits(), Ordering::Relaxed);
        }
        self.len.store(start + samples.len(), Ordering::Release);
        true
    }
}

/// A plugin instance rendered forward into a retained buffer and read as a [`Sampler`] (mono
/// layout) or [`StereoSampler`] (stereo layout; frames are stored interleaved).
///
/// `at(t)` renders blocks until `t` is covered, then reads (with linear interpolation off the
/// frame grid). Everything rendered is kept for the life of the cache, so sampling out of order
/// or repeatedly costs nothing after the first pass and returns identical values. Note events
/// land on exact frames, and the plugin's latency is removed from the head of the output.
///
/// The plugin's inputs (`input` audio, the score) are pure, so they are sampled at whatever
/// frames each block needs.
///
/// **Real-time reading.** A read from a [real-time reader](aura_render::mark_realtime_reader)
/// never renders or waits: a frame not rendered yet reads as silence and counts as an underrun.
/// [`Sampler::prepare`] renders ahead first and starts a background worker that keeps the
/// requested margin past the furthest real-time read. Every other reader renders inline. The
/// same frames render in the same order whichever thread renders them, so output is identical.
///
/// Clones share one cache.
#[derive(Clone)]
pub struct RenderCache {
    inner: Arc<CacheInner>,
}

struct CacheInner {
    rate: f64,
    /// Channels per stored frame (the stage's layout).
    channels: usize,
    output: Output,
    state: Mutex<CacheState>,
    /// Furthest frame a real-time reader has asked for.
    playhead: AtomicU64,
    /// Frames the worker keeps rendered past `playhead`.
    ahead: AtomicU64,
    underruns: AtomicU64,
    worker_started: AtomicBool,
}

struct CacheState {
    plugin_id: String,
    stage: LayoutStage,
    events: EventSource,
    input: Option<AudioSampler>,
    /// Frames the plugin has processed (its own clock, which runs `latency` ahead of output).
    plugin_frame: u64,
    latency: u64,
    live_on_sent: bool,
    /// Plugin frame the live note-off was sent at.
    live_off_frame: Option<u64>,
    quiet_frames: u64,
    finished: bool,
    failure: Option<PluginHostError>,
    block_events: Vec<Event>,
    /// One buffer per input channel.
    in_buf: Vec<Vec<f32>>,
    /// One buffer per output channel.
    out_buf: Vec<Vec<f32>>,
    /// Output frames interleaved for storage.
    interleaved: Vec<f32>,
}

impl RenderCache {
    pub fn new(
        plugin_id: impl Into<String>,
        stage: LayoutStage,
        events: EventSource,
        input: Option<AudioSampler>,
        sample_rate: SampleRate,
    ) -> Self {
        let latency = u64::from(stage.latency_frames());
        let block = BLOCK_FRAMES as usize;
        let channels = stage.layout().channels();
        let input_channels = input.as_ref().map_or(0, |input| input.layout().channels());
        Self {
            inner: Arc::new(CacheInner {
                rate: f64::from(sample_rate.get()),
                channels,
                output: Output::new(),
                state: Mutex::new(CacheState {
                    plugin_id: plugin_id.into(),
                    stage,
                    events,
                    input,
                    plugin_frame: 0,
                    latency,
                    live_on_sent: false,
                    live_off_frame: None,
                    quiet_frames: 0,
                    finished: false,
                    failure: None,
                    block_events: Vec::with_capacity(64),
                    in_buf: vec![vec![0.0; block]; input_channels],
                    out_buf: vec![vec![0.0; block]; channels],
                    interleaved: Vec::with_capacity(block * channels),
                }),
                playhead: AtomicU64::new(0),
                ahead: AtomicU64::new(0),
                underruns: AtomicU64::new(0),
                worker_started: AtomicBool::new(false),
            }),
        }
    }

    /// The error that stopped rendering early, if any. Output after it is silence.
    pub fn failure(&self) -> Option<PluginHostError> {
        self.inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.failure.clone())
    }

    /// Frames rendered so far.
    pub fn rendered_frames(&self) -> usize {
        self.inner.output.len() / self.inner.channels
    }

    /// The layout the cache renders.
    pub fn layout(&self) -> Layout {
        if self.inner.channels == 2 {
            Layout::Stereo
        } else {
            Layout::Mono
        }
    }

    /// Whether the render-ahead worker has been started (by [`Sampler::prepare`]).
    pub fn has_worker(&self) -> bool {
        self.inner.worker_started.load(Ordering::Acquire)
    }
}

impl CacheInner {
    fn frame_of(&self, secs: f64) -> u64 {
        (secs.max(0.0) * self.rate) as u64
    }

    /// Renders until `frames` frames exist or the plugin has finished.
    fn render_until(&self, frames: usize) {
        let samples = frames * self.channels;
        if self.output.len() >= samples || self.output.is_finished() {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.ensure(samples, self.rate, &self.output);
        }
    }

    /// Both channels of stereo frame `frame`, like [`CacheInner::sample`].
    fn stereo_frame(&self, frame: usize, realtime: bool) -> [f32; 2] {
        let index = frame * 2;
        if let (Some(left), Some(right)) = (self.output.get(index), self.output.get(index + 1)) {
            return [left, right];
        }
        if self.output.is_finished() {
            return [0.0; 2];
        }
        if realtime {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            return [0.0; 2];
        }
        self.render_until(frame + 1);
        [
            self.output.get(index).unwrap_or(0.0),
            self.output.get(index + 1).unwrap_or(0.0),
        ]
    }

    /// Channel `channel` of frame `frame`: rendered inline when needed, except for real-time
    /// readers.
    fn sample(&self, frame: usize, channel: usize, realtime: bool) -> f32 {
        let index = frame * self.channels + channel;
        if let Some(value) = self.output.get(index) {
            return value;
        }
        if self.output.is_finished() {
            return 0.0;
        }
        if realtime {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            return 0.0;
        }
        self.render_until(frame + 1);
        self.output.get(index).unwrap_or(0.0)
    }
}

/// Keeps a cache rendered `ahead` frames past its playhead until it finishes or is dropped.
fn render_ahead(cache: Weak<CacheInner>) {
    loop {
        let Some(inner) = cache.upgrade() else {
            return;
        };
        if inner.output.is_finished() {
            return;
        }
        let target = inner
            .playhead
            .load(Ordering::Relaxed)
            .saturating_add(inner.ahead.load(Ordering::Relaxed));
        let rendered = (inner.output.len() / inner.channels) as u64;
        if rendered < target {
            let batch = rendered + WORKER_BATCH_BLOCKS * u64::from(BLOCK_FRAMES);
            inner.render_until(target.min(batch) as usize);
        } else {
            drop(inner);
            std::thread::sleep(WORKER_IDLE);
        }
    }
}

impl RenderCache {
    /// Both channels of a stereo cache at `time_secs`; a miss counts as one underrun.
    fn read_frame(&self, time_secs: f64) -> [f32; 2] {
        if time_secs.is_nan() || time_secs < 0.0 {
            return [0.0; 2];
        }
        let inner = &self.inner;
        let position = time_secs * inner.rate;
        let index = position.floor() as usize;
        let frac = (position - index as f64) as f32;
        let realtime = is_realtime_reader();
        if realtime {
            inner.playhead.fetch_max(index as u64, Ordering::Relaxed);
        }
        let a = inner.stereo_frame(index, realtime);
        if frac > 0.0 {
            let b = inner.stereo_frame(index + 1, realtime);
            [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
        } else {
            a
        }
    }

    /// Channel `channel` at `time_secs`, linearly interpolated off the frame grid.
    fn read(&self, channel: usize, time_secs: f64) -> f32 {
        if time_secs.is_nan() || time_secs < 0.0 {
            return 0.0;
        }
        let inner = &self.inner;
        let position = time_secs * inner.rate;
        let index = position.floor() as usize;
        let frac = (position - index as f64) as f32;
        let realtime = is_realtime_reader();
        if realtime {
            inner.playhead.fetch_max(index as u64, Ordering::Relaxed);
        }
        let a = inner.sample(index, channel, realtime);
        if frac > 0.0 {
            let b = inner.sample(index + 1, channel, realtime);
            a + (b - a) * frac
        } else {
            a
        }
    }
}

impl StereoSampler for RenderCache {
    fn frame(&self, time_secs: f64) -> [f32; 2] {
        if self.inner.channels == 1 {
            let value = self.read(0, time_secs);
            return [value, value];
        }
        self.read_frame(time_secs)
    }

    fn channel(&self, channel: usize, time_secs: f64) -> f32 {
        self.read(channel.min(self.inner.channels - 1), time_secs)
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        RenderCache::prepare(self, start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        RenderCache::underruns(self)
    }
}

impl Sampler for RenderCache {
    /// The mono signal; a stereo cache reads as the average of its channels.
    fn at(&self, time_secs: f64) -> f32 {
        if self.inner.channels == 1 {
            self.read(0, time_secs)
        } else {
            (self.read(0, time_secs) + self.read(1, time_secs)) * 0.5
        }
    }

    fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        RenderCache::prepare(self, start_secs, ahead_secs);
    }

    fn underruns(&self) -> u64 {
        RenderCache::underruns(self)
    }
}

impl RenderCache {
    /// Renders `ahead_secs` past `start_secs` now and starts the render-ahead worker (see
    /// [`Sampler::prepare`]).
    pub fn prepare(&self, start_secs: f64, ahead_secs: f64) {
        let inner = &self.inner;
        let start = inner.frame_of(start_secs);
        let ahead = inner.frame_of(ahead_secs);
        inner.ahead.fetch_max(ahead, Ordering::Relaxed);
        inner.playhead.fetch_max(start, Ordering::Relaxed);
        inner.render_until((start + ahead) as usize);
        if inner.output.is_finished() || inner.worker_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let cache = Arc::downgrade(inner);
        if let Err(error) = std::thread::Builder::new()
            .name("aura-plugin-render".into())
            .spawn(move || render_ahead(cache))
        {
            // Without a worker, real-time reads past the prepared range read as silence
            // (counted as underruns); rendering still happens for every other reader.
            eprintln!("aura: could not start plugin render-ahead: {error}");
            inner.worker_started.store(false, Ordering::Release);
        }
    }

    /// Real-time reads that found nothing rendered (see [`Sampler::underruns`]).
    pub fn underruns(&self) -> u64 {
        self.inner.underruns.load(Ordering::Relaxed)
    }
}

impl CacheState {
    /// Renders until `frames` frames exist or the plugin has finished.
    fn ensure(&mut self, frames: usize, rate: f64, output: &Output) {
        while output.len() < frames && !self.finished {
            self.render_block(rate, output);
        }
    }

    fn render_block(&mut self, rate: f64, output: &Output) {
        let frames = BLOCK_FRAMES;
        let start = self.plugin_frame;
        self.block_events.clear();
        match &self.events {
            EventSource::None => {}
            EventSource::Score(notes) => notes.events_in(start, frames, &mut self.block_events),
            EventSource::Live(live) => {
                if !self.live_on_sent {
                    self.block_events.push(Event::NoteOn {
                        frame: 0,
                        note_id: 0,
                        key: live.semitone.0,
                        velocity: live.velocity,
                    });
                    self.live_on_sent = true;
                }
                if self.live_off_frame.is_none() && live.released_after().is_some() {
                    // The release is stamped while earlier frames are already rendered; it
                    // takes effect at this block, the earliest the plugin can still hear it.
                    self.block_events.push(Event::NoteOff {
                        frame: 0,
                        note_id: 0,
                        key: live.semitone.0,
                    });
                    self.live_off_frame = Some(start);
                }
            }
        }
        sort_events(&mut self.block_events);

        let block = frames as usize;
        match &self.input {
            Some(AudioSampler::Mono(sampler)) => {
                for (k, sample) in self.in_buf[0].iter_mut().enumerate() {
                    *sample = sampler.at((start + k as u64) as f64 / rate);
                }
            }
            Some(AudioSampler::Stereo(sampler)) => {
                for k in 0..block {
                    let [left, right] = sampler.frame((start + k as u64) as f64 / rate);
                    self.in_buf[0][k] = left;
                    self.in_buf[1][k] = right;
                }
            }
            None => {}
        }
        let inputs: Vec<&[f32]> = self.in_buf.iter().map(Vec::as_slice).collect();
        let input = self.input.as_ref().map(|_| inputs.as_slice());
        let mut outputs: Vec<&mut [f32]> =
            self.out_buf.iter_mut().map(Vec::as_mut_slice).collect();
        if let Err(error) = self
            .stage
            .process(input, &mut outputs, block, &self.block_events)
        {
            eprintln!("aura: plugin `{}` stopped: {error}", self.plugin_id);
            self.failure = Some(error);
            self.finished = true;
            output.finish();
            return;
        }
        self.plugin_frame += u64::from(frames);

        // Plugin frame p is output frame p - latency.
        let skip = self.latency.saturating_sub(start).min(u64::from(frames)) as usize;
        self.interleaved.clear();
        for frame in skip..block {
            for channel in &self.out_buf {
                self.interleaved.push(channel[frame]);
            }
        }
        if !output.push(&self.interleaved) {
            self.finished = true;
            output.finish();
            return;
        }

        let peak = self
            .out_buf
            .iter()
            .flatten()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        if peak >= QUIET_LEVEL {
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += u64::from(frames);
        }
        self.finished = self.is_done(rate);
        if self.finished {
            output.finish();
        }
    }

    /// Whether nothing more can sound: no audio input, no notes left, and the tail is over.
    fn is_done(&mut self, rate: f64) -> bool {
        if self.input.is_some() {
            return false;
        }
        let last_event = match &self.events {
            EventSource::None => Some(0),
            EventSource::Score(notes) => notes.end_frame(),
            EventSource::Live(_) => self.live_off_frame,
        };
        let Some(last_event) = last_event else {
            return false;
        };
        if self.plugin_frame < last_event {
            return false;
        }
        let since = self.plugin_frame - last_event;
        let cap = (MAX_TAIL_SECS * rate) as u64;
        if since >= cap {
            return true;
        }
        let quiet = self.quiet_frames as f64 / rate >= QUIET_SECS;
        match self.stage.processor_mut().tail() {
            Tail::Frames(tail) => since >= tail + self.latency && quiet,
            Tail::Infinite | Tail::Unknown => quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, Processor};
    use aura_sequencing::{NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature};

    const RATE: SampleRate = SampleRate::RATE_44100;

    /// Outputs the count of held notes on every frame, plus its input, delayed by `latency`.
    struct Counter {
        held: i32,
        latency: usize,
        line: Vec<f32>,
    }

    impl Processor for Counter {
        fn layout(&self) -> ChannelLayout {
            ChannelLayout {
                inputs: 1,
                outputs: 2,
            }
        }
        fn latency_frames(&self) -> u32 {
            self.latency as u32
        }
        fn tail(&mut self) -> Tail {
            Tail::Frames(0)
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [&mut [f32]],
            events: &[Event],
        ) -> Result<(), PluginHostError> {
            let mut next = events.iter().peekable();
            for frame in 0..outputs[0].len() {
                while let Some(event) = next.next_if(|e| e.frame() as usize == frame) {
                    match event {
                        Event::NoteOn { .. } => self.held += 1,
                        Event::NoteOff { .. } => self.held -= 1,
                    }
                }
                self.line.push(self.held as f32 + inputs[0][frame]);
                let value = if self.line.len() > self.latency {
                    self.line.remove(0)
                } else {
                    0.0
                };
                for channel in outputs.iter_mut() {
                    channel[frame] = value;
                }
            }
            Ok(())
        }
    }

    fn counter(latency: usize) -> LayoutStage {
        LayoutStage::new(
            Box::new(Counter {
                held: 0,
                latency,
                line: Vec::new(),
            }),
            BLOCK_FRAMES as usize,
            Layout::Mono,
        )
    }

    fn score_events() -> EventSource {
        // 120 bpm: note from 0.5 s to 1.0 s.
        let score = Score::new(
            TimeSignature::default(),
            Tempo::new(120.0).unwrap(),
            vec![NoteEvent::new(Semitone(60), 1.0, 1.0)],
        );
        EventSource::Score(NoteEventSource::from_score_signal(
            &ScoreSignal::finite(score),
            RATE,
        ))
    }

    fn at_frame(cache: &RenderCache, frame: u64) -> f32 {
        cache.at(frame as f64 / 44100.0)
    }

    #[test]
    fn notes_land_on_exact_frames_with_latency_removed() {
        for latency in [0, 64, 300] {
            let cache = RenderCache::new("counter", counter(latency), score_events(), None, RATE);
            assert_eq!(at_frame(&cache, 22049), 0.0, "latency {latency}");
            assert_eq!(at_frame(&cache, 22050), 1.0, "latency {latency}");
            assert_eq!(at_frame(&cache, 44099), 1.0, "latency {latency}");
            assert_eq!(at_frame(&cache, 44100), 0.0, "latency {latency}");
        }
    }

    #[test]
    fn out_of_order_sampling_matches_a_sequential_pass() {
        let sequential = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        let expected: Vec<f32> = (0..50_000).map(|f| at_frame(&sequential, f)).collect();
        let sparse = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        for frame in (0..50_000u64).rev().step_by(997) {
            assert_eq!(at_frame(&sparse, frame), expected[frame as usize]);
        }
        for frame in [30_000u64, 22_050, 0, 49_999] {
            assert_eq!(at_frame(&sparse, frame), expected[frame as usize]);
        }
    }

    #[test]
    fn finishes_after_the_last_note_and_stays_silent() {
        let cache = RenderCache::new("counter", counter(0), score_events(), None, RATE);
        assert_eq!(cache.at(10.0), 0.0);
        let rendered = cache.rendered_frames();
        assert!(rendered < 44100 + 44100, "stopped rendering, got {rendered} frames");
        assert!(cache.failure().is_none());
    }

    struct ConstantInput;
    impl Sampler for ConstantInput {
        fn at(&self, _t: f64) -> f32 {
            0.5
        }
    }

    #[test]
    fn effect_reads_its_input_and_never_finishes_early() {
        let cache = RenderCache::new(
            "counter",
            counter(10),
            EventSource::None,
            Some(AudioSampler::Mono(Box::new(ConstantInput))),
            RATE,
        );
        assert_eq!(at_frame(&cache, 0), 0.5);
        assert_eq!(cache.at(20.0), 0.5);
    }

    /// Runs `f` on a fresh thread marked as a real-time reader.
    fn on_realtime_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        std::thread::spawn(move || {
            aura_render::mark_realtime_reader();
            f()
        })
        .join()
        .expect("real-time thread")
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn realtime_reads_never_render_and_count_underruns() {
        let cache = RenderCache::new("counter", counter(0), score_events(), None, RATE);
        let reader = cache.clone();
        let value = on_realtime_thread(move || at_frame(&reader, 30_000));
        assert_eq!(value, 0.0, "nothing prepared reads as silence");
        assert_eq!(cache.rendered_frames(), 0, "the real-time reader rendered nothing");
        assert_eq!(cache.underruns(), 1);
        // Any other reader still renders inline.
        assert_eq!(at_frame(&cache, 30_000), 1.0);
    }

    #[test]
    fn prepare_renders_ahead_and_the_worker_follows_the_playhead() {
        let reference = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        let expected: Vec<f32> = (0..60_000).map(|f| at_frame(&reference, f)).collect();

        let cache = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        // Prepare 0.2 s from 0.5 s: frames up to 30_870 render now, on this thread.
        cache.prepare(0.5, 0.2);
        assert!(cache.rendered_frames() >= 30_870);
        assert!(cache.has_worker());
        let reader = cache.clone();
        let (prepared, underruns) = on_realtime_thread(move || {
            let values: Vec<f32> = (22_000..30_000).map(|f| at_frame(&reader, f)).collect();
            (values, reader.underruns())
        });
        assert_eq!(underruns, 0);
        assert_eq!(prepared, expected[22_000..30_000]);

        // A real-time read further on moves the playhead; the worker renders past it.
        let reader = cache.clone();
        on_realtime_thread(move || at_frame(&reader, 40_000));
        wait_until(|| cache.rendered_frames() >= 40_000 + 8_820);
        let reader = cache.clone();
        let later = on_realtime_thread(move || {
            (40_000..48_000).map(|f| at_frame(&reader, f)).collect::<Vec<f32>>()
        });
        assert_eq!(later, expected[40_000..48_000]);
    }

    /// A processor that reports when it is dropped.
    struct Dropped(Arc<AtomicBool>, Counter);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl Processor for Dropped {
        fn layout(&self) -> ChannelLayout {
            self.1.layout()
        }
        fn latency_frames(&self) -> u32 {
            0
        }
        fn tail(&mut self) -> Tail {
            Tail::Frames(0)
        }
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [&mut [f32]],
            events: &[Event],
        ) -> Result<(), PluginHostError> {
            self.1.process(inputs, outputs, events)
        }
    }

    #[test]
    fn dropping_a_prepared_cache_stops_its_worker_and_frees_the_plugin() {
        let dropped = Arc::new(AtomicBool::new(false));
        let processor = Dropped(
            Arc::clone(&dropped),
            Counter {
                held: 0,
                latency: 0,
                line: Vec::new(),
            },
        );
        // An effect never finishes, so its worker would run forever if it held the cache.
        let cache = RenderCache::new(
            "counter",
            LayoutStage::new(Box::new(processor), BLOCK_FRAMES as usize, Layout::Mono),
            EventSource::None,
            Some(AudioSampler::Mono(Box::new(ConstantInput))),
            RATE,
        );
        cache.prepare(0.0, 0.1);
        assert!(cache.has_worker());
        drop(cache);
        wait_until(|| dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn live_note_holds_until_release() {
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, Default::default()));
        let cache = RenderCache::new(
            "counter",
            counter(0),
            EventSource::Live(Arc::clone(&live)),
            None,
            RATE,
        );
        assert_eq!(at_frame(&cache, 0), 1.0);
        assert_eq!(at_frame(&cache, 1000), 1.0);
        live.release(1000.0 / 44100.0);
        // The release takes effect at the next unrendered block.
        let next_block = cache.rendered_frames() as u64;
        assert_eq!(at_frame(&cache, next_block), 0.0);
    }
}
