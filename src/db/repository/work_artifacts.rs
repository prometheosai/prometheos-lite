//! WorkContext artifact repository operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;
use crate::work::artifact::{Artifact, ArtifactKind, ArtifactStorage};

/// WorkContext artifact operations trait
pub trait WorkArtifactOperations {
    fn create_artifact(&self, artifact: &Artifact) -> anyhow::Result<Artifact>;
    fn get_artifacts_for_context(&self, work_context_id: &str) -> anyhow::Result<Vec<Artifact>>;
    fn get_artifact(&self, id: &str) -> anyhow::Result<Option<Artifact>>;
}

impl<T: AsDb> WorkArtifactOperations for T {
    fn create_artifact(&self, artifact: &Artifact) -> anyhow::Result<Artifact> {
        let conn = self.as_db().conn();

        let storage_type = match &artifact.storage {
            ArtifactStorage::Inline => "inline".to_string(),
            ArtifactStorage::FilePath(path) => format!("file:{}", path),
        };

        let file_path = match &artifact.storage {
            ArtifactStorage::FilePath(path) => Some(path.as_str()),
            _ => None,
        };

        conn.execute(
            "INSERT INTO work_artifacts (id, work_context_id, kind, name, content, created_by, storage_type, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &artifact.id,
                &artifact.work_context_id,
                serde_json::to_string(&artifact.kind)?,
                &artifact.name,
                serde_json::to_string(&artifact.content)?,
                &artifact.created_by,
                storage_type,
                file_path,
                &artifact.created_at.to_rfc3339(),
            ],
        )
        .context("Failed to insert artifact")?;

        Ok(artifact.clone())
    }

    fn get_artifacts_for_context(&self, work_context_id: &str) -> anyhow::Result<Vec<Artifact>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, work_context_id, kind, name, content, created_by, storage_type, file_path, created_at
             FROM work_artifacts
             WHERE work_context_id = ?1
             ORDER BY created_at ASC",
        )
        .context("Failed to prepare artifacts query")?;

        let artifacts = stmt.query_map(params![work_context_id], |row| {
            let storage_type: String = row.get(6)?;
            let storage = if storage_type.starts_with("file:") {
                let path = storage_type.strip_prefix("file:").unwrap_or("");
                ArtifactStorage::FilePath(path.to_string())
            } else {
                ArtifactStorage::Inline
            };

            Ok(Artifact {
                id: row.get(0)?,
                work_context_id: row.get(1)?,
                kind: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(ArtifactKind::Other),
                name: row.get(3)?,
                content: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(serde_json::Value::Null),
                created_by: row.get(5)?,
                storage,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })
        .context("Failed to query artifacts")?;

        let mut result = Vec::new();
        for artifact in artifacts {
            result.push(artifact.context("Failed to parse artifact")?);
        }

        Ok(result)
    }

    fn get_artifact(&self, id: &str) -> anyhow::Result<Option<Artifact>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, work_context_id, kind, name, content, created_by, storage_type, file_path, created_at
             FROM work_artifacts
             WHERE id = ?1",
        )
        .context("Failed to prepare artifact query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            let storage_type: String = row.get(6)?;
            let storage = if storage_type.starts_with("file:") {
                let path = storage_type.strip_prefix("file:").unwrap_or("");
                ArtifactStorage::FilePath(path.to_string())
            } else {
                ArtifactStorage::Inline
            };

            Ok(Artifact {
                id: row.get(0)?,
                work_context_id: row.get(1)?,
                kind: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(ArtifactKind::Other),
                name: row.get(3)?,
                content: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(serde_json::Value::Null),
                created_by: row.get(5)?,
                storage,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })
        .context("Failed to query artifact")?;

        match rows.next() {
            Some(Ok(artifact)) => Ok(Some(artifact)),
            Some(Err(e)) => Err(anyhow::anyhow!("Failed to parse artifact: {}", e)),
            None => Ok(None),
        }
    }
}
