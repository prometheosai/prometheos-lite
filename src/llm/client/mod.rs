//! Local-first LLM client integrations

mod client;
mod types;
mod utils;

pub use client::LlmClient;
pub use types::*;
pub use utils::generate;
