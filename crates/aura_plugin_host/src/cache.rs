//! A plugin as a seekable sampler: render forward once, keep the result.

use crate::{sort_events, Event, MonoStage, NoteEventSource, PluginHostError, Tail, BLOCK_FRAMES};
use aura_render::Sampler;
use aura_sample::SampleRate;
use aura_sequencing::LiveNote;
use std::sync::{Arc, Mutex};

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

/// A plugin instance rendered forward into a retained buffer and read as a [`Sampler`].
///
/// `at(t)` renders blocks until `t` is covered, then reads (with linear interpolation off the
/// frame grid). Everything rendered is kept for the life of the cache, so sampling out of order
/// or repeatedly costs nothing after the first pass and returns identical values. Note events
/// land on exact frames, and the plugin's latency is removed from the head of the output.
///
/// The plugin's inputs (`input` audio, the score) are pure, so they are sampled at whatever
/// frames each block needs.
pub struct RenderCache {
    state: Mutex<CacheState>,
    rate: f64,
}

struct CacheState {
    plugin_id: String,
    stage: MonoStage,
    events: EventSource,
    input: Option<Box<dyn Sampler>>,
    /// Output, latency removed: `rendered[k]` is the sample at frame `k`.
    rendered: Vec<f32>,
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
    in_buf: Vec<f32>,
    out_buf: Vec<f32>,
}

impl RenderCache {
    pub fn new(
        plugin_id: impl Into<String>,
        stage: MonoStage,
        events: EventSource,
        input: Option<Box<dyn Sampler>>,
        sample_rate: SampleRate,
    ) -> Self {
        let latency = u64::from(stage.latency_frames());
        let block = BLOCK_FRAMES as usize;
        Self {
            state: Mutex::new(CacheState {
                plugin_id: plugin_id.into(),
                stage,
                events,
                input,
                rendered: Vec::new(),
                plugin_frame: 0,
                latency,
                live_on_sent: false,
                live_off_frame: None,
                quiet_frames: 0,
                finished: false,
                failure: None,
                block_events: Vec::with_capacity(64),
                in_buf: vec![0.0; block],
                out_buf: vec![0.0; block],
            }),
            rate: f64::from(sample_rate.get()),
        }
    }

    /// The error that stopped rendering early, if any. Output after it is silence.
    pub fn failure(&self) -> Option<PluginHostError> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.failure.clone())
    }

    /// Frames rendered so far.
    pub fn rendered_frames(&self) -> usize {
        self.state.lock().map(|s| s.rendered.len()).unwrap_or(0)
    }
}

impl Sampler for RenderCache {
    fn at(&self, time_secs: f64) -> f32 {
        if time_secs.is_nan() || time_secs < 0.0 {
            return 0.0;
        }
        let position = time_secs * self.rate;
        let index = position.floor() as usize;
        let frac = (position - index as f64) as f32;
        let Ok(mut state) = self.state.lock() else {
            return 0.0;
        };
        let needed = if frac > 0.0 { index + 1 } else { index };
        state.ensure(needed, self.rate);
        let a = state.rendered.get(index).copied().unwrap_or(0.0);
        if frac > 0.0 {
            let b = state.rendered.get(index + 1).copied().unwrap_or(0.0);
            a + (b - a) * frac
        } else {
            a
        }
    }
}

impl CacheState {
    /// Renders until frame `index` exists or the plugin has finished.
    fn ensure(&mut self, index: usize, rate: f64) {
        while self.rendered.len() <= index && !self.finished {
            self.render_block(rate);
        }
    }

    fn render_block(&mut self, rate: f64) {
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

        let input = match &self.input {
            Some(sampler) => {
                for (k, sample) in self.in_buf.iter_mut().enumerate() {
                    *sample = sampler.at((start + k as u64) as f64 / rate);
                }
                Some(&self.in_buf[..])
            }
            None => None,
        };
        if let Err(error) = self
            .stage
            .process(input, &mut self.out_buf, &self.block_events)
        {
            eprintln!("aura: plugin `{}` stopped: {error}", self.plugin_id);
            self.failure = Some(error);
            self.finished = true;
            return;
        }
        self.plugin_frame += u64::from(frames);

        // Plugin frame p is output frame p - latency.
        let skip = self.latency.saturating_sub(start).min(u64::from(frames)) as usize;
        self.rendered.extend_from_slice(&self.out_buf[skip..]);

        let peak = self.out_buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        if peak >= QUIET_LEVEL {
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += u64::from(frames);
        }
        self.finished = self.is_done(rate);
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

    fn counter(latency: usize) -> MonoStage {
        MonoStage::new(
            Box::new(Counter {
                held: 0,
                latency,
                line: Vec::new(),
            }),
            BLOCK_FRAMES as usize,
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
            Some(Box::new(ConstantInput)),
            RATE,
        );
        assert_eq!(at_frame(&cache, 0), 0.5);
        assert_eq!(cache.at(20.0), 0.5);
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
