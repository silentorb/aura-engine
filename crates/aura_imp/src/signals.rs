//! Shared signal type identifiers for Aura Imp libraries.

use imp_core_types::{concrete_type, Type};

/// Seconds from render start.
pub const TIME: &str = "time";

/// Scalar control value.
pub const CONTROL: &str = "control";

/// Mono audio sample amplitude.
pub const AUDIO_MONO: &str = "audio_mono";

/// Stereo audio frame (left, right).
pub const AUDIO_STEREO: &str = "audio_stereo";

/// Musical score (translate-time only).
pub const SCORE: &str = "score";

/// Chord progression (translate-time data signal).
pub const CHORD_PROGRESSION: &str = "chord_progression";

/// Tempo (translate-time data signal).
pub const TEMPO: &str = "tempo";

/// Time signature (translate-time data signal).
pub const TIME_SIGNATURE: &str = "time_signature";

/// Top type for polymorphic scalar inputs (e.g. noise seed from time or control).
pub const ANY: &str = "any";

pub fn signal(id: &str) -> Type {
    concrete_type(id, vec![])
}
