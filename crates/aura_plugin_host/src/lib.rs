//! Protocol-neutral hosting of third-party audio plugins (CLAP first) in Aura's playback modes.
//!
//! Plugins are stateful, block-based, and forward-only. This crate turns them into Aura runtime
//! nodes without any protocol code:
//!
//! - **Deterministic** — [`RenderCache`] owns a plugin instance and renders it forward into a
//!   retained buffer, so `Sampler::at(t)` can be sampled in any order. Note events come from the
//!   fully known score at exact frames, and the plugin's latency is compensated.
//! - **Streaming** — [`PluginStage`] is a [`BlockSource`](aura_render::BlockSource) holding one
//!   shared instance, driven by note commands as they arrive. A per-voice [`RenderCache`] bound
//!   to one live note is the fallback where a shared instance cannot express the graph.
//!
//! Protocol backends (`aura_clap`, later others) implement [`Processor`], [`ProcessorFactory`],
//! and [`PluginResolver`]. Design: `docs/design/plugin-hosting.md`.

mod cache;
mod layout;
mod notes;
mod stage;

pub use cache::{EventSource, RenderCache};
pub use layout::LayoutStage;
pub use notes::NoteEventSource;
pub use stage::{plugin_stage, PluginStage, StageCommand, StageSendError, StageSender};

use aura_sample::SampleRate;
use std::sync::Arc;
use thiserror::Error;

/// Frames a plugin processes per call. Small enough for streaming latency, large enough that
/// per-call overhead stays negligible offline.
pub const BLOCK_FRAMES: u32 = 256;

/// A note event for one processing block. `frame` is relative to the block start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    NoteOn {
        frame: u32,
        note_id: i32,
        key: i16,
        velocity: f32,
    },
    NoteOff {
        frame: u32,
        note_id: i32,
        key: i16,
    },
}

impl Event {
    pub fn frame(&self) -> u32 {
        match self {
            Event::NoteOn { frame, .. } | Event::NoteOff { frame, .. } => *frame,
        }
    }

    /// Sort key within a block: by frame, note-offs before note-ons at the same frame so a
    /// repeated key re-triggers instead of being cut.
    fn order(&self) -> (u32, u8) {
        match self {
            Event::NoteOff { frame, .. } => (*frame, 0),
            Event::NoteOn { frame, .. } => (*frame, 1),
        }
    }
}

/// Sorts block events by frame, note-offs first at equal frames.
pub fn sort_events(events: &mut [Event]) {
    events.sort_by_key(Event::order);
}

/// Channel counts of a processor's main audio ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelLayout {
    pub inputs: usize,
    pub outputs: usize,
}

/// How long a processor keeps sounding after its input and notes stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    Frames(u64),
    Infinite,
    /// The plugin does not say; the host decides by listening for silence.
    Unknown,
}

/// Whether a processor may use offline-only quality and timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Deterministic rendering: no real-time deadline.
    Offline,
    /// Streaming: the processor must keep up with a device.
    Realtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorConfig {
    pub sample_rate: SampleRate,
    pub max_block_frames: u32,
    pub mode: RenderMode,
}

/// An activated plugin instance, ready to process on whichever thread owns it.
pub trait Processor: Send {
    fn layout(&self) -> ChannelLayout;
    /// Frames the processor delays its output by.
    fn latency_frames(&self) -> u32;
    fn tail(&mut self) -> Tail;
    /// Processes one block. Every slice holds the same number of frames, at most
    /// [`ProcessorConfig::max_block_frames`]; `inputs` and `outputs` match [`Processor::layout`].
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[Event],
    ) -> Result<(), PluginHostError>;
}

/// Creates activated instances of one plugin with its [`PluginSettings`] applied.
pub trait ProcessorFactory: Send + Sync {
    fn plugin_id(&self) -> &str;
    fn create(&self, config: ProcessorConfig) -> Result<Box<dyn Processor>, PluginHostError>;
    /// The plugin's parameters, in the plugin's order.
    fn params(&self) -> Result<Vec<ParamDescription>, PluginHostError>;
}

/// How a plugin node configures its plugin, from the node's embedded data.
///
/// `state` (an opaque plugin-owned blob) is applied first; `params` then override it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginSettings {
    pub state: Option<Vec<u8>>,
    pub params: Vec<ParamSetting>,
}

/// A parameter set to a plain value (the plugin's own units), clamped to its range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSetting {
    pub id: u32,
    pub value: f64,
}

/// One plugin parameter, as the plugin describes it. Values are plain (the plugin's units).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDescription {
    /// Stable identifier; this is what node data refers to.
    pub id: u32,
    pub name: String,
    /// Grouping path, `/`-separated (e.g. `Oscillators/Osc 1`).
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: ParamFlags,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParamFlags {
    pub automatable: bool,
    pub modulatable: bool,
    pub stepped: bool,
    pub hidden: bool,
    pub readonly: bool,
}

/// Finds installed plugins by id. Hosts pass one to translation.
pub trait PluginResolver: Send + Sync {
    /// A factory for `plugin_id` that applies `settings` to every instance it creates.
    fn resolve(
        &self,
        plugin_id: &str,
        settings: &PluginSettings,
    ) -> Result<Arc<dyn ProcessorFactory>, PluginHostError>;
}

#[derive(Debug, Clone, Error, PartialEq)]
pub enum PluginHostError {
    #[error("plugin `{id}` is not installed")]
    NotFound { id: String },
    #[error("failed to load plugin bundle `{path}`: {reason}")]
    BundleLoad { path: String, reason: String },
    #[error("failed to instantiate plugin `{id}`: {reason}")]
    Instantiate { id: String, reason: String },
    #[error("failed to activate plugin `{id}`: {reason}")]
    Activate { id: String, reason: String },
    #[error("plugin `{id}` rejected its stored state")]
    StateLoad { id: String },
    #[error("plugin `{id}` has no parameter {param}")]
    UnknownParam { id: String, param: u32 },
    #[error("plugin `{id}` parameter {param} is read-only")]
    ReadonlyParam { id: String, param: u32 },
    #[error("plugin `{id}` has no GUI")]
    NoGui { id: String },
    #[error("could not open the GUI of plugin `{id}`: {reason}")]
    Gui { id: String, reason: String },
    #[error("plugin `{id}` failed while processing: {reason}")]
    Process { id: String, reason: String },
}
