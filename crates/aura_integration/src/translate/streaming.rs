//! Streaming implementations of Aura node types, and streaming programs.
//!
//! Every node type has a deterministic implementation (`translate_deterministic`); a node type
//! listed in [`streaming_implementation`] also has a streaming one. In streaming mode the
//! translator builds the streaming implementation when there is one, and the deterministic one
//! otherwise. Both build the same runtime form (`Sampler::at(t)`); in streaming mode `t` only
//! moves forward and is voice-local (the live note's onset is `t = 0`).
//!
//! Note-reading implementations act on the live note only when their score input carries it
//! (`ScoreSignal::live`). A fixed score (for example an `arpeggio`) is fully known in either mode,
//! so those nodes then use their deterministic implementation unchanged.

use super::{
    resolve_score_signal_input, signal_input, translate_deterministic, Bindings, NodeValue,
    PreparedGraph, TargetKey, TranslateError, TranslateMode,
};
use aura_dsp::sine;
use aura_imp::{Graph, GraphSource, Node, NodeId, PortReference, Registry, NOTATION_NODE_TYPE};
use aura_instrumentation::LinearAdsr;
use aura_render::Sampler;
use aura_sample::SampleRate;
use aura_sequencing::{LiveNote, ScoreSignal};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Everything a node implementation needs to translate one node.
pub(super) struct NodeCtx<'a, 'b> {
    pub(super) graph: &'a Graph,
    pub(super) registry: &'a Registry,
    pub(super) sample_rate: SampleRate,
    pub(super) edges_by_target: &'a BTreeMap<TargetKey, Vec<PortReference>>,
    pub(super) visiting: &'b mut BTreeSet<NodeId>,
    pub(super) bindings: &'a Bindings<'a>,
    pub(super) node: &'a Node,
    pub(super) output_port: &'a str,
}

impl NodeCtx<'_, '_> {
    fn signal(&mut self, port: &str) -> Result<Box<dyn Sampler>, TranslateError> {
        signal_input(
            self.graph,
            self.registry,
            self.sample_rate,
            self.edges_by_target,
            self.visiting,
            self.bindings,
            &self.node.id,
            port,
        )
    }

    fn score(&mut self, port: &str) -> Result<ScoreSignal, TranslateError> {
        resolve_score_signal_input(
            self.graph,
            self.registry,
            self.sample_rate,
            self.edges_by_target,
            self.visiting,
            self.bindings,
            &self.node.id,
            port,
        )
    }

    /// The live note carried by the `score` input, if any.
    fn live_score(&mut self) -> Result<Option<Arc<LiveNote>>, TranslateError> {
        Ok(self.score("score")?.live)
    }

    fn deterministic(&mut self) -> Result<NodeValue, TranslateError> {
        translate_deterministic(
            self.graph,
            self.registry,
            self.sample_rate,
            self.edges_by_target,
            self.visiting,
            self.bindings,
            self.node,
            self.output_port,
        )
    }

    fn missing_port(&self) -> TranslateError {
        TranslateError::MissingOutputPort {
            node: self.node.id.clone(),
            port: self.output_port.into(),
        }
    }
}

type StreamingImpl = fn(&mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError>;

/// The streaming implementation of a node type, when it has one.
pub(super) fn streaming_implementation(node_type: &str) -> Option<StreamingImpl> {
    Some(match node_type {
        NOTATION_NODE_TYPE => stream_notation,
        "note_at_time" => stream_note_at_time,
        "note_param_at_time" => stream_note_param_at_time,
        "linear_adsr" => stream_linear_adsr,
        "note_envelope" => stream_note_envelope,
        "poly_sine_score" => stream_poly_sine_score,
        _ => return None,
    })
}

/// A notation node is the voice's live note.
fn stream_notation(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    if ctx.output_port != "score" {
        return Err(ctx.missing_port());
    }
    match ctx.bindings.live() {
        Some(live) => Ok(NodeValue::ScoreSignal(ScoreSignal::live(Arc::clone(live)))),
        None => ctx.deterministic(),
    }
}

/// Note facts from the live note; `note_duration` is the duration so far.
fn stream_note_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let Some(live) = ctx.live_score()? else {
        return ctx.deterministic();
    };
    let field = match ctx.output_port {
        "semitone" => LiveField::Semitone,
        "active" => LiveField::Held,
        "velocity" => LiveField::Velocity,
        "note_start" => LiveField::Onset,
        "note_duration" => LiveField::DurationSoFar,
        _ => return Err(ctx.missing_port()),
    };
    Ok(NodeValue::Signal(Box::new(LiveFieldSampler {
        live,
        field,
    })))
}

fn stream_note_param_at_time(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    if ctx.output_port != "value" {
        return Err(ctx.missing_port());
    }
    let Some(live) = ctx.live_score()? else {
        return ctx.deterministic();
    };
    let param_name = super::read_string_input(ctx.node, "param", "cutoff_mult")?;
    // Same default as the deterministic implementation for a missing key.
    let value = live.param_or(&param_name, 1.0) as f32;
    Ok(NodeValue::Signal(Box::new(super::ConstantSampler(value))))
}

/// Gated envelope: the same A/D/S/R, released by the voice's note-off instead of placed inside
/// a known note length. The `note_duration` input is not read.
///
/// Applies only when the envelope times a live note (its `elapsed` or `note_duration` input
/// depends on a notation node). An envelope timed by a fixed score keeps its deterministic
/// implementation, whose note lengths are known.
fn stream_linear_adsr(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let live = ctx.bindings.live().cloned().filter(|_| {
        ["elapsed", "note_duration"]
            .iter()
            .any(|port| depends_on_notation(ctx.graph, ctx.edges_by_target, &ctx.node.id, port))
    });
    let Some(live) = live else {
        return ctx.deterministic();
    };
    Ok(NodeValue::Signal(Box::new(GatedAdsrSampler {
        attack: ctx.signal("attack")?,
        decay: ctx.signal("decay")?,
        sustain: ctx.signal("sustain")?,
        release: ctx.signal("release")?,
        elapsed: ctx.signal("elapsed")?,
        live,
    })))
}

fn stream_note_envelope(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let Some(live) = ctx.live_score()? else {
        return ctx.deterministic();
    };
    Ok(NodeValue::Signal(Box::new(LiveEnvelopeSampler {
        live,
        adsr: LinearAdsr::default(),
    })))
}

fn stream_poly_sine_score(ctx: &mut NodeCtx<'_, '_>) -> Result<NodeValue, TranslateError> {
    let Some(live) = ctx.live_score()? else {
        return ctx.deterministic();
    };
    // Translated for parity with the deterministic implementation, which also ignores it.
    let _time = ctx.signal("time")?;
    Ok(NodeValue::Signal(Box::new(LiveSineSampler {
        frequency: live.semitone.to_hz() as f32,
        live,
        adsr: LinearAdsr::default(),
    })))
}

/// Whether the value wired into `node_id.port` is computed from a notation node (the live note
/// in streaming mode), following edges upstream.
fn depends_on_notation(
    graph: &Graph,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    node_id: &str,
    port: &str,
) -> bool {
    let mut pending: Vec<NodeId> = edges_by_target
        .get(&(node_id.to_string(), port.to_string()))
        .into_iter()
        .flatten()
        .map(|source| source.node.clone())
        .collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(node) = graph.nodes.get(&id) else {
            continue;
        };
        if node.node_type == NOTATION_NODE_TYPE {
            return true;
        }
        pending.extend(
            edges_by_target
                .iter()
                .filter(|((target, _), _)| *target == id)
                .flat_map(|(_, sources)| sources.iter().map(|source| source.node.clone())),
        );
    }
    false
}

#[derive(Debug, Clone, Copy)]
enum LiveField {
    Semitone,
    Held,
    Velocity,
    Onset,
    DurationSoFar,
}

struct LiveFieldSampler {
    live: Arc<LiveNote>,
    field: LiveField,
}

impl Sampler for LiveFieldSampler {
    fn at(&self, t: f64) -> f32 {
        match self.field {
            LiveField::Semitone => f32::from(self.live.semitone.0),
            LiveField::Held => {
                if self.live.is_held() {
                    1.0
                } else {
                    0.0
                }
            }
            LiveField::Velocity => self.live.velocity,
            LiveField::Onset => 0.0,
            LiveField::DurationSoFar => self.live.duration_so_far(t) as f32,
        }
    }
}

struct GatedAdsrSampler {
    attack: Box<dyn Sampler>,
    decay: Box<dyn Sampler>,
    sustain: Box<dyn Sampler>,
    release: Box<dyn Sampler>,
    elapsed: Box<dyn Sampler>,
    live: Arc<LiveNote>,
}

impl Sampler for GatedAdsrSampler {
    fn at(&self, t: f64) -> f32 {
        let adsr = LinearAdsr {
            attack: f64::from(self.attack.at(t)),
            decay: f64::from(self.decay.at(t)),
            sustain: self.sustain.at(t),
            release: f64::from(self.release.at(t)),
        };
        adsr.gated_value_at(f64::from(self.elapsed.at(t)), self.live.released_after())
    }
}

struct LiveEnvelopeSampler {
    live: Arc<LiveNote>,
    adsr: LinearAdsr,
}

impl Sampler for LiveEnvelopeSampler {
    fn at(&self, t: f64) -> f32 {
        self.adsr.gated_value_at(t, self.live.released_after())
    }
}

struct LiveSineSampler {
    live: Arc<LiveNote>,
    frequency: f32,
    adsr: LinearAdsr,
}

impl Sampler for LiveSineSampler {
    fn at(&self, t: f64) -> f32 {
        let env = self.adsr.gated_value_at(t, self.live.released_after());
        sine(self.frequency, t) * env * self.live.velocity
    }
}

/// A voice graph prepared once for streaming mode, instantiated once per live note.
///
/// The graph is type-checked and flattened at [`prepare`](Self::prepare); each
/// [`instantiate`](Self::instantiate) builds a fresh runtime graph bound to one live note, whose
/// notation nodes resolve to that note.
#[derive(Debug, Clone)]
pub struct StreamingProgram {
    prepared: PreparedGraph,
    registry: Registry,
    sample_rate: SampleRate,
}

impl StreamingProgram {
    /// Prepares `source` for streaming. Translation problems surface here (via a trial
    /// instance) rather than at the first note-on.
    pub fn prepare<'a>(
        source: impl Into<GraphSource<'a>>,
        registry: &Registry,
        sample_rate: SampleRate,
    ) -> Result<Self, TranslateError> {
        let program = Self {
            prepared: PreparedGraph::prepare(source, registry)?,
            registry: registry.clone(),
            sample_rate,
        };
        let trial = Arc::new(LiveNote::new(
            aura_sequencing::Semitone(60),
            1.0,
            Default::default(),
        ));
        program.instantiate(trial)?;
        Ok(program)
    }

    /// A runtime graph for one voice sounding `live` (onset at `t = 0`).
    pub fn instantiate(&self, live: Arc<LiveNote>) -> Result<Box<dyn Sampler>, TranslateError> {
        let scores = BTreeMap::new();
        let bindings = Bindings {
            scores: &scores,
            mode: TranslateMode::Streaming { live },
        };
        self.prepared
            .translate(&self.registry, self.sample_rate, &bindings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::{translate_graph, translate_graph_with_bindings};
    use aura_imp::{
        aura_registry_with_libraries, graph_from_json_str, graph_library_from_json_path,
    };
    use aura_sequencing::{NoteParams, Semitone};
    use std::path::PathBuf;

    const RATE: SampleRate = SampleRate::RATE_44100;

    fn workspace(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    fn pack_registry() -> Registry {
        let library =
            graph_library_from_json_path(&workspace("lab-pack/catalogs/instruments.json"))
                .expect("instruments library");
        aura_registry_with_libraries(&[library]).expect("registry")
    }

    /// `notation → <voice>.score`, `time → <voice>.time`, `<voice>.audio → out`.
    fn voice_graph(voice_type: &str) -> Graph {
        let json = format!(
            r#"{{
              "nodes": {{
                "n": {{ "id": "n", "type": "{NOTATION_NODE_TYPE}", "inputs": {{}} }},
                "t": {{ "id": "t", "type": "time", "inputs": {{}} }},
                "v": {{ "id": "v", "type": "{voice_type}", "inputs": {{}} }},
                "o": {{ "id": "o", "type": "output", "inputs": {{}}, "title": "out" }}
              }},
              "edges": {{
                "e1": {{ "id": "e1", "from": {{ "node": "n", "port": "score" }}, "to": {{ "node": "v", "port": "score" }} }},
                "e2": {{ "id": "e2", "from": {{ "node": "t", "port": "time" }}, "to": {{ "node": "v", "port": "time" }} }},
                "e3": {{ "id": "e3", "from": {{ "node": "v", "port": "audio" }}, "to": {{ "node": "o", "port": "value" }} }}
              }}
            }}"#
        );
        graph_from_json_str(&json).expect("voice graph")
    }

    fn live(semitone: i16) -> Arc<LiveNote> {
        Arc::new(LiveNote::new(Semitone(semitone), 1.0, NoteParams::new()))
    }

    fn peak(sampler: &dyn Sampler, from: f64, to: f64) -> f32 {
        let rate = f64::from(RATE.get());
        let start = (from * rate) as u64;
        let end = (to * rate) as u64;
        (start..end)
            .map(|frame| sampler.at(frame as f64 / rate).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn sine_note_voice_sounds_while_held_and_rings_out_after_release() {
        let registry = pack_registry();
        let program = StreamingProgram::prepare(&voice_graph("sine_note_voice"), &registry, RATE)
            .expect("sine_note_voice streams");
        let note = live(69);
        let voice = program.instantiate(Arc::clone(&note)).expect("instance");

        // Held far longer than any note length the voice could know about.
        assert!(peak(voice.as_ref(), 1.9, 2.0) > 0.5, "sustains while held");
        note.release(2.0);
        assert!(
            peak(voice.as_ref(), 2.0, 2.01) > 0.1,
            "release rings past note-off"
        );
        assert_eq!(
            peak(voice.as_ref(), 2.06, 2.2),
            0.0,
            "silent after the release"
        );
    }

    #[test]
    fn kick_drum_streams_with_its_envelope_gated_by_note_off() {
        let registry = pack_registry();
        let program = StreamingProgram::prepare(&voice_graph("kick_drum"), &registry, RATE)
            .expect("kick_drum streams");
        let note = live(36);
        let voice = program.instantiate(Arc::clone(&note)).expect("instance");
        assert!(peak(voice.as_ref(), 0.0, 0.05) > 1e-3, "audible attack");
        note.release(0.1);
        assert_eq!(peak(voice.as_ref(), 2.0, 2.1), 0.0);
    }

    #[test]
    fn live_note_duration_grows_while_held_and_freezes_at_release() {
        let note = live(60);
        let sampler = LiveFieldSampler {
            live: Arc::clone(&note),
            field: LiveField::DurationSoFar,
        };
        assert!((sampler.at(0.3) - 0.3).abs() < 1e-6);
        note.release(0.5);
        assert!((sampler.at(3.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn fixed_scores_use_deterministic_implementations_in_streaming() {
        // demo-01 is built only from fixed scores (arpeggio, bass line, drum grids, loops), and
        // its envelopes are timed by them. None of that depends on a live note, so in streaming
        // every node uses its deterministic implementation and the output is identical.
        let loaded = crate::load_graph(&workspace("demos/demo-01.json")).expect("demo-01");
        let deterministic =
            translate_graph_with_bindings(&loaded.graph, &loaded.registry, RATE, &BTreeMap::new())
                .expect("deterministic");
        assert!(
            peak(deterministic.as_ref(), 0.0, 2.0) > 1e-3,
            "demo-01 is audible"
        );
        let streaming = StreamingProgram::prepare(&loaded.graph, &loaded.registry, RATE)
            .expect("streams")
            .instantiate(live(60))
            .expect("instance");
        for frame in 0..8000 {
            let t = frame as f64 / 2000.0;
            assert_eq!(deterministic.at(t), streaming.at(t), "t={t}");
        }
    }

    #[test]
    fn mode_agnostic_graphs_sample_identically_in_both_modes() {
        let json = std::fs::read_to_string(workspace("demos/sine.json")).expect("sine.json");
        let graph = graph_from_json_str(&json).expect("graph");
        let registry = aura_registry_with_libraries(&[]).expect("registry");
        let deterministic = translate_graph(&graph, &registry, RATE).expect("deterministic");
        let streaming = StreamingProgram::prepare(&graph, &registry, RATE)
            .expect("streaming")
            .instantiate(live(60))
            .expect("instance");
        for frame in 0..4410 {
            let t = frame as f64 / f64::from(RATE.get());
            assert_eq!(deterministic.at(t), streaming.at(t));
        }
    }
}
