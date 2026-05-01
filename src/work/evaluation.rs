//! Enhanced evaluation system for WorkContext assessment
//!
//! This module provides multi-dimensional evaluation including structural validation,
//! semantic evaluation, and penalization rules for poor performance.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::flow::ModelRouter;

/// Evaluation dimensions for comprehensive assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationDimensions {
    /// Correctness score (0.0 to 1.0)
    pub correctness: f32,
    /// Completeness score (0.0 to 1.0)
    pub completeness: f32,
    /// Efficiency score (0.0 to 1.0)
    pub efficiency: f32,
    /// Reliability score (0.0 to 1.0)
    pub reliability: f32,
}

impl Default for EvaluationDimensions {
    fn default() -> Self {
        Self {
            correctness: 0.5,
            completeness: 0.5,
            efficiency: 0.5,
            reliability: 0.5,
        }
    }
}

impl EvaluationDimensions {
    /// Calculate overall score from dimensions
    pub fn overall_score(&self) -> f32 {
        (self.correctness * 0.4
            + self.completeness * 0.3
            + self.efficiency * 0.15
            + self.reliability * 0.15)
            .clamp(0.0, 1.0)
    }
}

/// Enhanced evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedEvaluationResult {
    /// Overall score (0.0 to 1.0)
    pub overall_score: f32,
    /// Semantic correctness score (LLM-based)
    pub semantic_score: f32,
    /// Structural correctness score (schema validation)
    pub structural_score: f32,
    /// Tool consistency score
    pub tool_consistency_score: f32,
    /// Artifact completeness score
    pub artifact_completeness_score: f32,
    /// Evaluation dimensions
    pub dimensions: EvaluationDimensions,
    /// Applied penalties
    pub penalties: Vec<Penalty>,
    /// Evaluation timestamp
    pub evaluated_at: chrono::DateTime<chrono::Utc>,
    /// Evaluation details
    pub details: String,
}

/// Penalty for poor performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Penalty {
    /// Penalty type
    pub penalty_type: PenaltyType,
    /// Severity (0.0 to 1.0)
    pub severity: f32,
    /// Description
    pub description: String,
}

/// Types of penalties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PenaltyType {
    HighRetries,
    FailedTests,
    HallucinatedOutput,
    Timeout,
    ToolFailure,
    MemoryOverflow,
    ContextTruncation,
}

/// Structural validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralValidation {
    /// Whether the structure is valid
    pub is_valid: bool,
    /// Validation errors
    pub errors: Vec<String>,
    /// Validation warnings
    pub warnings: Vec<String>,
}

/// Evaluation engine
pub struct EvaluationEngine {
    model_router: Option<std::sync::Arc<ModelRouter>>,
}

impl EvaluationEngine {
    /// Create a new evaluation engine
    pub fn new(model_router: Option<std::sync::Arc<ModelRouter>>) -> Self {
        Self { model_router }
    }

    /// Evaluate a WorkContext with comprehensive assessment
    pub async fn evaluate(
        &self,
        context: &crate::work::types::WorkContext,
        execution_metadata: &serde_json::Value,
    ) -> Result<EnhancedEvaluationResult> {
        let mut dimensions = EvaluationDimensions::default();
        let mut penalties = Vec::new();
        let mut details = String::new();

        // Structural validation
        let structural_validation = self.validate_structure(context, execution_metadata)?;
        dimensions.correctness = if structural_validation.is_valid {
            1.0
        } else {
            0.5
        };

        if !structural_validation.is_valid {
            details.push_str(&format!(
                "Structural validation failed: {:?}\n",
                structural_validation.errors
            ));
        }

        // Semantic evaluation (if LLM available)
        let semantic_score = if let Some(ref router) = self.model_router {
            self.semantic_evaluate(context, router).await?
        } else {
            0.5 // Default if no LLM
        };

        // Tool consistency
        let tool_consistency = self.evaluate_tool_consistency(execution_metadata)?;
        dimensions.reliability = tool_consistency;

        // Artifact completeness
        let artifact_completeness = self.evaluate_artifact_completeness(context)?;
        dimensions.completeness = artifact_completeness;

        // Efficiency based on retry count
        let retry_count = execution_metadata["retry_count"].as_u64().unwrap_or(0);
        if retry_count > 3 {
            penalties.push(Penalty {
                penalty_type: PenaltyType::HighRetries,
                severity: (retry_count as f32 / 10.0).min(1.0),
                description: format!("High retry count: {}", retry_count),
            });
            dimensions.efficiency = 0.3;
        }

        // Test failures
        if let Some(test_results) = execution_metadata.get("test_results") {
            if let Some(failed) = test_results["failed"].as_u64() {
                if failed > 0 {
                    penalties.push(Penalty {
                        penalty_type: PenaltyType::FailedTests,
                        severity: (failed as f32 / 5.0).min(1.0),
                        description: format!("Failed tests: {}", failed),
                    });
                    dimensions.correctness *= 0.5;
                }
            }
        }

        // Apply penalties to overall score
        let penalty_severity: f32 = penalties.iter().map(|p| p.severity).sum();
        let overall_score = dimensions.overall_score() * (1.0 - penalty_severity.min(0.8));

        details.push_str(&format!(
            "Dimensions: correctness={:.2}, completeness={:.2}, efficiency={:.2}, reliability={:.2}\n",
            dimensions.correctness, dimensions.completeness, dimensions.efficiency, dimensions.reliability
        ));

        Ok(EnhancedEvaluationResult {
            overall_score,
            semantic_score,
            structural_score: dimensions.correctness,
            tool_consistency_score: tool_consistency,
            artifact_completeness_score: artifact_completeness,
            dimensions,
            penalties,
            evaluated_at: chrono::Utc::now(),
            details,
        })
    }

    /// Validate the structure of a WorkContext
    pub fn validate_structure(&self, context: &crate::work::types::WorkContext, execution_metadata: &serde_json::Value) -> Result<StructuralValidation> {
        let mut errors = Vec::new();

        // Check if required fields are present
        if context.goal.is_empty() {
            errors.push("Goal is empty".to_string());
        }

        // Validate patch structure if present
        if let Some(patch) = execution_metadata.get("patch") {
            if !patch.is_object() {
                errors.push("Patch is not a valid object".to_string());
            }
        }

        // Validate test results structure if present
        if let Some(test_results) = execution_metadata.get("test_results") {
            if !test_results.is_object() {
                errors.push("Test results is not a valid object".to_string());
            }
        }

        Ok(StructuralValidation {
            is_valid: errors.is_empty(),
            errors,
            warnings: Vec::new(),
        })
    }

    /// Semantic evaluation using LLM
    async fn semantic_evaluate(
        &self,
        context: &crate::work::types::WorkContext,
        router: &ModelRouter,
    ) -> Result<f32> {
        let prompt = format!(
            "Evaluate the quality of the following work on a scale of 0.0 to 1.0:\n\nGoal: {}\n\nArtifacts: {}\n\nRespond with only a number between 0.0 and 1.0.",
            context.goal,
            serde_json::to_string(&context.artifacts).unwrap_or_default()
        );

        let result = router.generate(&prompt).await?;
        
        // Parse the score from the response
        let score_str = result.trim();
        score_str.parse::<f32>()
            .map(|s| s.clamp(0.0, 1.0))
            .map_err(|e| anyhow::anyhow!("Failed to parse semantic score '{}': {}", score_str, e))
    }

    /// Evaluate tool consistency
    pub fn evaluate_tool_consistency(&self, execution_metadata: &serde_json::Value) -> Result<f32> {
        let tool_calls = execution_metadata.get("tool_calls");

        if let Some(calls) = tool_calls.and_then(|c| c.as_array()) {
            if calls.is_empty() {
                return Ok(1.0); // No tools used is OK
            }

            // Check for failed tool calls
            let failed_count = calls
                .iter()
                .filter(|call| call.get("success").and_then(|s| s.as_bool()) == Some(false))
                .count();

            if failed_count > 0 {
                return Ok(1.0 - (failed_count as f32 / calls.len() as f32));
            }
        }

        Ok(1.0)
    }

    /// Evaluate artifact completeness
    pub fn evaluate_artifact_completeness(&self, context: &crate::work::types::WorkContext) -> Result<f32> {
        if context.artifacts.is_empty() {
            return Ok(0.5); // Neutral if no artifacts
        }

        // Check if artifacts have required fields
        let complete_count = context
            .artifacts
            .iter()
            .filter(|a| !a.content.as_str().map(|s| s.is_empty()).unwrap_or(true))
            .count();

        Ok(complete_count as f32 / context.artifacts.len() as f32)
    }
}

impl Default for EvaluationEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_evaluation_dimensions_overall_score() {
        let dimensions = EvaluationDimensions {
            correctness: 0.8,
            completeness: 0.9,
            efficiency: 0.7,
            reliability: 0.85,
        };

        let overall = dimensions.overall_score();
        assert!(overall > 0.0 && overall <= 1.0);
    }

    #[test]
    fn test_structural_validation_valid() {
        let engine = EvaluationEngine::default();
        
        let context = crate::work::types::WorkContext::new(
            uuid::Uuid::new_v4().to_string(),
            "test-user".to_string(),
            "Test task".to_string(),
            crate::work::types::WorkDomain::Software,
            "Test goal".to_string(),
        );

        let execution_metadata = serde_json::json!({});
        let validation = engine.validate_structure(&context, &execution_metadata).unwrap();
        
        assert!(validation.is_valid);
        assert!(validation.errors.is_empty());
    }

    #[test]
    fn test_structural_validation_invalid() {
        let engine = EvaluationEngine::default();
        
        let mut context = crate::work::types::WorkContext::new(
            uuid::Uuid::new_v4().to_string(),
            "test-user".to_string(),
            "Test task".to_string(),
            crate::work::types::WorkDomain::Software,
            "".to_string(), // Empty goal
        );

        let execution_metadata = serde_json::json!({});
        let validation = engine.validate_structure(&context, &execution_metadata).unwrap();
        
        assert!(!validation.is_valid);
        assert!(!validation.errors.is_empty());
    }

    #[test]
    fn test_evaluate_tool_consistency() {
        let engine = EvaluationEngine::default();

        let metadata = serde_json::json!({
            "tool_calls": [
                {"success": true},
                {"success": true},
                {"success": false}
            ]
        });

        let score = engine.evaluate_tool_consistency(&metadata).unwrap();
        assert!(score < 1.0); // Should be less than 1.0 due to failure
    }

    #[test]
    fn test_evaluate_artifact_completeness() {
        let engine = EvaluationEngine::default();
        
        let mut context = crate::work::types::WorkContext::new(
            uuid::Uuid::new_v4().to_string(),
            "test-user".to_string(),
            "Test task".to_string(),
            crate::work::types::WorkDomain::Software,
            "Test goal".to_string(),
        );
        
        let context_id = context.id.clone();
        context.artifacts = vec![
            crate::work::Artifact::new(
                "1".to_string(),
                context_id.clone(),
                crate::work::ArtifactKind::Code,
                "test_artifact".to_string(),
                serde_json::Value::String("test".to_string()),
                "test-user".to_string(),
            ),
            crate::work::Artifact::new(
                "2".to_string(),
                context_id,
                crate::work::ArtifactKind::Code,
                "incomplete_artifact".to_string(),
                serde_json::Value::String("".to_string()), // Incomplete
                "test-user".to_string(),
            ),
        ];

        let score = engine.evaluate_artifact_completeness(&context).unwrap();
        assert!(score == 0.5); // 1 out of 2 complete
    }

    #[test]
    fn test_evaluate_artifact_completeness_no_artifacts() {
        let engine = EvaluationEngine::default();
        
        let context = crate::work::types::WorkContext::new(
            uuid::Uuid::new_v4().to_string(),
            "test-user".to_string(),
            "Test task".to_string(),
            crate::work::types::WorkDomain::Software,
            "Test goal".to_string(),
        );
        
        let score = engine.evaluate_artifact_completeness(&context).unwrap();
        
        assert_eq!(score, 0.5); // Neutral if no artifacts
    }
}
