//! Imp graph integration for Aura.
//!
//! Aura Imp graphs denote pure **Time → Sample** functions. Translation and sampling
//! live in `aura_integration`; this crate provides node libraries and JSON helpers.

pub mod types;
pub mod dsp;
pub mod envelope;
pub mod music;
pub mod plugin;
mod node_def;
pub mod registry;
pub mod signals;
pub mod time;

pub use imp_core_types::{
    concrete_type, core_node_library, graph_source, materialize_graph, Edge, EdgeId, Graph,
    GraphSource, ImpFile, InputValues, LayeredNodes, Node, NodeProvider,
    NodeDefinition, NodeId, NodeLibrary, NodeType, NodeTypeId, Port, PortId, PortReference, Ports,
    PrimitiveValue, Type, TypeDefinition, TypeId,
};
pub use imp_file::{imp_file_from_graph, imp_file_graph, ImpFileParseError};
pub use imp_graph_resolve::flatten_graph;
pub use imp_registry::{
    create_registry, get_node_definition, get_node_definitions, get_node_type, get_type_definition,
    list_imp_files, list_libraries, list_node_types, load_imp_file, load_library,
    DuplicateNodeTypeError, Registry,
};
pub use music::{NOTATION_COLUMN, NOTATION_NODE_TYPE};
pub use registry::{
    aura_registry, aura_registry_with_files, discover_imp_file_paths, discover_imp_files,
    graph_from_json_path, graph_from_json_str, graph_to_json_string, imp_file_from_json_path,
    imp_file_from_json_str, GraphJsonError, RegistryError,
};
