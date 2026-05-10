//! Execution plan repository operations

use anyhow::Context;
use rusqlite::params;

use super::trait_def::Repository;
use crate::work::{ExecutionPlan, PlanStep};

/// PlanOperations trait for execution plan repository operations
pub trait PlanOperations: Repository {
    fn create_plan(
        &self,
        work_context_id: &str,
        plan: &ExecutionPlan,
    ) -> anyhow::Result<ExecutionPlan>;
    fn get_plan(&self, work_context_id: &str) -> anyhow::Result<Option<ExecutionPlan>>;
    fn update_plan(
        &self,
        work_context_id: &str,
        plan: &ExecutionPlan,
    ) -> anyhow::Result<ExecutionPlan>;
    fn delete_plan(&self, work_context_id: &str) -> anyhow::Result<()>;
}

impl PlanOperations for crate::db::Db {
    fn create_plan(
        &self,
        work_context_id: &str,
        plan: &ExecutionPlan,
    ) -> anyhow::Result<ExecutionPlan> {
        let conn = self.conn();

        let now = chrono::Utc::now().to_rfc3339();

        // Insert the plan
        conn.execute(
            "INSERT INTO execution_plans (work_context_id, steps_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                work_context_id,
                serde_json::to_string(&plan.steps)?,
                now,
                now,
            ],
        )
        .context("Failed to insert execution plan")?;

        Ok(plan.clone())
    }

    fn get_plan(&self, work_context_id: &str) -> anyhow::Result<Option<ExecutionPlan>> {
        let conn = self.conn();

        let mut stmt = conn
            .prepare(
                "SELECT steps_json
             FROM execution_plans
             WHERE work_context_id = ?1",
            )
            .context("Failed to prepare execution plan query")?;

        let mut rows = stmt
            .query_map(params![work_context_id], |row| {
                let steps_json: String = row.get(0)?;
                let steps: Vec<PlanStep> = serde_json::from_str(&steps_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(ExecutionPlan { steps })
            })
            .context("Failed to query execution plan")?;

        match rows.next() {
            Some(result) => Ok(Some(result.context("Failed to parse execution plan")?)),
            None => Ok(None),
        }
    }

    fn update_plan(
        &self,
        work_context_id: &str,
        plan: &ExecutionPlan,
    ) -> anyhow::Result<ExecutionPlan> {
        let conn = self.conn();

        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "UPDATE execution_plans
             SET steps_json = ?1, updated_at = ?2
             WHERE work_context_id = ?3",
            params![serde_json::to_string(&plan.steps)?, now, work_context_id,],
        )
        .context("Failed to update execution plan")?;

        Ok(plan.clone())
    }

    fn delete_plan(&self, work_context_id: &str) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute(
            "DELETE FROM execution_plans WHERE work_context_id = ?1",
            params![work_context_id],
        )
        .context("Failed to delete execution plan")?;

        Ok(())
    }
}
