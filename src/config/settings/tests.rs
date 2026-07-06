use super::types::{AppConfig, BillingSource, LlmProviderConfig, LlmRoutingConfig};

#[test]
fn test_default_config_builds() {
    let config: AppConfig = serde_json::from_str("{}").expect("default config should parse");
    assert_eq!(config.provider, "openrouter");
    assert_eq!(config.base_url, "https://openrouter.ai/api");
    assert_eq!(config.model, "meta-llama/llama-3.3-8b-instruct:free");
    assert_eq!(config.llm_routing.providers.len(), 4);
    assert_eq!(
        config.llm_routing.billing_source,
        BillingSource::OpenrouterUser
    );
}

#[test]
fn test_openrouter_provider_config_parses() {
    let json = r#"{
        "name": "my_openrouter",
        "provider_type": "openrouter",
        "enabled": true,
        "base_url": "https://openrouter.ai/api",
        "model": "meta-llama/llama-3.3-8b-instruct:free",
        "api_key_env": "OPENROUTER_API_KEY"
    }"#;
    let config: LlmProviderConfig =
        serde_json::from_str(json).expect("OpenRouter config should parse");
    assert_eq!(config.provider_type, "openrouter");
    assert!(config.enabled);
    assert_eq!(config.api_key_env, Some("OPENROUTER_API_KEY".to_string()));
}

#[test]
fn test_ollama_provider_config_parses() {
    let json = r#"{
        "name": "ollama_local",
        "provider_type": "ollama",
        "enabled": true,
        "base_url": "http://localhost:11434",
        "model": "llama3.2",
        "api_key_env": null
    }"#;
    let config: LlmProviderConfig = serde_json::from_str(json).expect("Ollama config should parse");
    assert_eq!(config.provider_type, "ollama");
    assert!(config.enabled);
    assert_eq!(config.base_url, "http://localhost:11434");
    assert_eq!(config.api_key_env, None);
}

#[test]
fn test_lmstudio_provider_config_parses() {
    let json = r#"{
        "name": "lmstudio_local",
        "provider_type": "lmstudio",
        "enabled": true,
        "base_url": "http://localhost:1234",
        "model": "local-model",
        "api_key_env": null
    }"#;
    let config: LlmProviderConfig =
        serde_json::from_str(json).expect("LM Studio config should parse");
    assert_eq!(config.provider_type, "lmstudio");
    assert_eq!(config.base_url, "http://localhost:1234");
}

#[test]
fn test_generic_openai_provider_config_parses() {
    let json = r#"{
        "name": "custom_provider",
        "provider_type": "my_custom_type",
        "enabled": true,
        "base_url": "https://custom-api.example.com",
        "model": "custom-model",
        "api_key_env": "CUSTOM_API_KEY"
    }"#;
    let config: LlmProviderConfig =
        serde_json::from_str(json).expect("generic config should parse");
    assert_eq!(config.provider_type, "my_custom_type");
    assert_eq!(config.api_key_env, Some("CUSTOM_API_KEY".to_string()));
}

#[test]
fn test_disabled_provider_has_enabled_false() {
    let json = r#"{
        "name": "disabled_provider",
        "provider_type": "openrouter",
        "enabled": false,
        "base_url": "https://openrouter.ai/api",
        "model": "some-model",
        "api_key_env": null
    }"#;
    let config: LlmProviderConfig =
        serde_json::from_str(json).expect("disabled config should parse");
    assert!(!config.enabled);
}

#[test]
fn test_mode_chains_default_ordering() {
    let routing: LlmRoutingConfig =
        serde_json::from_str("{}").expect("default routing should parse");
    assert_eq!(
        routing.mode_chains.fast,
        vec!["openrouter_fast", "openrouter_balanced"]
    );
    assert_eq!(
        routing.mode_chains.balanced,
        vec!["openrouter_balanced", "openrouter_fast"]
    );
    assert_eq!(
        routing.mode_chains.deep,
        vec!["openrouter_deep", "openrouter_balanced"]
    );
    assert_eq!(
        routing.mode_chains.coding,
        vec!["openrouter_coding", "openrouter_deep"]
    );
}

#[test]
fn test_missing_api_key_env_is_none() {
    let json = r#"{
        "name": "no_key_provider",
        "provider_type": "ollama",
        "enabled": true,
        "base_url": "http://localhost:11434",
        "model": "llama3.2"
    }"#;
    let config: LlmProviderConfig =
        serde_json::from_str(json).expect("config without api_key_env should parse");
    assert_eq!(config.api_key_env, None);
}

#[test]
fn test_mode_chain_custom_order_preserved() {
    let json = r#"{
        "providers": [
            { "name": "a", "provider_type": "openrouter", "enabled": true, "base_url": "http://a", "model": "m1", "api_key_env": null },
            { "name": "b", "provider_type": "openrouter", "enabled": true, "base_url": "http://b", "model": "m2", "api_key_env": null },
            { "name": "c", "provider_type": "openrouter", "enabled": true, "base_url": "http://c", "model": "m3", "api_key_env": null }
        ],
        "mode_chains": {
            "fast": ["c", "a"],
            "balanced": ["b", "c", "a"],
            "deep": ["a"],
            "coding": ["b"]
        }
    }"#;
    let routing: LlmRoutingConfig =
        serde_json::from_str(json).expect("custom routing should parse");
    assert_eq!(routing.mode_chains.fast, vec!["c", "a"]);
    assert_eq!(routing.mode_chains.balanced, vec!["b", "c", "a"]);
    assert_eq!(routing.mode_chains.deep, vec!["a"]);
    assert_eq!(routing.mode_chains.coding, vec!["b"]);
}

#[test]
fn test_top_level_fields_override_defaults() {
    let json = r#"{
        "provider": "ollama",
        "base_url": "http://localhost:11434",
        "model": "llama3.2"
    }"#;
    let config: AppConfig = serde_json::from_str(json).expect("top-level config should parse");
    assert_eq!(config.provider, "ollama");
    assert_eq!(config.base_url, "http://localhost:11434");
    assert_eq!(config.model, "llama3.2");
}
