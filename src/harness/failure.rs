use crate::harness::{
    patch_applier::PatchFailure,
    validation::{CommandResult, ValidationCategory, ValidationResult},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FailureKind {
    LocalizationFailure,
    PatchParseFailure,
    PatchApplyFailure,
    CompileFailure,
    TestFailure,
    RegressionFailure,
    SemanticFailure,
    TimeoutFailure,
    PermissionFailure,
    ModelFailure,
    ToolFailure,
    SandboxFailure,
    NetworkFailure,
    ResourceExhaustion,
    UnknownFailure,
    Fatal,
    Critical,
    ValidationFailed,
    PatchRolledBack,
    RollbackFailed,
    CheckpointFailed,
    SyntaxError,
}

impl FailureKind {
    /// Returns true if this failure kind is critical
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            Self::Fatal | Self::Critical | Self::SandboxFailure | Self::PermissionFailure
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureDetails {
    pub kind: FailureKind,
    pub category: FailureCategory,
    pub severity: FailureSeverity,
    pub message: String,
    pub context: FailureContext,
    pub suggestion: Option<String>,
    pub recovery_action: RecoveryAction,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureCategory {
    Syntax,
    Semantic,
    Environmental,
    Tooling,
    Resource,
    Logic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureSeverity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
    Fatal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FailureContext {
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub operation: Option<String>,
    pub command: Option<String>,
    pub nearby_code: Option<String>,
    pub stack_trace: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryAction {
    Retry,
    RetryWithBackoff,
    Skip,
    PromptUser,
    UseAlternative,
    Abort,
    None,
}

impl FailureKind {
    pub fn category(&self) -> FailureCategory {
        match self {
            FailureKind::LocalizationFailure => FailureCategory::Logic,
            FailureKind::PatchParseFailure => FailureCategory::Syntax,
            FailureKind::PatchApplyFailure => FailureCategory::Syntax,
            FailureKind::CompileFailure => FailureCategory::Syntax,
            FailureKind::TestFailure => FailureCategory::Semantic,
            FailureKind::RegressionFailure => FailureCategory::Semantic,
            FailureKind::SemanticFailure => FailureCategory::Semantic,
            FailureKind::TimeoutFailure => FailureCategory::Resource,
            FailureKind::PermissionFailure => FailureCategory::Environmental,
            FailureKind::ModelFailure => FailureCategory::Tooling,
            FailureKind::ToolFailure => FailureCategory::Tooling,
            FailureKind::SandboxFailure => FailureCategory::Environmental,
            FailureKind::NetworkFailure => FailureCategory::Environmental,
            FailureKind::ResourceExhaustion => FailureCategory::Resource,
            FailureKind::UnknownFailure => FailureCategory::Tooling,
            FailureKind::Fatal => FailureCategory::Tooling,
            FailureKind::Critical => FailureCategory::Tooling,
            FailureKind::ValidationFailed => FailureCategory::Semantic,
            FailureKind::PatchRolledBack => FailureCategory::Tooling,
            FailureKind::RollbackFailed => FailureCategory::Tooling,
            FailureKind::CheckpointFailed => FailureCategory::Environmental,
            FailureKind::SyntaxError => FailureCategory::Syntax,
        }
    }

    pub fn default_severity(&self) -> FailureSeverity {
        match self {
            FailureKind::Fatal => FailureSeverity::Fatal,
            FailureKind::Critical => FailureSeverity::Critical,
            FailureKind::PatchParseFailure => FailureSeverity::Error,
            FailureKind::PatchApplyFailure => FailureSeverity::Error,
            FailureKind::CompileFailure => FailureSeverity::Error,
            FailureKind::TestFailure => FailureSeverity::Error,
            FailureKind::RegressionFailure => FailureSeverity::Critical,
            FailureKind::TimeoutFailure => FailureSeverity::Warning,
            FailureKind::PermissionFailure => FailureSeverity::Error,
            FailureKind::NetworkFailure => FailureSeverity::Warning,
            FailureKind::ResourceExhaustion => FailureSeverity::Critical,
            FailureKind::SandboxFailure => FailureSeverity::Error,
            FailureKind::ValidationFailed => FailureSeverity::Error,
            FailureKind::PatchRolledBack => FailureSeverity::Warning,
            FailureKind::RollbackFailed => FailureSeverity::Critical,
            FailureKind::CheckpointFailed => FailureSeverity::Error,
            FailureKind::SyntaxError => FailureSeverity::Error,
            _ => FailureSeverity::Info,
        }
    }

    pub fn default_recovery(&self) -> RecoveryAction {
        match self {
            FailureKind::TimeoutFailure => RecoveryAction::RetryWithBackoff,
            FailureKind::NetworkFailure => RecoveryAction::RetryWithBackoff,
            FailureKind::PermissionFailure => RecoveryAction::PromptUser,
            FailureKind::SandboxFailure => RecoveryAction::UseAlternative,
            FailureKind::ResourceExhaustion => RecoveryAction::Abort,
            FailureKind::ToolFailure => RecoveryAction::UseAlternative,
            FailureKind::ValidationFailed => RecoveryAction::Retry,
            FailureKind::PatchRolledBack => RecoveryAction::Retry,
            FailureKind::RollbackFailed => RecoveryAction::Abort,
            FailureKind::SyntaxError => RecoveryAction::Retry,
            _ => RecoveryAction::Retry,
        }
    }
}

pub fn classify_patch_failure(failure: &PatchFailure) -> FailureKind {
    let reason_lower = failure.reason.to_lowercase();
    let operation_lower = failure.operation.to_lowercase();

    if operation_lower.contains("search") {
        if reason_lower.contains("not found") || reason_lower.contains("0 times") {
            return FailureKind::PatchApplyFailure;
        }
        if reason_lower.contains("matched") && reason_lower.contains("times") {
            return FailureKind::PatchApplyFailure;
        }
    }

    if operation_lower.contains("create") {
        if reason_lower.contains("exists") {
            return FailureKind::PatchApplyFailure;
        }
    }

    if operation_lower.contains("delete") || operation_lower.contains("rename") {
        if reason_lower.contains("not exist") || reason_lower.contains("cannot find") {
            return FailureKind::PatchApplyFailure;
        }
    }

    if reason_lower.contains("denied") || reason_lower.contains("permission") {
        return FailureKind::PermissionFailure;
    }

    if reason_lower.contains("timeout")
        || failure.line_number.is_none() && reason_lower.contains("took too long")
    {
        return FailureKind::TimeoutFailure;
    }

    if operation_lower.contains("diff") {
        return FailureKind::PatchParseFailure;
    }

    FailureKind::PatchApplyFailure
}

pub fn classify_validation_failure(result: &ValidationResult) -> FailureKind {
    let has_compile_errors = result
        .category_results
        .get(&ValidationCategory::Lint)
        .map(|r| !r.passed)
        .unwrap_or(false);

    let has_test_failures = result
        .category_results
        .get(&ValidationCategory::Test)
        .map(|r| !r.passed)
        .unwrap_or(false);

    if has_compile_errors && !has_test_failures {
        return FailureKind::CompileFailure;
    }

    if has_test_failures {
        for cmd in &result.command_results {
            let stderr_lower = cmd.stderr.to_lowercase();
            if stderr_lower.contains("regression") || stderr_lower.contains("previously passed") {
                return FailureKind::RegressionFailure;
            }
            if stderr_lower.contains("semantic") || stderr_lower.contains("logic") {
                return FailureKind::SemanticFailure;
            }
            if stderr_lower.contains("timeout") || cmd.timed_out {
                return FailureKind::TimeoutFailure;
            }
            if stderr_lower.contains("permission") || stderr_lower.contains("denied") {
                return FailureKind::PermissionFailure;
            }
        }
        return FailureKind::TestFailure;
    }

    if result.command_results.iter().any(|r| r.timed_out) {
        return FailureKind::TimeoutFailure;
    }

    FailureKind::UnknownFailure
}

pub fn classify_command_failure(cmd: &CommandResult) -> FailureKind {
    let stderr_lower = cmd.stderr.to_lowercase();
    let stdout_lower = cmd.stdout.to_lowercase();

    if cmd.timed_out {
        return FailureKind::TimeoutFailure;
    }

    if stderr_lower.contains("compile error") || stderr_lower.contains("syntax error") {
        return FailureKind::CompileFailure;
    }

    if stderr_lower.contains("test failed") || stdout_lower.contains("test result: failed") {
        return FailureKind::TestFailure;
    }

    if stderr_lower.contains("permission denied") || stderr_lower.contains("access denied") {
        return FailureKind::PermissionFailure;
    }

    if stderr_lower.contains("no such file") || stderr_lower.contains("not found") {
        return FailureKind::ToolFailure;
    }

    if stderr_lower.contains("out of memory")
        || stderr_lower.contains("resource temporarily unavailable")
    {
        return FailureKind::ResourceExhaustion;
    }

    if stderr_lower.contains("network") || stderr_lower.contains("connection") {
        return FailureKind::NetworkFailure;
    }

    FailureKind::UnknownFailure
}

pub fn analyze_failure_pattern(failures: &[FailureKind]) -> FailurePattern {
    let mut counts: HashMap<FailureKind, usize> = HashMap::new();
    for f in failures {
        *counts.entry(*f).or_insert(0) += 1;
    }

    let most_common = counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(kind, _)| *kind)
        .unwrap_or(FailureKind::UnknownFailure);

    let total = failures.len();
    let unique = counts.len();

    let concentration = if total > 0 {
        let max_count = counts.values().max().copied().unwrap_or(0);
        max_count as f64 / total as f64
    } else {
        0.0
    };

    let is_systematic = unique <= 2 && concentration >= 0.8;
    let is_intermittent = unique > 3 || concentration < 0.5;

    FailurePattern {
        most_common,
        total_failures: total,
        unique_kinds: unique,
        concentration,
        is_systematic,
        is_intermittent,
        kind_distribution: counts,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePattern {
    pub most_common: FailureKind,
    pub total_failures: usize,
    pub unique_kinds: usize,
    pub concentration: f64,
    pub is_systematic: bool,
    pub is_intermittent: bool,
    pub kind_distribution: HashMap<FailureKind, usize>,
}

pub fn create_failure_details(
    kind: FailureKind,
    message: impl Into<String>,
    context: FailureContext,
) -> FailureDetails {
    let msg = message.into();
    let suggestion = generate_suggestion(&kind, &msg, &context);

    FailureDetails {
        kind,
        category: kind.category(),
        severity: kind.default_severity(),
        message: msg,
        context,
        suggestion,
        recovery_action: kind.default_recovery(),
    }
}

fn generate_suggestion(
    kind: &FailureKind,
    message: &str,
    context: &FailureContext,
) -> Option<String> {
    match kind {
        FailureKind::PatchApplyFailure => {
            if message.contains("matched 0 times") {
                Some("The search pattern was not found in the file. Verify the exact text including whitespace and line endings.".into())
            } else if message.contains("matched") && message.contains("times") {
                Some("The search pattern appears multiple times. Consider using a more specific pattern or set replace_all:true.".into())
            } else {
                Some(
                    "Check file permissions and ensure the target file exists and is writable."
                        .into(),
                )
            }
        }
        FailureKind::CompileFailure => {
            if let Some(ref file) = context.file {
                Some(format!(
                    "Check syntax in {} and verify all imports/dependencies are available.",
                    file.display()
                ))
            } else {
                Some("Review the code for syntax errors or missing dependencies.".into())
            }
        }
        FailureKind::TestFailure => Some(
            "Review test output to identify the failing assertions and expected vs actual values."
                .into(),
        ),
        FailureKind::TimeoutFailure => Some(
            "Consider increasing timeout limits or breaking the operation into smaller chunks."
                .into(),
        ),
        FailureKind::PermissionFailure => Some(
            "Check file permissions and ensure the process has appropriate access rights.".into(),
        ),
        FailureKind::NetworkFailure => {
            Some("Check network connectivity and retry with exponential backoff.".into())
        }
        FailureKind::ResourceExhaustion => {
            Some("Free up system resources or reduce the scope of the operation.".into())
        }
        _ => None,
    }
}

pub fn format_failure_report(details: &FailureDetails) -> String {
    let mut report = format!(
        "[{:?}] {:?} Failure: {}\n",
        details.severity, details.kind, details.message
    );

    report.push_str(&format!("  Category: {:?}\n", details.category));

    if let Some(ref file) = details.context.file {
        report.push_str(&format!("  File: {}\n", file.display()));
    }
    if let Some(line) = details.context.line {
        report.push_str(&format!("  Line: {}\n", line));
    }
    if let Some(ref operation) = details.context.operation {
        report.push_str(&format!("  Operation: {}\n", operation));
    }

    report.push_str(&format!("  Recovery: {:?}\n", details.recovery_action));

    if let Some(ref suggestion) = details.suggestion {
        report.push_str(&format!("  Suggestion: {}\n", suggestion));
    }

    report
}

pub struct FailureAggregator {
    failures: Vec<FailureDetails>,
    max_failures: usize,
}

impl FailureAggregator {
    pub fn new(max_failures: usize) -> Self {
        Self {
            failures: Vec::new(),
            max_failures,
        }
    }

    pub fn add(&mut self, details: FailureDetails) {
        if self.failures.len() < self.max_failures {
            self.failures.push(details);
        }
    }

    pub fn get_critical(&self) -> Vec<&FailureDetails> {
        self.failures
            .iter()
            .filter(|f| f.severity >= FailureSeverity::Error)
            .collect()
    }

    pub fn get_by_kind(&self, kind: FailureKind) -> Vec<&FailureDetails> {
        self.failures.iter().filter(|f| f.kind == kind).collect()
    }

    pub fn has_fatal(&self) -> bool {
        self.failures
            .iter()
            .any(|f| f.severity == FailureSeverity::Fatal)
    }

    pub fn generate_report(&self) -> String {
        if self.failures.is_empty() {
            return "No failures recorded.".into();
        }

        let mut report = format!("Failure Report ({} total):\n", self.failures.len());
        report.push_str("=".repeat(40).as_str());
        report.push('\n');

        for (i, failure) in self.failures.iter().enumerate() {
            report.push_str(&format!("\n[{}] ", i + 1));
            report.push_str(&format_failure_report(failure));
        }

        report
    }
}
