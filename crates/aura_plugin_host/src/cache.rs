//! A plugin as a block node: render forward once, keep the result, answer any range from it.

use crate::{sort_events, Event, LayoutStage, NoteEventSource, PluginHostError, Tail, BLOCK_FRAMES};
use aura_render::{AudioSignal, Frame, Layout, Signal, StereoSignal};
use aura_sample::SampleRate;
use aura_sequencing::LiveNote;
use std::sync::Arc;

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

/// Most samples (all channels) a cache retains: about 3.4 hours of mono (1.7 of stereo) at
/// 44.1 kHz. Rendering stops (and the rest is silence) past that.
const MAX_RETAINED_SAMPLES: usize = 1 << 29;

/// A plugin instance rendered forward on its own block grid from frame 0 into retained output,
/// read as a [`Signal`] (mono layout) or [`StereoSignal`] (stereo layout).
///
/// A plugin's state is essential: it can only be run forward and cannot be snapshotted. The
/// cache keeps the deterministic contract anyway. Rendering a range first renders the plugin up
/// to the end of that range (if it has not got there yet), then copies the range out of what it
/// kept. Everything rendered is kept for the life of the cache, so ranges in any order, and
/// repeated ranges, return identical frames. Note events land on exact frames, and the plugin's
/// latency is removed from the head of the output.
///
/// The plugin's audio input is a deterministic signal, rendered on the plugin's grid as each
/// block needs it.
pub struct RenderCache {
    plugin_id: String,
    stage: LayoutStage,
    events: EventSource,
    input: Option<AudioSignal>,
    rate: f64,
    /// Output channels (the stage's layout).
    channels: usize,
    /// Rendered output frames, one buffer per channel.
    retained: Vec<Vec<f32>>,
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
    /// Scratch for the other channel when a stereo cache is read as mono.
    mix: Vec<f32>,
}

impl RenderCache {
    pub fn new(
        plugin_id: impl Into<String>,
        stage: LayoutStage,
        events: EventSource,
        input: Option<AudioSignal>,
        sample_rate: SampleRate,
    ) -> Self {
        let latency = u64::from(stage.latency_frames());
        let block = BLOCK_FRAMES as usize;
        let channels = stage.layout().channels();
        let input_channels = input.as_ref().map_or(0, |input| input.layout().channels());
        Self {
            plugin_id: plugin_id.into(),
            stage,
            events,
            input,
            rate: f64::from(sample_rate.get()),
            channels,
            retained: vec![Vec::new(); channels],
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
            mix: vec![0.0; aura_render::BLOCK_FRAMES],
        }
    }

    /// The error that stopped rendering early, if any. Output after it is silence.
    pub fn failure(&self) -> Option<PluginHostError> {
        self.failure.clone()
    }

    /// Output frames rendered so far.
    pub fn rendered_frames(&self) -> usize {
        self.retained[0].len()
    }

    /// The layout the cache renders.
    pub fn layout(&self) -> Layout {
        if self.channels == 2 {
            Layout::Stereo
        } else {
            Layout::Mono
        }
    }

    /// Renders until `frames` output frames exist or the plugin has finished.
    fn ensure(&mut self, frames: usize) {
        while self.rendered_frames() < frames && !self.finished {
            self.render_block();
        }
    }

    /// Copies channel `channel` of frames `start ..` into `out`; silence past what was rendered.
    fn copy(&self, channel: usize, start: Frame, out: &mut [f32]) {
        let retained = &self.retained[channel];
        let start = usize::try_from(start).unwrap_or(usize::MAX).min(retained.len());
        let available = (retained.len() - start).min(out.len());
        out[..available].copy_from_slice(&retained[start..start + available]);
        out[available..].fill(0.0);
    }

    fn end_of(start: Frame, len: usize) -> usize {
        usize::try_from(start)
            .unwrap_or(usize::MAX)
            .saturating_add(len)
    }

    fn render_block(&mut self) {
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
        match &mut self.input {
            Some(AudioSignal::Mono(signal)) => signal.render(start, &mut self.in_buf[0][..block]),
            Some(AudioSignal::Stereo(signal)) => {
                let (left, right) = self.in_buf.split_at_mut(1);
                signal.render(start, &mut left[0][..block], &mut right[0][..block]);
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
            return;
        }
        self.plugin_frame += u64::from(frames);

        // Plugin frame p is output frame p - latency.
        let skip = self.latency.saturating_sub(start).min(u64::from(frames)) as usize;
        if (self.rendered_frames() + block - skip) * self.channels > MAX_RETAINED_SAMPLES {
            self.finished = true;
            return;
        }
        for (retained, rendered) in self.retained.iter_mut().zip(&self.out_buf) {
            retained.extend_from_slice(&rendered[skip..block]);
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
        self.finished = self.is_done();
    }

    /// Whether nothing more can sound: no audio input, no notes left, and the tail is over.
    fn is_done(&mut self) -> bool {
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
        let cap = (MAX_TAIL_SECS * self.rate) as u64;
        if since >= cap {
            return true;
        }
        let quiet = self.quiet_frames as f64 / self.rate >= QUIET_SECS;
        match self.stage.processor_mut().tail() {
            Tail::Frames(tail) => since >= tail + self.latency && quiet,
            Tail::Infinite | Tail::Unknown => quiet,
        }
    }
}

impl StereoSignal for RenderCache {
    /// A mono cache reads the same on both channels.
    fn render(&mut self, start: Frame, left: &mut [f32], right: &mut [f32]) {
        self.ensure(Self::end_of(start, left.len()));
        self.copy(0, start, left);
        self.copy(self.channels - 1, start, right);
    }
}

impl Signal for RenderCache {
    /// The mono signal; a stereo cache reads as the average of its channels.
    fn render(&mut self, start: Frame, out: &mut [f32]) {
        self.ensure(Self::end_of(start, out.len()));
        self.copy(0, start, out);
        if self.channels == 2 {
            let mut mix = std::mem::take(&mut self.mix);
            let other = &mut mix[..out.len()];
            self.copy(1, start, other);
            for (sample, &right) in out.iter_mut().zip(other.iter()) {
                *sample = (*sample + right) * 0.5;
            }
            self.mix = mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, Processor};
    use aura_render::{Constant, Frame};
    use aura_sequencing::{NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature};
    use std::sync::atomic::{AtomicBool, Ordering};

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

    fn at_frame(cache: &mut RenderCache, frame: Frame) -> f32 {
        let mut out = [0.0];
        Signal::render(cache, frame, &mut out);
        out[0]
    }

    #[test]
    fn notes_land_on_exact_frames_with_latency_removed() {
        for latency in [0, 64, 300] {
            let mut cache =
                RenderCache::new("counter", counter(latency), score_events(), None, RATE);
            assert_eq!(at_frame(&mut cache, 22049), 0.0, "latency {latency}");
            assert_eq!(at_frame(&mut cache, 22050), 1.0, "latency {latency}");
            assert_eq!(at_frame(&mut cache, 44099), 1.0, "latency {latency}");
            assert_eq!(at_frame(&mut cache, 44100), 0.0, "latency {latency}");
        }
    }

    #[test]
    fn out_of_order_ranges_match_a_sequential_pass() {
        let mut sequential = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        let mut expected = vec![0.0; 50_000];
        aura_render::render_mono(&mut sequential, 0, &mut expected);
        let mut sparse = RenderCache::new("counter", counter(64), score_events(), None, RATE);
        for frame in (0..49_900u64).rev().step_by(997) {
            let mut out = [0.0; 100];
            Signal::render(&mut sparse, frame, &mut out);
            assert_eq!(out[..], expected[frame as usize..frame as usize + 100]);
        }
        for frame in [30_000u64, 22_050, 0, 49_999] {
            assert_eq!(at_frame(&mut sparse, frame), expected[frame as usize]);
        }
    }

    #[test]
    fn finishes_after_the_last_note_and_stays_silent() {
        let mut cache = RenderCache::new("counter", counter(0), score_events(), None, RATE);
        assert_eq!(at_frame(&mut cache, 441_000), 0.0);
        let rendered = cache.rendered_frames();
        assert!(rendered < 44100 + 44100, "stopped rendering, got {rendered} frames");
        assert!(cache.failure().is_none());
    }

    #[test]
    fn effect_reads_its_input_and_never_finishes_early() {
        let mut cache = RenderCache::new(
            "counter",
            counter(10),
            EventSource::None,
            Some(AudioSignal::Mono(Box::new(Constant(0.5)))),
            RATE,
        );
        assert_eq!(at_frame(&mut cache, 0), 0.5);
        assert_eq!(at_frame(&mut cache, 882_000), 0.5);
    }

    #[test]
    fn a_stereo_read_of_a_mono_cache_duplicates_the_channel() {
        let mut cache = RenderCache::new("counter", counter(0), score_events(), None, RATE);
        let (mut left, mut right) = ([9.0; 4], [9.0; 4]);
        StereoSignal::render(&mut cache, 22_050, &mut left, &mut right);
        assert_eq!((left, right), ([1.0; 4], [1.0; 4]));
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
    fn dropping_a_cache_frees_the_plugin() {
        let dropped = Arc::new(AtomicBool::new(false));
        let processor = Dropped(
            Arc::clone(&dropped),
            Counter {
                held: 0,
                latency: 0,
                line: Vec::new(),
            },
        );
        let mut cache = RenderCache::new(
            "counter",
            LayoutStage::new(Box::new(processor), BLOCK_FRAMES as usize, Layout::Mono),
            EventSource::None,
            Some(AudioSignal::Mono(Box::new(Constant(0.5)))),
            RATE,
        );
        at_frame(&mut cache, 4_410);
        drop(cache);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn live_note_holds_until_release() {
        let live = Arc::new(LiveNote::new(Semitone(60), 1.0, Default::default()));
        let mut cache = RenderCache::new(
            "counter",
            counter(0),
            EventSource::Live(Arc::clone(&live)),
            None,
            RATE,
        );
        assert_eq!(at_frame(&mut cache, 0), 1.0);
        assert_eq!(at_frame(&mut cache, 1000), 1.0);
        live.release(1000.0 / 44100.0);
        // The release takes effect at the next unrendered block.
        let next_block = cache.rendered_frames() as u64;
        assert_eq!(at_frame(&mut cache, next_block), 0.0);
    }
}
