//! Flow run operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use crate::db::models::FlowRun;
use super::AsDb;

/// Flow run operations trait
pub trait FlowRunOperations {
    fn create_flow_run(&self, conversation_id: &str) -> anyhow::Result<FlowRun>;
    fn update_flow_run_status(&self, id: &str, status: &str) -> anyhow::Result<()>;
}

impl<T: AsDb> FlowRunOperations for T {
    fn create_flow_run(&self, conversation_id: &str) -> anyhow::Result<FlowRun> {
        let conn = self.as_db().conn();
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        
        conn.execute(
            "INSERT INTO flow_runs (id, conversation_id, status, started_at, completed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, conversation_id, "running", &now.to_rfc3339(), None::<String>],
        ).context("Failed to insert flow run")?;

        Ok(FlowRun {
            id,
            conversation_id: conversation_id.to_string(),
            status: "running".to_string(),
            started_at: now,
            completed_at: None,
        })
    }

    fn update_flow_run_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        let completed_at = if status == "completed" || status == "failed" {
            Some(Utc::now().to_rfc3339())
        } else {
            None
        };

        if let Some(at) = completed_at {
            conn.execute(
                "UPDATE flow_runs SET status = ?1, completed_at = ?2 WHERE id = ?3",
                params![status, at, id],
            ).context("Failed to update flow run")?;
        } else {
            conn.execute(
                "UPDATE flow_runs SET status = ?1 WHERE id = ?2",
                params![status, id],
            ).context("Failed to update flow run")?;
        }

        Ok(())
    }
}
