//! Tool execution with sandboxing

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

use crate::tools::{ToolPermission, ToolPolicy};

/// Tool input and output types
pub type ToolInput = serde_json::Value;
pub type ToolOutput = serde_json::Value;

/// Tool Sandbox Profile - defines permissions and limits for tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSandboxProfile {
    pub allowed_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allow_network: bool,
    pub allowed_network_hosts: Vec<String>,
    pub blocked_network_hosts: Vec<String>,
    pub allow_file_read: bool,
    pub allowed_file_paths: Vec<String>,
    pub blocked_file_paths: Vec<String>,
    pub allow_file_write: bool,
    pub max_memory_mb: usize,
    pub max_cpu_percent: f64,
    /// Declarative tool policy (WHAT is allowed)
    #[serde(skip)]
    pub tool_policy: ToolPolicy,
}

impl ToolSandboxProfile {
    pub fn new() -> Self {
        let tool_policy = ToolPolicy::conservative();

        Self {
            allowed_commands: vec![
                "echo".to_string(),
                "cat".to_string(),
                "ls".to_string(),
                "pwd".to_string(),
                "grep".to_string(),
                "find".to_string(),
                "head".to_string(),
                "tail".to_string(),
                "wc".to_string(),
                "sort".to_string(),
                "uniq".to_string(),
                "cut".to_string(),
                "sed".to_string(),
                "awk".to_string(),
                "tr".to_string(),
                "diff".to_string(),
                "file".to_string(),
                "stat".to_string(),
                "date".to_string(),
                "whoami".to_string(),
                "hostname".to_string(),
                "uname".to_string(),
                "env".to_string(),
                "printenv".to_string(),
                "cmd".to_string(),        // Windows
                "powershell".to_string(), // Windows
            ],
            blocked_commands: vec![
                "rm".to_string(),
                "rmdir".to_string(),
                "mv".to_string(),
                "cp".to_string(),
                "dd".to_string(),
                "mkfs".to_string(),
                "fdisk".to_string(),
                "format".to_string(), // Windows
                "del".to_string(),    // Windows
                "rmdir".to_string(),  // Windows
            ],
            timeout_ms: 30000,                  // 30 seconds
            max_output_bytes: 10 * 1024 * 1024, // 10 MB
            allow_network: tool_policy.is_allowed(ToolPermission::Network),
            allowed_network_hosts: vec![],
            blocked_network_hosts: vec![],
            allow_file_read: tool_policy.is_allowed(ToolPermission::FileRead),
            allowed_file_paths: vec![],
            blocked_file_paths: vec!["/etc".to_string(), "/sys".to_string(), "/proc".to_string()],
            allow_file_write: tool_policy.is_allowed(ToolPermission::FileWrite),
            max_memory_mb: 512,
            max_cpu_percent: 80.0,
            tool_policy,
        }
    }

    pub fn custom(
        allowed_commands: Vec<String>,
        blocked_commands: Vec<String>,
        timeout_ms: u64,
        max_output_bytes: usize,
    ) -> Self {
        let tool_policy = ToolPolicy::conservative();

        Self {
            allowed_commands,
            blocked_commands,
            timeout_ms,
            max_output_bytes,
            allow_network: tool_policy.is_allowed(ToolPermission::Network),
            allowed_network_hosts: vec![],
            blocked_network_hosts: vec![],
            allow_file_read: tool_policy.is_allowed(ToolPermission::FileRead),
            allowed_file_paths: vec![],
            blocked_file_paths: vec!["/etc".to_string(), "/sys".to_string(), "/proc".to_string()],
            allow_file_write: tool_policy.is_allowed(ToolPermission::FileWrite),
            max_memory_mb: 512,
            max_cpu_percent: 80.0,
            tool_policy,
        }
    }

    /// Create a profile with a custom tool policy
    pub fn with_tool_policy(tool_policy: ToolPolicy) -> Self {
        Self {
            allow_network: tool_policy.is_allowed(ToolPermission::Network),
            allow_file_read: tool_policy.is_allowed(ToolPermission::FileRead),
            allow_file_write: tool_policy.is_allowed(ToolPermission::FileWrite),
            tool_policy,
            ..Self::new()
        }
    }

    pub fn is_command_allowed(&self, command: &str) -> bool {
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

    pub fn is_network_allowed(&self, host: &str) -> bool {
        if !self.allow_network {
            return false;
        }

        // Check blocked hosts
        for blocked in &self.blocked_network_hosts {
            if host.contains(blocked) {
                return false;
            }
        }

        // If allowed list is empty, allow all (except blocked)
        if self.allowed_network_hosts.is_empty() {
            return true;
        }

        // Check allowed hosts
        for allowed in &self.allowed_network_hosts {
            if host.contains(allowed) {
                return true;
            }
        }

        false
    }

    pub fn is_file_read_allowed(&self, path: &str) -> bool {
        if !self.allow_file_read {
            return false;
        }

        // Check blocked paths
        for blocked in &self.blocked_file_paths {
            if path.starts_with(blocked) {
                return false;
            }
        }

        // If allowed list is empty, allow all (except blocked)
        if self.allowed_file_paths.is_empty() {
            return true;
        }

        // Check allowed paths
        for allowed in &self.allowed_file_paths {
            if path.starts_with(allowed) {
                return true;
            }
        }

        false
    }

    pub fn is_file_write_allowed(&self, path: &str) -> bool {
        if !self.allow_file_write {
            return false;
        }

        // Check blocked paths
        for blocked in &self.blocked_file_paths {
            if path.starts_with(blocked) {
                return false;
            }
        }

        // If allowed list is empty, allow all (except blocked)
        if self.allowed_file_paths.is_empty() {
            return true;
        }

        // Check allowed paths
        for allowed in &self.allowed_file_paths {
            if path.starts_with(allowed) {
                return true;
            }
        }

        false
    }
}

/// Tool runtime for executing tools with sandboxing
pub struct ToolRuntime {
    profile: ToolSandboxProfile,
}

impl ToolRuntime {
    pub fn new(profile: ToolSandboxProfile) -> Self {
        Self { profile }
    }

    pub fn with_default_profile() -> Self {
        Self::new(ToolSandboxProfile::new())
    }

    /// Execute a command as a tool
    pub async fn execute_command(&self, command: &str, args: Vec<String>) -> Result<String> {
        // Check if command is allowed by sandbox profile
        if !self.profile.is_command_allowed(command) {
            anyhow::bail!("Command '{}' is not allowed by sandbox profile", command);
        }

        // Check if command requires Shell permission
        if !self.profile.tool_policy.is_allowed(ToolPermission::Shell) {
            anyhow::bail!("Shell execution is not allowed by tool policy");
        }

        let timeout = Duration::from_millis(self.profile.timeout_ms);

        let output = tokio::time::timeout(timeout, Command::new(command).args(&args).output())
            .await
            .context("Command execution timed out")??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Command failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Check output size limit
        if stdout.len() > self.profile.max_output_bytes {
            anyhow::bail!("Output exceeds maximum size limit");
        }

        Ok(stdout.to_string())
    }

    /// Execute a tool with the given input
    pub async fn execute_tool(&self, tool: &dyn Tool, input: ToolInput) -> Result<ToolOutput> {
        tool.call(input).await
    }
}

/// Tool trait for tool execution
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> String;

    /// Get the tool description
    fn description(&self) -> String;

    /// Execute the tool with the given input
    async fn call(&self, input: ToolInput) -> Result<ToolOutput>;

    /// Get the tool's input schema (optional)
    fn input_schema(&self) -> Option<serde_json::Value> {
        None
    }
}
