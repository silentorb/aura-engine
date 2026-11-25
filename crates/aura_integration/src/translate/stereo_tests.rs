//! Stereo: layout inference, the implementation matrix's dual-mono cells, converters, streaming
//! voices, and generic graph-backed definitions. Design: `docs/design/audio-layouts.md`.

use super::{translate_graph, StreamingProgram, TranslateError};
use aura_dsp::{pan_gains, sine};
use aura_imp::constraints::AUDIO;
use aura_imp::{
    aura_registry, aura_registry_with_libraries, graph_from_json_str, Graph, GraphLibrary,
    NodeDefinition, Port,
};
use aura_render::{AudioSignal, Layout};
use aura_sample::SampleRate;
use aura_sequencing::{LiveNote, Semitone};
use imp_core_types::{type_var, BoundaryBindings, TypeParam};
use std::collections::BTreeMap;
use std::sync::Arc;

const RATE: SampleRate = SampleRate::RATE_44100;

/// A graph from `(id, type, literal inputs JSON)` nodes and `(from, from port, to, to port,
/// order)` edges.
fn graph(nodes: &[(&str, &str, &str)], edges: &[(&str, &str, &str, &str, Option<i64>)]) -> Graph {
    let nodes: Vec<String> = nodes
        .iter()
        .map(|(id, node_type, inputs)| {
            // The graph output is titled `out`, so inlined definition outputs do not compete.
            let title = if *id == "out" { r#", "title": "out""# } else { "" };
            format!(
                r#""{id}": {{ "id": "{id}", "type": "{node_type}", "inputs": {{ {inputs} }}{title} }}"#
            )
        })
        .collect();
    let edges: Vec<String> = edges
        .iter()
        .enumerate()
        .map(|(index, (from, from_port, to, to_port, order))| {
            let order = order.map_or(String::new(), |order| format!(r#", "order": {order}"#));
            format!(
                r#""e{index}": {{ "id": "e{index}", "from": {{ "node": "{from}", "port": "{from_port}" }}, "to": {{ "node": "{to}", "port": "{to_port}" }}{order} }}"#
            )
        })
        .collect();
    graph_from_json_str(&format!(
        r#"{{ "nodes": {{ {} }}, "edges": {{ {} }} }}"#,
        nodes.join(", "),
        edges.join(", ")
    ))
    .expect("graph")
}

/// `time`, then a sine at `hz` panned to `position`, as nodes `{prefix}s` and `{prefix}p`.
fn panned_sine(prefix: &str, hz: f64, position: f64) -> Vec<(String, String, String)> {
    vec![
        (
            format!("{prefix}s"),
            "sine".into(),
            format!(r#""frequency": {hz}"#),
        ),
        (
            format!("{prefix}p"),
            "pan".into(),
            format!(r#""position": {position}"#),
        ),
    ]
}

fn owned_graph(
    nodes: &[(String, String, String)],
    edges: &[(String, &str, String, &str, Option<i64>)],
) -> Graph {
    let nodes: Vec<(&str, &str, &str)> = nodes
        .iter()
        .map(|(id, node_type, inputs)| (id.as_str(), node_type.as_str(), inputs.as_str()))
        .collect();
    let edges: Vec<(&str, &str, &str, &str, Option<i64>)> = edges
        .iter()
        .map(|(from, from_port, to, to_port, order)| {
            (from.as_str(), *from_port, to.as_str(), *to_port, *order)
        })
        .collect();
    graph(&nodes, &edges)
}

fn stereo(signal: AudioSignal) -> AudioSignal {
    assert_eq!(signal.layout(), Layout::Stereo, "expected a stereo graph");
    signal
}

/// The frame at `t` seconds (on the 44.1 kHz frame grid).
fn at(signal: &mut AudioSignal, t: f64) -> [f32; 2] {
    let frame = (t * 44_100.0).round() as u64;
    aura_render::render_range(signal, frame, 1)[0]
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
}

/// Two sines panned hard left and right, summed by the generic `add` and halved by the
/// generic `multiply`: both resolve to stereo and run as dual mono.
#[test]
fn panned_sines_mix_in_stereo() {
    let mut nodes = vec![("t".to_string(), "time".to_string(), String::new())];
    nodes.extend(panned_sine("a", 440.0, -1.0));
    nodes.extend(panned_sine("b", 660.0, 1.0));
    nodes.extend([
        ("mix".into(), "add".into(), String::new()),
        ("gain".into(), "multiply".into(), r#""b": 0.5"#.into()),
        ("out".into(), "output".into(), String::new()),
    ]);
    let edges = [
        ("t".to_string(), "time", "as".to_string(), "time", None),
        ("t".into(), "time", "bs".into(), "time", None),
        ("as".into(), "audio", "ap".into(), "audio", None),
        ("bs".into(), "audio", "bp".into(), "audio", None),
        ("ap".into(), "audio", "mix".into(), "inputs", Some(0)),
        ("bp".into(), "audio", "mix".into(), "inputs", Some(1)),
        ("mix".into(), "audio", "gain".into(), "a", None),
        ("gain".into(), "audio", "out".into(), "value", None),
    ];
    let graph = owned_graph(&nodes, &edges);
    let mut signal = stereo(translate_graph(&graph, &aura_registry().unwrap(), RATE).expect("translate"));
    let (hard_left, _) = pan_gains(-1.0);
    for frame in (0..2000).step_by(37) {
        let t = f64::from(frame) / 44_100.0;
        let expected = [
            0.5 * hard_left * sine(440.0, t),
            0.5 * hard_left * sine(660.0, t),
        ];
        assert!(close(at(&mut signal, t), expected), "t={t}");
    }
}

#[test]
fn mono_and_stereo_into_one_parameter_is_a_type_error() {
    let mut nodes = vec![
        ("t".to_string(), "time".to_string(), String::new()),
        ("m".into(), "sine".into(), String::new()),
    ];
    nodes.extend(panned_sine("b", 660.0, 1.0));
    nodes.extend([
        ("mix".into(), "add".into(), String::new()),
        ("out".into(), "output".into(), String::new()),
    ]);
    let edges = [
        ("t".to_string(), "time", "m".to_string(), "time", None),
        ("t".into(), "time", "bs".into(), "time", None),
        ("bs".into(), "audio", "bp".into(), "audio", None),
        ("m".into(), "audio", "mix".into(), "inputs", Some(0)),
        ("bp".into(), "audio", "mix".into(), "inputs", Some(1)),
        ("mix".into(), "audio", "out".into(), "value", None),
    ];
    let error = translate_graph(&owned_graph(&nodes, &edges), &aura_registry().unwrap(), RATE)
        .err()
        .expect("mixed layouts must not translate");
    match error {
        TranslateError::TypeCheck { message } => {
            assert!(message.contains("type parameter A"), "{message}");
            assert!(message.contains("audio_mono") && message.contains("audio_stereo"), "{message}");
        }
        other => panic!("expected a type error, got {other:?}"),
    }
}

/// A chain of dual-mono nodes reads each upstream channel once per frame. If a channel view
/// computed both channels, 40 stages would cost 2^40 evaluations per frame.
#[test]
fn dual_mono_chains_stay_linear() {
    let depth = 40;
    let mut nodes = vec![("t".to_string(), "time".to_string(), String::new())];
    nodes.extend(panned_sine("a", 440.0, -0.5));
    let mut edges = vec![
        ("t".to_string(), "time", "as".to_string(), "time", None),
        ("as".into(), "audio", "ap".into(), "audio", None),
    ];
    let mut previous = "ap".to_string();
    for stage in 0..depth {
        let id = format!("g{stage}");
        nodes.push((id.clone(), "multiply".into(), r#""b": 1.0"#.into()));
        edges.push((previous, "audio", id.clone(), "a", None));
        previous = id;
    }
    nodes.push(("out".into(), "output".into(), String::new()));
    edges.push((previous, "audio", "out".into(), "value", None));
    let mut signal = stereo(
        translate_graph(&owned_graph(&nodes, &edges), &aura_registry().unwrap(), RATE)
            .expect("translate"),
    );
    let (left, right) = pan_gains(-0.5);
    let t = 44.0 / 44_100.0;
    let value = sine(440.0, t);
    assert!(close(at(&mut signal, t), [value * left, value * right]));
}

#[test]
fn split_and_join_round_trip() {
    let mut nodes = vec![("t".to_string(), "time".to_string(), String::new())];
    nodes.extend(panned_sine("a", 440.0, 0.3));
    nodes.extend([
        ("split".into(), "stereo_split".into(), String::new()),
        ("join".into(), "stereo_join".into(), String::new()),
        ("out".into(), "output".into(), String::new()),
    ]);
    let edges = [
        ("t".to_string(), "time", "as".to_string(), "time", None),
        ("as".into(), "audio", "ap".into(), "audio", None),
        ("ap".into(), "audio", "split".into(), "audio", None),
        ("split".into(), "right", "join".into(), "left", None),
        ("split".into(), "left", "join".into(), "right", None),
        ("join".into(), "audio", "out".into(), "value", None),
    ];
    let mut signal = stereo(
        translate_graph(&owned_graph(&nodes, &edges), &aura_registry().unwrap(), RATE)
            .expect("translate"),
    );
    let (left, right) = pan_gains(0.3);
    let t = 88.0 / 44_100.0;
    let value = sine(440.0, t);
    assert!(close(at(&mut signal, t), [value * right, value * left]), "channels swapped");
}

#[test]
fn stereo_to_mono_averages() {
    let mut nodes = vec![("t".to_string(), "time".to_string(), String::new())];
    nodes.extend(panned_sine("a", 440.0, -1.0));
    nodes.extend([
        ("down".into(), "stereo_to_mono".into(), String::new()),
        ("out".into(), "output".into(), String::new()),
    ]);
    let edges = [
        ("t".to_string(), "time", "as".to_string(), "time", None),
        ("as".into(), "audio", "ap".into(), "audio", None),
        ("ap".into(), "audio", "down".into(), "audio", None),
        ("down".into(), "audio", "out".into(), "value", None),
    ];
    let mut signal = translate_graph(&owned_graph(&nodes, &edges), &aura_registry().unwrap(), RATE)
        .expect("translate");
    assert_eq!(signal.layout(), Layout::Mono);
    let t = 132.0 / 44_100.0;
    let expected = 0.5 * pan_gains(-1.0).0 * sine(440.0, t);
    assert!((at(&mut signal, t)[0] - expected).abs() < 1e-5);
}

#[test]
fn streaming_voices_play_stereo() {
    let mut nodes = vec![("t".to_string(), "time".to_string(), String::new())];
    nodes.extend(panned_sine("a", 440.0, -1.0));
    nodes.push(("out".into(), "output".into(), String::new()));
    let edges = [
        ("t".to_string(), "time", "as".to_string(), "time", None),
        ("as".into(), "audio", "ap".into(), "audio", None),
        ("ap".into(), "audio", "out".into(), "value", None),
    ];
    let program = StreamingProgram::prepare(
        &owned_graph(&nodes, &edges),
        &aura_registry().unwrap(),
        RATE,
    )
    .expect("prepare");
    let (control, mut source) = program.open().expect("open");
    control
        .note_on(1, Arc::new(LiveNote::new(Semitone(69), 1.0, Default::default())))
        .unwrap();
    let mut block = vec![[0.0; 2]; 512];
    source.render_block(&mut block);
    let left = block.iter().fold(0.0_f32, |m, f| m.max(f[0].abs()));
    let right = block.iter().fold(0.0_f32, |m, f| m.max(f[1].abs()));
    assert!(left > 0.5, "hard-left voice sounds on the left: {left}");
    assert!(right < 1e-6, "and not on the right: {right}");
}

/// `half<A: Audio>(audio: A) → A` = `multiply(audio, 0.5)`, defined once as a graph and used at
/// both layouts in one graph.
#[test]
fn generic_graph_definitions_work_at_both_layouts() {
    let a = type_var("A");
    let port = |id: &str| {
        (
            id.to_string(),
            Port {
                id: id.into(),
                ty: a.clone(),
                default_value: None,
                variadic: false,
            },
        )
    };
    let body = graph(
        &[
            ("in", "input", ""),
            ("gain", "multiply", r#""b": 0.5"#),
            ("result", "output", ""),
        ],
        &[
            ("in", "value", "gain", "a", None),
            ("gain", "audio", "result", "value", None),
        ],
    );
    let half = NodeDefinition {
        id: "half".into(),
        name: None,
        type_params: vec![TypeParam {
            id: "A".into(),
            bounds: vec![AUDIO.into()],
        }],
        implementation: None,
        inputs: BTreeMap::from([port("audio")]),
        outputs: BTreeMap::from([port("audio")]),
        body: Some(body),
        bindings: Some(BoundaryBindings {
            inputs: BTreeMap::from([("audio".into(), "in".into())]),
            outputs: BTreeMap::from([("audio".into(), "result".into())]),
        }),
    };
    let registry = aura_registry_with_libraries(&[GraphLibrary {
        id: "test.generic".into(),
        definitions: vec![half],
    }])
    .expect("a generic graph definition type-checks");

    // Mono: sine → half; stereo: sine → pan → half; joined into one stereo output.
    let graph = graph(
        &[
            ("t", "time", ""),
            ("s", "sine", r#""frequency": 440"#),
            ("mono_half", "half", ""),
            ("p", "pan", r#""position": 1"#),
            ("stereo_half", "half", ""),
            ("down", "stereo_to_mono", ""),
            ("join", "stereo_join", ""),
            ("out", "output", ""),
        ],
        &[
            ("t", "time", "s", "time", None),
            ("s", "audio", "mono_half", "audio", None),
            ("s", "audio", "p", "audio", None),
            ("p", "audio", "stereo_half", "audio", None),
            ("stereo_half", "audio", "down", "audio", None),
            ("mono_half", "audio", "join", "left", None),
            ("down", "audio", "join", "right", None),
            ("join", "audio", "out", "value", None),
        ],
    );
    let mut signal = stereo(translate_graph(&graph, &registry, RATE).expect("translate"));
    let t = 176.0 / 44_100.0;
    let value = sine(440.0, t);
    let hard_right = pan_gains(1.0).1;
    let expected = [0.5 * value, 0.5 * (0.5 * hard_right * value)];
    assert!(close(at(&mut signal, t), expected), "{:?} vs {expected:?}", at(&mut signal, t));}
