//! ArtifactMapper - maps flow outputs to WorkContext artifacts

use anyhow::Result;
use serde_json::Value;

use super::artifact::{Artifact, ArtifactKind};

/// ArtifactMapper - converts flow execution outputs into WorkContext artifacts
pub struct ArtifactMapper;

impl ArtifactMapper {
    /// Map a flow's FinalOutput to an Artifact
    pub fn map_flow_output(
        work_context_id: String,
        flow_name: String,
        primary_output: Value,
        additional_outputs: std::collections::HashMap<String, Value>,
    ) -> Vec<Artifact> {
        let mut artifacts = Vec::new();

        // Map primary output
        let primary_artifact = Artifact::new(
            uuid::Uuid::new_v4().to_string(),
            work_context_id.clone(),
            Self::infer_artifact_kind(&flow_name, "primary"),
            format!("{} - primary output", flow_name),
            primary_output,
            format!("flow:{}", flow_name),
        );
        artifacts.push(primary_artifact);

        // Map additional outputs
        for (key, value) in additional_outputs {
            let artifact = Artifact::new(
                uuid::Uuid::new_v4().to_string(),
                work_context_id.clone(),
                Self::infer_artifact_kind(&flow_name, &key),
                format!("{} - {}", flow_name, key),
                value,
                format!("flow:{}:{}", flow_name, key),
            );
            artifacts.push(artifact);
        }

        artifacts
    }

    /// Infer artifact kind from flow name and output key
    fn infer_artifact_kind(flow_name: &str, output_key: &str) -> ArtifactKind {
        let flow_lower = flow_name.to_lowercase();
        let key_lower = output_key.to_lowercase();

        // Code-related flows
        if flow_lower.contains("codegen") || flow_lower.contains("code") {
            if key_lower.contains("test") {
                ArtifactKind::Test
            } else if key_lower.contains("doc") {
                ArtifactKind::Document
            } else {
                ArtifactKind::Code
            }
        }
        // Planning flows
        else if flow_lower.contains("plan") {
            ArtifactKind::Plan
        }
        // Research flows
        else if flow_lower.contains("research") {
            ArtifactKind::Research
        }
        // Review flows
        else if flow_lower.contains("review") {
            ArtifactKind::Review
        }
        // Default based on output key
        else {
            match key_lower {
                k if k.contains("code") => ArtifactKind::Code,
                k if k.contains("test") => ArtifactKind::Test,
                k if k.contains("doc") => ArtifactKind::Document,
                k if k.contains("plan") => ArtifactKind::Plan,
                k if k.contains("review") => ArtifactKind::Review,
                k if k.contains("research") => ArtifactKind::Research,
                _ => ArtifactKind::Other,
            }
        }
    }

    /// Create a file-backed artifact from a path
    pub fn create_file_artifact(
        work_context_id: String,
        file_path: String,
        kind: ArtifactKind,
        description: String,
    ) -> Result<Artifact> {
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;

        Ok(Artifact::new(
            uuid::Uuid::new_v4().to_string(),
            work_context_id,
            kind,
            description,
            serde_json::json!({"content": content, "file_path": file_path}),
            format!("file:{}", file_path),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_artifact_kind_code() {
        let kind = ArtifactMapper::infer_artifact_kind("codegen.flow.yaml", "primary");
        assert_eq!(kind, ArtifactKind::Code);
    }

    #[test]
    fn test_infer_artifact_kind_test() {
        let kind = ArtifactMapper::infer_artifact_kind("codegen.flow.yaml", "test_output");
        assert_eq!(kind, ArtifactKind::Test);
    }

    #[test]
    fn test_infer_artifact_kind_plan() {
        let kind = ArtifactMapper::infer_artifact_kind("planning.flow.yaml", "primary");
        assert_eq!(kind, ArtifactKind::Plan);
    }

    #[test]
    fn test_map_flow_output() {
        let mut additional = std::collections::HashMap::new();
        additional.insert("test".to_string(), serde_json::json!("test code"));

        let artifacts = ArtifactMapper::map_flow_output(
            "ctx-1".to_string(),
            "codegen.flow.yaml".to_string(),
            serde_json::json!("fn main() {}"),
            additional,
        );

        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].kind, ArtifactKind::Code);
        assert_eq!(artifacts[1].kind, ArtifactKind::Test);
    }
}
