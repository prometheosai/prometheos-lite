//! End-to-end persistence contract tests for the metrics repositories.
//! Exercises `prometheos_lite::db` against the public repository API,
//! including rejection of corrupt stored envelopes (nothing is
//! silently re-fabricated on read).

use std::sync::Arc;

use chrono::Utc;
use prometheos_lite::db::Db;
use prometheos_lite::db::repository::{FlowPerformanceOperations, WorkRunMetricsOperations};
use prometheos_lite::work::WorkContextService;
use prometheos_lite::work::provenance::JournalContext;
use prometheos_lite::work::types::{
    ApprovalPolicy, AutonomyLevel, FlowPerformanceRecord, HarnessQualityMetrics,
    HarnessRunMetricsRecord, HarnessTraceSummary, TokenUsageSummary, WorkDomain,
};

fn journal() -> JournalContext {
    JournalContext::internal_system(
        format!("test-{}", uuid::Uuid::new_v4()),
        JournalContext::work_authority(AutonomyLevel::Review, ApprovalPolicy::Auto),
    )
}

/// Stand up an in-memory database with one context; returns the service
/// (which keeps the `Arc<Db>` alive) and the created context id.
fn setup() -> (WorkContextService, String) {
    let db = Db::in_memory().expect("db");
    let svc = WorkContextService::new(Arc::new(db));
    let ctx = svc
        .create_context(
            "user-1".to_string(),
            "metrics ctx".to_string(),
            WorkDomain::Software,
            "goal".to_string(),
            &journal(),
        )
        .expect("create context");
    (svc, ctx.id)
}

fn flow_record(id: &str, flow_id: &str, work_context_id: &str) -> FlowPerformanceRecord {
    FlowPerformanceRecord {
        id: id.to_string(),
        flow_id: flow_id.to_string(),
        work_context_id: work_context_id.to_string(),
        success_score: 0.9,
        duration_ms: 125,
        token_cost: 0.42,
        revision_count: 2,
        executed_at: Utc::now(),
    }
}

fn metrics_record(work_context_id: &str, run_id: &str) -> HarnessRunMetricsRecord {
    HarnessRunMetricsRecord {
        work_context_id: work_context_id.to_string(),
        run_id: run_id.to_string(),
        trace_summary: HarnessTraceSummary {
            run_id: run_id.to_string(),
            duration_ms: 100,
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
    }
}

#[test]
fn flow_performance_round_trips_through_the_repository() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let record = flow_record("fpr-1", "flow-1", &ctx_id);
    db.create_flow_performance(&record).expect("create");

    let by_context = db
        .get_performance_by_work_context(&ctx_id)
        .expect("query by context");
    assert_eq!(by_context.len(), 1);
    assert_eq!(by_context[0].id, "fpr-1");
    assert_eq!(by_context[0].duration_ms, 125);

    let by_flow = db.get_performance_by_flow("flow-1").expect("query by flow");
    assert_eq!(by_flow.len(), 1);
    assert_eq!(by_flow[0].token_cost, 0.42);

    let latest = db
        .get_latest_performance(&ctx_id)
        .expect("latest")
        .expect("present");
    assert_eq!(latest.revision_count, 2);
}

#[test]
fn corrupt_stored_timestamp_is_refused_not_fabricated() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let record = flow_record("fpr-bad", "flow-bad", &ctx_id);
    db.create_flow_performance(&record).expect("create");

    db.conn()
        .execute(
            "UPDATE flow_performance_records SET executed_at = 'not-a-timestamp' WHERE id = 'fpr-bad'",
            [],
        )
        .expect("corrupt the stored row");

    let error = db
        .get_performance_by_flow("flow-bad")
        .expect_err("corrupt timestamp must be refused, not replaced with now()");
    assert!(format!("{error:#}").contains("executed_at"));
}

#[test]
fn negative_stored_duration_is_refused_not_wrapped() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let record = flow_record("fpr-neg", "flow-neg", &ctx_id);
    db.create_flow_performance(&record).expect("create");

    db.conn()
        .execute(
            "UPDATE flow_performance_records SET duration_ms = -1 WHERE id = 'fpr-neg'",
            [],
        )
        .expect("corrupt the stored row");

    let error = db
        .get_performance_by_flow("flow-neg")
        .expect_err("negative duration must be refused, not wrapped to a huge u64");
    assert!(format!("{error:#}").contains("duration_ms"));
}

#[test]
fn harness_metrics_round_trips_through_the_repository() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let record = metrics_record(&ctx_id, "run-1");
    db.upsert_harness_run_metrics(&record).expect("upsert");

    let loaded = db
        .get_harness_run_metrics(&ctx_id, "run-1")
        .expect("get")
        .expect("present");
    assert_eq!(loaded.run_id, "run-1");
    assert_eq!(loaded.trace_summary.total_tokens, 30);
    assert_eq!(loaded.trajectory, record.trajectory);

    let listed = db.list_harness_run_metrics(&ctx_id).expect("list");
    assert_eq!(listed.len(), 1);
}

#[test]
fn corrupt_stored_metrics_json_is_refused_not_zeroed() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let record = metrics_record(&ctx_id, "run-corrupt");
    db.upsert_harness_run_metrics(&record).expect("upsert");

    db.conn()
        .execute(
            "UPDATE harness_run_metrics SET trace_summary = '{\"run_id\": 1' WHERE run_id = 'run-corrupt'",
            [],
        )
        .expect("corrupt the stored row");

    let error = db
        .get_harness_run_metrics(&ctx_id, "run-corrupt")
        .expect_err("corrupt JSON must be refused, not zero-filled");
    assert!(format!("{error:#}").contains("trace_summary"));
}

#[test]
fn unrepresentable_duration_is_rejected_before_persistence() {
    let (svc, ctx_id) = setup();
    let db = svc.get_db();
    let mut record = flow_record("fpr-wrap", "flow-wrap", &ctx_id);
    record.duration_ms = u64::MAX;
    let error = db
        .create_flow_performance(&record)
        .expect_err("out-of-i64-range duration must be refused, not wrapped");
    assert!(format!("{error:#}").contains("duration_ms"));
    let remaining = db
        .get_performance_by_work_context(&ctx_id)
        .expect("query context");
    assert!(remaining.is_empty(), "refused record must not be persisted");
}
