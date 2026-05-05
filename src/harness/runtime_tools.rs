//! Runtime Tools - Issue #27
//! Dynamic tool loading and execution at runtime

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTool {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tool_type: ToolType,
    pub executable_path: PathBuf,
    pub args_template: Vec<String>,
    pub env_vars: HashMap<String, String>,
    pub timeout_ms: u64,
    pub max_memory_mb: u64,
    pub description: String,
    pub supported_extensions: Vec<String>,
    pub health_check_cmd: Option<String>,
    /// P1-009: Command to get tool version (e.g., "cargo --version")
    pub version_cmd: Option<String>,
}

impl RuntimeTool {
    /// P1-009: Build the command string for this tool
    pub fn command(&self) -> String {
        let mut cmd = self.executable_path.to_string_lossy().to_string();
        for arg in &self.args_template {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        cmd
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolType {
    Linter,
    Formatter,
    Compiler,
    TestRunner,
    StaticAnalyzer,
    SecurityScanner,
    DocumentationGenerator,
    Custom,
}

#[derive(Debug, Clone)]
pub struct RuntimeToolRegistry {
    tools: HashMap<String, RuntimeTool>,
    execution_history: Vec<ToolExecution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub tool_id: String,
    pub input_file: Option<PathBuf>,
    pub args: Vec<String>,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub tool_id: String,
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub issues: Vec<ToolIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolIssue {
    pub severity: IssueSeverity,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
    pub code: Option<String>,
    pub fix_suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl RuntimeToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            execution_history: Vec::new(),
        }
    }

    pub fn with_builtin_tools() -> Self {
        let mut registry = Self::new();
        registry.register_builtin_tools();
        registry
    }

    fn register_builtin_tools(&mut self) {
        // Register common Rust tools
        self.register(RuntimeTool {
            id: "rustfmt".to_string(),
            name: "Rustfmt".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Formatter,
            executable_path: PathBuf::from("rustfmt"),
            args_template: vec![
                "--emit".to_string(),
                "stdout".to_string(),
                "{file}".to_string(),
            ],
            env_vars: HashMap::new(),
            timeout_ms: 30000,
            max_memory_mb: 512,
            description: "Format Rust code".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("rustfmt --version".to_string()),
            version_cmd: Some("rustfmt --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "clippy".to_string(),
            name: "Clippy".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Linter,
            executable_path: PathBuf::from("cargo"),
            args_template: vec![
                "clippy".to_string(),
                "--".to_string(),
                "-D".to_string(),
                "warnings".to_string(),
            ],
            env_vars: HashMap::new(),
            timeout_ms: 120000,
            max_memory_mb: 1024,
            description: "Lint Rust code".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo clippy --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "cargo-check".to_string(),
            name: "Cargo Check".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Compiler,
            executable_path: PathBuf::from("cargo"),
            args_template: vec!["check".to_string(), "--message-format=short".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 120000,
            max_memory_mb: 2048,
            description: "Check Rust code compiles".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "cargo-test".to_string(),
            name: "Cargo Test".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::TestRunner,
            executable_path: PathBuf::from("cargo"),
            args_template: vec!["test".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 300000,
            max_memory_mb: 2048,
            description: "Run Rust tests".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });
    }

    pub fn register(&mut self, tool: RuntimeTool) {
        self.tools.insert(tool.id.clone(), tool);
    }

    pub fn unregister(&mut self, tool_id: &str) -> Option<RuntimeTool> {
        self.tools.remove(tool_id)
    }

    pub fn get(&self, tool_id: &str) -> Option<&RuntimeTool> {
        self.tools.get(tool_id)
    }

    pub fn list(&self) -> Vec<&RuntimeTool> {
        self.tools.values().collect()
    }

    pub fn list_by_type(&self, tool_type: ToolType) -> Vec<&RuntimeTool> {
        self.tools
            .values()
            .filter(|t| t.tool_type == tool_type)
            .collect()
    }

    /// P1-009: Find a tool by its command string
    pub fn find_by_command(&self, command: &str) -> Option<&RuntimeTool> {
        self.tools.values().find(|t| {
            // Check if the command matches the tool's command or executable
            t.command() == command
                || t.executable_path.to_string_lossy() == command
                || command.starts_with(&t.command())
        })
    }

    pub async fn execute(
        &mut self,
        tool_id: &str,
        working_dir: &Path,
        input_file: Option<&Path>,
        extra_args: &[String],
    ) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(tool_id)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", tool_id))?;

        let start_time = chrono::Utc::now();
        let start_instant = std::time::Instant::now();

        // Build command
        let mut cmd = Command::new(&tool.executable_path);
        cmd.current_dir(working_dir)
            .envs(&tool.env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Add args
        for arg in &tool.args_template {
            if arg == "{file}" {
                if let Some(file) = input_file {
                    cmd.arg(file);
                }
            } else {
                cmd.arg(arg);
            }
        }

        // Add extra args
        for arg in extra_args {
            cmd.arg(arg);
        }

        // Execute with timeout
        let output = tokio::time::timeout(
            tokio::time::Duration::from_millis(tool.timeout_ms),
            cmd.output(),
        )
        .await;

        let (success, exit_code, stdout, stderr) = match output {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let success = output.status.success();
                let exit_code = output.status.code().unwrap_or(-1);
                (success, exit_code, stdout, stderr)
            }
            Ok(Err(e)) => (false, -1, String::new(), format!("Execution error: {}", e)),
            Err(_) => (false, -1, String::new(), "Timeout exceeded".to_string()),
        };

        let duration_ms = start_instant.elapsed().as_millis() as u64;
        let end_time = chrono::Utc::now();

        // Parse issues from output
        let issues = self.parse_issues(tool_id, &stdout, &stderr);

        // Record execution
        self.execution_history.push(ToolExecution {
            tool_id: tool_id.to_string(),
            input_file: input_file.map(|p| p.to_path_buf()),
            args: extra_args.to_vec(),
            start_time,
            end_time: Some(end_time),
            exit_code: Some(exit_code),
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            success,
        });

        Ok(ToolResult {
            tool_id: tool_id.to_string(),
            success,
            exit_code,
            stdout,
            stderr,
            duration_ms,
            issues,
        })
    }

    fn parse_issues(&self, tool_id: &str, stdout: &str, stderr: &str) -> Vec<ToolIssue> {
        let mut issues = Vec::new();
        let combined = format!("{}\n{}", stdout, stderr);

        // Tool-specific parsing
        match tool_id {
            "clippy" | "cargo-check" => {
                // Parse cargo error format: "error[<code>]: <message> at <file>:<line>:<col>"
                for line in combined.lines() {
                    if let Some(issue) = self.parse_cargo_error(line) {
                        issues.push(issue);
                    }
                }
            }
            "rustfmt" => {
                // Parse rustfmt errors
                for line in combined.lines() {
                    if line.contains("error") {
                        issues.push(ToolIssue {
                            severity: IssueSeverity::Error,
                            file: None,
                            line: None,
                            column: None,
                            message: line.to_string(),
                            code: None,
                            fix_suggestion: None,
                        });
                    }
                }
            }
            _ => {
                // Generic parsing - look for common patterns
                for line in combined.lines() {
                    if let Some(issue) = self.parse_generic_issue(line) {
                        issues.push(issue);
                    }
                }
            }
        }

        issues
    }

    fn parse_cargo_error(&self, line: &str) -> Option<ToolIssue> {
        // Simple parser for "error[E0000]: message at file:line:col"
        if !line.contains("error") && !line.contains("warning") {
            return None;
        }

        let severity = if line.contains("error:") || line.contains("error[") {
            IssueSeverity::Error
        } else if line.contains("warning:") || line.contains("warning[") {
            IssueSeverity::Warning
        } else {
            IssueSeverity::Info
        };

        // Try to extract file and line
        let mut file = None;
        let mut line_num = None;

        // Look for file:line pattern
        for part in line.split_whitespace() {
            if part.contains(":") && !part.starts_with("http") {
                let parts: Vec<_> = part.split(':').collect();
                if parts.len() >= 2 {
                    if let Ok(num) = parts[1].parse::<u32>() {
                        file = Some(PathBuf::from(parts[0]));
                        line_num = Some(num);
                        break;
                    }
                }
            }
        }

        Some(ToolIssue {
            severity,
            file,
            line: line_num,
            column: None,
            message: line.to_string(),
            code: None,
            fix_suggestion: None,
        })
    }

    fn parse_generic_issue(&self, line: &str) -> Option<ToolIssue> {
        // Generic issue detection
        if line.to_lowercase().contains("error")
            || line.to_lowercase().contains("warning")
            || line.to_lowercase().contains("failed")
        {
            let severity = if line.to_lowercase().contains("error") {
                IssueSeverity::Error
            } else if line.to_lowercase().contains("warning") {
                IssueSeverity::Warning
            } else {
                IssueSeverity::Info
            };

            return Some(ToolIssue {
                severity,
                file: None,
                line: None,
                column: None,
                message: line.to_string(),
                code: None,
                fix_suggestion: None,
            });
        }
        None
    }

    pub async fn health_check(&self, tool_id: &str) -> Result<bool> {
        if let Some(tool) = self.tools.get(tool_id) {
            if let Some(check_cmd) = &tool.health_check_cmd {
                let parts: Vec<_> = check_cmd.split_whitespace().collect();
                if parts.is_empty() {
                    return Ok(false);
                }

                let output = Command::new(parts[0]).args(&parts[1..]).output().await;

                return match output {
                    Ok(output) => Ok(output.status.success()),
                    Err(_) => Ok(false),
                };
            }
            return Ok(true); // No health check configured, assume OK
        }
        Ok(false)
    }

    pub fn get_execution_history(&self) -> &[ToolExecution] {
        &self.execution_history
    }

    pub fn get_tool_stats(&self, tool_id: &str) -> Option<ToolStats> {
        let executions: Vec<_> = self
            .execution_history
            .iter()
            .filter(|e| e.tool_id == tool_id)
            .collect();

        if executions.is_empty() {
            return None;
        }

        let total = executions.len();
        let successful = executions.iter().filter(|e| e.success).count();
        let avg_duration = executions
            .iter()
            .filter_map(|e| {
                e.end_time
                    .map(|end| (end - e.start_time).num_milliseconds() as u64)
            })
            .sum::<u64>()
            / total as u64;

        Some(ToolStats {
            tool_id: tool_id.to_string(),
            total_executions: total as u32,
            successful_executions: successful as u32,
            failed_executions: (total - successful) as u32,
            average_duration_ms: avg_duration,
            success_rate: successful as f64 / total as f64,
        })
    }

    /// P1-009: Get tool version by running the version command
    pub async fn get_tool_version(&self, tool_id: &str) -> Option<String> {
        if let Some(tool) = self.tools.get(tool_id) {
            if let Some(version_cmd) = &tool.version_cmd {
                let parts: Vec<_> = version_cmd.split_whitespace().collect();
                if parts.is_empty() {
                    return None;
                }

                let output = Command::new(parts[0]).args(&parts[1..]).output().await.ok()?;
                if output.status.success() {
                    return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
                }
            }
        }
        None
    }

    /// P1-009: Run health checks for all tools referenced in a ValidationPlan
    pub async fn health_check_plan(&self, plan: &crate::harness::validation::ValidationPlan) -> Vec<(String, bool)> {
        let mut results = Vec::new();

        // Check tools by ID
        for tool_id in &plan.tool_ids {
            let healthy = self.health_check(tool_id).await.unwrap_or(false);
            results.push((tool_id.clone(), healthy));
        }

        // Also check tools referenced by raw commands
        for cmd in &plan.format_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }
        for cmd in &plan.lint_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }
        for cmd in &plan.test_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }

        results
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStats {
    pub tool_id: String,
    pub total_executions: u32,
    pub successful_executions: u32,
    pub failed_executions: u32,
    pub average_duration_ms: u64,
    pub success_rate: f64,
}

pub fn create_tool_registry() -> RuntimeToolRegistry {
    RuntimeToolRegistry::with_builtin_tools()
}

pub fn format_tool_result(result: &ToolResult) -> String {
    let status = if result.success { "✓" } else { "✗" };
    format!(
        r#"{} Tool Result: {}
   Exit Code: {}
   Duration: {}ms
   Issues Found: {}
"#,
        status,
        result.tool_id,
        result.exit_code,
        result.duration_ms,
        result.issues.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get_tool() {
        let mut registry = RuntimeToolRegistry::new();
        let tool = RuntimeTool {
            id: "test-tool".to_string(),
            name: "Test Tool".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Custom,
            executable_path: PathBuf::from("echo"),
            args_template: vec!["hello".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 5000,
            max_memory_mb: 128,
            description: "Test tool".to_string(),
            supported_extensions: vec!["txt".to_string()],
            health_check_cmd: Some("echo --version".to_string()),
            version_cmd: Some("echo --version".to_string()),
        };

        registry.register(tool);
        assert!(registry.get("test-tool").is_some());
    }

    #[test]
    fn test_list_tools_by_type() {
        let registry = RuntimeToolRegistry::with_builtin_tools();
        let formatters = registry.list_by_type(ToolType::Formatter);
        assert!(!formatters.is_empty());
        assert!(formatters.iter().any(|t| t.id == "rustfmt"));
    }

    #[test]
    fn test_parse_cargo_error() {
        let registry = RuntimeToolRegistry::new();
        let line = "error[E0425]: cannot find value `x` in this scope --> src/main.rs:10:5";
        let issue = registry.parse_cargo_error(line);
        assert!(issue.is_some());
        let issue = issue.unwrap();
        assert!(matches!(issue.severity, IssueSeverity::Error));
    }
}
