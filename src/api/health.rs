//! Health check endpoint

use axum::http::StatusCode;
use axum::Json;
use std::collections::HashSet;

use crate::config::AppConfig;

/// Health check response
#[derive(serde::Serialize)]
pub struct HealthResponse {
    status: String,
}

#[derive(serde::Serialize)]
pub struct RuntimeStackResponse {
    provider: String,
    provider_label: String,
    primary_model: String,
    fallback_models: Vec<String>,
    embedding_model: String,
    embedding_dimension: usize,
}

fn provider_label(provider: &str) -> String {
    match provider.to_ascii_lowercase().as_str() {
        "openrouter" => "OpenRouter".to_string(),
        "lmstudio" => "LM Studio".to_string(),
        "openai" => "OpenAI".to_string(),
        "anthropic" => "Anthropic".to_string(),
        other => {
            if other.is_empty() {
                "Unknown".to_string()
            } else {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => "Unknown".to_string(),
                }
            }
        }
    }
}

/// Health check endpoint
///
/// Returns a simple JSON response indicating the server is running.
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

/// Returns the currently configured runtime LLM + embedding stack.
pub async fn runtime_stack() -> Result<Json<RuntimeStackResponse>, StatusCode> {
    let config = AppConfig::load().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut seen = HashSet::new();
    let fallback_models: Vec<String> = config
        .llm_routing
        .providers
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|p| {
            if p.model == config.model {
                return None;
            }
            if seen.insert(p.model.clone()) {
                Some(p.model.clone())
            } else {
                None
            }
        })
        .collect();

    let embedding_model = if config.embedding_model.trim().is_empty() {
        "not-configured".to_string()
    } else {
        config.embedding_model.clone()
    };

    Ok(Json(RuntimeStackResponse {
        provider: config.provider.clone(),
        provider_label: provider_label(&config.provider),
        primary_model: config.model.clone(),
        fallback_models,
        embedding_model,
        embedding_dimension: config.embedding_dimension,
    }))
}
