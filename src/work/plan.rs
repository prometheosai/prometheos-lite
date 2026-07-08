//! Execution plan system for WorkContext

use serde::{Deserialize, Serialize};

/// ExecutionPlan - a plan for executing work
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
}

impl ExecutionPlan {
    /// Create a new execution plan
    pub fn new(steps: Vec<PlanStep>) -> Self {
        Self { steps }
    }

    /// Get the next pending step
    pub fn next_pending_step(&self) -> Option<&PlanStep> {
        self.steps.iter().find(|s| s.status == StepStatus::Pending)
    }

    /// Check if all steps are completed
    pub fn is_complete(&self) -> bool {
        self.steps.iter().all(|s| s.status == StepStatus::Completed)
    }
}

/// PlanStep - a single step in an execution plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub flow_ref: String,
    pub status: StepStatus,
}

impl PlanStep {
    /// Create a new plan step
    pub fn new(id: String, description: String, flow_ref: String) -> Self {
        Self {
            id,
            description,
            flow_ref,
            status: StepStatus::Pending,
        }
    }

    /// Mark the step as in progress
    pub fn start(&mut self) {
        self.status = StepStatus::InProgress;
    }

    /// Mark the step as completed
    pub fn complete(&mut self) {
        self.status = StepStatus::Completed;
    }

    /// Mark the step as failed
    pub fn fail(&mut self) {
        self.status = StepStatus::Failed;
    }
}

/// StepStatus - the status of a plan step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_plan_creation() {
        let plan = ExecutionPlan::new(vec![PlanStep::new(
            "step-1".to_string(),
            "Plan the work".to_string(),
            "planning.flow.yaml".to_string(),
        )]);

        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn test_next_pending_step() {
        let mut plan = ExecutionPlan::new(vec![
            PlanStep::new(
                "step-1".to_string(),
                "Plan".to_string(),
                "planning.flow.yaml".to_string(),
            ),
            PlanStep::new(
                "step-2".to_string(),
                "Execute".to_string(),
                "execute.flow.yaml".to_string(),
            ),
        ]);

        let next = plan.next_pending_step();
        assert!(next.is_some());
        assert_eq!(next.unwrap().id, "step-1");

        plan.steps[0].start();
        plan.steps[0].complete();

        let next = plan.next_pending_step();
        assert!(next.is_some());
        assert_eq!(next.unwrap().id, "step-2");
    }

    #[test]
    fn test_is_complete() {
        let mut plan = ExecutionPlan::new(vec![PlanStep::new(
            "step-1".to_string(),
            "Plan".to_string(),
            "planning.flow.yaml".to_string(),
        )]);

        assert!(!plan.is_complete());

        plan.steps[0].complete();
        assert!(plan.is_complete());
    }

    #[test]
    fn test_plan_step_transitions() {
        let mut step = PlanStep::new(
            "step-1".to_string(),
            "Plan".to_string(),
            "planning.flow.yaml".to_string(),
        );

        assert_eq!(step.status, StepStatus::Pending);

        step.start();
        assert_eq!(step.status, StepStatus::InProgress);

        step.complete();
        assert_eq!(step.status, StepStatus::Completed);

        step.fail();
        assert_eq!(step.status, StepStatus::Failed);
    }
}
