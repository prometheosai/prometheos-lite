//! Command harness - safe command execution with timeout and output limits
//!
//! This module provides tools for executing commands with safety guards:
//! - run_command: Execute commands with timeout, output limits, and deterministic execution
//! - run_tests: Wrapper for running test commands

use crate::flow::Tool;
use crate::tools::{ToolContext, ToolMetadata};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

/// Command output structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub success: bool,
}

/// Command tool - executes commands with safety guards
pub struct CommandTool {
    timeout_ms: u64,
    max_output_bytes: usize,
    allowed_commands: Vec<String>,
    blocked_commands: Vec<String>,
}

impl CommandTool {
    pub fn new() -> Self {
        Self {
            timeout_ms: 30000, // 30 seconds default
            max_output_bytes: 10 * 1024 * 1024, // 10 MB default
            allowed_commands: vec![
                "cargo".to_string(),
                "rustc".to_string(),
                "python".to_string(),
                "python3".to_string(),
                "node".to_string(),
                "npm".to_string(),
                "yarn".to_string(),
                "go".to_string(),
                "javac".to_string(),
                "java".to_string(),
                "gcc".to_string(),
                "g++".to_string(),
                "make".to_string(),
                "cmake".to_string(),
                "pytest".to_string(),
                "pytest-3".to_string(),
                "cargo".to_string(),
                "dotnet".to_string(),
                "mvn".to_string(),
                "gradle".to_string(),
            ],
            blocked_commands: vec![
                "rm".to_string(),
                "rmdir".to_string(),
                "del".to_string(),
                "format".to_string(),
                "fdisk".to_string(),
                "mkfs".to_string(),
                "dd".to_string(),
                "shutdown".to_string(),
                "reboot".to_string(),
                "poweroff".to_string(),
            ],
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_max_output(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn with_allowed_commands(mut self, commands: Vec<String>) -> Self {
        self.allowed_commands = commands;
        self
    }

    pub fn with_blocked_commands(mut self, commands: Vec<String>) -> Self {
        self.blocked_commands = commands;
        self
    }

    fn is_command_allowed(&self, command: &str) -> bool {
        // Check blocked commands first
        for blocked in &self.blocked_commands {
            if command.starts_with(blocked) {
                return false;
            }
        }

        // If allowed list is empty, allow all (except blocked)
        if self.allowed_commands.is_empty() {
            return true;
        }

        // Check allowed commands
        for allowed in &self.allowed_commands {
            if command.starts_with(allowed) {
                return true;
            }
        }

        false
    }
}

impl Default for CommandTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for CommandTool {
    fn name(&self) -> String {
        "run_command".to_string()
    }

    fn description(&self) -> String {
        "Executes a command with timeout, output limits, and safety guards".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command to execute"
                },
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Command arguments"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory (absolute path)"
                }
            },
            "required": ["command"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let command = input.get("command")
            .and_then(|v| v.as_str())
            .context("Missing command")?;

        let args: Vec<String> = input.get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        let cwd = input.get("cwd")
            .and_then(|v| v.as_str());

        // Check if command is allowed
        if !self.is_command_allowed(command) {
            return Ok(serde_json::json!({
                "error": format!("Command '{}' is not allowed by safety policy", command),
                "success": false,
                "stdout": "",
                "stderr": "",
                "exit_code": -1,
                "duration_ms": 0
            }));
        }

        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(self.timeout_ms);

        let mut cmd = Command::new(command);
        cmd.args(&args);
        if let Some(working_dir) = cwd {
            cmd.current_dir(working_dir);
        }

        // Execute with timeout
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .context("Command execution timed out")??;

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        let success = output.status.success();

        // Check output size limits
        if stdout.len() > self.max_output_bytes {
            return Ok(serde_json::json!({
                "error": format!("Stdout exceeds maximum size limit ({} bytes)", self.max_output_bytes),
                "success": false,
                "stdout": "",
                "stderr": stderr,
                "exit_code": exit_code,
                "duration_ms": duration_ms
            }));
        }

        if stderr.len() > self.max_output_bytes {
            return Ok(serde_json::json!({
                "error": format!("Stderr exceeds maximum size limit ({} bytes)", self.max_output_bytes),
                "success": false,
                "stdout": stdout,
                "stderr": "",
                "exit_code": exit_code,
                "duration_ms": duration_ms
            }));
        }

        Ok(serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
            "success": success
        }))
    }
}

/// Run tests tool - wrapper for running test commands
pub struct RunTestsTool {
    command_tool: CommandTool,
}

impl RunTestsTool {
    pub fn new() -> Self {
        Self {
            command_tool: CommandTool::new(),
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.command_tool = self.command_tool.with_timeout(timeout_ms);
        self
    }

    pub fn with_max_output(mut self, max_output_bytes: usize) -> Self {
        self.command_tool = self.command_tool.with_max_output(max_output_bytes);
        self
    }
}

impl Default for RunTestsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for RunTestsTool {
    fn name(&self) -> String {
        "run_tests".to_string()
    }

    fn description(&self) -> String {
        "Runs tests for the project using the appropriate test command".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "cwd": {
                    "type": "string",
                    "description": "Working directory (absolute path)"
                },
                "test_command": {
                    "type": "string",
                    "description": "Test command to use (auto-detected if not provided)",
                    "enum": ["cargo test", "pytest", "pytest-3", "npm test", "yarn test", "go test", "mvn test", "gradle test", "dotnet test"]
                },
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Additional arguments for the test command"
                }
            }
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let cwd = input.get("cwd")
            .and_then(|v| v.as_str());

        let test_command = input.get("test_command")
            .and_then(|v| v.as_str());

        let additional_args: Vec<String> = input.get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        // Auto-detect test command if not provided
        let (command, args) = if let Some(tc) = test_command {
            let parts: Vec<&str> = tc.split_whitespace().collect();
            let cmd = parts[0];
            let cmd_args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).chain(additional_args).collect();
            (cmd.to_string(), cmd_args)
        } else {
            self.detect_test_command(cwd, additional_args)?
        };

        // Execute the test command
        let command_input = serde_json::json!({
            "command": command,
            "args": args,
            "cwd": cwd
        });

        let result = self.command_tool.call(command_input).await?;

        // Parse test results
        let test_results = self.parse_test_results(&result);

        Ok(serde_json::json!({
            "test_results": test_results,
            "command": command,
            "args": args,
            "stdout": result["stdout"],
            "stderr": result["stderr"],
            "exit_code": result["exit_code"],
            "duration_ms": result["duration_ms"],
            "success": result["success"]
        }))
    }
}

impl RunTestsTool {
    fn detect_test_command(&self, cwd: Option<&str>, additional_args: Vec<String>) -> Result<(String, Vec<String>)> {
        // Check for common project files to determine test command
        if let Some(working_dir) = cwd {
            let path = std::path::Path::new(working_dir);

            // Check for Cargo.toml (Rust)
            if path.join("Cargo.toml").exists() {
                return Ok(("cargo".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
            }

            // Check for package.json (Node.js)
            if path.join("package.json").exists() {
                return Ok(("npm".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
            }

            // Check for go.mod (Go)
            if path.join("go.mod").exists() {
                return Ok(("go".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
            }

            // Check for pom.xml (Maven)
            if path.join("pom.xml").exists() {
                return Ok(("mvn".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
            }

            // Check for build.gradle (Gradle)
            if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() {
                return Ok(("gradle".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
            }

            // Check for .csproj file (dotnet)
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".csproj") {
                            return Ok(("dotnet".to_string(), vec!["test".to_string()].into_iter().chain(additional_args).collect()));
                        }
                    }
                }
            }
        }

        // Default to pytest for Python projects
        Ok(("pytest".to_string(), additional_args))
    }

    fn parse_test_results(&self, result: &serde_json::Value) -> serde_json::Value {
        let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
        let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
        let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

        // Parse common test output formats
        let total_tests = self.extract_test_count(stdout, stderr);
        let passed_tests = self.extract_passed_count(stdout, stderr);
        let failed_tests = self.extract_failed_count(stdout, stderr);

        serde_json::json!({
            "total": total_tests,
            "passed": passed_tests,
            "failed": failed_tests,
            "exit_code": exit_code,
            "success": success
        })
    }

    fn extract_test_count(&self, stdout: &str, stderr: &str) -> u64 {
        let combined = format!("{} {}", stdout, stderr);
        
        // Cargo test output: "test result: ok. X passed"
        if let Some(captures) = regex::Regex::new(r"test result: ok\. (\d+) passed").unwrap().captures(&combined) {
            if let Some(count) = captures.get(1) {
                return count.as_str().parse().unwrap_or(0);
            }
        }

        // Cargo test output: "test result: FAILED. X passed; Y failed"
        if let Some(captures) = regex::Regex::new(r"test result: FAILED\. (\d+) passed; (\d+) failed").unwrap().captures(&combined) {
            let passed = captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            let failed = captures.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            return passed + failed;
        }

        // pytest output: "X passed, Y failed"
        if let Some(captures) = regex::Regex::new(r"(\d+) passed").unwrap().captures(&combined) {
            if let Some(count) = captures.get(1) {
                let passed = count.as_str().parse().unwrap_or(0);
                if let Some(failed_captures) = regex::Regex::new(r"(\d+) failed").unwrap().captures(&combined) {
                    let failed = failed_captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                    return passed + failed;
                }
                return passed;
            }
        }

        0
    }

    fn extract_passed_count(&self, stdout: &str, stderr: &str) -> u64 {
        let combined = format!("{} {}", stdout, stderr);

        // Cargo test output
        if let Some(captures) = regex::Regex::new(r"(\d+) passed").unwrap().captures(&combined) {
            if let Some(count) = captures.get(1) {
                return count.as_str().parse().unwrap_or(0);
            }
        }

        0
    }

    fn extract_failed_count(&self, stdout: &str, stderr: &str) -> u64 {
        let combined = format!("{} {}", stdout, stderr);

        // Cargo test output
        if let Some(captures) = regex::Regex::new(r"(\d+) failed").unwrap().captures(&combined) {
            if let Some(count) = captures.get(1) {
                return count.as_str().parse().unwrap_or(0);
            }
        }

        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_command_tool_echo() {
        let tool = CommandTool::new();
        let result = tool.call(serde_json::json!({
            "command": "echo",
            "args": ["hello", "world"]
        })).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["stdout"].as_str().unwrap(), "hello world\n");
        assert_eq!(result["exit_code"].as_i64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_command_tool_blocked() {
        let tool = CommandTool::new();
        let result = tool.call(serde_json::json!({
            "command": "rm",
            "args": ["-rf", "/"]
        })).await.unwrap();

        assert!(!result["success"].as_bool().unwrap());
        assert!(result["error"].as_str().unwrap().contains("not allowed"));
    }

    #[tokio::test]
    async fn test_command_tool_timeout() {
        let tool = CommandTool::new().with_timeout(100); // 100ms timeout
        let result = tool.call(serde_json::json!({
            "command": "sleep",
            "args": ["10"]
        })).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn test_command_tool_is_allowed() {
        let tool = CommandTool::new();
        assert!(tool.is_command_allowed("cargo"));
        assert!(tool.is_command_allowed("cargo test"));
        assert!(!tool.is_command_allowed("rm"));
        assert!(!tool.is_command_allowed("rm -rf"));
    }

    #[test]
    fn test_detect_test_command_rust() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        std::fs::write(repo_path.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let tool = RunTestsTool::new();
        let (command, args) = tool.detect_test_command(Some(repo_path.to_str().unwrap()), vec![]).unwrap();

        assert_eq!(command, "cargo");
        assert_eq!(args, vec!["test"]);
    }

    #[test]
    fn test_detect_test_command_node() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        std::fs::write(repo_path.join("package.json"), "{\"name\": \"test\"}").unwrap();

        let tool = RunTestsTool::new();
        let (command, args) = tool.detect_test_command(Some(repo_path.to_str().unwrap()), vec![]).unwrap();

        assert_eq!(command, "npm");
        assert_eq!(args, vec!["test"]);
    }

    #[test]
    fn test_parse_test_results() {
        let tool = RunTestsTool::new();
        let result = serde_json::json!({
            "stdout": "test result: ok. 5 passed",
            "stderr": "",
            "exit_code": 0,
            "success": true
        });

        let test_results = tool.parse_test_results(&result);
        assert_eq!(test_results["total"].as_u64().unwrap(), 5);
        assert_eq!(test_results["passed"].as_u64().unwrap(), 5);
        assert_eq!(test_results["failed"].as_u64().unwrap(), 0);
    }
}
