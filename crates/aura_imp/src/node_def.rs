//! Helpers for building primitive Aura node definitions.

use imp_core_types::{NodeDefinition, NodeImplementation, Ports, TypeParam};

pub fn primitive_node(
    id: &str,
    type_params: Vec<TypeParam>,
    implementation: Option<NodeImplementation>,
    inputs: Ports,
    outputs: Ports,
) -> NodeDefinition {
    NodeDefinition {
        id: id.into(),
        name: None,
        type_params,
        implementation,
        inputs,
        outputs,
        body: None,
        bindings: None,
    }
}
