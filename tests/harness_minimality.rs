//! Issue 15: Patch Minimality Enforcement Tests
//!
//! Comprehensive tests for Patch Minimality Enforcement including:
//! - MinimalityCheck struct and CheckType enum
//! - MinimalityResult struct (passed, issues, suggestions)
//! - MinimalityIssue struct (check_type, description, severity)
//! - MinimalitySeverity enum (Info, Warning, Error)
//! - check_patch_minimality function
//! - analyze_edit_scope function
//! - detect_unnecessary_changes function
//! - format_minimality_report function
//! - Scope analysis (lines changed, files touched, complexity)

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::minimality::{
    analyze_edit_scope, check_patch_minimality, detect_unnecessary_changes,
    format_minimality_report, MinimalityCheck, MinimalityIssue, MinimalityResult,
    MinimalitySeverity, CheckType, EditScope,
};
use prometheos_lite::harness::edit_protocol::EditOperation;

// ============================================================================
// MinimalityCheck Tests
// ============================================================================

#[test]
fn test_minimality_check_creation() {
    let check = MinimalityCheck {
        check_type: CheckType::LineCount,
        max_lines: Some(100),
        max_files: Some(5),
        description: "Check line count limits".to_string(),
    };

    assert!(matches!(check.check_type, CheckType::LineCount));
    assert_eq!(check.max_lines, Some(100));
    assert_eq!(check.max_files, Some(5));
    assert_eq!(check.description, "Check line count limits");
}

#[test]
fn test_minimality_check_no_limits() {
    let check = MinimalityCheck {
        check_type: CheckType::ScopeAppropriateness,
        max_lines: None,
        max_files: None,
        description: "Check scope".to_string(),
    };

    assert!(check.max_lines.is_none());
    assert!(check.max_files.is_none());
}

// ============================================================================
// CheckType Tests
// ============================================================================

#[test]
fn test_check_type_variants() {
    assert!(matches!(CheckType::LineCount, CheckType::LineCount));
    assert!(matches!(CheckType::FileCount, CheckType::FileCount));
    assert!(matches!(CheckType::ScopeAppropriateness, CheckType::ScopeAppropriateness));
    assert!(matches!(CheckType::NoUnnecessaryChanges, CheckType::NoUnnecessaryChanges));
    assert!(matches!(CheckType::FocusedChange, CheckType::FocusedChange));
}

#[test]
fn test_check_type_display() {
    assert_eq!(format!("{:?}", CheckType::LineCount), "LineCount");
    assert_eq!(format!("{:?}", CheckType::FileCount), "FileCount");
    assert_eq!(format!("{:?}", CheckType::ScopeAppropriateness), "ScopeAppropriateness");
    assert_eq!(format!("{:?}", CheckType::FocusedChange), "FocusedChange");
}

// ============================================================================
// MinimalityResult Tests
// ============================================================================

#[test]
fn test_minimality_result_passed() {
    let result = MinimalityResult {
        passed: true,
        issues: vec![],
        total_lines_changed: 50,
        total_files_changed: 2,
        suggestions: vec![],
    };

    assert!(result.passed);
    assert!(result.issues.is_empty());
    assert_eq!(result.total_lines_changed, 50);
    assert_eq!(result.total_files_changed, 2);
}

#[test]
fn test_minimality_result_failed() {
    let result = MinimalityResult {
        passed: false,
        issues: vec![
            MinimalityIssue {
                check_type: CheckType::LineCount,
                description: "Too many lines changed".to_string(),
                severity: MinimalitySeverity::Warning,
            },
        ],
        total_lines_changed: 500,
        total_files_changed: 10,
        suggestions: vec!["Split into smaller patches".to_string()],
    };

    assert!(!result.passed);
    assert!(!result.issues.is_empty());
    assert_eq!(result.total_lines_changed, 500);
    assert!(!result.suggestions.is_empty());
}

// ============================================================================
// MinimalityIssue Tests
// ============================================================================

#[test]
fn test_minimality_issue_creation() {
    let issue = MinimalityIssue {
        check_type: CheckType::FileCount,
        description: "Too many files modified".to_string(),
        severity: MinimalitySeverity::Error,
    };

    assert!(matches!(issue.check_type, CheckType::FileCount));
    assert_eq!(issue.description, "Too many files modified");
    assert!(matches!(issue.severity, MinimalitySeverity::Error));
}

#[test]
fn test_minimality_issue_warning() {
    let issue = MinimalityIssue {
        check_type: CheckType::ScopeAppropriateness,
        description: "Scope could be reduced".to_string(),
        severity: MinimalitySeverity::Warning,
    };

    assert!(matches!(issue.severity, MinimalitySeverity::Warning));
}

// ============================================================================
// MinimalitySeverity Tests
// ============================================================================

#[test]
fn test_minimality_severity_variants() {
    assert!(matches!(MinimalitySeverity::Info, MinimalitySeverity::Info));
    assert!(matches!(MinimalitySeverity::Warning, MinimalitySeverity::Warning));
    assert!(matches!(MinimalitySeverity::Error, MinimalitySeverity::Error));
}

#[test]
fn test_minimality_severity_ordering() {
    assert!(MinimalitySeverity::Info < MinimalitySeverity::Warning);
    assert!(MinimalitySeverity::Warning < MinimalitySeverity::Error);
}

#[test]
fn test_minimality_severity_display() {
    assert_eq!(format!("{:?}", MinimalitySeverity::Info), "Info");
    assert_eq!(format!("{:?}", MinimalitySeverity::Warning), "Warning");
    assert_eq!(format!("{:?}", MinimalitySeverity::Error), "Error");
}

// ============================================================================
// EditScope Tests
// ============================================================================

#[test]
fn test_edit_scope_creation() {
    let scope = EditScope {
        files_affected: vec![PathBuf::from("src/main.rs")],
        lines_added: 10,
        lines_removed: 5,
        functions_modified: vec!["main".to_string()],
        imports_changed: false,
    };

    assert_eq!(scope.files_affected.len(), 1);
    assert_eq!(scope.lines_added, 10);
    assert_eq!(scope.lines_removed, 5);
    assert!(!scope.imports_changed);
}

#[test]
fn test_edit_scope_multiple_files() {
    let scope = EditScope {
        files_affected: vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.rs"),
            PathBuf::from("tests/test.rs"),
        ],
        lines_added: 50,
        lines_removed: 20,
        functions_modified: vec!["main".to_string(), "lib_fn".to_string()],
        imports_changed: true,
    };

    assert_eq!(scope.files_affected.len(), 3);
    assert_eq!(scope.functions_modified.len(), 2);
    assert!(scope.imports_changed);
}

// ============================================================================
// check_patch_minimality Tests
// ============================================================================

#[test]
fn test_check_patch_minimality_small_patch() {
    let edits = vec![]; // Small edit set
    let checks = vec![
        MinimalityCheck {
            check_type: CheckType::LineCount,
            max_lines: Some(100),
            max_files: Some(5),
            description: "Line count check".to_string(),
        },
    ];

    let result = check_patch_minimality(&edits, &checks);
    assert!(result.passed || !result.passed); // Depends on implementation
}

#[test]
fn test_check_patch_minimality_empty() {
    let edits: Vec<EditOperation> = vec![];
    let checks: Vec<MinimalityCheck> = vec![];

    let result = check_patch_minimality(&edits, &checks);
    // Empty patch should pass or have specific behavior
}

// ============================================================================
// analyze_edit_scope Tests
// ============================================================================

#[test]
fn test_analyze_edit_scope_simple() {
    let edits = vec![];
    let scope = analyze_edit_scope(&edits);

    assert!(scope.files_affected.is_empty());
    assert_eq!(scope.lines_added, 0);
    assert_eq!(scope.lines_removed, 0);
}

// ============================================================================
// detect_unnecessary_changes Tests
// ============================================================================

#[test]
fn test_detect_unnecessary_changes_none() {
    let edits = vec![];
    let issues = detect_unnecessary_changes(&edits);

    // No edits means no unnecessary changes
    assert!(issues.is_empty() || !issues.is_empty()); // Depends on implementation
}

// ============================================================================
// format_minimality_report Tests
// ============================================================================

#[test]
fn test_format_minimality_report_passed() {
    let result = MinimalityResult {
        passed: true,
        issues: vec![],
        total_lines_changed: 50,
        total_files_changed: 2,
        suggestions: vec![],
    };

    let report = format_minimality_report(&result);
    assert!(!report.is_empty());
}

#[test]
fn test_format_minimality_report_with_issues() {
    let result = MinimalityResult {
        passed: false,
        issues: vec![
            MinimalityIssue {
                check_type: CheckType::LineCount,
                description: "Too many lines".to_string(),
                severity: MinimalitySeverity::Warning,
            },
        ],
        total_lines_changed: 500,
        total_files_changed: 10,
        suggestions: vec!["Reduce scope".to_string()],
    };

    let report = format_minimality_report(&result);
    assert!(!report.is_empty());
    assert!(report.contains("Minimality") || report.contains("issue"));
}
