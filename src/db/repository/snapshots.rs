//! Flow snapshot operations for versioning and resume

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;

/// Flow snapshot model
#[derive(Debug, Clone)]
pub struct FlowSnapshotEntry {
    pub id: String,
    pub flow_name: String,
    pub flow_version: String,
    pub source_hash: String,
    pub source_text: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Flow snapshot operations trait
pub trait FlowSnapshotOperations {
    fn create_flow_snapshot(
        &self,
        flow_name: &str,
        flow_version: &str,
        source_hash: &str,
        source_text: &str,
    ) -> anyhow::Result<FlowSnapshotEntry>;

    fn get_flow_snapshot_by_hash(
        &self,
        source_hash: &str,
    ) -> anyhow::Result<Option<FlowSnapshotEntry>>;

    fn get_latest_flow_snapshot(
        &self,
        flow_name: &str,
    ) -> anyhow::Result<Option<FlowSnapshotEntry>>;
}

impl<T: AsDb> FlowSnapshotOperations for T {
    fn create_flow_snapshot(
        &self,
        flow_name: &str,
        flow_version: &str,
        source_hash: &str,
        source_text: &str,
    ) -> anyhow::Result<FlowSnapshotEntry> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        conn.execute(
            "INSERT INTO flow_snapshots (id, flow_name, flow_version, source_hash, source_text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&id, flow_name, flow_version, source_hash, source_text, &now.to_rfc3339()],
        ).context("Failed to insert flow snapshot")?;

        Ok(FlowSnapshotEntry {
            id,
            flow_name: flow_name.to_string(),
            flow_version: flow_version.to_string(),
            source_hash: source_hash.to_string(),
            source_text: source_text.to_string(),
            created_at: now,
        })
    }

    fn get_flow_snapshot_by_hash(
        &self,
        source_hash: &str,
    ) -> anyhow::Result<Option<FlowSnapshotEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, flow_name, flow_version, source_hash, source_text, created_at
             FROM flow_snapshots
             WHERE source_hash = ?1
             ORDER BY created_at DESC
             LIMIT 1"
        ).context("Failed to prepare flow snapshot query")?;

        let result = stmt.query_row(params![source_hash], |row| {
            Ok(FlowSnapshotEntry {
                id: row.get(0)?,
                flow_name: row.get(1)?,
                flow_version: row.get(2)?,
                source_hash: row.get(3)?,
                source_text: row.get(4)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?).unwrap().with_timezone(&chrono::Utc),
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_latest_flow_snapshot(
        &self,
        flow_name: &str,
    ) -> anyhow::Result<Option<FlowSnapshotEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, flow_name, flow_version, source_hash, source_text, created_at
             FROM flow_snapshots
             WHERE flow_name = ?1
             ORDER BY created_at DESC
             LIMIT 1"
        ).context("Failed to prepare latest flow snapshot query")?;

        let result = stmt.query_row(params![flow_name], |row| {
            Ok(FlowSnapshotEntry {
                id: row.get(0)?,
                flow_name: row.get(1)?,
                flow_version: row.get(2)?,
                source_hash: row.get(3)?,
                source_text: row.get(4)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?).unwrap().with_timezone(&chrono::Utc),
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
