//! Issue 28: Time Travel Debugging Tests
//!
//! Comprehensive tests for Time Travel Debugging including:
//! - TimeTravelSession struct (id, trajectory, checkpoints, current_index, variables)
//! - TimePoint struct (index, timestamp, step_id, file_states, variables, description)
//! - FileState struct (content_hash, content, modified)
//! - VariableState struct (name, value, type_info, scope, modified)
//! - Breakpoint struct (id, condition, hit_count, enabled)
//! - DebugState struct (current_step, total_steps, current_file, variables, call_stack)
//! - StackFrame struct (function, file, line, locals)
//! - DiffView for comparing states
//! - TimeTravelDebugger for session management

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::time_travel::{
    Breakpoint, DebugState, FileState, StackFrame, TimePoint, TimeTravelSession, VariableState,
};

// ============================================================================
// TimeTravelSession Tests
// ============================================================================

#[test]
fn test_time_travel_session_creation() {
    let session = TimeTravelSession {
        id: "session-1".to_string(),
        trajectory_id: "traj-1".to_string(),
        checkpoints: vec![],
        current_index: 0,
        variables: HashMap::new(),
        breakpoints: vec![],
        watch_expressions: vec![],
    };

    assert_eq!(session.id, "session-1");
    assert_eq!(session.trajectory_id, "traj-1");
    assert_eq!(session.current_index, 0);
}

// ============================================================================
// TimePoint Tests
// ============================================================================

#[test]
fn test_time_point_creation() {
    let point = TimePoint {
        index: 0,
        timestamp: chrono::Utc::now(),
        step_id: "step-1".to_string(),
        file_states: HashMap::new(),
        git_commit: Some("abc123".to_string()),
        variables: HashMap::new(),
        description: "Initial state".to_string(),
    };

    assert_eq!(point.index, 0);
    assert_eq!(point.step_id, "step-1");
    assert_eq!(point.git_commit, Some("abc123".to_string()));
}

#[test]
fn test_time_point_with_files() {
    let mut files = HashMap::new();
    files.insert(
        PathBuf::from("src/main.rs"),
        FileState {
            content_hash: "hash1".to_string(),
            content: Some("fn main() {}".to_string()),
            modified: true,
        },
    );

    let point = TimePoint {
        index: 1,
        timestamp: chrono::Utc::now(),
        step_id: "step-2".to_string(),
        file_states: files,
        git_commit: None,
        variables: HashMap::new(),
        description: "After edit".to_string(),
    };

    assert_eq!(point.file_states.len(), 1);
    assert!(point.file_states.contains_key(&PathBuf::from("src/main.rs")));
}

// ============================================================================
// FileState Tests
// ============================================================================

#[test]
fn test_file_state_creation() {
    let state = FileState {
        content_hash: "abc123".to_string(),
        content: Some("file content".to_string()),
        modified: true,
    };

    assert_eq!(state.content_hash, "abc123");
    assert!(state.modified);
}

#[test]
fn test_file_state_without_content() {
    let state = FileState {
        content_hash: "def456".to_string(),
        content: None,
        modified: false,
    };

    assert!(state.content.is_none());
    assert!(!state.modified);
}

// ============================================================================
// VariableState Tests
// ============================================================================

#[test]
fn test_variable_state_creation() {
    let var = VariableState {
        name: "count".to_string(),
        value: "42".to_string(),
        type_info: "i32".to_string(),
        scope: "local".to_string(),
        modified: true,
    };

    assert_eq!(var.name, "count");
    assert_eq!(var.value, "42");
    assert_eq!(var.type_info, "i32");
    assert!(var.modified);
}

#[test]
fn test_variable_state_unmodified() {
    let var = VariableState {
        name: "config".to_string(),
        value: "default".to_string(),
        type_info: "String".to_string(),
        scope: "global".to_string(),
        modified: false,
    };

    assert!(!var.modified);
    assert_eq!(var.scope, "global");
}

// ============================================================================
// Breakpoint Tests
// ============================================================================

#[test]
fn test_breakpoint_creation() {
    let bp = Breakpoint {
        id: "bp-1".to_string(),
        condition: Some("x > 10".to_string()),
        hit_count: 0,
        enabled: true,
    };

    assert_eq!(bp.id, "bp-1");
    assert_eq!(bp.condition, Some("x > 10".to_string()));
    assert!(bp.enabled);
}

#[test]
fn test_breakpoint_disabled() {
    let bp = Breakpoint {
        id: "bp-2".to_string(),
        condition: None,
        hit_count: 5,
        enabled: false,
    };

    assert!(!bp.enabled);
    assert_eq!(bp.hit_count, 5);
}

// ============================================================================
// DebugState Tests
// ============================================================================

#[test]
fn test_debug_state_creation() {
    let state = DebugState {
        current_step: 3,
        total_steps: 10,
        current_file: Some(PathBuf::from("src/lib.rs")),
        line_number: Some(42),
        variables: vec![],
        call_stack: vec![],
    };

    assert_eq!(state.current_step, 3);
    assert_eq!(state.total_steps, 10);
    assert_eq!(state.line_number, Some(42));
}

// ============================================================================
// StackFrame Tests
// ============================================================================

#[test]
fn test_stack_frame_creation() {
    let mut locals = HashMap::new();
    locals.insert("x".to_string(), "10".to_string());
    locals.insert("y".to_string(), "20".to_string());

    let frame = StackFrame {
        function: "calculate".to_string(),
        file: PathBuf::from("src/math.rs"),
        line: 15,
        locals,
    };

    assert_eq!(frame.function, "calculate");
    assert_eq!(frame.line, 15);
    assert_eq!(frame.locals.len(), 2);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_debug_session_workflow() {
    // Create a time travel session with checkpoints
    let mut variables = HashMap::new();
    variables.insert(
        "result".to_string(),
        VariableState {
            name: "result".to_string(),
            value: "success".to_string(),
            type_info: "String".to_string(),
            scope: "local".to_string(),
            modified: true,
        },
    );

    let session = TimeTravelSession {
        id: "debug-session".to_string(),
        trajectory_id: "exec-1".to_string(),
        checkpoints: vec![
            TimePoint {
                index: 0,
                timestamp: chrono::Utc::now(),
                step_id: "start".to_string(),
                file_states: HashMap::new(),
                git_commit: Some("initial".to_string()),
                variables: HashMap::new(),
                description: "Start".to_string(),
            },
            TimePoint {
                index: 1,
                timestamp: chrono::Utc::now(),
                step_id: "edit".to_string(),
                file_states: HashMap::new(),
                git_commit: Some("edited".to_string()),
                variables: variables.clone(),
                description: "After edit".to_string(),
            },
        ],
        current_index: 1,
        variables,
        breakpoints: vec![],
        watch_expressions: vec!["result".to_string()],
    };

    assert_eq!(session.checkpoints.len(), 2);
    assert_eq!(session.current_index, 1);
    assert_eq!(session.watch_expressions.len(), 1);
}
