//! Domain profile system for WorkContext

use serde::{Deserialize, Serialize};

use super::types::ApprovalPolicy;

/// WorkDomainProfile - a profile for a specific work domain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkDomainProfile {
    pub id: String,
    pub name: String,
    pub parent_domain: Option<String>,
    pub default_flows: Vec<String>,
    pub artifact_kinds: Vec<String>,
    pub approval_defaults: ApprovalPolicy,
    pub lifecycle_template: LifecycleTemplate,
}

impl WorkDomainProfile {
    /// Create a new domain profile
    pub fn new(
        id: String,
        name: String,
        default_flows: Vec<String>,
        artifact_kinds: Vec<String>,
    ) -> Self {
        Self {
            id,
            name,
            parent_domain: None,
            default_flows,
            artifact_kinds,
            approval_defaults: ApprovalPolicy::Auto,
            lifecycle_template: LifecycleTemplate::default(),
        }
    }
}

/// LifecycleTemplate - a template for the lifecycle of work in a domain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleTemplate {
    pub phases: Vec<String>,
    pub transitions: Vec<(String, String)>,
}

impl LifecycleTemplate {
    /// Create a new lifecycle template
    pub fn new(phases: Vec<String>, transitions: Vec<(String, String)>) -> Self {
        Self {
            phases,
            transitions,
        }
    }
}

impl Default for LifecycleTemplate {
    fn default() -> Self {
        Self::new(
            vec![
                "Intake".to_string(),
                "Planning".to_string(),
                "Execution".to_string(),
                "Review".to_string(),
                "Finalization".to_string(),
            ],
            vec![
                ("Intake".to_string(), "Planning".to_string()),
                ("Planning".to_string(), "Execution".to_string()),
                ("Execution".to_string(), "Review".to_string()),
                ("Review".to_string(), "Finalization".to_string()),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_profile_creation() {
        let profile = WorkDomainProfile::new(
            "software".to_string(),
            "Software Development".to_string(),
            vec![
                "planning.flow.yaml".to_string(),
                "codegen.flow.yaml".to_string(),
            ],
            vec!["Code".to_string(), "Document".to_string()],
        );

        assert_eq!(profile.id, "software");
        assert_eq!(profile.default_flows.len(), 2);
    }

    #[test]
    fn test_lifecycle_template_default() {
        let template = LifecycleTemplate::default();

        assert_eq!(template.phases.len(), 5);
        assert_eq!(template.transitions.len(), 4);
    }

    #[test]
    fn test_lifecycle_template_custom() {
        let template = LifecycleTemplate::new(
            vec!["Start".to_string(), "End".to_string()],
            vec![("Start".to_string(), "End".to_string())],
        );

        assert_eq!(template.phases.len(), 2);
        assert_eq!(template.transitions.len(), 1);
    }
}
