//! Tests for intelligence module

use super::provider::{LlmProvider, StreamCallback};
use super::router::ModelRouter;
use super::tool::{Tool, ToolInput, ToolOutput, ToolRuntime, ToolSandboxProfile};
use super::*;
use std::sync::Arc;

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

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        if self.should_fail {
            anyhow::bail!("Provider failed")
        }
        Ok(format!("{}: {}", self.model, prompt))
    }

    async fn generate_stream(
        &self,
        prompt: &str,
        callback: StreamCallback,
    ) -> anyhow::Result<String> {
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
    let provider = Box::new(MockLlmProvider::new(
        "test".to_string(),
        "gpt-4".to_string(),
        false,
    ));
    let router = ModelRouter::new(vec![provider]);

    let result = router.generate("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
    assert!(result.contains("test prompt"));
}

#[tokio::test]
async fn test_model_router_fallback() {
    let provider1 = Box::new(MockLlmProvider::new(
        "failing".to_string(),
        "gpt-3".to_string(),
        true,
    ));
    let provider2 = Box::new(MockLlmProvider::new(
        "working".to_string(),
        "gpt-4".to_string(),
        false,
    ));
    let router = ModelRouter::new(vec![provider1, provider2]);

    let result = router.generate("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_model_router_fallback_chain() {
    let provider1 = Box::new(MockLlmProvider::new(
        "p1".to_string(),
        "gpt-3".to_string(),
        true,
    ));
    let provider2 = Box::new(MockLlmProvider::new(
        "p2".to_string(),
        "gpt-4".to_string(),
        true,
    ));
    let provider3 = Box::new(MockLlmProvider::new(
        "p3".to_string(),
        "claude".to_string(),
        false,
    ));
    let router =
        ModelRouter::new(vec![provider1, provider2, provider3]).with_fallback_chain(vec![0, 2, 1]);

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

#[async_trait::async_trait]
impl Tool for MockTool {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    async fn call(&self, input: ToolInput) -> anyhow::Result<ToolOutput> {
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
    use crate::tools::{ToolContext, ToolPermission, ToolPolicy};

    let tool_policy = ToolPolicy::new().with_permission(ToolPermission::Shell);
    let profile = ToolSandboxProfile::with_tool_policy(tool_policy);

    let runtime = ToolRuntime::new(profile);

    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "echo".to_string(),
        ToolPolicy::new().with_permission(ToolPermission::Shell),
    );

    #[cfg(unix)]
    let result = runtime
        .execute_command("echo", vec!["hello".to_string()], &context)
        .await;

    #[cfg(windows)]
    let result = runtime
        .execute_command(
            "cmd",
            vec!["/C".to_string(), "echo".to_string(), "hello".to_string()],
            &context,
        )
        .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_tool_runtime_command_blocked() {
    use crate::tools::{ToolContext, ToolPolicy};

    let profile =
        ToolSandboxProfile::custom(vec!["echo".to_string()], vec![], 30000, 10 * 1024 * 1024);
    let runtime = ToolRuntime::new(profile);

    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "rm".to_string(),
        ToolPolicy::new(),
    );

    let result = runtime
        .execute_command("rm", vec!["-rf".to_string(), "/".to_string()], &context)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_llm_utilities_call() {
    let provider = Box::new(MockLlmProvider::new(
        "test".to_string(),
        "gpt-4".to_string(),
        false,
    ));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);

    let result = utils.call("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
    assert!(result.contains("test prompt"));
}

#[tokio::test]
async fn test_llm_utilities_call_with_retry() {
    let provider = Box::new(MockLlmProvider::new(
        "test".to_string(),
        "gpt-4".to_string(),
        false,
    ));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);

    let result = utils.call_with_retry("test prompt", 3, 10).await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_llm_utilities_call_stream() {
    let provider = Box::new(MockLlmProvider::new(
        "test".to_string(),
        "gpt-4".to_string(),
        false,
    ));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);

    let result = utils
        .call_stream("test prompt", |_chunk| {
            // Just verify the callback is called
        })
        .await
        .unwrap();

    assert!(result.contains("gpt-4"));
}
