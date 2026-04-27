//! Flow file validation

use anyhow::{bail, Result};

use super::{FlowFile, NodeDefinition, TransitionDefinition};

/// Validate a flow file structure
pub fn validate_flow_file(flow: &FlowFile) -> Result<()> {
    // Validate version is not empty
    if flow.version.is_empty() {
        bail!("Flow version cannot be empty");
    }

    // Validate name is not empty
    if flow.name.is_empty() {
        bail!("Flow name cannot be empty");
    }

    // Validate start_node is not empty
    if flow.start_node.is_empty() {
        bail!("Start node cannot be empty");
    }

    // Validate nodes is not empty
    if flow.nodes.is_empty() {
        bail!("Flow must have at least one node");
    }

    // Validate inputs if defined
    if let Some(ref inputs) = flow.inputs {
        if inputs.required.is_empty() {
            bail!("Flow inputs.required cannot be empty if inputs is defined");
        }
    }

    // Validate outputs if defined
    if let Some(ref outputs) = flow.outputs {
        if outputs.primary.is_empty() {
            bail!("Flow outputs.primary cannot be empty if outputs is defined");
        }
    }

    // Validate each node definition
    for node in &flow.nodes {
        validate_node_definition(node)?;
    }

    // Validate transitions
    for transition in &flow.transitions {
        validate_transition_definition(transition)?;
    }

    // Validate start_node exists in nodes
    let node_ids: std::collections::HashSet<_> = flow.nodes.iter().map(|n| &n.id).collect();
    if !node_ids.contains(&flow.start_node) {
        bail!("Start node '{}' not found in nodes", flow.start_node);
    }

    // Validate all transition sources and targets exist
    for transition in &flow.transitions {
        if !node_ids.contains(&transition.from) {
            bail!("Transition source node '{}' not found", transition.from);
        }
        if !node_ids.contains(&transition.to) {
            bail!("Transition target node '{}' not found", transition.to);
        }
    }

    Ok(())
}

fn validate_node_definition(node: &NodeDefinition) -> Result<()> {
    // Validate id is not empty
    if node.id.is_empty() {
        bail!("Node id cannot be empty");
    }

    // Validate node_type is not empty
    if node.node_type.is_empty() {
        bail!("Node type cannot be empty");
    }

    // Validate node_type is one of the known types (strict by default)
    let valid_types = [
        "planner", "coder", "reviewer", "llm", "tool",
        "file_writer", "context_loader", "memory_write", "conditional",
        "passthrough",
    ];

    if !valid_types.contains(&node.node_type.as_str()) {
        bail!(
            "Unknown node type '{}' in node '{}'. Valid types: {}",
            node.node_type,
            node.id,
            valid_types.join(", ")
        );
    }

    // Validate config if present
    if let Some(config) = &node.config {
        if !config.is_object() {
            bail!("Node config must be a JSON object");
        }
    }

    Ok(())
}

fn validate_transition_definition(transition: &TransitionDefinition) -> Result<()> {
    if transition.from.is_empty() {
        bail!("Transition 'from' cannot be empty");
    }
    if transition.action.is_empty() {
        bail!("Transition 'action' cannot be empty");
    }
    if transition.to.is_empty() {
        bail!("Transition 'to' cannot be empty");
    }
    Ok(())
}
