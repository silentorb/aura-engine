//! Imp graph → block signal translation.
//!
//! Translation builds each node output once, as a [`Signal`] or [`StereoSignal`] that renders a
//! range of frames. An output read by several consumers is shared ([`Shared`]), so fan-out
//! renders it once per range.

mod impls;
mod nodes;
mod note_schedule;
mod plugin;
mod signals;
mod streaming;
#[cfg(test)]
mod stereo_tests;
#[cfg(test)]
mod contract_tests;

pub use plugin::plugin_node_settings;
pub use streaming::{StreamingControl, StreamingError, StreamingProgram};

use note_schedule::NoteSchedule;
use aura_composition::{
    arpeggio, bass_line, drum_grid, epic_minor_progression, ArpeggioConfig, BassLineConfig,
    DrumGridConfig, DrumLane,
};
use aura_sequencing::{
    LiveNote,
    ChordProgression, ProgressionSignal, Score, ScoreSignal, Semitone, Tempo,
    TimeSignature,
};
use aura_imp::{
    flatten_graph, Graph, GraphSource, Node, NodeId, PortReference, PrimitiveValue, Registry,
    NOTATION_NODE_TYPE,
};
use aura_instrumentation::LinearAdsr;
use aura_plugin_host::PluginResolver;
use aura_render::{
    AudioSignal, Constant, Input, Layout, Shared, SharedStereo, Signal, StereoChannel,
    StereoSignal,
};
use signals::*;
use aura_sample::SampleRate;
use aura_imp::types::{named, AUDIO};
use aura_imp::signals::AUDIO_STEREO;
use aura_imp::{get_node_definition, Type};
use imp_typecheck::{check_graph, resolve_type_args};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use thiserror::Error;

type TargetKey = (NodeId, String);

#[derive(Debug, Error, PartialEq)]
pub enum TranslateError {
    #[error(
        "audio layout of node `{node}` is not statically known (it resolves to a union of layouts); state the layout with typeArgs"
    )]
    UnresolvedLayout { node: NodeId },
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
    Signal(Box<dyn Signal>),
    Stereo(Box<dyn StereoSignal>),
    ScoreSignal(ScoreSignal),
    ProgressionSignal(ProgressionSignal),
    Tempo(Tempo),
    TimeSignature(TimeSignature),
}

/// A node output shared by several consumers.
#[derive(Clone)]
enum SharedOutput {
    Mono(Shared),
    Stereo(SharedStereo),
}

impl SharedOutput {
    fn value(&self) -> NodeValue {
        match self {
            SharedOutput::Mono(shared) => NodeValue::Signal(Box::new(shared.clone())),
            SharedOutput::Stereo(shared) => NodeValue::Stereo(Box::new(shared.clone())),
        }
    }
}

/// Polyphonic sine audition for a concrete [`Score`] (no Imp graph / proxy type).
pub fn signal_from_score(score: Score, sample_rate: SampleRate) -> Box<dyn Signal> {
    let schedule = NoteSchedule::from_score_signal(&ScoreSignal::finite(score), sample_rate);
    Box::new(PolySineScoreSignal {
        schedule,
        adsr: LinearAdsr::default(),
        rate: f64::from(sample_rate.get()),
    })
}

/// Translates an Imp graph into a deterministic signal: mono (frame → sample) or stereo
/// (frame → `[left, right]`), renderable over any range.
pub fn translate_graph(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
) -> Result<AudioSignal, TranslateError> {
    translate_graph_with_bindings(graph, registry, sample_rate, &BTreeMap::new())
}

/// Translates an Imp graph with scores for its notation nodes.
pub fn translate_graph_with_bindings(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    score_bindings: &BTreeMap<NodeId, ScoreSignal>,
) -> Result<AudioSignal, TranslateError> {
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
) -> Result<AudioSignal, TranslateError> {
    let prepared = PreparedGraph::prepare(source, registry)?;
    let bindings = Bindings::new(
        options.score_bindings,
        TranslateMode::Deterministic,
        options.plugins,
    );
    prepared.translate(registry, sample_rate, &bindings)
}

/// The execution mode a graph is translated for (see `docs/design/playback-modes.md`).
///
/// Every node type has a deterministic implementation and may have a streaming one. In
/// streaming mode the translator builds a node type's streaming implementation when it has one
/// and its deterministic implementation otherwise. Nodes never check the mode while rendering.
#[derive(Debug, Clone, Default)]
pub(crate) enum TranslateMode {
    /// Any range in any order, with full knowledge of the score. Node state is nonessential.
    #[default]
    Deterministic,
    /// Consecutive blocks of one voice instance sounding a live note (onset at frame 0). Node
    /// state may carry history.
    Streaming { live: Arc<LiveNote> },
}

/// Translation-wide inputs threaded through node translation: notation scores and the mode.
pub(crate) struct Bindings<'a> {
    pub(crate) scores: &'a BTreeMap<NodeId, ScoreSignal>,
    pub(crate) mode: TranslateMode,
    pub(crate) plugins: Option<&'a dyn PluginResolver>,
    /// Plugin instances created so far.
    pub(crate) plugin_instances: std::cell::Cell<usize>,
    /// Resolved layout of each layout-polymorphic node, set by the graph being translated.
    pub(crate) layouts: std::cell::RefCell<Arc<BTreeMap<NodeId, Layout>>>,
    /// How many times each node output may be read, set by the graph being translated.
    reads: std::cell::RefCell<Arc<BTreeMap<TargetKey, usize>>>,
    /// Node outputs built so far that more than one consumer reads.
    shared: std::cell::RefCell<BTreeMap<TargetKey, SharedOutput>>,
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
            plugin_instances: Default::default(),
            layouts: Default::default(),
            reads: Default::default(),
            shared: Default::default(),
            channel_passes: Default::default(),
        }
    }

    fn reads_of(&self, key: &TargetKey) -> usize {
        self.reads.borrow().get(key).copied().unwrap_or(1)
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
    /// How many times translation may read each node output (see [`count_reads`]).
    reads: Arc<BTreeMap<TargetKey, usize>>,
}

/// An upper bound on how many times translation reads each node output: once per edge out of
/// it, times the number of the consumer's output ports in use (each is translated on its own),
/// times two when the consumer is stereo (a dual-mono consumer translates once per channel).
/// An output read more than once is shared; overcounting only shares an output that did not
/// need it.
fn count_reads(graph: &Graph, layouts: &BTreeMap<NodeId, Layout>) -> BTreeMap<TargetKey, usize> {
    let mut ports_in_use: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in graph.edges.values() {
        ports_in_use
            .entry(edge.from.node.as_str())
            .or_default()
            .insert(edge.from.port.as_str());
    }
    let mut reads = BTreeMap::new();
    for edge in graph.edges.values() {
        let consumer = edge.to.node.as_str();
        let ports = ports_in_use.get(consumer).map_or(1, |ports| ports.len().max(1));
        let channels = layouts.get(consumer).map_or(1, |layout| layout.channels());
        *reads
            .entry((edge.from.node.clone(), edge.from.port.clone()))
            .or_insert(0) += ports * channels;
    }
    reads
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
        let layouts = resolve_layouts(&graph, registry)?;
        let reads = Arc::new(count_reads(&graph, &layouts));
        Ok(Self {
            graph,
            output_node,
            edges_by_target,
            layouts: Arc::new(layouts),
            reads,
        })
    }

    pub(crate) fn translate(
        &self,
        registry: &Registry,
        sample_rate: SampleRate,
        bindings: &Bindings<'_>,
    ) -> Result<AudioSignal, TranslateError> {
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
    ) -> Result<AudioSignal, TranslateError> {
        *bindings.layouts.borrow_mut() = Arc::clone(&self.layouts);
        *bindings.reads.borrow_mut() = Arc::clone(&self.reads);
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
            NodeValue::Signal(signal) => Ok(AudioSignal::Mono(signal)),
            NodeValue::Stereo(signal) => Ok(AudioSignal::Stereo(signal)),
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
    // An output read more than once is built once and shared by every reader.
    let key: TargetKey = (node_id.to_string(), output_port.to_string());
    if let Some(shared) = bindings.shared.borrow().get(&key) {
        return Ok(shared.value());
    }
    let value = build_node(
        graph,
        registry,
        sample_rate,
        edges_by_target,
        visiting,
        bindings,
        node_id,
        output_port,
    )?;
    if bindings.reads_of(&key) <= 1 {
        return Ok(value);
    }
    let shared = match value {
        NodeValue::Signal(signal) => SharedOutput::Mono(Shared::new(signal)),
        NodeValue::Stereo(signal) => SharedOutput::Stereo(SharedStereo::new(signal)),
        other => return Ok(other),
    };
    let value = shared.value();
    bindings.shared.borrow_mut().insert(key, shared);
    Ok(value)
}

/// Builds one node output from its type's implementation matrix.
#[allow(clippy::too_many_arguments)]
fn build_node(
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

    let field = match output_port {
        "semitone" => NoteField::Semitone,
        "active" => NoteField::Active,
        "velocity" => NoteField::Velocity,
        "note_start" => NoteField::Start,
        "note_duration" => NoteField::Duration,
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(Box::new(NoteFieldSignal {
        schedule,
        field,
        rate: f64::from(sample_rate.get()),
    })))
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

    Ok(NodeValue::Signal(Box::new(NoteFieldSignal {
        schedule,
        field: NoteField::Param(param_name),
        rate: f64::from(sample_rate.get()),
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

    let rate = f64::from(sample_rate.get());
    let signal: Box<dyn Signal> = match output_port {
        "root" => Box::new(ChordRootSignal {
            progression,
            tempo,
            rate,
        }),
        "active" => Box::new(ChordActiveSignal {
            progression,
            tempo,
            rate,
        }),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(signal))
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

    match output_port {
        "bpm" => Ok(NodeValue::Signal(Box::new(Constant(tempo.bpm as f32)))),
        _ => Err(TranslateError::MissingOutputPort {
            node: node_id.into(),
            port: output_port.into(),
        }),
    }
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

    let value = match output_port {
        "beats_per_bar" => f32::from(time_signature.beats_per_bar),
        "beat_unit" => f32::from(time_signature.beat_unit),
        _ => {
            return Err(TranslateError::MissingOutputPort {
                node: node_id.into(),
                port: output_port.into(),
            })
        }
    };

    Ok(NodeValue::Signal(Box::new(Constant(value))))
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
) -> Result<Box<dyn Signal>, TranslateError> {
    if let Some(literal) = graph.nodes.get(node_id).and_then(|node| node.inputs.get(port)) {
        let number = primitive_to_f64(node_id, port, literal)?;
        return Ok(Box::new(Constant(number as f32)));
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
) -> Result<Vec<Box<dyn Signal>>, TranslateError> {
    let sources = wired_sources(edges_by_target, node_id, port);
    if sources.is_empty() {
        return Err(TranslateError::UnwiredInput {
            node: node_id.into(),
            port: port.into(),
        });
    }
    sources
        .iter()
        .map(|source| {
            signal_from(
                graph,
                registry,
                sample_rate,
                edges_by_target,
                visiting,
                bindings,
                node_id,
                port,
                source,
            )
        })
        .collect()
}

/// The mono signal `consumer` reads from `source` through input `port`.
///
/// Inside a dual-mono channel pass of `consumer`, a stereo input is seen as the pass's channel.
/// Both passes read the same input: a stereo consumer's inputs count as read twice
/// ([`count_reads`]), so translation shares them.
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
    source: &PortReference,
) -> Result<Box<dyn Signal>, TranslateError> {
    let pass = bindings.channel_pass(consumer);
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
    match (value, pass) {
        (NodeValue::Signal(signal), _) => Ok(signal),
        (NodeValue::Stereo(signal), Some(pass)) => {
            Ok(Box::new(StereoChannel::new(signal, pass.channel)))
        }
        (NodeValue::Stereo(_), None) => Err(TranslateError::UnexpectedStereo {
            node: consumer.into(),
            port: port.into(),
        }),
        (other, _) => Err(invalid_signal_source(other, &source.node)),
    }
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
) -> Result<Box<dyn StereoSignal>, TranslateError> {
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
        NodeValue::Stereo(signal) => Ok(signal),
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
/// Nodes left out are mono. A layout that resolves to a union (e.g. fed by an `Audio`-typed port)
/// has no runtime tag to decide it, so it is an error rather than a guess.
fn resolve_layouts(graph: &Graph, registry: &Registry) -> Result<BTreeMap<NodeId, Layout>, TranslateError> {
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
            .position(|param| param.bounds.contains(&named(AUDIO)))
        else {
            continue;
        };
        let arg = resolve_type_args(graph, &node.id, registry, None).and_then(|args| args.get(index).cloned());
        match arg {
            Some(Type::Union { .. }) => {
                return Err(TranslateError::UnresolvedLayout {
                    node: node.id.clone(),
                })
            }
            Some(Type::Concrete { id, .. }) if id == AUDIO_STEREO => {
                layouts.insert(node.id.clone(), Layout::Stereo);
            }
            _ => {}
        }
    }
    Ok(layouts)
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
    use aura_imp::{aura_registry, aura_registry_with_files, discover_imp_files};
    use std::path::PathBuf;

    fn workspace_demo_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos").join(name)
    }

    /// The value of a mono signal at one frame.
    fn value_at(signal: &mut dyn Signal, frame: u64) -> f32 {
        let mut out = [0.0];
        signal.render(frame, &mut out);
        out[0]
    }
    #[test]
    fn translate_sine_graph_is_pure() {
        let json = include_str!("../../../../demos/sine.json");
        let graph = aura_imp::graph_from_json_str(json).expect("graph");
        let registry = aura_registry().expect("registry");
        let mut signal = translate_graph(&graph, &registry, SampleRate::RATE_44100)
            .expect("translate")
            .into_mono()
            .expect("mono");
        assert_eq!(value_at(signal.as_mut(), 11_025), value_at(signal.as_mut(), 11_025));
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
        let mut signal = translate_graph(&graph, &registry, SampleRate::RATE_44100)
            .expect("translate")
            .into_mono()
            .expect("mono");
        assert!((value_at(signal.as_mut(), 0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn translate_arpeggio_graph_with_loop_nodes() {
        let json = include_str!("../../../../demos/demo-01.json");
        let graph = aura_imp::graph_from_json_str(json).expect("graph");
        let graph_path = workspace_demo_path("demo-01.json");
        let libraries = discover_imp_files(&graph_path).expect("libraries");
        let registry = aura_registry_with_files(&libraries).expect("registry");
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
        let mut signal = NoteEnvelopeSignal {
            schedule,
            adsr: LinearAdsr::default(),
            rate: 44_100.0,
        };
        let frame = ((cycle + 0.01) * 44_100.0).round() as u64;
        let early_second_cycle = value_at(&mut signal, frame) as f64;
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
        let mut signal = translate_graph_with_options(
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

        let chord = value_at(signal.as_mut(), 2_205);
        assert!(
            chord.abs() > 1e-4,
            "expected audible chord mix, got {chord}"
        );
        assert_eq!(value_at(signal.as_mut(), 2_205), value_at(signal.as_mut(), 2_205));
        assert_eq!(value_at(signal.as_mut(), 44_100), 0.0);
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
