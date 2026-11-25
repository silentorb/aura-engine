//! Imp graph → Sampler translation.

mod impls;
mod nodes;
mod note_schedule;
mod plugin;
mod streaming;
#[cfg(test)]
mod stereo_tests;

pub use plugin::plugin_node_settings;
pub use streaming::{StreamingControl, StreamingError, StreamingProgram};

use note_schedule::NoteSchedule;
use aura_composition::{
    arpeggio, bass_line, drum_grid, epic_minor_progression, ArpeggioConfig, BassLineConfig,
    DrumGridConfig, DrumLane,
};
use aura_sequencing::{
    LiveNote,
    ChordProgression, ChordSignal, ProgressionSignal, Score, ScoreSignal, Semitone, Tempo,
    TimeSignature,
};
use aura_dsp::{
    add, bandlimited_saw, deterministic_noise, exponential_sweep_sine, highpass_noise, multiply,
    sine,
};
use aura_imp::{
    flatten_graph, Graph, GraphSource, Node, NodeId, PortReference, PrimitiveValue, Registry,
    NOTATION_NODE_TYPE,
};
use aura_instrumentation::LinearAdsr;
use aura_plugin_host::PluginResolver;
use aura_render::{AudioSampler, Layout, Sampler, StereoSampler};
use aura_sample::SampleRate;
use aura_imp::constraints::AUDIO;
use aura_imp::signals::AUDIO_STEREO;
use aura_imp::{get_node_definition, SignalType};
use imp_typecheck::{check_graph, resolve_type_args};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use thiserror::Error;

type TargetKey = (NodeId, String);

#[derive(Debug, Error, PartialEq)]
pub enum TranslateError {
    #[error("graph has no output node")]
    MissingOutput,
    #[error("multiple output nodes are not supported")]
    MultipleOutputs,
    #[error("unknown node `{node}`")]
    UnknownNode { node: NodeId },
    #[error("unknown node type `{node_type}` on node `{node}`")]
    UnknownNodeType { node: NodeId, node_type: String },
    #[error("unwired required input `{port}` on node `{node}`")]
    UnwiredInput { node: NodeId, port: String },
    #[error("singleton input `{port}` on node `{node}` has multiple inbound edges")]
    DuplicateEdgeTarget { node: NodeId, port: String },
    #[error("cycle detected at node `{node}`")]
    CycleDetected { node: NodeId },
    #[error("expected numeric literal on node `{node}` port `{port}`")]
    ExpectedNumber { node: NodeId, port: String },
    #[error("expected string literal on node `{node}` port `{port}`")]
    ExpectedString { node: NodeId, port: String },
    #[error("node `{node}` output `{port}` is not available")]
    MissingOutputPort { node: NodeId, port: String },
    #[error("score input on node `{node}` must connect to a score-producing node")]
    InvalidScoreSource { node: NodeId },
    #[error("progression input on node `{node}` must connect to a chord progression node")]
    InvalidProgressionSource { node: NodeId },
    #[error("tempo input on node `{node}` must connect to a tempo-producing node")]
    InvalidTempoSource { node: NodeId },
    #[error("time signature input on node `{node}` must connect to a time signature node")]
    InvalidTimeSignatureSource { node: NodeId },
    #[error("loopable input on node `{node}` must connect to a score or chord progression node")]
    InvalidLoopableSource { node: NodeId },
    #[error("graph type check failed: {message}")]
    TypeCheck { message: String },
    #[error("graph resolve failed: {message}")]
    Resolve { message: String },
    #[error("notation node `{node}` has no bound ScoreSignal")]
    MissingScoreBinding { node: NodeId },
    #[error("node `{node}` input `{port}` is stereo, but the node resolved to mono")]
    UnexpectedStereo { node: NodeId, port: String },
    #[error("node `{node}` input `{port}` must be stereo")]
    ExpectedStereo { node: NodeId, port: String },
    #[error("plugin node `{node}` names no plugin")]
    MissingPluginId { node: NodeId },
    #[error("plugin node `{node}` needs a plugin host to resolve `{plugin}`")]
    NoPluginHost { node: NodeId, plugin: String },
    #[error("plugin node `{node}` state is not valid base64")]
    InvalidPluginState { node: NodeId },
    #[error("plugin node `{node}` params are invalid: {reason}")]
    InvalidPluginParams { node: NodeId, reason: String },
    #[error("plugin node `{node}`: {source}")]
    Plugin {
        node: NodeId,
        source: aura_plugin_host::PluginHostError,
    },
}

/// Options for Imp graph translation.
#[derive(Clone, Copy)]
pub struct TranslateOptions<'a> {
    /// Scores for notation nodes (`NOTATION_NODE_TYPE`), by NodeId. Hosts read them from the
    /// notation records that the graph source resolved.
    pub score_bindings: &'a BTreeMap<NodeId, ScoreSignal>,
    /// Resolves `plugin_instrument` / `plugin_effect` nodes. Translating a graph with plugin
    /// nodes and no resolver fails with [`TranslateError::NoPluginHost`].
    pub plugins: Option<&'a dyn PluginResolver>,
}

enum NodeValue {
    Signal(Box<dyn Sampler>),
    Stereo(Box<dyn StereoSampler>),
    ScoreSignal(ScoreSignal),
    ProgressionSignal(ProgressionSignal),
    Tempo(Tempo),
    TimeSignature(TimeSignature),
}

/// Polyphonic sine audition for a concrete [`Score`] (no Imp graph / proxy type).
pub fn sampler_from_score(score: Score, sample_rate: SampleRate) -> Box<dyn Sampler> {
    let schedule = NoteSchedule::from_score_signal(&ScoreSignal::finite(score), sample_rate);
    Box::new(PolySineScoreSampler {
        schedule,
        adsr: LinearAdsr::default(),
    })
}

/// Translates an Imp graph into a pure Time → Sample (mono) or Time → Frame (stereo) function.
pub fn translate_graph(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
) -> Result<AudioSampler, TranslateError> {
    translate_graph_with_bindings(graph, registry, sample_rate, &BTreeMap::new())
}

/// Translates an Imp graph with scores for its notation nodes.
pub fn translate_graph_with_bindings(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    score_bindings: &BTreeMap<NodeId, ScoreSignal>,
) -> Result<AudioSampler, TranslateError> {
    translate_graph_with_options(
        graph,
        registry,
        sample_rate,
        TranslateOptions {
            score_bindings,
            plugins: None,
        },
    )
}

/// Translates a graph source (a document plus any providers for nodes it references) for
/// deterministic mode.
pub fn translate_graph_with_options<'a>(
    source: impl Into<GraphSource<'a>>,
    registry: &Registry,
    sample_rate: SampleRate,
    options: TranslateOptions<'_>,
) -> Result<AudioSampler, TranslateError> {
    let prepared = PreparedGraph::prepare(source, registry)?;
    let bindings = Bindings::new(
        options.score_bindings,
        TranslateMode::Deterministic,
        options.plugins,
    );
    let root = prepared.translate(registry, sample_rate, &bindings)?;
    Ok(plugin::prepared_root(root, bindings.caches.take()))
}

/// The execution mode a graph is translated for (see `docs/design/playback-modes.md`).
///
/// Every node type has a deterministic implementation and may have a streaming one. In
/// streaming mode the translator builds a node type's streaming implementation when it has one
/// and its deterministic implementation otherwise. Nodes never check the mode while sampling.
#[derive(Debug, Clone, Default)]
pub(crate) enum TranslateMode {
    /// Random-access sampling with full knowledge of the score.
    #[default]
    Deterministic,
    /// Sequential sampling of one voice instance sounding a live note (onset at `t = 0`).
    Streaming { live: Arc<LiveNote> },
}

/// Translation-wide inputs threaded through node translation: notation scores and the mode.
pub(crate) struct Bindings<'a> {
    pub(crate) scores: &'a BTreeMap<NodeId, ScoreSignal>,
    pub(crate) mode: TranslateMode,
    pub(crate) plugins: Option<&'a dyn PluginResolver>,
    /// Plugin render caches built so far, so the root can prepare them for real-time reading.
    pub(crate) caches: std::cell::RefCell<Vec<aura_plugin_host::RenderCache>>,
    /// Resolved layout of each layout-polymorphic node, set by the graph being translated.
    pub(crate) layouts: std::cell::RefCell<Arc<BTreeMap<NodeId, Layout>>>,
    /// Dual-mono channel passes in progress, innermost last.
    pub(crate) channel_passes: std::cell::RefCell<Vec<impls::ChannelPass>>,
}

impl<'a> Bindings<'a> {
    pub(crate) fn new(
        scores: &'a BTreeMap<NodeId, ScoreSignal>,
        mode: TranslateMode,
        plugins: Option<&'a dyn PluginResolver>,
    ) -> Self {
        Self {
            scores,
            mode,
            plugins,
            caches: Default::default(),
            layouts: Default::default(),
            channel_passes: Default::default(),
        }
    }

    /// The channel pass of a dual-mono `consumer` whose inputs are being translated, if any.
    pub(crate) fn channel_pass(&self, consumer: &str) -> Option<impls::ChannelPass> {
        self.channel_passes
            .borrow()
            .last()
            .filter(|pass| pass.consumer == consumer)
            .cloned()
    }

    fn layout_of(&self, node: &str) -> Layout {
        self.layouts.borrow().get(node).copied().unwrap_or_default()
    }
}

impl Bindings<'_> {
    /// The live note when translating for streaming mode.
    pub(crate) fn live(&self) -> Option<&Arc<LiveNote>> {
        match &self.mode {
            TranslateMode::Deterministic => None,
            TranslateMode::Streaming { live } => Some(live),
        }
    }
}

/// A type-checked, flattened graph with its output node and edge index, ready to translate
/// (once for deterministic mode, or once per voice instance for streaming mode).
#[derive(Debug, Clone)]
pub(crate) struct PreparedGraph {
    graph: Graph,
    output_node: NodeId,
    edges_by_target: BTreeMap<TargetKey, Vec<PortReference>>,
    /// Resolved layout of each layout-polymorphic node.
    layouts: Arc<BTreeMap<NodeId, Layout>>,
}

impl PreparedGraph {
    pub(crate) fn prepare<'a>(
        source: impl Into<GraphSource<'a>>,
        registry: &Registry,
    ) -> Result<Self, TranslateError> {
        let source: GraphSource<'a> = source.into();
        let type_errors = check_graph(source, registry);
        if let Some(error) = type_errors.first() {
            return Err(TranslateError::TypeCheck {
                message: error.message.clone(),
            });
        }
        let graph = flatten_graph(source, registry)
            .map_err(|message| TranslateError::Resolve { message })?;
        let output_node = find_output_node(&graph)?;
        let edges_by_target = index_edges_by_target(&graph);
        let layouts = Arc::new(resolve_layouts(&graph, registry));
        Ok(Self {
            graph,
            output_node,
            edges_by_target,
            layouts,
        })
    }

    pub(crate) fn translate(
        &self,
        registry: &Registry,
        sample_rate: SampleRate,
        bindings: &Bindings<'_>,
    ) -> Result<AudioSampler, TranslateError> {
        self.translate_port(registry, sample_rate, bindings, &self.output_node, "value")
    }

    /// Translates the signal at one node's output port, as if it were the graph's output.
    pub(crate) fn translate_port(
        &self,
        registry: &Registry,
        sample_rate: SampleRate,
        bindings: &Bindings<'_>,
        node: &str,
        port: &str,
    ) -> Result<AudioSampler, TranslateError> {
        *bindings.layouts.borrow_mut() = Arc::clone(&self.layouts);
        let mut visiting = BTreeSet::new();
        match translate_node(
            &self.graph,
            registry,
            sample_rate,
            &self.edges_by_target,
            &mut visiting,
            bindings,
            node,
            port,
        )? {
            NodeValue::Signal(sampler) => Ok(AudioSampler::Mono(sampler)),
            NodeValue::Stereo(sampler) => Ok(AudioSampler::Stereo(sampler)),
            NodeValue::ScoreSignal(_)
            | NodeValue::ProgressionSignal(_)
            | NodeValue::Tempo(_)
            | NodeValue::TimeSignature(_) => Err(TranslateError::MissingOutputPort {
                node: node.into(),
                port: port.into(),
            }),
        }
    }
}

/// Resolves the graph's audio sink: prefer title `out` among `output` nodes,
/// else require exactly one `output` node.
pub fn find_output_node(graph: &Graph) -> Result<NodeId, TranslateError> {
    let outputs: Vec<_> = graph
        .nodes
        .values()
        .filter(|node| node.node_type == "output")
        .collect();

    if let Some(preferred) = outputs
        .iter()
        .find(|node| node.title.as_deref() == Some("out"))
    {
        return Ok(preferred.id.clone());
    }

    match outputs.as_slice() {
        [] => Err(TranslateError::MissingOutput),
        [node] => Ok(node.id.clone()),
        _ => Err(TranslateError::MultipleOutputs),
    }
}

fn index_edges_by_target(graph: &Graph) -> BTreeMap<TargetKey, Vec<PortReference>> {
    let mut edges_by_target: BTreeMap<TargetKey, Vec<(Option<i64>, PortReference)>> =
        BTreeMap::new();
    for edge in graph.edges.values() {
        let key = (edge.to.node.clone(), edge.to.port.clone());
        edges_by_target
            .entry(key)
            .or_default()
            .push((edge.order, edge.from.clone()));
    }
    edges_by_target
        .into_iter()
        .map(|(key, mut sources)| {
            sources.sort_by_key(|(order, _)| *order);
            (
                key,
                sources.into_iter().map(|(_, source)| source).collect(),
            )
        })
        .collect()
}

fn translate_node(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    if !visiting.insert(node_id.to_string()) {
        return Err(TranslateError::CycleDetected {
            node: node_id.into(),
        });
    }

    let Some(node) = graph.nodes.get(node_id) else {
        visiting.remove(node_id);
        return Err(TranslateError::UnknownNode {
            node: node_id.into(),
        });
    };

    // One cell of the node type's implementation matrix: its resolved layout × the mode.
    let Some(matrix) = impls::node_impls().get(node.node_type.as_str()) else {
        visiting.remove(node_id);
        return Err(TranslateError::UnknownNodeType {
            node: node_id.into(),
            node_type: node.node_type.clone(),
        });
    };
    let layout = bindings.layout_of(node_id);
    let mode = match bindings.mode {
        TranslateMode::Deterministic => impls::Mode::Deterministic,
        TranslateMode::Streaming { .. } => impls::Mode::Streaming,
    };
    let result = matrix.cell(layout, mode)(&mut streaming::NodeCtx {
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node,
        output_port,
        layout,
    });
    visiting.remove(node_id);
    result
}

fn translate_note_at_time_port(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    let score_signal = resolve_score_signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "score",
    )?;
    let schedule = NoteSchedule::from_score_signal(&score_signal, sample_rate);

    let sampler: Box<dyn Sampler> = match output_port {
        "semitone" => Box::new(NoteSemitoneSampler { schedule }),
        "active" => Box::new(NoteActiveSampler { schedule }),
        "velocity" => Box::new(NoteVelocitySampler { schedule }),
        "note_start" => Box::new(NoteStartSampler { schedule }),
        "note_duration" => Box::new(NoteDurationSampler { schedule }),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(sampler))
}

fn translate_note_param_at_time_port(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node: &Node,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    if output_port != "value" {
        return Err(TranslateError::MissingOutputPort {
            node: node.id.clone(),
            port: output_port.into(),
        });
    }

    let score_signal = resolve_score_signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "score",
    )?;
    let schedule = NoteSchedule::from_score_signal(&score_signal, sample_rate);
    let param_name = read_string_input(node, "param", "cutoff_mult")?;

    Ok(NodeValue::Signal(Box::new(NoteParamSampler {
        schedule,
        param_name,
    })))
}

fn translate_chord_at_time_port(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    let progression = resolve_progression_signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "progression",
    )?;
    let tempo = resolve_tempo_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "tempo",
    )?;
    let _time = signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "time",
    )?;

    let sampler: Box<dyn Sampler> = match output_port {
        "root" => Box::new(ChordRootSampler { progression, tempo }),
        "active" => Box::new(ChordActiveSampler { progression, tempo }),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(sampler))
}

fn translate_tempo_at_time_port(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    let tempo = resolve_tempo_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "tempo",
    )?;
    let _time = signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "time",
    )?;

    let sampler: Box<dyn Sampler> = match output_port {
        "bpm" => Box::new(TempoBpmSampler { tempo }),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(sampler))
}

fn translate_time_signature_at_time_port(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    output_port: &str,
) -> Result<NodeValue, TranslateError> {
    let time_signature = resolve_time_signature_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "time_signature",
    )?;
    let _time = signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        "time",
    )?;

    let sampler: Box<dyn Sampler> = match output_port {
        "beats_per_bar" => Box::new(BeatsPerBarSampler { time_signature }),
        "beat_unit" => Box::new(BeatUnitSampler { time_signature }),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(sampler))
}

fn translate_loop_value(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
) -> Result<NodeValue, TranslateError> {
    let source = wired_source(edges_by_target, node_id, "value")?;
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::ScoreSignal(signal) => Ok(NodeValue::ScoreSignal(signal.with_looping(true))),
        NodeValue::ProgressionSignal(signal) => {
            Ok(NodeValue::ProgressionSignal(signal.with_looping(true)))
        }
        _ => Err(TranslateError::InvalidLoopableSource {
            node: node_id.into(),
        }),
    }
}

fn resolve_score_signal_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<ScoreSignal, TranslateError> {
    let source = wired_source(edges_by_target, node_id, port)?;
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::ScoreSignal(signal) => Ok(signal),
        _ => Err(TranslateError::InvalidScoreSource {
            node: source.node,
        }),
    }
}

fn resolve_progression_signal_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<ProgressionSignal, TranslateError> {
    let source = wired_source(edges_by_target, node_id, port)?;
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::ProgressionSignal(signal) => Ok(signal),
        _ => Err(TranslateError::InvalidProgressionSource {
            node: source.node,
        }),
    }
}

fn resolve_tempo_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<Tempo, TranslateError> {
    let Some(source) = wired_source_optional(edges_by_target, node_id, port)? else {
        return Ok(Tempo::default());
    };
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::Tempo(tempo) => Ok(tempo),
        _ => Err(TranslateError::InvalidTempoSource {
            node: source.node.clone(),
        }),
    }
}

fn resolve_time_signature_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<TimeSignature, TranslateError> {
    let Some(source) = wired_source_optional(edges_by_target, node_id, port)? else {
        return Ok(TimeSignature::default());
    };
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::TimeSignature(time_signature) => Ok(time_signature),
        _ => Err(TranslateError::InvalidTimeSignatureSource {
            node: source.node.clone(),
        }),
    }
}

fn build_constant_tempo(node: &Node) -> Result<Tempo, TranslateError> {
    Tempo::new(read_number_input(node, "bpm", 120.0)?).map_err(|_| TranslateError::ExpectedNumber {
        node: node.id.clone(),
        port: "bpm".into(),
    })
}

fn build_constant_time_signature(node: &Node) -> Result<TimeSignature, TranslateError> {
    let beats_per_bar = read_number_input(node, "beats_per_bar", 4.0)? as u8;
    let beat_unit = read_number_input(node, "beat_unit", 4.0)? as u8;
    Ok(TimeSignature {
        beats_per_bar,
        beat_unit,
    })
}

fn build_epic_minor_progression(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node: &Node,
) -> Result<ChordProgression, TranslateError> {
    let root = read_number_input(node, "root", Semitone::A3.0 as f64)? as i16;
    let bars_per_chord = read_number_input(node, "bars_per_chord", 1.0)? as u32;
    let time_signature = resolve_time_signature_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "time_signature",
    )?;

    Ok(epic_minor_progression(
        Semitone(root),
        bars_per_chord,
        time_signature,
    ))
}

fn build_arpeggio_score(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node: &Node,
) -> Result<Score, TranslateError> {
    let progression = resolve_progression_signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "progression",
    )?;
    let tempo = resolve_tempo_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "tempo",
    )?;
    let time_signature = resolve_time_signature_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "time_signature",
    )?;
    let subdivision = read_number_input(node, "subdivision", 4.0)? as u8;

    Ok(arpeggio(ArpeggioConfig {
        progression: progression.progression,
        tempo,
        time_signature,
        subdivision,
    }))
}

fn build_bass_line_score(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node: &Node,
) -> Result<Score, TranslateError> {
    let progression = resolve_progression_signal_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "progression",
    )?;
    let tempo = resolve_tempo_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "tempo",
    )?;
    let time_signature = resolve_time_signature_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "time_signature",
    )?;
    let subdivision = read_number_input(node, "subdivision", 8.0)? as u8;
    let octave_offset = read_number_input(node, "octave_offset", -24.0)? as i16;

    Ok(bass_line(BassLineConfig {
        progression: progression.progression,
        tempo,
        time_signature,
        subdivision,
        octave_offset,
    }))
}

fn build_drum_grid_score(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node: &Node,
) -> Result<Score, TranslateError> {
    let tempo = resolve_tempo_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "tempo",
    )?;
    let time_signature = resolve_time_signature_input(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &node.id,
        "time_signature",
    )?;
    let lane_value = read_number_input(node, "lane", 0.0)? as i32;
    let lane = match lane_value {
        0 => DrumLane::Kick,
        1 => DrumLane::Snare,
        _ => DrumLane::Kick,
    };

    Ok(drum_grid(DrumGridConfig {
        tempo,
        time_signature,
        lane,
        hit_duration_beats: 0.25,
    }))
}

fn read_number_input(node: &Node, port: &str, default: f64) -> Result<f64, TranslateError> {
    match node.inputs.get(port) {
        Some(value) => primitive_to_f64(&node.id, port, value),
        None => Ok(default),
    }
}

fn read_string_input(node: &Node, port: &str, default: &str) -> Result<String, TranslateError> {
    match node.inputs.get(port) {
        Some(value) => primitive_to_string(&node.id, port, value),
        None => Ok(default.to_string()),
    }
}

fn wired_sources(
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    node_id: &str,
    port_id: &str,
) -> Vec<PortReference> {
    edges_by_target
        .get(&(node_id.into(), port_id.into()))
        .cloned()
        .unwrap_or_default()
}

fn wired_source(
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    node_id: &str,
    port_id: &str,
) -> Result<PortReference, TranslateError> {
    let sources = wired_sources(edges_by_target, node_id, port_id);
    match sources.as_slice() {
        [] => Err(TranslateError::UnwiredInput {
            node: node_id.into(),
            port: port_id.into(),
        }),
        [source] => Ok(source.clone()),
        _ => Err(TranslateError::DuplicateEdgeTarget {
            node: node_id.into(),
            port: port_id.into(),
        }),
    }
}

fn wired_source_optional(
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    node_id: &str,
    port_id: &str,
) -> Result<Option<PortReference>, TranslateError> {
    let sources = wired_sources(edges_by_target, node_id, port_id);
    match sources.as_slice() {
        [] => Ok(None),
        [source] => Ok(Some(source.clone())),
        _ => Err(TranslateError::DuplicateEdgeTarget {
            node: node_id.into(),
            port: port_id.into(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn signal_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<Box<dyn Sampler>, TranslateError> {
    if let Some(literal) = graph.nodes.get(node_id).and_then(|node| node.inputs.get(port)) {
        let number = primitive_to_f64(node_id, port, literal)?;
        return Ok(Box::new(ConstantSampler(number as f32)));
    }

    let source = wired_source(edges_by_target, node_id, port)?;
    signal_from(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        port,
        0,
        &source,
    )
}

#[allow(clippy::too_many_arguments)]
fn signal_inputs_variadic(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<Vec<Box<dyn Sampler>>, TranslateError> {
    let sources = wired_sources(edges_by_target, node_id, port);
    if sources.is_empty() {
        return Err(TranslateError::UnwiredInput {
            node: node_id.into(),
            port: port.into(),
        });
    }
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            signal_from(
                graph,
                registry,
                sample_rate,
                edges_by_target,
                visiting,
                bindings,
                node_id,
                port,
                index,
                source,
            )
        })
        .collect()
}

/// The mono signal `consumer` reads from `source` through input `port` (edge `index`).
///
/// Inside a dual-mono channel pass of `consumer`, the input is translated once and shared by
/// both passes, and a stereo input is seen as the pass's channel.
#[allow(clippy::too_many_arguments)]
fn signal_from(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    consumer: &str,
    port: &str,
    index: usize,
    source: &PortReference,
) -> Result<Box<dyn Sampler>, TranslateError> {
    let pass = bindings.channel_pass(consumer);
    if let Some(input) = pass.as_ref().and_then(|pass| pass.remembered(port, index)) {
        return Ok(pass.expect("pass").view(input));
    }
    let value = translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )?;
    let input = match (value, &pass) {
        (NodeValue::Signal(sampler), None) => return Ok(sampler),
        (NodeValue::Signal(sampler), Some(_)) => impls::SharedInput::Mono(Arc::from(sampler)),
        (NodeValue::Stereo(sampler), Some(_)) => impls::SharedInput::Stereo(Arc::from(sampler)),
        (NodeValue::Stereo(_), None) => {
            return Err(TranslateError::UnexpectedStereo {
                node: consumer.into(),
                port: port.into(),
            })
        }
        (other, _) => return Err(invalid_signal_source(other, &source.node)),
    };
    let pass = pass.expect("pass");
    pass.remember(port, index, input.clone());
    Ok(pass.view(input))
}

/// A stereo signal into `port` of a stereo implementation.
#[allow(clippy::too_many_arguments)]
fn stereo_signal_input(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    edges_by_target: &BTreeMap<TargetKey, Vec<PortReference>>,
    visiting: &mut BTreeSet<NodeId>,
    bindings: &Bindings<'_>,
    node_id: &str,
    port: &str,
) -> Result<Box<dyn StereoSampler>, TranslateError> {
    let source = wired_source(edges_by_target, node_id, port)?;
    match translate_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        &source.node,
        &source.port,
    )? {
        NodeValue::Stereo(sampler) => Ok(sampler),
        NodeValue::Signal(_) => Err(TranslateError::ExpectedStereo {
            node: node_id.into(),
            port: port.into(),
        }),
        other => Err(invalid_signal_source(other, &source.node)),
    }
}

/// The error for a non-signal value wired into a signal input.
fn invalid_signal_source(value: NodeValue, source: &str) -> TranslateError {
    let node = source.into();
    match value {
        NodeValue::ScoreSignal(_) => TranslateError::InvalidScoreSource { node },
        NodeValue::ProgressionSignal(_) => TranslateError::InvalidProgressionSource { node },
        NodeValue::Tempo(_) => TranslateError::InvalidTempoSource { node },
        NodeValue::TimeSignature(_) => TranslateError::InvalidTimeSignatureSource { node },
        NodeValue::Signal(_) | NodeValue::Stereo(_) => {
            unreachable!("signal values are handled by the caller")
        }
    }
}

/// The resolved layout of every layout-polymorphic node (a type parameter bounded by `Audio`).
/// Nodes left out are mono.
fn resolve_layouts(graph: &Graph, registry: &Registry) -> BTreeMap<NodeId, Layout> {
    let mut layouts = BTreeMap::new();
    for node in graph.nodes.values() {
        let polymorphic = impls::node_impls()
            .get(node.node_type.as_str())
            .is_some_and(impls::Matrix::is_layout_polymorphic);
        if !polymorphic {
            continue;
        }
        let Some(definition) = get_node_definition(registry, &node.node_type) else {
            continue;
        };
        let Some(index) = definition
            .type_params
            .iter()
            .position(|param| param.bounds.iter().any(|bound| bound == AUDIO))
        else {
            continue;
        };
        let stereo = resolve_type_args(graph, &node.id, registry, None)
            .and_then(|args| args.get(index).cloned())
            .is_some_and(|arg| matches!(arg, SignalType::Concrete { id, .. } if id == AUDIO_STEREO));
        if stereo {
            layouts.insert(node.id.clone(), Layout::Stereo);
        }
    }
    layouts
}

fn primitive_to_f64(
    node_id: &str,
    port_id: &str,
    value: &PrimitiveValue,
) -> Result<f64, TranslateError> {
    match value {
        PrimitiveValue::Number(n) => Ok(*n),
        _ => Err(TranslateError::ExpectedNumber {
            node: node_id.into(),
            port: port_id.into(),
        }),
    }
}

fn primitive_to_string(
    node_id: &str,
    port_id: &str,
    value: &PrimitiveValue,
) -> Result<String, TranslateError> {
    match value {
        PrimitiveValue::String(s) => Ok(s.clone()),
        _ => Err(TranslateError::ExpectedString {
            node: node_id.into(),
            port: port_id.into(),
        }),
    }
}

struct ConstantSampler(f32);

impl Sampler for ConstantSampler {
    fn at(&self, _t: f64) -> f32 {
        self.0
    }
}

struct TimeSampler;

impl Sampler for TimeSampler {
    fn at(&self, t: f64) -> f32 {
        t as f32
    }
}

struct TimeElapsedSampler {
    start: Box<dyn Sampler>,
}

impl Sampler for TimeElapsedSampler {
    fn at(&self, t: f64) -> f32 {
        let start = self.start.at(t) as f64;
        (t - start).max(0.0) as f32
    }
}

struct SineSampler {
    frequency: Box<dyn Sampler>,
    time: Box<dyn Sampler>,
}

impl Sampler for SineSampler {
    fn at(&self, t: f64) -> f32 {
        let frequency = self.frequency.at(t);
        let phase_time = self.time.at(t) as f64;
        sine(frequency, phase_time)
    }
}

struct MultiplySampler {
    a: Box<dyn Sampler>,
    b: Box<dyn Sampler>,
}

impl Sampler for MultiplySampler {
    fn at(&self, t: f64) -> f32 {
        multiply(self.a.at(t), self.b.at(t))
    }
}

struct MultiplyControlSampler {
    a: Box<dyn Sampler>,
    b: Box<dyn Sampler>,
}

impl Sampler for MultiplyControlSampler {
    fn at(&self, t: f64) -> f32 {
        multiply(self.a.at(t), self.b.at(t))
    }
}

struct AddSampler {
    inputs: Vec<Box<dyn Sampler>>,
}

impl Sampler for AddSampler {
    fn at(&self, t: f64) -> f32 {
        self.inputs
            .iter()
            .map(|input| input.at(t))
            .reduce(add)
            .unwrap_or(0.0)
    }
}

struct ExponentialSweepSineSampler {
    start_hz: Box<dyn Sampler>,
    decay_rate: Box<dyn Sampler>,
    elapsed: Box<dyn Sampler>,
}

impl Sampler for ExponentialSweepSineSampler {
    fn at(&self, t: f64) -> f32 {
        exponential_sweep_sine(
            self.start_hz.at(t),
            self.decay_rate.at(t),
            self.elapsed.at(t) as f64,
        )
    }
}

struct DeterministicNoiseSampler {
    seed: Box<dyn Sampler>,
    time: Box<dyn Sampler>,
}

impl Sampler for DeterministicNoiseSampler {
    fn at(&self, t: f64) -> f32 {
        deterministic_noise(self.seed.at(t), self.time.at(t) as f64)
    }
}

struct HighpassNoiseSampler {
    seed: Box<dyn Sampler>,
    time: Box<dyn Sampler>,
    dt: Box<dyn Sampler>,
}

impl Sampler for HighpassNoiseSampler {
    fn at(&self, t: f64) -> f32 {
        highpass_noise(
            self.seed.at(t),
            self.time.at(t) as f64,
            self.dt.at(t) as f64,
        )
    }
}

struct LinearAdsrSampler {
    attack: Box<dyn Sampler>,
    decay: Box<dyn Sampler>,
    sustain: Box<dyn Sampler>,
    release: Box<dyn Sampler>,
    elapsed: Box<dyn Sampler>,
    note_duration: Box<dyn Sampler>,
}

impl Sampler for LinearAdsrSampler {
    fn at(&self, t: f64) -> f32 {
        let adsr = LinearAdsr {
            attack: self.attack.at(t) as f64,
            decay: self.decay.at(t) as f64,
            sustain: self.sustain.at(t),
            release: self.release.at(t) as f64,
        };
        adsr.value_at(
            self.elapsed.at(t) as f64,
            self.note_duration.at(t) as f64,
        )
    }
}

struct SemitoneToHzSampler {
    semitone: Box<dyn Sampler>,
}

impl Sampler for SemitoneToHzSampler {
    fn at(&self, t: f64) -> f32 {
        let value = self.semitone.at(t).round() as i16;
        Semitone(value).to_hz() as f32
    }
}

struct NoteSemitoneSampler {
    schedule: NoteSchedule,
}

impl Sampler for NoteSemitoneSampler {
    fn at(&self, t: f64) -> f32 {
        self.schedule
            .active_at(t)
            .map(|note| note.semitone.0 as f32)
            .unwrap_or(0.0)
    }
}

struct NoteActiveSampler {
    schedule: NoteSchedule,
}

impl Sampler for NoteActiveSampler {
    fn at(&self, t: f64) -> f32 {
        if self.schedule.active_at(t).is_some() {
            1.0
        } else {
            0.0
        }
    }
}

struct NoteStartSampler {
    schedule: NoteSchedule,
}

impl Sampler for NoteStartSampler {
    fn at(&self, t: f64) -> f32 {
        self.schedule
            .note_global_start_secs(t)
            .map(|start| start as f32)
            .unwrap_or(0.0)
    }
}

struct NoteDurationSampler {
    schedule: NoteSchedule,
}

impl Sampler for NoteDurationSampler {
    fn at(&self, t: f64) -> f32 {
        self.schedule
            .active_at(t)
            .map(|note| self.schedule.note_duration_secs(note) as f32)
            .unwrap_or(0.0)
    }
}

struct NoteVelocitySampler {
    schedule: NoteSchedule,
}

impl Sampler for NoteVelocitySampler {
    fn at(&self, t: f64) -> f32 {
        self.schedule
            .active_at(t)
            .map(|note| note.velocity)
            .unwrap_or(0.0)
    }
}

struct NoteParamSampler {
    schedule: NoteSchedule,
    param_name: String,
}

impl Sampler for NoteParamSampler {
    fn at(&self, t: f64) -> f32 {
        match self.schedule.active_at(t) {
            None => 0.0,
            Some(note) => note.param_or(&self.param_name, 1.0) as f32,
        }
    }
}

struct BandlimitedSawSampler {
    frequency: Box<dyn Sampler>,
    elapsed: Box<dyn Sampler>,
    cutoff: Box<dyn Sampler>,
}

impl Sampler for BandlimitedSawSampler {
    fn at(&self, t: f64) -> f32 {
        let frequency = self.frequency.at(t);
        let elapsed = self.elapsed.at(t) as f64;
        let cutoff = self.cutoff.at(t);
        bandlimited_saw(frequency, elapsed, cutoff)
    }
}

struct NoteEnvelopeSampler {
    schedule: NoteSchedule,
    adsr: LinearAdsr,
}

impl Sampler for NoteEnvelopeSampler {
    fn at(&self, t: f64) -> f32 {
        match self.schedule.note_local_secs(t) {
            None => 0.0,
            Some(local) => {
                let duration = self
                    .schedule
                    .active_at(t)
                    .map(|note| self.schedule.note_duration_secs(note))
                    .unwrap_or(0.0);
                self.adsr.value_at(local, duration)
            }
        }
    }
}

/// Polyphonic sine voice: mixes overlapping notes (sine × envelope × velocity).
struct PolySineScoreSampler {
    schedule: NoteSchedule,
    adsr: LinearAdsr,
}

impl Sampler for PolySineScoreSampler {
    fn at(&self, t: f64) -> f32 {
        let mut mix = 0.0f32;
        for note in self.schedule.overlapping_at(t) {
            let local = self.schedule.note_local_secs_for(note, t);
            let duration = self.schedule.note_duration_secs(note);
            let env = self.adsr.value_at(local, duration);
            let frequency = note.semitone.to_hz() as f32;
            mix += sine(frequency, local) * env * note.velocity;
        }
        mix
    }
}

struct ChordRootSampler {
    progression: ProgressionSignal,
    tempo: Tempo,
}

impl Sampler for ChordRootSampler {
    fn at(&self, t: f64) -> f32 {
        self.progression.chord_at(t, self.tempo).root.0 as f32
    }
}

struct ChordActiveSampler {
    progression: ProgressionSignal,
    tempo: Tempo,
}

impl Sampler for ChordActiveSampler {
    fn at(&self, t: f64) -> f32 {
        if self.progression.looping {
            return 1.0;
        }
        let beat = t / self.tempo.seconds_per_beat();
        if self
            .progression
            .progression
            .regions
            .iter()
            .any(|region| {
                beat >= region.start_beats - 1e-9
                    && beat < region.start_beats + region.duration_beats - 1e-9
            })
        {
            1.0
        } else {
            0.0
        }
    }
}

struct TempoBpmSampler {
    tempo: Tempo,
}

impl Sampler for TempoBpmSampler {
    fn at(&self, _t: f64) -> f32 {
        self.tempo.bpm as f32
    }
}

struct BeatsPerBarSampler {
    time_signature: TimeSignature,
}

impl Sampler for BeatsPerBarSampler {
    fn at(&self, _t: f64) -> f32 {
        f32::from(self.time_signature.beats_per_bar)
    }
}

struct BeatUnitSampler {
    time_signature: TimeSignature,
}

impl Sampler for BeatUnitSampler {
    fn at(&self, _t: f64) -> f32 {
        f32::from(self.time_signature.beat_unit)
    }
}

pub fn infer_score_duration_secs(graph: &Graph, sample_rate: SampleRate) -> Option<f64> {
    let score_node = graph
        .nodes
        .values()
        .find(|node| node.node_type == "arpeggio")?;
    let edges_by_target = index_edges_by_target(graph);
    let mut visiting = BTreeSet::new();
    let registry = aura_imp::aura_registry().ok()?;
    let score = build_arpeggio_score(
        graph,
        &registry,
        sample_rate,
        &edges_by_target,
        &mut visiting,
        &Bindings::new(&BTreeMap::new(), TranslateMode::Deterministic, None),
        score_node,
    )
    .ok()?;
    Some(score.duration_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_imp::{aura_registry, aura_registry_with_libraries, discover_graph_libraries};
    use std::path::PathBuf;

    fn workspace_demo_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos").join(name)
    }
    #[test]
    fn translate_sine_graph_is_pure() {
        let json = include_str!("../../../../demos/sine.json");
        let graph = aura_imp::graph_from_json_str(json).expect("graph");
        let registry = aura_registry().expect("registry");
        let sampler = translate_graph(&graph, &registry, SampleRate::RATE_44100)
            .expect("translate")
            .into_mono()
            .expect("mono");
        assert_eq!(sampler.at(0.25), sampler.at(0.25));
    }

    #[test]
    fn translate_variadic_add_sums_constant_inputs() {
        use aura_imp::{Edge, Graph, Node, PortReference, PrimitiveValue};

        let graph = Graph {
            nodes: BTreeMap::from([
                (
                    "a".into(),
                    Node {
                        id: "a".into(),
                        title: None,
                        node_type: "parameter".into(),
                        type_args: vec![],
                        inputs: BTreeMap::from([(
                            "value".into(),
                            PrimitiveValue::Number(0.25),
                        )]),
                    },
                ),
                (
                    "b".into(),
                    Node {
                        id: "b".into(),
                        title: None,
                        node_type: "parameter".into(),
                        type_args: vec![],
                        inputs: BTreeMap::from([(
                            "value".into(),
                            PrimitiveValue::Number(0.50),
                        )]),
                    },
                ),
                (
                    "c".into(),
                    Node {
                        id: "c".into(),
                        title: None,
                        node_type: "parameter".into(),
                        type_args: vec![],
                        inputs: BTreeMap::from([(
                            "value".into(),
                            PrimitiveValue::Number(0.25),
                        )]),
                    },
                ),
                (
                    "mix".into(),
                    Node {
                        id: "mix".into(),
                        title: None,
                        node_type: "add".into(),
                        // Constant inputs carry no layout, so the sum states it.
                        type_args: vec![aura_imp::signals::signal(aura_imp::signals::AUDIO_MONO)],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "out".into(),
                    Node {
                        id: "out".into(),
                        title: None,
                        node_type: "output".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
            ]),
            edges: BTreeMap::from([
                (
                    "a_to_mix".into(),
                    Edge {
                        from: PortReference {
                            node: "a".into(),
                            port: "value".into(),
                        },
                        to: PortReference {
                            node: "mix".into(),
                            port: "inputs".into(),
                        },
                        order: Some(0),
                    },
                ),
                (
                    "b_to_mix".into(),
                    Edge {
                        from: PortReference {
                            node: "b".into(),
                            port: "value".into(),
                        },
                        to: PortReference {
                            node: "mix".into(),
                            port: "inputs".into(),
                        },
                        order: Some(1),
                    },
                ),
                (
                    "c_to_mix".into(),
                    Edge {
                        from: PortReference {
                            node: "c".into(),
                            port: "value".into(),
                        },
                        to: PortReference {
                            node: "mix".into(),
                            port: "inputs".into(),
                        },
                        order: Some(2),
                    },
                ),
                (
                    "mix_to_out".into(),
                    Edge {
                        from: PortReference {
                            node: "mix".into(),
                            port: "audio".into(),
                        },
                        to: PortReference {
                            node: "out".into(),
                            port: "value".into(),
                        },
                        order: None,
                    },
                ),
            ]),
        };
        let registry = aura_registry().expect("registry");
        let sampler = translate_graph(&graph, &registry, SampleRate::RATE_44100)
            .expect("translate")
            .into_mono()
            .expect("mono");
        assert!((sampler.at(0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn translate_arpeggio_graph_with_loop_nodes() {
        let json = include_str!("../../../../demos/demo-01.json");
        let graph = aura_imp::graph_from_json_str(json).expect("graph");
        let graph_path = workspace_demo_path("demo-01.json");
        let libraries = discover_graph_libraries(&graph_path).expect("libraries");
        let registry = aura_registry_with_libraries(&libraries).expect("registry");
        translate_graph(&graph, &registry, SampleRate::RATE_44100).expect("translate");
    }

    #[test]
    fn unresolved_notation_endpoint_typechecks_as_unknown() {
        use aura_imp::{Edge, Graph, Node, PortReference};

        let registry = aura_registry().expect("registry");
        let tip_id = "01MTESTNOTATIONTIP000000099";
        let graph = Graph {
            nodes: BTreeMap::from([
                (
                    "clock".into(),
                    Node {
                        id: "clock".into(),
                        title: None,
                        node_type: "time".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "voice".into(),
                    Node {
                        id: "voice".into(),
                        title: None,
                        node_type: "poly_sine_score".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "out".into(),
                    Node {
                        id: "out".into(),
                        title: None,
                        node_type: "output".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
            ]),
            edges: BTreeMap::from([
                (
                    "score_to_voice".into(),
                    Edge {
                        from: PortReference {
                            node: tip_id.into(),
                            port: "score".into(),
                        },
                        to: PortReference {
                            node: "voice".into(),
                            port: "score".into(),
                        },
                        order: None,
                    },
                ),
                (
                    "time_to_voice".into(),
                    Edge {
                        from: PortReference {
                            node: "clock".into(),
                            port: "time".into(),
                        },
                        to: PortReference {
                            node: "voice".into(),
                            port: "time".into(),
                        },
                        order: None,
                    },
                ),
                (
                    "voice_to_out".into(),
                    Edge {
                        from: PortReference {
                            node: "voice".into(),
                            port: "audio".into(),
                        },
                        to: PortReference {
                            node: "out".into(),
                            port: "value".into(),
                        },
                        order: None,
                    },
                ),
            ]),
        };

        let err = match translate_graph(&graph, &registry, SampleRate::RATE_44100) {
            Ok(_) => panic!("expected TypeCheck"),
            Err(error) => error,
        };
        assert!(matches!(err, TranslateError::TypeCheck { .. }));
    }

    #[test]
    fn translate_rejects_edge_to_missing_node() {
        use aura_imp::{Edge, Graph, Node, PortReference};

        let registry = aura_registry().expect("registry");
        let graph = Graph {
            nodes: BTreeMap::from([(
                "out".into(),
                Node {
                    id: "out".into(),
                    title: Some("out".into()),
                    node_type: "output".into(),
                    type_args: vec![],
                    inputs: BTreeMap::new(),
                },
            )]),
            edges: BTreeMap::from([(
                "ghost_to_out".into(),
                Edge {
                    from: PortReference {
                        node: "ghost".into(),
                        port: "value".into(),
                    },
                    to: PortReference {
                        node: "out".into(),
                        port: "value".into(),
                    },
                    order: None,
                },
            )]),
        };

        let err = match translate_graph(&graph, &registry, SampleRate::RATE_44100) {
            Ok(_) => panic!("expected typecheck failure for missing node"),
            Err(error) => error,
        };
        match err {
            TranslateError::TypeCheck { message } => {
                assert!(
                    message.contains("unknown node"),
                    "unexpected typecheck message: {message}"
                );
            }
            other => panic!("expected TypeCheck, got {other:?}"),
        }
    }

    #[test]
    fn looped_note_envelope_is_active_in_second_cycle() {
        use aura_composition::{arpeggio, ArpeggioConfig};

        let score = arpeggio(ArpeggioConfig::default());
        let signal = ScoreSignal::finite(score).with_looping(true);
        let cycle = signal.score.loop_cycle_secs();
        let schedule = NoteSchedule::from_score_signal(&signal, SampleRate::RATE_44100);
        let sampler = NoteEnvelopeSampler {
            schedule,
            adsr: LinearAdsr::default(),
        };
        let early_second_cycle = sampler.at(cycle + 0.01) as f64;
        assert!(
            early_second_cycle > 0.0,
            "expected audible envelope after loop cycle, got {early_second_cycle}"
        );
    }

    #[test]
    fn notation_node_score_binding_feeds_poly_sine_and_mixes_chords() {
        use aura_imp::{Edge, Graph, Node, PortReference};
        use aura_imp::LayeredNodes;
        use aura_sequencing::{NoteEvent, Score, Semitone, Tempo, TimeSignature};

        let registry = aura_registry().expect("registry");
        let remote_id = "notation_record";
        let graph = Graph {
            nodes: BTreeMap::from([
                (
                    "clock".into(),
                    Node {
                        id: "clock".into(),
                        title: None,
                        node_type: "time".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "voice".into(),
                    Node {
                        id: "voice".into(),
                        title: None,
                        node_type: "poly_sine_score".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "out".into(),
                    Node {
                        id: "out".into(),
                        title: None,
                        node_type: "output".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
            ]),
            edges: BTreeMap::from([
                (
                    "score_to_voice".into(),
                    Edge {
                        from: PortReference {
                            node: remote_id.into(),
                            port: "score".into(),
                        },
                        to: PortReference {
                            node: "voice".into(),
                            port: "score".into(),
                        },
                        order: None,
                    },
                ),
                (
                    "time_to_voice".into(),
                    Edge {
                        from: PortReference {
                            node: "clock".into(),
                            port: "time".into(),
                        },
                        to: PortReference {
                            node: "voice".into(),
                            port: "time".into(),
                        },
                        order: None,
                    },
                ),
                (
                    "voice_to_out".into(),
                    Edge {
                        from: PortReference {
                            node: "voice".into(),
                            port: "audio".into(),
                        },
                        to: PortReference {
                            node: "out".into(),
                            port: "value".into(),
                        },
                        order: None,
                    },
                ),
            ]),
        };

        let score = Score::new(
            TimeSignature::FOUR_FOUR,
            Tempo::default(),
            vec![
                NoteEvent::new(Semitone(60), 0.0, 0.5),
                NoteEvent::new(Semitone(64), 0.0, 0.5),
                NoteEvent::new(Semitone(67), 0.0, 0.5),
            ],
        );
        let bindings = BTreeMap::from([(
            remote_id.into(),
            ScoreSignal::finite(score),
        )]);
        // The notation is a node from another provider (a Tome store in hosts), not the document.
        let notation_nodes = BTreeMap::from([(
            remote_id.to_string(),
            Node {
                id: remote_id.into(),
                title: None,
                node_type: NOTATION_NODE_TYPE.into(),
                type_args: vec![],
                inputs: BTreeMap::new(),
            },
        )]);
        let nodes = LayeredNodes {
            providers: vec![&graph.nodes, &notation_nodes],
        };
        let source = GraphSource {
            nodes: &nodes,
            edges: &graph.edges,
        };
        let sampler = translate_graph_with_options(
            source,
            &registry,
            SampleRate::RATE_44100,
            TranslateOptions {
                score_bindings: &bindings,
                plugins: None,
            },
        )
        .expect("translate")
        .into_mono()
        .expect("mono");

        let chord = sampler.at(0.05);
        assert!(
            chord.abs() > 1e-4,
            "expected audible chord mix, got {chord}"
        );
        assert_eq!(sampler.at(0.05), sampler.at(0.05));
        assert_eq!(sampler.at(1.0), 0.0);
    }

    #[test]
    fn typecheck_rejects_loop_wired_to_tempo() {
        use aura_imp::{Edge, Graph, Node, PortReference};

        let registry = aura_registry().expect("registry");
        let graph = Graph {
            nodes: BTreeMap::from([
                (
                    "tempo".into(),
                    Node {
                        id: "tempo".into(),
                        title: None,
                        node_type: "constant_tempo".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
                (
                    "bad_loop".into(),
                    Node {
                        id: "bad_loop".into(),
                        title: None,
                        node_type: "loop".into(),
                        type_args: vec![],
                        inputs: BTreeMap::new(),
                    },
                ),
            ]),
            edges: BTreeMap::from([(
                "e1".into(),
                Edge {
                    from: PortReference {
                        node: "tempo".into(),
                        port: "tempo".into(),
                    },
                    to: PortReference {
                        node: "bad_loop".into(),
                        port: "value".into(),
                    },
                    order: None,
                },
            )]),
        };

        let errors = check_graph(&graph, &registry);
        assert!(
            errors.iter().any(|e| e.message.contains("Loopable")
                || e.message.contains("does not satisfy bound")
                || e.message.contains("incompatible")),
            "expected type error for tempo into loop, got: {:?}",
            errors
        );
    }
}
