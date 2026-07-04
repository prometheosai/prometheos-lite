//! Local-first LLM client integrations

mod llm_client;
mod types;
mod utils;

pub use llm_client::LlmClient;
pub use types::*;
pub use utils::generate;
