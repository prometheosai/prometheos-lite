//! Hybrid intent classifier (rule-based + LLM fallback)

use crate::config::AppConfig;
use crate::intent::rules::RuleClassifier;
use crate::intent::types::{Intent, IntentClassificationResult};
use crate::llm::LlmClient;
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Hybrid intent classifier
pub struct IntentClassifier {
    llm_client: Option<LlmClient>,
    confidence_threshold: f32,
    cache: HashMap<String, IntentClassificationResult>,
}

impl IntentClassifier {
    /// Create a new intent classifier
    pub fn new() -> Result<Self> {
        let llm_client = match AppConfig::load() {
            Ok(config) => match LlmClient::from_config(&config) {
                Ok(client) => Some(client),
                Err(_) => None,
            },
            Err(_) => None,
        };

        // Initialize with common cached intents
        let mut cache = HashMap::new();
        cache.insert(
            "hi".to_string(),
            IntentClassificationResult {
                intent: Intent::Conversation,
                confidence: 1.0,
                reason: "Cached common intent".to_string(),
                override_: false,
            },
        );
        cache.insert(
            "hello".to_string(),
            IntentClassificationResult {
                intent: Intent::Conversation,
                confidence: 1.0,
                reason: "Cached common intent".to_string(),
                override_: false,
            },
        );
        cache.insert(
            "thanks".to_string(),
            IntentClassificationResult {
                intent: Intent::Conversation,
                confidence: 1.0,
                reason: "Cached common intent".to_string(),
                override_: false,
            },
        );
        cache.insert(
            "thank you".to_string(),
            IntentClassificationResult {
                intent: Intent::Conversation,
                confidence: 1.0,
                reason: "Cached common intent".to_string(),
                override_: false,
            },
        );

        Ok(Self {
            llm_client,
            confidence_threshold: 0.7,
            cache,
        })
    }

    /// Set the confidence threshold for LLM fallback
    pub fn with_confidence_threshold(mut self, threshold: f32) -> Self {
        self.confidence_threshold = threshold;
        self
    }

    /// Classify intent using hybrid approach
    pub async fn classify(&self, message: &str) -> Result<IntentClassificationResult> {
        self.classify_with_override(message, None).await
    }

    /// Classify intent with optional override
    pub async fn classify_with_override(
        &self,
        message: &str,
        override_intent: Option<Intent>,
    ) -> Result<IntentClassificationResult> {
        // If override is provided, use it
        if let Some(intent) = override_intent {
            let result = IntentClassificationResult {
                intent,
                confidence: 1.0,
                reason: "User override command".to_string(),
                override_: true,
            };
            println!(
                "[INTENT] {} (confidence: {:.2}, routing: override)",
                result.intent.display_name(),
                result.confidence
            );
            return Ok(result);
        }

        let normalized = message.trim().to_lowercase();

        // Check cache first
        if let Some(cached) = self.cache.get(&normalized) {
            let mut result = cached.clone();
            result.override_ = false;
            println!(
                "[INTENT] {} (confidence: {:.2}, routing: cached)",
                result.intent.display_name(),
                result.confidence
            );
            return Ok(result);
        }

        // First, try rule-based classification
        if let Some(intent) = RuleClassifier::classify(message) {
            let result = IntentClassificationResult {
                intent,
                confidence: 0.95, // High confidence for rule-based matches
                reason: "Rule-based classification matched".to_string(),
                override_: false,
            };
            println!(
                "[INTENT] {} (confidence: {:.2}, routing: rule_based)",
                result.intent.display_name(),
                result.confidence
            );
            return Ok(result);
        }

        // Rule-based didn't match, try LLM fallback
        if let Some(ref llm_client) = self.llm_client {
            let result = self.llm_classify(llm_client, message).await?;
            println!(
                "[INTENT] {} (confidence: {:.2}, routing: llm_fallback)",
                result.intent.display_name(),
                result.confidence
            );
            Ok(result)
        } else {
            // No LLM client available, return ambiguous
            let result = IntentClassificationResult {
                intent: Intent::Ambiguous,
                confidence: 0.0,
                reason: "No LLM client available for classification".to_string(),
                override_: false,
            };
            println!(
                "[INTENT] {} (confidence: {:.2}, routing: fallback)",
                result.intent.display_name(),
                result.confidence
            );
            Ok(result)
        }
    }

    /// Classify intent using LLM
    async fn llm_classify(
        &self,
        llm_client: &LlmClient,
        message: &str,
    ) -> Result<IntentClassificationResult> {
        let classifier_prompt = format!(
            r#"Classify the user message into one intent:
CONVERSATION, QUESTION, CODING_TASK, FILE_EDIT, TOOL_ACTION, PROJECT_ACTION, AMBIGUOUS.

User message: "{}"

Return only JSON in this exact format:
{{"intent":"...", "confidence":0.0, "reason":"..."}}"#,
            message
        );

        let response = llm_client
            .generate(&classifier_prompt)
            .await
            .context("LLM classification call failed")?;

        // Parse JSON response
        let parsed: serde_json::Value =
            serde_json::from_str(&response).context("Failed to parse LLM classification JSON")?;

        let intent_str = parsed["intent"]
            .as_str()
            .context("Missing 'intent' field in LLM response")?;

        let confidence = parsed["confidence"]
            .as_f64()
            .context("Missing 'confidence' field in LLM response")? as f32;

        let reason = parsed["reason"]
            .as_str()
            .unwrap_or("LLM classification")
            .to_string();

        // Parse intent string
        let intent = match intent_str.to_lowercase().as_str() {
            "conversation" => Intent::Conversation,
            "question" => Intent::Question,
            "coding_task" => Intent::CodingTask,
            "file_edit" => Intent::FileEdit,
            "tool_action" => Intent::ToolAction,
            "project_action" => Intent::ProjectAction,
            "ambiguous" => Intent::Ambiguous,
            _ => Intent::Ambiguous,
        };

        Ok(IntentClassificationResult {
            intent,
            confidence,
            reason,
            override_: false,
        })
    }
}

impl Default for IntentClassifier {
    fn default() -> Self {
        Self::new().expect("Failed to create IntentClassifier")
    }
}
