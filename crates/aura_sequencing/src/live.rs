//! Live notes: notation for a note that is still sounding (streaming mode).

use crate::{NoteParams, Semitone};
use std::sync::atomic::{AtomicU64, Ordering};

/// A note in progress whose end is not yet known.
///
/// Streaming mode knows a note's pitch, velocity, and params at note-on, but not its length.
/// The release time (seconds after onset) is stamped once, when note-off arrives; until then the
/// note is held. Shared read-only (`Arc<LiveNote>`) between a voice and the runtime nodes that
/// read it, so the release is stored atomically.
#[derive(Debug)]
pub struct LiveNote {
    pub semitone: Semitone,
    pub velocity: f32,
    pub params: NoteParams,
    /// `f64` bits of the release time in seconds after onset; NaN while held.
    released_after: AtomicU64,
}

impl LiveNote {
    pub fn new(semitone: Semitone, velocity: f32, params: NoteParams) -> Self {
        Self {
            semitone,
            velocity,
            params,
            released_after: AtomicU64::new(f64::NAN.to_bits()),
        }
    }

    /// Stamps note-off at `at_secs` after onset. The first release wins.
    pub fn release(&self, at_secs: f64) {
        let at = at_secs.max(0.0);
        let _ = self.released_after.compare_exchange(
            f64::NAN.to_bits(),
            at.to_bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Seconds after onset at which the note was released, or `None` while held.
    pub fn released_after(&self) -> Option<f64> {
        let value = f64::from_bits(self.released_after.load(Ordering::Acquire));
        (!value.is_nan()).then_some(value)
    }

    pub fn is_held(&self) -> bool {
        self.released_after().is_none()
    }

    /// The note's length so far at `elapsed` seconds after onset: grows while held, then fixed
    /// at its final length once released.
    pub fn duration_so_far(&self, elapsed: f64) -> f64 {
        match self.released_after() {
            Some(released) => released,
            None => elapsed.max(0.0),
        }
    }

    pub fn param_or(&self, key: &str, default: f64) -> f64 {
        self.params.get(key).copied().unwrap_or(default)
    }
}

impl PartialEq for LiveNote {
    fn eq(&self, other: &Self) -> bool {
        self.semitone == other.semitone
            && self.velocity == other.velocity
            && self.params == other.params
            && self.released_after.load(Ordering::Acquire)
                == other.released_after.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_until_released_and_first_release_wins() {
        let note = LiveNote::new(Semitone(60), 1.0, NoteParams::new());
        assert!(note.is_held());
        assert_eq!(note.duration_so_far(0.25), 0.25);
        note.release(0.5);
        note.release(0.9);
        assert_eq!(note.released_after(), Some(0.5));
        assert_eq!(note.duration_so_far(2.0), 0.5);
    }
}
