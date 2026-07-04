//! Project operations

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

use crate::db::models::{CreateProject, Project};

/// Project operations trait
pub trait ProjectOperations {
    fn create_project(&self, input: CreateProject) -> anyhow::Result<Project>;
    fn get_projects(&self) -> anyhow::Result<Vec<Project>>;
    fn get_project(&self, id: &str) -> anyhow::Result<Option<Project>>;
}

impl<T: AsDb> ProjectOperations for T {
    fn create_project(&self, input: CreateProject) -> anyhow::Result<Project> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        conn.execute(
            "INSERT INTO projects (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![&id, &input.name, &now.to_rfc3339(), &now.to_rfc3339()],
        )
        .context("Failed to insert project")?;

        Ok(Project {
            id,
            name: input.name,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_projects(&self) -> anyhow::Result<Vec<Project>> {
        let conn = self.as_db().conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, created_at, updated_at FROM projects ORDER BY created_at DESC",
            )
            .context("Failed to prepare projects query")?;

        let projects = stmt
            .query_map([], |row| {
                let created_str: String = row.get(2)?;
                let updated_str: String = row.get(3)?;
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&updated_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .context("Failed to query projects")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect projects")?;

        Ok(projects)
    }

    fn get_project(&self, id: &str) -> anyhow::Result<Option<Project>> {
        let conn = self.as_db().conn();
        let mut stmt = conn
            .prepare("SELECT id, name, created_at, updated_at FROM projects WHERE id = ?1")
            .context("Failed to prepare project query")?;

        let project = stmt
            .query_row(params![id], |row| {
                let created_str: String = row.get(2)?;
                let updated_str: String = row.get(3)?;
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: DateTime::parse_from_rfc3339(&created_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&updated_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .optional()
            .context("Failed to query project")?;

        Ok(project)
    }
}

/// Helper trait to get Db reference
pub trait AsDb {
    fn as_db(&self) -> &super::db::Db;
}

impl AsDb for super::db::Db {
    fn as_db(&self) -> &super::db::Db {
        self
    }
}
