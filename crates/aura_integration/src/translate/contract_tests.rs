//! The deterministic contract (`docs/design/imp-execution.md`): a translated graph renders any
//! range, in any order, from a fresh or a used signal, with identical output. Node state is
//! nonessential in deterministic mode; these tests hold every registered node type to that.

use super::{impls, PreparedGraph, TranslateOptions, translate_graph_with_options};
use aura_clap::ClapHost;
use aura_clap_fixture::FixtureEntry;
use aura_imp::{aura_registry, graph_from_json_str, Graph, NodeId, Registry, NOTATION_NODE_TYPE};
use aura_plugin_host::PluginResolver;
use aura_render::{render_range, AudioSignal, Frame};
use aura_sample::SampleRate;
use aura_sequencing::{NoteEvent, Score, ScoreSignal, Semitone, Tempo, TimeSignature};
use clack_host::prelude::PluginEntry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

const RATE: SampleRate = SampleRate::RATE_44100;
/// About 2.3 s: long enough to cross notes, chords, and plugin blocks.
const FRAMES: usize = 100_000;

fn workspace(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
}

fn fixture_host() -> Arc<ClapHost> {
    let host = ClapHost::new(Vec::new());
    let entry = PluginEntry::load_from_clack::<FixtureEntry>(c"/fixture/aura_clap_fixture.clap")
        .expect("fixture entry");
    host.add_entry(entry, "/fixture/aura_clap_fixture.clap");
    Arc::new(host)
}

/// A chord and a melody note for notation nodes.
fn scores() -> BTreeMap<NodeId, ScoreSignal> {
    BTreeMap::from([(
        "n".to_string(),
        ScoreSignal::finite(Score::new(
            TimeSignature::default(),
            Tempo::new(120.0).unwrap(),
            vec![
                NoteEvent::new(Semitone(60), 0.25, 1.0),
                NoteEvent::new(Semitone(64), 0.25, 1.0),
                NoteEvent::new(Semitone(67), 1.5, 2.0),
            ],
        )),
    )])
}

/// Every node type the demos and the Lab pack's instruments leave out, in one mono graph: chord,
/// tempo, and meter readers feed a gain; noise and a polyphonic notation voice, panned, split,
/// swapped, joined, gained in stereo (dual mono), and folded back to mono, join the mix.
fn coverage_graph() -> Graph {
    let nodes = [
        ("n", NOTATION_NODE_TYPE, "{}"),
        ("t", "time", "{}"),
        ("tempo", "constant_tempo", "{}"),
        ("meter", "constant_time_signature", "{}"),
        ("prog", "epic_minor_progression", "{}"),
        ("chord", "chord_at_time", "{}"),
        ("bpm", "tempo_at_time", "{}"),
        ("bpb", "time_signature_at_time", "{}"),
        ("hz", "semitone_to_hz", "{}"),
        ("tone", "sine", "{}"),
        ("ctl1", "multiply_control", "{}"),
        ("ctl2", "multiply_control", "{}"),
        ("ctl3", "multiply_control", r#"{ "b": 0.0005 }"#),
        ("gain", "multiply", "{}"),
        ("noise", "deterministic_noise", r#"{ "seed": 3.0 }"#),
        ("poly", "poly_sine_score", "{}"),
        ("pan", "pan", r#"{ "position": 0.3 }"#),
        ("split", "stereo_split", "{}"),
        ("join", "stereo_join", "{}"),
        ("stereo_gain", "multiply", r#"{ "b": 0.8 }"#),
        ("down", "stereo_to_mono", "{}"),
        ("mix", "add", "{}"),
        ("out", "output", "{}"),
    ];
    let edges: [(&str, &str, &str, &str, Option<i64>); 31] = [
        ("meter", "time_signature", "prog", "time_signature", None),
        ("prog", "progression", "chord", "progression", None),
        ("tempo", "tempo", "chord", "tempo", None),
        ("t", "time", "chord", "time", None),
        ("tempo", "tempo", "bpm", "tempo", None),
        ("t", "time", "bpm", "time", None),
        ("meter", "time_signature", "bpb", "time_signature", None),
        ("t", "time", "bpb", "time", None),
        ("chord", "root", "hz", "semitone", None),
        ("hz", "frequency", "tone", "frequency", None),
        ("t", "time", "tone", "time", None),
        ("chord", "active", "ctl1", "a", None),
        ("bpm", "bpm", "ctl1", "b", None),
        ("ctl1", "value", "ctl2", "a", None),
        ("bpb", "beats_per_bar", "ctl2", "b", None),
        ("ctl2", "value", "ctl3", "a", None),
        ("tone", "audio", "gain", "a", None),
        ("ctl3", "value", "gain", "b", None),
        ("t", "time", "noise", "time", None),
        ("n", "score", "poly", "score", None),
        ("t", "time", "poly", "time", None),
        ("poly", "audio", "pan", "audio", None),
        ("pan", "audio", "split", "audio", None),
        ("split", "right", "join", "left", None),
        ("split", "left", "join", "right", None),
        ("join", "audio", "stereo_gain", "a", None),
        ("stereo_gain", "audio", "down", "audio", None),
        ("gain", "audio", "mix", "inputs", Some(0)),
        ("noise", "audio", "mix", "inputs", Some(1)),
        ("down", "audio", "mix", "inputs", Some(2)),
        ("mix", "audio", "out", "value", None),
    ];
    let nodes: Vec<String> = nodes
        .iter()
        .map(|(id, node_type, inputs)| {
            format!(r#""{id}": {{ "id": "{id}", "type": "{node_type}", "inputs": {inputs} }}"#)
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
    let json = format!(
        r#"{{ "nodes": {{ {} }}, "edges": {{ {} }} }}"#,
        nodes.join(", "),
        edges.join(", ")
    );
    graph_from_json_str(&json).expect("coverage graph")
}

/// `notation → <instrument>.score`, `time → <instrument>.time`, `→ out`.
fn instrument_graph(instrument: &str) -> Graph {
    let json = format!(
        r#"{{
          "nodes": {{
            "n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
            "t": {{ "id": "t", "type": "time", "inputs": {{}} }},
            "v": {{ "id": "v", "type": "{instrument}", "inputs": {{}} }},
            "o": {{ "id": "o", "type": "output", "inputs": {{}}, "title": "out" }}
          }},
          "edges": {{
            "e1": {{ "id": "e1", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "v", "port": "score" }} }},
            "e2": {{ "id": "e2", "from": {{ "node": "t", "port": "time" }}, "to": {{ "node": "v", "port": "time" }} }},
            "e3": {{ "id": "e3", "from": {{ "node": "v", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}
          }}
        }}"#
    );
    graph_from_json_str(&json).expect("instrument graph")
}

/// A graph and how to translate it.
struct Case {
    name: String,
    graph: Graph,
    registry: Registry,
    plugins: Option<Arc<ClapHost>>,
}

impl Case {
    fn translate(&self) -> AudioSignal {
        let scores = scores();
        let plugins = self.plugins.as_deref().map(|host| host as &dyn PluginResolver);
        translate_graph_with_options(
            &self.graph,
            &self.registry,
            RATE,
            TranslateOptions {
                score_bindings: &scores,
                plugins,
            },
        )
        .unwrap_or_else(|error| panic!("{}: {error}", self.name))
    }

    /// Node types in the flattened graph (graph-backed definitions expanded).
    fn node_types(&self) -> BTreeSet<String> {
        let prepared = PreparedGraph::prepare(&self.graph, &self.registry).expect("prepare");
        prepared
            .graph
            .nodes
            .values()
            .map(|node| node.node_type.clone())
            .collect()
    }
}

fn cases() -> Vec<Case> {
    let demo = |name: &str| {
        let loaded = crate::load_graph(&workspace(&format!("demos/{name}"))).expect(name);
        Case {
            name: name.into(),
            graph: loaded.graph,
            registry: loaded.registry,
            plugins: Some(fixture_host()),
        }
    };
    let pack = {
        let library = aura_imp::graph_library_from_json_path(&workspace(
            "lab-pack/catalogs/instruments.json",
        ))
        .expect("instruments library");
        aura_imp::aura_registry_with_libraries(&[library]).expect("registry")
    };
    let mut cases = vec![
        demo("demo-01.json"),
        demo("demo-03-plugin.json"),
        demo("demo-04-stereo.json"),
        Case {
            name: "coverage".into(),
            graph: coverage_graph(),
            registry: aura_registry().expect("registry"),
            plugins: None,
        },
    ];
    for instrument in ["sine_note_voice", "kick_drum", "snare_drum", "bass_saw_voice"] {
        cases.push(Case {
            name: instrument.into(),
            graph: instrument_graph(instrument),
            registry: pack.clone(),
            plugins: None,
        });
    }
    cases
}

/// Renders `ranges` (start, length) into a buffer of `FRAMES` frames with one signal.
fn render_ranges(signal: &mut AudioSignal, ranges: &[(usize, usize)]) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0; 2]; FRAMES];
    for &(start, len) in ranges {
        let frames = render_range(signal, start as Frame, len);
        out[start..start + len].copy_from_slice(&frames);
    }
    out
}

/// Ranges covering `0..FRAMES` in chunks of the given sizes, cycling.
fn chunks(sizes: &[usize]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for size in sizes.iter().cycle() {
        if start >= FRAMES {
            break;
        }
        let len = (*size).min(FRAMES - start);
        ranges.push((start, len));
        start += len;
    }
    ranges
}

/// A fixed shuffle (no randomness in tests).
fn shuffled(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let len = ranges.len();
    for i in 0..len {
        let j = (i * 7919 + 13) % len;
        ranges.swap(i, j);
    }
    ranges
}

fn assert_identical(name: &str, how: &str, got: &[[f32; 2]], expected: &[[f32; 2]]) {
    if let Some(frame) = (0..FRAMES).find(|&f| {
        got[f][0].to_bits() != expected[f][0].to_bits()
            || got[f][1].to_bits() != expected[f][1].to_bits()
    }) {
        panic!(
            "{name}: {how} differs at frame {frame}: {:?} vs {:?}",
            got[frame], expected[frame]
        );
    }
}

#[test]
fn every_deterministic_graph_renders_any_range_in_any_order_identically() {
    for case in cases() {
        let name = case.name.as_str();
        let mut reference = case.translate();
        let expected = render_ranges(&mut reference, &chunks(&[FRAMES]));
        assert!(
            expected.iter().flatten().any(|s| s.abs() > 1e-4),
            "{name}: audible over the test range"
        );

        let uneven = chunks(&[1, 7, 256, 100, 255, 3, 1024]);
        let mut chunked = case.translate();
        assert_identical(name, "uneven chunks", &render_ranges(&mut chunked, &uneven), &expected);

        let mut out_of_order = case.translate();
        let order = shuffled(chunks(&[997, 64, 4096]));
        assert_identical(name, "shuffled ranges", &render_ranges(&mut out_of_order, &order), &expected);

        // The same used signal again, backwards: state left over from before changes nothing.
        let mut backwards = order.clone();
        backwards.reverse();
        assert_identical(
            name,
            "a used signal rendering again",
            &render_ranges(&mut out_of_order, &backwards),
            &expected,
        );

        // A fresh signal for every range.
        let mut fresh = vec![[0.0; 2]; FRAMES];
        for &(start, len) in chunks(&[9_973]).iter().rev() {
            let frames = render_range(&mut case.translate(), start as Frame, len);
            fresh[start..start + len].copy_from_slice(&frames);
        }
        assert_identical(name, "fresh signals", &fresh, &expected);
    }
}

/// Holds the contract test to every node type: a new node type must appear in one of its
/// graphs.
#[test]
fn contract_graphs_cover_every_registered_node_type() {
    let covered: BTreeSet<String> = cases().iter().flat_map(Case::node_types).collect();
    let missing: Vec<&str> = impls::node_impls()
        .keys()
        .copied()
        .filter(|node_type| !covered.contains(*node_type))
        .collect();
    assert!(missing.is_empty(), "node types without a contract graph: {missing:?}");
}

/// Stereo layouts of the generic node types (`add`, `multiply`, `plugin_effect`): dual mono and
/// true stereo cells.
#[test]
fn stereo_cells_are_covered() {
    let stereo: Vec<(String, String)> = cases()
        .iter()
        .flat_map(|case| {
            let prepared = PreparedGraph::prepare(&case.graph, &case.registry).expect("prepare");
            prepared
                .layouts
                .iter()
                .filter(|(_, layout)| **layout == aura_render::Layout::Stereo)
                .filter_map(|(id, _)| prepared.graph.nodes.get(id))
                .map(|node| (case.name.clone(), node.node_type.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    for node_type in ["add", "multiply"] {
        assert!(
            stereo.iter().any(|(_, t)| t == node_type),
            "no contract graph runs `{node_type}` in stereo: {stereo:?}"
        );
    }
}
