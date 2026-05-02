use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trajectory {
    pub id: String,
    pub work_context_id: String,
    pub steps: Vec<TrajectoryStep>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrajectoryStep {
    pub step_id: String,
    pub phase: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub tool_results: Vec<ToolResultRecord>,
    pub errors: Vec<String>,
    pub tokens: Option<u32>,
    pub duration_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub tool: String,
    pub input_summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResultRecord {
    pub tool: String,
    pub success: bool,
    pub output_summary: String,
}
impl Trajectory {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            work_context_id: id.into(),
            steps: vec![],
            started_at: Utc::now(),
            completed_at: None,
        }
    }
    pub fn record_step(&mut self, phase: impl Into<String>, duration_ms: u64, errors: Vec<String>) {
        self.steps.push(TrajectoryStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            phase: phase.into(),
            tool_calls: vec![],
            tool_results: vec![],
            errors,
            tokens: None,
            duration_ms,
        })
    }
    pub fn complete(&mut self) {
        self.completed_at = Some(Utc::now())
    }
}
