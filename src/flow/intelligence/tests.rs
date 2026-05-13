//! Tests for intelligence module

use super::provider::{
    LlmProvider, ProviderErrorKind, ProviderKind, ProviderMetadata, StreamCallback,
};
use super::router::{LlmMode, ModelRouter};
use super::tool::{Tool, ToolInput, ToolOutput, ToolRuntime, ToolSandboxProfile};
use super::*;

struct MockLlmProvider {
    name: String,
    model: String,
    fail_kind: Option<ProviderErrorKind>,
}

impl MockLlmProvider {
    fn new(name: String, model: String, fail_kind: Option<ProviderErrorKind>) -> Self {
        Self {
            name,
            model,
            fail_kind,
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        if let Some(kind) = self.fail_kind {
            return Err(anyhow::anyhow!("Provider failed: {:?}", kind));
        }
        Ok(format!("{}: {}", self.model, prompt))
    }

    async fn generate_stream(
        &self,
        prompt: &str,
        callback: StreamCallback,
    ) -> anyhow::Result<String> {
        if let Some(kind) = self.fail_kind {
            return Err(anyhow::anyhow!("Provider failed: {:?}", kind));
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

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            kind: ProviderKind::GenericOpenAiCompatible,
            supports_streaming: true,
            local: false,
        }
    }

    fn classify_error(&self, err: &anyhow::Error) -> ProviderErrorKind {
        let msg = err.to_string().to_lowercase();
        if msg.contains("quota") {
            return ProviderErrorKind::Quota;
        }
        ProviderErrorKind::Fatal
    }
}

#[tokio::test]
async fn test_model_router_basic() {
    let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), None));
    let router = ModelRouter::new(vec![provider]);
    let result = router.generate("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_model_router_fallback() {
    let provider1 = Box::new(MockLlmProvider::new(
        "failing".to_string(),
        "gpt-3".to_string(),
        Some(ProviderErrorKind::Fatal),
    ));
    let provider2 = Box::new(MockLlmProvider::new("working".to_string(), "gpt-4".to_string(), None));
    let router = ModelRouter::new(vec![provider1, provider2]);
    let result = router.generate("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_model_router_mode_chain() {
    let provider1 = Box::new(MockLlmProvider::new("p1".to_string(), "slow".to_string(), None));
    let provider2 = Box::new(MockLlmProvider::new("p2".to_string(), "fast".to_string(), None));
    let router = ModelRouter::new(vec![provider1, provider2]).with_mode_chain(LlmMode::Fast, vec![1, 0]);
    let result = router.generate_for_mode(LlmMode::Fast, "test prompt").await.unwrap();
    assert!(result.contains("fast"));
}

#[tokio::test]
async fn test_model_router_quota_rotation_metadata() {
    let provider1 = Box::new(MockLlmProvider::new(
        "quota".to_string(),
        "m1".to_string(),
        Some(ProviderErrorKind::Quota),
    ));
    let provider2 = Box::new(MockLlmProvider::new("ok".to_string(), "m2".to_string(), None));
    let router = ModelRouter::new(vec![provider1, provider2]);

    let result = router
        .generate_for_mode_with_metadata(LlmMode::Balanced, "test prompt")
        .await
        .unwrap();
    assert!(result.quota_rotation_used);
    assert_eq!(result.provider, "ok");
}

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
        Ok(serde_json::json!({"tool": self.name,"input": input}))
    }
}

#[tokio::test]
async fn test_tool_trait() {
    let tool = MockTool::new("test_tool".to_string(), "A test tool".to_string());
    assert_eq!(tool.name(), "test_tool");
    let input = serde_json::json!({ "arg": "value" });
    let output = tool.call(input).await.unwrap();
    assert_eq!(output["tool"], "test_tool");
}

#[test]
fn test_tool_sandbox_profile_default() {
    let profile = ToolSandboxProfile::new();
    assert!(!profile.allowed_commands.is_empty());
    assert_eq!(profile.timeout_ms, 30_000);
    assert!(!profile.allow_network);
}

#[tokio::test]
async fn test_tool_sandbox_profile_command_checking() {
    let profile = ToolSandboxProfile::new();
    assert!(profile.is_command_allowed("echo hello"));
    assert!(!profile.is_command_allowed("rm file.txt"));
}

#[test]
fn test_tool_sandbox_profile_network_checking() {
    let mut profile = ToolSandboxProfile::new();
    assert!(!profile.is_network_allowed("example.com"));
    profile.allow_network = true;
    assert!(profile.is_network_allowed("example.com"));
}

#[test]
fn test_tool_sandbox_profile_file_checking() {
    let profile = ToolSandboxProfile::new();
    assert!(profile.is_file_read_allowed("/home/user/file.txt"));
    assert!(!profile.is_file_write_allowed("/home/user/file.txt"));
}

#[tokio::test]
async fn test_tool_runtime_command_blocked() {
    use crate::tools::{ToolContext, ToolPolicy};
    let profile = ToolSandboxProfile::custom(vec!["echo".to_string()], vec![], 30000, 10 * 1024 * 1024);
    let runtime = ToolRuntime::new(profile);
    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "rm".to_string(),
        ToolPolicy::new(),
    );
    let result = runtime.execute_command("rm", vec!["-rf".to_string(), "/".to_string()], &context).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_llm_utilities_call() {
    let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), None));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);
    let result = utils.call("test prompt").await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_llm_utilities_call_with_retry() {
    let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), None));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);
    let result = utils.call_with_retry("test prompt", 2, 1).await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_llm_utilities_call_stream() {
    let provider = Box::new(MockLlmProvider::new("test".to_string(), "gpt-4".to_string(), None));
    let router = ModelRouter::new(vec![provider]);
    let utils = LlmUtilities::new(router);
    let result = utils.call_stream("test prompt", |_| {}).await.unwrap();
    assert!(result.contains("gpt-4"));
}

#[tokio::test]
async fn test_model_router_stream_metadata() {
    let provider1 = Box::new(MockLlmProvider::new(
        "quota".to_string(),
        "m1".to_string(),
        Some(ProviderErrorKind::Quota),
    ));
    let provider2 = Box::new(MockLlmProvider::new("ok".to_string(), "m2".to_string(), None));
    let router = ModelRouter::new(vec![provider1, provider2]);
    let res = router
        .generate_stream_with_metadata("test prompt", std::sync::Arc::new(|_| {}))
        .await
        .unwrap();
    assert_eq!(res.provider, "ok");
    assert!(res.fallback_used);
    assert!(res.quota_rotation_used);
    assert_eq!(res.fallback_count, 1);
    assert_eq!(res.attempted_path.len(), 2);
}
