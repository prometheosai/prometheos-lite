//! Human-readable patch report generation
//!
//! P1: Produce final Markdown/JSON report including:
//! - Changed files
//! - Diff summary
//! - Validation commands
//! - Risks
//! - Rollback status
//! - Evidence trace

use crate::harness::{
    edit_protocol::EditOperation,
    evidence::{EvidenceEntry, EvidenceEntryKind, EvidenceLog},
    execution_loop::HarnessExecutionResult,
    failure::FailureKind,
    review::ReviewIssue,
    risk::{RiskAssessment, RiskLevel},
    validation::ValidationResult,
};
use serde::{Deserialize, Serialize};

/// Human-readable patch report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchReport {
    /// Report title
    pub title: String,
    /// Work context ID
    pub work_context_id: String,
    /// Summary of changes
    pub summary: String,
    /// Changed files
    pub changed_files: Vec<FileChange>,
    /// Diff statistics
    pub diff_stats: DiffStats,
    /// Validation results
    pub validation: ValidationSummary,
    /// Risk assessment
    pub risk: RiskSummary,
    /// Review findings
    pub review: ReviewSummary,
    /// Rollback status
    pub rollback: RollbackStatus,
    /// Evidence trace
    pub evidence_trace: Vec<EvidenceEntry>,
    /// Final decision
    pub decision: String,
    /// Execution metrics
    pub metrics: ExecutionMetrics,
}

/// File change details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub operation: String, // "modified", "created", "deleted", "renamed"
    pub lines_added: usize,
    pub lines_removed: usize,
}

/// Diff statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub total_lines_changed: usize,
}

/// Validation summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub passed: bool,
    pub commands_run: usize,
    pub commands_passed: usize,
    pub commands_failed: usize,
    pub duration_ms: u64,
    pub cached: bool,
}

/// Risk summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub level: RiskLevel,
    pub level_string: String,
    pub requires_approval: bool,
    pub reasons: Vec<String>,
}

/// Review summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub issues_found: usize,
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub passed: bool,
}

/// Rollback status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackStatus {
    pub available: bool,
    pub checkpoint_created: bool,
    pub rolled_back: bool,
    pub rollback_reason: Option<String>,
}

/// Execution metrics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionMetrics {
    pub total_duration_ms: u64,
    pub repo_analysis_ms: u64,
    pub patch_generation_ms: u64,
    pub validation_ms: u64,
    pub step_count: u32,
}

/// Generate a human-readable patch report
pub fn generate_patch_report(result: &HarnessExecutionResult) -> PatchReport {
    let work_context_id = result.work_context_id.clone();

    // Collect changed files
    let changed_files = if let Some(ref patch) = result.patch_result {
        patch.changed_files.iter()
            .map(|path| FileChange {
                path: path.display().to_string(),
                operation: "modified".into(),
                lines_added: result.execution_metrics.lines_changed,
                lines_removed: 0, // Would need actual diff parsing
            })
            .collect()
    } else {
        vec![]
    };

    // Diff stats
    let diff_stats = if let Some(ref patch) = result.patch_result {
        DiffStats {
            files_changed: patch.changed_files.len(),
            insertions: patch.diff.lines().filter(|l| l.starts_with('+')).count(),
            deletions: patch.diff.lines().filter(|l| l.starts_with('-')).count(),
            total_lines_changed: patch.diff.lines().count(),
        }
    } else {
        DiffStats::default()
    };

    // Validation summary
    let validation = if let Some(ref val) = result.validation_result {
        ValidationSummary {
            passed: val.passed,
            commands_run: val.command_results.len(),
            commands_passed: val.command_results.iter().filter(|r| r.exit_code == Some(0)).count(),
            commands_failed: val.command_results.iter().filter(|r| r.exit_code != Some(0)).count(),
            duration_ms: val.duration_ms,
            cached: val.cached,
        }
    } else {
        ValidationSummary {
            passed: false,
            commands_run: 0,
            commands_passed: 0,
            commands_failed: 0,
            duration_ms: 0,
            cached: false,
        }
    };

    // Risk summary
    let risk = RiskSummary {
        level: result.risk_assessment.level.clone(),
        level_string: format!("{:?}", result.risk_assessment.level),
        requires_approval: result.risk_assessment.requires_approval,
        reasons: result.risk_assessment.reasons.iter()
            .map(|r| r.description.clone())
            .collect(),
    };

    // Review summary
    let critical = result.review_issues.iter().filter(|i| matches!(i.severity, crate::harness::review::ReviewSeverity::Critical)).count();
    let high = result.review_issues.iter().filter(|i| matches!(i.severity, crate::harness::review::ReviewSeverity::High)).count();
    let medium = result.review_issues.iter().filter(|i| matches!(i.severity, crate::harness::review::ReviewSeverity::Medium)).count();
    let low = result.review_issues.iter().filter(|i| matches!(i.severity, crate::harness::review::ReviewSeverity::Low)).count();

    let review = ReviewSummary {
        issues_found: result.review_issues.len(),
        critical,
        high,
        medium,
        low,
        passed: critical == 0,
    };

    // Rollback status
    let rollback = RollbackStatus {
        available: result.git_checkpoint.is_some(),
        checkpoint_created: result.git_checkpoint.is_some(),
        rolled_back: result.failures.contains(&FailureKind::PatchRolledBack),
        rollback_reason: if result.failures.contains(&FailureKind::PatchRolledBack) {
            Some("Validation failed - automatic rollback".into())
        } else {
            None
        },
    };

    // Generate summary
    let summary = format!(
        "Patch execution {} with {} files changed, {} validation commands run ({} passed). \
         Risk level: {:?}. Review: {} critical, {} high issues.",
        if matches!(result.completion_decision, crate::harness::completion::CompletionDecision::Complete) { "completed" } else { "blocked" },
        diff_stats.files_changed,
        validation.commands_run,
        validation.commands_passed,
        risk.level,
        review.critical,
        review.high
    );

    PatchReport {
        title: format!("Patch Report: {}", work_context_id),
        work_context_id,
        summary,
        changed_files,
        diff_stats,
        validation,
        risk,
        review,
        rollback,
        evidence_trace: result.evidence_log.entries.clone(),
        decision: format!("{:?}", result.completion_decision),
        metrics: ExecutionMetrics {
            total_duration_ms: result.execution_metrics.total_duration_ms,
            repo_analysis_ms: result.execution_metrics.repo_analysis_ms,
            patch_generation_ms: result.execution_metrics.patch_generation_ms,
            validation_ms: result.execution_metrics.validation_ms,
            step_count: result.step_count,
        },
    }
}

/// Generate Markdown report
pub fn generate_markdown_report(report: &PatchReport) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {}\n\n", report.title));
    md.push_str(&format!("**Work Context:** `{}`\n\n", report.work_context_id));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str(&format!("{}\n\n", report.summary));

    // Changed Files
    md.push_str("## Changed Files\n\n");
    if report.changed_files.is_empty() {
        md.push_str("*No files were changed*\n\n");
    } else {
        md.push_str("| File | Operation | Lines +/- |\n");
        md.push_str("|------|-----------|-----------|\n");
        for file in &report.changed_files {
            md.push_str(&format!(
                "| `{}` | {} | +{}/-{} |\n",
                file.path, file.operation, file.lines_added, file.lines_removed
            ));
        }
        md.push('\n');
    }

    // Diff Stats
    md.push_str("## Diff Statistics\n\n");
    md.push_str(&format!("- **Files changed:** {}\n", report.diff_stats.files_changed));
    md.push_str(&format!("- **Insertions:** {}\n", report.diff_stats.insertions));
    md.push_str(&format!("- **Deletions:** {}\n", report.diff_stats.deletions));
    md.push('\n');

    // Validation
    md.push_str("## Validation\n\n");
    md.push_str(&format!("**Status:** {}\n\n", if report.validation.passed { "✅ PASSED" } else { "❌ FAILED" }));
    md.push_str(&format!("- **Commands run:** {}\n", report.validation.commands_run));
    md.push_str(&format!("- **Passed:** {}\n", report.validation.commands_passed));
    md.push_str(&format!("- **Failed:** {}\n", report.validation.commands_failed));
    md.push_str(&format!("- **Duration:** {}ms\n", report.validation.duration_ms));
    md.push_str(&format!("- **Cached:** {}\n", if report.validation.cached { "Yes" } else { "No" }));
    md.push('\n');

    // Risk
    md.push_str("## Risk Assessment\n\n");
    let risk_emoji = match report.risk.level {
        RiskLevel::None | RiskLevel::Low => "🟢",
        RiskLevel::Medium => "🟡",
        RiskLevel::High => "🟠",
        RiskLevel::Critical => "🔴",
    };
    md.push_str(&format!("**Level:** {} {:?}\n\n", risk_emoji, report.risk.level));
    md.push_str(&format!("**Requires Approval:** {}\n\n", if report.risk.requires_approval { "Yes" } else { "No" }));
    if !report.risk.reasons.is_empty() {
        md.push_str("**Reasons:**\n");
        for reason in &report.risk.reasons {
            md.push_str(&format!("- {}\n", reason));
        }
        md.push('\n');
    }

    // Review
    md.push_str("## Code Review\n\n");
    md.push_str(&format!("**Issues Found:** {}\n\n", report.review.issues_found));
    md.push_str(&format!("- 🔴 Critical: {}\n", report.review.critical));
    md.push_str(&format!("- 🟠 High: {}\n", report.review.high));
    md.push_str(&format!("- 🟡 Medium: {}\n", report.review.medium));
    md.push_str(&format!("- 🟢 Low: {}\n", report.review.low));
    md.push('\n');

    // Rollback
    md.push_str("## Rollback Status\n\n");
    md.push_str(&format!("**Checkpoint Created:** {}\n", if report.rollback.checkpoint_created { "Yes" } else { "No" }));
    md.push_str(&format!("**Rolled Back:** {}\n", if report.rollback.rolled_back { "Yes" } else { "No" }));
    if let Some(ref reason) = report.rollback.rollback_reason {
        md.push_str(&format!("**Reason:** {}\n", reason));
    }
    md.push('\n');

    // Decision
    md.push_str("## Decision\n\n");
    md.push_str(&format!("**{}**\n\n", report.decision));

    // Metrics
    md.push_str("## Execution Metrics\n\n");
    md.push_str(&format!("- **Total Duration:** {}ms\n", report.metrics.total_duration_ms));
    md.push_str(&format!("- **Repo Analysis:** {}ms\n", report.metrics.repo_analysis_ms));
    md.push_str(&format!("- **Patch Generation:** {}ms\n", report.metrics.patch_generation_ms));
    md.push_str(&format!("- **Validation:** {}ms\n", report.metrics.validation_ms));
    md.push_str(&format!("- **Steps:** {}\n", report.metrics.step_count));

    md
}

/// Generate JSON report
pub fn generate_json_report(report: &PatchReport) -> anyhow::Result<String> {
    serde_json::to_string_pretty(report).map_err(|e| e.into())
}
