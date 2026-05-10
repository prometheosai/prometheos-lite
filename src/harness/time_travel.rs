//! Time Travel Debugging - Issue #34
//! Debug state reconstruction from trajectory and checkpoints

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeTravelSession {
    pub id: String,
    pub trajectory_id: String,
    pub checkpoints: Vec<TimePoint>,
    pub current_index: usize,
    pub variables: HashMap<String, VariableState>,
    pub breakpoints: Vec<Breakpoint>,
    pub watch_expressions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimePoint {
    pub index: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub step_id: String,
    pub file_states: HashMap<PathBuf, FileState>,
    pub git_commit: Option<String>,
    pub variables: HashMap<String, VariableState>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileState {
    pub content_hash: String,
    pub content: Option<String>,
    pub modified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariableState {
    pub name: String,
    pub value: String,
    pub type_info: String,
    pub scope: String,
    pub modified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Breakpoint {
    pub id: String,
    pub condition: Option<String>,
    pub hit_count: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct TimeTravelDebugger {
    sessions: HashMap<String, TimeTravelSession>,
    current_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DebugState {
    pub current_step: usize,
    pub total_steps: usize,
    pub current_file: Option<PathBuf>,
    pub line_number: Option<u32>,
    pub variables: Vec<VariableState>,
    pub call_stack: Vec<StackFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackFrame {
    pub function: String,
    pub file: PathBuf,
    pub line: u32,
    pub locals: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffView {
    pub from_index: usize,
    pub to_index: usize,
    pub file_changes: Vec<FileChangeDiff>,
    pub variable_changes: Vec<VariableChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeDiff {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
    pub diff_lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffLine {
    pub line_number: u32,
    pub content: String,
    pub change_type: ChangeType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariableChange {
    pub name: String,
    pub before: String,
    pub after: String,
    pub scope: String,
}

impl TimeTravelDebugger {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            current_session: None,
        }
    }

    pub fn create_session(&mut self, trajectory_id: String, checkpoints: Vec<TimePoint>) -> String {
        let session_id = format!("session-{}", self.sessions.len() + 1);
        let session = TimeTravelSession {
            id: session_id.clone(),
            trajectory_id,
            checkpoints,
            current_index: 0,
            variables: HashMap::new(),
            breakpoints: Vec::new(),
            watch_expressions: Vec::new(),
        };

        self.sessions.insert(session_id.clone(), session);
        self.current_session = Some(session_id.clone());
        session_id
    }

    pub fn step_forward(&mut self, session_id: &str) -> Result<TimePoint> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if session.current_index + 1 >= session.checkpoints.len() {
            bail!("Already at the end of the timeline");
        }

        session.current_index += 1;
        let checkpoint = session.checkpoints[session.current_index].clone();

        // Update variables
        for (name, var) in &checkpoint.variables {
            session.variables.insert(name.clone(), var.clone());
        }

        Ok(checkpoint)
    }

    pub fn step_backward(&mut self, session_id: &str) -> Result<TimePoint> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if session.current_index == 0 {
            bail!("Already at the beginning of the timeline");
        }

        session.current_index -= 1;
        let checkpoint = session.checkpoints[session.current_index].clone();

        // Restore variables from checkpoint
        for (name, var) in &checkpoint.variables {
            session.variables.insert(name.clone(), var.clone());
        }

        Ok(checkpoint)
    }

    pub fn jump_to(&mut self, session_id: &str, index: usize) -> Result<TimePoint> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if index >= session.checkpoints.len() {
            bail!("Index out of bounds");
        }

        session.current_index = index;
        let checkpoint = session.checkpoints[index].clone();

        // Reconstruct variables from all previous checkpoints up to this point
        session.variables.clear();
        for i in 0..=index {
            for (name, var) in &session.checkpoints[i].variables {
                session.variables.insert(name.clone(), var.clone());
            }
        }

        Ok(checkpoint)
    }

    pub fn get_current_state(&self, session_id: &str) -> Result<DebugState> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let checkpoint = &session.checkpoints[session.current_index];

        // Find current file and line from description
        let (current_file, line_number) = self.parse_location(&checkpoint.description);

        let variables: Vec<_> = session.variables.values().cloned().collect();

        Ok(DebugState {
            current_step: session.current_index,
            total_steps: session.checkpoints.len(),
            current_file,
            line_number,
            variables,
            call_stack: vec![], // Would need more context
        })
    }

    pub fn add_breakpoint(&mut self, session_id: &str, condition: Option<String>) -> String {
        if let Some(session) = self.sessions.get_mut(session_id) {
            let bp_id = format!("bp-{}", session.breakpoints.len() + 1);
            session.breakpoints.push(Breakpoint {
                id: bp_id.clone(),
                condition,
                hit_count: 0,
                enabled: true,
            });
            bp_id
        } else {
            String::new()
        }
    }

    pub fn add_watch(&mut self, session_id: &str, expression: String) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.watch_expressions.push(expression);
        }
    }

    pub fn continue_until_breakpoint(&mut self, session_id: &str) -> Result<Option<TimePoint>> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let enabled_breakpoints: Vec<_> =
            session.breakpoints.iter().filter(|bp| bp.enabled).collect();

        if enabled_breakpoints.is_empty() {
            return Ok(None);
        }

        let start_index = session.current_index;
        drop(session); // Release borrow

        for i in (start_index + 1)..self.sessions.get(session_id).unwrap().checkpoints.len() {
            // Check if any breakpoint condition matches
            for bp in &enabled_breakpoints {
                // Simple condition checking - would be more sophisticated in real impl
                if self.check_breakpoint_condition(session_id, i, &bp.condition)? {
                    return self.jump_to(session_id, i).map(Some);
                }
            }
        }

        Ok(None)
    }

    fn check_breakpoint_condition(
        &self,
        session_id: &str,
        index: usize,
        condition: &Option<String>,
    ) -> Result<bool> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if let Some(cond) = condition {
            // Simple condition evaluation
            if cond.contains("step_id=") {
                let step_id = cond.trim_start_matches("step_id=");
                return Ok(session.checkpoints[index].step_id == step_id);
            }
            if cond.contains("variable=") {
                // Check if variable exists and matches
                let var_name = cond.trim_start_matches("variable=");
                return Ok(session.checkpoints[index].variables.contains_key(var_name));
            }
        } else {
            // No condition = always break
            return Ok(true);
        }

        Ok(false)
    }

    pub fn compare_points(
        &self,
        session_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> Result<DiffView> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if from_index >= session.checkpoints.len() || to_index >= session.checkpoints.len() {
            bail!("Index out of bounds");
        }

        let from = &session.checkpoints[from_index];
        let to = &session.checkpoints[to_index];

        // Compare file states
        let mut file_changes = Vec::new();
        let all_files: std::collections::HashSet<_> = from
            .file_states
            .keys()
            .chain(to.file_states.keys())
            .collect();

        for file in all_files {
            let before = from.file_states.get(file).cloned();
            let after = to.file_states.get(file).cloned();

            if before != after {
                let diff_lines = self.compute_diff(
                    before.as_ref().and_then(|f| f.content.as_ref()),
                    after.as_ref().and_then(|f| f.content.as_ref()),
                );

                file_changes.push(FileChangeDiff {
                    path: file.clone(),
                    before: before.and_then(|f| f.content),
                    after: after.and_then(|f| f.content),
                    diff_lines,
                });
            }
        }

        // Compare variables
        let mut variable_changes = Vec::new();
        let all_vars: std::collections::HashSet<_> =
            from.variables.keys().chain(to.variables.keys()).collect();

        for var in all_vars {
            let before = from
                .variables
                .get(var)
                .map(|v| v.value.clone())
                .unwrap_or_default();
            let after = to
                .variables
                .get(var)
                .map(|v| v.value.clone())
                .unwrap_or_default();

            if before != after {
                let scope = to
                    .variables
                    .get(var)
                    .map(|v| v.scope.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                variable_changes.push(VariableChange {
                    name: var.clone(),
                    before,
                    after,
                    scope,
                });
            }
        }

        Ok(DiffView {
            from_index,
            to_index,
            file_changes,
            variable_changes,
        })
    }

    fn compute_diff(&self, before: Option<&String>, after: Option<&String>) -> Vec<DiffLine> {
        let mut diff_lines = Vec::new();

        let before_lines: Vec<_> = before.map(|s| s.lines().collect()).unwrap_or_default();
        let after_lines: Vec<_> = after.map(|s| s.lines().collect()).unwrap_or_default();

        let max_lines = before_lines.len().max(after_lines.len());

        for i in 0..max_lines {
            let before_line = before_lines.get(i);
            let after_line = after_lines.get(i);

            match (before_line, after_line) {
                (Some(b), Some(a)) if b != a => {
                    diff_lines.push(DiffLine {
                        line_number: (i + 1) as u32,
                        content: format!("-{}\n+{}", b, a),
                        change_type: ChangeType::Modified,
                    });
                }
                (None, Some(a)) => {
                    diff_lines.push(DiffLine {
                        line_number: (i + 1) as u32,
                        content: format!("+{}", a),
                        change_type: ChangeType::Added,
                    });
                }
                (Some(b), None) => {
                    diff_lines.push(DiffLine {
                        line_number: (i + 1) as u32,
                        content: format!("-{}", b),
                        change_type: ChangeType::Removed,
                    });
                }
                _ => {
                    diff_lines.push(DiffLine {
                        line_number: (i + 1) as u32,
                        content: before_line.map(|s| s.to_string()).unwrap_or_default(),
                        change_type: ChangeType::Context,
                    });
                }
            }
        }

        diff_lines
    }

    fn parse_location(&self, description: &str) -> (Option<PathBuf>, Option<u32>) {
        // Parse "file:line" pattern from description
        if let Some(pos) = description.find(':') {
            let file_part = &description[..pos];
            let line_part = &description[pos + 1..];

            if let Ok(line) = line_part
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse::<u32>()
            {
                return (Some(PathBuf::from(file_part)), Some(line));
            }
        }
        (None, None)
    }

    pub fn export_session(&self, session_id: &str) -> Result<TimeTravelSession> {
        self.sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session not found"))
    }

    pub fn import_session(&mut self, session: TimeTravelSession) {
        self.sessions.insert(session.id.clone(), session);
    }

    pub fn get_session_stats(&self, session_id: &str) -> Option<SessionStats> {
        self.sessions.get(session_id).map(|session| SessionStats {
            total_checkpoints: session.checkpoints.len(),
            current_position: session.current_index,
            breakpoints_set: session.breakpoints.len(),
            watch_expressions: session.watch_expressions.len(),
            variables_tracked: session.variables.len(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub total_checkpoints: usize,
    pub current_position: usize,
    pub breakpoints_set: usize,
    pub watch_expressions: usize,
    pub variables_tracked: usize,
}

pub fn create_time_travel_debugger() -> TimeTravelDebugger {
    TimeTravelDebugger::new()
}

pub fn format_debug_state(state: &DebugState) -> String {
    format!(
        r#"Debug State
===========
Step: {} / {}
Location: {}:{}

Variables ({}):
{}

Call Stack:
{}
"#,
        state.current_step + 1,
        state.total_steps,
        state
            .current_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "?".to_string()),
        state
            .line_number
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_string()),
        state.variables.len(),
        state
            .variables
            .iter()
            .map(|v| format!("  {}: {} ({})", v.name, v.value, v.type_info))
            .collect::<Vec<_>>()
            .join("\n"),
        state
            .call_stack
            .iter()
            .map(|f| format!("  {} at {}:{}", f.function, f.file.display(), f.line))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub fn format_diff_view(diff: &DiffView) -> String {
    format!(
        r#"Changes from checkpoint {} to {}
File changes: {}
Variable changes: {}
"#,
        diff.from_index,
        diff.to_index,
        diff.file_changes.len(),
        diff.variable_changes.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_timepoint(index: usize, step_id: &str) -> TimePoint {
        TimePoint {
            index,
            timestamp: chrono::Utc::now(),
            step_id: step_id.to_string(),
            file_states: HashMap::new(),
            git_commit: None,
            variables: HashMap::new(),
            description: format!("step {} at src/lib.rs:10", step_id),
        }
    }

    #[test]
    fn test_create_and_navigate_session() {
        let mut debugger = TimeTravelDebugger::new();

        let checkpoints = vec![
            create_test_timepoint(0, "start"),
            create_test_timepoint(1, "middle"),
            create_test_timepoint(2, "end"),
        ];

        let session_id = debugger.create_session("traj-1".to_string(), checkpoints);

        // Step forward
        let point = debugger.step_forward(&session_id).unwrap();
        assert_eq!(point.step_id, "middle");

        // Step backward
        let point = debugger.step_backward(&session_id).unwrap();
        assert_eq!(point.step_id, "start");
    }

    #[test]
    fn test_jump_to() {
        let mut debugger = TimeTravelDebugger::new();

        let checkpoints = vec![
            create_test_timepoint(0, "start"),
            create_test_timepoint(1, "middle"),
            create_test_timepoint(2, "end"),
        ];

        let session_id = debugger.create_session("traj-1".to_string(), checkpoints);

        let point = debugger.jump_to(&session_id, 2).unwrap();
        assert_eq!(point.step_id, "end");
    }

    #[test]
    fn test_add_breakpoint() {
        let mut debugger = TimeTravelDebugger::new();

        let checkpoints = vec![create_test_timepoint(0, "start")];
        let session_id = debugger.create_session("traj-1".to_string(), checkpoints);

        let bp_id = debugger.add_breakpoint(&session_id, Some("step_id=middle".to_string()));
        assert!(!bp_id.is_empty());
    }
}
