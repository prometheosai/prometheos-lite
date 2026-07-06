//! Skills repository operations

use anyhow::{Context, Result};
use rusqlite::params;

use super::AsDb;

/// Skills operations trait
pub trait SkillsOperations {
    /// Count total number of skills
    fn count_skills(&self) -> Result<i64>;

    /// Count skills by tag
    fn count_skills_by_tag(&self, tag: &str) -> Result<i64>;
}

impl<T: AsDb> SkillsOperations for T {
    fn count_skills(&self) -> Result<i64> {
        let conn = self.as_db().conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM skills", [], |row| row.get(0))
            .context("Failed to count skills")?;
        Ok(count)
    }

    fn count_skills_by_tag(&self, tag: &str) -> Result<i64> {
        let conn = self.as_db().conn();
        let pattern = format!("%{}%", tag);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skills WHERE tags LIKE ?1",
                params![pattern],
                |row| row.get(0),
            )
            .context("Failed to count skills by tag")?;
        Ok(count)
    }
}
