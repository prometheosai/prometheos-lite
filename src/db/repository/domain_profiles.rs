//! Domain profile repository operations

use anyhow::Context;
use rusqlite::params;

use super::trait_def::Repository;
use crate::work::WorkDomainProfile;

/// DomainProfileOperations trait for domain profile repository operations
pub trait DomainProfileOperations: Repository {
    fn create_domain_profile(&self, profile: &WorkDomainProfile) -> anyhow::Result<WorkDomainProfile>;
    fn get_domain_profile(&self, id: &str) -> anyhow::Result<Option<WorkDomainProfile>>;
    fn list_domain_profiles(&self) -> anyhow::Result<Vec<WorkDomainProfile>>;
    fn update_domain_profile(&self, profile: &WorkDomainProfile) -> anyhow::Result<WorkDomainProfile>;
    fn delete_domain_profile(&self, id: &str) -> anyhow::Result<()>;
}

impl DomainProfileOperations for crate::db::Db {
    fn create_domain_profile(&self, profile: &WorkDomainProfile) -> anyhow::Result<WorkDomainProfile> {
        let conn = self.conn();

        conn.execute(
            "INSERT INTO work_domain_profiles (id, name, parent_domain, default_flows, artifact_kinds, approval_defaults, lifecycle_template_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &profile.id,
                &profile.name,
                &profile.parent_domain,
                serde_json::to_string(&profile.default_flows)?,
                serde_json::to_string(&profile.artifact_kinds)?,
                serde_json::to_string(&profile.approval_defaults)?,
                serde_json::to_string(&profile.lifecycle_template)?,
            ],
        )
        .context("Failed to insert domain profile")?;

        Ok(profile.clone())
    }

    fn get_domain_profile(&self, id: &str) -> anyhow::Result<Option<WorkDomainProfile>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, name, parent_domain, default_flows, artifact_kinds, approval_defaults, lifecycle_template_json
             FROM work_domain_profiles
             WHERE id = ?1",
        )
        .context("Failed to prepare domain profile query")?;

        let mut rows = stmt.query_map(params![id], |row| {
            let default_flows_json: String = row.get(3)?;
            let default_flows: Vec<String> = serde_json::from_str(&default_flows_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let artifact_kinds_json: String = row.get(4)?;
            let artifact_kinds: Vec<String> = serde_json::from_str(&artifact_kinds_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let approval_defaults_json: String = row.get(5)?;
            let approval_defaults: crate::work::types::ApprovalPolicy = serde_json::from_str(&approval_defaults_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let lifecycle_template_json: String = row.get(6)?;
            let lifecycle_template: crate::work::domain::LifecycleTemplate = serde_json::from_str(&lifecycle_template_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            Ok(WorkDomainProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_domain: row.get(2)?,
                default_flows,
                artifact_kinds,
                approval_defaults,
                lifecycle_template,
            })
        })
        .context("Failed to query domain profile")?;

        match rows.next() {
            Some(result) => Ok(Some(result.context("Failed to parse domain profile")?)),
            None => Ok(None),
        }
    }

    fn list_domain_profiles(&self) -> anyhow::Result<Vec<WorkDomainProfile>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, name, parent_domain, default_flows, artifact_kinds, approval_defaults, lifecycle_template_json
             FROM work_domain_profiles
             ORDER BY name ASC",
        )
        .context("Failed to prepare domain profiles query")?;

        let profiles = stmt
            .query_map([], |row| {
                let default_flows_json: String = row.get(3)?;
                let default_flows: Vec<String> = serde_json::from_str(&default_flows_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let artifact_kinds_json: String = row.get(4)?;
                let artifact_kinds: Vec<String> = serde_json::from_str(&artifact_kinds_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let approval_defaults_json: String = row.get(5)?;
                let approval_defaults: crate::work::types::ApprovalPolicy = serde_json::from_str(&approval_defaults_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let lifecycle_template_json: String = row.get(6)?;
                let lifecycle_template: crate::work::domain::LifecycleTemplate = serde_json::from_str(&lifecycle_template_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(WorkDomainProfile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_domain: row.get(2)?,
                    default_flows,
                    artifact_kinds,
                    approval_defaults,
                    lifecycle_template,
                })
            })
            .context("Failed to query domain profiles")?;

        let mut result = Vec::new();
        for profile in profiles {
            result.push(profile.context("Failed to parse domain profile")?);
        }

        Ok(result)
    }

    fn update_domain_profile(&self, profile: &WorkDomainProfile) -> anyhow::Result<WorkDomainProfile> {
        let conn = self.conn();

        conn.execute(
            "UPDATE work_domain_profiles
             SET name = ?1, parent_domain = ?2, default_flows = ?3, artifact_kinds = ?4, approval_defaults = ?5, lifecycle_template_json = ?6
             WHERE id = ?7",
            params![
                &profile.name,
                &profile.parent_domain,
                serde_json::to_string(&profile.default_flows)?,
                serde_json::to_string(&profile.artifact_kinds)?,
                serde_json::to_string(&profile.approval_defaults)?,
                serde_json::to_string(&profile.lifecycle_template)?,
                &profile.id,
            ],
        )
        .context("Failed to update domain profile")?;

        Ok(profile.clone())
    }

    fn delete_domain_profile(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn();

        conn.execute("DELETE FROM work_domain_profiles WHERE id = ?1", params![id])
            .context("Failed to delete domain profile")?;

        Ok(())
    }
}
