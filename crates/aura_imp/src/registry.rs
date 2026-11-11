//! Aura Imp registry and JSON helpers.

use crate::{constraints, dsp, envelope, music, time};
use imp_core_types::{Graph, GraphLibrary};
use imp_graph_library::{parse_graph_library_json, GraphLibraryParseError};
use imp_registry::{
    create_registry, load_graph_library, load_library, load_type_constraint_library,
    DuplicateNodeTypeError, DuplicateTypeConstraintError, Registry, RegistryError as ImpRegistryError,
};
use imp_typecheck::check_definition;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const INSTRUMENTS_LIBRARY_FILE: &str = "instruments.json";

/// Returns a registry with core Imp nodes and all Aura libraries loaded.
pub fn aura_registry() -> Result<Registry, RegistryError> {
    aura_registry_with_libraries(&[])
}

/// Returns a registry with Aura node libraries and the given graph libraries loaded.
pub fn aura_registry_with_libraries(
    graph_libraries: &[GraphLibrary],
) -> Result<Registry, RegistryError> {
    let registry = create_registry();
    let registry = load_library(registry, imp_core_types::core_node_library())?;
    let registry = load_type_constraint_library(registry, constraints::aura_type_constraint_library())?;
    let registry = load_library(registry, time::time_node_library())?;
    let registry = load_library(registry, dsp::dsp_node_library())?;
    let registry = load_library(registry, envelope::envelope_node_library())?;
    let registry = load_library(registry, music::music_node_library())?;

    let mut registry = registry;
    for library in graph_libraries {
        registry = load_graph_library(registry, library.clone()).map_err(RegistryError::from)?;
        for definition in &library.definitions {
            let errors = check_definition(definition, &registry);
            if let Some(error) = errors.first() {
                return Err(RegistryError::DefinitionTypeCheck {
                    library_id: library.id.clone(),
                    definition_id: definition.id.clone(),
                    message: error.message.clone(),
                });
            }
        }
    }

    Ok(registry)
}

/// Discovers graph library files neighboring a graph path.
///
/// Convention: `{parent}/instruments.json` when present.
pub fn discover_graph_library_paths(graph_path: &Path) -> Vec<PathBuf> {
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

/// Loads graph libraries from neighboring paths for a graph file.
pub fn discover_graph_libraries(graph_path: &Path) -> Result<Vec<GraphLibrary>, GraphJsonError> {
    discover_graph_library_paths(graph_path)
        .into_iter()
        .map(|path| graph_library_from_json_path(&path))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    DuplicateNodeType(#[from] DuplicateNodeTypeError),
    #[error(transparent)]
    DuplicateTypeConstraint(#[from] DuplicateTypeConstraintError),
    #[error(transparent)]
    ImpRegistry(#[from] ImpRegistryError),
    #[error("graph library \"{library_id}\" definition \"{definition_id}\": {message}")]
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
    #[error("failed to parse graph JSON from {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("failed to parse graph library JSON from {path}: {message}")]
    LibraryParse {
        path: String,
        message: String,
    },
    #[error("failed to serialize graph JSON: {0}")]
    Serialize(serde_json::Error),
}

/// Loads an Imp graph from a JSON file.
pub fn graph_from_json_path(path: &Path) -> Result<Graph, GraphJsonError> {
    let path_str = path.to_string_lossy().into_owned();
    let contents = fs::read_to_string(path).map_err(|source| GraphJsonError::Read {
        path: path_str.clone(),
        source,
    })?;
    graph_from_json_str(&contents).map_err(|source| GraphJsonError::Parse {
        path: path_str,
        source,
    })
}

/// Parses an Imp graph from a JSON string.
pub fn graph_from_json_str(json: &str) -> Result<Graph, serde_json::Error> {
    serde_json::from_str(json)
}

/// Loads a graph library from a JSON file.
pub fn graph_library_from_json_path(path: &Path) -> Result<GraphLibrary, GraphJsonError> {
    let path_str = path.to_string_lossy().into_owned();
    let contents = fs::read_to_string(path).map_err(|source| GraphJsonError::Read {
        path: path_str.clone(),
        source,
    })?;
    graph_library_from_json_str(&contents).map_err(|err| GraphJsonError::LibraryParse {
        path: path_str,
        message: err.0,
    })
}

/// Parses a graph library from a JSON string.
pub fn graph_library_from_json_str(json: &str) -> Result<GraphLibrary, GraphLibraryParseError> {
    parse_graph_library_json(json)
}

/// Serializes an Imp graph to a JSON string.
pub fn graph_to_json_string(graph: &Graph) -> Result<String, GraphJsonError> {
    serde_json::to_string_pretty(graph).map_err(GraphJsonError::Serialize)
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
        assert!(get_node_type(&registry, "notation_score").is_some());
        assert!(get_node_type(&registry, "external_score").is_some());
    }

    #[test]
    fn discovers_instruments_library_next_to_demo_graph() {
        let graph_path = workspace_demo_path("demo-01.json");
        let paths = discover_graph_library_paths(&graph_path);
        assert_eq!(paths, vec![workspace_demo_path("instruments.json")]);
    }

    #[test]
    fn loads_demo_instruments_library() {
        let graph_path = workspace_demo_path("demo-01.json");
        let libraries = discover_graph_libraries(&graph_path).expect("libraries");
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].id, "aura.demo-instruments");
        assert_eq!(libraries[0].definitions.len(), 4);

        let registry = aura_registry_with_libraries(&libraries).expect("registry");
        assert!(get_node_type(&registry, "kick_drum").is_some());
        assert!(get_node_type(&registry, "sine_note_voice").is_some());
    }

    fn workspace_demo_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos").join(name)
    }
}
