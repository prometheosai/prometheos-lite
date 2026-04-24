//! Intelligence - Model Router, Tool Runtime, LLM Utilities

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

/// Streaming callback type
pub type StreamCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// LLM Provider trait for provider abstraction
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a completion from the given prompt
    async fn generate(&self, prompt: &str) -> Result<String>;

    /// Generate a completion with streaming support
    async fn generate_stream(&self, prompt: &str, callback: StreamCallback) -> Result<String>;

    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the model name
    fn model(&self) -> &str;
}

/// Model router for selecting and routing to different LLM providers
pub struct ModelRouter {
    providers: Vec<Box<dyn LlmProvider>>,
    fallback_chain: Vec<usize>,
    current_provider: usize,
}

impl ModelRouter {
    /// Create a new ModelRouter with a list of providers
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self {
            providers,
            fallback_chain: Vec::new(),
            current_provider: 0,
        }
    }

    /// Set the fallback chain (indices of providers to try in order)
    pub fn with_fallback_chain(mut self, chain: Vec<usize>) -> Self {
        self.fallback_chain = chain;
        self
    }

    /// Generate a completion using the current provider with fallback
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;

        for provider_idx in providers_to_try {
            if let Some(provider) = self.providers.get(provider_idx) {
                match provider.generate(prompt).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Generate a completion with streaming support
    pub async fn generate_stream(&self, prompt: &str, callback: StreamCallback) -> Result<String> {
        let providers_to_try = if self.fallback_chain.is_empty() {
            (0..self.providers.len()).collect()
        } else {
            self.fallback_chain.clone()
        };

        let mut last_error = None;

        for provider_idx in providers_to_try {
            if let Some(provider) = self.providers.get(provider_idx) {
                match provider.generate_stream(prompt, callback.clone()).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        last_error = Some(e);
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers available")))
    }

    /// Get the current provider
    pub fn current_provider(&self) -> Option<&dyn LlmProvider> {
        self.providers.get(self.current_provider).map(|p| p.as_ref())
    }

    /// Set the current provider index
    pub fn set_current_provider(&mut self, idx: usize) -> Result<()> {
        if idx >= self.providers.len() {
            anyhow::bail!("Provider index out of bounds: {}", idx);
        }
        self.current_provider = idx;
        Ok(())
    }
}

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
}

impl ToolSandboxProfile {
    pub fn new() -> Self {
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
                "cmd".to_string(), // Windows
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
                "del".to_string(), // Windows
                "rmdir".to_string(), // Windows
            ],
            timeout_ms: 30000, // 30 seconds
            max_output_bytes: 10 * 1024 * 1024, // 10 MB
            allow_network: false,
            allowed_network_hosts: vec![],
            blocked_network_hosts: vec![],
            allow_file_read: true,
            allowed_file_paths: vec![],
            blocked_file_paths: vec!["/etc".to_string(), "/sys".to_string(), "/proc".to_string()],
            allow_file_write: false,
            max_memory_mb: 512,
            max_cpu_percent: 80.0,
        }
    }

    pub fn custom(
        allowed_commands: Vec<String>,
        blocked_commands: Vec<String>,
        timeout_ms: u64,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            allowed_commands,
            blocked_commands,
            timeout_ms,
            max_output_bytes,
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
    pub async fn execute_command(
        &self,
        command: &str,
        args: Vec<String>,
    ) -> Result<String> {
        // Check if command is allowed
        if !self.profile.is_command_allowed(command) {
            anyhow::bail!("Command '{}' is not allowed by sandbox profile", command);
        }

        let timeout = Duration::from_millis(self.profile.timeout_ms);

        let output = tokio::time::timeout(
            timeout,
            Command::new(command)
                .args(&args)
                .output(),
        )
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

/// LLM Utilities - unified interface for LLM operations
pub struct LlmUtilities {
    router: ModelRouter,
}

impl LlmUtilities {
    pub fn new(router: ModelRouter) -> Self {
        Self { router }
    }

    /// Unified call with automatic retry
    pub async fn call_with_retry(
        &self,
        prompt: &str,
        max_retries: u32,
        initial_delay_ms: u64,
    ) -> Result<String> {
        let mut last_error = None;
        
        for attempt in 0..=max_retries {
            match self.router.generate(prompt).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries {
                        let delay = initial_delay_ms * 2_u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM call failed")))
    }

    /// Streaming call with automatic retry
    pub async fn call_stream_with_retry<F>(
        &self,
        prompt: &str,
        callback: F,
        max_retries: u32,
        initial_delay_ms: u64,
    ) -> Result<String>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        let mut last_error = None;
        
        for attempt in 0..=max_retries {
            match self.router.generate_stream(prompt, callback.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries {
                        let delay = initial_delay_ms * 2_u64.pow(attempt);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM call failed")))
    }

    /// Simple call without retry
    pub async fn call(&self, prompt: &str) -> Result<String> {
        self.router.generate(prompt).await
    }

    /// Streaming call without retry
    pub async fn call_stream<F>(&self, prompt: &str, callback: F) -> Result<String>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        self.router.generate_stream(prompt, callback).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock LLM provider for testing
    struct MockLlmProvider {
        name: String,
        model: String,
        should_fail: bool,
    }

    impl MockLlmProvider {
        fn new(name: String, model: String, should_fail: bool) -> Self {
            Self {
                name,
                model,
                should_fail,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
        async fn generate(&self, prompt: &str) -> Result<String> {
            if self.should_fail {
                anyhow::bail!("Provider failed")
            }
            Ok(format!("{}: {}", self.model, prompt))
        }

        async fn generate_stream(&self, prompt: &str, callback: StreamCallback) -> Result<String> {
            if self.should_fail {
                anyhow::bail!("Provider failed")
            }
            let result = format!("{}: {}", self.model, prompt);
            callback(&result);
            Ok(result)
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn model(&self) -> &str {
            &self.model
        }
    }

    #[tokio::test]
    async fn test_model_router_basic() {
        let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), false));
        let router = ModelRouter::new(vec![provider]);

        let result = router.generate("test prompt").await.unwrap();
        assert!(result.contains("gpt-4"));
        assert!(result.contains("test prompt"));
    }

    #[tokio::test]
    async fn test_model_router_fallback() {
        let provider1 = Box::new(MockLlmProvider::new("failing".to_string(), "gpt-3".to_string(), true));
        let provider2 = Box::new(MockLlmProvider::new("working".to_string(), "gpt-4".to_string(), false));
        let router = ModelRouter::new(vec![provider1, provider2]);

        let result = router.generate("test prompt").await.unwrap();
        assert!(result.contains("gpt-4"));
    }

    #[tokio::test]
    async fn test_model_router_fallback_chain() {
        let provider1 = Box::new(MockLlmProvider::new("p1".to_string(), "gpt-3".to_string(), true));
        let provider2 = Box::new(MockLlmProvider::new("p2".to_string(), "gpt-4".to_string(), true));
        let provider3 = Box::new(MockLlmProvider::new("p3".to_string(), "claude".to_string(), false));
        let router = ModelRouter::new(vec![provider1, provider2, provider3])
            .with_fallback_chain(vec![0, 2, 1]);

        let result = router.generate("test prompt").await.unwrap();
        assert!(result.contains("claude"));
    }

    // Mock tool for testing
    struct MockTool {
        name: String,
        description: String,
    }

    impl MockTool {
        fn new(name: String, description: String) -> Self {
            Self { name, description }
        }
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> String {
            self.name.clone()
        }

        fn description(&self) -> String {
            self.description.clone()
        }

        async fn call(&self, input: ToolInput) -> Result<ToolOutput> {
            Ok(serde_json::json!({
                "tool": self.name,
                "input": input
            }))
        }
    }

    #[tokio::test]
    async fn test_tool_trait() {
        let tool = MockTool::new("test_tool".to_string(), "A test tool".to_string());
        
        assert_eq!(tool.name(), "test_tool");
        assert_eq!(tool.description(), "A test tool");
        
        let input = serde_json::json!({ "arg": "value" });
        let output = tool.call(input).await.unwrap();
        
        assert_eq!(output["tool"], "test_tool");
    }

    #[test]
    fn test_tool_sandbox_profile_default() {
        let profile = ToolSandboxProfile::new();
        assert!(profile.allowed_commands.len() > 0);
        assert_eq!(profile.timeout_ms, 30_000);
        assert!(!profile.allow_network);
    }

    #[tokio::test]
    async fn test_tool_sandbox_profile_command_checking() {
        let profile = ToolSandboxProfile::new();
        
        assert!(profile.is_command_allowed("echo hello"));
        assert!(profile.is_command_allowed("cat file.txt"));
        assert!(!profile.is_command_allowed("rm file.txt"));
        assert!(!profile.is_command_allowed("rmdir dir"));
    }

    #[test]
    fn test_tool_sandbox_profile_network_checking() {
        let mut profile = ToolSandboxProfile::new();
        
        // Network disabled by default
        assert!(!profile.is_network_allowed("example.com"));
        
        // Enable network
        profile.allow_network = true;
        assert!(profile.is_network_allowed("example.com"));
        
        // Add blocked host
        profile.blocked_network_hosts = vec!["malicious.com".to_string()];
        assert!(!profile.is_network_allowed("malicious.com"));
        assert!(profile.is_network_allowed("example.com"));
        
        // Add allowed host
        profile.allowed_network_hosts = vec!["trusted.com".to_string()];
        assert!(profile.is_network_allowed("trusted.com"));
        assert!(!profile.is_network_allowed("example.com"));
    }

    #[test]
    fn test_tool_sandbox_profile_file_checking() {
        let profile = ToolSandboxProfile::new();
        
        // File read enabled by default
        assert!(profile.is_file_read_allowed("/home/user/file.txt"));
        assert!(!profile.is_file_read_allowed("/etc/passwd"));
        
        // File write disabled by default
        assert!(!profile.is_file_write_allowed("/home/user/file.txt"));
        
        // Enable file write
        let mut profile = ToolSandboxProfile::new();
        profile.allow_file_write = true;
        assert!(profile.is_file_write_allowed("/home/user/file.txt"));
        assert!(!profile.is_file_write_allowed("/etc/passwd"));
    }

    #[tokio::test]
    async fn test_tool_runtime_execute_command() {
        // Use a custom profile that allows the appropriate command for each platform
        #[cfg(unix)]
        let profile = ToolSandboxProfile::custom(
            vec!["echo".to_string()],
            vec![],
            30000,
            10 * 1024 * 1024,
        );
        
        #[cfg(windows)]
        let profile = ToolSandboxProfile::custom(
            vec!["cmd".to_string()],
            vec![],
            30000,
            10 * 1024 * 1024,
        );
        
        let runtime = ToolRuntime::new(profile);
        
        #[cfg(unix)]
        let result = runtime.execute_command("echo", vec!["hello".to_string()]).await;
        
        #[cfg(windows)]
        let result = runtime.execute_command("cmd", vec!["/C".to_string(), "echo".to_string(), "hello".to_string()]).await;
        
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_runtime_command_blocked() {
        let profile = ToolSandboxProfile::custom(
            vec!["echo".to_string()],
            vec![],
            30000,
            10 * 1024 * 1024,
        );
        let runtime = ToolRuntime::new(profile);
        
        let result = runtime.execute_command("rm", vec!["-rf".to_string(), "/".to_string()]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_llm_utilities_call() {
        let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), false));
        let router = ModelRouter::new(vec![provider]);
        let utils = LlmUtilities::new(router);

        let result = utils.call("test prompt").await.unwrap();
        assert!(result.contains("gpt-4"));
        assert!(result.contains("test prompt"));
    }

    #[tokio::test]
    async fn test_llm_utilities_call_with_retry() {
        let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), false));
        let router = ModelRouter::new(vec![provider]);
        let utils = LlmUtilities::new(router);

        let result = utils.call_with_retry("test prompt", 3, 10).await.unwrap();
        assert!(result.contains("gpt-4"));
    }

    #[tokio::test]
    async fn test_llm_utilities_call_stream() {
        let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), false));
        let router = ModelRouter::new(vec![provider]);
        let utils = LlmUtilities::new(router);

        let result = utils.call_stream("test prompt", |_chunk| {
            // Just verify the callback is called
        }).await.unwrap();

        assert!(result.contains("gpt-4"));
    }
}
