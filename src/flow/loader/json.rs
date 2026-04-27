//! JSON flow file loader

use anyhow::{Context, Result};
use std::path::Path;
use std::fs;

use super::{FlowLoader, FlowFile};

/// JSON flow file loader
pub struct JsonLoader;

impl JsonLoader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowLoader for JsonLoader {
    fn load_from_path(&self, path: &Path) -> Result<FlowFile> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read JSON file: {}", path.display()))?;

        let flow_file: FlowFile = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse JSON file: {}", path.display()))?;

        Ok(flow_file)
    }
}
