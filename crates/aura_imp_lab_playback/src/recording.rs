//! Live note capture: record a take from the MIDI input into notation notes.
//!
//! `record_start` plays the notation on the transport (deterministic mode) from one bar before
//! the playhead with a metronome click when the click is on, and captures input events on the
//! notation's timeline. While the take runs, the take so far is published on [`TOPIC_TAKE`].
//! `record_stop` stops the transport and returns the take translated into notes
//! (`aura_sequencing::capture`): raw timing, beats at the notation's tempo.

use crate::{playback_registry, publish, PlaybackServiceError, SAMPLE_RATE};
use aura_composition::click_bar;
use aura_imp::{graph_from_json_str, NodeId, NOTATION_NODE_TYPE};
use aura_integration::{
    sampler_from_score, score_from_notation_str, translate_graph_with_bindings,
};
use aura_render::Sampler;
use aura_sequencing::capture::{notes_from_capture, take_snapshot, CapturedEvent, CapturedKind};
use aura_sequencing::{
    beats_to_seconds, NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Topic carrying the take in progress: `{recording, countIn, notes}`.
pub const TOPIC_TAKE: &str = "aura.capture.take";
/// Instrument graph that voices the metronome (Lab pack `catalogs/instruments.json`).
pub const CLICK_INSTRUMENT: &str = "metronome_click";
const TAKE_PUBLISH_INTERVAL: Duration = Duration::from_millis(33);

/// The transport while recording: the notation from `origin_secs` (one bar before the playhead
/// with the click on), plus the click on the notation's bar grid. Transport time `t` is
/// notation time `t + origin_secs`.
pub struct RecordingTransport {
    notation: Box<dyn Sampler>,
    click: Option<Box<dyn Sampler>>,
    origin_secs: f64,
    bar_secs: f64,
}

impl Sampler for RecordingTransport {
    fn at(&self, t: f64) -> f32 {
        let n = t + self.origin_secs;
        let mut value = if n >= 0.0 { self.notation.at(n) } else { 0.0 };
        if let Some(click) = &self.click {
            if self.bar_secs > 0.0 {
                value += click.at(n.rem_euclid(self.bar_secs));
            }
        }
        value
    }
}

fn bar_secs(time_signature: TimeSignature, tempo: Tempo) -> f64 {
    beats_to_seconds(time_signature.quarter_beats_per_bar(), tempo)
}

fn click_graph_json() -> String {
    format!(
        r#"{{
          "nodes": {{
            "click": {{ "id": "click", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
            "time": {{ "id": "time", "type": "time", "inputs": {{}} }},
            "voice": {{ "id": "voice", "type": "{CLICK_INSTRUMENT}", "inputs": {{}} }},
            "out": {{ "id": "out", "type": "output", "inputs": {{}}, "title": "out" }}
          }},
          "edges": {{
            "score": {{ "id": "score", "from": {{ "node": "click", "port": "score" }}, "to": {{ "node": "voice", "port": "score" }} }},
            "time": {{ "id": "time", "from": {{ "node": "time", "port": "time" }}, "to": {{ "node": "voice", "port": "time" }} }},
            "audio": {{ "id": "audio", "from": {{ "node": "voice", "port": "audio" }}, "to": {{ "node": "out", "port": "value" }} }}
          }}
        }}"#
    )
}

/// One bar of clicks (deterministic), voiced by the pack's click instrument.
pub fn click_sampler(
    time_signature: TimeSignature,
    tempo: Tempo,
) -> Result<Box<dyn Sampler>, PlaybackServiceError> {
    let graph = graph_from_json_str(&click_graph_json())
        .map_err(|e| PlaybackServiceError::GraphJson(e.to_string()))?;
    let registry = playback_registry(None)?;
    let bindings = BTreeMap::from([(
        NodeId::from("click"),
        ScoreSignal::finite(click_bar(time_signature, tempo)),
    )]);
    translate_graph_with_bindings(&graph, &registry, SAMPLE_RATE, &bindings)
        .map_err(|e| PlaybackServiceError::Translate(e.to_string()))
}

/// The recording transport for `score` from `start_beats`; one bar of count-in with the click.
pub fn recording_transport(
    score: Score,
    start_beats: f64,
    click: bool,
) -> Result<RecordingTransport, PlaybackServiceError> {
    let tempo = score.tempo;
    let time_signature = score.time_signature;
    let bar = bar_secs(time_signature, tempo);
    let playhead = beats_to_seconds(start_beats.max(0.0), tempo);
    let click = if click {
        Some(click_sampler(time_signature, tempo)?)
    } else {
        None
    };
    let origin_secs = if click.is_some() {
        playhead - bar
    } else {
        playhead
    };
    Ok(RecordingTransport {
        notation: sampler_from_score(score, SAMPLE_RATE),
        click,
        origin_secs,
        bar_secs: bar,
    })
}

/// A take in progress.
#[derive(Debug, Clone)]
pub struct Take {
    tempo: Tempo,
    playhead_secs: f64,
    events: Vec<CapturedEvent>,
    /// `(input timestamp µs, notation secs)` paired at the first event of the take.
    anchor: Option<(u64, f64)>,
    keys_down: BTreeSet<i16>,
    pedal: bool,
    changed: bool,
    generation: u64,
}

impl Take {
    pub fn new(tempo: Tempo, playhead_secs: f64) -> Self {
        Self {
            tempo,
            playhead_secs,
            events: Vec::new(),
            anchor: None,
            keys_down: BTreeSet::new(),
            pedal: false,
            changed: true,
            generation: 0,
        }
    }

    /// Places an input event on the notation timeline. The first event pairs the input clock
    /// with `now_secs` (the transport's notation time); later events follow the input's own
    /// timestamps, which avoids the jitter of reading the transport clock per event.
    pub fn push(&mut self, timestamp_us: u64, kind: CapturedKind, now_secs: impl FnOnce() -> f64) {
        let at_secs = match self.anchor {
            Some((anchor_us, anchor_secs)) => {
                anchor_secs + (timestamp_us as f64 - anchor_us as f64) / 1e6
            }
            None => {
                let now = now_secs();
                self.anchor = Some((timestamp_us, now));
                now
            }
        };
        match kind {
            CapturedKind::KeyDown { semitone, .. } => {
                self.keys_down.insert(semitone.0);
            }
            CapturedKind::KeyUp { semitone } => {
                self.keys_down.remove(&semitone.0);
            }
            CapturedKind::SustainDown => self.pedal = true,
            CapturedKind::SustainUp => self.pedal = false,
        }
        self.events.push(CapturedEvent::new(at_secs, kind));
        self.changed = true;
    }

    /// Ends held keys and the pedal at `at_secs` (the device went away).
    pub fn release(&mut self, keys: &[i16], at_secs: f64) {
        for key in keys {
            if self.keys_down.remove(key) {
                self.events.push(CapturedEvent::new(
                    at_secs,
                    CapturedKind::KeyUp {
                        semitone: Semitone(*key),
                    },
                ));
            }
        }
        if self.pedal {
            self.pedal = false;
            self.events
                .push(CapturedEvent::new(at_secs, CapturedKind::SustainUp));
        }
        self.changed = true;
    }

    /// Whether notes are still growing (a key or the pedal is down).
    pub fn sounding(&self) -> bool {
        !self.keys_down.is_empty() || self.pedal
    }

    pub fn notes(&self, now_secs: f64) -> Vec<NoteEvent> {
        take_snapshot(&self.events, self.tempo, now_secs)
    }

    pub fn finish(&self, stop_secs: f64) -> Vec<NoteEvent> {
        notes_from_capture(&self.events, self.tempo, stop_secs)
    }

    pub fn published(&self, now_secs: f64) -> Value {
        json!({
            "recording": true,
            "countIn": now_secs < self.playhead_secs,
            "notes": notes_json(&self.notes(now_secs)),
        })
    }
}

/// Notation notes as Lab's notation event JSON.
pub fn notes_json(notes: &[NoteEvent]) -> Value {
    Value::Array(
        notes
            .iter()
            .map(|n| {
                json!({
                    "semitone": n.semitone.0,
                    "startBeats": n.start_beats,
                    "durationBeats": n.duration_beats,
                    "velocity": n.velocity,
                })
            })
            .collect(),
    )
}

#[derive(Default)]
struct Recorder {
    take: Option<Take>,
    /// A take ended by stopping the transport, kept until `record_stop` collects it.
    ended: Option<Vec<NoteEvent>>,
    generation: u64,
}

fn recorder() -> &'static Mutex<Recorder> {
    static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();
    RECORDER.get_or_init(|| Mutex::new(Recorder::default()))
}

/// The transport's position on the notation timeline.
fn transport_now() -> f64 {
    crate::position_secs_unclamped().unwrap_or(0.0)
}

fn idle_value() -> Value {
    json!({"recording": false, "countIn": false, "notes": []})
}

/// Starts a take: transport playback of `notation_json` from `start_beats` (with click and
/// count-in when `click`), and capture of input events.
pub fn record_start(
    notation_json: &str,
    start_beats: f64,
    click: bool,
) -> Result<(), PlaybackServiceError> {
    let score =
        score_from_notation_str(notation_json).map_err(PlaybackServiceError::NotationJson)?;
    let tempo = score.tempo;
    let playhead_secs = beats_to_seconds(start_beats.max(0.0), tempo);
    let transport = recording_transport(score, start_beats, click)?;
    let origin_secs = transport.origin_secs;

    let mut guard = recorder()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    if guard.take.is_some() {
        return Err(PlaybackServiceError::Recording("already recording".into()));
    }
    crate::start_transport(Arc::new(transport), origin_secs)?;
    guard.generation += 1;
    let mut take = Take::new(tempo, playhead_secs);
    take.generation = guard.generation;
    publish(TOPIC_TAKE, &take.published(origin_secs));
    let generation = take.generation;
    guard.take = Some(take);
    guard.ended = None;
    drop(guard);
    spawn_publisher(generation);
    Ok(())
}

fn spawn_publisher(generation: u64) {
    let _ = std::thread::Builder::new()
        .name("aura-take".into())
        .spawn(move || loop {
            std::thread::sleep(TAKE_PUBLISH_INTERVAL);
            let Ok(mut guard) = recorder().lock() else {
                return;
            };
            let Some(take) = guard.take.as_mut() else {
                return;
            };
            if take.generation != generation {
                return;
            }
            let now = transport_now();
            // Publish while notes grow, on new events, and once when the count-in ends.
            let count_in_edge = now >= take.playhead_secs
                && now - TAKE_PUBLISH_INTERVAL.as_secs_f64() < take.playhead_secs;
            if take.changed || take.sounding() || count_in_edge {
                take.changed = false;
                publish(TOPIC_TAKE, &take.published(now));
            }
        });
}

/// Ends the take (if any) at the transport's current position, stopping the transport.
fn end_take(guard: &mut Recorder) -> Option<Vec<NoteEvent>> {
    let take = guard.take.take()?;
    let stop_secs = transport_now();
    crate::stop_transport();
    publish(TOPIC_TAKE, &idle_value());
    Some(take.finish(stop_secs))
}

/// Stops the take and returns its notes (or those of a take the transport stop already ended).
pub fn record_stop() -> Result<Vec<NoteEvent>, PlaybackServiceError> {
    let mut guard = recorder()
        .lock()
        .map_err(|_| PlaybackServiceError::Poisoned)?;
    if let Some(notes) = end_take(&mut guard) {
        return Ok(notes);
    }
    Ok(guard.ended.take().unwrap_or_default())
}

/// Called before the transport stops for another reason: ends the take, keeping its notes for
/// the next `record_stop`.
pub(crate) fn end_if_active() {
    if let Ok(mut guard) = recorder().lock() {
        if let Some(notes) = end_take(&mut guard) {
            guard.ended = Some(notes);
        }
    }
}

pub fn is_recording() -> bool {
    recorder().lock().is_ok_and(|guard| guard.take.is_some())
}

/// Adds an input event to the take, if one is running.
pub(crate) fn capture(timestamp_us: u64, kind: CapturedKind) {
    if let Ok(mut guard) = recorder().lock() {
        if let Some(take) = guard.take.as_mut() {
            take.push(timestamp_us, kind, transport_now);
        }
    }
}

/// Ends the given held keys (and the pedal) now: their device went away.
pub(crate) fn release_keys(keys: &[i16]) {
    if let Ok(mut guard) = recorder().lock() {
        if let Some(take) = guard.take.as_mut() {
            take.release(keys, transport_now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down(semitone: i16) -> CapturedKind {
        CapturedKind::KeyDown {
            semitone: Semitone(semitone),
            velocity: 1.0,
        }
    }

    fn up(semitone: i16) -> CapturedKind {
        CapturedKind::KeyUp {
            semitone: Semitone(semitone),
        }
    }

    #[test]
    fn take_follows_input_timestamps_from_the_first_event() {
        let tempo = Tempo::new(120.0).unwrap();
        let mut take = Take::new(tempo, 2.0);
        take.push(1_000_000, down(60), || 2.0);
        take.push(1_500_000, up(60), || {
            panic!("anchored: the transport is not read again")
        });
        take.push(1_750_000, down(62), || unreachable!());
        assert!(take.sounding());
        let snapshot = take.notes(2.875);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(
            (snapshot[0].start_beats, snapshot[0].duration_beats),
            (4.0, 1.0)
        );
        assert_eq!(snapshot[1].start_beats, 5.5);
        assert!(
            (snapshot[1].duration_beats - 0.25).abs() < 1e-9,
            "held: duration so far"
        );

        let notes = take.finish(3.0);
        assert!((notes[1].duration_beats - 0.5).abs() < 1e-9);
        let published = take.published(1.0);
        assert_eq!(published["countIn"], true);
        assert_eq!(published["notes"][0]["semitone"], 60);
    }

    #[test]
    fn released_keys_end_when_the_device_goes() {
        let tempo = Tempo::new(120.0).unwrap();
        let mut take = Take::new(tempo, 0.0);
        take.push(0, down(60), || 0.0);
        take.push(0, CapturedKind::SustainDown, || 0.0);
        take.release(&[60], 1.0);
        assert!(!take.sounding());
        let notes = take.finish(5.0);
        assert_eq!(notes[0].duration_beats, 2.0);
    }

    #[test]
    fn transport_counts_in_one_bar_with_clicks_on_the_beats() {
        crate::tests::use_pack_libraries();
        let tempo = Tempo::new(120.0).unwrap();
        let score = Score::new(TimeSignature::FOUR_FOUR, tempo, vec![]);
        // Playhead at beat 1 (0.5 s): count-in starts at notation time -1.5 s.
        let transport = recording_transport(score, 1.0, true).expect("transport");
        assert!((transport.origin_secs + 1.5).abs() < 1e-9);
        let peak = |from: f64| {
            (0..200)
                .map(|i| transport.at(from + i as f64 / 44_100.0).abs())
                .fold(0.0_f32, f32::max)
        };
        // Transport 0.5 s = notation -1.0 s = beat 2 of the bar: a click.
        assert!(peak(0.5) > 0.05, "click on the beat");
        // Between beats: silent.
        assert!(peak(0.75) < 1e-3, "silent between beats");
        // Downbeat louder than other beats. Transport 1.5 s = notation 0 s.
        assert!(peak(1.5) > peak(0.5));
    }

    #[test]
    fn transport_without_click_starts_at_the_playhead() {
        let tempo = Tempo::new(120.0).unwrap();
        let score = Score::new(TimeSignature::FOUR_FOUR, tempo, vec![]);
        let transport = recording_transport(score, 2.0, false).expect("transport");
        assert_eq!(transport.origin_secs, 1.0);
        assert_eq!(transport.at(0.25), 0.0);
    }
}
