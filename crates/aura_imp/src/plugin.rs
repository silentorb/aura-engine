//! Third-party plugin node library (`aura.plugin`).
//!
//! A plugin node names an installed plugin by id and carries its configuration as embedded
//! data: a saved state blob and parameter values. Plugin nodes have no plugin-specific ports.
//! The host resolves the id when the graph is translated; see `docs/design/plugin-hosting.md`.

use crate::node_def::primitive_node;
use crate::types::{named, AUDIO};
use crate::signals::{signal, AUDIO_STEREO, CONTROL, SCORE};
use imp_core_types::{type_var, NodeLibrary, Port, PrimitiveValue, TypeParam};
use std::collections::BTreeMap;

/// A plugin instrument: plays the notes of its `score` input. Its output is stereo (a mono
/// plugin's output is duplicated).
pub const PLUGIN_INSTRUMENT: &str = "plugin_instrument";
/// A plugin effect: processes its `audio` input, mono or stereo (`A: Audio`); its output has the
/// same layout.
pub const PLUGIN_EFFECT: &str = "plugin_effect";

/// Literal input naming the plugin (for CLAP, its plugin id, such as `org.surge-synth-team.surge-xt`).
pub const PLUGIN_ID_PORT: &str = "plugin";
/// Literal input holding the plugin's saved state, base64-encoded. Empty for the plugin's
/// defaults.
pub const PLUGIN_STATE_PORT: &str = "state";
/// Literal input holding parameter values as a JSON object string: CLAP parameter id (decimal)
/// to plain value, e.g. `{"0": 0.4}`. Applied after `state`, overriding it. Empty for none.
pub const PLUGIN_PARAMS_PORT: &str = "params";

fn port(id: &str, ty: imp_core_types::Type, default_value: Option<PrimitiveValue>) -> Port {
    Port {
        id: id.into(),
        ty,
        default_value,
        variadic: false,
    }
}

fn literal(id: &str) -> (String, Port) {
    (
        id.into(),
        port(
            id,
            signal(CONTROL),
            Some(PrimitiveValue::String(String::new())),
        ),
    )
}

/// Returns the Aura plugin node library.
pub fn plugin_node_library() -> NodeLibrary {
    let stereo = signal(AUDIO_STEREO);
    let audio = type_var("A");
    NodeLibrary {
        types: Vec::new(),
        id: "aura.plugin".into(),
        definitions: vec![
            primitive_node(
                PLUGIN_INSTRUMENT,
                Vec::new(),
                None,
                BTreeMap::from([
                    ("score".into(), port("score", signal(SCORE), None)),
                    literal(PLUGIN_ID_PORT),
                    literal(PLUGIN_STATE_PORT),
                    literal(PLUGIN_PARAMS_PORT),
                ]),
                BTreeMap::from([("audio".into(), port("audio", stereo, None))]),
            ),
            primitive_node(
                PLUGIN_EFFECT,
                vec![TypeParam {
                    id: "A".into(),
                    bounds: vec![named(AUDIO)],
                }],
                None,
                BTreeMap::from([
                    ("audio".into(), port("audio", audio.clone(), None)),
                    literal(PLUGIN_ID_PORT),
                    literal(PLUGIN_STATE_PORT),
                    literal(PLUGIN_PARAMS_PORT),
                ]),
                BTreeMap::from([("audio".into(), port("audio", audio, None))]),
            ),
        ],
    }
}
