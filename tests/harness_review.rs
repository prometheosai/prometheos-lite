//! Issue 13: Review Layer Tests
//!
//! Comprehensive tests for the Review Layer including:
//! - ReviewIssue struct (issue_type, severity, file, line, message, suggestion)
//! - ReviewIssueType enum (Bug, Security, Performance, Style, Documentation)
//! - ReviewSeverity enum (Info, Low, Medium, High, Critical)
//! - ReviewReport struct (issues, summary, passed)
//! - ReviewSummary struct (total_issues, by_type, by_severity)
//! - ReviewContext struct (file_path, file_content, language)
//! - Language enum (Rust, Python, JavaScript, TypeScript, Go, Java, Cpp)
//! - review_diff function
//! - review_file function
//! - generate_review_report function
//! - format_review_report function
//! - has_critical_issues function
//! - get_issues_by_type function
//! - get_issues_by_severity function

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::review::{
    generate_review_report, has_critical_issues, review_diff, review_file, format_review_report,
    get_issues_by_severity, get_issues_by_type, Language, ReviewContext, ReviewIssue,
    ReviewIssueType, ReviewReport, ReviewSeverity, ReviewSummary,
};

// ============================================================================
// ReviewIssue Tests
// ============================================================================

#[test]
fn test_review_issue_creation() {
    let issue = ReviewIssue {
        issue_type: ReviewIssueType::Bug,
        severity: ReviewSeverity::High,
        file: Some("src/main.rs".to_string()),
        line: Some(42),
        message: "Potential null pointer dereference".to_string(),
        suggestion: Some("Add null check".to_string()),
        rule_id: "null_check".to_string(),
    };

    assert!(matches!(issue.issue_type, ReviewIssueType::Bug));
    assert!(matches!(issue.severity, ReviewSeverity::High));
    assert_eq!(issue.file, Some("src/main.rs".to_string()));
    assert_eq!(issue.line, Some(42));
    assert_eq!(issue.message, "Potential null pointer dereference");
    assert_eq!(issue.suggestion, Some("Add null check".to_string()));
}

#[test]
fn test_review_issue_without_location() {
    let issue = ReviewIssue {
        issue_type: ReviewIssueType::Style,
        severity: ReviewSeverity::Low,
        file: None,
        line: None,
        message: "General style issue".to_string(),
        suggestion: None,
        rule_id: "style_general".to_string(),
    };

    assert!(issue.file.is_none());
    assert!(issue.line.is_none());
    assert!(issue.suggestion.is_none());
}

// ============================================================================
// ReviewIssueType Tests
// ============================================================================

#[test]
fn test_review_issue_type_variants() {
    assert!(matches!(ReviewIssueType::Bug, ReviewIssueType::Bug));
    assert!(matches!(ReviewIssueType::Security, ReviewIssueType::Security));
    assert!(matches!(ReviewIssueType::Performance, ReviewIssueType::Performance));
    assert!(matches!(ReviewIssueType::Style, ReviewIssueType::Style));
    assert!(matches!(ReviewIssueType::Documentation, ReviewIssueType::Documentation));
}

#[test]
fn test_review_issue_type_display() {
    assert_eq!(format!("{:?}", ReviewIssueType::Bug), "Bug");
    assert_eq!(format!("{:?}", ReviewIssueType::Security), "Security");
    assert_eq!(format!("{:?}", ReviewIssueType::Performance), "Performance");
    assert_eq!(format!("{:?}", ReviewIssueType::Style), "Style");
    assert_eq!(format!("{:?}", ReviewIssueType::Documentation), "Documentation");
}

// ============================================================================
// ReviewSeverity Tests
// ============================================================================

#[test]
fn test_review_severity_variants() {
    assert!(matches!(ReviewSeverity::Info, ReviewSeverity::Info));
    assert!(matches!(ReviewSeverity::Low, ReviewSeverity::Low));
    assert!(matches!(ReviewSeverity::Medium, ReviewSeverity::Medium));
    assert!(matches!(ReviewSeverity::High, ReviewSeverity::High));
    assert!(matches!(ReviewSeverity::Critical, ReviewSeverity::Critical));
}

#[test]
fn test_review_severity_ordering() {
    assert!(ReviewSeverity::Info < ReviewSeverity::Low);
    assert!(ReviewSeverity::Low < ReviewSeverity::Medium);
    assert!(ReviewSeverity::Medium < ReviewSeverity::High);
    assert!(ReviewSeverity::High < ReviewSeverity::Critical);
}

#[test]
fn test_review_severity_display() {
    assert_eq!(format!("{:?}", ReviewSeverity::Info), "Info");
    assert_eq!(format!("{:?}", ReviewSeverity::Low), "Low");
    assert_eq!(format!("{:?}", ReviewSeverity::Medium), "Medium");
    assert_eq!(format!("{:?}", ReviewSeverity::High), "High");
    assert_eq!(format!("{:?}", ReviewSeverity::Critical), "Critical");
}

// ============================================================================
// ReviewReport Tests
// ============================================================================

#[test]
fn test_review_report_passed() {
    let report = ReviewReport {
        issues: vec![],
        summary: ReviewSummary {
            total_issues: 0,
            by_type: HashMap::new(),
            by_severity: HashMap::new(),
            files_reviewed: 0,
            files_with_issues: 0,
        },
        passed: true,
        ast_analysis_enabled: false,
        critical_count: 0,
        high_count: 0,
    };

    assert!(report.passed);
    assert!(report.issues.is_empty());
}

#[test]
fn test_review_report_failed() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::High,
                file: Some("src/main.rs".to_string()),
                line: Some(10),
                message: "Bug found".to_string(),
                suggestion: None,
                rule_id: "bug_found".to_string(),
            },
        ],
        summary: ReviewSummary {
            total_issues: 1,
            by_type: {
                let mut map = HashMap::new();
                map.insert(ReviewIssueType::Bug, 1);
                map
            },
            by_severity: {
                let mut map = HashMap::new();
                map.insert(ReviewSeverity::High, 1);
                map
            },
            files_reviewed: 1,
            files_with_issues: 1,
        },
        passed: false,
        ast_analysis_enabled: false,
        critical_count: 1,
        high_count: 1,
    };

    assert!(!report.passed);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.summary.total_issues, 1);
}

// ============================================================================
// ReviewSummary Tests
// ============================================================================

#[test]
fn test_review_summary_default() {
    let summary = ReviewSummary::default();

    assert_eq!(summary.total_issues, 0);
    assert!(summary.by_type.is_empty());
    assert!(summary.by_severity.is_empty());
}

#[test]
fn test_review_summary_with_data() {
    let mut by_type = HashMap::new();
    by_type.insert(ReviewIssueType::Bug, 2);
    by_type.insert(ReviewIssueType::Style, 3);

    let mut by_severity = HashMap::new();
    by_severity.insert(ReviewSeverity::High, 2);
    by_severity.insert(ReviewSeverity::Low, 3);

    let summary = ReviewSummary {
        total_issues: 5,
        by_type,
        by_severity,
        files_reviewed: 3,
        files_with_issues: 2,
    };

    assert_eq!(summary.total_issues, 5);
    assert_eq!(summary.by_type.get(&ReviewIssueType::Bug), Some(&2));
    assert_eq!(summary.by_severity.get(&ReviewSeverity::High), Some(&2));
}

// ============================================================================
// Language Tests
// ============================================================================

#[test]
fn test_language_variants() {
    assert!(matches!(Language::Rust, Language::Rust));
    assert!(matches!(Language::Python, Language::Python));
    assert!(matches!(Language::JavaScript, Language::JavaScript));
    assert!(matches!(Language::TypeScript, Language::TypeScript));
    assert!(matches!(Language::Go, Language::Go));
    assert!(matches!(Language::Java, Language::Java));
    assert!(matches!(Language::Other, Language::Other));
}

#[test]
fn test_language_display() {
    assert_eq!(format!("{:?}", Language::Rust), "Rust");
    assert_eq!(format!("{:?}", Language::Python), "Python");
    assert_eq!(format!("{:?}", Language::JavaScript), "JavaScript");
    assert_eq!(format!("{:?}", Language::TypeScript), "TypeScript");
}

// ============================================================================
// ReviewContext Tests
// ============================================================================

#[test]
fn test_review_context_creation() {
    let ctx = ReviewContext {
        file_path: PathBuf::from("src/main.rs"),
        file_content: "fn main() {}".to_string(),
        language: Language::Rust,
        ast: None,
    };

    assert_eq!(ctx.file_path, PathBuf::from("src/main.rs"));
    assert_eq!(ctx.file_content, "fn main() {}");
    assert!(matches!(ctx.language, Language::Rust));
}

// ============================================================================
// review_diff Tests
// ============================================================================

#[test]
fn test_review_diff_simple() {
    let diff = r#"--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = 1;
+    let x = 2;
     println!("{}", x);
 }
"#;

    let issues = review_diff(diff);
    // The review may or may not find issues depending on the patterns
    // This test mainly verifies the function runs without panic
}

// ============================================================================
// review_file Tests
// ============================================================================

#[test]
fn test_review_file_rust() {
    let content = r#"
fn main() {
    let unused = 5;
    println!("Hello");
}
"#;

    let issues = review_file(PathBuf::from("test.rs").as_path(), content);
    // Verify the function runs without panic
}

#[test]
fn test_review_file_python() {
    let content = r#"
def main():
    unused = 5
    print("Hello")
"#;

    let issues = review_file(PathBuf::from("test.py").as_path(), content);
}

// ============================================================================
// generate_review_report Tests
// ============================================================================

#[test]
fn test_generate_review_report_empty() {
    let files: Vec<(PathBuf, String)> = vec![];
    let report = generate_review_report(&files);

    assert!(report.passed);
    assert!(report.issues.is_empty());
}

#[test]
fn test_generate_review_report_with_files() {
    let files = vec![
        (PathBuf::from("src/main.rs"), "fn main() {}".to_string()),
        (PathBuf::from("src/lib.rs"), "pub fn lib() {}".to_string()),
    ];
    let report = generate_review_report(&files);

    // Report should process all files
    assert!(!report.summary.by_type.is_empty() || report.issues.is_empty());
}

// ============================================================================
// format_review_report Tests
// ============================================================================

#[test]
fn test_format_review_report_empty() {
    let report = ReviewReport {
        issues: vec![],
        summary: ReviewSummary::default(),
        passed: true,
        critical_count: 0,
        high_count: 0,
        ast_analysis_enabled: false,
    };

    let formatted = format_review_report(&report);
    assert!(!formatted.is_empty());
}

#[test]
fn test_format_review_report_with_issues() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::High,
                file: Some("src/main.rs".to_string()),
                line: Some(10),
                message: "Test issue".to_string(),
                suggestion: Some("Fix it".to_string()),
                rule_id: "test_rule".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Style,
                severity: ReviewSeverity::Low,
                file: Some("src/style.rs".to_string()),
                line: Some(20),
                message: "Style issue".to_string(),
                suggestion: Some("Fix style".to_string()),
                rule_id: "style_rule".to_string(),
            },
        ],
        summary: ReviewSummary {
            total_issues: 2,
            by_type: {
                let mut map = HashMap::new();
                map.insert(ReviewIssueType::Bug, 1);
                map
            },
            by_severity: {
                let mut map = HashMap::new();
                map.insert(ReviewSeverity::High, 1);
                map
            },
            files_reviewed: 1,
            files_with_issues: 1,
        },
        passed: false,
        critical_count: 1,
        high_count: 1,
        ast_analysis_enabled: false,
    };

    let formatted = format_review_report(&report);
    assert!(!formatted.is_empty());
    assert!(formatted.contains("Review Report") || formatted.contains("issue"));
}

// ============================================================================
// has_critical_issues Tests
// ============================================================================

#[test]
fn test_has_critical_issues_true() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Security,
                severity: ReviewSeverity::Critical,
                file: None,
                line: None,
                message: "Critical security issue".to_string(),
                suggestion: None,
                rule_id: "security_rule".to_string(),
            },
        ],
        summary: ReviewSummary {
            total_issues: 1,
            by_type: HashMap::new(),
            by_severity: {
                let mut map = HashMap::new();
                map.insert(ReviewSeverity::Critical, 1);
                map
            },
            files_reviewed: 1,
            files_with_issues: 1,
        },
        passed: false,
        critical_count: 1,
        high_count: 0,
        ast_analysis_enabled: false,
    };

    assert!(has_critical_issues(&report));
}

#[test]
fn test_has_critical_issues_false() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Style,
                severity: ReviewSeverity::Low,
                file: None,
                line: None,
                message: "Minor style issue".to_string(),
                suggestion: None,
                rule_id: "style_rule".to_string(),
            },
        ],
        summary: ReviewSummary {
            total_issues: 1,
            by_type: HashMap::new(),
            by_severity: {
                let mut map = HashMap::new();
                map.insert(ReviewSeverity::Low, 1);
                map
            },
            files_reviewed: 1,
            files_with_issues: 1,
        },
        passed: false,
        critical_count: 0,
        high_count: 1,
        ast_analysis_enabled: false,
    };

    assert!(!has_critical_issues(&report));
}

// ============================================================================
// get_issues_by_type Tests
// ============================================================================

#[test]
fn test_get_issues_by_type() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::High,
                file: None,
                line: None,
                message: "Bug 1".to_string(),
                suggestion: None,
                rule_id: "bug_rule".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::Medium,
                file: None,
                line: None,
                message: "Bug 2".to_string(),
                suggestion: None,
                rule_id: "bug_rule".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Style,
                severity: ReviewSeverity::Low,
                file: None,
                line: None,
                message: "Style issue".to_string(),
                suggestion: None,
                rule_id: "style_rule".to_string(),
            },
        ],
        summary: ReviewSummary::default(),
        passed: false,
        critical_count: 0,
        high_count: 2,
        ast_analysis_enabled: false,
    };

    let bugs = get_issues_by_type(&report, ReviewIssueType::Bug);
    assert_eq!(bugs.len(), 2);

    let styles = get_issues_by_type(&report, ReviewIssueType::Style);
    assert_eq!(styles.len(), 1);
}

// ============================================================================
// get_issues_by_severity Tests
// ============================================================================

#[test]
fn test_get_issues_by_severity() {
    let report = ReviewReport {
        issues: vec![
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::High,
                file: None,
                line: None,
                message: "High severity".to_string(),
                suggestion: None,
                rule_id: "severity_rule".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Bug,
                severity: ReviewSeverity::High,
                file: None,
                line: None,
                message: "Another high".to_string(),
                suggestion: None,
                rule_id: "severity_rule".to_string(),
            },
            ReviewIssue {
                issue_type: ReviewIssueType::Style,
                severity: ReviewSeverity::Low,
                file: None,
                line: None,
                message: "Low severity".to_string(),
                suggestion: None,
                rule_id: "style_rule".to_string(),
            },
        ],
        summary: ReviewSummary::default(),
        passed: false,
        critical_count: 0,
        high_count: 2,
        ast_analysis_enabled: false,
    };

    let high = get_issues_by_severity(&report, ReviewSeverity::High);
    assert_eq!(high.len(), 2);

    let low = get_issues_by_severity(&report, ReviewSeverity::Low);
    assert_eq!(low.len(), 1);
}
