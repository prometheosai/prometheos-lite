//! WorkContext repository operations

use anyhow::Context;
use chrono::Utc;
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::AsDb;
use crate::work::persistence::{WorkContextRow, decode_work_context, encode_work_context};
use crate::work::types::WorkContext;

/// Full-row work-context UPDATE on an arbitrary Connection - the
/// tx-capable primitive (single source of truth for the 35-parameter
/// statement). Optimistic concurrency guard (#132 / PR #220): every write
/// is conditioned on the stored row being non-Cancelled, so a stale
/// in-memory snapshot taken before a concurrent cancel cannot resurrect
/// the terminal status. Returns the number of affected rows (0 = the row
/// is Cancelled, or no such id - callers disambiguate).
pub fn update_work_context_on_conn(
    conn: &rusqlite::Connection,
    context: &WorkContext,
) -> anyhow::Result<usize> {
    let row = encode_work_context(context);

    conn.execute(
        "UPDATE work_contexts SET
            title = ?1, domain = ?2, domain_profile_id = ?3, context_type = ?4,
            project_id = ?5, conversation_id = ?6, parent_context_id = ?7, priority = ?8, due_at = ?9,
            goal = ?10, requirements = ?11, constraints = ?12, status = ?13, current_phase = ?14,
            blocked_reason = ?15, plan = ?16, approved_plan = ?17, artifacts = ?18, memory_refs = ?19,
            decisions = ?20, flow_runs = ?21, tool_trace = ?22, execution_metadata = ?23, open_questions = ?24,
            autonomy_level = ?25, approval_policy = ?26, summary = ?27, completion_criteria = ?28,
            last_activity_at = ?29, metadata = ?30, playbook_id = ?31, evaluation_result = ?32, updated_at = ?33
         WHERE id = ?34 AND status <> ?35",
        params![
            row.title,
            row.domain,
            row.domain_profile_id,
            row.context_type,
            row.project_id,
            row.conversation_id,
            row.parent_context_id,
            row.priority,
            row.due_at,
            row.goal,
            row.requirements,
            row.constraints,
            row.status,
            row.current_phase,
            row.blocked_reason,
            row.plan,
            row.approved_plan,
            row.artifacts,
            row.memory_refs,
            row.decisions,
            row.flow_runs,
            row.tool_trace,
            row.execution_metadata,
            row.open_questions,
            row.autonomy_level,
            row.approval_policy,
            row.summary,
            row.completion_criteria,
            row.last_activity_at,
            row.metadata,
            row.playbook_id,
            row.evaluation_result,
            row.updated_at,
            row.id,
            "\"Cancelled\"",
        ],
    )
    .context("Failed to update work context")
}

/// WorkContext operations trait
pub trait WorkContextOperations {
    fn create_work_context(&self, context: &WorkContext) -> anyhow::Result<WorkContext>;
    fn get_work_context(&self, id: &str) -> anyhow::Result<Option<WorkContext>>;
    fn update_work_context(&self, context: &WorkContext) -> anyhow::Result<()>;
    fn list_work_contexts(&self, user_id: &str) -> anyhow::Result<Vec<WorkContext>>;
    fn get_active_context_for_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Option<WorkContext>>;
    fn set_active_context_for_conversation(
        &self,
        conversation_id: &str,
        work_context_id: &str,
    ) -> anyhow::Result<()>;
}

/// Extract the typed row from a SQLite row in the shared column order. Both
/// SELECT statements used here (full list; single-id lookup) use the same
/// column order, so the same projection works for both.
fn row_to_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkContextRow> {
    Ok(WorkContextRow {
        id: row.get(0)?,
        user_id: row.get(1)?,
        title: row.get(2)?,
        domain: row.get(3)?,
        domain_profile_id: row.get(4)?,
        context_type: row.get(5)?,
        project_id: row.get(6)?,
        conversation_id: row.get(7)?,
        parent_context_id: row.get(8)?,
        priority: row.get(9)?,
        due_at: row.get(10)?,
        goal: row.get(11)?,
        requirements: row.get(12)?,
        constraints: row.get(13)?,
        status: row.get(14)?,
        current_phase: row.get(15)?,
        blocked_reason: row.get(16)?,
        plan: row.get(17)?,
        approved_plan: row.get(18)?,
        artifacts: row.get(19)?,
        memory_refs: row.get(20)?,
        decisions: row.get(21)?,
        flow_runs: row.get(22)?,
        tool_trace: row.get(23)?,
        execution_metadata: row.get(24)?,
        open_questions: row.get(25)?,
        autonomy_level: row.get(26)?,
        approval_policy: row.get(27)?,
        summary: row.get(28)?,
        completion_criteria: row.get(29)?,
        last_activity_at: row.get(30)?,
        metadata: row.get(31)?,
        playbook_id: row.get(32)?,
        evaluation_result: row.get(33)?,
        created_at: row.get(34)?,
        updated_at: row.get(35)?,
    })
}

const WORK_CONTEXT_SELECT: &str = "SELECT id, user_id, title, domain, domain_profile_id, context_type,
                    project_id, conversation_id, parent_context_id, priority, due_at,
                    goal, requirements, constraints, status, current_phase, blocked_reason,
                    plan, approved_plan, artifacts, memory_refs, decisions, flow_runs,
                    tool_trace, execution_metadata, open_questions, autonomy_level, approval_policy, summary,
                    completion_criteria, last_activity_at, metadata, playbook_id, evaluation_result, created_at, updated_at
             FROM work_contexts";

impl<T: AsDb> WorkContextOperations for T {
    fn create_work_context(&self, context: &WorkContext) -> anyhow::Result<WorkContext> {
        let conn = self.as_db().conn();
        let row = encode_work_context(context);

        conn.execute(
            "INSERT INTO work_contexts (
                id, user_id, title, domain, domain_profile_id, context_type,
                project_id, conversation_id, parent_context_id, priority, due_at,
                goal, requirements, constraints, status, current_phase, blocked_reason,
                plan, approved_plan, artifacts, memory_refs, decisions, flow_runs,
                tool_trace, open_questions, autonomy_level, approval_policy, summary,
                completion_criteria, last_activity_at, metadata, execution_metadata, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34)",
            params![
                row.id,
                row.user_id,
                row.title,
                row.domain,
                row.domain_profile_id,
                row.context_type,
                row.project_id,
                row.conversation_id,
                row.parent_context_id,
                row.priority,
                row.due_at,
                row.goal,
                row.requirements,
                row.constraints,
                row.status,
                row.current_phase,
                row.blocked_reason,
                row.plan,
                row.approved_plan,
                row.artifacts,
                row.memory_refs,
                row.decisions,
                row.flow_runs,
                row.tool_trace,
                row.open_questions,
                row.autonomy_level,
                row.approval_policy,
                row.summary,
                row.completion_criteria,
                row.last_activity_at,
                row.metadata,
                row.execution_metadata,
                row.created_at,
                row.updated_at,
            ],
        ).context("Failed to insert work context")?;

        Ok(context.clone())
    }

    fn get_work_context(&self, id: &str) -> anyhow::Result<Option<WorkContext>> {
        let conn = self.as_db().conn();
        let sql = format!("{} WHERE id = ?1", WORK_CONTEXT_SELECT);
        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare work context query")?;

        let result = stmt.query_row(params![id], row_to_raw);

        match result {
            Ok(raw) => Ok(Some(decode_work_context(raw)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn update_work_context(&self, context: &WorkContext) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        let affected = update_work_context_on_conn(conn, context)?;

        if affected == 0 {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM work_contexts WHERE id = ?1 LIMIT 1",
                    params![context.id],
                    |_| Ok(true),
                )
                .optional()
                .context("Failed to re-check work context")?
                .unwrap_or(false);
            if !exists {
                anyhow::bail!("work context not found: {}", context.id);
            }
            anyhow::bail!(
                "work context {} is Cancelled; refusing to overwrite terminal state",
                context.id
            );
        }

        Ok(())
    }

    fn list_work_contexts(&self, user_id: &str) -> anyhow::Result<Vec<WorkContext>> {
        let conn = self.as_db().conn();
        let sql = format!(
            "{} WHERE user_id = ?1 ORDER BY created_at DESC",
            WORK_CONTEXT_SELECT
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare work contexts list query")?;

        let raws: Vec<rusqlite::Result<WorkContextRow>> =
            stmt.query_map(params![user_id], row_to_raw)?.collect();

        let mut out = Vec::with_capacity(raws.len());
        for raw in raws {
            out.push(decode_work_context(raw.context("read work_contexts row")?)?);
        }
        Ok(out)
    }

    fn get_active_context_for_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Option<WorkContext>> {
        let conn = self.as_db().conn();
        let sql = "SELECT id, user_id, title, domain, domain_profile_id, context_type,
                    project_id, conversation_id, parent_context_id, priority, due_at,
                    goal, requirements, constraints, status, current_phase, blocked_reason,
                    plan, approved_plan, artifacts, memory_refs, decisions, flow_runs,
                    tool_trace, execution_metadata, open_questions, autonomy_level, approval_policy, summary,
                    completion_criteria, last_activity_at, metadata, playbook_id, evaluation_result, created_at, updated_at
             FROM work_contexts wc
             INNER JOIN conversation_work_contexts cwc ON wc.id = cwc.work_context_id
             WHERE cwc.conversation_id = ?1 AND cwc.is_active = 1";
        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare active context query")?;

        let result = stmt.query_row(params![conversation_id], row_to_raw);

        match result {
            Ok(raw) => Ok(Some(decode_work_context(raw)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set_active_context_for_conversation(
        &self,
        conversation_id: &str,
        work_context_id: &str,
    ) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "UPDATE conversation_work_contexts SET is_active = 0 WHERE conversation_id = ?1",
            params![conversation_id],
        )
        .context("Failed to deactivate existing contexts")?;

        conn.execute(
            "INSERT INTO conversation_work_contexts (conversation_id, work_context_id, is_active, created_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(conversation_id, work_context_id) DO UPDATE SET is_active = 1",
            params![conversation_id, work_context_id, &now],
        ).context("Failed to set active context")?;

        Ok(())
    }
}
