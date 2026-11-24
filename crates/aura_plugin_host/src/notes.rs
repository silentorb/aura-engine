//! Note events from a fully known score, at exact frames.

use crate::Event;
use aura_sample::SampleRate;
use aura_sequencing::{beats_to_seconds, ScoreSignal};

#[derive(Debug, Clone, Copy)]
struct NoteSpan {
    on: u64,
    off: u64,
    key: i16,
    velocity: f32,
}

/// The note-on / note-off events of a score, generated per block window. Looping scores
/// repeat every cycle without end.
#[derive(Debug, Clone)]
pub struct NoteEventSource {
    notes: Vec<NoteSpan>,
    /// Loop period in frames; `None` for a finite score.
    cycle: Option<u64>,
    /// Longest note, in frames: how far before a window a note-on can still end inside it.
    longest: u64,
}

impl NoteEventSource {
    pub fn from_score_signal(signal: &ScoreSignal, sample_rate: SampleRate) -> Self {
        let rate = f64::from(sample_rate.get());
        let score = &signal.score;
        let to_frame = |beats: f64| (beats_to_seconds(beats, score.tempo) * rate).round().max(0.0) as u64;
        let notes: Vec<NoteSpan> = score
            .events
            .iter()
            .map(|event| {
                let on = to_frame(event.start_beats);
                let off = to_frame(event.start_beats + event.duration_beats).max(on + 1);
                NoteSpan {
                    on,
                    off,
                    key: event.semitone.0,
                    velocity: event.velocity,
                }
            })
            .collect();
        let cycle = if signal.looping {
            Some(to_frame(score.loop_cycle_beats())).filter(|frames| *frames > 0)
        } else {
            None
        };
        let longest = notes.iter().map(|n| n.off - n.on).max().unwrap_or(0);
        Self {
            notes,
            cycle,
            longest,
        }
    }

    /// The frame after the last note-off; `None` when the score loops forever.
    pub fn end_frame(&self) -> Option<u64> {
        if self.cycle.is_some() {
            return None;
        }
        Some(self.notes.iter().map(|n| n.off).max().unwrap_or(0))
    }

    /// Appends the events in `[start, start + len)` to `out`, with block-relative frames.
    /// Note ids are unique per note instance (per loop cycle).
    pub fn events_in(&self, start: u64, len: u32, out: &mut Vec<Event>) {
        let end = start + u64::from(len);
        let count = self.notes.len() as u64;
        let mut push = |index: usize, cycle_index: u64, offset: u64| {
            let note = &self.notes[index];
            let note_id = ((cycle_index * count + index as u64) % i32::MAX as u64) as i32;
            let on = note.on + offset;
            let off = note.off + offset;
            if (start..end).contains(&on) {
                out.push(Event::NoteOn {
                    frame: (on - start) as u32,
                    note_id,
                    key: note.key,
                    velocity: note.velocity,
                });
            }
            if (start..end).contains(&off) {
                out.push(Event::NoteOff {
                    frame: (off - start) as u32,
                    note_id,
                    key: note.key,
                });
            }
        };
        match self.cycle {
            None => {
                for index in 0..self.notes.len() {
                    push(index, 0, 0);
                }
            }
            Some(cycle) => {
                let first = start.saturating_sub(self.longest) / cycle;
                let last = end / cycle;
                for cycle_index in first..=last {
                    for index in 0..self.notes.len() {
                        push(index, cycle_index, cycle_index * cycle);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_sequencing::{NoteEvent, Score, Semitone, Tempo, TimeSignature};

    const RATE: SampleRate = SampleRate::RATE_44100;

    fn score() -> Score {
        // 120 bpm: one beat = 0.5 s = 22050 frames.
        Score::new(
            TimeSignature::default(),
            Tempo::new(120.0).unwrap(),
            vec![
                NoteEvent::new(Semitone(60), 0.0, 1.0),
                NoteEvent::new(Semitone(64), 1.0, 1.0),
            ],
        )
    }

    fn collect(source: &NoteEventSource, start: u64, len: u32) -> Vec<Event> {
        let mut out = Vec::new();
        source.events_in(start, len, &mut out);
        crate::sort_events(&mut out);
        out
    }

    #[test]
    fn finite_score_places_events_on_exact_frames() {
        let source = NoteEventSource::from_score_signal(&ScoreSignal::finite(score()), RATE);
        assert_eq!(source.end_frame(), Some(44100));
        assert_eq!(
            collect(&source, 22000, 100),
            vec![
                Event::NoteOff { frame: 50, note_id: 0, key: 60 },
                Event::NoteOn { frame: 50, note_id: 1, key: 64, velocity: 1.0 },
            ]
        );
        assert!(collect(&source, 50000, 1000).is_empty());
    }

    #[test]
    fn looping_score_repeats_with_fresh_note_ids() {
        let signal = ScoreSignal {
            looping: true,
            ..ScoreSignal::finite(score())
        };
        let source = NoteEventSource::from_score_signal(&signal, RATE);
        assert_eq!(source.end_frame(), None);
        let cycle = (source.cycle.unwrap()) as u32;
        let events = collect(&source, u64::from(cycle), 1);
        assert_eq!(
            events,
            vec![
                Event::NoteOff { frame: 0, note_id: 1, key: 64 },
                Event::NoteOn { frame: 0, note_id: 2, key: 60, velocity: 1.0 },
            ]
        );
    }
}
