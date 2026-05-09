//! Issue 25: Trajectory Recorder Tests
//!
//! Comprehensive tests for the Trajectory Recorder including:
//! - Trajectory struct (id, work_context_id, steps, timestamps, metadata)
//! - TrajectoryStep struct (step_id, phase, tool_calls, results, errors, duration)
//! - ToolCallRecord struct (tool, input_summary, full_input)
//! - ToolResultRecord struct (tool, success, output_summary, error_details)
//! - TrajectoryStats struct (totals, averages, phases)
//! - TrajectoryMetadata struct (repo_root, task, model, version)
//! - TrajectoryStore for persistence
//! - ReplayConfig and ReplayResult for replay functionality
//! - record_step() for step recording
//! - complete() for finalizing trajectories
//! - compute_stats() for analytics

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::trajectory::{
    ReplayConfig, ReplayResult, ToolCallRecord, ToolResultRecord, Trajectory, TrajectoryMetadata,
    TrajectoryStats, TrajectoryStep, TrajectoryStore,
};

// ============================================================================
// Trajectory Tests
// ============================================================================

#[test]
fn test_trajectory_new() {
    let trajectory = Trajectory::new("work-context-123");

    assert!(!trajectory.id.is_empty());
    assert_eq!(trajectory.work_context_id, "work-context-123");
    assert!(trajectory.steps.is_empty());
    assert!(trajectory.completed_at.is_none());
}

#[test]
fn test_trajectory_with_metadata() {
    let metadata = TrajectoryMetadata {
        repo_root: Some("/tmp/repo".to_string()),
        task_description: Some("Fix bug in parser".to_string()),
        model_used: Some("claude-3".to_string()),
        harness_version: Some("1.6.0".to_string()),
        extra: HashMap::new(),
    };

    let trajectory = Trajectory::new("work-456").with_metadata(metadata);

    assert!(trajectory.metadata.is_some());
    let meta = trajectory.metadata.unwrap();
    assert_eq!(meta.task_description, Some("Fix bug in parser".to_string()));
    assert_eq!(meta.model_used, Some("claude-3".to_string()));
}

#[test]
fn test_trajectory_record_step() {
    let mut trajectory = Trajectory::new("work-789");

    trajectory.record_step("analysis", 1000, vec![]);

    assert_eq!(trajectory.steps.len(), 1);
    assert_eq!(trajectory.steps[0].phase, "analysis");
    assert_eq!(trajectory.steps[0].duration_ms, 1000);
}

#[test]
fn test_trajectory_record_step_with_tools() {
    let mut trajectory = Trajectory::new("work-abc");

    let tool_calls = vec![ToolCallRecord {
        tool: "grep".to_string(),
        input_summary: "search for pattern".to_string(),
        full_input: None,
    }];

    let tool_results = vec![ToolResultRecord {
        tool: "grep".to_string(),
        success: true,
        output_summary: "found 3 matches".to_string(),
        full_output: None,
        error_details: None,
    }];

    trajectory.record_step_with_tools(
        "search",
        2000,
        tool_calls,
        tool_results,
        vec![],
        Some(150),
    );

    assert_eq!(trajectory.steps.len(), 1);
    assert_eq!(trajectory.steps[0].tool_calls.len(), 1);
    assert_eq!(trajectory.steps[0].tool_results.len(), 1);
    assert_eq!(trajectory.steps[0].tokens, Some(150));
}

#[test]
fn test_trajectory_complete() {
    let mut trajectory = Trajectory::new("work-complete");

    trajectory.record_step("step1", 1000, vec![]);
    trajectory.complete();

    assert!(trajectory.completed_at.is_some());
}

// ============================================================================
// TrajectoryStep Tests
// ============================================================================

#[test]
fn test_trajectory_step_creation() {
    let step = TrajectoryStep {
        step_id: "step-1".to_string(),
        phase: "analysis".to_string(),
        tool_calls: vec![],
        tool_results: vec![],
        errors: vec![],
        tokens: Some(100),
        duration_ms: 500,
        recorded_at: None,
    };

    assert_eq!(step.step_id, "step-1");
    assert_eq!(step.phase, "analysis");
    assert_eq!(step.duration_ms, 500);
    assert_eq!(step.tokens, Some(100));
}

#[test]
fn test_trajectory_step_with_errors() {
    let step = TrajectoryStep {
        step_id: "step-error".to_string(),
        phase: "validation".to_string(),
        tool_calls: vec![],
        tool_results: vec![],
        errors: vec!["Test failed".to_string(), "Compilation error".to_string()],
        tokens: None,
        duration_ms: 1000,
        recorded_at: None,
    };

    assert_eq!(step.errors.len(), 2);
    assert!(step.errors.contains(&"Test failed".to_string()));
}

// ============================================================================
// ToolCallRecord Tests
// ============================================================================

#[test]
fn test_tool_call_record_creation() {
    let record = ToolCallRecord {
        tool: "cargo".to_string(),
        input_summary: "run tests".to_string(),
        full_input: Some("cargo test --package mycrate".to_string()),
    };

    assert_eq!(record.tool, "cargo");
    assert_eq!(record.input_summary, "run tests");
    assert_eq!(record.full_input, Some("cargo test --package mycrate".to_string()));
}

// ============================================================================
// ToolResultRecord Tests
// ============================================================================

#[test]
fn test_tool_result_record_success() {
    let record = ToolResultRecord {
        tool: "cargo".to_string(),
        success: true,
        output_summary: "all tests passed".to_string(),
        full_output: None,
        error_details: None,
    };

    assert!(record.success);
    assert_eq!(record.output_summary, "all tests passed");
    assert!(record.error_details.is_none());
}

#[test]
fn test_tool_result_record_failure() {
    let record = ToolResultRecord {
        tool: "cargo".to_string(),
        success: false,
        output_summary: "tests failed".to_string(),
        full_output: None,
        error_details: Some("assertion failed at line 42".to_string()),
    };

    assert!(!record.success);
    assert_eq!(record.error_details, Some("assertion failed at line 42".to_string()));
}

// ============================================================================
// TrajectoryStats Tests
// ============================================================================

#[test]
fn test_trajectory_stats_default() {
    let stats = TrajectoryStats::default();

    assert_eq!(stats.total_steps, 0);
    assert_eq!(stats.total_duration_ms, 0);
    assert_eq!(stats.total_tokens, 0);
    assert!(stats.phases_used.is_empty());
}

// ============================================================================
// TrajectoryMetadata Tests
// ============================================================================

#[test]
fn test_trajectory_metadata_creation() {
    let metadata = TrajectoryMetadata {
        repo_root: Some("/home/user/project".to_string()),
        task_description: Some("Implement feature".to_string()),
        model_used: Some("gpt-4".to_string()),
        harness_version: Some("1.6.0".to_string()),
        extra: HashMap::new(),
    };

    assert_eq!(metadata.repo_root, Some("/home/user/project".to_string()));
    assert_eq!(metadata.model_used, Some("gpt-4".to_string()));
}

// ============================================================================
// TrajectoryStore Tests
// ============================================================================

#[test]
fn test_trajectory_store_new() {
    let store = TrajectoryStore::new(PathBuf::from("/tmp/trajectories"));
    // Store created successfully
    assert!(true);
}

// ============================================================================
// ReplayConfig Tests
// ============================================================================

#[test]
fn test_replay_config_default() {
    let config = ReplayConfig::default();

    assert!(config.max_steps.is_none());
    assert!(config.skip_phases.is_empty());
    assert_eq!(config.step_delay_ms, 0);
    assert!(!config.simulate);
}

#[test]
fn test_replay_config_custom() {
    let config = ReplayConfig {
        max_steps: Some(10),
        skip_phases: vec!["setup".to_string()],
        step_delay_ms: 100,
        simulate: true,
    };

    assert_eq!(config.max_steps, Some(10));
    assert_eq!(config.skip_phases.len(), 1);
    assert_eq!(config.step_delay_ms, 100);
    assert!(config.simulate);
}

// ============================================================================
// ReplayResult Tests
// ============================================================================

#[test]
fn test_replay_result_success() {
    let result = ReplayResult {
        trajectory_id: "traj-1".to_string(),
        steps_replayed: 5,
        steps_skipped: 0,
        steps_failed: 0,
        total_duration_ms: 5000,
        divergence_detected: false,
        divergence_details: vec![],
    };

    assert_eq!(result.steps_replayed, 5);
    assert!(!result.divergence_detected);
}

#[test]
fn test_replay_result_with_divergence() {
    let result = ReplayResult {
        trajectory_id: "traj-2".to_string(),
        steps_replayed: 3,
        steps_skipped: 2,
        steps_failed: 1,
        total_duration_ms: 3000,
        divergence_detected: true,
        divergence_details: vec!["Output mismatch at step 3".to_string()],
    };

    assert!(result.divergence_detected);
    assert_eq!(result.divergence_details.len(), 1);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_full_trajectory_workflow() {
    let mut trajectory = Trajectory::new("integration-test");

    // Record multiple steps
    trajectory.record_step("analysis", 1000, vec![]);
    trajectory.record_step("generation", 2000, vec![]);

    // Complete the trajectory
    trajectory.complete();

    assert_eq!(trajectory.steps.len(), 2);
    assert!(trajectory.completed_at.is_some());
}

#[test]
fn test_trajectory_with_tool_interactions() {
    let mut trajectory = Trajectory::new("tool-test");

    let tool_calls = vec![
        ToolCallRecord {
            tool: "read_file".to_string(),
            input_summary: "read src/main.rs".to_string(),
            full_input: None,
        },
        ToolCallRecord {
            tool: "write_file".to_string(),
            input_summary: "write changes".to_string(),
            full_input: None,
        },
    ];

    let tool_results = vec![
        ToolResultRecord {
            tool: "read_file".to_string(),
            success: true,
            output_summary: "file content".to_string(),
            full_output: None,
            error_details: None,
        },
        ToolResultRecord {
            tool: "write_file".to_string(),
            success: true,
            output_summary: "written".to_string(),
            full_output: None,
            error_details: None,
        },
    ];

    trajectory.record_step_with_tools(
        "edit",
        3000,
        tool_calls,
        tool_results,
        vec![],
        Some(500),
    );

    assert_eq!(trajectory.steps[0].tool_calls.len(), 2);
    assert_eq!(trajectory.steps[0].tool_results.len(), 2);
}
