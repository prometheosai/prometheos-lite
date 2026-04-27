//! Flow file types for serialization

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Flow inputs specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowInputs {
    pub required: Vec<String>,
}

/// Flow outputs specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowOutputs {
    pub primary: String,
    pub include: Vec<String>,
}

/// Flow file format for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowFile {
    pub version: String,
    pub name: String,
    pub description: Option<String>,
    pub start_node: String,
    pub inputs: Option<FlowInputs>,
    pub outputs: Option<FlowOutputs>,
    pub nodes: Vec<NodeDefinition>,
    pub transitions: Vec<TransitionDefinition>,
}

impl FlowFile {
    /// Validate the flow file structure according to Flow JSON Schema v1
    pub fn validate(&self) -> Result<()> {
        // Validate version is not empty
        if self.version.is_empty() {
            anyhow::bail!("Flow version cannot be empty");
        }

        // Validate name is not empty
        if self.name.is_empty() {
            anyhow::bail!("Flow name cannot be empty");
        }

        // Validate start_node is not empty
        if self.start_node.is_empty() {
            anyhow::bail!("Start node cannot be empty");
        }

        // Validate nodes is not empty
        if self.nodes.is_empty() {
            anyhow::bail!("Flow must have at least one node");
        }

        // Validate inputs if defined
        if let Some(ref inputs) = self.inputs {
            if inputs.required.is_empty() {
                anyhow::bail!("Flow inputs.required cannot be empty if inputs is defined");
            }
        }

        // Validate outputs if defined
        if let Some(ref outputs) = self.outputs {
            if outputs.primary.is_empty() {
                anyhow::bail!("Flow outputs.primary cannot be empty if outputs is defined");
            }
        }

        // Validate each node definition
        for node in &self.nodes {
            node.validate()?;
        }

        // Validate transitions
        for transition in &self.transitions {
            transition.validate()?;
        }

        // Validate start_node exists in nodes
        let node_ids: std::collections::HashSet<_> = self.nodes.iter().map(|n| &n.id).collect();
        if !node_ids.contains(&self.start_node) {
            anyhow::bail!("Start node '{}' not found in nodes", self.start_node);
        }

        // Validate all transition sources and targets exist
        for transition in &self.transitions {
            if !node_ids.contains(&transition.from) {
                anyhow::bail!("Transition source node '{}' not found", transition.from);
            }
            if !node_ids.contains(&transition.to) {
                anyhow::bail!("Transition target node '{}' not found", transition.to);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub id: String,
    pub node_type: String,
    pub config: Option<serde_json::Value>,
}

impl NodeDefinition {
    /// Validate node definition according to Flow JSON Schema v1
    pub fn validate(&self) -> Result<()> {
        // Validate id is not empty
        if self.id.is_empty() {
            anyhow::bail!("Node id cannot be empty");
        }

        // Validate node_type is not empty
        if self.node_type.is_empty() {
            anyhow::bail!("Node type cannot be empty");
        }

        // Validate node_type is one of the known types
        let valid_types = [
            "planner", "coder", "reviewer", "llm", "tool",
            "file_writer", "context_loader", "memory_write", "conditional"
        ];

        if !valid_types.contains(&self.node_type.as_str()) {
            // Warn but don't fail - will default to passthrough
            eprintln!("Warning: Unknown node type '{}', will use passthrough", self.node_type);
        }

        // Validate config if present
        if let Some(config) = &self.config {
            if !config.is_object() {
                anyhow::bail!("Node config must be a JSON object");
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionDefinition {
    pub from: String,
    pub action: String,
    pub to: String,
}

impl TransitionDefinition {
    /// Validate transition definition
    pub fn validate(&self) -> Result<()> {
        if self.from.is_empty() {
            anyhow::bail!("Transition 'from' cannot be empty");
        }
        if self.action.is_empty() {
            anyhow::bail!("Transition 'action' cannot be empty");
        }
        if self.to.is_empty() {
            anyhow::bail!("Transition 'to' cannot be empty");
        }
        Ok(())
    }
}
