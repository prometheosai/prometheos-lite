//! Playbook evolutions repository operations

use anyhow::{Context, Result};
use rusqlite::params;

use super::AsDb;

/// Evolutions operations trait
pub trait EvolutionsOperations {
    /// Count total number of playbook evolutions
    fn count_evolutions(&self) -> Result<i64>;

    /// Count evolutions for a specific playbook
    fn count_evolutions_by_playbook(&self, playbook_id: &str) -> Result<i64>;
}

impl<T: AsDb> EvolutionsOperations for T {
    fn count_evolutions(&self) -> Result<i64> {
        let conn = self.as_db().conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM playbook_evolutions", [], |row| {
                row.get(0)
            })
            .context("Failed to count evolutions")?;
        Ok(count)
    }

    fn count_evolutions_by_playbook(&self, playbook_id: &str) -> Result<i64> {
        let conn = self.as_db().conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM playbook_evolutions WHERE playbook_id = ?1",
                params![playbook_id],
                |row| row.get(0),
            )
            .context("Failed to count evolutions by playbook")?;
        Ok(count)
    }
}
