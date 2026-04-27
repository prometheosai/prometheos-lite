//! YAML flow file loader

use anyhow::{Context, Result};
use std::path::Path;
use std::fs;

use super::{FlowLoader, FlowFile};

/// YAML flow file loader
pub struct YamlLoader;

impl YamlLoader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for YamlLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowLoader for YamlLoader {
    fn load_from_path(&self, path: &Path) -> Result<FlowFile> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read YAML file: {}", path.display()))?;

        let flow_file: FlowFile = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse YAML file: {}", path.display()))?;

        Ok(flow_file)
    }
}
