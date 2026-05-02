//! PhaseController - manages WorkContext lifecycle phase transitions

use anyhow::Result;

use super::domain::WorkDomainProfile;
use super::playbook::FlowPreference;
use super::types::{WorkContext, WorkPhase, WorkStatus};
use rand::Rng;

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
                WorkPhase::Intake => "planning",
                WorkPhase::Planning => "planning",
                WorkPhase::Execution => "codegen",
                WorkPhase::Review => "approval",
                WorkPhase::Iteration => "planning",
                WorkPhase::Finalization => "approval",
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
            WorkPhase::Intake => "planning.flow.yaml".to_string(),
            WorkPhase::Planning => "codegen.flow.yaml".to_string(),
            WorkPhase::Execution => "codegen.flow.yaml".to_string(),
            WorkPhase::Review => "approval.flow.yaml".to_string(),
            WorkPhase::Iteration => "planning.flow.yaml".to_string(),
            WorkPhase::Finalization => "approval.flow.yaml".to_string(),
        }
    }

    /// Select flow from playbook preferred_flows using weighted random selection with exploration factor
    /// exploration_factor: 0.0 = always pick highest weight, 1.0 = pure random
    pub fn weighted_flow_selection(
        phase: WorkPhase,
        preferred_flows: &[FlowPreference],
        exploration_factor: f32,
    ) -> String {
        if preferred_flows.is_empty() {
            // Fallback to default flow for phase
            return Self::flow_for_phase(phase, None);
        }

        // Filter flows that match the current phase
        let phase_key = match phase {
            WorkPhase::Intake => "planning",
            WorkPhase::Planning => "planning",
            WorkPhase::Execution => "codegen",
            WorkPhase::Review => "approval",
            WorkPhase::Iteration => "planning",
            WorkPhase::Finalization => "approval",
        };

        let matching_flows: Vec<&FlowPreference> = preferred_flows
            .iter()
            .filter(|fp| fp.flow_id.contains(phase_key))
            .collect();

        if matching_flows.is_empty() {
            // No matching flows, fallback to default
            return Self::flow_for_phase(phase, None);
        }

        // Weighted random selection with exploration
        let mut rng = rand::thread_rng();
        let random_val: f32 = rng.gen_range(0.0..1.0);

        if random_val < exploration_factor {
            // Exploration: pick random flow
            let idx = rng.gen_range(0..matching_flows.len());
            return matching_flows[idx].flow_id.clone();
        }

        // Exploitation: pick highest weight
        let best_flow = matching_flows
            .iter()
            .max_by(|a, b| {
                a.weight
                    .partial_cmp(&b.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        best_flow.flow_id.clone()
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
        assert_eq!(
            PhaseController::next_phase(&context),
            Some(WorkPhase::Planning)
        );
    }

    #[test]
    fn test_can_transition_valid() {
        assert!(PhaseController::can_transition(
            WorkPhase::Intake,
            WorkPhase::Planning
        ));
        assert!(PhaseController::can_transition(
            WorkPhase::Planning,
            WorkPhase::Execution
        ));
    }

    #[test]
    fn test_can_transition_invalid() {
        assert!(!PhaseController::can_transition(
            WorkPhase::Finalization,
            WorkPhase::Planning
        ));
        assert!(!PhaseController::can_transition(
            WorkPhase::Execution,
            WorkPhase::Intake
        ));
    }

    #[test]
    fn test_flow_for_phase() {
        assert_eq!(
            PhaseController::flow_for_phase(WorkPhase::Planning, None),
            "codegen.flow.yaml"
        );
        assert_eq!(
            PhaseController::flow_for_phase(WorkPhase::Execution, None),
            "codegen.flow.yaml"
        );
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
        assert!(PhaseController::requires_approval(
            &context,
            WorkPhase::Execution
        ));
    }

    #[test]
    fn test_status_for_phase() {
        assert_eq!(
            PhaseController::status_for_phase(WorkPhase::Intake),
            WorkStatus::Draft
        );
        assert_eq!(
            PhaseController::status_for_phase(WorkPhase::Finalization),
            WorkStatus::Completed
        );
    }
}
