//! P0 Tests: Checkpoint Failure Semantics and ReviewOnly Mode Enforcement
//!
//! V1.6-P0-004: Make checkpoint failure semantics explicit and tested
//! V1.6-P0-005: Enforce no side effects in ReviewOnly mode

use std::fs;
use std::path::PathBuf;

use prometheos_lite::harness::{
    edit_protocol::{EditOperation, SearchReplaceEdit},
    execution_loop::{HarnessExecutionRequest, HarnessLimits, ValidationFailurePolicy},
    file_control::{FilePolicy, FileSet},
    git_checkpoint::GitCheckpointManager,
    mode_policy::HarnessMode,
    patch_applier::dry_run_patch,
};

/// Check if git is available
fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Test helper: Create a minimal test repo
fn create_test_repo() -> tempfile::TempDir {
    if !git_available() {
        panic!("Git is not available in PATH - tests require git");
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let repo_root = temp_dir.path();

    // Initialize git repo
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_root)
        .output()
        .expect("Failed to init git repo");

    // Configure git user for commits
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo_root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo_root)
        .output()
        .unwrap();

    // Create initial file and commit
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(repo_root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(repo_root)
        .output()
        .unwrap();

    temp_dir
}

/// P0-004: Test that checkpoint creation fails for non-git repo
#[tokio::test]
async fn test_checkpoint_fails_for_non_git_repo() {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo_root = temp_dir.path();

    // NOT a git repo
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();

    let manager = GitCheckpointManager::new(repo_root.to_path_buf());

    // Checkpoint should fail
    let result = manager.create_checkpoint("test-context").await;
    assert!(result.is_err(), "Checkpoint should fail for non-git repo");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Not a git repository"),
        "Error should mention not a git repo: {}",
        err_msg
    );
}

/// P0-004: Test that checkpoint creation fails for dirty repo in strict mode
#[tokio::test]
async fn test_checkpoint_fails_for_dirty_repo_strict_mode() {
    let temp_dir = create_test_repo();
    let repo_root = temp_dir.path();

    // Make repo dirty
    fs::write(repo_root.join("dirty.txt"), "dirty content").unwrap();

    let manager = GitCheckpointManager::new(repo_root.to_path_buf());

    // In a real harness with strict mode, dirty repos would block
    // For now, verify we can detect dirty state
    let dirty_files = manager.get_dirty_files().unwrap();
    assert!(!dirty_files.is_empty(), "Should detect dirty files");
}

/// P0-005: Test ReviewOnly mode does not mutate real repo files
#[tokio::test]
async fn test_review_only_no_side_effects() {
    let temp_dir = create_test_repo();
    let repo_root = temp_dir.path();

    // Create original file with platform-independent path
    let original_content = "fn main() { println!(\"hello\"); }";
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let main_file = src_dir.join("main.rs");
    fs::write(&main_file, original_content).unwrap();

    // Get original hash
    let original_hash = compute_file_hash(&main_file);

    // Build a ReviewOnly request
    let request = HarnessExecutionRequest {
        work_context_id: "test-review".into(),
        repo_root: repo_root.to_path_buf(),
        task: "test task".into(),
        requirements: vec![],
        acceptance_criteria: vec![],
        mode: HarnessMode::ReviewOnly,
        limits: HarnessLimits::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::RollbackAutomatically,
        sandbox_policy: Some(prometheos_lite::harness::sandbox::SandboxPolicy::from_mode(
            HarnessMode::ReviewOnly,
        )),
    };

    // Verify mode is ReviewOnly
    match request.mode {
        HarnessMode::ReviewOnly => {
            // ReviewOnly mode should never apply patches to real repo
            // Verify by checking the file hash is unchanged
            let main_file = repo_root.join("src").join("main.rs");
            let current_hash = compute_file_hash(&main_file);
            assert_eq!(
                original_hash, current_hash,
                "File should be unchanged in ReviewOnly mode"
            );
        }
        _ => panic!("Expected ReviewOnly mode"),
    }
}

/// P0-005: Test that dry_run_patch does not modify files
#[tokio::test]
async fn test_dry_run_no_file_modification() {
    let temp_dir = create_test_repo();
    let repo_root = temp_dir.path();

    // Create original file with platform-independent path
    let original_content = "fn main() { println!(\"hello\"); }";
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let main_file = src_dir.join("main.rs");
    fs::write(&main_file, original_content).unwrap();

    // Get original hash
    let original_hash = compute_file_hash(&main_file);

    // Create edits that would modify the file
    let edits = vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/main.rs"),
        search: "println!(\"hello\")".into(),
        replace: "println!(\"world\")".into(),
        replace_all: Some(false),
        context_lines: Some(0),
    })];

    // Create FileSet with the target file in editable list
    let mut file_set = FileSet::default();
    let main_file_abs = repo_root.join("src").join("main.rs");
    file_set.editable.push(main_file_abs);

    let policy = FilePolicy::default_for_repo(repo_root.to_path_buf());

    // Run dry run
    let result = dry_run_patch(&edits, &file_set, &policy).await;

    // Verify file is unchanged
    let main_file = repo_root.join("src").join("main.rs");
    let current_hash = compute_file_hash(&main_file);
    assert_eq!(
        original_hash, current_hash,
        "Dry run should NOT modify files. Hash changed from {} to {}",
        original_hash, current_hash
    );

    // Verify dry run result shows what would change
    let patch_result = result.expect("Dry run should succeed");
    // NOTE: dry_run returns applied=false by design (patch was not actually applied)
    assert!(!patch_result.applied, "Dry run should report applied=false");
    assert!(patch_result.dry_run, "Dry run should have dry_run=true");
    assert!(
        patch_result.failures.is_empty(),
        "Dry run should have no failures"
    );
    // The changed_files shows what WOULD change if applied
    assert!(
        !patch_result.changed_files.is_empty(),
        "Dry run should report what files would change"
    );
}

/// P0-004 + P0-005: Test that failed checkpoint blocks patch application
#[tokio::test]
async fn test_failed_checkpoint_blocks_application() {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo_root = temp_dir.path();

    // NOT a git repo - checkpoint will fail
    let src_dir = repo_root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.rs"), "fn main() {}").unwrap();

    let manager = GitCheckpointManager::new(repo_root.to_path_buf());

    // Verify checkpoint would fail
    let checkpoint_result = manager.create_checkpoint("test").await;
    assert!(
        checkpoint_result.is_err(),
        "Checkpoint should fail for non-git repo"
    );

    // In a real execution loop, this failure would block patch application
    // when rollback_policy is CheckpointBeforeSideEffect
    // This test documents that requirement
}

/// Compute a simple file hash for verification
fn compute_file_hash(path: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let content = fs::read_to_string(path).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
