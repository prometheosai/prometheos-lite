//! Domain-owned persistence contract for the flow and harness metrics rows.
//!
//! The `db` layer depends on this module and on nothing else under `work`
//! for the metrics repositories. Ownership:
//!
//! - domain types and invariants: `work::types` — records are plain data,
//!   timestamps are RFC 3339, and this contract never fabricates values;
//! - persistence representations: the `*Row` structs below are the exact
//!   column layout the tables store;
//! - serialization mappings: `encode_*`/`decode_*` below are the only code
//!   that knows timestamp formats, integer widths and JSON envelopes.
//!   Corrupt stored values are refused, never silently replaced with
//!   `Utc::now()` or a zero-valued record;
//! - repository interfaces: `db` implements them on top of this contract.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

pub use crate::work::artifact::Artifact;
pub use crate::work::decision::DecisionRecord;
pub use crate::work::plan::ExecutionPlan;
pub use crate::work::types::{
    ApprovalPolicy, AutonomyLevel, CompletionCriterion, ExecutionRecord, WorkContext, WorkDomain,
    WorkPhase, WorkPriority, WorkStatus,
};
pub use crate::work::types::{
    FlowPerformanceRecord, HarnessQualityMetrics, HarnessRunMetricsRecord, HarnessTraceSummary,
    TokenUsageSummary,
};

/// The exact `flow_performance_records` row, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowPerformanceRow {
    pub id: String,
    pub flow_id: String,
    pub work_context_id: String,
    pub success_score: f64,
    pub duration_ms: i64,
    pub token_cost: f64,
    pub revision_count: i64,
    pub executed_at: String,
}

/// Map a domain record to its persisted representation. Values that cannot
/// be represented or round-tripped faithfully are rejected here, never
/// wrapped or silently truncated.
pub fn encode_flow_performance(record: &FlowPerformanceRecord) -> Result<FlowPerformanceRow> {
    if record.duration_ms > i64::MAX as u64 {
        bail!(
            "duration_ms {} exceeds the i64 range of the flow_performance_records.duration_ms column",
            record.duration_ms
        );
    }
    if record.revision_count > i64::MAX as u32 {
        bail!(
            "revision_count {} exceeds the i64 range of the flow_performance_records.revision_count column",
            record.revision_count
        );
    }
    if !record.success_score.is_finite() {
        bail!(
            "success_score {} is not finite; refusing to persist a fabricated measurement",
            record.success_score
        );
    }
    if !record.token_cost.is_finite() {
        bail!(
            "token_cost {} is not finite; refusing to persist a fabricated measurement",
            record.token_cost
        );
    }
    Ok(FlowPerformanceRow {
        id: record.id.clone(),
        flow_id: record.flow_id.clone(),
        work_context_id: record.work_context_id.clone(),
        success_score: record.success_score as f64,
        duration_ms: record.duration_ms as i64,
        token_cost: record.token_cost,
        revision_count: record.revision_count as i64,
        executed_at: record.executed_at.to_rfc3339(),
    })
}

/// Rebuild a domain record from a stored row. Corrupt values are refused:
/// a bad timestamp, a negative duration or a negative revision count is a
/// typed error that names the row, never a fabricated value.
pub fn decode_flow_performance(row: FlowPerformanceRow) -> Result<FlowPerformanceRecord> {
    let executed_at = DateTime::parse_from_rfc3339(&row.executed_at)
        .with_context(|| {
            format!(
                "corrupt executed_at timestamp in flow_performance_records (id {}): {:?}",
                row.id, row.executed_at
            )
        })?
        .with_timezone(&Utc);
    if row.duration_ms < 0 {
        bail!(
            "corrupt duration_ms in flow_performance_records (id {}): {}",
            row.id,
            row.duration_ms
        );
    }
    if row.revision_count < 0 || row.revision_count > u32::MAX as i64 {
        bail!(
            "corrupt revision_count in flow_performance_records (id {}): {}",
            row.id,
            row.revision_count
        );
    }
    if !row.success_score.is_finite() {
        bail!(
            "corrupt success_score in flow_performance_records (id {}): {}",
            row.id,
            row.success_score
        );
    }
    if !row.token_cost.is_finite() {
        bail!(
            "corrupt token_cost in flow_performance_records (id {}): {}",
            row.id,
            row.token_cost
        );
    }
    Ok(FlowPerformanceRecord {
        id: row.id,
        flow_id: row.flow_id,
        work_context_id: row.work_context_id,
        success_score: row.success_score as f32,
        duration_ms: row.duration_ms as u64,
        token_cost: row.token_cost,
        revision_count: row.revision_count as u32,
        executed_at,
    })
}

/// The exact `harness_run_metrics` row, as stored: nested records are
/// JSON envelopes in their own columns.
#[derive(Debug, Clone, PartialEq)]
pub struct HarnessRunMetricsRow {
    pub work_context_id: String,
    pub run_id: String,
    pub trace_summary: String,
    pub token_usage: String,
    pub quality_metrics: String,
    pub trajectory: String,
    pub created_at: String,
}

fn encode_json<T: serde::Serialize>(value: &T, column: &str, run_id: &str) -> Result<String> {
    serde_json::to_string(value).with_context(|| {
        format!("failed to serialize {column} for harness_run_metrics (run_id {run_id})")
    })
}

fn decode_json<T: serde::de::DeserializeOwned>(
    encoded: &str,
    column: &str,
    run_id: &str,
) -> Result<T> {
    serde_json::from_str(encoded).with_context(|| {
        format!("corrupt {column} JSON in harness_run_metrics (run_id {run_id}): refusing to fabricate a zero record")
    })
}

/// Map a domain record to its persisted representation.
pub fn encode_harness_run_metrics(
    record: &HarnessRunMetricsRecord,
) -> Result<HarnessRunMetricsRow> {
    Ok(HarnessRunMetricsRow {
        work_context_id: record.work_context_id.clone(),
        run_id: record.run_id.clone(),
        trace_summary: encode_json(&record.trace_summary, "trace_summary", &record.run_id)?,
        token_usage: encode_json(&record.token_usage, "token_usage", &record.run_id)?,
        quality_metrics: encode_json(&record.quality_metrics, "quality_metrics", &record.run_id)?,
        trajectory: record.trajectory.to_string(),
        created_at: record.created_at.to_rfc3339(),
    })
}

/// Rebuild a domain record from a stored row. Corrupt envelopes are refused;
/// they are never replaced with a default/zero record.
pub fn decode_harness_run_metrics(row: HarnessRunMetricsRow) -> Result<HarnessRunMetricsRecord> {
    let created_at = DateTime::parse_from_rfc3339(&row.created_at)
        .with_context(|| {
            format!(
                "corrupt created_at timestamp in harness_run_metrics (run_id {}): {:?}",
                row.run_id, row.created_at
            )
        })?
        .with_timezone(&Utc);
    let run_id = row.run_id.clone();
    Ok(HarnessRunMetricsRecord {
        work_context_id: row.work_context_id,
        run_id,
        trace_summary: decode_json(&row.trace_summary, "trace_summary", &row.run_id)?,
        token_usage: decode_json(&row.token_usage, "token_usage", &row.run_id)?,
        quality_metrics: decode_json(&row.quality_metrics, "quality_metrics", &row.run_id)?,
        trajectory: serde_json::from_str(&row.trajectory).with_context(|| {
            format!(
                "corrupt trajectory JSON in harness_run_metrics (run_id {})",
                row.run_id
            )
        })?,
        created_at,
    })
}

/// The control-plane settings a context starts with: the autonomy level
/// and the approval policy. The tables store them as JSON-encoded enum tags.
/// This module is the only place that knows those spellings; a mismatch
/// surfaces as a typed error naming the column, never as a fabricated value.
pub fn encode_approval_policy(policy: &ApprovalPolicy) -> String {
    serde_json::to_string(policy).expect("approval policy serializes")
}

pub fn parse_approval_policy(text: &str, column: &str) -> Result<ApprovalPolicy> {
    serde_json::from_str(text)
        .with_context(|| format!("corrupt approval policy in {column}: {text:?}"))
}

pub fn encode_autonomy_level(level: &AutonomyLevel) -> String {
    serde_json::to_string(level).expect("autonomy level serializes")
}

pub fn parse_autonomy_level(text: &str, column: &str) -> Result<AutonomyLevel> {
    serde_json::from_str(text)
        .with_context(|| format!("corrupt autonomy level in {column}: {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow_record() -> FlowPerformanceRecord {
        FlowPerformanceRecord {
            id: "fpr-1".to_string(),
            flow_id: "flow-7".to_string(),
            work_context_id: "ctx-1".to_string(),
            success_score: 0.85,
            duration_ms: 42,
            token_cost: 1.5,
            revision_count: 3,
            executed_at: "2026-09-01T12:00:00+00:00".parse().unwrap(),
        }
    }

    fn metrics_record() -> HarnessRunMetricsRecord {
        HarnessRunMetricsRecord {
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
            created_at: "2026-09-01T12:00:00+00:00".parse().unwrap(),
        }
    }

    #[test]
    fn flow_performance_round_trips() {
        let record = flow_record();
        let row = encode_flow_performance(&record).expect("encodes");
        let decoded = decode_flow_performance(row).expect("round trip");
        assert_eq!(decoded.id, record.id);
        assert_eq!(decoded.duration_ms, record.duration_ms);
        assert_eq!(decoded.revision_count, record.revision_count);
        assert!((decoded.success_score - record.success_score).abs() < f32::EPSILON);
        assert!((decoded.token_cost - record.token_cost).abs() < f64::EPSILON);
    }

    #[test]
    fn corrupt_executed_at_is_refused_not_fabricated() {
        let mut row = encode_flow_performance(&flow_record()).expect("encodes");
        row.executed_at = "not-a-timestamp".to_string();
        let error = decode_flow_performance(row).expect_err("corrupt timestamp must error");
        assert!(format!("{error:#}").contains("executed_at"));
    }

    #[test]
    fn negative_duration_is_refused_not_wrapped() {
        let mut row = encode_flow_performance(&flow_record()).expect("encodes");
        row.duration_ms = -1;
        let error = decode_flow_performance(row).expect_err("negative duration must error");
        assert!(format!("{error:#}").contains("duration_ms"));
    }

    #[test]
    fn harness_metrics_round_trip() {
        let record = metrics_record();
        let row = encode_harness_run_metrics(&record).expect("encodes");
        let decoded = decode_harness_run_metrics(row).expect("round trip");
        assert_eq!(decoded.run_id, record.run_id);
        assert_eq!(decoded.trace_summary.total_tokens, 30);
        assert_eq!(decoded.trajectory, record.trajectory);
    }

    #[test]
    fn corrupt_trace_summary_is_refused_not_zeroed() {
        let mut row = encode_harness_run_metrics(&metrics_record()).expect("encodes");
        row.trace_summary = "{\"run_id\": 123".to_string();
        let error = decode_harness_run_metrics(row).expect_err("corrupt JSON must error");
        assert!(format!("{error:#}").contains("trace_summary"));
    }

    #[test]
    fn corrupt_token_usage_is_refused_not_zeroed() {
        let mut row = encode_harness_run_metrics(&metrics_record()).expect("encodes");
        row.token_usage = "garbage".to_string();
        let error = decode_harness_run_metrics(row).expect_err("corrupt JSON must error");
        assert!(format!("{error:#}").contains("token_usage"));
    }

    #[test]
    fn corrupt_quality_metrics_is_refused_not_zeroed() {
        let mut row = encode_harness_run_metrics(&metrics_record()).expect("encodes");
        row.quality_metrics = "{".to_string();
        let error = decode_harness_run_metrics(row).expect_err("corrupt JSON must error");
        assert!(format!("{error:#}").contains("quality_metrics"));
    }

    #[test]
    fn corrupt_created_at_is_refused_not_fabricated() {
        let mut row = encode_harness_run_metrics(&metrics_record()).expect("encodes");
        row.created_at = "2026-13-40T99:99:99Z".to_string();
        let error = decode_harness_run_metrics(row).expect_err("corrupt timestamp must error");
        assert!(format!("{error:#}").contains("created_at"));
    }

    #[test]
    fn approval_policy_round_trip_every_variant() {
        for policy in [
            ApprovalPolicy::Auto,
            ApprovalPolicy::RequireForTools,
            ApprovalPolicy::RequireForSideEffects,
            ApprovalPolicy::RequireForUntrusted,
            ApprovalPolicy::ManualAll,
        ] {
            let tag = encode_approval_policy(&policy);
            assert_eq!(parse_approval_policy(&tag, "test.column").unwrap(), policy);
        }
        assert_eq!(encode_approval_policy(&ApprovalPolicy::Auto), "\"Auto\"");
        assert_eq!(
            encode_approval_policy(&ApprovalPolicy::ManualAll),
            "\"ManualAll\""
        );
    }

    #[test]
    fn approval_policy_rejects_unknown_and_malformed_tags() {
        for bad in ["\"Sometimes\"", "1", "auto", "\"auto\"", "[]", "{}"] {
            let error = parse_approval_policy(bad, "test.column")
                .expect_err(&format!("{bad:?} must be refused"));
            assert!(format!("{error:#}").contains("test.column"));
        }
    }

    #[test]
    fn autonomy_level_round_trip_every_variant() {
        for level in [
            AutonomyLevel::Chat,
            AutonomyLevel::Review,
            AutonomyLevel::Autonomous,
        ] {
            let tag = encode_autonomy_level(&level);
            assert_eq!(parse_autonomy_level(&tag, "test.column").unwrap(), level);
        }
        assert_eq!(encode_autonomy_level(&AutonomyLevel::Review), "\"Review\"");
    }

    #[test]
    fn autonomy_level_rejects_unknown_and_malformed_tags() {
        for bad in ["\"Fully\"", "2", "review", "\"review\""] {
            let error = parse_autonomy_level(bad, "test.column")
                .expect_err(&format!("{bad:?} must be refused"));
            assert!(format!("{error:#}").contains("test.column"));
        }
    }

    #[test]
    fn encode_flow_performance_rejects_unsnappable_duration() {
        let mut record = flow_record();
        record.duration_ms = u64::MAX;
        let error = encode_flow_performance(&record).expect_err("must refuse to wrap");
        assert!(format!("{error:#}").contains("duration_ms"));
    }

    #[test]
    fn encode_flow_performance_rejects_nan_success_score() {
        let mut record = flow_record();
        record.success_score = f32::NAN;
        let error = encode_flow_performance(&record).expect_err("NaN must be refused");
        assert!(format!("{error:#}").contains("success_score"));
    }

    #[test]
    fn encode_flow_performance_rejects_infinite_token_cost() {
        let mut record = flow_record();
        record.token_cost = f64::INFINITY;
        let error = encode_flow_performance(&record).expect_err("infinity must be refused");
        assert!(format!("{error:#}").contains("token_cost"));
    }

    #[test]
    fn decode_flow_performance_rejects_non_finite_success_score() {
        let mut row = encode_flow_performance(&flow_record()).expect("encodes");
        row.success_score = f64::NAN;
        let error = decode_flow_performance(row).expect_err("NaN must be refused");
        assert!(format!("{error:#}").contains("success_score"));
    }

    #[test]
    fn decode_flow_performance_rejects_non_finite_token_cost() {
        let mut row = encode_flow_performance(&flow_record()).expect("encodes");
        row.token_cost = f64::INFINITY;
        let error = decode_flow_performance(row).expect_err("infinity must be refused");
        assert!(format!("{error:#}").contains("token_cost"));
    }
}

/// -- WorkContext row persistence -------------------------------------------
///
/// The exact column layout of the `work_contexts` table, owned by the domain
/// persistence contract. `db` may depend on this module; nothing else does.
///
/// All JSON-typed columns are decoded through a fail-closed parser: corrupt
/// rows are refused with the column name, never silently defaulted; string
/// timestamps are RFC3339, refused on a parse failure; the `"Cancelled"`
/// literal is the JSON value of `WorkStatus::Cancelled`.
///
/// `EvaluationResult`, `CompletionCriterion`, etc. are parsed by the same
/// `decode_json` gate so malformed shapes cannot leak into the domain.

/// The raw column strings for a stored `work_contexts` row. Field order does
/// not match the table's physical column order; this is a typed record, not a
/// positional row. The `db` layer is responsible for projecting the table's
/// columns into this shape.
pub struct WorkContextRow {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub domain: String,
    pub domain_profile_id: Option<String>,
    pub context_type: String,
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
    pub parent_context_id: Option<String>,
    pub priority: String,
    pub due_at: Option<String>,
    pub goal: String,
    pub requirements: String,
    pub constraints: String,
    pub status: String,
    pub current_phase: String,
    pub blocked_reason: Option<String>,
    pub plan: Option<String>,
    pub approved_plan: Option<String>,
    pub artifacts: String,
    pub memory_refs: String,
    pub decisions: String,
    pub flow_runs: String,
    pub tool_trace: String,
    pub execution_metadata: String,
    pub open_questions: String,
    pub autonomy_level: String,
    pub approval_policy: String,
    pub summary: Option<String>,
    pub completion_criteria: String,
    pub last_activity_at: String,
    pub metadata: String,
    pub playbook_id: Option<String>,
    pub evaluation_result: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Decode a raw column record into a domain `WorkContext`, failing closed on
/// any corrupt column.
pub fn decode_work_context(row: WorkContextRow) -> Result<WorkContext> {
    Ok(WorkContext {
        id: row.id,
        user_id: row.user_id,
        title: row.title,
        domain: decode_json_spec(&row.domain, "domain")?,
        domain_profile_id: row.domain_profile_id,
        context_type: row.context_type,
        project_id: row.project_id,
        conversation_id: row.conversation_id,
        parent_context_id: row.parent_context_id,
        priority: decode_json_spec(&row.priority, "priority")?,
        due_at: decode_opt_timestamp(row.due_at, "due_at")?,
        goal: row.goal,
        requirements: decode_json_spec(&row.requirements, "requirements")?,
        constraints: decode_json_spec(&row.constraints, "constraints")?,
        status: decode_json_spec(&row.status, "status")?,
        current_phase: decode_json_spec(&row.current_phase, "current_phase")?,
        blocked_reason: row.blocked_reason,
        plan: decode_opt_json_spec(&row.plan, "plan")?,
        approved_plan: decode_opt_json_spec(&row.approved_plan, "approved_plan")?,
        artifacts: decode_json_spec(&row.artifacts, "artifacts")?,
        memory_refs: decode_json_spec(&row.memory_refs, "memory_refs")?,
        decisions: decode_json_spec(&row.decisions, "decisions")?,
        flow_runs: decode_json_spec(&row.flow_runs, "flow_runs")?,
        tool_trace: decode_json_spec(&row.tool_trace, "tool_trace")?,
        execution_metadata: decode_json_spec(&row.execution_metadata, "execution_metadata")?,
        open_questions: decode_json_spec(&row.open_questions, "open_questions")?,
        autonomy_level: decode_json_spec(&row.autonomy_level, "autonomy_level")?,
        approval_policy: decode_json_spec(&row.approval_policy, "approval_policy")?,
        summary: row.summary,
        completion_criteria: decode_json_spec(&row.completion_criteria, "completion_criteria")?,
        last_activity_at: decode_timestamp(&row.last_activity_at, "last_activity_at")?,
        metadata: serde_json::from_str(&row.metadata)
            .with_context(|| format!("work_contexts.metadata is corrupt: {:?}", row.metadata))?,
        playbook_id: row.playbook_id,
        evaluation_result: decode_opt_json(&row.evaluation_result, "evaluation_result")?,
        created_at: decode_timestamp(&row.created_at, "created_at")?,
        updated_at: decode_timestamp(&row.updated_at, "updated_at")?,
    })
}

fn decode_json_spec<T: serde::de::DeserializeOwned>(text: &str, field: &str) -> Result<T> {
    serde_json::from_str(text)
        .with_context(|| format!("work_contexts.{field} is corrupt: {text:?}"))
}

fn decode_opt_json_spec<T: serde::de::DeserializeOwned>(
    text: &Option<String>,
    field: &str,
) -> Result<Option<T>> {
    match text {
        None => Ok(None),
        Some(value) => serde_json::from_str(value)
            .with_context(|| format!("work_contexts.{field} is corrupt: {value:?}"))
            .map(Some),
    }
}

fn decode_opt_json(text: &Option<String>, field: &str) -> Result<Option<serde_json::Value>> {
    match text {
        None => Ok(None),
        Some(value) => serde_json::from_str(value)
            .with_context(|| format!("work_contexts.{field} is corrupt: {value:?}"))
            .map(Some),
    }
}

fn decode_timestamp(text: &str, field: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .with_context(|| format!("work_contexts.{field} is not RFC3339: {text:?}"))
}

fn decode_opt_timestamp(
    text: Option<String>,
    field: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    match text {
        None => Ok(None),
        Some(value) => chrono::DateTime::parse_from_rfc3339(&value)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .map(Some)
            .with_context(|| format!("work_contexts.{field} is not RFC3339: {value:?}")),
    }
}

/// Serialize a work-context back to the row layout's JSON-typed columns. Used
/// by the write paths. Values are not validated against column width here —
/// the caller picks columns by table name; the contract only refuses
/// unserializable values.
pub fn encode_work_context(context: &WorkContext) -> WorkContextRow {
    WorkContextRow {
        id: context.id.clone(),
        user_id: context.user_id.clone(),
        title: context.title.clone(),
        domain: serde_json::to_string(&context.domain).expect("domain serializes"),
        domain_profile_id: context.domain_profile_id.clone(),
        context_type: context.context_type.clone(),
        project_id: context.project_id.clone(),
        conversation_id: context.conversation_id.clone(),
        parent_context_id: context.parent_context_id.clone(),
        priority: serde_json::to_string(&context.priority).expect("priority serializes"),
        due_at: context.due_at.map(|d| d.to_rfc3339()),
        goal: context.goal.clone(),
        requirements: serde_json::to_string(&context.requirements)
            .expect("requirements serializes"),
        constraints: serde_json::to_string(&context.constraints).expect("constraints serializes"),
        status: serde_json::to_string(&context.status).expect("status serializes"),
        current_phase: serde_json::to_string(&context.current_phase)
            .expect("current_phase serializes"),
        blocked_reason: context.blocked_reason.clone(),
        plan: context
            .plan
            .as_ref()
            .map(|p| serde_json::to_string(p).expect("plan serializes")),
        approved_plan: context
            .approved_plan
            .as_ref()
            .map(|p| serde_json::to_string(p).expect("approved_plan serializes")),
        artifacts: serde_json::to_string(&context.artifacts).expect("artifacts serializes"),
        memory_refs: serde_json::to_string(&context.memory_refs).expect("memory_refs serializes"),
        decisions: serde_json::to_string(&context.decisions).expect("decisions serializes"),
        flow_runs: serde_json::to_string(&context.flow_runs).expect("flow_runs serializes"),
        tool_trace: serde_json::to_string(&context.tool_trace).expect("tool_trace serializes"),
        execution_metadata: serde_json::to_string(&context.execution_metadata)
            .expect("execution_metadata serializes"),
        open_questions: serde_json::to_string(&context.open_questions)
            .expect("open_questions serializes"),
        autonomy_level: serde_json::to_string(&context.autonomy_level)
            .expect("autonomy_level serializes"),
        approval_policy: serde_json::to_string(&context.approval_policy)
            .expect("approval_policy serializes"),
        summary: context.summary.clone(),
        completion_criteria: serde_json::to_string(&context.completion_criteria)
            .expect("completion_criteria serializes"),
        last_activity_at: context.last_activity_at.to_rfc3339(),
        metadata: serde_json::to_string(&context.metadata).expect("metadata serializes"),
        playbook_id: context.playbook_id.clone(),
        evaluation_result: context
            .evaluation_result
            .as_ref()
            .map(|v| serde_json::to_string(v).expect("evaluation_result serializes")),
        created_at: context.created_at.to_rfc3339(),
        updated_at: context.updated_at.to_rfc3339(),
    }
}
