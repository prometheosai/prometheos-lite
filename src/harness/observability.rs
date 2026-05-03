//! Observability Layer - Issue #15
//! Metrics, tracing, and monitoring for harness operations

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessMetrics {
    pub execution_id: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_ms: u64,
    pub steps_completed: u32,
    pub steps_failed: u32,
    pub patches_generated: u32,
    pub patches_applied: u32,
    pub validations_run: u32,
    pub validations_passed: u32,
    pub reviews_performed: u32,
    pub issues_found: u32,
    pub commands_executed: u32,
    pub tokens_consumed: u64,
    pub cost_usd: f64,
    pub custom_metrics: HashMap<String, f64>,
}

#[derive(Debug, Clone)]
pub struct ObservabilityCollector {
    metrics: HarnessMetrics,
    spans: Vec<OperationSpan>,
    current_span: Option<usize>,
    start_instant: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationSpan {
    pub name: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_ms: u64,
    pub status: SpanStatus,
    pub attributes: HashMap<String, String>,
    pub events: Vec<SpanEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SpanStatus {
    Ok,
    Error(String),
    InProgress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub name: String,
    pub attributes: HashMap<String, String>,
}

impl ObservabilityCollector {
    pub fn new(execution_id: String) -> Self {
        Self {
            metrics: HarnessMetrics {
                execution_id,
                start_time: chrono::Utc::now(),
                end_time: None,
                duration_ms: 0,
                steps_completed: 0,
                steps_failed: 0,
                patches_generated: 0,
                patches_applied: 0,
                validations_run: 0,
                validations_passed: 0,
                reviews_performed: 0,
                issues_found: 0,
                commands_executed: 0,
                tokens_consumed: 0,
                cost_usd: 0.0,
                custom_metrics: HashMap::new(),
            },
            spans: Vec::new(),
            current_span: None,
            start_instant: Instant::now(),
        }
    }

    pub fn start_span(&mut self, name: &str) -> usize {
        let span = OperationSpan {
            name: name.to_string(),
            start_time: chrono::Utc::now(),
            end_time: None,
            duration_ms: 0,
            status: SpanStatus::InProgress,
            attributes: HashMap::new(),
            events: Vec::new(),
        };
        let idx = self.spans.len();
        self.spans.push(span);
        self.current_span = Some(idx);
        idx
    }

    pub fn end_span(&mut self, span_id: usize, status: SpanStatus) {
        if let Some(span) = self.spans.get_mut(span_id) {
            let end_time = chrono::Utc::now();
            span.end_time = Some(end_time);
            span.duration_ms =
                (end_time.timestamp_millis() - span.start_time.timestamp_millis()) as u64;
            span.status = status;
        }
        self.current_span = None;
    }

    pub fn add_event(&mut self, name: &str, attributes: HashMap<String, String>) {
        if let Some(idx) = self.current_span {
            if let Some(span) = self.spans.get_mut(idx) {
                span.events.push(SpanEvent {
                    timestamp: chrono::Utc::now(),
                    name: name.to_string(),
                    attributes,
                });
            }
        }
    }

    pub fn record_step_completed(&mut self, success: bool) {
        if success {
            self.metrics.steps_completed += 1;
        } else {
            self.metrics.steps_failed += 1;
        }
    }

    pub fn record_patch_generated(&mut self) {
        self.metrics.patches_generated += 1;
    }

    pub fn record_patch_applied(&mut self, success: bool) {
        if success {
            self.metrics.patches_applied += 1;
        }
    }

    pub fn record_validation(&mut self, passed: bool) {
        self.metrics.validations_run += 1;
        if passed {
            self.metrics.validations_passed += 1;
        }
    }

    pub fn record_review(&mut self, issues: u32) {
        self.metrics.reviews_performed += 1;
        self.metrics.issues_found += issues;
    }

    pub fn record_command(&mut self) {
        self.metrics.commands_executed += 1;
    }

    pub fn record_tokens(&mut self, tokens: u64, cost: f64) {
        self.metrics.tokens_consumed += tokens;
        self.metrics.cost_usd += cost;
    }

    pub fn record_custom_metric(&mut self, name: &str, value: f64) {
        self.metrics.custom_metrics.insert(name.to_string(), value);
    }

    pub fn finish(&mut self) -> HarnessMetrics {
        self.metrics.end_time = Some(chrono::Utc::now());
        self.metrics.duration_ms = self.start_instant.elapsed().as_millis() as u64;
        self.metrics.clone()
    }

    pub fn get_metrics(&self) -> &HarnessMetrics {
        &self.metrics
    }

    pub fn get_spans(&self) -> &[OperationSpan] {
        &self.spans
    }

    pub fn get_summary(&self) -> ObservabilitySummary {
        ObservabilitySummary {
            execution_id: self.metrics.execution_id.clone(),
            status: if self.metrics.steps_failed > 0 {
                ExecutionStatus::PartialFailure
            } else {
                ExecutionStatus::Success
            },
            duration_seconds: self.metrics.duration_ms as f64 / 1000.0,
            success_rate: if self.metrics.steps_completed + self.metrics.steps_failed > 0 {
                self.metrics.steps_completed as f64
                    / (self.metrics.steps_completed + self.metrics.steps_failed) as f64
            } else {
                1.0
            },
            total_patches: self.metrics.patches_generated,
            total_issues: self.metrics.issues_found,
            total_cost: self.metrics.cost_usd,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilitySummary {
    pub execution_id: String,
    pub status: ExecutionStatus,
    pub duration_seconds: f64,
    pub success_rate: f64,
    pub total_patches: u32,
    pub total_issues: u32,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionStatus {
    Success,
    PartialFailure,
    Failure,
    Timeout,
}

pub fn create_collector(execution_id: String) -> ObservabilityCollector {
    ObservabilityCollector::new(execution_id)
}

pub fn format_metrics_report(metrics: &HarnessMetrics) -> String {
    format!(
        r#"Harness Execution Metrics
==========================
Execution ID: {}
Duration: {:.2}s
Steps: {} completed, {} failed
Patches: {} generated, {} applied
Validations: {} run, {} passed
Reviews: {} performed, {} issues found
Commands: {} executed
Tokens: {} consumed
Cost: ${:.4}
"#,
        metrics.execution_id,
        metrics.duration_ms as f64 / 1000.0,
        metrics.steps_completed,
        metrics.steps_failed,
        metrics.patches_generated,
        metrics.patches_applied,
        metrics.validations_run,
        metrics.validations_passed,
        metrics.reviews_performed,
        metrics.issues_found,
        metrics.commands_executed,
        metrics.tokens_consumed,
        metrics.cost_usd
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_records_metrics() {
        let mut collector = ObservabilityCollector::new("test-123".to_string());

        collector.record_step_completed(true);
        collector.record_patch_generated();
        collector.record_validation(true);
        collector.record_review(2);

        let metrics = collector.finish();
        assert_eq!(metrics.steps_completed, 1);
        assert_eq!(metrics.patches_generated, 1);
        assert_eq!(metrics.validations_run, 1);
        assert_eq!(metrics.issues_found, 2);
    }

    #[test]
    fn test_span_tracking() {
        let mut collector = ObservabilityCollector::new("test-456".to_string());

        let span_id = collector.start_span("validation");
        collector.add_event("command_start", HashMap::new());
        collector.end_span(span_id, SpanStatus::Ok);

        let spans = collector.get_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "validation");
    }

    #[test]
    fn test_custom_metrics() {
        let mut collector = ObservabilityCollector::new("test-789".to_string());
        collector.record_custom_metric("cache_hit_rate", 0.85);

        let metrics = collector.finish();
        assert_eq!(metrics.custom_metrics.get("cache_hit_rate"), Some(&0.85));
    }
}
