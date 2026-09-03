//! Time source node library.

use crate::node_def::primitive_node;
use crate::signals::{signal, TIME};
use imp_core_types::{NodeLibrary, Port, PrimitiveValue};
use std::collections::BTreeMap;

fn port(id: &str, signal_type: imp_core_types::SignalType, default_value: Option<PrimitiveValue>) -> Port {
    Port {
        id: id.into(),
        signal_type,
        default_value,
    }
}

/// Returns the Aura time node library.
pub fn time_node_library() -> NodeLibrary {
    let time = signal(TIME);

    let definitions = vec![
        primitive_node(
            "time",
            Vec::new(),
            None,
            BTreeMap::new(),
            BTreeMap::from([(
                "time".into(),
                port("time", time.clone(), None),
            )]),
        ),
        primitive_node(
            "time_elapsed",
            Vec::new(),
            None,
            BTreeMap::from([
                ("time".into(), port("time", time.clone(), None)),
                ("start".into(), port("start", time.clone(), None)),
            ]),
            BTreeMap::from([(
                "elapsed".into(),
                port("elapsed", time, None),
            )]),
        ),
    ];

    NodeLibrary {
        id: "aura.time".into(),
        definitions,
    }
}
