//! Default configuration values

use super::types::{
    BillingSource, LlmProviderConfig, LlmRoutingConfig, MemoryBudget, ModeChains, StrictMode,
};

pub fn default_provider() -> String {
    "openrouter".to_string()
}

pub fn default_base_url() -> String {
    "https://openrouter.ai/api".to_string()
}

pub fn default_model() -> String {
    "meta-llama/llama-3.3-8b-instruct:free".to_string()
}

pub fn default_embedding_url() -> String {
    "http://localhost:11434".to_string()
}

pub fn default_embedding_dimension() -> usize {
    1536
}

pub fn default_memory_db_path() -> String {
    "prometheos_memory.db".to_string()
}

pub fn default_context_window_size() -> usize {
    4096
}

pub fn default_memory_budget() -> MemoryBudget {
    MemoryBudget {
        project_facts: 0.4,
        user_preferences: 0.25,
        recent_episodes: 0.2,
        decisions_constraints: 0.15,
    }
}

pub fn default_strict_mode() -> StrictMode {
    StrictMode::default()
}

pub fn default_strict_mode_enforce_missing_inputs() -> bool {
    false
}

pub fn default_strict_mode_enforce_missing_services() -> bool {
    false
}

pub fn default_strict_mode_enforce_empty_outputs() -> bool {
    false
}

pub fn default_strict_mode_enforce_no_unwrap() -> bool {
    false
}

pub fn default_strict_mode_enforce_no_silent_none() -> bool {
    false
}

pub fn default_strict_mode_enforce_idempotency() -> bool {
    false
}

pub fn default_repo_path() -> String {
    ".".to_string()
}

pub fn default_billing_source() -> BillingSource {
    BillingSource::OpenrouterUser
}

pub fn default_provider_entries() -> Vec<LlmProviderConfig> {
    vec![
        LlmProviderConfig {
            name: "openrouter_fast".to_string(),
            provider_type: "openrouter".to_string(),
            enabled: true,
            base_url: "https://openrouter.ai/api".to_string(),
            model: "meta-llama/llama-3.3-8b-instruct:free".to_string(),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
        },
        LlmProviderConfig {
            name: "openrouter_balanced".to_string(),
            provider_type: "openrouter".to_string(),
            enabled: true,
            base_url: "https://openrouter.ai/api".to_string(),
            model: "mistralai/mistral-7b-instruct:free".to_string(),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
        },
        LlmProviderConfig {
            name: "openrouter_deep".to_string(),
            provider_type: "openrouter".to_string(),
            enabled: true,
            base_url: "https://openrouter.ai/api".to_string(),
            model: "qwen/qwen3-8b:free".to_string(),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
        },
        LlmProviderConfig {
            name: "openrouter_coding".to_string(),
            provider_type: "openrouter".to_string(),
            enabled: true,
            base_url: "https://openrouter.ai/api".to_string(),
            model: "deepseek/deepseek-r1-0528-qwen3-8b:free".to_string(),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
        },
    ]
}

pub fn default_mode_chains() -> ModeChains {
    ModeChains {
        fast: vec!["openrouter_fast".to_string(), "openrouter_balanced".to_string()],
        balanced: vec!["openrouter_balanced".to_string(), "openrouter_fast".to_string()],
        deep: vec!["openrouter_deep".to_string(), "openrouter_balanced".to_string()],
        coding: vec!["openrouter_coding".to_string(), "openrouter_deep".to_string()],
    }
}

pub fn default_llm_routing() -> LlmRoutingConfig {
    LlmRoutingConfig {
        billing_source: default_billing_source(),
        providers: default_provider_entries(),
        mode_chains: default_mode_chains(),
    }
}
