//! WorkContext templates for common work patterns

use crate::work::WorkContext;
use crate::work::types::{
    ApprovalPolicy, AutonomyLevel, CompletionCriterion, WorkDomain, WorkPhase, WorkPriority,
    WorkStatus,
};
use uuid::Uuid;

/// WorkContext template for software development tasks
pub fn software_development_template(title: String, goal: String) -> WorkContext {
    WorkContext {
        id: Uuid::new_v4().to_string(),
        user_id: "template-user".to_string(),
        title,
        domain: WorkDomain::Software,
        domain_profile_id: None,
        context_type: "feature".to_string(),
        project_id: None,
        conversation_id: None,
        parent_context_id: None,
        priority: WorkPriority::High,
        due_at: None,
        goal,
        requirements: vec!["Code should be well-documented and tested".to_string()],
        constraints: vec!["Follow existing code style and patterns".to_string()],
        status: WorkStatus::Draft,
        current_phase: WorkPhase::Intake,
        blocked_reason: None,
        plan: None,
        approved_plan: None,
        artifacts: vec![],
        memory_refs: vec![],
        decisions: vec![],
        flow_runs: vec![],
        tool_trace: vec![],
        execution_metadata: vec![],
        open_questions: vec![],
        autonomy_level: AutonomyLevel::Review,
        approval_policy: ApprovalPolicy::RequireForSideEffects,
        summary: None,
        completion_criteria: vec![
            CompletionCriterion::new(
                "code-generated".to_string(),
                "Code has been generated".to_string(),
            ),
            CompletionCriterion::new(
                "tests-written".to_string(),
                "Tests have been written".to_string(),
            ),
            CompletionCriterion::new(
                "code-reviewed".to_string(),
                "Code has been reviewed".to_string(),
            ),
        ],
        last_activity_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        playbook_id: None,
        evaluation_result: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// WorkContext template for research tasks
pub fn research_template(title: String, goal: String) -> WorkContext {
    WorkContext {
        id: Uuid::new_v4().to_string(),
        user_id: "template-user".to_string(),
        title,
        domain: WorkDomain::Research,
        domain_profile_id: None,
        context_type: "investigation".to_string(),
        project_id: None,
        conversation_id: None,
        parent_context_id: None,
        priority: WorkPriority::Medium,
        due_at: None,
        goal,
        requirements: vec!["Provide citations and sources".to_string()],
        constraints: vec!["Focus on credible sources".to_string()],
        status: WorkStatus::Draft,
        current_phase: WorkPhase::Intake,
        blocked_reason: None,
        plan: None,
        approved_plan: None,
        artifacts: vec![],
        memory_refs: vec![],
        decisions: vec![],
        flow_runs: vec![],
        tool_trace: vec![],
        execution_metadata: vec![],
        open_questions: vec![],
        autonomy_level: AutonomyLevel::Autonomous,
        approval_policy: ApprovalPolicy::Auto,
        summary: None,
        completion_criteria: vec![
            CompletionCriterion::new(
                "research-complete".to_string(),
                "Research is complete".to_string(),
            ),
            CompletionCriterion::new(
                "document-created".to_string(),
                "Document has been created".to_string(),
            ),
        ],
        last_activity_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        playbook_id: None,
        evaluation_result: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// WorkContext template for planning tasks
pub fn planning_template(title: String, goal: String) -> WorkContext {
    WorkContext {
        id: Uuid::new_v4().to_string(),
        user_id: "template-user".to_string(),
        title,
        domain: WorkDomain::Software,
        domain_profile_id: None,
        context_type: "planning".to_string(),
        project_id: None,
        conversation_id: None,
        parent_context_id: None,
        priority: WorkPriority::High,
        due_at: None,
        goal,
        requirements: vec!["Include timeline and resource estimates".to_string()],
        constraints: vec!["Align with project goals".to_string()],
        status: WorkStatus::Draft,
        current_phase: WorkPhase::Intake,
        blocked_reason: None,
        plan: None,
        approved_plan: None,
        artifacts: vec![],
        memory_refs: vec![],
        decisions: vec![],
        flow_runs: vec![],
        tool_trace: vec![],
        execution_metadata: vec![],
        open_questions: vec![],
        autonomy_level: AutonomyLevel::Review,
        approval_policy: ApprovalPolicy::ManualAll,
        summary: None,
        completion_criteria: vec![
            CompletionCriterion::new(
                "plan-approved".to_string(),
                "Plan has been approved".to_string(),
            ),
            CompletionCriterion::new(
                "document-created".to_string(),
                "Document has been created".to_string(),
            ),
        ],
        last_activity_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        playbook_id: None,
        evaluation_result: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// WorkContext template for bug fix tasks
pub fn bug_fix_template(title: String, goal: String) -> WorkContext {
    WorkContext {
        id: Uuid::new_v4().to_string(),
        user_id: "template-user".to_string(),
        title,
        domain: WorkDomain::Software,
        domain_profile_id: None,
        context_type: "bugfix".to_string(),
        project_id: None,
        conversation_id: None,
        parent_context_id: None,
        priority: WorkPriority::Urgent,
        due_at: None,
        goal,
        requirements: vec!["Include reproduction steps".to_string()],
        constraints: vec!["Minimal changes only".to_string()],
        status: WorkStatus::Draft,
        current_phase: WorkPhase::Intake,
        blocked_reason: None,
        plan: None,
        approved_plan: None,
        artifacts: vec![],
        memory_refs: vec![],
        decisions: vec![],
        flow_runs: vec![],
        tool_trace: vec![],
        execution_metadata: vec![],
        open_questions: vec![],
        autonomy_level: AutonomyLevel::Review,
        approval_policy: ApprovalPolicy::RequireForSideEffects,
        summary: None,
        completion_criteria: vec![
            CompletionCriterion::new(
                "bug-reproduced".to_string(),
                "Bug has been reproduced".to_string(),
            ),
            CompletionCriterion::new(
                "fix-implemented".to_string(),
                "Fix has been implemented".to_string(),
            ),
            CompletionCriterion::new(
                "tests-written".to_string(),
                "Tests have been written".to_string(),
            ),
        ],
        last_activity_at: chrono::Utc::now(),
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        playbook_id: None,
        evaluation_result: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_software_development_template() {
        let context =
            software_development_template("Build API".to_string(), "Create a REST API".to_string());
        assert_eq!(context.domain, WorkDomain::Software);
        assert_eq!(context.priority, WorkPriority::High);
        assert_eq!(context.autonomy_level, AutonomyLevel::Review);
        assert_eq!(context.completion_criteria.len(), 3);
    }

    #[test]
    fn test_research_template() {
        let context = research_template(
            "Research AI".to_string(),
            "Investigate AI techniques".to_string(),
        );
        assert_eq!(context.domain, WorkDomain::Research);
        assert_eq!(context.autonomy_level, AutonomyLevel::Autonomous);
        assert_eq!(context.approval_policy, ApprovalPolicy::Auto);
    }

    #[test]
    fn test_planning_template() {
        let context = planning_template(
            "Project Plan".to_string(),
            "Create project roadmap".to_string(),
        );
        assert_eq!(context.context_type, "planning");
        assert_eq!(context.approval_policy, ApprovalPolicy::ManualAll);
    }

    #[test]
    fn test_bug_fix_template() {
        let context = bug_fix_template("Fix bug".to_string(), "Fix critical issue".to_string());
        assert_eq!(context.priority, WorkPriority::Urgent);
        assert_eq!(context.context_type, "bugfix");
    }
}
