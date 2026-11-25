//! Aura type constraint libraries for Imp generics.

use crate::signals::{AUDIO_MONO, AUDIO_STEREO, CHORD_PROGRESSION, SCORE};
use imp_core_types::{concrete_type, TypeConstraint, TypeConstraintLibrary};
use std::collections::BTreeMap;

/// Type constraint id for types that support unbounded loop (modulus over cycle length).
pub const LOOPABLE: &str = "Loopable";

/// Type constraint id for audio signals of any layout (`audio_mono | audio_stereo`). A node type
/// generic over `A: Audio` is layout-polymorphic (see `docs/design/audio-layouts.md`).
pub const AUDIO: &str = "Audio";

fn constraint(id: &str, members: &[&str]) -> (String, TypeConstraint) {
    (
        id.into(),
        TypeConstraint {
            id: id.into(),
            members: members.iter().map(|member| concrete_type(*member, vec![])).collect(),
        },
    )
}

/// Returns the Aura type constraint library (`Loopable`, `Audio`).
pub fn aura_type_constraint_library() -> TypeConstraintLibrary {
    TypeConstraintLibrary {
        id: "aura.constraints".into(),
        constraints: BTreeMap::from([
            constraint(LOOPABLE, &[SCORE, CHORD_PROGRESSION]),
            constraint(AUDIO, &[AUDIO_MONO, AUDIO_STEREO]),
        ]),
    }
}
