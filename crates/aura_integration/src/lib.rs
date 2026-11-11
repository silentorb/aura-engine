//! Surface integration: Imp graph translation and sampling for Aura.

mod duration;
mod miro;
mod notation_file;
mod translate;

use translate::{infer_score_duration_secs, translate_graph};
use aura_imp::{
    aura_registry_with_libraries, discover_graph_libraries, graph_from_json_path, Graph,
    GraphJsonError, Registry, RegistryError,
};
use aura_io_wav::{write_wav32, WavError};
use aura_render::{sample_offline, RenderError, RenderSpec, Sampler};
use aura_sample::SampleRate;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub use duration::{DurationParseError, DurationSpec, SampleSpec};
pub use miro::{
    aura_json_kind, aura_json_kind_fn, miro_sync_content_root, open_aura_miro_session,
    open_aura_miro_session_in_memory, pull_graph, AuraMiroError,
};
pub use notation_file::{score_from_notation_path, score_from_notation_str, NotationFileError};
pub use translate::{
    find_output_node, translate_graph_with_bindings, translate_graph_with_options, TranslateError,
    TranslateOptions,
};

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("{0}")]
    GraphJson(#[from] GraphJsonError),
    #[error("{0}")]
    Translate(#[from] TranslateError),
    #[error("{0}")]
    Duration(#[from] DurationParseError),
    #[error("{0}")]
    Render(#[from] RenderError),
    #[error("{0}")]
    Wav(#[from] WavError),
    #[error("failed to create output directory: {0}")]
    CreateDir(#[from] std::io::Error),
    #[error("failed to load Aura Imp registry: {0}")]
    Registry(#[from] RegistryError),
    #[error("failed to resolve graph composites: {0}")]
    Resolve(String),
}

/// A graph loaded with its neighboring graph libraries and merged registry.
pub struct LoadedGraph {
    pub graph: Graph,
    pub registry: Registry,
    /// Directory containing the graph file; used to resolve relative `notation_score` paths.
    pub graph_dir: Option<PathBuf>,
}

/// Loads an Imp graph and neighboring graph libraries from a JSON file path.
pub fn load_graph(path: &Path) -> Result<LoadedGraph, IntegrationError> {
    let graph = graph_from_json_path(path)?;
    let libraries = discover_graph_libraries(path)?;
    let registry = aura_registry_with_libraries(&libraries)?;
    let graph_dir = path.parent().map(Path::to_path_buf);
    Ok(LoadedGraph {
        graph,
        registry,
        graph_dir,
    })
}

/// Loads an Imp graph from a JSON file path (graph only, no libraries).
pub fn load_graph_only(path: &Path) -> Result<Graph, IntegrationError> {
    Ok(graph_from_json_path(path)?)
}

/// Translates an Imp graph into a pure Time → Sample function.
pub fn translate_graph_to_sampler(
    graph: &Graph,
    registry: &aura_imp::Registry,
    sample_rate: SampleRate,
) -> Result<Box<dyn Sampler>, IntegrationError> {
    Ok(translate_graph(graph, registry, sample_rate)?)
}

/// Translates an Imp graph with optional score bindings and a graph directory for `notation_score`.
pub fn translate_graph_to_sampler_with_options(
    graph: &Graph,
    registry: &aura_imp::Registry,
    sample_rate: SampleRate,
    options: TranslateOptions<'_>,
) -> Result<Box<dyn Sampler>, IntegrationError> {
    Ok(translate_graph_with_options(
        graph,
        registry,
        sample_rate,
        options,
    )?)
}

/// Translates an Imp graph with bound [`aura_sequencing::ScoreSignal`] values for `external_score` nodes.
pub fn translate_graph_to_sampler_with_bindings(
    graph: &Graph,
    registry: &aura_imp::Registry,
    sample_rate: SampleRate,
    score_bindings: &BTreeMap<aura_imp::NodeId, aura_sequencing::ScoreSignal>,
) -> Result<Box<dyn Sampler>, IntegrationError> {
    Ok(translate_graph_with_bindings(
        graph,
        registry,
        sample_rate,
        score_bindings,
    )?)
}

/// Resolves render duration from explicit spec or score-implied length.
pub fn resolve_duration_secs(graph: &Graph, spec: &SampleSpec) -> Result<f64, IntegrationError> {
    if let Ok(secs) = spec.duration.to_seconds() {
        return Ok(secs);
    }

    infer_score_duration_secs(graph, spec.sample_rate)
        .ok_or(DurationParseError::InvalidSeconds(0.0))
        .map_err(IntegrationError::from)
}

/// Samples a translated graph over the requested interval.
pub fn sample_graph(sampler: &dyn Sampler, spec: SampleSpec) -> Result<Vec<f32>, IntegrationError> {
    let duration_secs = resolve_duration_secs_for_render(&spec)?;
    let render_spec = RenderSpec {
        sample_rate: spec.sample_rate,
        duration_secs,
        channels: 1,
    };
    Ok(sample_offline(render_spec, sampler)?)
}

/// Loads, translates, and samples a graph to mono PCM.
pub fn render_graph_to_pcm(loaded: &LoadedGraph, spec: SampleSpec) -> Result<Vec<f32>, IntegrationError> {
    let empty_bindings = BTreeMap::new();
    let sampler = translate_graph_with_options(
        &loaded.graph,
        &loaded.registry,
        spec.sample_rate,
        TranslateOptions {
            score_bindings: &empty_bindings,
            graph_dir: loaded.graph_dir.as_deref(),
        },
    )?;
    sample_graph(sampler.as_ref(), spec)
}

/// Loads, translates, and samples a graph file to mono PCM.
pub fn render_graph_file_to_pcm(path: &Path, spec: SampleSpec) -> Result<Vec<f32>, IntegrationError> {
    let loaded = load_graph(path)?;
    render_graph_to_pcm(&loaded, spec)
}

/// Writes mono PCM samples to a 32-bit float WAV file.
pub fn write_pcm_to_wav(
    path: &Path,
    samples: &[f32],
    sample_rate: SampleRate,
) -> Result<(), IntegrationError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(write_wav32(path, samples, sample_rate)?)
}

fn resolve_duration_secs_for_render(spec: &SampleSpec) -> Result<f64, IntegrationError> {
    spec.duration.to_seconds().map_err(IntegrationError::from)
}

/// Parses CLI duration strings such as `10`, `10s`, or `2m`.
pub fn parse_duration_spec(input: &str, tempo_bpm: f64) -> Result<DurationSpec, DurationParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DurationParseError::InvalidSeconds(0.0));
    }

    if let Some(stripped) = trimmed.strip_suffix('s') {
        let secs: f64 = stripped.parse().map_err(|_| DurationParseError::InvalidSeconds(0.0))?;
        return Ok(DurationSpec::Seconds(secs));
    }

    if let Some(stripped) = trimmed.strip_suffix('m') {
        let measures: f64 = stripped.parse().map_err(|_| DurationParseError::InvalidMeasures(0.0))?;
        let tempo = aura_sequencing::Tempo::new(tempo_bpm)
            .map_err(|_| DurationParseError::InvalidMeasures(measures))?;
        return Ok(DurationSpec::measures_4_4(measures, tempo));
    }

    let secs: f64 = trimmed.parse().map_err(|_| DurationParseError::InvalidSeconds(0.0))?;
    Ok(DurationSpec::Seconds(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn workspace_demo_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos").join(name)
    }

    #[test]
    fn parse_duration_supports_seconds_and_measures() {
        let secs = parse_duration_spec("10s", 120.0).expect("seconds");
        assert_eq!(secs, DurationSpec::Seconds(10.0));

        let measures = parse_duration_spec("2m", 120.0).expect("measures");
        assert_eq!(
            measures,
            DurationSpec::measures_4_4(2.0, aura_sequencing::Tempo::default()),
        );
    }

    #[test]
    fn render_sine_graph_matches_expected_frames() {
        let loaded = load_graph(&workspace_demo_path("sine.json")).expect("load");
        let spec = SampleSpec {
            sample_rate: SampleRate::RATE_44100,
            duration: DurationSpec::Seconds(10.0),
        };
        let pcm = render_graph_to_pcm(&loaded, spec).expect("render");
        assert_eq!(pcm.len(), 441_000);
    }

    #[test]
    fn render_arpeggio_graph_matches_expected_frames() {
        let loaded = load_graph(&workspace_demo_path("demo-01.json")).expect("load");
        let spec = SampleSpec {
            sample_rate: SampleRate::RATE_44100,
            duration: DurationSpec::measures_4_4(8.0, aura_sequencing::Tempo::default()),
        };
        let pcm = render_graph_to_pcm(&loaded, spec).expect("render");
        assert_eq!(pcm.len(), 705_600);
    }

    #[test]
    fn render_static_arpeggio_demo_matches_expected_frames() {
        let loaded = load_graph(&workspace_demo_path("demo-02.json")).expect("load");
        let spec = SampleSpec {
            sample_rate: SampleRate::RATE_44100,
            duration: DurationSpec::measures_4_4(8.0, aura_sequencing::Tempo::default()),
        };
        let pcm = render_graph_to_pcm(&loaded, spec).expect("render");
        assert_eq!(pcm.len(), 705_600);
    }

    #[test]
    fn sampler_is_pure() {
        let loaded = load_graph(&workspace_demo_path("sine.json")).expect("load");
        let sampler =
            translate_graph(&loaded.graph, &loaded.registry, SampleRate::RATE_44100).expect("translate");
        assert_eq!(sampler.at(0.25), sampler.at(0.25));
    }
}
