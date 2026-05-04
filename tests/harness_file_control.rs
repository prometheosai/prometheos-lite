//! Issue 3: File Control System Tests
//!
//! Comprehensive tests for the File Control System including:
//! - FileSet categorization (editable, readonly, generated, denied)
//! - FilePolicy enforcement (denied paths, size limits, permissions)
//! - DenyReason variants and Display implementation
//! - File categorization and stats
//! - Edit/delete/rename permission checking

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::file_control::{
    assert_delete_allowed, assert_edit_allowed, assert_rename_allowed, build_file_set,
    DenyReason, FilePolicy, FileSet, get_file_category, get_file_stats,
};
use prometheos_lite::harness::repo_intelligence::RepoContext;

// ============================================================================
// Basic Structure Tests
// ============================================================================

#[test]
fn test_file_set_default() {
    let set = FileSet::default();

    assert!(set.editable.is_empty());
    assert!(set.readonly.is_empty());
    assert!(set.generated.is_empty());
    assert!(set.artifacts.is_empty());
    assert!(set.denied.is_empty());
    assert!(set.binary.is_empty());
}

#[test]
fn test_file_set_creation() {
    let editable_file = PathBuf::from("src/main.rs");
    let readonly_file = PathBuf::from("README.md");
    let generated_file = PathBuf::from("target/generated.rs");
    let denied_file = PathBuf::from(".env");

    let set = FileSet {
        editable: vec![editable_file.clone()],
        readonly: vec![readonly_file.clone()],
        generated: vec![generated_file.clone()],
        artifacts: vec![],
        denied: vec![(denied_file.clone(), DenyReason::SensitiveFile)],
        binary: vec![],
    };

    assert_eq!(set.editable.len(), 1);
    assert_eq!(set.readonly.len(), 1);
    assert_eq!(set.generated.len(), 1);
    assert_eq!(set.denied.len(), 1);
    assert!(set.editable.contains(&editable_file));
}

// ============================================================================
// DenyReason Tests
// ============================================================================

#[test]
fn test_deny_reason_variants() {
    assert!(matches!(DenyReason::OutsideRepo, DenyReason::OutsideRepo));
    assert!(matches!(DenyReason::DeniedPath, DenyReason::DeniedPath));
    assert!(matches!(DenyReason::BinaryFile, DenyReason::BinaryFile));
    assert!(matches!(DenyReason::TooLarge, DenyReason::TooLarge));
    assert!(matches!(DenyReason::SensitiveFile, DenyReason::SensitiveFile));
    assert!(matches!(DenyReason::Generated, DenyReason::Generated));
    assert!(matches!(DenyReason::NotTracked, DenyReason::NotTracked));
}

#[test]
fn test_deny_reason_display() {
    assert_eq!(
        format!("{}", DenyReason::OutsideRepo),
        "file is outside repository root"
    );
    assert_eq!(format!("{}", DenyReason::DeniedPath), "path is in deny list");
    assert_eq!(
        format!("{}", DenyReason::BinaryFile),
        "binary files cannot be edited"
    );
    assert_eq!(
        format!("{}", DenyReason::TooLarge),
        "file exceeds size limit"
    );
    assert_eq!(
        format!("{}", DenyReason::SensitiveFile),
        "sensitive file detected"
    );
    assert_eq!(
        format!("{}", DenyReason::Generated),
        "generated file should not be edited directly"
    );
    assert_eq!(
        format!("{}", DenyReason::NotTracked),
        "file not tracked by git or in editable set"
    );
}

// ============================================================================
// FilePolicy Tests
// ============================================================================

#[test]
fn test_file_policy_default_for_repo() {
    let repo_root = PathBuf::from("/test/repo");
    let policy = FilePolicy::default_for_repo(&repo_root);

    assert_eq!(policy.repo_root, repo_root);
    assert!(policy.allow_delete);
    assert!(policy.allow_rename);
    assert!(!policy.allow_generated_edits);
    assert!(policy.respect_gitignore);
    assert!(!policy.allow_binary_edit);
    assert!(policy.max_file_size_bytes > 0);

    // Check denied paths
    assert!(policy.denied_paths.contains(&PathBuf::from(".git")));
    assert!(policy.denied_paths.contains(&PathBuf::from(".env")));
    assert!(policy.denied_paths.contains(&PathBuf::from("target")));
}

#[test]
fn test_file_policy_custom() {
    let policy = FilePolicy {
        repo_root: PathBuf::from("/custom/repo"),
        allowed_write_paths: vec![PathBuf::from("src")],
        denied_paths: vec![PathBuf::from("secret")],
        denied_patterns: vec!["*.secret".to_string()],
        max_file_size_bytes: 1024 * 1024,
        allow_delete: false,
        allow_rename: false,
        allow_generated_edits: true,
        respect_gitignore: false,
        allow_binary_edit: true,
        sensitive_file_patterns: vec![".env*".to_string()],
        generated_file_patterns: vec!["*.gen.rs".to_string()],
    };

    assert!(!policy.allow_delete);
    assert!(!policy.allow_rename);
    assert!(policy.allow_generated_edits);
    assert!(policy.allow_binary_edit);
    assert_eq!(policy.max_file_size_bytes, 1024 * 1024);
}

// ============================================================================
// FileSet Building Tests
// ============================================================================

#[test]
fn test_build_file_set_basic() {
    // Use the sample_repo fixture
    let fixture_path = PathBuf::from("tests/fixtures/sample_repo");

    // Build a minimal RepoContext
    let ctx = RepoContext {
        root: fixture_path.clone(),
        ranked_files: vec![],
        symbols: vec![],
        relationships: vec![],
        compressed_context: String::new(),
        token_estimate: 0,
        language_breakdown: HashMap::new(),
        dependency_graph: Default::default(),
    };

    let policy = FilePolicy::default_for_repo(&fixture_path);
    let mentioned_files: Vec<PathBuf> = vec![];

    let file_set = build_file_set(&ctx, &mentioned_files, &policy).unwrap();

    // The sample repo should have src/main.rs as editable
    assert!(!file_set.editable.is_empty() || !file_set.readonly.is_empty());
}

// ============================================================================
// File Category Tests
// ============================================================================

#[test]
fn test_get_file_category_editable() {
    let set = FileSet {
        editable: vec![PathBuf::from("src/main.rs")],
        ..Default::default()
    };

    let category = get_file_category(PathBuf::from("src/main.rs").as_path(), &set);
    assert_eq!(category, "editable");
}

#[test]
fn test_get_file_category_readonly() {
    let set = FileSet {
        readonly: vec![PathBuf::from("README.md")],
        ..Default::default()
    };

    let category = get_file_category(PathBuf::from("README.md").as_path(), &set);
    assert_eq!(category, "readonly");
}

#[test]
fn test_get_file_category_generated() {
    let set = FileSet {
        generated: vec![PathBuf::from("target/generated.rs")],
        ..Default::default()
    };

    let category = get_file_category(PathBuf::from("target/generated.rs").as_path(), &set);
    assert_eq!(category, "generated");
}

#[test]
fn test_get_file_category_denied() {
    let set = FileSet {
        denied: vec![(PathBuf::from(".env"), DenyReason::SensitiveFile)],
        ..Default::default()
    };

    let category = get_file_category(PathBuf::from(".env").as_path(), &set);
    assert_eq!(category, "denied");
}

#[test]
fn test_get_file_category_unknown() {
    let set = FileSet::default();

    let category = get_file_category(PathBuf::from("unknown.txt").as_path(), &set);
    assert_eq!(category, "unknown");
}

// ============================================================================
// File Stats Tests
// ============================================================================

#[test]
fn test_get_file_stats() {
    let set = FileSet {
        editable: vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.rs"),
        ],
        readonly: vec![PathBuf::from("README.md")],
        generated: vec![PathBuf::from("target/gen.rs")],
        ..Default::default()
    };

    let stats = get_file_stats(&set);

    assert_eq!(stats.get("editable"), Some(&2));
    assert_eq!(stats.get("readonly"), Some(&1));
    assert_eq!(stats.get("generated"), Some(&1));
    assert_eq!(stats.get("denied"), Some(&0));
}

// ============================================================================
// Permission Check Tests
// ============================================================================

#[test]
fn test_assert_edit_allowed_editable() {
    let temp_dir = std::env::temp_dir().join("test_edit_allowed");
    std::fs::create_dir_all(&temp_dir).ok();
    std::fs::create_dir_all(temp_dir.join("src")).ok();

    let test_file = temp_dir.join("src/main.rs");
    std::fs::write(&test_file, "fn main() {}").unwrap();

    let policy = FilePolicy::default_for_repo(&temp_dir);
    let set = FileSet {
        editable: vec![test_file.clone()],
        ..Default::default()
    };

    let result = assert_edit_allowed(&test_file, &set, &policy);
    assert!(result.is_ok());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_assert_edit_allowed_denied() {
    let temp_dir = std::env::temp_dir().join("test_edit_denied");
    std::fs::create_dir_all(&temp_dir).ok();

    let test_file = temp_dir.join(".env");
    std::fs::write(&test_file, "SECRET=value").unwrap();

    let policy = FilePolicy::default_for_repo(&temp_dir);
    let set = FileSet {
        denied: vec![(test_file.clone(), DenyReason::SensitiveFile)],
        ..Default::default()
    };

    let result = assert_edit_allowed(&test_file, &set, &policy);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("sensitive file"));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_assert_delete_allowed_when_enabled() {
    let temp_dir = std::env::temp_dir().join("test_delete_allowed");
    std::fs::create_dir_all(&temp_dir).ok();

    let test_file = temp_dir.join("old_file.rs");
    std::fs::write(&test_file, "content").unwrap();

    let mut policy = FilePolicy::default_for_repo(&temp_dir);
    policy.allow_delete = true;

    let set = FileSet {
        editable: vec![test_file.clone()],
        ..Default::default()
    };

    let result = assert_delete_allowed(&test_file, &set, &policy);
    assert!(result.is_ok());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_assert_delete_allowed_when_disabled() {
    let temp_dir = std::env::temp_dir().join("test_delete_disabled");
    std::fs::create_dir_all(&temp_dir).ok();

    let test_file = temp_dir.join("old_file.rs");
    std::fs::write(&test_file, "content").unwrap();

    let mut policy = FilePolicy::default_for_repo(&temp_dir);
    policy.allow_delete = false;

    let set = FileSet {
        editable: vec![test_file.clone()],
        ..Default::default()
    };

    let result = assert_delete_allowed(&test_file, &set, &policy);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not allowed"));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_assert_rename_allowed_when_enabled() {
    let temp_dir = std::env::temp_dir().join("test_rename_allowed");
    std::fs::create_dir_all(&temp_dir).ok();

    let from_file = temp_dir.join("old_name.rs");
    let to_file = temp_dir.join("new_name.rs");
    std::fs::write(&from_file, "content").unwrap();

    let mut policy = FilePolicy::default_for_repo(&temp_dir);
    policy.allow_rename = true;

    let set = FileSet {
        editable: vec![from_file.clone()],
        ..Default::default()
    };

    let result = assert_rename_allowed(&from_file, &to_file, &set, &policy);
    assert!(result.is_ok());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_assert_rename_allowed_when_disabled() {
    let temp_dir = std::env::temp_dir().join("test_rename_disabled");
    std::fs::create_dir_all(&temp_dir).ok();

    let from_file = temp_dir.join("old_name.rs");
    let to_file = temp_dir.join("new_name.rs");
    std::fs::write(&from_file, "content").unwrap();

    let mut policy = FilePolicy::default_for_repo(&temp_dir);
    policy.allow_rename = false;

    let set = FileSet {
        editable: vec![from_file.clone()],
        ..Default::default()
    };

    let result = assert_rename_allowed(&from_file, &to_file, &set, &policy);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not allowed"));

    std::fs::remove_dir_all(&temp_dir).ok();
}
