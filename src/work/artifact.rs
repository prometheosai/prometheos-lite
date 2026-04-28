//! Artifact system for WorkContext

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Artifact - a produced output from work execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub work_context_id: String,
    pub kind: ArtifactKind,
    pub name: String,
    pub content: serde_json::Value,
    pub storage: ArtifactStorage,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

impl Artifact {
    /// Create a new artifact
    pub fn new(
        id: String,
        work_context_id: String,
        kind: ArtifactKind,
        name: String,
        content: serde_json::Value,
        created_by: String,
    ) -> Self {
        Self {
            id,
            work_context_id,
            kind,
            name,
            content,
            storage: ArtifactStorage::Inline,
            created_by,
            created_at: Utc::now(),
        }
    }

    /// Create a file-backed artifact
    pub fn new_file_backed(
        id: String,
        work_context_id: String,
        kind: ArtifactKind,
        name: String,
        file_path: String,
        created_by: String,
    ) -> Self {
        Self {
            id,
            work_context_id,
            kind,
            name,
            content: serde_json::json!({ "file_path": file_path }),
            storage: ArtifactStorage::FilePath(file_path),
            created_by,
            created_at: Utc::now(),
        }
    }
}

/// ArtifactKind - the type of artifact
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtifactKind {
    Plan,
    Code,
    Test,
    Document,
    Analysis,
    Research,
    Review,
    MarketingCopy,
    TaskList,
    EmailDraft,
    Report,
    Other,
}

/// ArtifactStorage - how the artifact content is stored
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArtifactStorage {
    /// Content stored inline in JSON
    Inline,
    /// Content stored as a file at the given path
    FilePath(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artifact_creation() {
        let artifact = Artifact::new(
            "art-1".to_string(),
            "ctx-1".to_string(),
            ArtifactKind::Code,
            "main.rs".to_string(),
            serde_json::json!("fn main() {}"),
            "system".to_string(),
        );

        assert_eq!(artifact.id, "art-1");
        assert_eq!(artifact.kind, ArtifactKind::Code);
        assert!(matches!(artifact.storage, ArtifactStorage::Inline));
    }

    #[test]
    fn test_artifact_file_backed() {
        let artifact = Artifact::new_file_backed(
            "art-1".to_string(),
            "ctx-1".to_string(),
            ArtifactKind::Document,
            "report.pdf".to_string(),
            "output/report.pdf".to_string(),
            "system".to_string(),
        );

        assert_eq!(artifact.id, "art-1");
        assert!(matches!(
            artifact.storage,
            ArtifactStorage::FilePath(_)
        ));
    }
}
