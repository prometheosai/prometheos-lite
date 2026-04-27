//! Artifact operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;
use crate::db::models::Artifact;

/// Artifact operations trait
pub trait ArtifactOperations {
    fn create_artifact(
        &self,
        run_id: &str,
        file_path: &str,
        content: &str,
    ) -> anyhow::Result<Artifact>;
}

impl<T: AsDb> ArtifactOperations for T {
    fn create_artifact(
        &self,
        run_id: &str,
        file_path: &str,
        content: &str,
    ) -> anyhow::Result<Artifact> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        conn.execute(
            "INSERT INTO artifacts (id, run_id, file_path, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, run_id, file_path, content, &now.to_rfc3339()],
        ).context("Failed to insert artifact")?;

        Ok(Artifact {
            id,
            run_id: run_id.to_string(),
            file_path: file_path.to_string(),
            content: content.to_string(),
            created_at: now,
        })
    }
}
