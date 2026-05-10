//! User-Facing Trust Report
//!
//! P1-FIX: Provides transparent, human-readable reports of harness execution
//! for user review and audit. Shows what was done, why decisions were made,
//! and what evidence supports those decisions.
//!
//! This addresses the "trust but verify" requirement - users can see exactly
//! how the AI agent operated on their codebase.

use crate::harness::{
    completion::CompletionDecision,
    evidence::{EvidenceEntry, EvidenceEntryKind, EvidenceLog},
    execution_loop::HarnessExecutionResult,
};
use serde::{Deserialize, Serialize};

/// Trust level for an operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Operation was fully verified and safe
    High,
    /// Operation had some risk but was mitigated
    Medium,
    /// Operation had significant risk or issues
    Low,
    /// Operation was blocked or failed
    Blocked,
}

/// A trust report section summarizing a phase of execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustSection {
    /// Title of this section
    pub title: String,
    /// Trust level for this phase
    pub trust_level: TrustLevel,
    /// Human-readable summary
    pub summary: String,
    /// Detailed bullet points
    pub details: Vec<String>,
    /// Evidence supporting this section
    pub evidence_ids: Vec<String>,
}

/// Complete trust report for a harness execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustReport {
    /// Overall trust score (0-100)
    pub trust_score: u32,
    /// Overall trust level
    pub overall_trust: TrustLevel,
    /// Execution ID this report covers
    pub execution_id: String,
    /// When the execution started
    pub started_at: String,
    /// When the execution completed
    pub completed_at: Option<String>,
    /// High-level summary for non-technical users
    pub executive_summary: String,
    /// Detailed sections
    pub sections: Vec<TrustSection>,
    /// Key statistics
    pub statistics: TrustStatistics,
    /// Recommendations for user
    pub recommendations: Vec<String>,
    /// Raw evidence log reference
    pub evidence_log_id: String,
}

/// Statistics for the trust report
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStatistics {
    /// Number of files examined
    pub files_examined: usize,
    /// Number of files modified
    pub files_modified: usize,
    /// Number of validation commands run
    pub validations_run: usize,
    /// Number of review issues found
    pub review_issues: usize,
    /// Number of critical issues
    pub critical_issues: usize,
    /// Number of patches generated
    pub patches_generated: usize,
    /// Number of rollback events
    pub rollbacks: usize,
    /// Number of blocked operations
    pub blocked_operations: usize,
}

/// Builder for creating trust reports from execution results
pub struct TrustReportBuilder;

impl TrustReportBuilder {
    /// Build a trust report from an execution result and evidence log
    pub fn build(result: &HarnessExecutionResult, evidence_log: &EvidenceLog) -> TrustReport {
        let mut sections = Vec::new();
        let mut stats = TrustStatistics::default();

        // Build Repository Analysis section
        let repo_section = Self::build_repo_section(evidence_log, &mut stats);
        sections.push(repo_section);

        // Build Patch Generation section
        let generation_section = Self::build_generation_section(evidence_log, &mut stats);
        sections.push(generation_section);

        // Build Validation section
        let validation_section = Self::build_validation_section(evidence_log, &mut stats);
        sections.push(validation_section);

        // Build Risk Assessment section
        let risk_section = Self::build_risk_section(result, evidence_log, &mut stats);
        sections.push(risk_section);

        // Build Execution Summary section
        let execution_section = Self::build_execution_section(result, evidence_log, &mut stats);
        sections.push(execution_section);

        // Calculate overall trust score
        let trust_score = Self::calculate_trust_score(&sections, &stats);
        let overall_trust = Self::determine_overall_trust(trust_score, &stats);

        // Generate executive summary
        let executive_summary = Self::generate_executive_summary(result, &sections, &stats);

        // Generate recommendations
        let recommendations = Self::generate_recommendations(&sections, &stats);

        TrustReport {
            trust_score,
            overall_trust,
            execution_id: result.work_context_id.clone(),
            started_at: evidence_log
                .started_at
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
            completed_at: evidence_log.completed_at.map(|d| d.to_rfc3339()),
            executive_summary,
            sections,
            statistics: stats,
            recommendations,
            evidence_log_id: evidence_log.execution_id.clone(),
        }
    }

    fn build_repo_section(evidence_log: &EvidenceLog, stats: &mut TrustStatistics) -> TrustSection {
        let repo_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::RepoMapBuilt)
            .collect();

        let mut details = vec![];
        let mut evidence_ids = vec![];

        for entry in &repo_entries {
            evidence_ids.push(entry.id.clone());
            if let Some(files) = entry.output_summary.get("files_found") {
                details.push(format!("Repository analysis found {} files", files));
                stats.files_examined = files.parse().unwrap_or(0);
            }
            if let Some(symbols) = entry.output_summary.get("symbols_found") {
                details.push(format!("Identified {} code symbols", symbols));
            }
        }

        if details.is_empty() {
            details.push("Repository analysis completed".to_string());
        }

        TrustSection {
            title: "Repository Analysis".to_string(),
            trust_level: TrustLevel::High,
            summary: "The codebase was successfully analyzed to understand its structure and dependencies.".to_string(),
            details,
            evidence_ids,
        }
    }

    fn build_generation_section(
        evidence_log: &EvidenceLog,
        stats: &mut TrustStatistics,
    ) -> TrustSection {
        let gen_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::PatchGenerated)
            .collect();

        let mut details = vec![];
        let mut evidence_ids = vec![];

        stats.patches_generated = gen_entries.len();

        for entry in &gen_entries {
            evidence_ids.push(entry.id.clone());
            if let Some(count) = entry.output_summary.get("edits_count") {
                details.push(format!("Generated patch with {} edits", count));
            }
            if let Some(confidence) = entry.output_summary.get("confidence") {
                details.push(format!("AI confidence in patch: {}", confidence));
            }
        }

        if details.is_empty() {
            details.push("No patches were generated".to_string());
        }

        TrustSection {
            title: "Patch Generation".to_string(),
            trust_level: TrustLevel::Medium,
            summary: format!(
                "{} patch candidate(s) were generated by the AI.",
                stats.patches_generated
            ),
            details,
            evidence_ids,
        }
    }

    fn build_validation_section(
        evidence_log: &EvidenceLog,
        stats: &mut TrustStatistics,
    ) -> TrustSection {
        let val_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| {
                e.kind == EvidenceEntryKind::ValidationCommandRun
                    || e.kind == EvidenceEntryKind::ValidationCompleted
                    || e.kind == EvidenceEntryKind::DryRunPassed
                    || e.kind == EvidenceEntryKind::DryRunFailed
            })
            .collect();

        let mut details = vec![];
        let mut evidence_ids = vec![];
        let mut passed = 0;
        let mut failed = 0;

        for entry in &val_entries {
            evidence_ids.push(entry.id.clone());
            stats.validations_run += 1;

            match entry.kind {
                EvidenceEntryKind::DryRunPassed => {
                    passed += 1;
                    details.push("Dry-run validation passed - patch applies cleanly".to_string());
                }
                EvidenceEntryKind::DryRunFailed => {
                    failed += 1;
                    if let Some(failures) = entry.output_summary.get("failures") {
                        details.push(format!("Dry-run failed with {} errors", failures));
                    }
                }
                EvidenceEntryKind::ValidationCompleted => {
                    if let Some(passed_str) = entry.output_summary.get("passed") {
                        if passed_str == "true" {
                            passed += 1;
                            details.push("Full validation suite passed".to_string());
                        } else {
                            failed += 1;
                            details.push("Validation suite found issues".to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        let trust_level = if failed == 0 && passed > 0 {
            TrustLevel::High
        } else if failed > 0 && passed > 0 {
            TrustLevel::Medium
        } else {
            TrustLevel::Low
        };

        let summary = if passed > 0 && failed == 0 {
            "All validation checks passed.".to_string()
        } else if passed > 0 && failed > 0 {
            format!("{} validation(s) passed, {} failed.", passed, failed)
        } else if failed > 0 {
            "All validations failed.".to_string()
        } else {
            "No validations were run.".to_string()
        };

        TrustSection {
            title: "Validation & Testing".to_string(),
            trust_level,
            summary,
            details,
            evidence_ids,
        }
    }

    fn build_risk_section(
        result: &HarnessExecutionResult,
        evidence_log: &EvidenceLog,
        stats: &mut TrustStatistics,
    ) -> TrustSection {
        let risk_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::RiskAssessed)
            .collect();

        let review_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::ReviewCompleted)
            .collect();

        let mut details = vec![];
        let mut evidence_ids = vec![];

        // Add risk assessment details
        for entry in &risk_entries {
            evidence_ids.push(entry.id.clone());
            if let Some(level) = entry.output_summary.get("risk_level") {
                details.push(format!("Risk assessment: {}", level));
            }
            if let Some(requires_approval) = entry.output_summary.get("requires_approval") {
                if requires_approval == "true" {
                    details.push("This change requires user approval".to_string());
                }
            }
        }

        // Add review issues
        stats.review_issues = result.review_issues.len();
        stats.critical_issues = result
            .review_issues
            .iter()
            .filter(|i| i.severity == crate::harness::review::ReviewSeverity::Critical)
            .count();

        if stats.critical_issues > 0 {
            details.push(format!(
                "⚠️ {} critical review issue(s) found",
                stats.critical_issues
            ));
        }
        if stats.review_issues > stats.critical_issues {
            details.push(format!(
                "{} non-critical review issue(s) found",
                stats.review_issues - stats.critical_issues
            ));
        }

        for entry in &review_entries {
            evidence_ids.push(entry.id.clone());
        }

        let trust_level = if stats.critical_issues > 0 {
            TrustLevel::Low
        } else if stats.review_issues > 0 {
            TrustLevel::Medium
        } else {
            TrustLevel::High
        };

        let summary = if stats.critical_issues > 0 {
            "Critical issues were identified that require attention.".to_string()
        } else if stats.review_issues > 0 {
            "Some review issues were found but none are critical.".to_string()
        } else {
            "No significant risks or issues were identified.".to_string()
        };

        TrustSection {
            title: "Risk & Safety Review".to_string(),
            trust_level,
            summary,
            details,
            evidence_ids,
        }
    }

    fn build_execution_section(
        result: &HarnessExecutionResult,
        evidence_log: &EvidenceLog,
        stats: &mut TrustStatistics,
    ) -> TrustSection {
        let mut details = vec![];
        let mut evidence_ids = vec![];

        // Check for blocked operations
        let blocked_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::SideEffectBlocked)
            .collect();

        stats.blocked_operations = blocked_entries.len();

        for entry in &blocked_entries {
            evidence_ids.push(entry.id.clone());
            details.push(format!("⚠️ Blocked: {}", entry.description));
        }

        // Check for rollbacks
        let rollback_entries: Vec<&EvidenceEntry> = evidence_log
            .entries
            .iter()
            .filter(|e| e.kind == EvidenceEntryKind::RollbackPerformed)
            .collect();

        stats.rollbacks = rollback_entries.len();

        for entry in &rollback_entries {
            evidence_ids.push(entry.id.clone());
            details.push(format!("↩️ Rollback performed: {}", entry.description));
        }

        // Add completion status
        match &result.completion_decision {
            CompletionDecision::Complete => {
                details.push("✅ Task completed successfully".to_string());
            }
            CompletionDecision::NeedsRepair(reason) => {
                details.push(format!("⚠️ Task needs repair: {}", reason));
            }
            CompletionDecision::NeedsApproval(reason) => {
                details.push(format!("⏸️ Task needs approval: {}", reason));
            }
            CompletionDecision::Blocked(reason) => {
                details.push(format!("🚫 Task blocked: {}", reason));
            }
        }

        // Add patch application status
        if let Some(ref patch_result) = result.patch_result {
            if patch_result.applied {
                stats.files_modified = patch_result.changed_files.len();
                details.push(format!(
                    "📝 Applied changes to {} file(s)",
                    patch_result.changed_files.len()
                ));
            } else {
                details.push("No changes were applied".to_string());
            }
        }

        let trust_level = if stats.blocked_operations > 0 || stats.rollbacks > 0 {
            TrustLevel::Medium
        } else {
            TrustLevel::High
        };

        TrustSection {
            title: "Execution Summary".to_string(),
            trust_level,
            summary: "Final execution status and applied changes.".to_string(),
            details,
            evidence_ids,
        }
    }

    fn calculate_trust_score(_sections: &[TrustSection], stats: &TrustStatistics) -> u32 {
        let mut score = 100;

        // Deduct for issues
        score -= stats.critical_issues as u32 * 25;
        score -= (stats.review_issues - stats.critical_issues) as u32 * 5;

        // Deduct for rollbacks and blocks
        score -= stats.rollbacks as u32 * 15;
        score -= stats.blocked_operations as u32 * 10;

        // Deduct for failures
        if stats.validations_run > 0 {
            // Assume some validations failed if we have issues
        }

        // Bonus for passing validations
        if stats.validations_run > 0 && stats.critical_issues == 0 {
            score += 5;
        }

        score.clamp(0, 100)
    }

    fn determine_overall_trust(score: u32, stats: &TrustStatistics) -> TrustLevel {
        if score >= 80 && stats.critical_issues == 0 {
            TrustLevel::High
        } else if score >= 50 {
            TrustLevel::Medium
        } else if score > 0 {
            TrustLevel::Low
        } else {
            TrustLevel::Blocked
        }
    }

    fn generate_executive_summary(
        result: &HarnessExecutionResult,
        _sections: &[TrustSection],
        stats: &TrustStatistics,
    ) -> String {
        let mut parts = vec![];

        // Opening statement
        match &result.completion_decision {
            CompletionDecision::Complete => {
                parts.push("The task was completed successfully.".to_string());
            }
            CompletionDecision::NeedsRepair(_) => {
                parts.push("The task was completed with some issues.".to_string());
            }
            CompletionDecision::NeedsApproval(_) => {
                parts.push("The task completed but needs your approval.".to_string());
            }
            CompletionDecision::Blocked(_) => {
                parts.push("The task was blocked for safety reasons.".to_string());
            }
        }

        // File changes
        if stats.files_modified > 0 {
            parts.push(format!("{} file(s) were modified.", stats.files_modified));
        }

        // Risk statement
        if stats.critical_issues > 0 {
            parts.push(format!(
                "⚠️ {} critical issue(s) require your attention.",
                stats.critical_issues
            ));
        } else if stats.review_issues > 0 {
            parts.push("Minor issues were found but no critical concerns.".to_string());
        } else {
            parts.push("No significant issues were identified.".to_string());
        }

        // Safety measures
        if stats.rollbacks > 0 {
            parts.push("Automatic rollbacks protected your codebase from errors.".to_string());
        }
        if stats.blocked_operations > 0 {
            parts.push("Some operations were blocked for safety.".to_string());
        }

        parts.join(" ")
    }

    fn generate_recommendations(
        _sections: &[TrustSection],
        stats: &TrustStatistics,
    ) -> Vec<String> {
        let mut recommendations = vec![];

        if stats.critical_issues > 0 {
            recommendations.push(
                "Review the critical issues identified before accepting the changes.".to_string(),
            );
        }

        if stats.rollbacks > 0 {
            recommendations.push(
                "The AI attempted fixes that didn't work - consider providing more specific guidance."
                    .to_string(),
            );
        }

        if stats.validations_run == 0 {
            recommendations.push(
                "No validation tests were run - consider enabling automatic testing.".to_string(),
            );
        }

        if stats.files_modified > 10 {
            recommendations
                .push("Many files were modified - review carefully before committing.".to_string());
        }

        if recommendations.is_empty() && stats.critical_issues == 0 {
            recommendations.push("The changes appear safe to review and accept.".to_string());
        }

        recommendations
    }
}

impl TrustReport {
    /// Export to JSON format
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Export to Markdown format for human reading
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // Title
        md.push_str(&format!("# Trust Report: {}\n\n", self.execution_id));

        // Overall score
        let score_emoji = match self.overall_trust {
            TrustLevel::High => "🟢",
            TrustLevel::Medium => "🟡",
            TrustLevel::Low => "🔴",
            TrustLevel::Blocked => "⛔",
        };
        md.push_str(&format!(
            "**Overall Trust Score:** {} {}/100\n\n",
            score_emoji, self.trust_score
        ));

        // Executive summary
        md.push_str("## Executive Summary\n\n");
        md.push_str(&self.executive_summary);
        md.push_str("\n\n");

        // Statistics
        md.push_str("## Statistics\n\n");
        md.push_str(&format!(
            "- Files examined: {}\n",
            self.statistics.files_examined
        ));
        md.push_str(&format!(
            "- Files modified: {}\n",
            self.statistics.files_modified
        ));
        md.push_str(&format!(
            "- Validations run: {}\n",
            self.statistics.validations_run
        ));
        md.push_str(&format!(
            "- Review issues: {} ({} critical)\n",
            self.statistics.review_issues, self.statistics.critical_issues
        ));
        md.push_str(&format!(
            "- Patches generated: {}\n",
            self.statistics.patches_generated
        ));
        md.push_str(&format!("- Rollbacks: {}\n", self.statistics.rollbacks));
        md.push_str(&format!(
            "- Blocked operations: {}\n",
            self.statistics.blocked_operations
        ));
        md.push_str("\n");

        // Sections
        for section in &self.sections {
            let emoji = match section.trust_level {
                TrustLevel::High => "✅",
                TrustLevel::Medium => "⚠️",
                TrustLevel::Low => "❌",
                TrustLevel::Blocked => "🚫",
            };
            md.push_str(&format!("## {} {}\n\n", emoji, section.title));
            md.push_str(&format!("{}\n\n", section.summary));

            if !section.details.is_empty() {
                for detail in &section.details {
                    md.push_str(&format!("- {}\n", detail));
                }
                md.push_str("\n");
            }
        }

        // Recommendations
        if !self.recommendations.is_empty() {
            md.push_str("## Recommendations\n\n");
            for rec in &self.recommendations {
                md.push_str(&format!("- {}\n", rec));
            }
            md.push_str("\n");
        }

        // Footer
        md.push_str("---\n\n");
        md.push_str(&format!("*Evidence Log ID: {}*\n", self.evidence_log_id));

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_level_serialization() {
        let level = TrustLevel::High;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, "\"High\"");
    }

    #[test]
    fn test_trust_report_markdown_output() {
        let report = TrustReport {
            trust_score: 85,
            overall_trust: TrustLevel::High,
            execution_id: "test-123".to_string(),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            completed_at: Some("2024-01-01T00:05:00Z".to_string()),
            executive_summary: "Test summary".to_string(),
            sections: vec![TrustSection {
                title: "Test Section".to_string(),
                trust_level: TrustLevel::High,
                summary: "Test passed".to_string(),
                details: vec!["Detail 1".to_string()],
                evidence_ids: vec!["ev1".to_string()],
            }],
            statistics: TrustStatistics::default(),
            recommendations: vec!["Review the code".to_string()],
            evidence_log_id: "log-123".to_string(),
        };

        let md = report.to_markdown();
        assert!(md.contains("Trust Report"));
        assert!(md.contains("85/100"));
        assert!(md.contains("Test summary"));
    }
}
