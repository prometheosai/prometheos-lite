//! Intelligence - Model Router, Tool Runtime, LLM Utilities

mod provider;
mod router;
mod tool;
mod utils;

#[cfg(test)]
mod tests;

pub use provider::{
    AnthropicProvider, GenericOpenAiCompatibleProvider, LlmProvider, LmStudioProvider,
    OllamaProvider, OpenAiProvider, OpenRouterProvider, ProviderError, ProviderErrorKind,
    ProviderKind, ProviderMetadata, StreamCallback,
};
pub use router::{AttemptMetadata, GenerateResult, LlmMode, ModelRouter};
pub use tool::{Tool, ToolInput, ToolOutput, ToolRegistry, ToolRuntime, ToolSandboxProfile};
pub use utils::LlmUtilities;
