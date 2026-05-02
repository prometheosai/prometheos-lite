use crate::harness::execution_loop::HarnessExecutionResult;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessArtifact {
    pub kind: String,
    pub path: Option<PathBuf>,
    pub content: Option<String>,
    pub metadata: serde_json::Value,
}
pub fn generate_completion_artifact(r: &HarnessExecutionResult) -> Result<String> {
    Ok(serde_json::to_string_pretty(r)?)
}
