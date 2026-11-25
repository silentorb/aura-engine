//! Compose a graph document with nodes from a Tome project.
//!
//! Node resolution is Tome's job (`tome_imp::TomeNodeProvider`); Aura only defines node types.
//! This module layers the provider behind the document's own nodes and reads the bodies of
//! notation records the graph references, so translation can bind their scores.

use crate::notation_file::score_from_notation_str;
use crate::translate::{translate_graph_with_options, TranslateError, TranslateOptions};
use aura_imp::{
    get_node_definitions, graph_source, Graph, GraphSource, LayeredNodes, NodeId, NodeProvider,
    Registry, NOTATION_COLUMN, NOTATION_NODE_TYPE,
};
use aura_plugin_host::PluginResolver;
use aura_render::AudioSignal;
use aura_sample::SampleRate;
use aura_sequencing::ScoreSignal;
use imp_core_types::unresolved_node_ids;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

pub use tome_imp::TomeNodeProvider;

#[derive(Debug, Error)]
pub enum ProjectSourceError {
    #[error("project {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: tome_imp::TomeImpError,
    },
    #[error("notation record `{node}`: {message}")]
    Notation { node: NodeId, message: String },
    #[error("{0}")]
    Translate(#[from] TranslateError),
}

/// A query cache for one open of a project, outside the project so opening never writes into it.
///
/// Unique per open: the provider is a snapshot, so nothing reuses the cache, and concurrent opens
/// of the same project must not share (and clean) one file.
fn cache_path_for(config_path: &Path) -> PathBuf {
    static OPENS: AtomicU64 = AtomicU64::new(0);
    let open = OPENS.fetch_add(1, Ordering::Relaxed);
    let key = std::fs::canonicalize(config_path)
        .unwrap_or_else(|_| config_path.to_path_buf())
        .display()
        .to_string();
    // FNV-1a: a stable file name per project path.
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    std::env::temp_dir()
        .join("aura-tome-cache")
        .join(format!("{hash:016x}-{}-{open}.sqlite", std::process::id()))
}

/// Open a Tome project's nodes. A record is an Imp node when its type hub is a registered type.
pub fn open_project_nodes(
    config_path: &Path,
    registry: &Registry,
) -> Result<TomeNodeProvider, ProjectSourceError> {
    let cache = cache_path_for(config_path);
    let opened = tome_imp::open_project(config_path, &cache, |type_id| {
        !get_node_definitions(registry, type_id).is_empty()
    });
    // The snapshot no longer needs its cache; a leftover temp file is harmless if removal fails.
    let _ = std::fs::remove_file(&cache);
    opened.map_err(|source| ProjectSourceError::Open {
        path: config_path.display().to_string(),
        source,
    })
}

/// Scores of the notation records `graph` references but does not hold.
pub fn notation_score_bindings(
    graph: &Graph,
    project: &TomeNodeProvider,
) -> Result<BTreeMap<NodeId, ScoreSignal>, ProjectSourceError> {
    let mut bindings = BTreeMap::new();
    for id in unresolved_node_ids(graph_source(graph)) {
        let is_notation = project
            .get_node(&id)
            .is_some_and(|node| node.node_type == NOTATION_NODE_TYPE);
        if !is_notation {
            continue;
        }
        let body = tome_imp::node_property(project, &id, NOTATION_COLUMN).ok_or_else(|| {
            ProjectSourceError::Notation {
                node: id.clone(),
                message: format!("missing `{NOTATION_COLUMN}` body"),
            }
        })?;
        let mut body = body.clone();
        if let Some(object) = body.as_object_mut() {
            object
                .entry("type")
                .or_insert_with(|| Value::from("notation"));
        }
        let score = score_from_notation_str(&body.to_string()).map_err(|message| {
            ProjectSourceError::Notation {
                node: id.clone(),
                message,
            }
        })?;
        bindings.insert(id, ScoreSignal::finite(score));
    }
    Ok(bindings)
}

/// Translate `graph`, resolving the nodes it references from `project` when given.
pub fn translate_graph_with_project(
    graph: &Graph,
    registry: &Registry,
    sample_rate: SampleRate,
    project: Option<&Path>,
    plugins: Option<&dyn PluginResolver>,
) -> Result<AudioSignal, ProjectSourceError> {
    let project = match project {
        Some(path) => open_project_nodes(path, registry)?,
        None => TomeNodeProvider::default(),
    };
    let score_bindings = notation_score_bindings(graph, &project)?;
    let nodes = LayeredNodes {
        providers: vec![&graph.nodes, &project],
    };
    let source = GraphSource {
        nodes: &nodes,
        edges: &graph.edges,
    };
    Ok(translate_graph_with_options(
        source,
        registry,
        sample_rate,
        TranslateOptions {
            score_bindings: &score_bindings,
            plugins,
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_imp::{aura_registry_with_libraries, discover_graph_libraries};

    fn workspace_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    #[test]
    fn demo_02_plays_its_arpeggio_from_the_demo_project() {
        let graph_path = workspace_path("demos/demo-02.json");
        let graph = aura_imp::graph_from_json_path(&graph_path).expect("graph");
        let libraries = discover_graph_libraries(&graph_path).expect("libraries");
        let registry = aura_registry_with_libraries(&libraries).expect("registry");
        let project = workspace_path("lab-pack/demo-project/imp-project.json");

        let nodes = open_project_nodes(&project, &registry).expect("project");
        let bindings = notation_score_bindings(&graph, &nodes).expect("bindings");
        assert_eq!(
            bindings.keys().collect::<Vec<_>>(),
            ["01M7HJ0CRN88N5QEQQQ1GBJT8H"]
        );

        let mut signal =
            translate_graph_with_project(&graph, &registry, SampleRate::RATE_44100, Some(&project), None)
                .expect("translate")
                .into_mono()
                .expect("mono");
        let mut a = [0.0; 64];
        let mut b = [0.0; 64];
        signal.render(11_025, &mut a);
        signal.render(11_025, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn demo_02_without_a_project_reports_the_unresolved_notation() {
        let graph_path = workspace_path("demos/demo-02.json");
        let graph = aura_imp::graph_from_json_path(&graph_path).expect("graph");
        let libraries = discover_graph_libraries(&graph_path).expect("libraries");
        let registry = aura_registry_with_libraries(&libraries).expect("registry");
        let err =
            match translate_graph_with_project(&graph, &registry, SampleRate::RATE_44100, None, None) {
                Ok(_) => panic!("expected an unresolved notation"),
                Err(error) => error,
            };
        assert!(
            matches!(&err, ProjectSourceError::Translate(TranslateError::TypeCheck { message }) if message.contains("unknown node")),
            "{err}"
        );
    }
}
