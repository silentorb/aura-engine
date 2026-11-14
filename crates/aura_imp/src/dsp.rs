//! Aura DSP node library.

use crate::node_def::primitive_node;
use crate::signals::{signal, ANY, AUDIO_MONO, CONTROL, TIME};
use imp_core_types::{NodeLibrary, Port, PrimitiveValue};
use std::collections::BTreeMap;

fn port(id: &str, signal_type: imp_core_types::SignalType, default_value: Option<PrimitiveValue>) -> Port {
    Port {
        id: id.into(),
        signal_type,
        default_value,
        variadic: false,
    }
}

fn variadic_port(id: &str, signal_type: imp_core_types::SignalType) -> Port {
    Port {
        id: id.into(),
        signal_type,
        default_value: None,
        variadic: true,
    }
}

/// Returns the Aura DSP node type library.
pub fn dsp_node_library() -> NodeLibrary {
    let control = signal(CONTROL);
    let time = signal(TIME);
    let audio_mono = signal(AUDIO_MONO);

    let definitions = vec![
        primitive_node(
            "sine",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "frequency".into(),
                    port(
                        "frequency",
                        control.clone(),
                        Some(PrimitiveValue::Number(440.0)),
                    ),
                ),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "exponential_sweep_sine",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "start_hz".into(),
                    port(
                        "start_hz",
                        control.clone(),
                        Some(PrimitiveValue::Number(150.0)),
                    ),
                ),
                (
                    "decay_rate".into(),
                    port(
                        "decay_rate",
                        control.clone(),
                        Some(PrimitiveValue::Number(12.0)),
                    ),
                ),
                ("elapsed".into(), port("elapsed", time.clone(), None)),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "deterministic_noise",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "seed".into(),
                    port("seed", signal(ANY), Some(PrimitiveValue::Number(0.0))),
                ),
                ("time".into(), port("time", time.clone(), None)),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "highpass_noise",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "seed".into(),
                    port("seed", signal(ANY), Some(PrimitiveValue::Number(0.0))),
                ),
                ("time".into(), port("time", time.clone(), None)),
                (
                    "dt".into(),
                    port("dt", control.clone(), Some(PrimitiveValue::Number(0.000_1))),
                ),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "bandlimited_saw",
            Vec::new(),
            None,
            BTreeMap::from([
                (
                    "frequency".into(),
                    port(
                        "frequency",
                        control.clone(),
                        Some(PrimitiveValue::Number(110.0)),
                    ),
                ),
                ("elapsed".into(), port("elapsed", time.clone(), None)),
                (
                    "cutoff".into(),
                    port(
                        "cutoff",
                        control.clone(),
                        Some(PrimitiveValue::Number(800.0)),
                    ),
                ),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "multiply_control",
            Vec::new(),
            None,
            BTreeMap::from([
                ("a".into(), port("a", control.clone(), None)),
                ("b".into(), port("b", control.clone(), None)),
            ]),
            BTreeMap::from([(
                "value".into(),
                port("value", control.clone(), None),
            )]),
        ),
        primitive_node(
            "multiply",
            Vec::new(),
            None,
            BTreeMap::from([
                ("a".into(), port("a", audio_mono.clone(), None)),
                ("b".into(), port("b", control.clone(), None)),
            ]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", audio_mono.clone(), None),
            )]),
        ),
        primitive_node(
            "add",
            Vec::new(),
            None,
            BTreeMap::from([(
                "inputs".into(),
                variadic_port("inputs", audio_mono),
            )]),
            BTreeMap::from([(
                "audio".into(),
                port("audio", signal(AUDIO_MONO), None),
            )]),
        ),
    ];

    NodeLibrary {
        id: "aura.dsp".into(),
        definitions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imp_registry::get_node_type;

    #[test]
    fn dsp_library_contains_expected_nodes() {
        let library = dsp_node_library();
        let registry = imp_registry::load_library(
            imp_registry::create_registry(),
            library,
        )
        .expect("registry");
        assert!(get_node_type(&registry, "sine").is_some());
        assert!(get_node_type(&registry, "multiply").is_some());
        let add = get_node_type(&registry, "add").expect("add");
        assert!(add.inputs.get("inputs").is_some_and(|port| port.variadic));
        assert!(get_node_type(&registry, "exponential_sweep_sine").is_some());
        assert!(get_node_type(&registry, "deterministic_noise").is_some());
        assert!(get_node_type(&registry, "highpass_noise").is_some());
        assert!(get_node_type(&registry, "bandlimited_saw").is_some());
    }
}
