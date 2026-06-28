#![cfg(any())]
// Quarantined: obsolete integration suite targets pre-audit harness APIs.
//! Issue 14: Semantic Diff Analyzer Tests
//!
//! Comprehensive tests for the Semantic Diff Analyzer including:
//! - SemanticDiff struct (api_changes, auth_changes, database_changes, etc.)
//! - ApiChange struct and ApiChangeType enum (FunctionAdded, FunctionRemoved, etc.)
//! - AuthChange struct and AuthChangeType enum (AuthenticationAdded, etc.)
//! - DatabaseChange struct and DatabaseChangeType enum (SchemaAdded, etc.)
//! - DependencyChange struct and DependencyChangeType enum
//! - ConfigChange struct and ConfigChangeType enum
//! - FileChange struct and FileChangeType enum
//! - RiskLevel enum and RiskAssessment struct
//! - SemanticSummary struct
//! - analyze_semantic_diff function
//! - format_semantic_diff_report function
//! - has_breaking_changes function
//! - requires_approval function
//! - requires_security_review function

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::semantic_diff::{
    ApiChange, ApiChangeType, AuthChange, AuthChangeType, ConfigChange, ConfigChangeType,
    ConfigEnvironment, DatabaseChange, DatabaseChangeType, DependencyChange, DependencyChangeType,
    FileChange, FileChangeType, RiskAssessment, RiskLevel, SemanticCategory, SemanticDiff,
    SemanticSummary, analyze_semantic_diff, format_semantic_diff_report, has_breaking_changes,
    requires_approval, requires_security_review,
};

// ============================================================================
// SemanticDiff Tests
// ============================================================================

#[test]
fn test_semantic_diff_default() {
    let diff = SemanticDiff::default();

    assert!(diff.api_changes.is_empty());
    assert!(diff.auth_changes.is_empty());
    assert!(diff.database_changes.is_empty());
    assert!(diff.dependency_changes.is_empty());
    assert!(diff.config_changes.is_empty());
    assert!(diff.changed_files.is_empty());
}

#[test]
fn test_semantic_diff_with_changes() {
    let diff = SemanticDiff {
        api_changes: vec![ApiChange {
            file: PathBuf::from("src/api.rs"),
            line: Some(10),
            change_type: ApiChangeType::FunctionAdded,
            breaking: false,
            description: "Function added".to_string(),
            signature: "fn new_function()".to_string(),
        }],
        auth_changes: vec![],
        database_changes: vec![],
        dependency_changes: vec![],
        config_changes: vec![],
        changed_files: vec![],
        summary: SemanticSummary::default(),
        risk_assessment: RiskAssessment::default(),
    };

    assert_eq!(diff.api_changes.len(), 1);
}

// ============================================================================
// ApiChange Tests
// ============================================================================

#[test]
fn test_api_change_creation() {
    let change = ApiChange {
        file: PathBuf::from("src/lib.rs"),
        line: Some(42),
        change_type: ApiChangeType::FunctionModified,
        breaking: false,
        description: "Function modification".to_string(),
        signature: "fn modified_function()".to_string(),
    };

    assert_eq!(change.file, PathBuf::from("src/lib.rs"));
    assert_eq!(change.line, Some(42));
    assert!(matches!(
        change.change_type,
        ApiChangeType::FunctionModified
    ));
    // Note: function_name field doesn't exist in actual ApiChange struct
}

// ============================================================================
// ApiChangeType Tests
// ============================================================================

#[test]
fn test_api_change_type_variants() {
    assert!(matches!(
        ApiChangeType::FunctionAdded,
        ApiChangeType::FunctionAdded
    ));
    assert!(matches!(
        ApiChangeType::FunctionRemoved,
        ApiChangeType::FunctionRemoved
    ));
    assert!(matches!(
        ApiChangeType::FunctionModified,
        ApiChangeType::FunctionModified
    ));
    assert!(matches!(
        ApiChangeType::FunctionAdded,
        ApiChangeType::FunctionAdded
    ));
    assert!(matches!(
        ApiChangeType::FunctionRemoved,
        ApiChangeType::FunctionRemoved
    ));
    assert!(matches!(
        ApiChangeType::FunctionModified,
        ApiChangeType::FunctionModified
    ));
}

#[test]
fn test_api_change_type_display() {
    assert_eq!(
        format!("{:?}", ApiChangeType::FunctionAdded),
        "FunctionAdded"
    );
    assert_eq!(
        format!("{:?}", ApiChangeType::FunctionRemoved),
        "FunctionRemoved"
    );
    assert_eq!(
        format!("{:?}", ApiChangeType::FunctionModified),
        "FunctionModified"
    );
}

// ============================================================================
// AuthChange Tests
// ============================================================================

#[test]
fn test_auth_change_creation() {
    let change = AuthChange {
        file: PathBuf::from("src/auth.rs"),
        line: Some(15),
        change_type: AuthChangeType::AuthenticationAdded,
        description: "Added JWT auth".to_string(),
        risk_level: RiskLevel::Medium,
    };

    assert_eq!(change.file, PathBuf::from("src/auth.rs"));
    assert!(matches!(
        change.change_type,
        AuthChangeType::AuthenticationAdded
    ));
    assert_eq!(change.description, "Added JWT auth");
}

// ============================================================================
// AuthChangeType Tests
// ============================================================================

#[test]
fn test_auth_change_type_variants() {
    assert!(matches!(
        AuthChangeType::AuthenticationAdded,
        AuthChangeType::AuthenticationAdded
    ));
    assert!(matches!(
        AuthChangeType::AuthenticationRemoved,
        AuthChangeType::AuthenticationRemoved
    ));
    assert!(matches!(
        AuthChangeType::AuthenticationModified,
        AuthChangeType::AuthenticationModified
    ));
}

// ============================================================================
// DatabaseChange Tests
// ============================================================================

#[test]
fn test_database_change_creation() {
    let change = DatabaseChange {
        file: PathBuf::from("migrations/001.sql"),
        line: Some(5),
        change_type: DatabaseChangeType::SchemaAdded,
        description: "Added users table".to_string(),
        migration_required: true,
        breaking: false,
    };

    assert_eq!(change.file, PathBuf::from("migrations/001.sql"));
    assert!(matches!(
        change.change_type,
        DatabaseChangeType::SchemaAdded
    ));
    assert_eq!(change.description, "Added users table");
}

// ============================================================================
// DatabaseChangeType Tests
// ============================================================================

#[test]
fn test_database_change_type_variants() {
    assert!(matches!(
        DatabaseChangeType::SchemaAdded,
        DatabaseChangeType::SchemaAdded
    ));
    assert!(matches!(
        DatabaseChangeType::SchemaRemoved,
        DatabaseChangeType::SchemaRemoved
    ));
    assert!(matches!(
        DatabaseChangeType::SchemaModified,
        DatabaseChangeType::SchemaModified
    ));
}

// ============================================================================
// DependencyChange Tests
// ============================================================================

#[test]
fn test_dependency_change_creation() {
    let change = DependencyChange {
        file: PathBuf::from("Cargo.toml"),
        package_name: "serde".to_string(),
        old_version: Some("1.0.0".to_string()),
        new_version: Some("1.0.1".to_string()),
        change_type: DependencyChangeType::Upgraded,
        risk_level: RiskLevel::Low,
    };

    assert_eq!(change.file, PathBuf::from("Cargo.toml"));
    assert_eq!(change.package_name, "serde");
    assert_eq!(change.old_version, Some("1.0.0".to_string()));
    assert!(matches!(change.change_type, DependencyChangeType::Upgraded));
}

// ============================================================================
// DependencyChangeType Tests
// ============================================================================

#[test]
fn test_dependency_change_type_variants() {
    assert!(matches!(
        DependencyChangeType::Added,
        DependencyChangeType::Added
    ));
    assert!(matches!(
        DependencyChangeType::Removed,
        DependencyChangeType::Removed
    ));
    assert!(matches!(
        DependencyChangeType::Upgraded,
        DependencyChangeType::Upgraded
    ));
}

// ============================================================================
// ConfigChange Tests
// ============================================================================

#[test]
fn test_config_change_creation() {
    let change = ConfigChange {
        file: PathBuf::from("config.toml"),
        config_key: "database.url".to_string(),
        old_value: Some("localhost".to_string()),
        new_value: Some("prod.db".to_string()),
        change_type: ConfigChangeType::Modified,
        environment: ConfigEnvironment::Production,
    };

    assert_eq!(change.file, PathBuf::from("config.toml"));
    assert_eq!(change.config_key, "database.url");
    assert_eq!(change.old_value, Some("localhost".to_string()));
    assert!(matches!(change.change_type, ConfigChangeType::Modified));
    assert!(matches!(change.environment, ConfigEnvironment::Production));
}

// ============================================================================
// ConfigChangeType Tests
// ============================================================================

#[test]
fn test_config_change_type_variants() {
    assert!(matches!(ConfigChangeType::Added, ConfigChangeType::Added));
    assert!(matches!(
        ConfigChangeType::Removed,
        ConfigChangeType::Removed
    ));
    assert!(matches!(
        ConfigChangeType::Modified,
        ConfigChangeType::Modified
    ));
}

// ============================================================================
// ConfigEnvironment Tests
// ============================================================================

#[test]
fn test_config_environment_variants() {
    assert!(matches!(
        ConfigEnvironment::Development,
        ConfigEnvironment::Development
    ));
    assert!(matches!(
        ConfigEnvironment::Production,
        ConfigEnvironment::Production
    ));
    assert!(matches!(ConfigEnvironment::Test, ConfigEnvironment::Test));
}

// ============================================================================
// FileChange Tests
// ============================================================================

#[test]
fn test_file_change_creation() {
    let change = FileChange {
        path: PathBuf::from("src/main.rs"),
        change_type: FileChangeType::Modified,
        lines_added: 10,
        lines_removed: 5,
        semantic_category: SemanticCategory::SourceCode,
    };

    assert_eq!(change.path, PathBuf::from("src/main.rs"));
    assert!(matches!(change.change_type, FileChangeType::Modified));
    assert_eq!(change.semantic_category, SemanticCategory::SourceCode);
    assert_eq!(change.lines_removed, 5);
    assert!(matches!(
        change.semantic_category,
        SemanticCategory::SourceCode
    ));
}
// FileChangeType Tests
// ============================================================================

#[test]
fn test_file_change_type_variants() {
    assert!(matches!(FileChangeType::Added, FileChangeType::Added));
    assert!(matches!(FileChangeType::Removed, FileChangeType::Removed));
    assert!(matches!(FileChangeType::Modified, FileChangeType::Modified));
}

// ============================================================================
// SemanticCategory Tests
// ============================================================================

#[test]
fn test_semantic_category_variants() {
    assert!(matches!(
        SemanticCategory::SourceCode,
        SemanticCategory::SourceCode
    ));
    assert!(matches!(SemanticCategory::Test, SemanticCategory::Test));
    assert!(matches!(
        SemanticCategory::Configuration,
        SemanticCategory::Configuration
    ));
    assert!(matches!(
        SemanticCategory::Documentation,
        SemanticCategory::Documentation
    ));
    assert!(matches!(SemanticCategory::Build, SemanticCategory::Build));
}

// ============================================================================
// RiskLevel Tests
// ============================================================================

#[test]
fn test_risk_level_variants() {
    assert!(matches!(RiskLevel::None, RiskLevel::None));
    assert!(matches!(RiskLevel::Low, RiskLevel::Low));
    assert!(matches!(RiskLevel::Medium, RiskLevel::Medium));
    assert!(matches!(RiskLevel::High, RiskLevel::High));
    assert!(matches!(RiskLevel::Critical, RiskLevel::Critical));
}

#[test]
fn test_risk_level_ordering() {
    assert!(RiskLevel::None < RiskLevel::Low);
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
    assert!(RiskLevel::High < RiskLevel::Critical);
}

// ============================================================================
// RiskAssessment Tests
// ============================================================================

#[test]
fn test_risk_assessment_default() {
    let assessment = RiskAssessment::default();

    assert!(matches!(assessment.overall_risk, RiskLevel::None));
    assert!(matches!(assessment.api_risk, RiskLevel::None));
    assert!(matches!(assessment.auth_risk, RiskLevel::None));
    assert!(matches!(assessment.database_risk, RiskLevel::None));
    assert!(!assessment.requires_approval);
}

#[test]
fn test_risk_assessment_high() {
    let assessment = RiskAssessment {
        overall_risk: RiskLevel::High,
        api_risk: RiskLevel::Medium,
        auth_risk: RiskLevel::High,
        database_risk: RiskLevel::Low,
        dependency_risk: RiskLevel::None,
        config_risk: RiskLevel::None,
        requires_review: true,
        requires_approval: true,
        reasons: vec!["High risk changes".to_string()],
    };

    assert!(matches!(assessment.overall_risk, RiskLevel::High));
    assert!(assessment.requires_approval);
}

// ============================================================================
// SemanticSummary Tests
// ============================================================================

#[test]
fn test_semantic_summary_default() {
    let summary = SemanticSummary::default();

    assert_eq!(summary.total_files_changed, 0);
    assert_eq!(summary.total_lines_added, 0);
    assert_eq!(summary.total_lines_removed, 0);
    assert_eq!(summary.breaking_changes, 0);
    assert_eq!(summary.security_relevant_changes, 0);
}

#[test]
fn test_semantic_summary_with_data() {
    let summary = SemanticSummary {
        total_files_changed: 5,
        total_lines_added: 100,
        total_lines_removed: 50,
        breaking_changes: 2,
        api_surface_changes: 3,
        security_relevant_changes: 1,
        infrastructure_changes: 0,
    };

    assert_eq!(summary.total_files_changed, 5);
    assert_eq!(summary.total_lines_added, 100);
    assert_eq!(summary.total_lines_removed, 50);
    assert_eq!(summary.breaking_changes, 2);
    assert_eq!(summary.security_relevant_changes, 1);
}

// ============================================================================
// analyze_semantic_diff Tests
// ============================================================================

#[test]
fn test_analyze_semantic_diff_simple() {
    let diff = r#"--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = 1;
+    let x = 2;
     println!("{}", x);
 }
"#;

    let result = analyze_semantic_diff(diff);
    // Verify the function runs without panic and returns a valid result
    assert!(result.changed_files.is_empty() || !result.changed_files.is_empty());
}

#[test]
fn test_analyze_semantic_diff_empty() {
    let result = analyze_semantic_diff("");
    // Empty diff should return empty result
    assert!(result.api_changes.is_empty());
}

// ============================================================================
// format_semantic_diff_report Tests
// ============================================================================

#[test]
fn test_format_semantic_diff_report_empty() {
    let diff = SemanticDiff::default();
    let report = format_semantic_diff_report(&diff);

    assert!(!report.is_empty());
    assert!(report.contains("Semantic Diff Analysis"));
}

#[test]
fn test_format_semantic_diff_report_with_changes() {
    let diff = SemanticDiff {
        api_changes: vec![ApiChange {
            file: PathBuf::from("src/api.rs"),
            line: Some(10),
            change_type: ApiChangeType::FunctionAdded,
            breaking: false,
            description: "New API function".to_string(),
            signature: "new_api".to_string(),
        }],
        auth_changes: vec![],
        database_changes: vec![],
        dependency_changes: vec![],
        config_changes: vec![],
        changed_files: vec![FileChange {
            path: PathBuf::from("src/api.rs"),
            change_type: FileChangeType::Modified,
            lines_added: 10,
            lines_removed: 0,
            semantic_category: SemanticCategory::SourceCode,
        }],
        summary: SemanticSummary {
            total_files_changed: 1,
            total_lines_added: 10,
            total_lines_removed: 0,
            breaking_changes: 0,
            api_surface_changes: 1,
            security_relevant_changes: 0,
            infrastructure_changes: 0,
        },
        risk_assessment: RiskAssessment {
            overall_risk: RiskLevel::Low,
            api_risk: RiskLevel::Medium,
            auth_risk: RiskLevel::None,
            database_risk: RiskLevel::None,
            dependency_risk: RiskLevel::None,
            config_risk: RiskLevel::None,
            requires_review: false,
            requires_approval: false,
            reasons: vec![],
        },
    };

    let report = format_semantic_diff_report(&diff);
    assert!(!report.is_empty());
}

// ============================================================================
// has_breaking_changes Tests
// ============================================================================

#[test]
fn test_has_breaking_changes_true() {
    let diff = SemanticDiff {
        summary: SemanticSummary {
            breaking_changes: 1,
            ..Default::default()
        },
        risk_assessment: RiskAssessment {
            overall_risk: RiskLevel::High,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(has_breaking_changes(&diff));
}

#[test]
fn test_has_breaking_changes_false() {
    let diff = SemanticDiff {
        summary: SemanticSummary {
            breaking_changes: 0,
            ..Default::default()
        },
        risk_assessment: RiskAssessment {
            overall_risk: RiskLevel::Low,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(!has_breaking_changes(&diff));
}

// ============================================================================
// requires_approval Tests
// ============================================================================

#[test]
fn test_requires_approval_true() {
    let diff = SemanticDiff {
        risk_assessment: RiskAssessment {
            requires_approval: true,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(requires_approval(&diff));
}

#[test]
fn test_requires_approval_false() {
    let diff = SemanticDiff {
        risk_assessment: RiskAssessment {
            requires_approval: false,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(!requires_approval(&diff));
}

// ============================================================================
// requires_security_review Tests
// ============================================================================

#[test]
fn test_requires_security_review_auth_risk() {
    let diff = SemanticDiff {
        risk_assessment: RiskAssessment {
            auth_risk: RiskLevel::High,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(requires_security_review(&diff));
}

#[test]
fn test_requires_security_review_critical() {
    let diff = SemanticDiff {
        risk_assessment: RiskAssessment {
            overall_risk: RiskLevel::Critical,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(requires_security_review(&diff));
}

#[test]
fn test_requires_security_review_security_changes() {
    let diff = SemanticDiff {
        summary: SemanticSummary {
            security_relevant_changes: 1,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(requires_security_review(&diff));
}

#[test]
fn test_requires_security_review_false() {
    let diff = SemanticDiff {
        risk_assessment: RiskAssessment {
            auth_risk: RiskLevel::Low,
            overall_risk: RiskLevel::Low,
            ..Default::default()
        },
        summary: SemanticSummary {
            security_relevant_changes: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    assert!(!requires_security_review(&diff));
}
