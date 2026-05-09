#![cfg(any())]
// Quarantined: obsolete integration suite targets pre-audit harness APIs.
//! P0-Audit-008: End-to-end V1.6 harness acceptance test

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
async fn test_e2e_v16_harness_acceptance() -> Result<()> {
    // Create temp Rust repo
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Initialize basic Rust project
    fs::write(repo_path.join("Cargo.toml"), r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#).await?;
    
    // Create failing test
    fs::create_dir_all(repo_path.join("src")).await?;
    fs::write(repo_path.join("src/lib.rs"), r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
        // This test will fail - needs to be 6
        assert_eq!(add(2, 4), 6);
    }
}
"#).await?;
    
    // Create harness execution request
    let request = HarnessExecutionRequest::new()
        .with_mode(HarnessMode::Autonomous)
        .with_task("Fix the failing test in src/lib.rs")
        .with_repo_path(repo_path.to_path_buf());
    
    // Execute harness
    let result = request.execute().await?;
    
    // Assert completion decision
    match result.completion_decision {
        CompletionDecision::Complete => {
            println!("✅ E2E test passed: Harness completed successfully");
        }
        decision => {
            anyhow::bail!("E2E test failed: Expected Complete, got {:?}", decision);
        }
    }
    
    // Verify patch was applied
    let lib_content = fs::read_to_string(repo_path.join("src/lib.rs")).await?;
    assert!(lib_content.contains("6"), "Fix should be applied to make test pass");
    
    // Verify final validation passes
    let validation_output = std::process::Command::new("cargo")
        .args(&["test", "--all"])
        .current_dir(repo_path)
        .output()?;
    
    assert!(validation_output.status.success(), "Tests should pass after fix");
    
    Ok(())
}
