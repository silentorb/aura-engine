//! Musical composition node library.

use crate::constraints::LOOPABLE;
use crate::node_def::primitive_node;
use crate::signals::{
    signal, CHORD_PROGRESSION, CONTROL, SCORE, TEMPO, TIME, TIME_SIGNATURE,
};
use imp_core_types::{
    concrete_type, type_var, ImplementationCase, NodeImplementation, NodeLibrary, Port,
    PrimitiveValue, TypeParam,
};
use std::collections::BTreeMap;

fn port(id: &str, signal_type: imp_core_types::SignalType, default_value: Option<PrimitiveValue>) -> Port {
    Port {
        id: id.into(),
        signal_type,
        default_value,
    }
}

/// Returns the Aura music node library.
pub fn music_node_library() -> NodeLibrary {
    let control = signal(CONTROL);
    let score = signal(SCORE);
    let time = signal(TIME);
    let progression = signal(CHORD_PROGRESSION);
    let tempo = signal(TEMPO);
    let time_signature = signal(TIME_SIGNATURE);

    let definitions = vec![
        primitive_node(
            "constant_tempo",
            Vec::new(),
            None,
            BTreeMap::from([(
                "bpm".into(),
                port("bpm", control.clone(), Some(PrimitiveValue::Number(120.0))),
            )]),
            BTreeMap::from([(
                "tempo".into(),
                port("tempo", tempo.clone(), None),
            )]),
        ),
        primitive_node(
            "constant_time_signature",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "beats_per_bar".into(),
                    port(
                        "beats_per_bar",
                        control.clone(),
                        Some(PrimitiveValue::Number(4.0)),
                    ),
                ),
                (
                    "beat_unit".into(),
                    port(
                        "beat_unit",
                        control.clone(),
                        Some(PrimitiveValue::Number(4.0)),
                    ),
                ),
            ]),
            BTreeMap::from([(
                "time_signature".into(),
                port("time_signature", time_signature.clone(), None),
            )]),
        ),
        primitive_node(
            "epic_minor_progression",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "root".into(),
                    port("root", control.clone(), Some(PrimitiveValue::Number(57.0))),
                ),
                (
                    "bars_per_chord".into(),
                    port(
                        "bars_per_chord",
                        control.clone(),
                        Some(PrimitiveValue::Number(1.0)),
                    ),
                ),
                (
                    "time_signature".into(),
                    port("time_signature", time_signature.clone(), None),
                ),
            ]),
            BTreeMap::from([(
                "progression".into(),
                port("progression", progression.clone(), None),
            )]),
        ),
        primitive_node(
            "drum_grid",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "tempo".into(),
                    port("tempo", tempo.clone(), None),
                ),
                (
                    "time_signature".into(),
                    port("time_signature", time_signature.clone(), None),
                ),
                (
                    "lane".into(),
                    port("lane", control.clone(), Some(PrimitiveValue::Number(0.0))),
                ),
            ]),
            BTreeMap::from([(
                "score".into(),
                port("score", score.clone(), None),
            )]),
        ),
        primitive_node(
            "loop",
            vec![TypeParam {
                id: "T".into(),
                bounds: vec![LOOPABLE.into()],
            }],
            Some(NodeImplementation::Dispatch {
                param_index: Some(0),
                cases: vec![
                    ImplementationCase {
                        match_type: concrete_type(SCORE, vec![]),
                        implementation: "loop_score".into(),
                    },
                    ImplementationCase {
                        match_type: concrete_type(CHORD_PROGRESSION, vec![]),
                        implementation: "loop_progression".into(),
                    },
                ],
                default: None,
            }),
            BTreeMap::from([(
                "value".into(),
                port("value", type_var("T"), None),
            )]),
            BTreeMap::from([(
                "value".into(),
                port("value", type_var("T"), None),
            )]),
        ),
        primitive_node(
            "arpeggio",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "progression".into(),
                    port("progression", progression.clone(), None),
                ),
                (
                    "tempo".into(),
                    port("tempo", tempo.clone(), None),
                ),
                (
                    "time_signature".into(),
                    port("time_signature", time_signature.clone(), None),
                ),
                (
                    "subdivision".into(),
                    port(
                        "subdivision",
                        control.clone(),
                        Some(PrimitiveValue::Number(4.0)),
                    ),
                ),
            ]),
            BTreeMap::from([(
                "score".into(),
                port("score", score.clone(), None),
            )]),
        ),
        primitive_node(
            "note_at_time",
            Vec::new(),
            None,
            BTreeMap::from([
                ("score".into(), port("score", score.clone(), None)),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([
                (
                    "semitone".into(),
                    port("semitone", control.clone(), None),
                ),
                (
                    "active".into(),
                    port("active", control.clone(), None),
                ),
                (
                    "velocity".into(),
                    port("velocity", control.clone(), None),
                ),
                (
                    "note_start".into(),
                    port("note_start", time.clone(), None),
                ),
                (
                    "note_duration".into(),
                    port("note_duration", time.clone(), None),
                ),
            ]),
        ),
        primitive_node(
            "note_param_at_time",
            Vec::new(),
            None,
            BTreeMap::from([
                ("score".into(), port("score", score.clone(), None)),
                ("time".into(), port("time", time.clone(), None)),
                (
                    "param".into(),
                    port(
                        "param",
                        control.clone(),
                        Some(PrimitiveValue::String("cutoff_mult".into())),
                    ),
                ),
            ]),
            BTreeMap::from([(
                "value".into(),
                port("value", control.clone(), None),
            )]),
        ),
        primitive_node(
            "bass_line",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "progression".into(),
                    port("progression", progression.clone(), None),
                ),
                (
                    "tempo".into(),
                    port("tempo", tempo.clone(), None),
                ),
                (
                    "time_signature".into(),
                    port("time_signature", time_signature.clone(), None),
                ),
                (
                    "subdivision".into(),
                    port(
                        "subdivision",
                        control.clone(),
                        Some(PrimitiveValue::Number(8.0)),
                    ),
                ),
                (
                    "octave_offset".into(),
                    port(
                        "octave_offset",
                        control.clone(),
                        Some(PrimitiveValue::Number(-24.0)),
                    ),
                ),
            ]),
            BTreeMap::from([(
                "score".into(),
                port("score", score.clone(), None),
            )]),
        ),
        primitive_node(
            "chord_at_time",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "progression".into(),
                    port("progression", progression.clone(), None),
                ),
                (
                    "tempo".into(),
                    port("tempo", tempo.clone(), None),
                ),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([
                (
                    "root".into(),
                    port("root", control.clone(), None),
                ),
                (
                    "active".into(),
                    port("active", control.clone(), None),
                ),
            ]),
        ),
        primitive_node(
            "tempo_at_time",
            Vec::new(),
            None,
            BTreeMap::from([
                ("tempo".into(), port("tempo", tempo.clone(), None)),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([(
                "bpm".into(),
                port("bpm", control.clone(), None),
            )]),
        ),
        primitive_node(
            "time_signature_at_time",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "time_signature".into(),
                    port("time_signature", time_signature.clone(), None),
                ),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([
                (
                    "beats_per_bar".into(),
                    port("beats_per_bar", control.clone(), None),
                ),
                (
                    "beat_unit".into(),
                    port("beat_unit", control.clone(), None),
                ),
            ]),
        ),
        primitive_node(
            "semitone_to_hz",
            Vec::new(),
            None,
            BTreeMap::from([(
                "semitone".into(),
                port("semitone", control, None),
            )]),
            BTreeMap::from([(
                "frequency".into(),
                port("frequency", signal(CONTROL), None),
            )]),
        ),
    ];

    NodeLibrary {
        id: "aura.music".into(),
        definitions,
    }
}
