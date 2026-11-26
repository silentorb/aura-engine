//! Aura type definitions for Imp generics (named unions).

use crate::signals::{signal, AUDIO_MONO, AUDIO_STEREO, CHORD_PROGRESSION, SCORE};
use imp_core_types::{union_of, NodeLibrary, Type, TypeDefinition};

/// Union of the types that support unbounded loop (modulus over cycle length):
/// `Loopable = score | chord_progression`.
pub const LOOPABLE: &str = "Loopable";

/// Union of the audio layouts: `Audio = audio_mono | audio_stereo`. A node type generic over
/// `A: Audio` is layout-polymorphic (see `docs/design/audio-layouts.md`).
pub const AUDIO: &str = "Audio";

fn union_type(id: &str, members: &[&str]) -> TypeDefinition {
    TypeDefinition {
        id: id.into(),
        params: Vec::new(),
        ty: union_of(members.iter().map(|member| signal(member)).collect()),
    }
}

/// Aura's type definitions (`Loopable`, `Audio`).
pub fn aura_type_definitions() -> Vec<TypeDefinition> {
    vec![
        union_type(LOOPABLE, &[SCORE, CHORD_PROGRESSION]),
        union_type(AUDIO, &[AUDIO_MONO, AUDIO_STEREO]),
    ]
}

/// A reference to a named Aura type (e.g. `named(AUDIO)` for a bound).
pub fn named(id: &str) -> Type {
    signal(id)
}

/// The types-only node library that carries Aura's type definitions.
pub fn aura_types_library() -> NodeLibrary {
    NodeLibrary {
        id: "aura.types".into(),
        definitions: Vec::new(),
        types: aura_type_definitions(),
    }
}
