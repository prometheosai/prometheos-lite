//! WorkContext repository operations

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::AsDb;
use crate::work::types::WorkContext;

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

impl<T: AsDb> WorkContextOperations for T {
    fn create_work_context(&self, context: &WorkContext) -> anyhow::Result<WorkContext> {
        let conn = self.as_db().conn();

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
                &context.id,
                &context.user_id,
                &context.title,
                serde_json::to_string(&context.domain)?,
                &context.domain_profile_id,
                &context.context_type,
                &context.project_id,
                &context.conversation_id,
                &context.parent_context_id,
                serde_json::to_string(&context.priority)?,
                &context.due_at.map(|d| d.to_rfc3339()),
                &context.goal,
                serde_json::to_string(&context.requirements)?,
                serde_json::to_string(&context.constraints)?,
                serde_json::to_string(&context.status)?,
                serde_json::to_string(&context.current_phase)?,
                &context.blocked_reason,
                serde_json::to_string(&context.plan)?,
                serde_json::to_string(&context.approved_plan)?,
                serde_json::to_string(&context.artifacts)?,
                serde_json::to_string(&context.memory_refs)?,
                serde_json::to_string(&context.decisions)?,
                serde_json::to_string(&context.flow_runs)?,
                serde_json::to_string(&context.tool_trace)?,
                serde_json::to_string(&context.open_questions)?,
                serde_json::to_string(&context.autonomy_level)?,
                serde_json::to_string(&context.approval_policy)?,
                &context.summary,
                serde_json::to_string(&context.completion_criteria)?,
                &context.last_activity_at.to_rfc3339(),
                serde_json::to_string(&context.metadata)?,
                serde_json::to_string(&context.execution_metadata)?,
                &context.created_at.to_rfc3339(),
                &context.updated_at.to_rfc3339(),
            ],
        ).context("Failed to insert work context")?;

        Ok(context.clone())
    }

    fn get_work_context(&self, id: &str) -> anyhow::Result<Option<WorkContext>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, user_id, title, domain, domain_profile_id, context_type,
                    project_id, conversation_id, parent_context_id, priority, due_at,
                    goal, requirements, constraints, status, current_phase, blocked_reason,
                    plan, approved_plan, artifacts, memory_refs, decisions, flow_runs,
                    tool_trace, execution_metadata, open_questions, autonomy_level, approval_policy, summary,
                    completion_criteria, last_activity_at, metadata, playbook_id, evaluation_result, created_at, updated_at
             FROM work_contexts WHERE id = ?1"
        ).context("Failed to prepare work context query")?;

        let result = stmt.query_row(params![id], |row| {
            Ok(WorkContext {
                id: row.get(0)?,
                user_id: row.get(1)?,
                title: row.get(2)?,
                domain: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                domain_profile_id: row.get(4)?,
                context_type: row.get(5)?,
                project_id: row.get(6)?,
                conversation_id: row.get(7)?,
                parent_context_id: row.get(8)?,
                priority: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                due_at: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                goal: row.get(11)?,
                requirements: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
                constraints: serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_default(),
                status: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                current_phase: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
                blocked_reason: row.get(16)?,
                plan: serde_json::from_str(&row.get::<_, String>(17)?).ok(),
                approved_plan: serde_json::from_str(&row.get::<_, String>(18)?).ok(),
                artifacts: serde_json::from_str(&row.get::<_, String>(19)?).unwrap_or_default(),
                memory_refs: serde_json::from_str(&row.get::<_, String>(20)?).unwrap_or_default(),
                decisions: serde_json::from_str(&row.get::<_, String>(21)?).unwrap_or_default(),
                flow_runs: serde_json::from_str(&row.get::<_, String>(22)?).unwrap_or_default(),
                tool_trace: serde_json::from_str(&row.get::<_, String>(23)?).unwrap_or_default(),
                execution_metadata: serde_json::from_str(&row.get::<_, String>(24)?)
                    .unwrap_or_default(),
                open_questions: serde_json::from_str(&row.get::<_, String>(25)?)
                    .unwrap_or_default(),
                autonomy_level: serde_json::from_str(&row.get::<_, String>(26)?)
                    .unwrap_or_default(),
                approval_policy: serde_json::from_str(&row.get::<_, String>(27)?)
                    .unwrap_or_default(),
                summary: row.get(28)?,
                completion_criteria: serde_json::from_str(&row.get::<_, String>(29)?)
                    .unwrap_or_default(),
                last_activity_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(30)?)
                    .unwrap()
                    .with_timezone(&Utc),
                metadata: serde_json::from_str(&row.get::<_, String>(31)?).unwrap_or_default(),
                playbook_id: row.get(32)?,
                evaluation_result: row
                    .get::<_, Option<String>>(33)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(34)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(35)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        });

        match result {
            Ok(context) => Ok(Some(context)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn update_work_context(&self, context: &WorkContext) -> anyhow::Result<()> {
        let conn = self.as_db().conn();

        conn.execute(
            "UPDATE work_contexts SET
                title = ?1, domain = ?2, domain_profile_id = ?3, context_type = ?4,
                project_id = ?5, conversation_id = ?6, parent_context_id = ?7, priority = ?8, due_at = ?9,
                goal = ?10, requirements = ?11, constraints = ?12, status = ?13, current_phase = ?14,
                blocked_reason = ?15, plan = ?16, approved_plan = ?17, artifacts = ?18, memory_refs = ?19,
                decisions = ?20, flow_runs = ?21, tool_trace = ?22, execution_metadata = ?23, open_questions = ?24,
                autonomy_level = ?25, approval_policy = ?26, summary = ?27, completion_criteria = ?28,
                last_activity_at = ?29, metadata = ?30, playbook_id = ?31, evaluation_result = ?32, updated_at = ?33
             WHERE id = ?34",
            params![
                &context.title,
                serde_json::to_string(&context.domain)?,
                &context.domain_profile_id,
                &context.context_type,
                &context.project_id,
                &context.conversation_id,
                &context.parent_context_id,
                serde_json::to_string(&context.priority)?,
                &context.due_at.map(|d| d.to_rfc3339()),
                &context.goal,
                serde_json::to_string(&context.requirements)?,
                serde_json::to_string(&context.constraints)?,
                serde_json::to_string(&context.status)?,
                serde_json::to_string(&context.current_phase)?,
                &context.blocked_reason,
                serde_json::to_string(&context.plan)?,
                serde_json::to_string(&context.approved_plan)?,
                serde_json::to_string(&context.artifacts)?,
                serde_json::to_string(&context.memory_refs)?,
                serde_json::to_string(&context.decisions)?,
                serde_json::to_string(&context.flow_runs)?,
                serde_json::to_string(&context.tool_trace)?,
                serde_json::to_string(&context.execution_metadata)?,
                serde_json::to_string(&context.open_questions)?,
                serde_json::to_string(&context.autonomy_level)?,
                serde_json::to_string(&context.approval_policy)?,
                &context.summary,
                serde_json::to_string(&context.completion_criteria)?,
                &context.last_activity_at.to_rfc3339(),
                serde_json::to_string(&context.metadata)?,
                &context.playbook_id,
                &context.evaluation_result.as_ref().and_then(|v| serde_json::to_string(v).ok()),
                &context.updated_at.to_rfc3339(),
                &context.id,
            ],
        ).context("Failed to update work context")?;

        Ok(())
    }

    fn list_work_contexts(&self, user_id: &str) -> anyhow::Result<Vec<WorkContext>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT id, user_id, title, domain, domain_profile_id, context_type,
                    project_id, conversation_id, parent_context_id, priority, due_at,
                    goal, requirements, constraints, status, current_phase, blocked_reason,
                    plan, approved_plan, artifacts, memory_refs, decisions, flow_runs,
                    tool_trace, execution_metadata, open_questions, autonomy_level, approval_policy, summary,
                    completion_criteria, last_activity_at, metadata, playbook_id, evaluation_result, created_at, updated_at
             FROM work_contexts WHERE user_id = ?1 ORDER BY created_at DESC"
        ).context("Failed to prepare work contexts list query")?;

        let contexts = stmt
            .query_map(params![user_id], |row| {
                Ok(WorkContext {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    title: row.get(2)?,
                    domain: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                    domain_profile_id: row.get(4)?,
                    context_type: row.get(5)?,
                    project_id: row.get(6)?,
                    conversation_id: row.get(7)?,
                    parent_context_id: row.get(8)?,
                    priority: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    due_at: row
                        .get::<_, Option<String>>(10)?
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    goal: row.get(11)?,
                    requirements: serde_json::from_str(&row.get::<_, String>(12)?)
                        .unwrap_or_default(),
                    constraints: serde_json::from_str(&row.get::<_, String>(13)?)
                        .unwrap_or_default(),
                    status: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                    current_phase: serde_json::from_str(&row.get::<_, String>(15)?)
                        .unwrap_or_default(),
                    blocked_reason: row.get(16)?,
                    plan: serde_json::from_str(&row.get::<_, String>(17)?).ok(),
                    approved_plan: serde_json::from_str(&row.get::<_, String>(18)?).ok(),
                    artifacts: serde_json::from_str(&row.get::<_, String>(19)?).unwrap_or_default(),
                    memory_refs: serde_json::from_str(&row.get::<_, String>(20)?)
                        .unwrap_or_default(),
                    decisions: serde_json::from_str(&row.get::<_, String>(21)?).unwrap_or_default(),
                    flow_runs: serde_json::from_str(&row.get::<_, String>(22)?).unwrap_or_default(),
                    tool_trace: serde_json::from_str(&row.get::<_, String>(23)?)
                        .unwrap_or_default(),
                    execution_metadata: serde_json::from_str(&row.get::<_, String>(24)?)
                        .unwrap_or_default(),
                    open_questions: serde_json::from_str(&row.get::<_, String>(25)?)
                        .unwrap_or_default(),
                    autonomy_level: serde_json::from_str(&row.get::<_, String>(26)?)
                        .unwrap_or_default(),
                    approval_policy: serde_json::from_str(&row.get::<_, String>(27)?)
                        .unwrap_or_default(),
                    summary: row.get(28)?,
                    completion_criteria: serde_json::from_str(&row.get::<_, String>(29)?)
                        .unwrap_or_default(),
                    last_activity_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(30)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    metadata: serde_json::from_str(&row.get::<_, String>(31)?).unwrap_or_default(),
                    playbook_id: row.get(32)?,
                    evaluation_result: row
                        .get::<_, Option<String>>(33)?
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(34)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(35)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })
            .context("Failed to query work contexts")?;

        let mut result = Vec::new();
        for context in contexts {
            result.push(context.context("Failed to parse work context")?);
        }

        Ok(result)
    }

    fn get_active_context_for_conversation(
        &self,
        conversation_id: &str,
    ) -> anyhow::Result<Option<WorkContext>> {
        let conn = self.as_db().conn();

        let mut stmt = conn.prepare(
            "SELECT wc.id, wc.user_id, wc.title, wc.domain, wc.domain_profile_id, wc.context_type,
                    wc.project_id, wc.conversation_id, wc.parent_context_id, wc.priority, wc.due_at,
                    wc.goal, wc.requirements, wc.constraints, wc.status, wc.current_phase, wc.blocked_reason,
                    wc.plan, wc.approved_plan, wc.artifacts, wc.memory_refs, wc.decisions, wc.flow_runs,
                    wc.tool_trace, wc.execution_metadata, wc.open_questions, wc.autonomy_level, wc.approval_policy, wc.summary,
                    wc.completion_criteria, wc.last_activity_at, wc.metadata, wc.playbook_id, wc.evaluation_result, wc.created_at, wc.updated_at
             FROM work_contexts wc
             INNER JOIN conversation_work_contexts cwc ON wc.id = cwc.work_context_id
             WHERE cwc.conversation_id = ?1 AND cwc.is_active = 1"
        ).context("Failed to prepare active context query")?;

        let result = stmt.query_row(params![conversation_id], |row| {
            Ok(WorkContext {
                id: row.get(0)?,
                user_id: row.get(1)?,
                title: row.get(2)?,
                domain: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                domain_profile_id: row.get(4)?,
                context_type: row.get(5)?,
                project_id: row.get(6)?,
                conversation_id: row.get(7)?,
                parent_context_id: row.get(8)?,
                priority: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                due_at: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                goal: row.get(11)?,
                requirements: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
                constraints: serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_default(),
                status: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                current_phase: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
                blocked_reason: row.get(16)?,
                plan: serde_json::from_str(&row.get::<_, String>(17)?).ok(),
                approved_plan: serde_json::from_str(&row.get::<_, String>(18)?).ok(),
                artifacts: serde_json::from_str(&row.get::<_, String>(19)?).unwrap_or_default(),
                memory_refs: serde_json::from_str(&row.get::<_, String>(20)?).unwrap_or_default(),
                decisions: serde_json::from_str(&row.get::<_, String>(21)?).unwrap_or_default(),
                flow_runs: serde_json::from_str(&row.get::<_, String>(22)?).unwrap_or_default(),
                tool_trace: serde_json::from_str(&row.get::<_, String>(23)?).unwrap_or_default(),
                execution_metadata: serde_json::from_str(&row.get::<_, String>(24)?)
                    .unwrap_or_default(),
                open_questions: serde_json::from_str(&row.get::<_, String>(25)?)
                    .unwrap_or_default(),
                autonomy_level: serde_json::from_str(&row.get::<_, String>(26)?)
                    .unwrap_or_default(),
                approval_policy: serde_json::from_str(&row.get::<_, String>(27)?)
                    .unwrap_or_default(),
                summary: row.get(28)?,
                completion_criteria: serde_json::from_str(&row.get::<_, String>(29)?)
                    .unwrap_or_default(),
                last_activity_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(30)?)
                    .unwrap()
                    .with_timezone(&Utc),
                metadata: serde_json::from_str(&row.get::<_, String>(31)?).unwrap_or_default(),
                playbook_id: row.get(32)?,
                evaluation_result: row
                    .get::<_, Option<String>>(33)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(34)?)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(35)?)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        });

        match result {
            Ok(context) => Ok(Some(context)),
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

        // First, deactivate all existing contexts for this conversation
        conn.execute(
            "UPDATE conversation_work_contexts SET is_active = 0 WHERE conversation_id = ?1",
            params![conversation_id],
        )
        .context("Failed to deactivate existing contexts")?;

        // Then, insert or update the active context
        conn.execute(
            "INSERT INTO conversation_work_contexts (conversation_id, work_context_id, is_active, created_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(conversation_id, work_context_id) DO UPDATE SET is_active = 1",
            params![conversation_id, work_context_id, &now],
        ).context("Failed to set active context")?;

        Ok(())
    }
}
