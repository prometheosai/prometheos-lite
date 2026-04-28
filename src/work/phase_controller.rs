//! PhaseController - manages WorkContext lifecycle phase transitions

use anyhow::Result;

use super::types::{WorkContext, WorkPhase, WorkStatus};
use super::domain::WorkDomainProfile;

/// PhaseController - manages lifecycle phase transitions
pub struct PhaseController;

impl PhaseController {
    /// Determine the next phase based on current phase and context state
    pub fn next_phase(context: &WorkContext) -> Option<WorkPhase> {
        match context.current_phase {
            WorkPhase::Intake => Some(WorkPhase::Planning),
            WorkPhase::Planning => {
                if context.is_complete() {
                    Some(WorkPhase::Finalization)
                } else {
                    Some(WorkPhase::Execution)
                }
            }
            WorkPhase::Execution => {
                if context.is_complete() {
                    Some(WorkPhase::Finalization)
                } else {
                    Some(WorkPhase::Review)
                }
            }
            WorkPhase::Review => {
                if context.is_complete() {
                    Some(WorkPhase::Finalization)
                } else {
                    Some(WorkPhase::Iteration)
                }
            }
            WorkPhase::Iteration => Some(WorkPhase::Planning),
            WorkPhase::Finalization => None, // End of lifecycle
        }
    }

    /// Check if a phase transition is valid
    pub fn can_transition(from: WorkPhase, to: WorkPhase) -> bool {
        match (from, to) {
            (WorkPhase::Intake, WorkPhase::Planning) => true,
            (WorkPhase::Planning, WorkPhase::Execution) => true,
            (WorkPhase::Planning, WorkPhase::Finalization) => true,
            (WorkPhase::Execution, WorkPhase::Review) => true,
            (WorkPhase::Execution, WorkPhase::Finalization) => true,
            (WorkPhase::Review, WorkPhase::Iteration) => true,
            (WorkPhase::Review, WorkPhase::Finalization) => true,
            (WorkPhase::Iteration, WorkPhase::Planning) => true,
            _ => false,
        }
    }

    /// Get the recommended flow for a given phase
    /// If domain profile is provided, use its default flows; otherwise use generic flows
    pub fn flow_for_phase(phase: WorkPhase, domain_profile: Option<&WorkDomainProfile>) -> String {
        if let Some(profile) = domain_profile {
            // Try to get flow from domain profile's default_flows
            let phase_key = match phase {
                WorkPhase::Intake => "intake",
                WorkPhase::Planning => "planning",
                WorkPhase::Execution => "execution",
                WorkPhase::Review => "review",
                WorkPhase::Iteration => "planning",
                WorkPhase::Finalization => "finalization",
            };

            // Look for a flow matching the phase in the domain profile
            for flow in &profile.default_flows {
                if flow.contains(phase_key) {
                    return flow.clone();
                }
            }
        }

        // Fallback to generic flows
        match phase {
            WorkPhase::Intake => "intake.flow.yaml".to_string(),
            WorkPhase::Planning => "planning.flow.yaml".to_string(),
            WorkPhase::Execution => "execution.flow.yaml".to_string(),
            WorkPhase::Review => "review.flow.yaml".to_string(),
            WorkPhase::Iteration => "planning.flow.yaml".to_string(),
            WorkPhase::Finalization => "finalization.flow.yaml".to_string(),
        }
    }

    /// Check if context should require approval before phase transition
    pub fn requires_approval(context: &WorkContext, next_phase: WorkPhase) -> bool {
        // Require approval before moving from Planning to Execution
        if context.current_phase == WorkPhase::Planning && next_phase == WorkPhase::Execution {
            // Check if plan has been approved
            if context.approved_plan.is_none() {
                return true; // Require approval if no approved plan exists
            }
            // Even with approved plan, respect approval policy
            if context.approval_policy == super::types::ApprovalPolicy::ManualAll {
                return true;
            }
        }
        // Require approval before Finalization
        if next_phase == WorkPhase::Finalization {
            return true;
        }
        false
    }

    /// Update status based on phase
    pub fn status_for_phase(phase: WorkPhase) -> WorkStatus {
        match phase {
            WorkPhase::Intake => WorkStatus::Draft,
            WorkPhase::Planning => WorkStatus::InProgress,
            WorkPhase::Execution => WorkStatus::InProgress,
            WorkPhase::Review => WorkStatus::AwaitingApproval,
            WorkPhase::Iteration => WorkStatus::InProgress,
            WorkPhase::Finalization => WorkStatus::Completed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_phase_intake() {
        let context = WorkContext::new(
            "test-id".to_string(),
            "user-1".to_string(),
            "Test".to_string(),
            super::super::types::WorkDomain::Software,
            "Test goal".to_string(),
        );
        assert_eq!(PhaseController::next_phase(&context), Some(WorkPhase::Planning));
    }

    #[test]
    fn test_can_transition_valid() {
        assert!(PhaseController::can_transition(WorkPhase::Intake, WorkPhase::Planning));
        assert!(PhaseController::can_transition(WorkPhase::Planning, WorkPhase::Execution));
    }

    #[test]
    fn test_can_transition_invalid() {
        assert!(!PhaseController::can_transition(WorkPhase::Finalization, WorkPhase::Planning));
        assert!(!PhaseController::can_transition(WorkPhase::Execution, WorkPhase::Intake));
    }

    #[test]
    fn test_flow_for_phase() {
        assert_eq!(PhaseController::flow_for_phase(WorkPhase::Planning, None), "planning.flow.yaml");
        assert_eq!(PhaseController::flow_for_phase(WorkPhase::Execution, None), "execution.flow.yaml");
    }

    #[test]
    fn test_requires_approval() {
        let mut context = WorkContext::new(
            "test-id".to_string(),
            "user-1".to_string(),
            "Test".to_string(),
            super::super::types::WorkDomain::Software,
            "Test goal".to_string(),
        );
        context.current_phase = WorkPhase::Planning;
        assert!(PhaseController::requires_approval(&context, WorkPhase::Execution));
    }

    #[test]
    fn test_status_for_phase() {
        assert_eq!(PhaseController::status_for_phase(WorkPhase::Intake), WorkStatus::Draft);
        assert_eq!(PhaseController::status_for_phase(WorkPhase::Finalization), WorkStatus::Completed);
    }
}
