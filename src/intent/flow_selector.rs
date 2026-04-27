//! FlowSelector - Intent to Flow mapping layer

use anyhow::Result;
use std::path::PathBuf;

use crate::intent::Intent;

/// Flow path for a given intent
pub type FlowPath = PathBuf;

/// FlowSelector trait for mapping intents to flow files
pub trait FlowSelector: Send + Sync {
    /// Select the appropriate flow file for a given intent
    fn select_flow(&self, intent: &Intent) -> Result<FlowPath>;

    /// Get the default flow path (fallback when intent is ambiguous)
    fn default_flow(&self) -> FlowPath;
}

/// Default implementation of FlowSelector
pub struct DefaultFlowSelector {
    /// Base directory for flow files
    flows_dir: PathBuf,
}

impl DefaultFlowSelector {
    /// Create a new DefaultFlowSelector with the given flows directory
    pub fn new(flows_dir: PathBuf) -> Self {
        Self { flows_dir }
    }

    /// Create a DefaultFlowSelector with default flows directory
    pub fn with_default_dir() -> Self {
        Self::new(PathBuf::from("flows"))
    }
}

impl FlowSelector for DefaultFlowSelector {
    fn select_flow(&self, intent: &Intent) -> Result<FlowPath> {
        let flow_name = match intent {
            Intent::Conversation => "chat.flow.yaml",
            Intent::Planning => "planning.flow.yaml",
            Intent::CodingTask => "codegen.flow.yaml",
            Intent::Approval => "approval.flow.yaml",
            Intent::Question => "chat.flow.yaml", // Default to chat for questions
            _ => "chat.flow.yaml",                // Default to chat for other intents
        };

        let flow_path = self.flows_dir.join(flow_name);

        if !flow_path.exists() {
            anyhow::bail!("Flow file not found: {}", flow_path.display());
        }

        Ok(flow_path)
    }

    fn default_flow(&self) -> FlowPath {
        self.flows_dir.join("chat.flow.yaml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_flow_selector_direct_chat() {
        let temp_dir = TempDir::new().unwrap();
        let flows_dir = temp_dir.path();

        // Create a dummy flow file
        fs::write(flows_dir.join("chat.flow.yaml"), "version: 1").unwrap();

        let selector = DefaultFlowSelector::new(flows_dir.to_path_buf());
        let intent = Intent::Conversation;

        let flow_path = selector.select_flow(&intent).unwrap();
        assert!(flow_path.ends_with("chat.flow.yaml"));
    }

    #[test]
    fn test_flow_selector_codegen() {
        let temp_dir = TempDir::new().unwrap();
        let flows_dir = temp_dir.path();

        fs::write(flows_dir.join("codegen.flow.yaml"), "version: 1").unwrap();

        let selector = DefaultFlowSelector::new(flows_dir.to_path_buf());
        let intent = Intent::CodingTask;

        let flow_path = selector.select_flow(&intent).unwrap();
        assert!(flow_path.ends_with("codegen.flow.yaml"));
    }

    #[test]
    fn test_flow_selector_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let flows_dir = temp_dir.path();

        let selector = DefaultFlowSelector::new(flows_dir.to_path_buf());
        let intent = Intent::Conversation;

        let result = selector.select_flow(&intent);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_flow() {
        let selector = DefaultFlowSelector::with_default_dir();
        let default = selector.default_flow();
        assert!(default.ends_with("chat.flow.yaml"));
    }
}
