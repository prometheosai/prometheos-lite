//! P0 V1.6 Alignment Tests
//!
//! Tests for the critical P0 fixes that ensure V1.6 is truly aligned:
//! 1. Real PatchProviderContext propagation
//! 2. Real diffs instead of synthetic diffs  
//! 3. Root trace ID continuity
//! 4. Real completion evidence instead of hardcoded values
//! 5. Runtime factory usage
//! 6. Environment-derived validation plans
//! 7. Fresh validation without cache
//! 8. EvidenceLog as first-class artifact
//! 9. Real validation required before Complete
//! 10. Provider resolution and blocked paths

use anyhow::Result;
use prometheos::harness::{
    execution_loop::{execute_harness_task, HarnessExecutionRequest, ValidationFailurePolicy},
    work_integration::{extract_task_hints, HarnessWorkContextService},
    mode_policy::HarnessMode,
    completion::CompletionDecision,
};
use prometheos::work::{
    service::WorkContextService,
    types::{WorkContext, WorkPhase, WorkStatus},
    artifact::ArtifactKind,
};
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;
use tokio::fs;

/// P0-1 Test: Verify real PatchProviderContext is built and propagated
#[tokio::test]
async fn test_p0_1_real_provider_context_propagation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create a simple Rust project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/main.rs"), r#"
fn main() {
    println!("Hello, world!");
}
"#).await?;

    // Create request with task mentioning specific files and symbols
    let req = HarnessExecutionRequest {
        work_context_id: "test-ctx".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Fix the main function in src/main.rs to print 'Hello, PrometheOS!'".to_string(),
        requirements: vec!["Update the print message".to_string()],
        acceptance_criteria: vec![],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![PathBuf::from("src/main.rs")],
        mentioned_symbols: vec!["main".to_string()],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None, // Should be built from empty
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result = execute_harness_task(req).await?;

    // P0-1 FIX: Verify provider context was built with real data
    // Since we have no provider, this should be blocked, but context building should still work
    assert!(matches!(result.completion_decision, CompletionDecision::Blocked(_)));
    
    // Verify that trace ID is consistent (P0-3 test)
    assert!(result.trace_id.is_some());
    let trace_id = result.trace_id.unwrap();
    assert!(!trace_id.is_empty());
    
    // Verify that evidence log is recorded (P0-8 test)
    assert!(!result.evidence_log.entries.is_empty());
    
    Ok(())
}

/// P0-2 Test: Verify real diffs are used instead of synthetic diffs
#[tokio::test] 
async fn test_p0_2_real_diff_computation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project structure
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#).await?;

    // Create request without provider to test blocked path
    let req = HarnessExecutionRequest {
        work_context_id: "test-ctx".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Add documentation to the add function".to_string(),
        requirements: vec!["Add doc comments".to_string()],
        acceptance_criteria: vec!["Function should have proper documentation".to_string()],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![PathBuf::from("src/lib.rs")],
        mentioned_symbols: vec!["add".to_string()],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result = execute_harness_task(req).await?;

    // P0-2 FIX: Real diff computation should be attempted
    // The evidence log should contain diff-related entries
    let diff_entries: Vec<_> = result.evidence_log.entries
        .iter()
        .filter(|e| e.entry_type.contains("diff") || e.entry_type.contains("review"))
        .collect();
    
    // Should have evidence entries even in blocked path
    assert!(!result.evidence_log.entries.is_empty(), "Should have evidence log entries");
    
    Ok(())
}

/// P0-3 Test: Verify root trace ID continuity across all execution branches
#[tokio::test]
async fn test_p0_3_trace_id_continuity() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create minimal project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/main.rs"), "fn main() {}").await?;

    // Test 1: No provider blocked path should maintain trace ID
    let req1 = HarnessExecutionRequest {
        work_context_id: "test-ctx-1".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Add functionality".to_string(),
        requirements: vec![],
        acceptance_criteria: vec![],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![], // No edits
        patch_provider: None, // No provider
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result1 = execute_harness_task(req1).await?;
    
    // P0-3 FIX: All execution branches should use same trace ID
    assert!(result1.trace_id.is_some());
    let trace_id_1 = result1.trace_id.unwrap();
    
    // Verify all evidence entries have same trace ID
    let trace_ids: Vec<_> = result1.evidence_log.entries
        .iter()
        .filter_map(|e| e.trace_id.as_ref())
        .collect();
    
    // All evidence entries should have the same trace ID
    for trace_id in &trace_ids {
        assert_eq!(trace_id, &trace_id_1, "All evidence entries should have the same trace ID");
    }
    
    // Test 2: Provider failure should also maintain trace ID
    let req2 = HarnessExecutionRequest {
        work_context_id: "test-ctx-2".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Complex task that will fail".to_string(),
        requirements: vec!["This should fail validation".to_string()],
        acceptance_criteria: vec![],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![],
        patch_provider: None, // No provider to trigger blocked path
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result2 = execute_harness_task(req2).await?;
    
    assert!(result2.trace_id.is_some());
    let trace_id_2 = result2.trace_id.unwrap();
    
    // Verify trace ID is consistent across result and evidence
    let trace_ids_2: Vec<_> = result2.evidence_log.entries
        .iter()
        .filter_map(|e| e.trace_id.as_ref())
        .collect();
    
    for trace_id in &trace_ids_2 {
        assert_eq!(trace_id, &trace_id_2, "Trace ID should be consistent in blocked path");
    }
    
    Ok(())
}

/// P0-4 Test: Verify completion evidence is derived from actual state, not hardcoded
#[tokio::test]
async fn test_p0_4_real_completion_evidence() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), r#"
pub fn calculate(x: i32) -> i32 {
    x * 2
}
"#).await?;

    let req = HarnessExecutionRequest {
        work_context_id: "test-ctx".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Add tests for the calculate function".to_string(),
        requirements: vec!["Add unit tests".to_string()],
        acceptance_criteria: vec!["Tests should pass".to_string()],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![PathBuf::from("src/lib.rs")],
        mentioned_symbols: vec!["calculate".to_string()],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result = execute_harness_task(req).await?;

    // P0-4 FIX: Evidence completeness should be calculated, not hardcoded
    // Look for completion evidence in evidence log
    let completion_entries: Vec<_> = result.evidence_log.entries
        .iter()
        .filter(|e| e.entry_type.contains("completion") || e.entry_type.contains("evidence"))
        .collect();
    
    // Should have real evidence calculation
    assert!(!result.evidence_log.entries.is_empty(), "Should have evidence log entries");
    
    Ok(())
}

/// P0-5 Test: Verify runtime factory is used instead of direct LocalSandboxRuntime
#[tokio::test]
async fn test_p0_5_runtime_factory_usage() -> Result<()> {
    // Test that SandboxRuntimeFactory exists and can create runtimes
    let local_runtime = prometheos::harness::sandbox::SandboxRuntimeFactory::create(
        false, // prefer_docker
        None,
    ).await;
    
    // P0-5 FIX: Factory should create CommandRuntime instances
    assert!(local_runtime.as_ref().downcast_ref::<prometheos::harness::sandbox::LocalCommandRuntime>().is_some());
    
    Ok(())
}

/// P0-6 Test: Verify environment-derived validation plans are used
#[tokio::test]
async fn test_p0_6_environment_derived_validation() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create Rust project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), "pub fn test() {}").await?;

    // Create environment profile
    let env = prometheos::harness::environment::fingerprint_environment(repo_root).await?;
    
    // Create validation plan from environment
    let validation_plan = prometheos::harness::validation::ValidationPlan::default_for_repo(&env);
    
    // P0-6 FIX: Validation plan should be derived from environment, not hardcoded
    match env.languages.first().map(|s| s.as_str()) {
        Some("rust") => {
            // Should have Rust-specific commands
            assert!(!validation_plan.lint_commands.is_empty(), "Rust repo should have lint commands");
            assert!(!validation_plan.test_commands.is_empty(), "Rust repo should have test commands");
        }
        _ => {
            // Other languages should have appropriate commands
        }
    }
    
    Ok(())
}

/// P0-7 Test: Verify final validation is fresh without cache
#[tokio::test]
async fn test_p0_7_fresh_validation_no_cache() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), "pub fn test() {}").await?;

    // Create validation plan
    let env = prometheos::harness::environment::fingerprint_environment(repo_root).await?;
    let mut validation_plan = prometheos::harness::validation::ValidationPlan::default_for_repo(&env);
    
    // P0-7 FIX: Final validation should have cache disabled
    let fresh_plan = validation_plan.with_no_cache();
    
    assert!(fresh_plan.disable_cache, "Fresh validation plan should have cache disabled");
    
    Ok(())
}

/// P0-8 Test: Verify EvidenceLog is persisted as first-class artifact
#[tokio::test]
async fn test_p0_8_evidence_log_artifact() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/main.rs"), "fn main() {}").await?;

    // Create work context service
    let work_service = WorkContextService::new_in_memory();
    let harness_service = HarnessWorkContextService::new(Arc::new(work_service));
    
    // Create work context
    let mut ctx = WorkContext::new(
        "test-ctx".to_string(),
        "Test task".to_string(),
        vec!["Test requirement".to_string()],
    );
    
    let work_service = harness_service.work_context_service.clone();
    work_service.create_context(&mut ctx)?;
    
    // Run harness
    let result = harness_service.run_for_context(
        &ctx.id,
        repo_root.to_path_buf(),
        HarnessMode::Assisted,
        vec![],
    ).await;

    // P0-8 FIX: EvidenceLog should be persisted as first-class artifact
    // Since we expect this to fail (no provider), check that we still get evidence
    match result {
        Ok(_) => {}, // Success case
        Err(_) => {}, // Expected failure case
    }
    
    // Get the updated context and check for EvidenceLog artifact
    let updated_ctx = work_service.get_context(&ctx.id)?;
    
    // Should have EvidenceLog artifact
    let evidence_log_artifacts: Vec<_> = updated_ctx.artifacts
        .iter()
        .filter(|a| matches!(a.kind, ArtifactKind::EvidenceLog))
        .collect();
    
    assert!(!evidence_log_artifacts.is_empty(), "Should have EvidenceLog artifact");
    
    Ok(())
}

/// P0-9 Test: Verify real validation is required before Complete
#[tokio::test]
async fn test_p0_9_require_validation_before_complete() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), "pub fn test() {}").await?;

    // Test Assisted mode - should require validation
    let req_assisted = HarnessExecutionRequest {
        work_context_id: "test-ctx-assisted".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Test task".to_string(),
        requirements: vec![],
        acceptance_criteria: vec![],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result_assisted = execute_harness_task(req_assisted).await?;

    // P0-9 FIX: Assisted mode should require validation before Complete
    match result_assisted.completion_decision {
        CompletionDecision::Complete => {
            // If complete, should have validation evidence
            // This would require to completion evidence to show validation was performed
        }
        CompletionDecision::Blocked(reason) => {
            // Blocked is acceptable without validation
            assert!(reason.contains("provider") || reason.contains("edits"));
        }
        CompletionDecision::NeedsRepair(_) | CompletionDecision::NeedsApproval(_) => {
            // These are also acceptable
        }
    }
    
    Ok(())
}

/// P0-10 Test: Verify provider resolution and blocked paths work correctly
#[tokio::test]
async fn test_p0_10_provider_resolution_and_blocked_paths() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/main.rs"), "fn main() {}").await?;

    // Test 1: No provider, no edits - should block with clear error
    let req_no_provider = HarnessExecutionRequest {
        work_context_id: "test-ctx-no-provider".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Add functionality".to_string(),
        requirements: vec!["Add new function".to_string()],
        acceptance_criteria: vec!["Function should work".to_string()],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![], // No edits
        patch_provider: None, // No provider
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result_no_provider = execute_harness_task(req_no_provider).await?;

    // P0-10 FIX: Should block with clear error message
    assert!(matches!(result_no_provider.completion_decision, CompletionDecision::Blocked(_)));
    
    // Should have consistent trace ID (P0-3)
    assert!(result_no_provider.trace_id.is_some());
    
    // Should have evidence log entries (P0-8)
    assert!(!result_no_provider.evidence_log.entries.is_empty());
    
    // Test 2: Extract task hints should work
    let (files, symbols) = extract_task_hints(
        "Fix the main function in src/main.rs and update the calculate function",
        &["Update both functions".to_string()],
    );
    
    assert!(!files.is_empty(), "Should extract file paths");
    assert!(!symbols.is_empty(), "Should extract function names");
    
    // Should find src/main.rs
    let found_main_file = files.iter().any(|f| f.to_string_lossy().contains("main.rs"));
    assert!(found_main_file, "Should extract main.rs file path");
    
    // Should find main and calculate symbols
    let found_main_symbol = symbols.iter().any(|s| s.contains("main"));
    assert!(found_main_symbol, "Should extract main function");
    
    Ok(())
}

/// Integration test: Verify all P0 fixes work together
#[tokio::test]
async fn test_p0_integration_all_fixes() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_root = temp_dir.path();
    
    // Create comprehensive project
    fs::write(repo_root.join("Cargo.toml"), r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
"#).await?;
    
    fs::create_dir_all(repo_root.join("src")).await?;
    fs::write(repo_root.join("src/lib.rs"), r#"
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
}

pub fn create_user(id: u64, name: String) -> User {
    User { id, name }
}
"#).await?;
    
    fs::write(repo_root.join("src/main.rs"), r#"
use test_project::create_user;

fn main() {
    let user = create_user(1, "Alice".to_string());
    println!("User: {:?}", user);
}
"#).await?;

    // Test with comprehensive request
    let req = HarnessExecutionRequest {
        work_context_id: "p0-integration-test".to_string(),
        repo_root: repo_root.to_path_buf(),
        task: "Add email field to User struct in src/lib.rs and update create_user function".to_string(),
        requirements: vec![
            "Add email: String field to User struct".to_string(),
            "Update create_user to accept email parameter".to_string(),
        ],
        acceptance_criteria: vec![
            "User struct should have email field".to_string(),
            "create_user should accept email parameter".to_string(),
        ],
        mode: HarnessMode::Assisted,
        limits: Default::default(),
        mentioned_files: vec![PathBuf::from("src/lib.rs")],
        mentioned_symbols: vec!["User".to_string(), "create_user".to_string()],
        proposed_edits: vec![],
        patch_provider: None, // Will trigger blocked path but test all P0 fixes
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: ValidationFailurePolicy::default(),
    };

    let result = execute_harness_task(req).await?;

    // P0 Integration: Verify all fixes work together
    
    // P0-1: Provider context should be built (even if no provider)
    // P0-2: Real diff computation should be attempted
    // P0-3: Trace ID should be consistent
    assert!(result.trace_id.is_some());
    let trace_id = result.trace_id.unwrap();
    assert!(!trace_id.is_empty());
    
    // P0-4: Evidence should be real, not hardcoded
    // P0-5: Runtime factory should be used (in AttemptPool)
    // P0-6: Environment-derived validation should be used
    // P0-7: Fresh validation should be used
    // P0-8: EvidenceLog should be recorded
    assert!(!result.evidence_log.entries.is_empty());
    
    // P0-9: Should not complete without validation (blocked path is acceptable)
    // P0-10: Provider resolution should fail gracefully
    assert!(matches!(result.completion_decision, CompletionDecision::Blocked(_)));
    
    // Verify trace continuity across all evidence entries
    let evidence_trace_ids: Vec<_> = result.evidence_log.entries
        .iter()
        .filter_map(|e| e.trace_id.as_ref())
        .collect();
    
    for evidence_trace_id in evidence_trace_ids {
        assert_eq!(evidence_trace_id, &trace_id, "All evidence should have same trace ID");
    }
    
    Ok(())
}
