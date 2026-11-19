//! Flow Performance Record Repository
//!
//! Provides database operations for FlowPerformanceRecord entities,
//! storing flow execution performance metrics for analysis and optimization.

use crate::db::Db;
use crate::work::persistence::{
    FlowPerformanceRecord, FlowPerformanceRow, decode_flow_performance, encode_flow_performance,
};
use anyhow::{Context, Result};
use rusqlite::params;

/// Operations for FlowPerformanceRecord entities
pub trait FlowPerformanceOperations {
    /// Create a new flow performance record
    fn create_flow_performance(&self, record: &FlowPerformanceRecord) -> Result<()>;

    /// Get performance records for a work context
    fn get_performance_by_work_context(
        &self,
        work_context_id: &str,
    ) -> Result<Vec<FlowPerformanceRecord>>;

    /// Get performance records for a specific flow
    fn get_performance_by_flow(&self, flow_id: &str) -> Result<Vec<FlowPerformanceRecord>>;

    /// Get the latest performance record for a work context
    fn get_latest_performance(
        &self,
        work_context_id: &str,
    ) -> Result<Option<FlowPerformanceRecord>>;

    /// Get average success score for a work context
    fn get_average_success_score(&self, work_context_id: &str) -> Result<Option<f64>>;

    /// Delete old performance records (cleanup)
    fn delete_old_performance_records(
        &self,
        before: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize>;
}

impl FlowPerformanceOperations for Db {
    fn create_flow_performance(&self, record: &FlowPerformanceRecord) -> Result<()> {
        let conn = self.conn();
        let row = encode_flow_performance(record);

        conn.execute(
            "INSERT INTO flow_performance_records (
                id, flow_id, work_context_id, success_score, duration_ms,
                token_cost, revision_count, executed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.id,
                row.flow_id,
                row.work_context_id,
                row.success_score,
                row.duration_ms,
                row.token_cost,
                row.revision_count,
                row.executed_at,
            ],
        )
        .context("Failed to create flow performance record")?;

        Ok(())
    }

    fn get_performance_by_work_context(
        &self,
        work_context_id: &str,
    ) -> Result<Vec<FlowPerformanceRecord>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, flow_id, work_context_id, success_score, duration_ms,
                    token_cost, revision_count, executed_at
             FROM flow_performance_records
             WHERE work_context_id = ?1
             ORDER BY executed_at DESC",
        )?;

        let rows = stmt
            .query_map(params![work_context_id], |row| {
                Ok(FlowPerformanceRow {
                    id: row.get(0)?,
                    flow_id: row.get(1)?,
                    work_context_id: row.get(2)?,
                    success_score: row.get(3)?,
                    duration_ms: row.get(4)?,
                    token_cost: row.get(5)?,
                    revision_count: row.get(6)?,
                    executed_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.into_iter().map(decode_flow_performance).collect()
    }

    fn get_performance_by_flow(&self, flow_id: &str) -> Result<Vec<FlowPerformanceRecord>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, flow_id, work_context_id, success_score, duration_ms,
                    token_cost, revision_count, executed_at
             FROM flow_performance_records
             WHERE flow_id = ?1
             ORDER BY executed_at DESC",
        )?;

        let rows = stmt
            .query_map(params![flow_id], |row| {
                Ok(FlowPerformanceRow {
                    id: row.get(0)?,
                    flow_id: row.get(1)?,
                    work_context_id: row.get(2)?,
                    success_score: row.get(3)?,
                    duration_ms: row.get(4)?,
                    token_cost: row.get(5)?,
                    revision_count: row.get(6)?,
                    executed_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.into_iter().map(decode_flow_performance).collect()
    }

    fn get_latest_performance(
        &self,
        work_context_id: &str,
    ) -> Result<Option<FlowPerformanceRecord>> {
        let conn = self.conn();

        let mut stmt = conn.prepare(
            "SELECT id, flow_id, work_context_id, success_score, duration_ms,
                    token_cost, revision_count, executed_at
             FROM flow_performance_records
             WHERE work_context_id = ?1
             ORDER BY executed_at DESC
             LIMIT 1",
        )?;

        let mut rows = stmt.query(params![work_context_id])?;

        if let Some(row) = rows.next()? {
            let row = FlowPerformanceRow {
                id: row.get(0)?,
                flow_id: row.get(1)?,
                work_context_id: row.get(2)?,
                success_score: row.get(3)?,
                duration_ms: row.get(4)?,
                token_cost: row.get(5)?,
                revision_count: row.get(6)?,
                executed_at: row.get(7)?,
            };
            Ok(Some(decode_flow_performance(row)?))
        } else {
            Ok(None)
        }
    }

    fn get_average_success_score(&self, work_context_id: &str) -> Result<Option<f64>> {
        let conn = self.conn();

        let avg: Option<f64> = conn.query_row(
            "SELECT AVG(success_score) FROM flow_performance_records WHERE work_context_id = ?1",
            params![work_context_id],
            |row| row.get(0),
        ).ok();

        Ok(avg)
    }

    fn delete_old_performance_records(
        &self,
        before: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        let conn = self.conn();

        let deleted = conn.execute(
            "DELETE FROM flow_performance_records WHERE executed_at < ?1",
            params![before.to_rfc3339()],
        )?;

        Ok(deleted)
    }
}
