//! Scaling Engine - Issue #10
//! Multi-attempt repair scaling and resource allocation

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalingConfig {
    pub max_attempts: u32,
    pub initial_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub backoff_multiplier: f64,
    pub enable_parallel_attempts: bool,
    pub max_parallel_attempts: u32,
    pub resource_limits: ResourceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceLimits {
    pub max_tokens_per_attempt: u64,
    pub max_cost_per_attempt: f64,
    pub max_memory_mb: u64,
    pub max_disk_mb: u64,
}

#[derive(Debug, Clone)]
pub struct ScalingEngine {
    config: ScalingConfig,
    attempt_history: Vec<AttemptRecord>,
    current_attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AttemptRecord {
    attempt_number: u32,
    strategy: String,
    duration_ms: u64,
    tokens_used: u64,
    cost: f64,
    success: bool,
    error_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalingDecision {
    pub should_continue: bool,
    pub next_timeout_ms: u64,
    pub recommended_strategy: String,
    pub reason: String,
    pub resource_allocation: ResourceAllocation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceAllocation {
    pub token_budget: u64,
    pub cost_budget: f64,
    pub time_budget_ms: u64,
    pub priority: AttemptPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum AttemptPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl Default for ScalingConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_timeout_ms: 30000,
            max_timeout_ms: 300000,
            backoff_multiplier: 1.5,
            enable_parallel_attempts: false,
            max_parallel_attempts: 2,
            resource_limits: ResourceLimits {
                max_tokens_per_attempt: 100000,
                max_cost_per_attempt: 2.0,
                max_memory_mb: 512,
                max_disk_mb: 1024,
            },
        }
    }
}

impl ScalingEngine {
    pub fn new(config: ScalingConfig) -> Self {
        Self {
            config,
            attempt_history: Vec::new(),
            current_attempt: 0,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(ScalingConfig::default())
    }

    pub fn should_attempt_another(&self) -> bool {
        self.current_attempt < self.config.max_attempts
    }

    pub fn next_attempt(&mut self, previous_result: Option<AttemptResult>) -> ScalingDecision {
        self.current_attempt += 1;
        
        if let Some(result) = previous_result {
            self.attempt_history.push(AttemptRecord {
                attempt_number: self.current_attempt - 1,
                strategy: result.strategy.clone(),
                duration_ms: result.duration_ms,
                tokens_used: result.tokens_used,
                cost: result.cost,
                success: result.success,
                error_type: result.error_type.clone(),
            });
        }

        let should_continue = self.current_attempt <= self.config.max_attempts
            && !self.is_resource_exhausted();

        if !should_continue {
            return ScalingDecision {
                should_continue: false,
                next_timeout_ms: 0,
                recommended_strategy: "none".to_string(),
                reason: self.termination_reason(),
                resource_allocation: ResourceAllocation {
                    token_budget: 0,
                    cost_budget: 0.0,
                    time_budget_ms: 0,
                    priority: AttemptPriority::Low,
                },
            };
        }

        let next_timeout = self.calculate_next_timeout();
        let strategy = self.recommend_strategy();
        let allocation = self.allocate_resources();

        ScalingDecision {
            should_continue: true,
            next_timeout_ms: next_timeout,
            recommended_strategy: strategy,
            reason: format!("Attempt {} of {}", self.current_attempt, self.config.max_attempts),
            resource_allocation: allocation,
        }
    }

    fn calculate_next_timeout(&self) -> u64 {
        let base = self.config.initial_timeout_ms as f64;
        let multiplier = self.config.backoff_multiplier.powi(self.current_attempt as i32 - 1);
        let timeout = (base * multiplier) as u64;
        timeout.min(self.config.max_timeout_ms)
    }

    fn recommend_strategy(&self) -> String {
        // Analyze previous failures and recommend a strategy
        let failures: Vec<&AttemptRecord> = self.attempt_history
            .iter()
            .filter(|a| !a.success)
            .collect();

        if failures.is_empty() {
            return "standard".to_string();
        }

        // Check for patterns
        let syntax_errors = failures.iter().filter(|f| {
            f.error_type.as_ref().map(|e| e.contains("syntax")).unwrap_or(false)
        }).count();

        let test_failures = failures.iter().filter(|f| {
            f.error_type.as_ref().map(|e| e.contains("test")).unwrap_or(false)
        }).count();

        if syntax_errors > 1 {
            "conservative".to_string()  // Be more careful with edits
        } else if test_failures > 1 {
            "test_focused".to_string()  // Focus on test fixes
        } else if self.current_attempt > 3 {
            "aggressive".to_string()  // Try more comprehensive changes
        } else {
            "adaptive".to_string()  // Adjust based on feedback
        }
    }

    fn allocate_resources(&self) -> ResourceAllocation {
        // Increase resources as attempts progress
        let attempt_factor = (self.current_attempt as f64).sqrt();
        
        let token_budget = (self.config.resource_limits.max_tokens_per_attempt as f64 * 
            (1.0 + (attempt_factor - 1.0) * 0.3)) as u64;
        
        let cost_budget = self.config.resource_limits.max_cost_per_attempt * 
            (1.0 + (attempt_factor - 1.0) * 0.2);
        
        let time_budget = self.calculate_next_timeout();

        let priority = match self.current_attempt {
            1 => AttemptPriority::Low,
            2 | 3 => AttemptPriority::Normal,
            4 => AttemptPriority::High,
            _ => AttemptPriority::Critical,
        };

        ResourceAllocation {
            token_budget,
            cost_budget,
            time_budget_ms: time_budget,
            priority,
        }
    }

    fn is_resource_exhausted(&self) -> bool {
        let total_cost: f64 = self.attempt_history.iter().map(|a| a.cost).sum();
        let total_tokens: u64 = self.attempt_history.iter().map(|a| a.tokens_used).sum();

        total_cost > self.config.resource_limits.max_cost_per_attempt * 3.0
            || total_tokens > self.config.resource_limits.max_tokens_per_attempt * 3
    }

    fn termination_reason(&self) -> String {
        if self.current_attempt > self.config.max_attempts {
            format!("Maximum attempts ({}) reached", self.config.max_attempts)
        } else if self.is_resource_exhausted() {
            "Resource budget exhausted".to_string()
        } else {
            "Unknown termination condition".to_string()
        }
    }

    pub fn get_attempt_history(&self) -> &[AttemptRecord] {
        &self.attempt_history
    }

    pub fn get_current_attempt(&self) -> u32 {
        self.current_attempt
    }

    pub fn get_success_rate(&self) -> f64 {
        if self.attempt_history.is_empty() {
            0.0
        } else {
            let successes = self.attempt_history.iter().filter(|a| a.success).count();
            successes as f64 / self.attempt_history.len() as f64
        }
    }

    pub fn estimate_remaining_cost(&self) -> f64 {
        let avg_cost = if self.attempt_history.is_empty() {
            self.config.resource_limits.max_cost_per_attempt * 0.5
        } else {
            self.attempt_history.iter().map(|a| a.cost).sum::<f64>() / self.attempt_history.len() as f64
        };

        let remaining = self.config.max_attempts - self.current_attempt;
        avg_cost * remaining as f64
    }
}

#[derive(Debug, Clone)]
pub struct AttemptResult {
    pub strategy: String,
    pub duration_ms: u64,
    pub tokens_used: u64,
    pub cost: f64,
    pub success: bool,
    pub error_type: Option<String>,
}

pub fn create_scaling_engine(config: Option<ScalingConfig>) -> ScalingEngine {
    match config {
        Some(c) => ScalingEngine::new(c),
        None => ScalingEngine::with_defaults(),
    }
}

pub fn format_scaling_report(engine: &ScalingEngine) -> String {
    let history = engine.get_attempt_history();
    let success_rate = engine.get_success_rate();
    let remaining_cost = engine.estimate_remaining_cost();

    format!(
        r#"Scaling Report
===============
Current Attempt: {}
Success Rate: {:.1}%
Total Attempts: {}
Estimated Remaining Cost: ${:.2}
"#,
        engine.get_current_attempt(),
        success_rate * 100.0,
        history.len(),
        remaining_cost
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scaling_engine_increases_timeout() {
        let mut engine = ScalingEngine::with_defaults();
        
        let decision1 = engine.next_attempt(None);
        let decision2 = engine.next_attempt(Some(AttemptResult {
            strategy: "test".to_string(),
            duration_ms: 1000,
            tokens_used: 1000,
            cost: 0.1,
            success: false,
            error_type: None,
        }));
        
        assert!(decision2.next_timeout_ms > decision1.next_timeout_ms);
    }

    #[test]
    fn test_resource_allocation_increases() {
        let mut engine = ScalingEngine::with_defaults();
        
        let decision1 = engine.next_attempt(None);
        let _ = engine.next_attempt(Some(AttemptResult {
            strategy: "test".to_string(),
            duration_ms: 1000,
            tokens_used: 1000,
            cost: 0.1,
            success: false,
            error_type: None,
        }));
        let decision2 = engine.next_attempt(None);
        
        assert!(decision2.resource_allocation.token_budget >= decision1.resource_allocation.token_budget);
    }

    #[test]
    fn test_max_attempts_terminates() {
        let mut config = ScalingConfig::default();
        config.max_attempts = 2;
        let mut engine = ScalingEngine::new(config);
        
        let _ = engine.next_attempt(None);
        let decision = engine.next_attempt(None);
        
        assert!(!decision.should_continue);
    }
}
