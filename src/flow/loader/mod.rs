//! Flow file loader for YAML and JSON formats

mod json;
mod yaml;
mod validate;

#[cfg(test)]
mod tests;

pub use json::JsonLoader;
pub use yaml::YamlLoader;
pub use validate::validate_flow_file;

use anyhow::Result;
use std::path::Path;

/// Flow loader trait for loading flows from different formats
pub trait FlowLoader {
    /// Load a flow file from a path
    fn load_from_path(&self, path: &Path) -> Result<FlowFile>;
}

/// Flow file structure (re-exported from CLI types for now)
/// TODO: Move this to src/flow/types.rs in Phase 1
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FlowInputs {
    pub required: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FlowOutputs {
    pub primary: String,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeDefinition {
    pub id: String,
    pub node_type: String,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransitionDefinition {
    pub from: String,
    pub action: String,
    pub to: String,
}
