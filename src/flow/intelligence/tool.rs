//! Tool execution with sandboxing

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

use crate::tools::{ToolContext, ToolMetadata, ToolPermission, ToolPolicy};

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

/// Tool registry for managing available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    tool_whitelist: HashMap<String, Vec<String>>, // context_id -> allowed tool names
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            tool_whitelist: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Set tool whitelist for a context
    pub fn set_whitelist(&mut self, context_id: String, allowed_tools: Vec<String>) {
        self.tool_whitelist.insert(context_id, allowed_tools);
    }

    /// Check if a tool is allowed for a context
    pub fn is_tool_allowed(&self, context_id: &str, tool_name: &str) -> bool {
        if let Some(allowed) = self.tool_whitelist.get(context_id) {
            allowed.contains(&tool_name.to_string())
        } else {
            // If no whitelist set, allow all tools
            true
        }
    }

    /// Get all registered tool names
    pub fn list_tools(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool runtime for executing tools with sandboxing
pub struct ToolRuntime {
    profile: ToolSandboxProfile,
    registry: Arc<ToolRegistry>,
    strict_mode: bool,
}

impl ToolRuntime {
    pub fn new(profile: ToolSandboxProfile) -> Self {
        let registry = Arc::new(ToolRegistry::new());
        Self {
            profile,
            registry,
            strict_mode: false,
        }
    }

    pub fn with_default_profile() -> Self {
        Self::new(ToolSandboxProfile::new())
    }

    /// Create a ToolRuntime with default tools registered
    pub fn with_default_tools(profile: ToolSandboxProfile, repo_path: std::path::PathBuf) -> Self {
        let mut registry = ToolRegistry::new();

        // Register repo tools
        use crate::tools::{
            GitDiffTool, ListTreeTool, PatchFileTool, RepoReadFileTool, SearchFilesTool,
            WriteFileTool,
        };
        registry.register(Arc::new(ListTreeTool::new(repo_path.clone())));
        registry.register(Arc::new(RepoReadFileTool::new(repo_path.clone())));
        registry.register(Arc::new(SearchFilesTool::new(repo_path.clone())));
        registry.register(Arc::new(WriteFileTool::new(repo_path.clone())));
        registry.register(Arc::new(PatchFileTool::new(repo_path.clone())));
        registry.register(Arc::new(GitDiffTool::new(repo_path)));

        // Register command tools
        use crate::tools::{CommandTool, RunTestsTool};
        registry.register(Arc::new(CommandTool::new()));
        registry.register(Arc::new(RunTestsTool::new()));

        Self {
            profile,
            registry: Arc::new(registry),
            strict_mode: false,
        }
    }

    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = registry;
        self
    }

    pub fn with_strict_mode(mut self, strict: bool) -> Self {
        self.strict_mode = strict;
        self
    }

    pub fn registry(&self) -> Arc<ToolRegistry> {
        Arc::clone(&self.registry)
    }

    /// Execute a command as a tool with context
    pub async fn execute_command(
        &self,
        command: &str,
        args: Vec<String>,
        context: &ToolContext,
    ) -> Result<String> {
        // Check tool policy permissions
        if !context.policy.is_allowed(ToolPermission::Shell) {
            anyhow::bail!("Shell execution is not allowed by tool policy");
        }

        // Check trust policy for untrusted tools
        use crate::tools::TrustRegistry;
        let registry = TrustRegistry::new();
        let trust_level = registry.get_trust_level(&context.tool_name);
        if registry.requires_approval(&context.tool_name) && context.requires_approval() {
            anyhow::bail!(
                "Tool '{}' requires approval (trust level: {:?})",
                context.tool_name,
                trust_level
            );
        }

        // Check if command is allowed by sandbox profile
        if !self.profile.is_command_allowed(command) {
            anyhow::bail!("Command '{}' is not allowed by sandbox profile", command);
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

    /// Execute a tool with the given input and context
    pub async fn execute_tool(
        &self,
        tool: &dyn Tool,
        input: ToolInput,
        context: &ToolContext,
    ) -> Result<ToolOutput> {
        // Check tool whitelist if strict mode is enabled
        if self.strict_mode {
            if !self.registry.is_tool_allowed(&context.run_id, &tool.name()) {
                anyhow::bail!(
                    "Tool '{}' is not in the whitelist for context '{}'",
                    tool.name(),
                    context.run_id
                );
            }
        }

        let result = tool.call(input).await?;

        // In strict mode, check for empty outputs
        if self.strict_mode {
            if result.is_null()
                || (result.is_object() && result.as_object().map(|o| o.is_empty()).unwrap_or(false))
            {
                anyhow::bail!(
                    "Tool '{}' returned empty output in strict mode",
                    tool.name()
                );
            }
        }

        Ok(result)
    }
}

/// Tool trait for tool execution
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> String;

    /// Get the tool description
    fn description(&self) -> String;

    /// Get the tool metadata (including schema hash)
    fn metadata(&self) -> ToolMetadata {
        let mut metadata = ToolMetadata::new(self.name(), self.name(), self.description());
        if let Some(schema) = self.input_schema() {
            let hash = ToolMetadata::generate_schema_hash(&schema);
            metadata = metadata.with_schema_hash(hash);
        }
        metadata
    }

    /// Execute the tool with the given input
    async fn call(&self, input: ToolInput) -> Result<ToolOutput>;

    /// Get the tool's input schema (optional)
    fn input_schema(&self) -> Option<serde_json::Value> {
        None
    }
}
