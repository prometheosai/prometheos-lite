//! Playbook repository operations

use anyhow::Context;
use rusqlite::params;

use super::trait_def::Repository;
use crate::work::WorkContextPlaybook;

/// PlaybookOperations trait for playbook repository operations
pub trait PlaybookOperations: Repository {
    fn create_playbook(&self, playbook: &WorkContextPlaybook) -> anyhow::Result<WorkContextPlaybook>;
    fn get_playbook(&self, id: &str) -> anyhow::Result<Option<WorkContextPlaybook>>;
    fn get_playbooks_for_user(&self, user_id: &str) -> anyhow::Result<Vec<WorkContextPlaybook>>;
    fn update_playbook(&self, playbook: &WorkContextPlaybook) -> anyhow::Result<WorkContextPlaybook>;
    fn delete_playbook(&self, id: &str) -> anyhow::Result<()>;
    fn increment_usage_count(&self, id: &str) -> anyhow::Result<()>;
    fn update_confidence(&self, id: &str, confidence: f32) -> anyhow::Result<()>;
}

impl PlaybookOperations for crate::db::Db {
    fn create_playbook(&self, playbook: &WorkContextPlaybook) -> anyhow::Result<WorkContextPlaybook> {
        let conn = self.conn();

        conn.execute(
            "INSERT INTO work_context_playbooks (id, user_id, domain_profile_id, name, description, preferred_flows, default_approval_policy, default_research_depth, default_creativity_level, evaluation_rules, confidence, usage_count, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                &playbook.id,
                &playbook.user_id,
                &playbook.domain_profile_id,
                &playbook.name,
                &playbook.description,
                serde_json::to_string(&playbook.preferred_flows)?,
                serde_json::to_string(&playbook.default_approval_policy)?,
                serde_json::to_string(&playbook.default_research_depth)?,
                serde_json::to_string(&playbook.default_creativity_level)?,
                serde_json::to_string(&playbook.evaluation_rules)?,
                playbook.confidence,
                playbook.usage_count,
                &playbook.updated_at.to_rfc3339(),
            ],
        )
        .context("Failed to insert playbook")?;

        Ok(playbook.clone())
    }

    fn get_playbook(&self, id: &str) -> anyhow::Result<Option<WorkContextPlaybook>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, user_id, domain_profile_id, name, description, preferred_flows, default_approval_policy, default_research_depth, default_creativity_level, evaluation_rules, confidence, usage_count, updated_at
             FROM work_context_playbooks
             WHERE id = ?1",
        )
        .context("Failed to prepare playbook query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            let preferred_flows_json: String = row.get(5)?;
            let preferred_flows: Vec<String> = serde_json::from_str(&preferred_flows_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let default_approval_policy_json: String = row.get(6)?;
            let default_approval_policy: crate::work::types::ApprovalPolicy = serde_json::from_str(&default_approval_policy_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let default_research_depth_json: String = row.get(7)?;
            let default_research_depth: crate::work::playbook::ResearchDepth = serde_json::from_str(&default_research_depth_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let default_creativity_level_json: String = row.get(8)?;
            let default_creativity_level: crate::work::playbook::CreativityLevel = serde_json::from_str(&default_creativity_level_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let evaluation_rules_json: String = row.get(9)?;
            let evaluation_rules: Vec<String> = serde_json::from_str(&evaluation_rules_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let updated_at_str: String = row.get(12)?;
            let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                .with_timezone(&chrono::Utc);

            Ok(WorkContextPlaybook {
                id: row.get(0)?,
                user_id: row.get(1)?,
                domain_profile_id: row.get(2)?,
                name: row.get(3)?,
                description: row.get(4)?,
                preferred_flows,
                default_approval_policy,
                default_research_depth,
                default_creativity_level,
                evaluation_rules,
                confidence: row.get(10)?,
                usage_count: row.get(11)?,
                updated_at,
            })
        })
        .context("Failed to query playbook")?;

        match rows.next() {
            Some(result) => Ok(Some(result.context("Failed to parse playbook")?)),
            None => Ok(None),
        }
    }

    fn get_playbooks_for_user(&self, user_id: &str) -> anyhow::Result<Vec<WorkContextPlaybook>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, user_id, domain_profile_id, name, description, preferred_flows, default_approval_policy, default_research_depth, default_creativity_level, evaluation_rules, confidence, usage_count, updated_at
             FROM work_context_playbooks
             WHERE user_id = ?1
             ORDER BY updated_at DESC",
        )
        .context("Failed to prepare playbooks query")?;

        let playbooks = stmt
            .query_map(params![user_id], |row| {
                let preferred_flows_json: String = row.get(5)?;
                let preferred_flows: Vec<String> = serde_json::from_str(&preferred_flows_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let default_approval_policy_json: String = row.get(6)?;
                let default_approval_policy: crate::work::types::ApprovalPolicy = serde_json::from_str(&default_approval_policy_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let default_research_depth_json: String = row.get(7)?;
                let default_research_depth: crate::work::playbook::ResearchDepth = serde_json::from_str(&default_research_depth_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let default_creativity_level_json: String = row.get(8)?;
                let default_creativity_level: crate::work::playbook::CreativityLevel = serde_json::from_str(&default_creativity_level_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let evaluation_rules_json: String = row.get(9)?;
                let evaluation_rules: Vec<String> = serde_json::from_str(&evaluation_rules_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let updated_at_str: String = row.get(12)?;
                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    .with_timezone(&chrono::Utc);

                Ok(WorkContextPlaybook {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    domain_profile_id: row.get(2)?,
                    name: row.get(3)?,
                    description: row.get(4)?,
                    preferred_flows,
                    default_approval_policy,
                    default_research_depth,
                    default_creativity_level,
                    evaluation_rules,
                    confidence: row.get(10)?,
                    usage_count: row.get(11)?,
                    updated_at,
                })
            })
            .context("Failed to query playbooks")?;

        let mut result = Vec::new();
        for playbook in playbooks {
            result.push(playbook.context("Failed to parse playbook")?);
        }

        Ok(result)
    }

    fn update_playbook(&self, playbook: &WorkContextPlaybook) -> anyhow::Result<WorkContextPlaybook> {
        let conn = self.conn();

        conn.execute(
            "UPDATE work_context_playbooks
             SET name = ?1, description = ?2, preferred_flows = ?3, default_approval_policy = ?4, default_research_depth = ?5, default_creativity_level = ?6, evaluation_rules = ?7, confidence = ?8, usage_count = ?9, updated_at = ?10
             WHERE id = ?11",
            params![
                &playbook.name,
                &playbook.description,
                serde_json::to_string(&playbook.preferred_flows)?,
                serde_json::to_string(&playbook.default_approval_policy)?,
                serde_json::to_string(&playbook.default_research_depth)?,
                serde_json::to_string(&playbook.default_creativity_level)?,
                serde_json::to_string(&playbook.evaluation_rules)?,
                playbook.confidence,
                playbook.usage_count,
                &playbook.updated_at.to_rfc3339(),
                &playbook.id,
            ],
        )
        .context("Failed to update playbook")?;

        Ok(playbook.clone())
    }

    fn delete_playbook(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute("DELETE FROM work_context_playbooks WHERE id = ?1", params![id])
            .context("Failed to delete playbook")?;

        Ok(())
    }

    fn increment_usage_count(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute(
            "UPDATE work_context_playbooks SET usage_count = usage_count + 1, updated_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), id],
        )
        .context("Failed to increment playbook usage count")?;

        Ok(())
    }

    fn update_confidence(&self, id: &str, confidence: f32) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute(
            "UPDATE work_context_playbooks SET confidence = ?1, updated_at = ?2 WHERE id = ?3",
            params![confidence.clamp(0.0, 1.0), chrono::Utc::now().to_rfc3339(), id],
        )
        .context("Failed to update playbook confidence")?;

        Ok(())
    }
}
