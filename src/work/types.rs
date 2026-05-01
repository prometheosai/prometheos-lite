//! Core WorkContext types and enums

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{artifact::Artifact, decision::DecisionRecord, plan::ExecutionPlan};

/// ExecutionRecord - metadata for individual execution steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub node_id: String,
    pub model: String,
    pub provider: String,
    pub latency_ms: u64,
    pub tokens: Option<u32>,
    pub cost: Option<f64>,
    pub timestamp: DateTime<Utc>,
}

/// FlowPerformanceRecord - tracks performance metrics for flow executions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPerformanceRecord {
    /// Unique identifier
    pub id: String,
    /// Flow ID that was executed
    pub flow_id: String,
    /// WorkContext ID this execution belongs to
    pub work_context_id: String,
    /// Success score (0.0 to 1.0)
    pub success_score: f32,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
    /// Token cost
    pub token_cost: f64,
    /// Number of revisions during execution
    pub revision_count: u32,
    /// Timestamp of execution
    pub executed_at: DateTime<Utc>,
}

impl ExecutionRecord {
    /// Create an ExecutionRecord from a GenerateResult
    pub fn from_generate_result(
        node_id: String,
        result: &crate::flow::intelligence::GenerateResult,
    ) -> Self {
        Self {
            node_id,
            model: result.model.clone(),
            provider: result.provider.clone(),
            latency_ms: result.latency_ms,
            tokens: result.tokens_used,
            cost: None, // Cost calculation to be implemented based on provider pricing
            timestamp: Utc::now(),
        }
    }
}

/// WorkContext - the primary object for managing persistent work across time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkContext {
    /// Unique identifier
    pub id: String,
    /// User identifier
    pub user_id: String,

    // Identity
    pub title: String,
    pub domain: WorkDomain,
    pub domain_profile_id: Option<String>,
    pub context_type: String,

    // Project/Conversation association
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
    pub parent_context_id: Option<String>,

    // Priority and timing
    pub priority: WorkPriority,
    pub due_at: Option<DateTime<Utc>>,

    // Intent
    pub goal: String,
    pub requirements: Vec<String>,
    pub constraints: Vec<String>,

    // Execution state
    pub status: WorkStatus,
    pub current_phase: WorkPhase,
    pub blocked_reason: Option<String>,

    // Planning
    pub plan: Option<ExecutionPlan>,
    pub approved_plan: Option<ExecutionPlan>,

    // Artifacts
    pub artifacts: Vec<Artifact>,

    // Memory
    pub memory_refs: Vec<String>,

    // Decisions
    pub decisions: Vec<DecisionRecord>,

    // Execution tracking
    pub flow_runs: Vec<String>,
    pub tool_trace: Vec<String>,
    pub execution_metadata: Vec<ExecutionRecord>,

    // Questions / blockers
    pub open_questions: Vec<String>,

    // Control
    pub autonomy_level: AutonomyLevel,
    pub approval_policy: ApprovalPolicy,

    // Summary and completion
    pub summary: Option<String>,
    pub completion_criteria: Vec<CompletionCriterion>,

    // Activity tracking
    pub last_activity_at: DateTime<Utc>,

    // Extensibility
    pub metadata: serde_json::Value,

    // Playbook tracking
    pub playbook_id: Option<String>,

    // Evaluation result
    pub evaluation_result: Option<serde_json::Value>,

    // Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkContext {
    /// Create a new WorkContext with minimal required fields
    pub fn new(
        id: String,
        user_id: String,
        title: String,
        domain: WorkDomain,
        goal: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            user_id,
            title,
            domain,
            domain_profile_id: None,
            context_type: "general".to_string(),
            project_id: None,
            conversation_id: None,
            parent_context_id: None,
            priority: WorkPriority::Medium,
            due_at: None,
            goal,
            requirements: Vec::new(),
            constraints: Vec::new(),
            status: WorkStatus::Draft,
            current_phase: WorkPhase::Intake,
            blocked_reason: None,
            plan: None,
            approved_plan: None,
            artifacts: Vec::new(),
            memory_refs: Vec::new(),
            decisions: Vec::new(),
            flow_runs: Vec::new(),
            tool_trace: Vec::new(),
            execution_metadata: Vec::new(),
            open_questions: Vec::new(),
            autonomy_level: AutonomyLevel::Chat,
            approval_policy: ApprovalPolicy::Auto,
            summary: None,
            completion_criteria: Vec::new(),
            last_activity_at: now,
            metadata: serde_json::Value::Null,
            playbook_id: None,
            evaluation_result: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update the last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity_at = Utc::now();
        self.updated_at = Utc::now();
    }

    /// Set the evaluation result
    pub fn set_evaluation_result(&mut self, evaluation_result: serde_json::Value) {
        self.evaluation_result = Some(evaluation_result);
        self.touch();
    }

    /// Check if the context is blocked
    pub fn is_blocked(&self) -> bool {
        self.status == WorkStatus::Blocked
    }

    /// Check if the context is complete
    pub fn is_complete(&self) -> bool {
        self.status == WorkStatus::Completed
    }

    /// Check if all completion criteria are satisfied
    pub fn is_completion_satisfied(&self) -> bool {
        self.completion_criteria.iter().all(|c| c.satisfied)
    }
}

/// WorkDomain - the domain of work
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkDomain {
    Software,
    Business,
    Marketing,
    Personal,
    Creative,
    Research,
    Operations,
    General,
    Custom(String),
}

impl Default for WorkDomain {
    fn default() -> Self {
        WorkDomain::General
    }
}

/// WorkStatus - the status of a WorkContext
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkStatus {
    Draft,
    Planning,
    AwaitingApproval,
    InProgress,
    Blocked,
    Completed,
    Failed,
    Archived,
}

impl Default for WorkStatus {
    fn default() -> Self {
        WorkStatus::Draft
    }
}

/// WorkPhase - the current phase of work
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkPhase {
    Intake,
    Planning,
    Execution,
    Review,
    Iteration,
    Finalization,
}

impl Default for WorkPhase {
    fn default() -> Self {
        WorkPhase::Intake
    }
}

/// AutonomyLevel - the autonomy level for execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutonomyLevel {
    Chat,
    Review,
    Autonomous,
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        AutonomyLevel::Chat
    }
}

/// ApprovalPolicy - when approval is required
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalPolicy {
    Auto,
    RequireForTools,
    RequireForSideEffects,
    RequireForUntrusted,
    ManualAll,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        ApprovalPolicy::Auto
    }
}

/// WorkPriority - priority level for work
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkPriority {
    Low,
    Medium,
    High,
    Urgent,
}

impl Default for WorkPriority {
    fn default() -> Self {
        WorkPriority::Medium
    }
}

/// CompletionCriterion - a criterion for work completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionCriterion {
    pub id: String,
    pub description: String,
    pub satisfied: bool,
}

impl CompletionCriterion {
    /// Create a new completion criterion
    pub fn new(id: String, description: String) -> Self {
        Self {
            id,
            description,
            satisfied: false,
        }
    }

    /// Mark the criterion as satisfied
    pub fn satisfy(&mut self) {
        self.satisfied = true;
    }

    /// Mark the criterion as unsatisfied
    pub fn unsatisfy(&mut self) {
        self.satisfied = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_work_context_creation() {
        let context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        assert_eq!(context.id, "ctx-1");
        assert_eq!(context.status, WorkStatus::Draft);
        assert_eq!(context.current_phase, WorkPhase::Intake);
        assert_eq!(context.priority, WorkPriority::Medium);
    }

    #[test]
    fn test_work_context_touch() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        let old_activity = context.last_activity_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        context.touch();

        assert!(context.last_activity_at > old_activity);
    }

    #[test]
    fn test_is_blocked() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        assert!(!context.is_blocked());

        context.status = WorkStatus::Blocked;
        assert!(context.is_blocked());
    }

    #[test]
    fn test_is_complete() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        assert!(!context.is_complete());

        context.status = WorkStatus::Completed;
        assert!(context.is_complete());
    }

    #[test]
    fn test_completion_criterion() {
        let mut criterion = CompletionCriterion::new("c1".to_string(), "Test passes".to_string());

        assert!(!criterion.satisfied);

        criterion.satisfy();
        assert!(criterion.satisfied);

        criterion.unsatisfy();
        assert!(!criterion.satisfied);
    }

    #[test]
    fn test_is_completion_satisfied() {
        let mut context = WorkContext::new(
            "ctx-1".to_string(),
            "user-1".to_string(),
            "Build API".to_string(),
            WorkDomain::Software,
            "Create a REST API".to_string(),
        );

        // No criteria - considered satisfied
        assert!(context.is_completion_satisfied());

        context.completion_criteria.push(CompletionCriterion::new(
            "c1".to_string(),
            "Test passes".to_string(),
        ));

        assert!(!context.is_completion_satisfied());

        context.completion_criteria[0].satisfy();
        assert!(context.is_completion_satisfied());
    }
}
