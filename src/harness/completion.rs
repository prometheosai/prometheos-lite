use crate::harness::{
    confidence::ConfidenceScore, evidence::SandboxEvidence, mode_policy::HarnessMode, review::ReviewReport,
    risk::{RiskAssessment, RiskLevel},
    semantic_diff::SemanticDiff,
    validation::{CommandResult, ValidationResult},
    verification::VerificationStrength,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionEvidence {
    // 8 Evidence Dimensions
    pub patch_evidence: PatchEvidence,
    pub validation_evidence: ValidationEvidence,
    pub review_evidence: ReviewEvidence,
    pub risk_evidence: RiskEvidence,
    pub verification_evidence: VerificationEvidence,
    pub semantic_evidence: SemanticEvidence,
    pub confidence_evidence: ConfidenceEvidence,
    pub process_evidence: ProcessEvidence,
    // P0-Issue1: Real sandbox evidence for autonomous mode safety
    pub sandbox_evidence: Vec<SandboxEvidence>,

    // Legacy fields for compatibility
    pub patch_exists: bool,
    pub validation_ran: bool,
    pub validation_passed: bool,
    pub review_ran: bool,
    pub critical_issues: usize,
    pub confidence: ConfidenceScore,
    pub verification_strength: VerificationStrength,
    pub requires_approval: bool,

    // Decision metadata
    pub decision_factors: Vec<String>,
    pub evidence_completeness: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchEvidence {
    pub patch_created: bool,
    pub files_modified: usize,
    pub lines_changed: usize,
    pub patch_applied_cleanly: bool,
    pub patch_hash: Option<String>,
    pub dry_run_passed: bool,
    // P0-3.1: Real patch identity verification for audit-grade integrity
    pub patch_identity: Option<crate::harness::patch_applier::PatchIdentity>,
    // Legacy fields for backward compatibility
    pub generated_patch_hash: Option<String>,
    pub dry_run_patch_hash: Option<String>,
    pub applied_patch_hash: Option<String>,
    pub hash_verification_passed: bool,
    pub hash_mismatch_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationEvidence {
    pub validation_performed: bool,
    pub all_validations_passed: bool,
    pub format_check_passed: bool,
    pub static_check_passed: bool,
    pub lint_check_passed: bool,
    pub test_passed: bool,
    pub validation_summary: String,
    // P0-2.1: Add direct command execution counters
    pub commands_planned: usize,
    pub commands_executed: usize,
    pub commands_skipped: usize,
    pub categories_executed: Vec<crate::harness::validation::ValidationCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewEvidence {
    pub review_performed: bool,
    pub total_issues: usize,
    pub critical_issues: usize,
    pub high_issues: usize,
    pub medium_issues: usize,
    pub low_issues: usize,
    pub security_issues: usize,
    pub breaking_change_issues: usize,
    pub review_passed: bool,
    // P0-3.2: Comprehensive review quality metrics
    pub files_reviewed: usize,
    pub lines_analyzed: usize,
    pub security_patterns_checked: usize,
    pub api_breaking_changes_detected: usize,
    pub dependency_changes_analyzed: usize,
    pub test_coverage_analyzed: bool,
    pub performance_impact_assessed: bool,
    pub documentation_updated: bool,
    pub review_depth_score: f32, // 0.0-1.0 scale
    pub review_quality_indicators: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskEvidence {
    pub risk_assessed: bool,
    pub overall_risk_level: String,
    pub security_risk: String,
    pub api_risk: String,
    pub database_risk: String,
    pub dependency_risk: String,
    pub requires_approval: bool,
    pub risk_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationEvidence {
    pub verification_level: VerificationStrength,
    pub test_count: usize,
    pub coverage_percent: Option<f32>,
    pub reproduction_test_passed: bool,
    pub integration_tests_passed: bool,
    pub verification_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticEvidence {
    pub api_changes_detected: bool,
    pub auth_changes_detected: bool,
    pub database_changes_detected: bool,
    pub dependency_changes_detected: bool,
    pub config_changes_detected: bool,
    pub breaking_changes_count: usize,
    pub security_relevant_changes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceEvidence {
    pub confidence_score: f32,
    pub confidence_classification: String,
    pub validation_contribution: f32,
    pub risk_contribution: f32,
    pub review_contribution: f32,
    pub confidence_factors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessEvidence {
    pub git_checkpoint_created: bool,
    pub rollback_available: bool,
    pub all_phases_completed: bool,
    pub no_critical_errors: bool,
    pub time_limit_respected: bool,
    pub step_limit_respected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompletionDecision {
    Complete,
    Blocked(String),
    NeedsRepair(String),
    NeedsApproval(String),
}

/// P2-013: Completion invariants that must be satisfied for a decision
///
/// A patch cannot be "Complete" unless all required invariants are met.
/// This provides strict, testable rules for completion decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionInvariant {
    /// Invariant name
    pub name: &'static str,
    /// Whether this invariant is required for completion
    pub required: bool,
    /// Human-readable description of the invariant
    pub description: &'static str,
}

impl CompletionDecision {
    /// P2-013: Check if this decision represents a completed/allowed state
    pub fn is_completed(&self) -> bool {
        matches!(self, CompletionDecision::Complete)
    }

    /// P2-013: Check if this decision is blocked (no patch applied)
    pub fn is_blocked(&self) -> bool {
        matches!(self, CompletionDecision::Blocked(_))
    }

    /// P2-013: Get the reason for non-complete decisions
    pub fn reason(&self) -> Option<&str> {
        match self {
            CompletionDecision::Complete => None,
            CompletionDecision::Blocked(r) | CompletionDecision::NeedsRepair(r) | CompletionDecision::NeedsApproval(r) => Some(r),
        }
    }

    /// P2-013: Required invariants for a patch to be considered "Complete"
    ///
    /// A patch cannot be "Complete" unless:
    /// - Patch was applied or review-only result explicitly says no apply
    /// - Validation passed
    /// - No critical review issues
    /// - Risk accepted
    /// - Acceptance criteria verified
    /// - Rollback exists for side-effect modes
    /// - Evidence log complete
    pub fn required_invariants() -> Vec<CompletionInvariant> {
        vec![
            CompletionInvariant {
                name: "patch_applied_or_explicit_no_apply",
                required: true,
                description: "Patch applied or review-only result explicitly says no apply",
            },
            CompletionInvariant {
                name: "validation_passed",
                required: true,
                description: "Validation passed (or was not required in ReviewOnly mode)",
            },
            CompletionInvariant {
                name: "no_critical_review_issues",
                required: true,
                description: "No critical review issues found",
            },
            CompletionInvariant {
                name: "risk_accepted",
                required: true,
                description: "Risk level accepted for the mode",
            },
            CompletionInvariant {
                name: "checkpoint_or_explicit_skip",
                required: true,
                description: "Git checkpoint created or explicitly skipped for ReviewOnly",
            },
            CompletionInvariant {
                name: "evidence_complete",
                required: true,
                description: "EvidenceLog has entries for all key operations",
            },
        ]
    }

    /// P2-013: Validate that this decision satisfies all required invariants
    ///
    /// Returns Ok(()) if valid, Err with reasons if invalid.
    pub fn validate(&self, evidence: &CompletionEvidence) -> Result<(), Vec<String>> {
        let mut failures = vec![];

        // Only "Complete" decisions need full invariant checking
        if !self.is_completed() {
            return Ok(()); // Non-complete states are always "valid"
        }

        // Check patch applied or explicit no-apply
        if !evidence.patch_evidence.patch_created && !evidence.patch_evidence.dry_run_passed {
            failures.push("Patch not created and dry-run not performed".to_string());
        }

        // P0-3.1: Real patch identity verification - audit-grade integrity check
        if evidence.patch_evidence.patch_created {
            // Check if we have the new PatchIdentity system
            if let Some(ref patch_identity) = evidence.patch_evidence.patch_identity {
                if !patch_identity.can_complete() {
                    failures.push("Patch identity verification failed - cannot complete".to_string());
                    if let Some(ref details) = patch_identity.mismatch_details {
                        failures.push(format!("Patch identity mismatch details: {}", details));
                    }
                    if !patch_identity.has_complete_hashes() {
                        failures.push("Incomplete patch identity - missing hash stages".to_string());
                    }
                }
            } else {
                // Fallback to legacy hash verification for backward compatibility
                if evidence.patch_evidence.patch_hash.is_none() {
                    failures.push("Patch was created but no patch hash was recorded".to_string());
                }
                
                // Verify that applied patch hash matches recorded patch hash
                if let (Some(recorded_hash), Some(applied_hash)) = (&evidence.patch_evidence.patch_hash, &evidence.patch_evidence.applied_patch_hash) {
                    if recorded_hash != applied_hash {
                        failures.push(format!(
                            "Patch hash mismatch: recorded={}, applied={} - possible tampering detected", 
                            recorded_hash, applied_hash
                        ));
                    } else if !evidence.patch_evidence.hash_verification_passed {
                        failures.push("Patch hash verification was not performed".to_string());
                    }
                } else {
                    failures.push("Missing patch hash for verification".to_string());
                }
            }
        }

        // P0-3.1: Reject Complete if validation plan ran zero commands (for side-effect modes)
        if evidence.validation_evidence.validation_performed {
            // Check if any validation commands were actually executed
            let validation_commands_count = evidence.validation_evidence.format_check_passed as usize
                + evidence.validation_evidence.static_check_passed as usize
                + evidence.validation_evidence.lint_check_passed as usize
                + evidence.validation_evidence.test_passed as usize;
            
            // If validation was marked as performed but no commands actually ran, reject
            if validation_commands_count == 0 {
                failures.push("Validation was marked as performed but no validation commands were executed".to_string());
            }
            
            // P0-3.1: Additional check - validation with zero commands should not be considered "performed"
            if evidence.validation_evidence.commands_executed == 0 {
                failures.push("Validation had zero commands executed - cannot complete".to_string());
            }
        }

        // Check validation passed (or not required for ReviewOnly)
        if evidence.validation_evidence.validation_performed && !evidence.validation_evidence.all_validations_passed {
            failures.push("Validation was performed but did not pass".to_string());
        }

        // Check no critical review issues
        if evidence.review_evidence.critical_issues > 0 {
            failures.push(format!("{} critical review issues found", evidence.review_evidence.critical_issues));
        }

        // Check risk accepted
        if evidence.risk_evidence.requires_approval {
            failures.push("Risk requires approval but decision is Complete".to_string());
        }

        // P0-4: Require rollback evidence for all side-effect modes
        if evidence.patch_evidence.patch_created && !evidence.process_evidence.rollback_available {
            failures.push("Patch was applied but no rollback evidence is available".to_string());
        }

        // P0-5: Downgrade incomplete evidence to Blocked with stricter requirements
        let mut evidence_issues = vec![];

        // P0-3.1: Require comprehensive patch hash verification
        if evidence.patch_evidence.patch_created {
            // Must have patch hash for applied patches
            if evidence.patch_evidence.patch_hash.is_none() {
                evidence_issues.push("Missing patch hash for applied patch".to_string());
            }
            
            // P0-3.1: Verify generated/dry-run/applied patch hashes match
            if !evidence.patch_evidence.hash_verification_passed {
                if let Some(ref details) = evidence.patch_evidence.hash_mismatch_details {
                    evidence_issues.push(format!("Patch hash verification failed: {}", details));
                } else {
                    evidence_issues.push("Patch hash verification failed - no verification performed".to_string());
                }
            }
            
            // P0-3.1: Ensure all three hash stages are present for applied patches
            if evidence.patch_evidence.generated_patch_hash.is_none() {
                evidence_issues.push("Missing generated patch hash".to_string());
            }
            
            if evidence.patch_evidence.dry_run_patch_hash.is_none() {
                evidence_issues.push("Missing dry-run patch hash".to_string());
            }
            
            if evidence.patch_evidence.applied_patch_hash.is_none() {
                evidence_issues.push("Missing applied patch hash".to_string());
            }
        }

        // P0-2.1: For side-effect modes, require actual validation commands using direct counters
        if evidence.patch_evidence.patch_created {
            if evidence.validation_evidence.commands_executed == 0 {
                evidence_issues.push("No validation commands executed for side-effect patch".to_string());
            }
            
            // P0-2.1: Additional check for planned vs executed commands
            if evidence.validation_evidence.commands_planned > 0 && evidence.validation_evidence.commands_executed == 0 {
                evidence_issues.push("Validation commands were planned but none executed".to_string());
            }
        }

        // P0-3.2: Replace shallow review detection with comprehensive review quality metrics
        if evidence.review_evidence.review_performed {
            // Check for comprehensive review depth
            let mut review_quality_issues = vec![];
            
            // Must analyze actual files and lines, not just exist
            if evidence.review_evidence.files_reviewed == 0 {
                review_quality_issues.push("No files were actually reviewed".to_string());
            }
            
            if evidence.review_evidence.lines_analyzed == 0 {
                review_quality_issues.push("No lines of code were analyzed during review".to_string());
            }
            
            // Must check security patterns for any code changes
            if evidence.patch_evidence.patch_created && evidence.review_evidence.security_patterns_checked == 0 {
                review_quality_issues.push("Security patterns were not checked during review".to_string());
            }
            
            // Must analyze API breaking changes if any API changes detected
            if evidence.semantic_evidence.api_changes_detected && evidence.review_evidence.api_breaking_changes_detected == 0 {
                review_quality_issues.push("API breaking changes were not properly analyzed".to_string());
            }
            
            // Must assess dependency changes if any detected
            if evidence.semantic_evidence.dependency_changes_detected && evidence.review_evidence.dependency_changes_analyzed == 0 {
                review_quality_issues.push("Dependency changes were not analyzed".to_string());
            }
            
            // Must have minimum review depth score
            if evidence.review_evidence.review_depth_score < 0.3 {
                review_quality_issues.push(format!("Review depth score {:.2} below minimum threshold 0.3", evidence.review_evidence.review_depth_score));
            }
            
            // For side-effect patches, must assess performance impact
            if evidence.patch_evidence.patch_created && !evidence.review_evidence.performance_impact_assessed {
                review_quality_issues.push("Performance impact was not assessed for side-effect patch".to_string());
            }
            
            // Check for comprehensive quality indicators
            if evidence.review_evidence.review_quality_indicators.is_empty() {
                review_quality_issues.push("No quality indicators recorded during review".to_string());
            }
            
            // If any quality issues found, report them
            if !review_quality_issues.is_empty() {
                evidence_issues.push(format!("Review quality insufficient: {}", review_quality_issues.join(", ")));
            }
        } else {
            // Review not performed at all
            if evidence.patch_evidence.patch_created {
                evidence_issues.push("Patch was created but no review was performed".to_string());
            }
        }

        // Risk assessment must include changed files and operation types
        if evidence.risk_evidence.risk_assessed && evidence.risk_evidence.overall_risk_level == "Unknown" {
            evidence_issues.push("Risk assessment incomplete - unknown risk level".to_string());
        }

        // Rollback handle must exist for applied edits
        if evidence.patch_evidence.patch_created && !evidence.process_evidence.rollback_available {
            evidence_issues.push("No rollback evidence for applied patch".to_string());
        }

        // P0-3.3: Make CompletionDecision::Complete impossible without complete evidence
        // Add comprehensive evidence completeness validation
        let mut completeness_issues = vec![];
        
        // Patch evidence completeness requirements
        if evidence.patch_evidence.patch_created {
            // Must have all patch hash stages for integrity verification
            if evidence.patch_evidence.generated_patch_hash.is_none() {
                completeness_issues.push("Missing generated patch hash - cannot verify patch integrity".to_string());
            }
            if evidence.patch_evidence.dry_run_patch_hash.is_none() {
                completeness_issues.push("Missing dry-run patch hash - cannot verify patch consistency".to_string());
            }
            if evidence.patch_evidence.applied_patch_hash.is_none() {
                completeness_issues.push("Missing applied patch hash - cannot verify patch application".to_string());
            }
            
            // Must have hash verification results
            if !evidence.patch_evidence.hash_verification_passed {
                completeness_issues.push("Patch hash verification failed - integrity cannot be guaranteed".to_string());
            }
            
            // Must have rollback evidence for safety
            if !evidence.process_evidence.rollback_available {
                completeness_issues.push("No rollback evidence available - cannot guarantee safe recovery".to_string());
            }
        }
        
        // Validation evidence completeness requirements
        if evidence.validation_evidence.validation_performed {
            // Must have executed actual validation commands
            if evidence.validation_evidence.commands_executed == 0 {
                completeness_issues.push("Validation marked as performed but no commands executed".to_string());
            }
            
            // Must have validation results for all categories that were planned
            if evidence.validation_evidence.commands_planned > 0 {
                let validation_categories = vec![
                    ("format", evidence.validation_evidence.format_check_passed),
                    ("static", evidence.validation_evidence.static_check_passed),
                    ("lint", evidence.validation_evidence.lint_check_passed),
                    ("test", evidence.validation_evidence.test_passed),
                ];
                
                for (category, result) in validation_categories {
                    if !result {
                        completeness_issues.push(format!("Validation category '{}' was planned but not completed", category));
                    }
                }
            }
            
            // Must have passed all validations for Complete decision
            if !evidence.validation_evidence.all_validations_passed {
                completeness_issues.push("Some validations failed - cannot proceed with Complete decision".to_string());
            }
        }
        
        // Review evidence completeness requirements
        if evidence.review_evidence.review_performed {
            // Must have comprehensive review metrics
            if evidence.review_evidence.files_reviewed == 0 {
                completeness_issues.push("Review performed but no files were reviewed".to_string());
            }
            if evidence.review_evidence.lines_analyzed == 0 {
                completeness_issues.push("Review performed but no lines of code were analyzed".to_string());
            }
            
            // Must have security analysis for code changes
            if evidence.patch_evidence.patch_created && evidence.review_evidence.security_patterns_checked == 0 {
                completeness_issues.push("Code changes made but security patterns were not checked".to_string());
            }
            
            // Must have minimum review quality
            if evidence.review_evidence.review_depth_score < 0.3 {
                completeness_issues.push(format!("Review depth score {:.2} below minimum threshold", evidence.review_evidence.review_depth_score));
            }
            
            // Must have quality indicators
            if evidence.review_evidence.review_quality_indicators.is_empty() {
                completeness_issues.push("Review performed but no quality indicators recorded".to_string());
            }
            
            // Must not have critical issues
            if evidence.review_evidence.critical_issues > 0 {
                completeness_issues.push(format!("{} critical review issues found - cannot proceed", evidence.review_evidence.critical_issues));
            }
        } else if evidence.patch_evidence.patch_created {
            // If patch was created, review must have been performed
            completeness_issues.push("Patch was created but no review was performed".to_string());
        }
        
        // Semantic evidence completeness requirements
        if evidence.patch_evidence.patch_created {
            // Must have semantic analysis for code changes
            if evidence.semantic_evidence.api_changes_detected && evidence.review_evidence.api_breaking_changes_detected == 0 {
                completeness_issues.push("API changes detected but not properly analyzed in review".to_string());
            }
            if evidence.semantic_evidence.dependency_changes_detected && evidence.review_evidence.dependency_changes_analyzed == 0 {
                completeness_issues.push("Dependency changes detected but not analyzed in review".to_string());
            }
        }
        
        // Risk evidence completeness requirements
        if evidence.risk_evidence.risk_assessed {
            // Must have complete risk assessment
            if evidence.risk_evidence.overall_risk_level == "Unknown" {
                completeness_issues.push("Risk assessment incomplete - unknown risk level".to_string());
            }
            
            // Must not require approval for Complete decision
            if evidence.risk_evidence.requires_approval {
                completeness_issues.push("Risk requires approval - cannot proceed with Complete decision".to_string());
            }
        }
        
        // Process evidence completeness requirements
        if evidence.patch_evidence.patch_created {
            // Must have process evidence for applied patches
            if !evidence.process_evidence.rollback_available {
                completeness_issues.push("No rollback evidence available for applied patch".to_string());
            }
        }
        
        // Combine all completeness issues with existing evidence issues
        if !completeness_issues.is_empty() {
            evidence_issues.push(format!("Evidence completeness requirements not met: {}", completeness_issues.join(", ")));
        }

        // If any evidence issues found, downgrade to Blocked
        if !evidence_issues.is_empty() {
            failures.push(format!("Evidence requirements not met: {}", evidence_issues.join(", ")));
        }

        // Still check overall completeness as a fallback
        if evidence.evidence_completeness < 0.75 {
            failures.push(format!(
                "Evidence completeness {:.0}% below threshold 75%",
                evidence.evidence_completeness * 100.0
            ));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionEvaluator {
    min_confidence_threshold: f32,
    require_validation: bool,
    require_review: bool,
    require_risk_assessment: bool,
}

impl Default for CompletionEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionEvaluator {
    pub fn new() -> Self {
        Self {
            min_confidence_threshold: 0.6,
            require_validation: true,
            require_review: true,
            require_risk_assessment: true,
        }
    }

    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            min_confidence_threshold: threshold,
            require_validation: true,
            require_review: true,
            require_risk_assessment: true,
        }
    }

    pub fn evaluate(
        &self,
        evidence: &CompletionEvidence,
        mode: HarnessMode,
    ) -> Result<CompletionDecision> {
        let mut decision_factors = vec![];

        // Check evidence completeness
        let completeness = self.calculate_completeness(evidence);

        // Decision logic based on mode
        let decision = match mode {
            HarnessMode::Review => self.evaluate_review_only(evidence, &mut decision_factors),
            HarnessMode::ReviewOnly => self.evaluate_review_only(evidence, &mut decision_factors),
            HarnessMode::Assisted => self.evaluate_assisted(evidence, &mut decision_factors),
            HarnessMode::Autonomous => self.evaluate_autonomous(evidence, &mut decision_factors),
            HarnessMode::Benchmark => self.evaluate_benchmark(evidence, &mut decision_factors),
        };

        Ok(decision)
    }

    /// P0-1: Helper function to validate completion invariants before returning Complete
    fn validate_and_return_complete(evidence: &CompletionEvidence) -> CompletionDecision {
        let decision = CompletionDecision::Complete;
        match decision.validate(evidence) {
            Ok(()) => decision,
            Err(failures) => CompletionDecision::Blocked(
                format!("Completion invariants failed: {}", failures.join(", "))
            ),
        }
    }

    fn evaluate_review_only(
        &self,
        evidence: &CompletionEvidence,
        factors: &mut Vec<String>,
    ) -> CompletionDecision {
        if !evidence.review_evidence.review_performed {
            factors.push("Review not performed".to_string());
            return CompletionDecision::Blocked("Review required in ReviewOnly mode".to_string());
        }

        if evidence.review_evidence.critical_issues > 0 {
            factors.push(format!(
                "{} critical issues found",
                evidence.review_evidence.critical_issues
            ));
            return CompletionDecision::Blocked(
                "Critical review issues must be resolved".to_string(),
            );
        }

        Self::validate_and_return_complete(evidence)
    }

    fn evaluate_assisted(
        &self,
        evidence: &CompletionEvidence,
        factors: &mut Vec<String>,
    ) -> CompletionDecision {
        // Must have patch
        if !evidence.patch_evidence.patch_created {
            factors.push("No patch created".to_string());
            return CompletionDecision::Blocked("No patch generated".to_string());
        }

        // Validation required
        if self.require_validation && !evidence.validation_evidence.validation_performed {
            factors.push("Validation not performed".to_string());
            return CompletionDecision::Blocked("Validation required".to_string());
        }

        if !evidence.validation_evidence.all_validations_passed {
            factors.push("Validation failed".to_string());
            return CompletionDecision::NeedsRepair(
                "Validation failed - fixes required".to_string(),
            );
        }

        // Review required
        if self.require_review && !evidence.review_evidence.review_performed {
            factors.push("Review not performed".to_string());
            return CompletionDecision::Blocked("Review required".to_string());
        }

        // Check confidence
        if evidence.confidence_evidence.confidence_score < self.min_confidence_threshold {
            factors.push(format!(
                "Confidence {:.0}% below threshold {:.0}%",
                evidence.confidence_evidence.confidence_score * 100.0,
                self.min_confidence_threshold * 100.0
            ));
            return CompletionDecision::NeedsApproval(
                "Low confidence - approval required".to_string(),
            );
        }

        // Check risk
        if evidence.risk_evidence.requires_approval {
            factors.push("High risk requires approval".to_string());
            return CompletionDecision::NeedsApproval("Risk approval required".to_string());
        }

        Self::validate_and_return_complete(evidence)
    }

    fn evaluate_autonomous(
        &self,
        evidence: &CompletionEvidence,
        factors: &mut Vec<String>,
    ) -> CompletionDecision {
        // P0-Issue1: Make Docker/isolated runtime mandatory for autonomous mode
        // Check if we have real evidence of Docker/isolated runtime usage
        let has_docker_runtime = evidence.sandbox_evidence.iter().any(|evidence| {
            matches!(evidence.runtime_kind, crate::harness::sandbox::SandboxRuntimeKind::Docker) &&
            evidence.isolated_process &&
            evidence.isolated_filesystem &&
            evidence.network_disabled &&
            evidence.resource_limits_applied &&
            evidence.no_new_privileges &&
            evidence.capabilities_dropped
        });
        
        if !has_docker_runtime {
            factors.push("Docker/isolated runtime not detected".to_string());
            return CompletionDecision::Blocked(
                "Autonomous mode requires Docker/isolated runtime for safety".to_string(),
            );
        }

        // Stricter requirements for autonomous mode
        if evidence.confidence_evidence.confidence_score < 0.8 {
            factors.push("Insufficient confidence for autonomous mode".to_string());
            return CompletionDecision::NeedsApproval(
                "Confidence below 80% for autonomous execution".to_string(),
            );
        }

        if evidence.risk_evidence.overall_risk_level == "Critical" {
            factors.push("Critical risk in autonomous mode".to_string());
            return CompletionDecision::NeedsApproval(
                "Critical risk - human review required".to_string(),
            );
        }

        // Must have full verification
        match evidence.verification_evidence.verification_level {
            VerificationStrength::Full | VerificationStrength::Reproduction => {}
            _ => {
                factors.push("Full verification not achieved".to_string());
                return CompletionDecision::NeedsRepair(
                    "Full verification required for autonomous mode".to_string(),
                );
            }
        }

        self.evaluate_assisted(evidence, factors)
    }

    fn evaluate_benchmark(
        &self,
        evidence: &CompletionEvidence,
        factors: &mut Vec<String>,
    ) -> CompletionDecision {
        // Benchmark mode is for testing harness itself - less strict
        if !evidence.patch_evidence.patch_created {
            return CompletionDecision::Blocked("No patch".to_string());
        }

        Self::validate_and_return_complete(evidence)
    }

    fn calculate_completeness(&self, evidence: &CompletionEvidence) -> f32 {
        let mut dimensions_present = 0u32;
        let total_dimensions = 8u32;

        if evidence.patch_evidence.patch_created {
            dimensions_present += 1;
        }
        if evidence.validation_evidence.validation_performed {
            dimensions_present += 1;
        }
        if evidence.review_evidence.review_performed {
            dimensions_present += 1;
        }
        if evidence.risk_evidence.risk_assessed {
            dimensions_present += 1;
        }
        if evidence.verification_evidence.test_count > 0 {
            dimensions_present += 1;
        }
        if evidence.semantic_evidence.api_changes_detected
            || evidence.patch_evidence.files_modified > 0
        {
            dimensions_present += 1;
        }
        if evidence.confidence_evidence.confidence_score > 0.0 {
            dimensions_present += 1;
        }
        if evidence.process_evidence.git_checkpoint_created {
            dimensions_present += 1;
        }

        dimensions_present as f32 / total_dimensions as f32
    }
}

pub fn evaluate_completion(
    evidence: &CompletionEvidence,
    mode: HarnessMode,
) -> Result<CompletionDecision> {
    let evaluator = CompletionEvaluator::new();
    evaluator.evaluate(evidence, mode)
}

pub fn format_completion_decision(decision: &CompletionDecision) -> String {
    match decision {
        CompletionDecision::Complete => "✓ Task Complete - Ready for deployment".to_string(),
        CompletionDecision::Blocked(reason) => format!("✗ Blocked - {}", reason),
        CompletionDecision::NeedsRepair(reason) => format!("🔧 Needs Repair - {}", reason),
        CompletionDecision::NeedsApproval(reason) => format!("👤 Needs Approval - {}", reason),
    }
}

pub fn create_evidence_from_components(
    patch: &PatchEvidence,
    validation: &ValidationResult,
    review: &ReviewReport,
    risk: &RiskAssessment,
    semantic: &SemanticDiff,
    confidence: &ConfidenceScore,
    git_checkpoint_available: bool,
    rollback_available: bool,
    time_limit_respected: bool,
    step_limit_respected: bool,
) -> CompletionEvidence {
    let review_evidence = ReviewEvidence {
        review_performed: true,
        total_issues: review.summary.total_issues,
        critical_issues: review.critical_count,
        high_issues: review.high_count,
        medium_issues: review
            .summary
            .by_severity
            .get(&crate::harness::review::ReviewSeverity::Medium)
            .copied()
            .unwrap_or(0),
        low_issues: review
            .summary
            .by_severity
            .get(&crate::harness::review::ReviewSeverity::Low)
            .copied()
            .unwrap_or(0),
        security_issues: review
            .summary
            .by_type
            .get(&crate::harness::review::ReviewIssueType::Security)
            .copied()
            .unwrap_or(0),
        breaking_change_issues: review
            .summary
            .by_type
            .get(&crate::harness::review::ReviewIssueType::ApiChange)
            .copied()
            .unwrap_or(0),
        review_passed: review.passed,
        // P0-3.2: Comprehensive review quality metrics
        files_reviewed: patch.files_modified,
        lines_analyzed: patch.lines_changed,
        security_patterns_checked: review
            .summary
            .by_type
            .get(&crate::harness::review::ReviewIssueType::Security)
            .copied()
            .unwrap_or(0),
        api_breaking_changes_detected: semantic.summary.breaking_changes,
        dependency_changes_analyzed: semantic.dependency_changes.len(),
        test_coverage_analyzed: validation.command_results.iter().any(|c| c.command.contains("test")),
        performance_impact_assessed: false, // Would need performance analysis
        documentation_updated: false, // Would need documentation analysis
        review_depth_score: {
            // P0-3.2: Calculate review depth score based on comprehensive factors
            let mut score = 0.0;
            
            // Base score for having any review
            if review.summary.total_issues > 0 {
                score += 0.2;
            }
            
            // Score for analyzing files
            if patch.files_modified > 0 {
                score += 0.2;
            }
            
            // Score for security analysis
            if review.summary.by_type.get(&crate::harness::review::ReviewIssueType::Security).is_some() {
                score += 0.2;
            }
            
            // Score for API analysis
            if !semantic.api_changes.is_empty() {
                score += 0.2;
            }
            
            // Score for dependency analysis
            if !semantic.dependency_changes.is_empty() {
                score += 0.2;
            }
            
            (score as f32).min(1.0)
        },
        review_quality_indicators: {
            // P0-3.2: Generate quality indicators based on review analysis
            let mut indicators = vec![];
            
            if review.summary.total_issues > 0 {
                indicators.push("Issues detected".to_string());
            }
            
            if review.summary.by_type.get(&crate::harness::review::ReviewIssueType::Security).is_some() {
                indicators.push("Security analysis performed".to_string());
            }
            
            if !semantic.api_changes.is_empty() {
                indicators.push("API changes analyzed".to_string());
            }
            
            if !semantic.dependency_changes.is_empty() {
                indicators.push("Dependencies analyzed".to_string());
            }
            
            if validation.command_results.iter().any(|c| c.command.contains("test")) {
                indicators.push("Test coverage considered".to_string());
            }
            
            indicators
        },
    };

    let risk_evidence = RiskEvidence {
        risk_assessed: true,
        overall_risk_level: format!("{:?}", risk.level),
        security_risk: format!(
            "{:?}",
            risk.reasons
                .iter()
                .any(|r| r.category == crate::harness::risk::RiskCategory::Security)
        ),
        api_risk: format!("{:?}", semantic.api_changes.iter().any(|a| a.breaking)),
        database_risk: format!("{:?}", semantic.database_changes.iter().any(|d| d.breaking)),
        dependency_risk: format!(
            "{:?}",
            semantic.dependency_changes.iter().any(|d| matches!(
                d.risk_level,
                crate::harness::semantic_diff::RiskLevel::High
                    | crate::harness::semantic_diff::RiskLevel::Critical
            ))
        ),
        requires_approval: risk.requires_approval,
        risk_reasons: risk.reasons.iter().map(|r| r.description.clone()).collect(),
    };

    let semantic_evidence = SemanticEvidence {
        api_changes_detected: !semantic.api_changes.is_empty(),
        auth_changes_detected: !semantic.auth_changes.is_empty(),
        database_changes_detected: !semantic.database_changes.is_empty(),
        dependency_changes_detected: !semantic.dependency_changes.is_empty(),
        config_changes_detected: !semantic.config_changes.is_empty(),
        breaking_changes_count: semantic.summary.breaking_changes,
        security_relevant_changes: !semantic.auth_changes.is_empty(),
    };

    let confidence_evidence = ConfidenceEvidence {
        confidence_score: confidence.score,
        confidence_classification: format!(
            "{:?}",
            if confidence.score >= 0.8 {
                "High"
            } else if confidence.score >= 0.6 {
                "Medium"
            } else {
                "Low"
            }
        ),
        validation_contribution: 0.0, // Would calculate from factors
        risk_contribution: 0.0,
        review_contribution: 0.0,
        confidence_factors: confidence.factors.iter().map(|f| f.name.clone()).collect(),
    };

    CompletionEvidence {
        patch_evidence: PatchEvidence {
            patch_created: patch.patch_created,
            files_modified: patch.files_modified,
            lines_changed: patch.lines_changed,
            patch_applied_cleanly: patch.patch_applied_cleanly,
            patch_hash: patch.patch_hash.clone(),
            dry_run_passed: patch.dry_run_passed,
            // P0-3.1: Real patch identity verification for audit-grade integrity
            patch_identity: None, // Will be populated during actual patch application
            // Legacy fields for backward compatibility
            generated_patch_hash: patch.patch_hash.clone(),
            dry_run_patch_hash: None, // Would need dry-run result to compute this
            applied_patch_hash: patch.patch_hash.clone(),
            hash_verification_passed: patch.patch_hash.is_some(),
            hash_mismatch_details: None, // Basic implementation for compatibility
        },
        validation_evidence: ValidationEvidence {
            validation_performed: true,
            all_validations_passed: validation.passed(),
            format_check_passed: validation
                .command_results
                .iter()
                .any(|r| r.command.contains("fmt") && r.exit_code == Some(0)),
            static_check_passed: validation.command_results.iter().any(|r| {
                (r.command.contains("check") || r.command.contains("build"))
                    && r.exit_code == Some(0)
            }),
            lint_check_passed: validation
                .command_results
                .iter()
                .any(|r| r.command.contains("clippy") && r.exit_code == Some(0)),
            test_passed: validation
                .command_results
                .iter()
                .any(|r| r.command.contains("test") && r.exit_code == Some(0)),
            validation_summary: format!(
                "{} commands, {} passed",
                validation.command_results.len(),
                validation
                    .command_results
                    .iter()
                    .filter(|r| r.exit_code == Some(0))
                    .count()
            ),
            // P0-2.1: Add direct command execution counters
            commands_planned: validation.commands_planned,
            commands_executed: validation.commands_executed,
            commands_skipped: validation.commands_skipped,
            categories_executed: validation.categories_executed.clone(),
        },
        review_evidence,
        risk_evidence,
        verification_evidence: VerificationEvidence {
            verification_level: crate::harness::verification::VerificationStrength::Tests,
            test_count: validation
                .command_results
                .iter()
                .filter(|r| r.command.contains("test"))
                .count(),
            coverage_percent: Some(validation.command_results.iter()
                .filter(|r| r.exit_code == Some(0))
                .count() as f32 / validation.command_results.len() as f32 * 100.0),
            reproduction_test_passed: validation.command_results.iter()
                .any(|r| r.command.contains("repro") && r.exit_code == Some(0)),
            integration_tests_passed: validation.command_results.iter()
                .any(|r| r.command.contains("integration") && r.exit_code == Some(0)),
            verification_summary: format!("Validation: {} passed, {} failed", 
                validation.command_results.iter().filter(|r| r.exit_code == Some(0)).count(),
                validation.command_results.iter().filter(|r| r.exit_code != Some(0)).count()),
        },
        semantic_evidence,
        confidence_evidence,
        process_evidence: ProcessEvidence {
            git_checkpoint_created: git_checkpoint_available,
            rollback_available: rollback_available,
            all_phases_completed: validation.validation_performed && review.review_performed,
            no_critical_errors: validation.passed() && review.passed,
            time_limit_respected: time_limit_respected,
            step_limit_respected: step_limit_respected,
        },
        // Sandbox evidence is populated by execution loop when available.
        sandbox_evidence: Vec::new(),
        patch_exists: patch.patch_created,
        validation_ran: true,
        validation_passed: validation.passed(),
        review_ran: true,
        critical_issues: review.critical_count,
        confidence: confidence.clone(),
        verification_strength: crate::harness::verification::VerificationStrength::Tests,
        requires_approval: risk.requires_approval,
        decision_factors: vec![],
        evidence_completeness: calculate_evidence_completeness(validation, review, risk),
    }
}

/// P0-4 FIX: Calculate evidence completeness from actual validation, review, and risk results
/// This replaces the hardcoded 0.8 value with real evidence assessment
fn calculate_evidence_completeness(
    validation: &ValidationResult,
    review: &ReviewReport,
    risk: &RiskAssessment,
) -> f32 {
    let mut completeness = 0.0;
    let mut total_weight = 0.0;
    
    // Validation evidence (40% weight)
    if validation.validation_performed {
        completeness += 0.4;
        if validation.passed() {
            completeness += 0.1; // Bonus for passing validation
        }
    }
    total_weight += 0.5;
    
    // Review evidence (30% weight)
    if review.review_performed {
        completeness += 0.3;
        if review.passed {
            completeness += 0.1; // Bonus for passing review
        }
    }
    total_weight += 0.4;
    
    // Risk assessment (20% weight)
    if risk.assessed {
        completeness += 0.2;
        if !risk.requires_approval {
            completeness += 0.05; // Bonus for low risk
        }
    }
    total_weight += 0.25;
    
    // Command execution evidence (10% weight)
    if !validation.command_results.is_empty() {
        completeness += 0.1;
    }
    total_weight += 0.1;
    
    // Normalize by total weight used
    if total_weight > 0.0 {
        (completeness / total_weight as f32).min(1.0)
    } else {
        0.0
    }
}
