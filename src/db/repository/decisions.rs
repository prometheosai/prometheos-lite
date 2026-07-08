//! Decision repository operations

use anyhow::Context;
use rusqlite::params;

use super::trait_def::Repository;
use crate::work::DecisionRecord;

/// DecisionOperations trait for decision repository operations
pub trait DecisionOperations: Repository {
    fn create_decision(&self, decision: &DecisionRecord) -> anyhow::Result<DecisionRecord>;
    fn get_decision(&self, id: &str) -> anyhow::Result<Option<DecisionRecord>>;
    fn get_decisions_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<DecisionRecord>>;
    fn update_decision(&self, decision: &DecisionRecord) -> anyhow::Result<DecisionRecord>;
    fn delete_decision(&self, id: &str) -> anyhow::Result<()>;
}

impl DecisionOperations for crate::db::Db {
    fn create_decision(&self, decision: &DecisionRecord) -> anyhow::Result<DecisionRecord> {
        let conn = self.conn();

        conn.execute(
            "INSERT INTO decisions (id, work_context_id, description, chosen_option, alternatives, approved, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
            params![
                &decision.id,
                &decision.description,
                &decision.chosen_option,
                serde_json::to_string(&decision.alternatives)?,
                decision.approved,
                &decision.created_at.to_rfc3339(),
            ],
        )
        .context("Failed to insert decision")?;

        Ok(decision.clone())
    }

    fn get_decision(&self, id: &str) -> anyhow::Result<Option<DecisionRecord>> {
        let conn = self.conn();

        let mut stmt = conn
            .prepare(
                "SELECT id, description, chosen_option, alternatives, approved, created_at
             FROM decisions
             WHERE id = ?1",
            )
            .context("Failed to prepare decision query")?;

        let mut rows = stmt
            .query_map(params![id], |row| {
                let alternatives_json: String = row.get(3)?;
                let alternatives: Vec<String> = serde_json::from_str(&alternatives_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let created_at_str: String = row.get(5)?;
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    .with_timezone(&chrono::Utc);

                Ok(DecisionRecord {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    chosen_option: row.get(2)?,
                    alternatives,
                    approved: row.get(4)?,
                    created_at,
                })
            })
            .context("Failed to query decision")?;

        match rows.next() {
            Some(result) => Ok(Some(result.context("Failed to parse decision")?)),
            None => Ok(None),
        }
    }

    fn get_decisions_for_context(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<DecisionRecord>> {
        let conn = self.conn();

        let mut stmt = conn
            .prepare(
                "SELECT id, description, chosen_option, alternatives, approved, created_at
             FROM decisions
             WHERE work_context_id = ?1
             ORDER BY created_at ASC",
            )
            .context("Failed to prepare decisions query")?;

        let decisions = stmt
            .query_map(params![work_context_id], |row| {
                let alternatives_json: String = row.get(3)?;
                let alternatives: Vec<String> = serde_json::from_str(&alternatives_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let created_at_str: String = row.get(5)?;
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    .with_timezone(&chrono::Utc);

                Ok(DecisionRecord {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    chosen_option: row.get(2)?,
                    alternatives,
                    approved: row.get(4)?,
                    created_at,
                })
            })
            .context("Failed to query decisions")?;

        let mut result = Vec::new();
        for decision in decisions {
            result.push(decision.context("Failed to parse decision")?);
        }

        Ok(result)
    }

    fn update_decision(&self, decision: &DecisionRecord) -> anyhow::Result<DecisionRecord> {
        let conn = self.conn();

        conn.execute(
            "UPDATE decisions
             SET description = ?1, chosen_option = ?2, alternatives = ?3, approved = ?4
             WHERE id = ?5",
            params![
                &decision.description,
                &decision.chosen_option,
                serde_json::to_string(&decision.alternatives)?,
                decision.approved,
                &decision.id,
            ],
        )
        .context("Failed to update decision")?;

        Ok(decision.clone())
    }

    fn delete_decision(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute("DELETE FROM decisions WHERE id = ?1", params![id])
            .context("Failed to delete decision")?;

        Ok(())
    }
}
