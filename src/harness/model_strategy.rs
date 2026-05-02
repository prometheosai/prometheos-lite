//! Model Strategy - Issue #18
//! Multi-model routing and selection for optimal task execution

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelTier {
    Fast,      // Quick tasks, simple edits
    Balanced,  // Standard development work
    Powerful,  // Complex analysis and generation
    Expert,    // Critical architectural decisions
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub tier: ModelTier,
    pub context_window: usize,
    pub cost_per_1k_tokens: f64,
    pub avg_latency_ms: u64,
    pub strengths: Vec<String>,
    pub weaknesses: Vec<String>,
    pub supported_languages: Vec<String>,
    pub max_file_size_kb: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRequirements {
    pub complexity: ComplexityLevel,
    pub code_size_lines: usize,
    pub languages: Vec<String>,
    pub requires_reasoning: bool,
    pub requires_creativity: bool,
    pub urgency: UrgencyLevel,
    pub budget_constraint: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplexityLevel {
    Simple,
    Moderate,
    Complex,
    Expert,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UrgencyLevel {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone)]
pub struct ModelStrategyEngine {
    models: Vec<ModelProfile>,
    selection_history: Vec<ModelSelection>,
    performance_data: HashMap<String, ModelPerformance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelSelection {
    task_id: String,
    selected_model: String,
    requirements: TaskRequirements,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelPerformance {
    total_requests: u64,
    successful_requests: u64,
    avg_latency_ms: f64,
    total_cost: f64,
    user_satisfaction: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRecommendation {
    pub model: ModelProfile,
    pub confidence: f64,
    pub estimated_cost: f64,
    pub estimated_latency_ms: u64,
    pub reasoning: Vec<String>,
    pub alternatives: Vec<ModelProfile>,
}

impl ModelStrategyEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            models: Vec::new(),
            selection_history: Vec::new(),
            performance_data: HashMap::new(),
        };
        engine.register_default_models();
        engine
    }

    fn register_default_models(&mut self) {
        self.models.push(ModelProfile {
            id: "gpt-4o-mini".to_string(),
            name: "GPT-4o Mini".to_string(),
            tier: ModelTier::Fast,
            context_window: 128000,
            cost_per_1k_tokens: 0.15,
            avg_latency_ms: 500,
            strengths: vec!["fast".to_string(), "cheap".to_string(), "simple_tasks".to_string()],
            weaknesses: vec!["limited_reasoning".to_string()],
            supported_languages: vec!["rust".to_string(), "python".to_string(), "javascript".to_string()],
            max_file_size_kb: 100,
        });

        self.models.push(ModelProfile {
            id: "gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
            tier: ModelTier::Balanced,
            context_window: 128000,
            cost_per_1k_tokens: 2.50,
            avg_latency_ms: 2000,
            strengths: vec!["code_generation".to_string(), "debugging".to_string(), "refactoring".to_string()],
            weaknesses: vec!["complex_architecture".to_string()],
            supported_languages: vec!["rust".to_string(), "python".to_string(), "javascript".to_string(), "go".to_string(), "java".to_string()],
            max_file_size_kb: 500,
        });

        self.models.push(ModelProfile {
            id: "claude-3-5-sonnet".to_string(),
            name: "Claude 3.5 Sonnet".to_string(),
            tier: ModelTier::Powerful,
            context_window: 200000,
            cost_per_1k_tokens: 3.00,
            avg_latency_ms: 3000,
            strengths: vec!["complex_reasoning".to_string(), "architecture".to_string(), "large_context".to_string()],
            weaknesses: vec!["higher_cost".to_string()],
            supported_languages: vec!["rust".to_string(), "python".to_string(), "javascript".to_string(), "go".to_string(), "java".to_string(), "cpp".to_string()],
            max_file_size_kb: 1000,
        });

        self.models.push(ModelProfile {
            id: "o1-preview".to_string(),
            name: "o1-preview".to_string(),
            tier: ModelTier::Expert,
            context_window: 128000,
            cost_per_1k_tokens: 15.00,
            avg_latency_ms: 10000,
            strengths: vec!["expert_reasoning".to_string(), "complex_algorithms".to_string(), "system_design".to_string()],
            weaknesses: vec!["very_expensive".to_string(), "slow".to_string()],
            supported_languages: vec!["rust".to_string(), "python".to_string(), "javascript".to_string(), "go".to_string(), "java".to_string(), "cpp".to_string(), "haskell".to_string()],
            max_file_size_kb: 2000,
        });
    }

    pub fn select_model(&mut self, requirements: &TaskRequirements, task_id: &str) -> Result<ModelRecommendation> {
        let candidates = self.rank_models(requirements);
        
        if candidates.is_empty() {
            bail!("No suitable model found for requirements");
        }

        let best = candidates[0].clone();
        
        self.selection_history.push(ModelSelection {
            task_id: task_id.to_string(),
            selected_model: best.model.id.clone(),
            requirements: requirements.clone(),
            timestamp: chrono::Utc::now(),
        });

        Ok(ModelRecommendation {
            model: best.model,
            confidence: best.confidence,
            estimated_cost: best.estimated_cost,
            estimated_latency_ms: best.estimated_latency_ms,
            reasoning: best.reasoning,
            alternatives: candidates.iter().skip(1).take(2).map(|c| c.model.clone()).collect(),
        })
    }

    fn rank_models(&self, requirements: &TaskRequirements) -> Vec<ScoredModel> {
        let mut scored: Vec<ScoredModel> = self.models.iter()
            .filter(|m| self.is_compatible(m, requirements))
            .map(|m| self.score_model(m, requirements))
            .collect();

        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());
        scored
    }

    fn is_compatible(&self, model: &ModelProfile, requirements: &TaskRequirements) -> bool {
        // Check language support
        let language_match = requirements.languages.is_empty() || 
            requirements.languages.iter().all(|lang| model.supported_languages.contains(lang));

        // Check size constraint
        let size_ok = requirements.code_size_lines * 50 < model.max_file_size_kb * 1024; // Rough estimate

        // Check budget constraint
        let budget_ok = match requirements.budget_constraint {
            Some(budget) => {
                let estimated_cost = self.estimate_cost(model, requirements);
                estimated_cost <= budget
            }
            None => true,
        };

        language_match && size_ok && budget_ok
    }

    fn score_model(&self, model: &ModelProfile, requirements: &TaskRequirements) -> ScoredModel {
        let mut score = 0.0;
        let mut reasoning = Vec::new();

        // Complexity matching (30%)
        let complexity_score = match (requirements.complexity, &model.tier) {
            (ComplexityLevel::Simple, ModelTier::Fast) => {
                reasoning.push("Perfect match: simple task with fast model".to_string());
                1.0
            }
            (ComplexityLevel::Moderate, ModelTier::Balanced) => {
                reasoning.push("Good match: moderate task with balanced model".to_string());
                1.0
            }
            (ComplexityLevel::Complex, ModelTier::Powerful) => {
                reasoning.push("Good match: complex task with powerful model".to_string());
                1.0
            }
            (ComplexityLevel::Expert, ModelTier::Expert) => {
                reasoning.push("Perfect match: expert task with expert model".to_string());
                1.0
            }
            (ComplexityLevel::Simple, _) => {
                reasoning.push("Overkill: simple task with powerful model".to_string());
                0.6
            }
            _ => {
                reasoning.push("Underpowered: complex task may need stronger model".to_string());
                0.5
            }
        };
        score += complexity_score * 0.3;

        // Latency preference (20%)
        let latency_score = match requirements.urgency {
            UrgencyLevel::Critical => {
                if model.avg_latency_ms < 1000 { 1.0 } else { 0.3 }
            }
            UrgencyLevel::High => {
                if model.avg_latency_ms < 2000 { 1.0 } else { 0.5 }
            }
            _ => 0.8,
        };
        score += latency_score * 0.2;

        // Cost efficiency (20%)
        let cost_score = 1.0 - (model.cost_per_1k_tokens / 20.0).min(1.0);
        score += cost_score * 0.2;

        // Historical performance (20%)
        let perf_score = self.get_performance_score(&model.id);
        score += perf_score * 0.2;

        // Capability match (10%)
        let capability_score = if requirements.requires_reasoning && 
            model.strengths.contains(&"complex_reasoning".to_string()) {
            1.0
        } else if requirements.requires_creativity && 
            model.strengths.contains(&"code_generation".to_string()) {
            1.0
        } else {
            0.7
        };
        score += capability_score * 0.1;

        let estimated_cost = self.estimate_cost(model, requirements);
        let estimated_latency = model.avg_latency_ms;

        ScoredModel {
            model: model.clone(),
            total_score: score,
            confidence: score,
            estimated_cost,
            estimated_latency_ms: estimated_latency,
            reasoning,
        }
    }

    fn estimate_cost(&self, model: &ModelProfile, requirements: &TaskRequirements) -> f64 {
        let estimated_tokens = (requirements.code_size_lines * 50) as f64; // Rough estimate
        (estimated_tokens / 1000.0) * model.cost_per_1k_tokens
    }

    fn get_performance_score(&self, model_id: &str) -> f64 {
        match self.performance_data.get(model_id) {
            Some(perf) => {
                let success_rate = perf.successful_requests as f64 / perf.total_requests.max(1) as f64;
                let satisfaction = perf.user_satisfaction;
                (success_rate + satisfaction) / 2.0
            }
            None => 0.5, // Neutral score for unknown models
        }
    }

    pub fn record_performance(&mut self, model_id: &str, success: bool, cost: f64, latency_ms: u64) {
        let entry = self.performance_data.entry(model_id.to_string()).or_insert(ModelPerformance {
            total_requests: 0,
            successful_requests: 0,
            avg_latency_ms: 0.0,
            total_cost: 0.0,
            user_satisfaction: 0.5,
        });

        entry.total_requests += 1;
        if success {
            entry.successful_requests += 1;
        }
        entry.total_cost += cost;
        
        // Update rolling average
        entry.avg_latency_ms = (entry.avg_latency_ms * (entry.total_requests - 1) as f64 + latency_ms as f64) 
            / entry.total_requests as f64;
    }

    pub fn get_model_usage_stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        for selection in &self.selection_history {
            *stats.entry(selection.selected_model.clone()).or_insert(0) += 1;
        }
        stats
    }

    pub fn add_model(&mut self, model: ModelProfile) {
        self.models.push(model);
    }
}

#[derive(Debug, Clone)]
struct ScoredModel {
    model: ModelProfile,
    total_score: f64,
    confidence: f64,
    estimated_cost: f64,
    estimated_latency_ms: u64,
    reasoning: Vec<String>,
}

pub fn select_model_for_task(
    requirements: &TaskRequirements,
    task_id: &str,
) -> Result<ModelRecommendation> {
    let mut engine = ModelStrategyEngine::new();
    engine.select_model(requirements, task_id)
}

pub fn format_recommendation(rec: &ModelRecommendation) -> String {
    format!(
        r#"Model Recommendation
======================
Model: {} ({})
Confidence: {:.0}%
Estimated Cost: ${:.4}
Estimated Latency: {}ms

Reasoning:
{}

Alternatives: {}
"#,
        rec.model.name,
        rec.model.id,
        rec.confidence * 100.0,
        rec.estimated_cost,
        rec.estimated_latency_ms,
        rec.reasoning.iter().map(|r| format!("  - {}", r)).collect::<Vec<_>>().join("\n"),
        rec.alternatives.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selects_appropriate_model() {
        let mut engine = ModelStrategyEngine::new();
        let requirements = TaskRequirements {
            complexity: ComplexityLevel::Simple,
            code_size_lines: 50,
            languages: vec!["rust".to_string()],
            requires_reasoning: false,
            requires_creativity: false,
            urgency: UrgencyLevel::Normal,
            budget_constraint: Some(1.0),
        };

        let rec = engine.select_model(&requirements, "test-1").unwrap();
        assert_eq!(rec.model.tier, ModelTier::Fast);
    }

    #[test]
    fn test_expert_task_selects_expert_model() {
        let mut engine = ModelStrategyEngine::new();
        let requirements = TaskRequirements {
            complexity: ComplexityLevel::Expert,
            code_size_lines: 1000,
            languages: vec!["rust".to_string()],
            requires_reasoning: true,
            requires_creativity: true,
            urgency: UrgencyLevel::Low,
            budget_constraint: None,
        };

        let rec = engine.select_model(&requirements, "test-2").unwrap();
        assert_eq!(rec.model.tier, ModelTier::Expert);
    }

    #[test]
    fn test_budget_constraint_filters_models() {
        let mut engine = ModelStrategyEngine::new();
        let requirements = TaskRequirements {
            complexity: ComplexityLevel::Complex,
            code_size_lines: 500,
            languages: vec!["rust".to_string()],
            requires_reasoning: true,
            requires_creativity: false,
            urgency: UrgencyLevel::Normal,
            budget_constraint: Some(1.0), // Very low budget
        };

        let rec = engine.select_model(&requirements, "test-3").unwrap();
        assert!(rec.estimated_cost <= 1.0);
    }
}
