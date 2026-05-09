//! P0-Audit-009: Negative-path tests for failure scenarios

use anyhow::Result;
use prometheos_lite::harness::{
    completion::CompletionDecision, 
    execution_loop::HarnessExecutionRequest,
    mode_policy::HarnessMode,
};
use std::path::Path;
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_provider_returns_invalid_file_path() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create basic project
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    fs::create_dir_all(repo_path.join("src")).await?;
    fs::write(repo_path.join("src/lib.rs"), "pub fn test() {}").await?;
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Create a file at /etc/passwd (invalid path)")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete due to invalid file path
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Invalid file path test passed");
    
    Ok(())
}

#[tokio::test]
async fn test_provider_returns_no_candidates() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create empty project
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Do nothing - should generate no candidates")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete when no candidates
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ No candidates test passed");
    
    Ok(())
}

#[tokio::test]
async fn test_candidate_fails_dry_run() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create project with syntax error that will fail dry-run
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    fs::create_dir_all(repo_path.join("src")).await?;
    fs::write(repo_path.join("src/lib.rs"), "pub fn test() {}").await?;
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Add invalid Rust syntax: fn test( {")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete due to dry-run failure
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Dry-run failure test passed");
    
    Ok(())
}

#[tokio::test]
async fn test_validation_has_zero_commands() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create project without any validation setup
    fs::write(repo_path.join("README.md"), "# Test Project").await?;
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Add a comment to README")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete when validation has zero commands
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Zero validation commands test passed");
    
    Ok(())
}

#[tokio::test]
async fn test_docker_required_but_unavailable() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create basic project
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    
    // Mock Docker unavailability by setting environment
    std::env::set_var("PROMETHEOS_DOCKER_UNAVAILABLE", "1");
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous) // Requires Docker
        .with_task("Add a simple function")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete when Docker required but unavailable
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Docker unavailable test passed");
    
    // Clean up
    std::env::remove_var("PROMETHEOS_DOCKER_UNAVAILABLE");
    
    Ok(())
}

#[tokio::test]
async fn test_patch_hash_mismatch() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create project
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    fs::create_dir_all(repo_path.join("src")).await?;
    fs::write(repo_path.join("src/lib.rs"), "pub fn original() {}").await?;
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Add a function")
        .with_repo_path(repo_path.to_path_buf());
    
    // Simulate hash mismatch by modifying file after patch generation
    // but before application (this would be detected by patch identity checks)
    std::env::set_var("PROMETHEOS_SIMULATE_HASH_MISMATCH", "1");
    
    let result = request.execute().await?;
    
    // Should not complete due to hash mismatch
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Hash mismatch test passed");
    
    // Clean up
    std::env::remove_var("PROMETHEOS_SIMULATE_HASH_MISMATCH");
    
    Ok(())
}

#[tokio::test]
async fn test_rollback_failure() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Create project
    fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").await?;
    fs::create_dir_all(repo_path.join("src")).await?;
    fs::write(repo_path.join("src/lib.rs"), "pub fn test() {}").await?;
    
    // Simulate rollback failure
    std::env::set_var("PROMETHEOS_ROLLBACK_FAILURE", "1");
    
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Add a function that will cause rollback failure")
        .with_repo_path(repo_path.to_path_buf());
    
    let result = request.execute().await?;
    
    // Should not complete when rollback fails
    assert_ne!(result.completion_decision, CompletionDecision::Complete);
    println!("✅ Rollback failure test passed");
    
    // Clean up
    std::env::remove_var("PROMETHEOS_ROLLBACK_FAILURE");
    
    Ok(())
}
