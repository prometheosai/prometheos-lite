//! Trust policy operations for persistent trust management

use anyhow::Context;
use chrono::Utc;
use rusqlite::params;

use super::AsDb;

/// Trust policy entry model
#[derive(Debug, Clone)]
pub struct TrustPolicyEntry {
    pub id: String,
    pub source: String,
    pub trust_level: String,
    pub require_approval: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Trust policy operations trait
pub trait TrustPolicyOperations {
    fn create_or_update_trust_policy(
        &self,
        source: &str,
        trust_level: &str,
        require_approval: bool,
    ) -> anyhow::Result<TrustPolicyEntry>;

    fn get_trust_policy(&self, source: &str) -> anyhow::Result<Option<TrustPolicyEntry>>;

    fn list_trust_policies(&self) -> anyhow::Result<Vec<TrustPolicyEntry>>;

    fn delete_trust_policy(&self, source: &str) -> anyhow::Result<()>;
}

impl<T: AsDb> TrustPolicyOperations for T {
    fn create_or_update_trust_policy(
        &self,
        source: &str,
        trust_level: &str,
        require_approval: bool,
    ) -> anyhow::Result<TrustPolicyEntry> {
        let conn = self.as_db().conn();
        let now = Utc::now();

        // Try to update existing policy first
        let updated = conn.execute(
            "UPDATE trust_policies SET trust_level = ?1, require_approval = ?2, updated_at = ?3 WHERE source = ?4",
            params![trust_level, if require_approval { 1 } else { 0 }, &now.to_rfc3339(), source],
        ).context("Failed to update trust policy")?;

        if updated > 0 {
            // Return updated policy
            let mut stmt = conn
                .prepare(
                    "SELECT id, source, trust_level, require_approval, created_at, updated_at
                 FROM trust_policies WHERE source = ?1",
                )
                .context("Failed to prepare trust policy query")?;

            let entry = stmt
                .query_row(params![source], |row| {
                    Ok(TrustPolicyEntry {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        trust_level: row.get(2)?,
                        require_approval: row.get::<_, i32>(3)? == 1,
                        created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                            .unwrap()
                            .with_timezone(&chrono::Utc),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                            .unwrap()
                            .with_timezone(&chrono::Utc),
                    })
                })
                .context("Failed to query updated trust policy")?;

            return Ok(entry);
        }

        // Create new policy
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO trust_policies (id, source, trust_level, require_approval, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&id, source, trust_level, if require_approval { 1 } else { 0 }, &now.to_rfc3339(), &now.to_rfc3339()],
        ).context("Failed to insert trust policy")?;

        Ok(TrustPolicyEntry {
            id,
            source: source.to_string(),
            trust_level: trust_level.to_string(),
            require_approval,
            created_at: now,
            updated_at: now,
        })
    }

    fn get_trust_policy(&self, source: &str) -> anyhow::Result<Option<TrustPolicyEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn
            .prepare(
                "SELECT id, source, trust_level, require_approval, created_at, updated_at
             FROM trust_policies WHERE source = ?1",
            )
            .context("Failed to prepare trust policy query")?;

        let result = stmt.query_row(params![source], |row| {
            Ok(TrustPolicyEntry {
                id: row.get(0)?,
                source: row.get(1)?,
                trust_level: row.get(2)?,
                require_approval: row.get::<_, i32>(3)? == 1,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn list_trust_policies(&self) -> anyhow::Result<Vec<TrustPolicyEntry>> {
        let conn = self.as_db().conn();

        let mut stmt = conn
            .prepare(
                "SELECT id, source, trust_level, require_approval, created_at, updated_at
             FROM trust_policies ORDER BY source",
            )
            .context("Failed to prepare trust policies query")?;

        let entries = stmt
            .query_map([], |row| {
                Ok(TrustPolicyEntry {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    trust_level: row.get(2)?,
                    require_approval: row.get::<_, i32>(3)? == 1,
                    created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                })
            })
            .context("Failed to query trust policies")?;

        let mut result = Vec::new();
        for entry in entries {
            result.push(entry.context("Failed to parse trust policy entry")?);
        }

        Ok(result)
    }

    fn delete_trust_policy(&self, source: &str) -> anyhow::Result<()> {
        let conn = self.as_db().conn();

        conn.execute(
            "DELETE FROM trust_policies WHERE source = ?1",
            params![source],
        )
        .context("Failed to delete trust policy")?;

        Ok(())
    }
}
