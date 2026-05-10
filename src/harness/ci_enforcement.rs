//! P1-5.2: Strict Anti-Placeholder CI Enforcement
//!
//! This module provides comprehensive CI enforcement to prevent placeholder code,
//! TODO comments, and other anti-patterns from being merged into production.

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Strict anti-placeholder CI enforcement system
#[derive(Debug, Clone)]
pub struct AntiPlaceholderCI {
    config: CIConfig,
    patterns: Vec<PlaceholderPattern>,
}

/// Configuration for anti-placeholder CI enforcement
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CIConfig {
    /// Whether to enforce strict mode (fail on any violation)
    pub strict_mode: bool,
    /// Maximum number of TODO comments allowed
    pub max_todo_comments: usize,
    /// Maximum number of placeholder variables allowed
    pub max_placeholder_vars: usize,
    /// Whether to check for unimplemented functions
    pub check_unimplemented: bool,
    /// Whether to check for panic! calls
    pub check_panics: bool,
    /// Whether to check for debug print statements
    pub check_debug_prints: bool,
    /// File patterns to exclude from checks
    pub exclude_patterns: Vec<String>,
    /// Custom patterns to check for
    pub custom_patterns: Vec<CustomPattern>,
}

/// A placeholder pattern that should be detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaceholderPattern {
    /// Pattern name for reporting
    pub name: String,
    /// Regex pattern to match
    pub pattern: String,
    /// Severity level
    pub severity: Severity,
    /// Description of why this is problematic
    pub description: String,
    /// Suggested fix
    pub suggestion: String,
}

/// Custom pattern configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomPattern {
    /// Pattern name
    pub name: String,
    /// Regex pattern
    pub pattern: String,
    /// Severity level
    pub severity: Severity,
    /// Description
    pub description: String,
    /// Suggested fix
    pub suggestion: String,
}

/// Severity levels for violations
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Critical - must be fixed
    Critical = 3,
    /// High - should be fixed
    High = 2,
    /// Medium - nice to fix
    Medium = 1,
    /// Low - informational
    Low = 0,
}

/// Result of CI enforcement check
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CIEnforcementResult {
    /// Overall pass/fail status
    pub passed: bool,
    /// Total violations found
    pub total_violations: usize,
    /// Violations by severity
    pub violations_by_severity: HashMap<Severity, usize>,
    /// Detailed violation reports
    pub violations: Vec<PlaceholderViolation>,
    /// Files checked
    pub files_checked: usize,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
}

/// A single placeholder violation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaceholderViolation {
    /// File where violation was found
    pub file_path: PathBuf,
    /// Line number
    pub line_number: usize,
    /// Column number
    pub column_number: usize,
    /// Pattern that was violated
    pub pattern_name: String,
    /// Severity level
    pub severity: Severity,
    /// Description of the violation
    pub description: String,
    /// Suggested fix
    pub suggestion: String,
    /// The actual violating line content
    pub line_content: String,
    /// Context lines before and after
    pub context_lines: Vec<String>,
}

impl Default for CIConfig {
    fn default() -> Self {
        Self {
            strict_mode: true,
            max_todo_comments: 0,
            max_placeholder_vars: 0,
            check_unimplemented: true,
            check_panics: true,
            check_debug_prints: true,
            exclude_patterns: vec![
                "**/target/**".to_string(),
                "**/node_modules/**".to_string(),
                "**/.git/**".to_string(),
                "**/tests/**".to_string(),
                "**/test/**".to_string(),
                "**/examples/**".to_string(),
            ],
            custom_patterns: vec![],
        }
    }
}

impl AntiPlaceholderCI {
    /// Create a new anti-placeholder CI enforcement system
    pub fn new(config: CIConfig) -> Result<Self> {
        let mut ci = Self {
            config,
            patterns: Vec::new(),
        };
        ci.initialize_patterns()?;
        Ok(ci)
    }

    /// Create with default configuration
    pub fn with_defaults() -> Result<Self> {
        Self::new(CIConfig::default())
    }

    /// Initialize default placeholder patterns
    fn initialize_patterns(&mut self) -> Result<()> {
        self.patterns.clear();

        // TODO comments
        self.patterns.push(PlaceholderPattern {
            name: "todo_comment".to_string(),
            pattern: r"//\s*TODO\b|//\s*todo\b".to_string(),
            severity: Severity::Medium,
            description: "TODO comment found".to_string(),
            suggestion: "Replace TODO with actual implementation or create a proper issue"
                .to_string(),
        });

        // Placeholder variables
        self.patterns.push(PlaceholderPattern {
            name: "placeholder_var".to_string(),
            pattern: r"\b(placeholder|dummy|fake|mock|stub)_\w+\b".to_string(),
            severity: Severity::High,
            description: "Placeholder variable name detected".to_string(),
            suggestion: "Use descriptive variable names that reflect actual purpose".to_string(),
        });

        // Unimplemented functions
        if self.config.check_unimplemented {
            self.patterns.push(PlaceholderPattern {
                name: "unimplemented".to_string(),
                pattern: r"\bunimplemented!\(\)|\btodo!\(\)".to_string(),
                severity: Severity::Critical,
                description: "Unimplemented function or method".to_string(),
                suggestion: "Implement the function or remove it if not needed".to_string(),
            });
        }

        // Panic calls in production code
        if self.config.check_panics {
            self.patterns.push(PlaceholderPattern {
                name: "panic_call".to_string(),
                pattern: r"\bpanic!\(\)".to_string(),
                severity: Severity::High,
                description: "Panic call in production code".to_string(),
                suggestion: "Replace with proper error handling using Result or Option".to_string(),
            });
        }

        // Debug print statements
        if self.config.check_debug_prints {
            self.patterns.push(PlaceholderPattern {
                name: "debug_print".to_string(),
                pattern: r"\b(debug_println|print!|println!)\s*\(".to_string(),
                severity: Severity::Medium,
                description: "Debug print statement found".to_string(),
                suggestion: "Remove debug prints or use proper logging".to_string(),
            });
        }

        // Placeholder return values
        self.patterns.push(PlaceholderPattern {
            name: "placeholder_return".to_string(),
            pattern: r#"\b(return\s+(None|0|""|false|vec!\[\]|HashMap::new\(\)))"#.to_string(),
            severity: Severity::Medium,
            description: "Placeholder return value".to_string(),
            suggestion: "Return meaningful values or use proper error handling".to_string(),
        });

        // Hardcoded credentials or secrets
        self.patterns.push(PlaceholderPattern {
            name: "hardcoded_secret".to_string(),
            pattern: r#"(password|secret|key|token)\s*=\s*["'][^"']+["']"#.to_string(),
            severity: Severity::Critical,
            description: "Hardcoded secret or credential".to_string(),
            suggestion: "Use environment variables or secure configuration".to_string(),
        });

        // Placeholder URLs
        self.patterns.push(PlaceholderPattern {
            name: "placeholder_url".to_string(),
            pattern: r"https?://(localhost|127\.0\.0\.1|example\.com|placeholder\.com)".to_string(),
            severity: Severity::Medium,
            description: "Placeholder or localhost URL".to_string(),
            suggestion: "Use proper configuration for URLs".to_string(),
        });

        // Commented out code blocks
        self.patterns.push(PlaceholderPattern {
            name: "commented_code".to_string(),
            pattern: r"^\s*//\s*(fn|struct|impl|let|if|for|while)\b".to_string(),
            severity: Severity::Low,
            description: "Commented out code".to_string(),
            suggestion: "Remove commented code or uncomment if needed".to_string(),
        });

        Ok(())
    }

    /// Run anti-placeholder checks on a repository
    pub fn check_repository(&self, repo_path: &Path) -> Result<CIEnforcementResult> {
        let start_time = std::time::Instant::now();
        let mut violations = Vec::new();
        let mut files_checked = 0;

        for entry in WalkDir::new(repo_path)
            .follow_links(false)
            .max_depth(20)
            .into_iter()
            .filter_entry(|e| !self.should_exclude_entry(e))
        {
            let entry = entry.with_context(|| {
                format!("Failed to read directory entry in {}", repo_path.display())
            })?;

            if entry.file_type().is_file() && self.is_source_file(entry.path()) {
                files_checked += 1;
                if let Ok(file_violations) = self.check_file(entry.path()) {
                    violations.extend(file_violations);
                }
            }
        }

        let execution_time = start_time.elapsed().as_millis() as u64;
        let passed = self.evaluate_pass_status(&violations);

        let mut violations_by_severity = HashMap::new();
        for violation in &violations {
            *violations_by_severity
                .entry(violation.severity)
                .or_insert(0) += 1;
        }

        Ok(CIEnforcementResult {
            passed,
            total_violations: violations.len(),
            violations_by_severity,
            violations,
            files_checked,
            execution_time_ms: execution_time,
        })
    }

    /// Check a single file for violations
    fn check_file(&self, file_path: &Path) -> Result<Vec<PlaceholderViolation>> {
        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        let mut violations = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (line_num, line) in lines.iter().enumerate() {
            for pattern in &self.patterns {
                if let Ok(regex) = Regex::new(&pattern.pattern) {
                    for mat in regex.find_iter(line) {
                        let violation = PlaceholderViolation {
                            file_path: file_path.to_path_buf(),
                            line_number: line_num + 1,
                            column_number: mat.start() + 1,
                            pattern_name: pattern.name.clone(),
                            severity: pattern.severity,
                            description: pattern.description.clone(),
                            suggestion: pattern.suggestion.clone(),
                            line_content: line.to_string(),
                            context_lines: self.get_context_lines(&lines, line_num, 2),
                        };
                        violations.push(violation);
                    }
                }
            }
        }

        // Check custom patterns
        for custom_pattern in &self.config.custom_patterns {
            if let Ok(regex) = Regex::new(&custom_pattern.pattern) {
                for (line_num, line) in lines.iter().enumerate() {
                    for mat in regex.find_iter(line) {
                        let violation = PlaceholderViolation {
                            file_path: file_path.to_path_buf(),
                            line_number: line_num + 1,
                            column_number: mat.start() + 1,
                            pattern_name: custom_pattern.name.clone(),
                            severity: custom_pattern.severity,
                            description: custom_pattern.description.clone(),
                            suggestion: custom_pattern.suggestion.clone(),
                            line_content: line.to_string(),
                            context_lines: self.get_context_lines(&lines, line_num, 2),
                        };
                        violations.push(violation);
                    }
                }
            }
        }

        Ok(violations)
    }

    /// Get context lines around a violation
    fn get_context_lines(&self, lines: &[&str], line_num: usize, context: usize) -> Vec<String> {
        let start = line_num.saturating_sub(context);
        let end = (line_num + context + 1).min(lines.len());

        lines[start..end]
            .iter()
            .map(|&line| line.to_string())
            .collect()
    }

    /// Check if an entry should be excluded
    fn should_exclude_entry(&self, entry: &walkdir::DirEntry) -> bool {
        let path = entry.path();
        let path_str = path.to_string_lossy();

        for pattern in &self.config.exclude_patterns {
            if self.matches_pattern(&path_str, pattern) {
                return true;
            }
        }

        false
    }

    /// Check if a file is a source file that should be checked
    fn is_source_file(&self, path: &Path) -> bool {
        if let Some(ext) = path.extension() {
            matches!(
                ext.to_str(),
                Some("rs") | Some("js") | Some("ts") | Some("py") | Some("go") | Some("java")
            )
        } else {
            false
        }
    }

    /// Simple pattern matching for exclusions
    fn matches_pattern(&self, path: &str, pattern: &str) -> bool {
        if pattern.contains('*') {
            // Simple glob matching
            let pattern_parts: Vec<&str> = pattern.split('*').collect();
            let mut current_pos = 0;

            for part in pattern_parts {
                if part.is_empty() {
                    continue;
                }

                if let Some(pos) = path[current_pos..].find(part) {
                    current_pos += pos + part.len();
                } else {
                    return false;
                }
            }

            true
        } else {
            path.contains(pattern)
        }
    }

    /// Evaluate whether the check passes based on configuration
    fn evaluate_pass_status(&self, violations: &[PlaceholderViolation]) -> bool {
        if self.config.strict_mode {
            violations.is_empty()
        } else {
            // Count violations by type
            let todo_count = violations
                .iter()
                .filter(|v| v.pattern_name.contains("todo"))
                .count();

            let placeholder_count = violations
                .iter()
                .filter(|v| v.pattern_name.contains("placeholder"))
                .count();

            let critical_count = violations
                .iter()
                .filter(|v| v.severity == Severity::Critical)
                .count();

            // Fail if there are any critical violations
            if critical_count > 0 {
                return false;
            }

            // Fail if TODO or placeholder limits exceeded
            if todo_count > self.config.max_todo_comments {
                return false;
            }

            if placeholder_count > self.config.max_placeholder_vars {
                return false;
            }

            true
        }
    }

    /// Generate a detailed report
    pub fn generate_report(&self, result: &CIEnforcementResult) -> String {
        let mut report = String::new();

        report.push_str("# Anti-Placeholder CI Enforcement Report\n\n");

        report.push_str(&format!(
            "**Status**: {}\n\n",
            if result.passed { "PASSED" } else { "FAILED" }
        ));

        report.push_str(&format!("**Files Checked**: {}\n", result.files_checked));
        report.push_str(&format!(
            "**Total Violations**: {}\n",
            result.total_violations
        ));
        report.push_str(&format!(
            "**Execution Time**: {}ms\n\n",
            result.execution_time_ms
        ));

        // Violations by severity
        if !result.violations_by_severity.is_empty() {
            report.push_str("## Violations by Severity\n\n");
            let mut severities: Vec<_> = result.violations_by_severity.keys().collect();
            severities.sort_by(|a, b| b.cmp(a)); // Sort in descending order

            for severity in severities {
                let count = result.violations_by_severity[severity];
                let marker = match severity {
                    Severity::Critical => "[CRITICAL]",
                    Severity::High => "[HIGH]",
                    Severity::Medium => "[MEDIUM]",
                    Severity::Low => "[LOW]",
                };
                report.push_str(&format!(
                    "{} **{}**: {}\n",
                    marker,
                    self.severity_to_string(*severity),
                    count
                ));
            }
            report.push_str("\n");
        }

        // Detailed violations
        if !result.violations.is_empty() {
            report.push_str("## Detailed Violations\n\n");

            let mut grouped_violations: HashMap<String, Vec<_>> = HashMap::new();
            for violation in &result.violations {
                grouped_violations
                    .entry(violation.pattern_name.clone())
                    .or_insert_with(Vec::new)
                    .push(violation);
            }

            for (pattern_name, pattern_violations) in grouped_violations {
                report.push_str(&format!("### {}\n\n", pattern_name));

                for violation in pattern_violations {
                    report.push_str(&format!("**File**: `{}`\n", violation.file_path.display()));
                    report.push_str(&format!("**Line**: {}\n", violation.line_number));
                    report.push_str(&format!(
                        "**Severity**: {}\n",
                        self.severity_to_string(violation.severity)
                    ));
                    report.push_str(&format!("**Description**: {}\n", violation.description));
                    report.push_str(&format!("**Suggestion**: {}\n", violation.suggestion));

                    report.push_str("**Code**:\n```rust\n");
                    for context_line in &violation.context_lines {
                        report.push_str(context_line);
                        report.push('\n');
                    }
                    report.push_str("```\n\n");
                }
            }
        }

        report
    }

    /// Convert severity to string
    fn severity_to_string(&self, severity: Severity) -> &'static str {
        match severity {
            Severity::Critical => "Critical",
            Severity::High => "High",
            Severity::Medium => "Medium",
            Severity::Low => "Low",
        }
    }

    /// Add a custom pattern
    pub fn add_custom_pattern(&mut self, pattern: CustomPattern) -> Result<()> {
        // Validate the regex pattern
        Regex::new(&pattern.pattern)
            .with_context(|| format!("Invalid regex pattern: {}", pattern.pattern))?;

        self.config.custom_patterns.push(pattern);
        Ok(())
    }

    /// Update configuration
    pub fn update_config(&mut self, config: CIConfig) -> Result<()> {
        self.config = config;
        self.initialize_patterns()
    }
}
