use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::AsDb;
use crate::work::types::HarnessRunMetricsRecord;

pub trait WorkRunMetricsOperations {
    fn upsert_harness_run_metrics(&self, record: &HarnessRunMetricsRecord) -> anyhow::Result<()>;
    fn list_harness_run_metrics(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<HarnessRunMetricsRecord>>;
    fn get_harness_run_metrics(
        &self,
        work_context_id: &str,
        run_id: &str,
    ) -> anyhow::Result<Option<HarnessRunMetricsRecord>>;
}

impl<T: AsDb> WorkRunMetricsOperations for T {
    fn upsert_harness_run_metrics(&self, record: &HarnessRunMetricsRecord) -> anyhow::Result<()> {
        let conn = self.as_db().conn();
        conn.execute(
            "INSERT OR REPLACE INTO harness_run_metrics (
                work_context_id, run_id, trace_summary, token_usage, quality_metrics, trajectory, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &record.work_context_id,
                &record.run_id,
                serde_json::to_string(&record.trace_summary)?,
                serde_json::to_string(&record.token_usage)?,
                serde_json::to_string(&record.quality_metrics)?,
                serde_json::to_string(&record.trajectory)?,
                &record.created_at.to_rfc3339(),
            ],
        )
        .context("Failed to upsert harness run metrics")?;
        Ok(())
    }

    fn list_harness_run_metrics(
        &self,
        work_context_id: &str,
    ) -> anyhow::Result<Vec<HarnessRunMetricsRecord>> {
        let conn = self.as_db().conn();
        let mut stmt = conn
            .prepare(
                "SELECT work_context_id, run_id, trace_summary, token_usage, quality_metrics, trajectory, created_at
                 FROM harness_run_metrics
                 WHERE work_context_id = ?1
                 ORDER BY created_at DESC",
            )
            .context("Failed to prepare harness run metrics list query")?;
        let rows = stmt
            .query_map(params![work_context_id], |row| {
                let created_at_raw: String = row.get(6)?;
                let created_at = DateTime::parse_from_rfc3339(&created_at_raw)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(HarnessRunMetricsRecord {
                    work_context_id: row.get(0)?,
                    run_id: row.get(1)?,
                    trace_summary: serde_json::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or_default(),
                    token_usage: serde_json::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or_default(),
                    quality_metrics: serde_json::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or_default(),
                    trajectory: serde_json::from_str(&row.get::<_, String>(5)?)
                        .unwrap_or(serde_json::Value::Null),
                    created_at,
                })
            })
            .context("Failed to query harness run metrics")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("Failed to parse harness run metrics row")?);
        }
        Ok(out)
    }

    fn get_harness_run_metrics(
        &self,
        work_context_id: &str,
        run_id: &str,
    ) -> anyhow::Result<Option<HarnessRunMetricsRecord>> {
        let conn = self.as_db().conn();
        let mut stmt = conn
            .prepare(
                "SELECT work_context_id, run_id, trace_summary, token_usage, quality_metrics, trajectory, created_at
                 FROM harness_run_metrics
                 WHERE work_context_id = ?1 AND run_id = ?2",
            )
            .context("Failed to prepare harness run metrics get query")?;
        let row = stmt.query_row(params![work_context_id, run_id], |row| {
            let created_at_raw: String = row.get(6)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok(HarnessRunMetricsRecord {
                work_context_id: row.get(0)?,
                run_id: row.get(1)?,
                trace_summary: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                token_usage: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                quality_metrics: serde_json::from_str(&row.get::<_, String>(4)?)
                    .unwrap_or_default(),
                trajectory: serde_json::from_str(&row.get::<_, String>(5)?)
                    .unwrap_or(serde_json::Value::Null),
                created_at,
            })
        });

        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e).context("Failed to load harness run metrics"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::Db;
    use crate::work::types::{HarnessQualityMetrics, HarnessTraceSummary, TokenUsageSummary};
    use crate::work::{WorkContextService, types::WorkDomain};
    use std::sync::Arc;

    #[test]
    fn upsert_and_query_run_metrics() {
        let db = Db::in_memory().expect("db");
        let svc = WorkContextService::new(Arc::new(db));
        let _ctx = svc
            .create_context(
                "user-1".to_string(),
                "ctx".to_string(),
                WorkDomain::Software,
                "goal".to_string(),
            )
            .expect("create context");
        let record = HarnessRunMetricsRecord {
            work_context_id: "ctx-1".to_string(),
            run_id: "run-1".to_string(),
            trace_summary: HarnessTraceSummary {
                run_id: "run-1".to_string(),
                duration_ms: 12,
                node_count: 3,
                tool_count: 1,
                error_count: 0,
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
                estimated_cost_cents: 1,
            },
            token_usage: TokenUsageSummary {
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
                estimated_cost_cents: 1,
            },
            quality_metrics: HarnessQualityMetrics {
                review_issue_count: 1,
                critical_issue_count: 0,
                rejection_rate: 0.0,
                hallucination_risk_rate: 0.0,
            },
            trajectory: serde_json::json!({"steps":[{"id":"s1"}]}),
            created_at: Utc::now(),
        };

        let mut record = record;
        record.work_context_id = _ctx.id.clone();

        WorkRunMetricsOperations::upsert_harness_run_metrics(svc.get_db().as_ref(), &record)
            .expect("upsert");
        let loaded = WorkRunMetricsOperations::get_harness_run_metrics(
            svc.get_db().as_ref(),
            &record.work_context_id,
            "run-1",
        )
        .expect("get")
        .expect("exists");
        assert_eq!(loaded.run_id, "run-1");
        assert_eq!(loaded.trace_summary.total_tokens, 30);

        let listed = WorkRunMetricsOperations::list_harness_run_metrics(
            svc.get_db().as_ref(),
            &record.work_context_id,
        )
        .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].work_context_id, record.work_context_id);
    }
}
