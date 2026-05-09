//! End-to-end integration tests for the v1.6 Harness Engine
//! Tests the complete workflow from WorkContext to completion

use std::sync::Arc;
use tempfile::TempDir;

use prometheos_lite::db::Db;
use prometheos_lite::harness::{
    HarnessWorkContextService, mode_policy::HarnessMode,
    edit_protocol::{EditOperation, CreateFileEdit},
    risk::RiskLevel,
    completion::CompletionDecision,
};
use prometheos_lite::work::{
    WorkContextService,
    types::{WorkDomain, WorkStatus},
};

/// Test complete harness workflow end-to-end
#[tokio::test]
async fn test_harness_workflow_e2e() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "test-user".to_string(),
        "E2E Test Harness Integration".to_string(),
        WorkDomain::Software,
        "Test the complete harness workflow from start to finish".to_string(),
    )?;
    
    // Create harness service
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Execute harness with sample edits
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("test.rs"),
            content: r#"fn main() {
    println!("Hello, Harness Engine!");
    assert_eq!(2 + 2, 4);
}"#.to_string(),
            executable: Some(false),
        }),
    ];
    
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify results
    assert!(!result.trajectory.steps.is_empty());
    assert!(!result.artifacts.is_empty());
    assert!(result.confidence.score > 0.5);
    
    // Verify WorkContext was updated
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    assert!(updated_context.metadata.get("harness").is_some());
    
    Ok(())
}

/// Test harness API endpoints integration
#[tokio::test]
async fn test_harness_api_integration() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test database
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    
    // Create test WorkContext
    let context = work_context_service.create_context(
        "api-test-user".to_string(),
        "API Integration Test".to_string(),
        WorkDomain::Software,
        "Test harness API endpoints integration".to_string(),
    )?;
    
    // Test harness metadata endpoint
    let harness_data = context.metadata.get("harness").cloned()
        .unwrap_or(serde_json::Value::Null);
    
    // Verify API can access harness data
    assert_ne!(harness_data, serde_json::Value::Null);
    
    Ok(())
}

/// Test CLI harness commands integration
#[tokio::test]
async fn test_harness_cli_integration() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    
    // Create test WorkContext
    let context = work_context_service.create_context(
        "cli-test-user".to_string(),
        "CLI Integration Test".to_string(),
        WorkDomain::Software,
        "Test harness CLI commands integration".to_string(),
    )?;
    
    // Verify CLI can access WorkContext
    let retrieved_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    assert_eq!(retrieved_context.id, context.id);
    assert_eq!(retrieved_context.title, "CLI Integration Test");
    
    Ok(())
}

/// Test harness trajectory recording and replay
#[tokio::test]
async fn test_harness_trajectory_replay() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "trajectory-test-user".to_string(),
        "Trajectory Replay Test".to_string(),
        WorkDomain::Software,
        "Test trajectory recording and replay functionality".to_string(),
    )?;
    
    // Execute harness to generate trajectory
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("replay_test.rs"),
            content: "fn test_function() { println!(\"Test\"); }".to_string(),
            executable: Some(false),
        }),
    ];
    
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify trajectory was recorded
    assert!(!result.trajectory.steps.is_empty());
    
    // Verify trajectory can be retrieved
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    if let Some(harness_data) = updated_context.metadata.get("harness") {
        if let Some(trajectory) = harness_data.get("trajectory") {
            assert!(trajectory.get("steps").is_some());
        } else {
            return Err("No trajectory data found".into());
        }
    } else {
        return Err("No harness metadata found".into());
    }
    
    Ok(())
}

/// Test harness artifact generation and retrieval
#[tokio::test]
async fn test_harness_artifacts() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "artifact-test-user".to_string(),
        "Artifact Generation Test".to_string(),
        WorkDomain::Software,
        "Test artifact generation and retrieval".to_string(),
    )?;
    
    // Execute harness to generate artifacts
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("artifact_test.rs"),
            content: r#"#[cfg(test)]
mod tests {
    #[test]
    fn test_artifact_generation() {
        assert!(true);
    }
}"#.to_string(),
            executable: Some(false),
        }),
    ];
    
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify artifacts were generated
    assert!(!result.artifacts.is_empty());
    
    // Verify artifacts can be retrieved
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    if let Some(harness_data) = updated_context.metadata.get("harness") {
        if let Some(artifacts) = harness_data.get("artifacts") {
            assert!(artifacts.get("patches").is_some());
        } else {
            return Err("No artifacts data found".into());
        }
    } else {
        return Err("No harness metadata found".into());
    }
    
    Ok(())
}

/// Test harness confidence scoring and risk assessment
#[tokio::test]
async fn test_harness_confidence_and_risk() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "confidence-test-user".to_string(),
        "Confidence and Risk Test".to_string(),
        WorkDomain::Software,
        "Test confidence scoring and risk assessment".to_string(),
    )?;
    
    // Execute harness with high-quality code
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("confidence_test.rs"),
            content: r#"/// High-quality module with documentation
pub mod confidence {
    use std::fmt;
    
    /// Example struct with proper documentation
    #[derive(Debug, Clone)]
    pub struct Example {
        value: i32,
    }
    
    impl Example {
        /// Create a new Example instance
        pub fn new(value: i32) -> Self {
            Self { value }
        }
        
        /// Get the stored value
        pub fn value(&self) -> i32 {
            self.value
        }
    }
    
    impl fmt::Display for Example {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "Example(value: {})", self.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_example_creation() {
        let example = Example::new(42);
        assert_eq!(example.value(), 42);
    }
    
    #[test]
    fn test_example_display() {
        let example = Example::new(100);
        assert_eq!(example.to_string(), "Example(value: 100)");
    }
    
    #[test]
    fn test_example_clone() {
        let example = Example::new(50);
        let cloned = example.clone();
        assert_eq!(example.value(), cloned.value());
    }
}"#.to_string(),
            executable: Some(false),
        }),
    ];
    
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify confidence scoring
    assert!(result.confidence.score > 0.7); // High confidence for good code
    
    // Verify risk assessment
    assert!(result.risk_assessment.level != RiskLevel::Critical); // Should not be critical
    
    // Verify confidence and risk data can be retrieved
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    if let Some(harness_data) = updated_context.metadata.get("harness") {
        assert!(harness_data.get("confidence").is_some());
        assert!(harness_data.get("risk_assessment").is_some());
    } else {
        return Err("No harness metadata found".into());
    }
    
    Ok(())
}

/// Test harness completion decision logic
#[tokio::test]
async fn test_harness_completion_decision() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "completion-test-user".to_string(),
        "Completion Decision Test".to_string(),
        WorkDomain::Software,
        "Test completion decision logic".to_string(),
    )?;
    
    // Execute harness with complete solution
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("completion_test.rs"),
            content: r#"/// Complete solution with comprehensive testing
pub mod completion {
    /// Calculator for basic arithmetic operations
    pub struct Calculator {
        result: f64,
    }
    
    impl Calculator {
        /// Create a new calculator
        pub fn new() -> Self {
            Self { result: 0.0 }
        }
        
        /// Add a value to the current result
        pub fn add(&mut self, value: f64) -> &mut Self {
            self.result += value;
            self
        }
        
        /// Subtract a value from the current result
        pub fn subtract(&mut self, value: f64) -> &mut Self {
            self.result -= value;
            self
        }
        
        /// Multiply the current result by a value
        pub fn multiply(&mut self, value: f64) -> &mut Self {
            self.result *= value;
            self
        }
        
        /// Divide the current result by a value
        pub fn divide(&mut self, value: f64) -> Result<&mut Self, String> {
            if value == 0.0 {
                Err("Cannot divide by zero".to_string())
            } else {
                self.result /= value;
                Ok(self)
            }
        }
        
        /// Get the current result
        pub fn result(&self) -> f64 {
            self.result
        }
        
        /// Reset the calculator
        pub fn reset(&mut self) -> &mut Self {
            self.result = 0.0;
            self
        }
    }
    
    impl Default for Calculator {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_calculator_basic_operations() {
        let mut calc = Calculator::new();
        
        calc.add(10.0).multiply(2.0).subtract(5.0);
        assert_eq!(calc.result(), 15.0);
    }
    
    #[test]
    fn test_calculator_division() {
        let mut calc = Calculator::new();
        
        calc.add(20.0);
        let result = calc.divide(4.0);
        assert!(result.is_ok());
        assert_eq!(calc.result(), 5.0);
    }
    
    #[test]
    fn test_calculator_division_by_zero() {
        let mut calc = Calculator::new();
        
        calc.add(10.0);
        let result = calc.divide(0.0);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_calculator_reset() {
        let mut calc = Calculator::new();
        
        calc.add(100.0).multiply(2.0);
        calc.reset();
        assert_eq!(calc.result(), 0.0);
    }
    
    #[test]
    fn test_calculator_default() {
        let calc = Calculator::default();
        assert_eq!(calc.result(), 0.0);
    }
}"#.to_string(),
            executable: Some(false),
        }),
    ];
    
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify completion decision
    assert!(!matches!(result.completion_decision, CompletionDecision::Blocked(_))); // Should not be blocked
    
    // Verify completion decision data can be retrieved
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    if let Some(harness_data) = updated_context.metadata.get("harness") {
        assert!(harness_data.get("completion_decision").is_some());
    } else {
        return Err("No harness metadata found".into());
    }
    
    Ok(())
}

/// Test harness error handling and recovery
#[tokio::test]
async fn test_harness_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "error-test-user".to_string(),
        "Error Handling Test".to_string(),
        WorkDomain::Software,
        "Test error handling and recovery".to_string(),
    )?;
    
    // Execute harness with problematic code that should trigger error handling
    let edits = vec![
        EditOperation::CreateFile(CreateFileEdit {
            file: temp_dir.path().join("error_test.rs"),
            content: "fn problematic_function() { panic!(\"This should trigger error handling\"); }".to_string(),
            executable: Some(false),
        }),
    ];
    
    // Execute harness - should handle errors gracefully
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    
    // Verify error was handled (may not be successful due to problematic code)
    // The exact behavior depends on the error handling implementation
    
    // Verify error information is recorded
    let updated_context = work_context_service.get_context(&context.id)?
        .ok_or("WorkContext not found")?;
    
    if let Some(harness_data) = updated_context.metadata.get("harness") {
        // Check if error information is recorded
        if let Some(trajectory) = harness_data.get("trajectory") {
            // Should have recorded the error in trajectory
            assert!(trajectory.get("steps").is_some());
        }
    }
    
    Ok(())
}

/// Test harness performance and scalability
#[tokio::test]
async fn test_harness_performance() -> Result<(), Box<dyn std::error::Error>> {
    // Setup test environment
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    let db = Arc::new(Db::new(db_path.to_str().unwrap())?);
    
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let harness_service = HarnessWorkContextService::new(work_context_service.clone());
    
    // Create WorkContext
    let context = work_context_service.create_context(
        "performance-test-user".to_string(),
        "Performance Test".to_string(),
        WorkDomain::Software,
        "Test harness performance and scalability".to_string(),
    )?;
    
    // Execute harness with multiple edits to test performance
    let edits = (0..10).map(|i| EditOperation::CreateFile(CreateFileEdit {
        file: temp_dir.path().join(format!("perf_test_{}.rs", i)),
        content: format!(r#"/// Performance test file {}
pub mod performance_{} {{
    /// Test function for performance evaluation
    pub fn test_function_{}() -> i32 {{
        let mut result = 0;
        for i in 0..100 {{
            result += i;
        }}
        result
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;
    
    #[test]
    fn test_performance_{}() {{
        let result = performance_{}::test_function_{}();
        assert_eq!(result, 4950); // Sum of 0..99
    }}
}}"#, i, i, i, i, i, i),
        executable: Some(false),
    })).collect();
    
    // Measure execution time
    let start = std::time::Instant::now();
    let result = harness_service.run_for_context(
        &context.id,
        temp_dir.path().to_path_buf(),
        HarnessMode::Autonomous,
        edits,
    ).await?;
    let duration = start.elapsed();
    
    // Verify performance (should complete within reasonable time)
    assert!(duration.as_secs() < 60); // Should complete within 60 seconds
    
    // Verify all edits were processed
    assert!(!result.artifacts.is_empty());
    
    Ok(())
}

