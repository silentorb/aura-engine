//! Translating a captured live performance into notation.
//!
//! Captured events are Aura terms (semitone, velocity 0–1, sustain), not MIDI bytes; an input
//! adapter produces them. Times are seconds on the notation's timeline (beat 0 = 0 s at the
//! notation tempo). Translation keeps raw timing: nothing is snapped to a grid.

use crate::{sort_events_by_start, NoteEvent, Semitone, Tempo};
use std::collections::{HashMap, HashSet};

/// Shortest note a take produces, so very short notes stay visible and selectable.
pub const MIN_DURATION_BEATS: f64 = 1.0 / 32.0;

/// What happened at one moment of a performance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapturedKind {
    KeyDown { semitone: Semitone, velocity: f32 },
    KeyUp { semitone: Semitone },
    SustainDown,
    SustainUp,
}

/// One timestamped performance event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapturedEvent {
    /// Seconds on the notation's timeline.
    pub at_secs: f64,
    pub kind: CapturedKind,
}

impl CapturedEvent {
    pub fn new(at_secs: f64, kind: CapturedKind) -> Self {
        Self { at_secs, kind }
    }
}

struct Open {
    start_secs: f64,
    velocity: f32,
}

/// Translates a take into notes.
///
/// - A key-down and the next key-up of the same key make one note; chords pair per key.
/// - A key-down for a key already sounding ends the earlier note there.
/// - A key released while the sustain pedal is down sounds until pedal-up.
/// - Keys (or the pedal) still held at `stop_secs` end there.
/// - A key-up without a key-down (pressed before capture began) is ignored.
/// - Starts are clamped at beat 0; lengths are at least [`MIN_DURATION_BEATS`].
pub fn notes_from_capture(
    events: &[CapturedEvent],
    tempo: Tempo,
    stop_secs: f64,
) -> Vec<NoteEvent> {
    let seconds_per_beat = tempo.seconds_per_beat();
    let mut ordered: Vec<&CapturedEvent> = events.iter().collect();
    ordered.sort_by(|a, b| {
        a.at_secs
            .partial_cmp(&b.at_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut open: HashMap<i16, Open> = HashMap::new();
    let mut sustained: HashSet<i16> = HashSet::new();
    let mut pedal = false;
    let mut notes = Vec::new();

    let close = |semitone: i16, note: Open, end_secs: f64, notes: &mut Vec<NoteEvent>| {
        let start_beats = (note.start_secs / seconds_per_beat).max(0.0);
        let end_beats = end_secs / seconds_per_beat;
        let duration_beats = (end_beats - start_beats).max(MIN_DURATION_BEATS);
        notes.push(
            NoteEvent::new(Semitone(semitone), start_beats, duration_beats)
                .with_velocity(note.velocity),
        );
    };

    for event in ordered {
        let t = event.at_secs;
        match event.kind {
            CapturedKind::KeyDown { semitone, velocity } => {
                if let Some(previous) = open.remove(&semitone.0) {
                    sustained.remove(&semitone.0);
                    close(semitone.0, previous, t, &mut notes);
                }
                open.insert(
                    semitone.0,
                    Open {
                        start_secs: t,
                        velocity: velocity.clamp(0.0, 1.0),
                    },
                );
            }
            CapturedKind::KeyUp { semitone } => {
                if !open.contains_key(&semitone.0) {
                    continue;
                }
                if pedal {
                    sustained.insert(semitone.0);
                } else if let Some(note) = open.remove(&semitone.0) {
                    close(semitone.0, note, t, &mut notes);
                }
            }
            CapturedKind::SustainDown => pedal = true,
            CapturedKind::SustainUp => {
                pedal = false;
                for semitone in sustained.drain() {
                    if let Some(note) = open.remove(&semitone) {
                        close(semitone, note, t, &mut notes);
                    }
                }
            }
        }
    }

    let mut held: Vec<(i16, Open)> = open.into_iter().collect();
    held.sort_by_key(|(semitone, _)| *semitone);
    for (semitone, note) in held {
        close(semitone, note, stop_secs, &mut notes);
    }

    // Chords: order by pitch first so the stable start sort is deterministic.
    notes.sort_by_key(|n| n.semitone.0);
    sort_events_by_start(&mut notes);
    notes
}

/// The take as it stands at `now_secs`: held notes have their duration so far.
pub fn take_snapshot(events: &[CapturedEvent], tempo: Tempo, now_secs: f64) -> Vec<NoteEvent> {
    notes_from_capture(events, tempo, now_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempo() -> Tempo {
        Tempo::new(120.0).expect("tempo") // 0.5 s per beat
    }

    fn down(at: f64, semitone: i16, velocity: f32) -> CapturedEvent {
        CapturedEvent::new(
            at,
            CapturedKind::KeyDown {
                semitone: Semitone(semitone),
                velocity,
            },
        )
    }

    fn up(at: f64, semitone: i16) -> CapturedEvent {
        CapturedEvent::new(
            at,
            CapturedKind::KeyUp {
                semitone: Semitone(semitone),
            },
        )
    }

    fn summary(notes: &[NoteEvent]) -> Vec<(i16, f64, f64)> {
        notes
            .iter()
            .map(|n| {
                (
                    n.semitone.0,
                    (n.start_beats * 1e6).round() / 1e6,
                    (n.duration_beats * 1e6).round() / 1e6,
                )
            })
            .collect()
    }

    #[test]
    fn one_note_in_beats_at_tempo() {
        let notes = notes_from_capture(&[down(1.0, 60, 0.5), up(1.5, 60)], tempo(), 4.0);
        assert_eq!(summary(&notes), vec![(60, 2.0, 1.0)]);
        assert_eq!(notes[0].velocity, 0.5);
    }

    #[test]
    fn chord_pairs_per_key() {
        let notes = notes_from_capture(
            &[
                down(0.0, 60, 1.0),
                down(0.0, 64, 1.0),
                up(0.5, 64),
                up(1.0, 60),
            ],
            tempo(),
            4.0,
        );
        assert_eq!(summary(&notes), vec![(60, 0.0, 2.0), (64, 0.0, 1.0)]);
    }

    #[test]
    fn retriggering_a_sounding_key_ends_the_earlier_note() {
        let notes = notes_from_capture(
            &[down(0.0, 60, 1.0), down(0.5, 60, 0.8), up(1.0, 60)],
            tempo(),
            4.0,
        );
        assert_eq!(summary(&notes), vec![(60, 0.0, 1.0), (60, 1.0, 1.0)]);
        assert_eq!(notes[1].velocity, 0.8);
    }

    #[test]
    fn sustain_pedal_holds_released_keys_until_pedal_up() {
        let notes = notes_from_capture(
            &[
                CapturedEvent::new(0.0, CapturedKind::SustainDown),
                down(0.0, 60, 1.0),
                up(0.25, 60),
                down(0.5, 64, 1.0),
                up(0.75, 64),
                CapturedEvent::new(1.5, CapturedKind::SustainUp),
                down(2.0, 67, 1.0),
                up(2.5, 67),
            ],
            tempo(),
            4.0,
        );
        assert_eq!(
            summary(&notes),
            vec![(60, 0.0, 3.0), (64, 1.0, 2.0), (67, 4.0, 1.0)]
        );
    }

    #[test]
    fn a_key_held_through_pedal_up_still_ends_at_its_own_release() {
        let notes = notes_from_capture(
            &[
                CapturedEvent::new(0.0, CapturedKind::SustainDown),
                down(0.0, 60, 1.0),
                CapturedEvent::new(0.5, CapturedKind::SustainUp),
                up(1.0, 60),
            ],
            tempo(),
            4.0,
        );
        assert_eq!(summary(&notes), vec![(60, 0.0, 2.0)]);
    }

    #[test]
    fn keys_and_pedal_held_at_stop_end_at_stop() {
        let notes = notes_from_capture(
            &[
                down(0.0, 60, 1.0),
                CapturedEvent::new(0.0, CapturedKind::SustainDown),
                down(0.5, 62, 1.0),
                up(1.0, 62),
            ],
            tempo(),
            2.0,
        );
        assert_eq!(summary(&notes), vec![(60, 0.0, 4.0), (62, 1.0, 3.0)]);
    }

    #[test]
    fn orphan_key_up_is_ignored() {
        let notes = notes_from_capture(
            &[up(0.5, 60), down(1.0, 62, 1.0), up(1.5, 62)],
            tempo(),
            4.0,
        );
        assert_eq!(summary(&notes), vec![(62, 2.0, 1.0)]);
    }

    #[test]
    fn count_in_pickups_are_kept_and_clamped_at_zero() {
        // Take starts at beat 4 (2 s); a pickup at 1.75 s is kept before the playhead.
        let notes = notes_from_capture(&[down(1.75, 60, 1.0), up(2.25, 60)], tempo(), 4.0);
        assert_eq!(summary(&notes), vec![(60, 3.5, 1.0)]);
        // Before beat 0 (count-in from a playhead under one bar): clamped.
        let notes = notes_from_capture(&[down(-0.25, 60, 1.0), up(0.5, 60)], tempo(), 4.0);
        assert_eq!(summary(&notes), vec![(60, 0.0, 1.0)]);
    }

    #[test]
    fn very_short_notes_get_the_minimum_length() {
        let notes = notes_from_capture(&[down(1.0, 60, 1.0), up(1.0, 60)], tempo(), 4.0);
        assert_eq!(notes[0].duration_beats, MIN_DURATION_BEATS);
    }

    #[test]
    fn events_out_of_order_are_sorted_by_time() {
        let notes = notes_from_capture(&[up(1.0, 60), down(0.0, 60, 1.0)], tempo(), 4.0);
        assert_eq!(summary(&notes), vec![(60, 0.0, 2.0)]);
    }

    #[test]
    fn snapshot_reports_held_notes_so_far() {
        let events = [down(0.0, 60, 1.0)];
        assert_eq!(
            summary(&take_snapshot(&events, tempo(), 0.5)),
            vec![(60, 0.0, 1.0)]
        );
        assert_eq!(
            summary(&take_snapshot(&events, tempo(), 1.0)),
            vec![(60, 0.0, 2.0)]
        );
    }
}
