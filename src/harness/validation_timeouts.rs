//! P2-Issue2: Configurable validation timeouts per category
//!
//! This module provides comprehensive timeout configuration for validation
//! commands with per-category settings, adaptive scaling, and escalation strategies.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

/// P2-Issue2: Configurable validation timeout per category
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationTimeoutConfig {
    /// Timeout configuration per category
    pub category_timeouts: HashMap<ValidationCategory, CategoryTimeout>,
    /// Default timeout for unspecified categories
    pub default_timeout: u64,
    /// Global timeout override
    pub global_timeout: Option<u64>,
    /// Timeout escalation strategy
    pub escalation_strategy: TimeoutEscalationStrategy,
    /// Adaptive timeout configuration
    pub adaptive_config: AdaptiveTimeoutConfig,
    /// Backoff multiplier for exponential escalation
    pub backoff_multiplier: f64,
}

/// P2-Issue2: Category-specific timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryTimeout {
    /// Base timeout in milliseconds
    pub base_timeout_ms: u64,
    /// Maximum timeout in milliseconds
    pub max_timeout_ms: u64,
    /// Timeout scaling factor based on file count
    pub file_count_factor: f64,
    /// Timeout scaling factor based on file size
    pub file_size_factor: f64,
    /// Whether to use adaptive timeout
    pub adaptive: bool,
    /// Timeout escalation steps
    pub escalation_steps: Vec<TimeoutStep>,
}

/// P2-Issue2: Timeout escalation step
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeoutStep {
    /// Step number
    pub step: u32,
    /// Timeout multiplier for this step
    pub multiplier: f64,
    /// Conditions for this step
    pub conditions: Vec<TimeoutCondition>,
    /// Maximum attempts at this step
    pub max_attempts: u32,
}

/// P2-Issue2: Timeout conditions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeoutCondition {
    /// Always apply this step
    Always,
    /// Apply if previous attempt timed out
    PreviousTimeout,
    /// Apply if error rate exceeds threshold
    ErrorRateExceeds(f64),
    /// Apply if file count exceeds threshold
    FileCountExceeds(u32),
    /// Apply if total file size exceeds threshold (MB)
    FileSizeExceeds(f64),
}

/// P2-Issue2: Timeout escalation strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeoutEscalationStrategy {
    /// No escalation
    None,
    /// Linear escalation
    Linear,
    /// Exponential escalation
    Exponential,
    /// Adaptive escalation based on history
    Adaptive,
    /// Custom escalation function
    Custom,
}

/// P2-Issue2: Adaptive timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdaptiveTimeoutConfig {
    /// Enable adaptive timeouts
    pub enabled: bool,
    /// Historical data window size
    pub history_window: usize,
    /// Minimum samples for adaptation
    pub min_samples: usize,
    /// Timeout adjustment factor
    pub adjustment_factor: f64,
    /// Maximum adjustment percentage
    pub max_adjustment_percent: f64,
    /// Performance targets
    pub performance_targets: PerformanceTargets,
}

/// P2-Issue2: Performance targets for adaptive timeouts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceTargets {
    /// Target success rate
    pub target_success_rate: f64,
    /// Target average duration percentage of timeout
    pub target_duration_percent: f64,
    /// Maximum acceptable timeout rate
    pub max_timeout_rate: f64,
}

/// P2-Issue2: Timeout execution result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeoutResult {
    /// Applied timeout in milliseconds
    pub applied_timeout_ms: u64,
    /// Actual execution time in milliseconds
    pub actual_duration_ms: u64,
    /// Whether execution timed out
    pub timed_out: bool,
    /// Number of escalation steps used
    pub escalation_steps_used: u32,
    /// Category timeout used
    pub category_timeout: CategoryTimeout,
    /// Adaptive adjustments made
    pub adaptive_adjustments: Vec<AdaptiveAdjustment>,
}

/// P2-Issue2: Adaptive adjustment information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdaptiveAdjustment {
    /// Adjustment type
    pub adjustment_type: AdjustmentType,
    /// Original timeout
    pub original_timeout_ms: u64,
    /// Adjusted timeout
    pub adjusted_timeout_ms: u64,
    /// Reason for adjustment
    pub reason: String,
    /// Confidence level
    pub confidence: f64,
}

/// P2-Issue2: Adjustment types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdjustmentType {
    /// Increase timeout
    Increase,
    /// Decrease timeout
    Decrease,
    /// No change
    NoChange,
}

/// P2-Issue2: Timeout history for adaptive learning
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeoutHistory {
    /// Category
    pub category: ValidationCategory,
    /// Historical timeout results
    pub results: Vec<TimeoutResult>,
    /// Performance statistics
    pub statistics: TimeoutStatistics,
    /// Last updated
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// P2-Issue2: Timeout statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeoutStatistics {
    /// Total executions
    pub total_executions: u64,
    /// Successful executions
    pub successful_executions: u64,
    /// Timed out executions
    pub timed_out_executions: u64,
    /// Average execution time
    pub avg_execution_time_ms: f64,
    /// Average applied timeout
    pub avg_applied_timeout_ms: f64,
    /// Timeout rate
    pub timeout_rate: f64,
    /// Success rate
    pub success_rate: f64,
}

/// P2-Issue2: Timeout manager
pub struct TimeoutManager {
    config: ValidationTimeoutConfig,
    history: HashMap<ValidationCategory, TimeoutHistory>,
    adaptive_engine: AdaptiveTimeoutEngine,
}

impl Default for ValidationTimeoutConfig {
    fn default() -> Self {
        let mut category_timeouts = HashMap::new();
        
        // Format commands - typically fast
        category_timeouts.insert(ValidationCategory::Format, CategoryTimeout {
            base_timeout_ms: 30000,  // 30 seconds
            max_timeout_ms: 120000,  // 2 minutes
            file_count_factor: 100.0,  // 100ms per file
            file_size_factor: 0.1,    // 0.1ms per KB
            adaptive: true,
            escalation_steps: vec![
                TimeoutStep {
                    step: 1,
                    multiplier: 1.0,
                    conditions: vec![TimeoutCondition::Always],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 2,
                    multiplier: 2.0,
                    conditions: vec![TimeoutCondition::PreviousTimeout],
                    max_attempts: 1,
                },
            ],
        });
        
        // Lint commands - moderate duration
        category_timeouts.insert(ValidationCategory::Lint, CategoryTimeout {
            base_timeout_ms: 60000,  // 1 minute
            max_timeout_ms: 300000,  // 5 minutes
            file_count_factor: 200.0,  // 200ms per file
            file_size_factor: 0.2,    // 0.2ms per KB
            adaptive: true,
            escalation_steps: vec![
                TimeoutStep {
                    step: 1,
                    multiplier: 1.0,
                    conditions: vec![TimeoutCondition::Always],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 2,
                    multiplier: 1.5,
                    conditions: vec![TimeoutCondition::PreviousTimeout],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 3,
                    multiplier: 2.0,
                    conditions: vec![TimeoutCondition::ErrorRateExceeds(0.3)],
                    max_attempts: 1,
                },
            ],
        });
        
        // Test commands - can be very slow
        category_timeouts.insert(ValidationCategory::Test, CategoryTimeout {
            base_timeout_ms: 300000,  // 5 minutes
            max_timeout_ms: 1800000, // 30 minutes
            file_count_factor: 1000.0, // 1s per test file
            file_size_factor: 1.0,    // 1ms per KB
            adaptive: true,
            escalation_steps: vec![
                TimeoutStep {
                    step: 1,
                    multiplier: 1.0,
                    conditions: vec![TimeoutCondition::Always],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 2,
                    multiplier: 2.0,
                    conditions: vec![TimeoutCondition::PreviousTimeout],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 3,
                    multiplier: 3.0,
                    conditions: vec![TimeoutCondition::FileCountExceeds(100)],
                    max_attempts: 1,
                },
            ],
        });
        
        // Reproducibility commands - moderate to slow
        category_timeouts.insert(ValidationCategory::Repro, CategoryTimeout {
            base_timeout_ms: 120000,  // 2 minutes
            max_timeout_ms: 600000,  // 10 minutes
            file_count_factor: 500.0,  // 500ms per file
            file_size_factor: 0.5,    // 0.5ms per KB
            adaptive: true,
            escalation_steps: vec![
                TimeoutStep {
                    step: 1,
                    multiplier: 1.0,
                    conditions: vec![TimeoutCondition::Always],
                    max_attempts: 1,
                },
                TimeoutStep {
                    step: 2,
                    multiplier: 1.5,
                    conditions: vec![TimeoutCondition::PreviousTimeout],
                    max_attempts: 1,
                },
            ],
        });
        
        Self {
            category_timeouts,
            default_timeout: 60000, // 1 minute default
            global_timeout: None,
            escalation_strategy: TimeoutEscalationStrategy::Adaptive,
            adaptive_config: AdaptiveTimeoutConfig {
                enabled: true,
                history_window: 100,
                min_samples: 10,
                adjustment_factor: 0.1,
                max_adjustment_percent: 0.5,
                performance_targets: PerformanceTargets {
                    target_success_rate: 0.95,
                    target_duration_percent: 0.8,
                    max_timeout_rate: 0.05,
                },
            },
            backoff_multiplier: 2.0,
        }
    }
}

impl TimeoutManager {
    /// Create new timeout manager
    pub fn new() -> Self {
        Self::with_config(ValidationTimeoutConfig::default())
    }
    
    /// Create timeout manager with custom config
    pub fn with_config(config: ValidationTimeoutConfig) -> Self {
        Self {
            adaptive_engine: AdaptiveTimeoutEngine::new(&config.adaptive_config),
            config,
            history: HashMap::new(),
        }
    }
    
    /// Calculate timeout for a validation category
    pub fn calculate_timeout(
        &mut self,
        category: ValidationCategory,
        file_count: usize,
        total_file_size_kb: f64,
        attempt_number: u32,
    ) -> TimeoutResult {
        let category_timeout = self.config.category_timeouts
            .get(&category)
            .cloned()
            .unwrap_or_else(|| CategoryTimeout {
                base_timeout_ms: self.config.default_timeout,
                max_timeout_ms: self.config.default_timeout * 4,
                file_count_factor: 500.0,
                file_size_factor: 1.0,
                adaptive: false,
                escalation_steps: vec![
                    TimeoutStep {
                        step: 1,
                        multiplier: 1.0,
                        conditions: vec![TimeoutCondition::Always],
                        max_attempts: 1,
                    },
                ],
            });
        
        // Calculate base timeout
        let mut base_timeout = category_timeout.base_timeout_ms;
        
        // Apply file count scaling
        base_timeout += (file_count as f64 * category_timeout.file_count_factor) as u64;
        
        // Apply file size scaling
        base_timeout += (total_file_size_kb * category_timeout.file_size_factor) as u64;
        
        // Apply escalation based on attempt number
        let escalation_multiplier = self.get_escalation_multiplier(&category_timeout, attempt_number);
        let mut escalated_timeout = (base_timeout as f64 * escalation_multiplier) as u64;
        
        // Apply adaptive adjustments if enabled
        let adaptive_adjustments = if category_timeout.adaptive && self.config.adaptive_config.enabled {
            self.apply_adaptive_adjustments(category, escalated_timeout, file_count, total_file_size_kb)
        } else {
            vec![]
        };
        
        // Apply adaptive adjustments
        for adjustment in &adaptive_adjustments {
            escalated_timeout = adjustment.adjusted_timeout_ms;
        }
        
        // Enforce maximum timeout
        let final_timeout = escalated_timeout.min(category_timeout.max_timeout_ms);
        
        // Apply global timeout if set
        let applied_timeout = if let Some(global_timeout) = self.config.global_timeout {
            final_timeout.min(global_timeout)
        } else {
            final_timeout
        };
        
        TimeoutResult {
            applied_timeout_ms: applied_timeout,
            actual_duration_ms: 0, // Will be set after execution
            timed_out: false,       // Will be set after execution
            escalation_steps_used: attempt_number,
            category_timeout,
            adaptive_adjustments,
        }
    }
    
    /// Get escalation multiplier based on attempt number
    fn get_escalation_multiplier(&self, category_timeout: &CategoryTimeout, attempt_number: u32) -> f64 {
        for step in &category_timeout.escalation_steps {
            if attempt_number == step.step
                && self.should_apply_escalation_step(category_timeout, step, attempt_number)
            {
                return step.multiplier;
            }
        }
        
        // Default escalation based on strategy
        match self.config.escalation_strategy {
            TimeoutEscalationStrategy::None => 1.0,
            TimeoutEscalationStrategy::Linear => 1.0 + (attempt_number as f64 - 1.0) * 0.5,
            TimeoutEscalationStrategy::Exponential => self.config.backoff_multiplier.powi(attempt_number as i32 - 1),
            TimeoutEscalationStrategy::Adaptive => self.adaptive_engine.get_adaptive_multiplier(attempt_number),
            TimeoutEscalationStrategy::Custom => 1.0, // Would use custom function
        }
    }
    
    /// Check if escalation step should be applied
    fn should_apply_escalation_step(
        &self,
        category_timeout: &CategoryTimeout,
        step: &TimeoutStep,
        attempt_number: u32,
    ) -> bool {
        if attempt_number > step.max_attempts {
            return false;
        }
        
        for condition in &step.conditions {
            if !self.evaluate_timeout_condition(category_timeout, condition, attempt_number) {
                return false;
            }
        }
        
        true
    }
    
    /// Evaluate timeout condition
    fn evaluate_timeout_condition(
        &self,
        category_timeout: &CategoryTimeout,
        condition: &TimeoutCondition,
        attempt_number: u32,
    ) -> bool {
        match condition {
            TimeoutCondition::Always => true,
            TimeoutCondition::PreviousTimeout => {
                // Check if previous attempt timed out
                if let Some(history) = self.history.get(&category_timeout_to_category(category_timeout, &self.config)) {
                    if let Some(last_result) = history.results.last() {
                        return last_result.timed_out;
                    }
                }
                false
            }
            TimeoutCondition::ErrorRateExceeds(threshold) => {
                // Check error rate in recent history
                if let Some(history) = self.history.get(&category_timeout_to_category(category_timeout, &self.config)) {
                    return history.statistics.timeout_rate > *threshold;
                }
                false
            }
            TimeoutCondition::FileCountExceeds(threshold) => {
                // Check if current file count exceeds threshold
                self.get_current_file_count().map_or(false, |count| count as u32 > *threshold)
            }
            TimeoutCondition::FileSizeExceeds(threshold) => {
                // Threshold is MB, computed size is MB.
                self.get_current_file_size_mb()
                    .map_or(false, |size_mb| size_mb > *threshold)
            }
        }
    }
    
    /// Get current file count for timeout evaluation
    fn get_current_file_count(&self) -> Result<usize> {
        let mut count = 0usize;
        for entry in walkdir::WalkDir::new(".")
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if entry.file_type().is_file() && !is_ignored_path(entry.path()) {
                count += 1;
            }
        }
        Ok(count)
    }
    
    /// Get current file size for timeout evaluation
    fn get_current_file_size_mb(&self) -> Result<f64> {
        let mut total_bytes = 0u64;
        for entry in walkdir::WalkDir::new(".")
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if entry.file_type().is_file() && !is_ignored_path(entry.path()) {
                if let Ok(metadata) = entry.metadata() {
                    total_bytes = total_bytes.saturating_add(metadata.len());
                }
            }
        }
        Ok(total_bytes as f64 / (1024.0 * 1024.0))
    }
    
    /// Apply adaptive adjustments
    fn apply_adaptive_adjustments(
        &mut self,
        category: ValidationCategory,
        timeout: u64,
        file_count: usize,
        total_file_size_kb: f64,
    ) -> Vec<AdaptiveAdjustment> {
        let history = self.history.entry(category).or_insert_with(|| TimeoutHistory {
            category,
            results: Vec::new(),
            statistics: TimeoutStatistics::default(),
            last_updated: chrono::Utc::now(),
        });
        
        self.adaptive_engine.calculate_adjustments(history, timeout, file_count, total_file_size_kb)
    }
    
    /// Record timeout execution result
    pub fn record_execution_result(&mut self, category: ValidationCategory, mut result: TimeoutResult) {
        let history = self.history.entry(category).or_insert_with(|| TimeoutHistory {
            category,
            results: Vec::new(),
            statistics: TimeoutStatistics::default(),
            last_updated: chrono::Utc::now(),
        });
        
        // Update result with execution data
        result.timed_out = result.actual_duration_ms > result.applied_timeout_ms;
        
        // Add to history
        history.results.push(result.clone());
        
        // Update statistics
        self.update_statistics(history);
        
        // Update adaptive engine
        self.adaptive_engine.update_with_result(&history);
        
        // Trim history if needed
        if history.results.len() > self.config.adaptive_config.history_window {
            history.results.remove(0);
        }
        
        history.last_updated = chrono::Utc::now();
    }
    
    /// Update timeout statistics
    fn update_statistics(&self, history: &mut TimeoutHistory) {
        let total = history.results.len() as u64;
        if total == 0 {
            return;
        }
        
        let successful = history.results.iter().filter(|r| !r.timed_out).count() as u64;
        let timed_out = total - successful;
        
        let total_duration: u64 = history.results.iter().map(|r| r.actual_duration_ms).sum();
        let total_timeout: u64 = history.results.iter().map(|r| r.applied_timeout_ms).sum();
        
        history.statistics = TimeoutStatistics {
            total_executions: total,
            successful_executions: successful,
            timed_out_executions: timed_out,
            avg_execution_time_ms: total_duration as f64 / total as f64,
            avg_applied_timeout_ms: total_timeout as f64 / total as f64,
            timeout_rate: timed_out as f64 / total as f64,
            success_rate: successful as f64 / total as f64,
        };
    }
    
    /// Get timeout statistics for all categories
    pub fn get_statistics(&self) -> HashMap<ValidationCategory, &TimeoutStatistics> {
        self.history
            .iter()
            .map(|(category, history)| (*category, &history.statistics))
            .collect()
    }
    
    /// Get timeout history for a category
    pub fn get_history(&self, category: ValidationCategory) -> Option<&TimeoutHistory> {
        self.history.get(&category)
    }
    
    /// Reset timeout history for a category
    pub fn reset_history(&mut self, category: ValidationCategory) {
        if let Some(history) = self.history.get_mut(&category) {
            history.results.clear();
            history.statistics = TimeoutStatistics::default();
            history.last_updated = chrono::Utc::now();
        }
    }
    
    /// Reset all timeout history
    pub fn reset_all_history(&mut self) {
        for history in self.history.values_mut() {
            history.results.clear();
            history.statistics = TimeoutStatistics::default();
            history.last_updated = chrono::Utc::now();
        }
    }
}

fn category_timeout_to_category(
    category_timeout: &CategoryTimeout,
    config: &ValidationTimeoutConfig,
) -> ValidationCategory {
    config
        .category_timeouts
        .iter()
        .find_map(|(category, timeout)| (timeout == category_timeout).then_some(*category))
        .unwrap_or(ValidationCategory::Format)
}

fn is_ignored_path(path: &Path) -> bool {
    path.components().any(|component| {
        let part = component.as_os_str().to_string_lossy();
        matches!(part.as_ref(), ".git" | "target" | "node_modules" | ".cargo")
    })
}

/// P2-Issue2: Adaptive timeout engine
pub struct AdaptiveTimeoutEngine {
    config: AdaptiveTimeoutConfig,
    learned_multiplier: f64,
}

impl AdaptiveTimeoutEngine {
    /// Create new adaptive timeout engine
    pub fn new(config: &AdaptiveTimeoutConfig) -> Self {
        Self {
            config: config.clone(),
            learned_multiplier: 1.0,
        }
    }
    
    /// Calculate adaptive adjustments
    pub fn calculate_adjustments(
        &self,
        history: &TimeoutHistory,
        current_timeout: u64,
        _file_count: usize,
        _total_file_size_kb: f64,
    ) -> Vec<AdaptiveAdjustment> {
        let mut adjustments = Vec::new();
        
        if history.results.len() < self.config.min_samples {
            return adjustments;
        }
        
        let stats = &history.statistics;
        let targets = &self.config.performance_targets;
        
        // Check if timeout rate is too high
        if stats.timeout_rate > targets.max_timeout_rate {
            let adjustment_factor = (stats.timeout_rate - targets.max_timeout_rate) * self.config.adjustment_factor;
            let new_timeout = (current_timeout as f64 * (1.0 + adjustment_factor)) as u64;
            
            adjustments.push(AdaptiveAdjustment {
                adjustment_type: AdjustmentType::Increase,
                original_timeout_ms: current_timeout,
                adjusted_timeout_ms: new_timeout,
                reason: format!("High timeout rate: {:.2}%", stats.timeout_rate * 100.0),
                confidence: stats.success_rate,
            });
        }
        
        // Check if average duration is too close to timeout
        if stats.avg_execution_time_ms > stats.avg_applied_timeout_ms * targets.target_duration_percent {
            let adjustment_factor = (stats.avg_execution_time_ms / stats.avg_applied_timeout_ms - targets.target_duration_percent) * self.config.adjustment_factor;
            let new_timeout = (current_timeout as f64 * (1.0 + adjustment_factor)) as u64;
            
            adjustments.push(AdaptiveAdjustment {
                adjustment_type: AdjustmentType::Increase,
                original_timeout_ms: current_timeout,
                adjusted_timeout_ms: new_timeout,
                reason: format!("Average duration too close to timeout: {:.2}%", (stats.avg_execution_time_ms / stats.avg_applied_timeout_ms) * 100.0),
                confidence: stats.success_rate,
            });
        }
        
        // Check if timeout is too generous (success rate is high and duration is much less than timeout)
        if stats.success_rate > targets.target_success_rate && 
           stats.avg_execution_time_ms < stats.avg_applied_timeout_ms * (targets.target_duration_percent - 0.2) {
            let adjustment_factor = (targets.target_duration_percent - (stats.avg_execution_time_ms / stats.avg_applied_timeout_ms)) * self.config.adjustment_factor;
            let new_timeout = (current_timeout as f64 * (1.0 - adjustment_factor)) as u64;
            
            adjustments.push(AdaptiveAdjustment {
                adjustment_type: AdjustmentType::Decrease,
                original_timeout_ms: current_timeout,
                adjusted_timeout_ms: new_timeout,
                reason: format!("Timeout too generous: success rate {:.2}%, duration {:.2}% of timeout", 
                    stats.success_rate * 100.0, (stats.avg_execution_time_ms / stats.avg_applied_timeout_ms) * 100.0),
                confidence: stats.success_rate,
            });
        }
        
        adjustments
    }
    
    /// Update adaptive engine with new result
    pub fn update_with_result(&mut self, history: &TimeoutHistory) {
        if history.results.len() < self.config.min_samples {
            return;
        }

        let stats = &history.statistics;
        let targets = &self.config.performance_targets;
        let mut adjustment = 0.0f64;

        if stats.timeout_rate > targets.max_timeout_rate {
            adjustment += (stats.timeout_rate - targets.max_timeout_rate) * self.config.adjustment_factor;
        }

        if stats.avg_applied_timeout_ms > 0.0 {
            let utilization = stats.avg_execution_time_ms / stats.avg_applied_timeout_ms;
            if utilization > targets.target_duration_percent {
                adjustment += (utilization - targets.target_duration_percent) * self.config.adjustment_factor;
            } else if stats.success_rate > targets.target_success_rate
                && utilization < (targets.target_duration_percent - 0.2).max(0.0)
            {
                adjustment -=
                    ((targets.target_duration_percent - 0.2) - utilization) * self.config.adjustment_factor;
            }
        }

        let clamp = self.config.max_adjustment_percent.max(0.01);
        let bounded_adjustment = adjustment.clamp(-clamp, clamp);
        self.learned_multiplier = (self.learned_multiplier * (1.0 + bounded_adjustment)).clamp(0.5, 3.0);
    }
    
    /// Get adaptive multiplier for attempt number
    pub fn get_adaptive_multiplier(&self, attempt_number: u32) -> f64 {
        let base = match attempt_number {
            1 => 1.0,
            2 => 1.5,
            3 => 2.0,
            _ => 2.5,
        };
        (base * self.learned_multiplier).clamp(1.0, 5.0)
    }
}

impl Default for TimeoutStatistics {
    fn default() -> Self {
        Self {
            total_executions: 0,
            successful_executions: 0,
            timed_out_executions: 0,
            avg_execution_time_ms: 0.0,
            avg_applied_timeout_ms: 0.0,
            timeout_rate: 0.0,
            success_rate: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_engine_increases_multiplier_when_timeout_rate_high() {
        let config = ValidationTimeoutConfig::default().adaptive_config;
        let mut engine = AdaptiveTimeoutEngine::new(&config);
        let history = TimeoutHistory {
            category: ValidationCategory::Format,
            results: vec![
                TimeoutResult {
                    applied_timeout_ms: 1000,
                    actual_duration_ms: 1500,
                    timed_out: true,
                    escalation_steps_used: 1,
                    category_timeout: CategoryTimeout {
                        base_timeout_ms: 1000,
                        max_timeout_ms: 5000,
                        file_count_factor: 0.0,
                        file_size_factor: 0.0,
                        adaptive: true,
                        escalation_steps: vec![],
                    },
                    adaptive_adjustments: vec![],
                };
                config.min_samples
            ],
            statistics: TimeoutStatistics {
                total_executions: config.min_samples as u64,
                successful_executions: 1,
                timed_out_executions: (config.min_samples as u64).saturating_sub(1),
                avg_execution_time_ms: 950.0,
                avg_applied_timeout_ms: 1000.0,
                timeout_rate: 0.5,
                success_rate: 0.5,
            },
            last_updated: chrono::Utc::now(),
        };

        engine.update_with_result(&history);
        assert!(engine.get_adaptive_multiplier(2) > 1.5);
    }

    #[test]
    fn adaptive_engine_decreases_multiplier_when_timeout_generous() {
        let config = ValidationTimeoutConfig::default().adaptive_config;
        let mut engine = AdaptiveTimeoutEngine::new(&config);
        let history = TimeoutHistory {
            category: ValidationCategory::Format,
            results: vec![
                TimeoutResult {
                    applied_timeout_ms: 1000,
                    actual_duration_ms: 100,
                    timed_out: false,
                    escalation_steps_used: 1,
                    category_timeout: CategoryTimeout {
                        base_timeout_ms: 1000,
                        max_timeout_ms: 5000,
                        file_count_factor: 0.0,
                        file_size_factor: 0.0,
                        adaptive: true,
                        escalation_steps: vec![],
                    },
                    adaptive_adjustments: vec![],
                };
                config.min_samples
            ],
            statistics: TimeoutStatistics {
                total_executions: config.min_samples as u64,
                successful_executions: config.min_samples as u64,
                timed_out_executions: 0,
                avg_execution_time_ms: 100.0,
                avg_applied_timeout_ms: 1000.0,
                timeout_rate: 0.0,
                success_rate: 1.0,
            },
            last_updated: chrono::Utc::now(),
        };

        engine.update_with_result(&history);
        assert!(engine.get_adaptive_multiplier(2) < 1.5);
    }
}
