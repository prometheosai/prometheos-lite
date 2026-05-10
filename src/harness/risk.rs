use crate::harness::{
    review::{ReviewIssue, ReviewIssueType, ReviewSeverity},
    semantic_diff::{RiskLevel as SemanticRiskLevel, SemanticDiff},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    pub reasons: Vec<RiskReason>,
    pub requires_approval: bool,
    pub can_override: bool,
    pub override_conditions: Vec<String>,
    // P0-4 FIX: Add assessed field for completion evidence
    pub assessed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskReason {
    pub category: RiskCategory,
    pub description: String,
    pub severity: RiskSeverity,
    pub mitigation: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskCategory {
    Security,
    ApiBreaking,
    DatabaseBreaking,
    Dependency,
    Configuration,
    Logic,
    Performance,
    Compliance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct RiskEngine {
    pub approval_required_for: Vec<RiskCategory>,
    pub auto_reject_critical: bool,
    pub override_policy: OverridePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverridePolicy {
    pub allowed_categories: Vec<RiskCategory>,
    pub require_secondary_approval: bool,
    pub max_override_count: u32,
    pub audit_required: bool,
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskEngine {
    pub fn new() -> Self {
        Self {
            approval_required_for: vec![
                RiskCategory::Security,
                RiskCategory::ApiBreaking,
                RiskCategory::DatabaseBreaking,
                RiskCategory::Dependency,
                RiskCategory::Configuration,
            ],
            auto_reject_critical: true,
            override_policy: OverridePolicy::default(),
        }
    }

    pub fn assess(&self, diff: &SemanticDiff, issues: &[ReviewIssue]) -> RiskAssessment {
        let mut reasons = vec![];
        let mut level = RiskLevel::None;

        // Check for critical security issues
        let critical_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == ReviewSeverity::Critical)
            .collect();

        for issue in &critical_issues {
            reasons.push(RiskReason {
                category: RiskCategory::Security,
                description: format!("Critical issue: {}", issue.message),
                severity: RiskSeverity::Critical,
                mitigation: issue.suggestion.clone(),
            });
            level = RiskLevel::Critical;
        }

        // Check for high severity issues
        let high_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == ReviewSeverity::High)
            .collect();

        for issue in &high_issues {
            let category = self.categorize_issue(issue);
            reasons.push(RiskReason {
                category,
                description: format!("High severity: {}", issue.message),
                severity: RiskSeverity::High,
                mitigation: issue.suggestion.clone(),
            });
            if level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        // Assess API changes
        let breaking_api_changes: Vec<_> = diff.api_changes.iter().filter(|a| a.breaking).collect();

        for change in &breaking_api_changes {
            reasons.push(RiskReason {
                category: RiskCategory::ApiBreaking,
                description: format!(
                    "Breaking API change: {} in {}",
                    change.signature,
                    change.file.display()
                ),
                severity: RiskSeverity::High,
                mitigation: Some("Document breaking changes and version appropriately".to_string()),
            });
            if level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        // Check for non-breaking API changes (medium risk)
        let non_breaking_api = diff.api_changes.iter().filter(|a| !a.breaking).count();
        if non_breaking_api > 0 && level < RiskLevel::Medium {
            reasons.push(RiskReason {
                category: RiskCategory::ApiBreaking,
                description: format!("{} non-breaking API changes", non_breaking_api),
                severity: RiskSeverity::Medium,
                mitigation: Some("Ensure backward compatibility".to_string()),
            });
            level = RiskLevel::Medium;
        }

        // Assess database changes
        let breaking_db_changes: Vec<_> = diff
            .database_changes
            .iter()
            .filter(|d| d.breaking)
            .collect();

        for change in &breaking_db_changes {
            reasons.push(RiskReason {
                category: RiskCategory::DatabaseBreaking,
                description: format!(
                    "Breaking DB change: {:?} in {}",
                    change.change_type,
                    change.file.display()
                ),
                severity: RiskSeverity::High,
                mitigation: Some("Ensure migration strategy is in place".to_string()),
            });
            if level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        let migration_required = diff.database_changes.iter().any(|d| d.migration_required);
        if migration_required && !breaking_db_changes.is_empty() {
            reasons.push(RiskReason {
                category: RiskCategory::DatabaseBreaking,
                description: "Database migration required".to_string(),
                severity: RiskSeverity::Medium,
                mitigation: Some("Verify migration is reversible".to_string()),
            });
            if level < RiskLevel::Medium {
                level = RiskLevel::Medium;
            }
        }

        // Assess auth/security changes
        let high_risk_auth: Vec<_> = diff
            .auth_changes
            .iter()
            .filter(|a| {
                matches!(
                    a.risk_level,
                    SemanticRiskLevel::High | SemanticRiskLevel::Critical
                )
            })
            .collect();

        for change in &high_risk_auth {
            reasons.push(RiskReason {
                category: RiskCategory::Security,
                description: format!("Auth change with high risk: {:?}", change.change_type),
                severity: RiskSeverity::High,
                mitigation: Some("Review security implications".to_string()),
            });
            if level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        // Assess dependency changes
        let high_risk_deps: Vec<_> = diff
            .dependency_changes
            .iter()
            .filter(|d| {
                matches!(
                    d.risk_level,
                    SemanticRiskLevel::High | SemanticRiskLevel::Critical
                )
            })
            .collect();

        for change in &high_risk_deps {
            reasons.push(RiskReason {
                category: RiskCategory::Dependency,
                description: format!(
                    "High-risk dependency change: {} {:?}",
                    change.package_name, change.change_type
                ),
                severity: RiskSeverity::Medium,
                mitigation: Some("Review dependency for security vulnerabilities".to_string()),
            });
            if level < RiskLevel::Medium {
                level = RiskLevel::Medium;
            }
        }

        // Assess config changes
        let prod_config_changes: Vec<_> = diff
            .config_changes
            .iter()
            .filter(|c| {
                matches!(
                    c.environment,
                    crate::harness::semantic_diff::ConfigEnvironment::Production
                )
            })
            .collect();

        for change in &prod_config_changes {
            reasons.push(RiskReason {
                category: RiskCategory::Configuration,
                description: format!(
                    "Production config change: {} {:?}",
                    change.config_key, change.change_type
                ),
                severity: RiskSeverity::High,
                mitigation: Some("Verify in staging before production".to_string()),
            });
            if level < RiskLevel::High {
                level = RiskLevel::High;
            }
        }

        // Determine if approval is required
        let requires_approval = level >= RiskLevel::High
            || reasons
                .iter()
                .any(|r| self.approval_required_for.contains(&r.category));

        // Determine if override is possible
        let can_override = level < RiskLevel::Critical || !self.auto_reject_critical;
        let override_conditions = self.generate_override_conditions(&reasons);

        RiskAssessment {
            level,
            reasons,
            requires_approval,
            can_override,
            override_conditions,
            // P0-4 FIX: Add assessed field for completion evidence
            assessed: true,
        }
    }

    fn categorize_issue(&self, issue: &ReviewIssue) -> RiskCategory {
        match issue.issue_type {
            ReviewIssueType::Security => RiskCategory::Security,
            ReviewIssueType::Bug => RiskCategory::Logic,
            ReviewIssueType::Performance => RiskCategory::Performance,
            ReviewIssueType::ApiChange => RiskCategory::ApiBreaking,
            ReviewIssueType::DependencyChange => RiskCategory::Dependency,
            _ => RiskCategory::Logic,
        }
    }

    fn generate_override_conditions(&self, reasons: &[RiskReason]) -> Vec<String> {
        let mut conditions = vec![];

        for reason in reasons {
            if self
                .override_policy
                .allowed_categories
                .contains(&reason.category)
            {
                conditions.push(format!(
                    "Override for {:?}: {}",
                    reason.category, reason.description
                ));
            }
        }

        if self.override_policy.require_secondary_approval {
            conditions.push("Secondary approval required".to_string());
        }

        if self.override_policy.audit_required {
            conditions.push("Override will be audited".to_string());
        }

        conditions
    }

    pub fn request_override(
        &self,
        assessment: &RiskAssessment,
        justification: &str,
    ) -> OverrideResult {
        if !assessment.can_override {
            return OverrideResult::Rejected("Critical risks cannot be overridden".to_string());
        }

        if justification.len() < 20 {
            return OverrideResult::Rejected("Justification too brief (min 20 chars)".to_string());
        }

        OverrideResult::Approved {
            conditions: assessment.override_conditions.clone(),
            audit_log: format!("Override approved with justification: {}", justification),
        }
    }
}

impl Default for OverridePolicy {
    fn default() -> Self {
        Self {
            allowed_categories: vec![
                RiskCategory::ApiBreaking,
                RiskCategory::Configuration,
                RiskCategory::Dependency,
            ],
            require_secondary_approval: true,
            max_override_count: 3,
            audit_required: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OverrideResult {
    Approved {
        conditions: Vec<String>,
        audit_log: String,
    },
    Rejected(String),
}

pub fn assess_risk(diff: &SemanticDiff, issues: &[ReviewIssue]) -> RiskAssessment {
    let engine = RiskEngine::new();
    engine.assess(diff, issues)
}

pub fn format_risk_assessment(assessment: &RiskAssessment) -> String {
    let mut output = String::new();

    output.push_str("Risk Assessment\n");
    output.push_str("===============\n\n");

    output.push_str(&format!("Overall Risk Level: {:?}\n", assessment.level));
    output.push_str(&format!(
        "Requires Approval: {}\n",
        assessment.requires_approval
    ));
    output.push_str(&format!("Can Override: {}\n\n", assessment.can_override));

    if !assessment.reasons.is_empty() {
        output.push_str("Risk Reasons:\n");
        for (i, reason) in assessment.reasons.iter().enumerate() {
            output.push_str(&format!(
                "\n{}. [{:?}] {:?}\n",
                i + 1,
                reason.severity,
                reason.category
            ));
            output.push_str(&format!("   Description: {}\n", reason.description));
            if let Some(mitigation) = &reason.mitigation {
                output.push_str(&format!("   Mitigation: {}\n", mitigation));
            }
        }
    }

    if !assessment.override_conditions.is_empty() {
        output.push_str("\nOverride Conditions:\n");
        for condition in &assessment.override_conditions {
            output.push_str(&format!("  - {}\n", condition));
        }
    }

    output
}
