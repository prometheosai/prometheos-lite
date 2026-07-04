//! Local-first LLM client integrations

mod llm_client;
mod types;
mod utils;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;

pub use llm_client::LlmClient;
pub use types::*;
pub use utils::generate;
