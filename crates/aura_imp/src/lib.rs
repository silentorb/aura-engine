//! Imp graph integration for Aura.
//!
//! Aura Imp graphs denote pure **Time → Sample** functions. Translation and sampling
//! live in `aura_integration`; this crate provides node libraries and JSON helpers.

pub mod constraints;
pub mod dsp;
pub mod envelope;
pub mod music;
mod node_def;
pub mod registry;
pub mod signals;
pub mod time;

pub use imp_core_types::{
    core_node_library, Edge, EdgeId, Graph, GraphLibrary, InputValues, Node, NodeId, NodeLibrary,
    NodeType, NodeTypeId, Port, PortId, PortReference, Ports, PrimitiveValue, SignalType,
    SignalTypeId,
};
pub use imp_graph_resolve::flatten_graph;
pub use imp_registry::{
    create_registry, get_node_definition, get_node_type, get_type_constraint, list_graph_libraries,
    list_libraries, list_node_types, load_graph_library, load_library, load_type_constraint_library,
    DuplicateNodeTypeError, Registry,
};
pub use registry::{
    aura_registry, aura_registry_with_libraries, discover_graph_libraries,
    discover_graph_library_paths, graph_from_json_path, graph_from_json_str,
    graph_library_from_json_path, graph_library_from_json_str, graph_to_json_string, GraphJsonError,
    RegistryError,
};
