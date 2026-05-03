//! Patch Minimality Enforcement - Issue #25
//! Ensures patches are minimal and focused on the specific issue

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MinimalityConfig {
    pub max_files_changed: usize,
    pub max_lines_per_file: usize,
    pub max_total_lines_changed: usize,
    pub require_focused_changes: bool,
    pub allow_unrelated_fixes: bool,
    pub max_context_lines: usize,
    pub enforce_single_concern: bool,
}

impl Default for MinimalityConfig {
    fn default() -> Self {
        Self {
            max_files_changed: 5,
            max_lines_per_file: 100,
            max_total_lines_changed: 200,
            require_focused_changes: true,
            allow_unrelated_fixes: false,
            max_context_lines: 3,
            enforce_single_concern: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchAnalysis {
    pub files_changed: Vec<FileChange>,
    pub total_lines_added: usize,
    pub total_lines_removed: usize,
    pub concerns_detected: Vec<String>,
    pub unrelated_changes: Vec<UnrelatedChange>,
    pub is_minimal: bool,
    pub violations: Vec<MinimalityViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChange {
    pub path: PathBuf,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub functions_modified: Vec<String>,
    pub change_type: ChangeType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    Fix,
    Feature,
    Refactor,
    Test,
    Doc,
    Config,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnrelatedChange {
    pub file: PathBuf,
    pub description: String,
    pub suggested_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MinimalityViolation {
    pub rule: String,
    pub severity: ViolationSeverity,
    pub description: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ViolationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct MinimalityEnforcer {
    config: MinimalityConfig,
    violation_history: Vec<PatchAnalysis>,
}

impl MinimalityEnforcer {
    pub fn new(config: MinimalityConfig) -> Self {
        Self {
            config,
            violation_history: Vec::new(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(MinimalityConfig::default())
    }

    pub fn analyze_patch(&mut self, patch_content: &str, target_issue: &str) -> PatchAnalysis {
        let files_changed = self.parse_patch_files(patch_content);
        let total_lines_added: usize = files_changed.iter().map(|f| f.lines_added).sum();
        let total_lines_removed: usize = files_changed.iter().map(|f| f.lines_removed).sum();

        let concerns_detected = self.detect_concerns(&files_changed, target_issue);
        let unrelated_changes = self.identify_unrelated_changes(&files_changed, target_issue);
        let violations = self.check_violations(
            &files_changed,
            total_lines_added,
            total_lines_removed,
            &concerns_detected,
            &unrelated_changes,
        );

        let is_minimal = violations
            .iter()
            .all(|v| matches!(v.severity, ViolationSeverity::Warning));

        let analysis = PatchAnalysis {
            files_changed,
            total_lines_added,
            total_lines_removed,
            concerns_detected,
            unrelated_changes,
            is_minimal,
            violations,
        };

        self.violation_history.push(analysis.clone());
        analysis
    }

    pub fn enforce(&mut self, patch_content: &str, target_issue: &str) -> Result<()> {
        let analysis = self.analyze_patch(patch_content, target_issue);

        let errors: Vec<_> = analysis
            .violations
            .iter()
            .filter(|v| matches!(v.severity, ViolationSeverity::Error))
            .collect();

        if !errors.is_empty() {
            let error_msg = errors
                .iter()
                .map(|v| format!("{}: {}", v.rule, v.description))
                .collect::<Vec<_>>()
                .join("\n");
            bail!("Patch minimality violations:\n{}", error_msg);
        }

        Ok(())
    }

    fn parse_patch_files(&self, patch_content: &str) -> Vec<FileChange> {
        let mut files = Vec::new();
        let mut current_file: Option<FileChange> = None;
        let mut lines_added = 0;
        let mut lines_removed = 0;

        for line in patch_content.lines() {
            if line.starts_with("+++") {
                if let Some(file) = current_file.take() {
                    files.push(FileChange {
                        lines_added,
                        lines_removed,
                        ..file
                    });
                }

                let path = line
                    .strip_prefix("+++ b/")
                    .or_else(|| line.strip_prefix("+++ "))
                    .unwrap_or("unknown");

                current_file = Some(FileChange {
                    path: PathBuf::from(path),
                    lines_added: 0,
                    lines_removed: 0,
                    functions_modified: Vec::new(),
                    change_type: ChangeType::Unknown,
                });
                lines_added = 0;
                lines_removed = 0;
            } else if line.starts_with('+') && !line.starts_with("+++") {
                lines_added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                lines_removed += 1;
            }
        }

        if let Some(file) = current_file {
            files.push(FileChange {
                lines_added,
                lines_removed,
                ..file
            });
        }

        files
    }

    fn detect_concerns(&self, files: &[FileChange], target_issue: &str) -> Vec<String> {
        let mut concerns = HashSet::new();

        for file in files {
            // Detect concern from file path and content
            if file.path.extension().map(|e| e == "rs").unwrap_or(false) {
                concerns.insert("code_change".to_string());
            }
            if file.path.to_string_lossy().contains("test") {
                concerns.insert("test_change".to_string());
            }
            if file.path.to_string_lossy().contains("doc") {
                concerns.insert("documentation".to_string());
            }
            if file.path.to_string_lossy().contains("config")
                || file
                    .path
                    .extension()
                    .map(|e| e == "toml" || e == "yaml" || e == "json")
                    .unwrap_or(false)
            {
                concerns.insert("configuration".to_string());
            }
        }

        // Check if addressing target issue
        if !target_issue.is_empty() {
            concerns.insert(format!("issue_{}", target_issue));
        }

        concerns.into_iter().collect()
    }

    fn identify_unrelated_changes(
        &self,
        files: &[FileChange],
        target_issue: &str,
    ) -> Vec<UnrelatedChange> {
        let mut unrelated = Vec::new();

        if self.config.allow_unrelated_fixes {
            return unrelated;
        }

        for file in files {
            // Check if change is unrelated to target issue
            let file_str = file.path.to_string_lossy();

            // Style-only changes
            if file.lines_added == file.lines_removed && file.lines_added < 5 {
                unrelated.push(UnrelatedChange {
                    file: file.path.clone(),
                    description: "Possible style-only change".to_string(),
                    suggested_action: "Remove if not related to the fix".to_string(),
                });
            }

            // Refactoring in unrelated files
            if file_str.contains("refactor")
                || (file.change_type == ChangeType::Refactor && !file_str.contains(target_issue))
            {
                unrelated.push(UnrelatedChange {
                    file: file.path.clone(),
                    description: "Refactoring in unrelated file".to_string(),
                    suggested_action: "Move to separate PR".to_string(),
                });
            }
        }

        unrelated
    }

    fn check_violations(
        &self,
        files: &[FileChange],
        total_added: usize,
        total_removed: usize,
        concerns: &[String],
        unrelated: &[UnrelatedChange],
    ) -> Vec<MinimalityViolation> {
        let mut violations = Vec::new();

        // Check file count
        if files.len() > self.config.max_files_changed {
            violations.push(MinimalityViolation {
                rule: "max_files".to_string(),
                severity: ViolationSeverity::Error,
                description: format!(
                    "Too many files changed: {} (max: {})",
                    files.len(),
                    self.config.max_files_changed
                ),
                suggestion: "Split into multiple focused PRs".to_string(),
            });
        }

        // Check lines per file
        for file in files {
            let file_total = file.lines_added + file.lines_removed;
            if file_total > self.config.max_lines_per_file {
                violations.push(MinimalityViolation {
                    rule: "max_lines_per_file".to_string(),
                    severity: ViolationSeverity::Error,
                    description: format!(
                        "File {} has {} lines changed (max: {})",
                        file.path.display(),
                        file_total,
                        self.config.max_lines_per_file
                    ),
                    suggestion: "Reduce scope of changes".to_string(),
                });
            }
        }

        // Check total lines
        let total_lines = total_added + total_removed;
        if total_lines > self.config.max_total_lines_changed {
            violations.push(MinimalityViolation {
                rule: "max_total_lines".to_string(),
                severity: ViolationSeverity::Warning,
                description: format!(
                    "Total lines changed: {} (recommended max: {})",
                    total_lines, self.config.max_total_lines_changed
                ),
                suggestion: "Consider splitting changes".to_string(),
            });
        }

        // Check single concern
        if self.config.enforce_single_concern && concerns.len() > 2 {
            violations.push(MinimalityViolation {
                rule: "single_concern".to_string(),
                severity: ViolationSeverity::Warning,
                description: format!("Multiple concerns detected: {}", concerns.join(", ")),
                suggestion: "Focus on one concern per patch".to_string(),
            });
        }

        // Check unrelated changes
        if !unrelated.is_empty() {
            for change in unrelated {
                violations.push(MinimalityViolation {
                    rule: "unrelated_change".to_string(),
                    severity: ViolationSeverity::Warning,
                    description: format!("{} in {}", change.description, change.file.display()),
                    suggestion: change.suggested_action.clone(),
                });
            }
        }

        violations
    }

    pub fn get_stats(&self) -> MinimalityStats {
        let total_analyzed = self.violation_history.len();
        let minimal_count = self
            .violation_history
            .iter()
            .filter(|a| a.is_minimal)
            .count();

        MinimalityStats {
            total_patches_analyzed: total_analyzed,
            minimal_patches: minimal_count,
            violation_rate: if total_analyzed > 0 {
                (total_analyzed - minimal_count) as f64 / total_analyzed as f64
            } else {
                0.0
            },
            average_violations_per_patch: if total_analyzed > 0 {
                self.violation_history
                    .iter()
                    .map(|a| a.violations.len())
                    .sum::<usize>() as f64
                    / total_analyzed as f64
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalityStats {
    pub total_patches_analyzed: usize,
    pub minimal_patches: usize,
    pub violation_rate: f64,
    pub average_violations_per_patch: f64,
}

pub fn analyze_patch_minimality(patch_content: &str, target_issue: &str) -> PatchAnalysis {
    let mut enforcer = MinimalityEnforcer::with_defaults();
    enforcer.analyze_patch(patch_content, target_issue)
}

pub fn enforce_patch_minimality(patch_content: &str, target_issue: &str) -> Result<()> {
    let mut enforcer = MinimalityEnforcer::with_defaults();
    enforcer.enforce(patch_content, target_issue)
}

pub fn format_analysis_report(analysis: &PatchAnalysis) -> String {
    let status = if analysis.is_minimal {
        "✓ Minimal"
    } else {
        "✗ Not Minimal"
    };

    let violations_str = if analysis.violations.is_empty() {
        "No violations found.\n".to_string()
    } else {
        analysis
            .violations
            .iter()
            .map(|v| {
                format!(
                    "  - [{}] {}: {}",
                    match v.severity {
                        ViolationSeverity::Error => "ERROR",
                        ViolationSeverity::Warning => "WARN",
                    },
                    v.rule,
                    v.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"Patch Minimality Analysis
==========================
Status: {}
Files Changed: {}
Lines Added: {}
Lines Removed: {}
Concerns: {}

Violations:
{}
"#,
        status,
        analysis.files_changed.len(),
        analysis.total_lines_added,
        analysis.total_lines_removed,
        analysis.concerns_detected.join(", "),
        violations_str
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_small_patch() {
        let patch = r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,5 +10,5 @@ fn foo() {
-    let x = 1;
+    let x = 2;
 }"#;

        let analysis = analyze_patch_minimality(patch, "fix-123");
        assert!(analysis.is_minimal);
        assert_eq!(analysis.files_changed.len(), 1);
    }

    #[test]
    fn test_detects_large_patch() {
        let mut config = MinimalityConfig::default();
        config.max_lines_per_file = 5;

        let patch = r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,100 +1,100 @@
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines
+// Many lines"#;

        let mut enforcer = MinimalityEnforcer::new(config);
        let analysis = enforcer.analyze_patch(patch, "fix-123");

        assert!(!analysis.is_minimal);
        assert!(
            analysis
                .violations
                .iter()
                .any(|v| v.rule == "max_lines_per_file")
        );
    }

    #[test]
    fn test_enforce_rejects_violations() {
        let mut config = MinimalityConfig::default();
        config.max_files_changed = 1;

        let patch = r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-foo
+bar
diff --git a/src/other.rs b/src/other.rs
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-foo
+bar"#;

        let mut enforcer = MinimalityEnforcer::new(config);
        let result = enforcer.enforce(patch, "fix-123");
        assert!(result.is_err());
    }
}
