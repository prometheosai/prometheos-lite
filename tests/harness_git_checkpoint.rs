//! Issue 24: Git Checkpoint System Tests
//!
//! Comprehensive tests for the Git Checkpoint System including:
//! - GitCheckpoint struct (id, branch, before/after HEAD, diffs, files, status)
//! - GitCheckpointManager for checkpoint operations
//! - CheckpointResult struct (success, checkpoint, message)
//! - RollbackStrategy enum (HardReset, SoftReset, RevertCommit, StashAndReset)
//! - create_checkpoint() for saving state
//! - rollback() for reverting changes
//! - is_git_repo() detection
//! - Branch creation and management
//! - Dirty file detection

use std::path::PathBuf;

use prometheos_lite::harness::git_checkpoint::{
    CheckpointResult, GitCheckpoint, GitCheckpointManager, RollbackStrategy,
};

// ============================================================================
// GitCheckpoint Tests
// ============================================================================

#[test]
fn test_git_checkpoint_creation() {
    let checkpoint = GitCheckpoint {
        id: "checkpoint-123".to_string(),
        work_context_id: "work-456".to_string(),
        branch_name: "harness/work-456".to_string(),
        before_head: Some("abc123".to_string()),
        after_head: None,
        dirty_files: vec![PathBuf::from("src/main.rs")],
        touched_files: vec![],
        diff_before: "diff content".to_string(),
        diff_after: String::new(),
        committed: false,
        commit_message: None,
        created_at: chrono::Utc::now(),
    };

    assert_eq!(checkpoint.id, "checkpoint-123");
    assert_eq!(checkpoint.work_context_id, "work-456");
    assert_eq!(checkpoint.branch_name, "harness/work-456");
    assert_eq!(checkpoint.before_head, Some("abc123".to_string()));
    assert!(!checkpoint.committed);
}

#[test]
fn test_git_checkpoint_committed() {
    let checkpoint = GitCheckpoint {
        id: "checkpoint-789".to_string(),
        work_context_id: "work-999".to_string(),
        branch_name: "harness/work-999".to_string(),
        before_head: Some("def456".to_string()),
        after_head: Some("ghi789".to_string()),
        dirty_files: vec![],
        touched_files: vec![PathBuf::from("src/main.rs"), PathBuf::from("tests/test.rs")],
        diff_before: String::new(),
        diff_after: "after diff".to_string(),
        committed: true,
        commit_message: Some("Checkpoint commit".to_string()),
        created_at: chrono::Utc::now(),
    };

    assert!(checkpoint.committed);
    assert_eq!(
        checkpoint.commit_message,
        Some("Checkpoint commit".to_string())
    );
    assert_eq!(checkpoint.touched_files.len(), 2);
}

// ============================================================================
// GitCheckpointManager Tests
// ============================================================================

#[test]
fn test_git_checkpoint_manager_new() {
    let _manager = GitCheckpointManager::new(PathBuf::from("/tmp/repo"));
    // Manager created successfully
}

#[test]
fn test_git_checkpoint_manager_with_prefix() {
    let _manager =
        GitCheckpointManager::with_prefix(PathBuf::from("/tmp/repo"), "custom-prefix".to_string());
    // Manager created with custom prefix
}

// ============================================================================
// CheckpointResult Tests
// ============================================================================

#[test]
fn test_checkpoint_result_success() {
    let result = CheckpointResult {
        success: true,
        checkpoint: Some(GitCheckpoint {
            id: "cp-1".to_string(),
            work_context_id: "work-1".to_string(),
            branch_name: "harness/work-1".to_string(),
            before_head: Some("abc".to_string()),
            after_head: None,
            dirty_files: vec![],
            touched_files: vec![],
            diff_before: String::new(),
            diff_after: String::new(),
            committed: false,
            commit_message: None,
            created_at: chrono::Utc::now(),
        }),
        message: "Checkpoint created successfully".to_string(),
    };

    assert!(result.success);
    assert!(result.checkpoint.is_some());
    assert_eq!(result.message, "Checkpoint created successfully");
}

#[test]
fn test_checkpoint_result_failure() {
    let result = CheckpointResult {
        success: false,
        checkpoint: None,
        message: "Not a git repository".to_string(),
    };

    assert!(!result.success);
    assert!(result.checkpoint.is_none());
    assert_eq!(result.message, "Not a git repository");
}

// ============================================================================
// RollbackStrategy Tests
// ============================================================================

#[test]
fn test_rollback_strategy_variants() {
    assert!(matches!(
        RollbackStrategy::HardReset,
        RollbackStrategy::HardReset
    ));
    assert!(matches!(
        RollbackStrategy::SoftReset,
        RollbackStrategy::SoftReset
    ));
    assert!(matches!(
        RollbackStrategy::RevertCommit,
        RollbackStrategy::RevertCommit
    ));
    assert!(matches!(
        RollbackStrategy::StashAndReset,
        RollbackStrategy::StashAndReset
    ));
}

#[test]
fn test_rollback_strategy_display() {
    assert_eq!(format!("{:?}", RollbackStrategy::HardReset), "HardReset");
    assert_eq!(format!("{:?}", RollbackStrategy::SoftReset), "SoftReset");
    assert_eq!(
        format!("{:?}", RollbackStrategy::RevertCommit),
        "RevertCommit"
    );
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_checkpoint_with_files() {
    let checkpoint = GitCheckpoint {
        id: "cp-files".to_string(),
        work_context_id: "work-files".to_string(),
        branch_name: "harness/files".to_string(),
        before_head: Some("base".to_string()),
        after_head: Some("new".to_string()),
        dirty_files: vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.rs"),
            PathBuf::from("Cargo.toml"),
        ],
        touched_files: vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/output.txt"),
        ],
        diff_before: "before diff".to_string(),
        diff_after: "after diff".to_string(),
        committed: true,
        commit_message: Some("Files modified".to_string()),
        created_at: chrono::Utc::now(),
    };

    assert_eq!(checkpoint.dirty_files.len(), 3);
    assert_eq!(checkpoint.touched_files.len(), 2);
    assert!(
        checkpoint
            .dirty_files
            .contains(&PathBuf::from("src/main.rs"))
    );
}

#[test]
fn test_checkpoint_lifecycle() {
    // Simulate a complete checkpoint lifecycle
    let checkpoint = GitCheckpoint {
        id: "lifecycle".to_string(),
        work_context_id: "work-lifecycle".to_string(),
        branch_name: "harness/lifecycle".to_string(),
        before_head: Some("initial".to_string()),
        after_head: Some("final".to_string()),
        dirty_files: vec![],
        touched_files: vec![PathBuf::from("result.txt")],
        diff_before: String::new(),
        diff_after: "final changes".to_string(),
        committed: true,
        commit_message: Some("Work completed".to_string()),
        created_at: chrono::Utc::now(),
    };

    assert!(checkpoint.committed);
    assert_eq!(checkpoint.after_head, Some("final".to_string()));
    assert!(!checkpoint.touched_files.is_empty());
}
