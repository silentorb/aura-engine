//! Aura Imp registry and JSON helpers.

use crate::{dsp, envelope, music, plugin, time, types};
use imp_core_types::{Graph, ImpFile, IMP_FILE_FORMAT};
use imp_file::{imp_file_from_graph, imp_file_graph, parse_imp_file_json, ImpFileParseError};
use imp_registry::{
    create_registry, load_imp_file, load_library, Registry, RegistryError as ImpRegistryError,
};
use imp_typecheck::check_definition;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const INSTRUMENTS_LIBRARY_FILE: &str = "instruments.json";

/// Returns a registry with core Imp nodes and all Aura libraries loaded.
pub fn aura_registry() -> Result<Registry, RegistryError> {
    aura_registry_with_files(&[])
}

/// Returns a registry with Aura node libraries plus the `types` and `definitions` of the given
/// Imp files loaded. Graph-backed definitions are type-checked as they load.
pub fn aura_registry_with_files(files: &[ImpFile]) -> Result<Registry, RegistryError> {
    let registry = create_registry();
    let registry = load_library(registry, imp_core_types::core_node_library())?;
    let registry = load_library(registry, types::aura_types_library())?;
    let registry = load_library(registry, time::time_node_library())?;
    let registry = load_library(registry, dsp::dsp_node_library())?;
    let registry = load_library(registry, envelope::envelope_node_library())?;
    let registry = load_library(registry, music::music_node_library())?;
    let registry = load_library(registry, plugin::plugin_node_library())?;

    let mut registry = registry;
    for file in files {
        registry = load_imp_file(registry, file.clone()).map_err(RegistryError::from)?;
        for definition in &file.definitions {
            let errors = check_definition(definition, &registry);
            if let Some(error) = errors.first() {
                return Err(RegistryError::DefinitionTypeCheck {
                    library_id: file.id.clone().unwrap_or_default(),
                    definition_id: definition.id.clone(),
                    message: error.message.clone(),
                });
            }
        }
    }

    Ok(registry)
}

/// Discovers library Imp files neighboring a graph path.
///
/// Convention: `{parent}/instruments.json` when present.
pub fn discover_imp_file_paths(graph_path: &Path) -> Vec<PathBuf> {
    let Some(parent) = graph_path.parent() else {
        return Vec::new();
    };
    let library_path = parent.join(INSTRUMENTS_LIBRARY_FILE);
    if library_path.is_file() {
        vec![library_path]
    } else {
        Vec::new()
    }
}

/// Loads library Imp files from neighboring paths for a graph file.
pub fn discover_imp_files(graph_path: &Path) -> Result<Vec<ImpFile>, GraphJsonError> {
    discover_imp_file_paths(graph_path)
        .into_iter()
        .map(|path| imp_file_from_json_path(&path))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    ImpRegistry(#[from] ImpRegistryError),
    #[error("Imp file \"{library_id}\" definition \"{definition_id}\": {message}")]
    DefinitionTypeCheck {
        library_id: String,
        definition_id: String,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum GraphJsonError {
    #[error("failed to read graph JSON from {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse Imp file from {path}: {source}")]
    Parse {
        path: String,
        source: ImpFileParseError,
    },
    #[error("failed to serialize graph JSON: {0}")]
    Serialize(serde_json::Error),
}

/// Loads an Imp file from a JSON path.
pub fn imp_file_from_json_path(path: &Path) -> Result<ImpFile, GraphJsonError> {
    let path_str = path.to_string_lossy().into_owned();
    let contents = fs::read_to_string(path).map_err(|source| GraphJsonError::Read {
        path: path_str.clone(),
        source,
    })?;
    imp_file_from_json_str(&contents).map_err(|source| GraphJsonError::Parse {
        path: path_str,
        source,
    })
}

/// Parses and validates an Imp file from a JSON string.
pub fn imp_file_from_json_str(json: &str) -> Result<ImpFile, ImpFileParseError> {
    parse_imp_file_json(json)
}

/// Loads the nodes and edges of an Imp file at `path` as a `Graph`.
pub fn graph_from_json_path(path: &Path) -> Result<Graph, GraphJsonError> {
    Ok(imp_file_graph(&imp_file_from_json_path(path)?))
}

/// Parses the nodes and edges of an Imp file as a `Graph`.
pub fn graph_from_json_str(json: &str) -> Result<Graph, ImpFileParseError> {
    Ok(imp_file_graph(&imp_file_from_json_str(json)?))
}

/// Serializes a graph as an Imp file (`format: "imp"`, nodes and edges).
pub fn graph_to_json_string(graph: &Graph) -> Result<String, GraphJsonError> {
    let file = ImpFile {
        format: Some(IMP_FILE_FORMAT.into()),
        ..imp_file_from_graph(graph)
    };
    serde_json::to_string_pretty(&file).map_err(GraphJsonError::Serialize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imp_registry::get_node_type;

    #[test]
    fn aura_registry_contains_expected_nodes() {
        let registry = aura_registry().expect("registry");
        assert!(get_node_type(&registry, "time").is_some());
        assert!(get_node_type(&registry, "sine").is_some());
        assert!(get_node_type(&registry, "arpeggio").is_some());
        assert!(get_node_type(&registry, "epic_minor_progression").is_some());
        assert!(get_node_type(&registry, "constant_tempo").is_some());
        assert!(get_node_type(&registry, "constant_time_signature").is_some());
        assert!(get_node_type(&registry, "loop").is_some());
        assert!(get_node_type(&registry, "external_score").is_none());
        assert!(get_node_type(&registry, "notation_score").is_none());
    }

    #[test]
    fn discovers_instruments_library_next_to_demo_graph() {
        let graph_path = workspace_demo_path("demo-01.json");
        let paths = discover_imp_file_paths(&graph_path);
        assert_eq!(paths, vec![workspace_demo_path("instruments.json")]);
    }

    #[test]
    fn loads_demo_instruments_library() {
        let graph_path = workspace_demo_path("demo-01.json");
        let libraries = discover_imp_files(&graph_path).expect("libraries");
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id.as_deref(), Some("aura.demo-instruments"));
        assert_eq!(libraries[0].definitions.len(), 4);

        let registry = aura_registry_with_files(&libraries).expect("registry");
        assert!(get_node_type(&registry, "kick_drum").is_some());
        assert!(get_node_type(&registry, "sine_note_voice").is_some());
    }

    fn workspace_demo_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos").join(name)
    }
}
