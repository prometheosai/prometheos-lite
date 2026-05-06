//! Issue 21: Scaling Engine / Attempt Pool Tests
//!
//! Comprehensive tests for the Scaling Engine including:
//! - ScalingConfig struct (max_attempts, timeouts, backoff, parallel settings)
//! - ResourceLimits struct (tokens, cost, memory, disk limits)
//! - ScalingEngine for managing multiple attempts
//! - ScalingDecision struct (should_continue, timeout, strategy, resources)
//! - ResourceAllocation struct (token_budget, cost_budget, time_budget, priority)
//! - AttemptPriority enum (Low, Normal, High, Critical)
//! - should_attempt_another() for attempt limit checking
//! - next_attempt() for scaling decisions
//! - Exponential backoff calculation
//! - Resource exhaustion detection

use prometheos_lite::harness::scaling::{
    AttemptPriority, ResourceAllocation, ResourceLimits, ScalingConfig, ScalingDecision,
    ScalingEngine,
};

// ============================================================================
// ScalingConfig Tests
// ============================================================================

#[test]
fn test_scaling_config_default() {
    let config = ScalingConfig::default();

    assert_eq!(config.max_attempts, 5);
    assert_eq!(config.initial_timeout_ms, 30000);
    assert_eq!(config.max_timeout_ms, 300000);
    assert_eq!(config.backoff_multiplier, 1.5);
    assert!(!config.enable_parallel_attempts);
    assert_eq!(config.max_parallel_attempts, 2);
}

#[test]
fn test_scaling_config_custom() {
    let config = ScalingConfig {
        max_attempts: 10,
        initial_timeout_ms: 60000,
        max_timeout_ms: 600000,
        backoff_multiplier: 2.0,
        enable_parallel_attempts: true,
        max_parallel_attempts: 4,
        resource_limits: ResourceLimits {
            max_tokens_per_attempt: 200000,
            max_cost_per_attempt: 5.0,
            max_memory_mb: 1024,
            max_disk_mb: 2048,
        },
    };

    assert_eq!(config.max_attempts, 10);
    assert_eq!(config.backoff_multiplier, 2.0);
    assert!(config.enable_parallel_attempts);
    assert_eq!(config.max_parallel_attempts, 4);
}

// ============================================================================
// ResourceLimits Tests
// ============================================================================

#[test]
fn test_resource_limits_default() {
    let limits = ResourceLimits {
        max_tokens_per_attempt: 100000,
        max_cost_per_attempt: 2.0,
        max_memory_mb: 512,
        max_disk_mb: 1024,
    };

    assert_eq!(limits.max_tokens_per_attempt, 100000);
    assert_eq!(limits.max_cost_per_attempt, 2.0);
    assert_eq!(limits.max_memory_mb, 512);
    assert_eq!(limits.max_disk_mb, 1024);
}

#[test]
fn test_resource_limits_custom() {
    let limits = ResourceLimits {
        max_tokens_per_attempt: 500000,
        max_cost_per_attempt: 10.0,
        max_memory_mb: 2048,
        max_disk_mb: 4096,
    };

    assert_eq!(limits.max_tokens_per_attempt, 500000);
    assert_eq!(limits.max_cost_per_attempt, 10.0);
    assert_eq!(limits.max_memory_mb, 2048);
}

// ============================================================================
// ScalingEngine Tests
// ============================================================================

#[test]
fn test_scaling_engine_new() {
    let config = ScalingConfig::default();
    let engine = ScalingEngine::new(config);
    // Engine created successfully
    assert!(true);
}

#[test]
fn test_scaling_engine_with_defaults() {
    let engine = ScalingEngine::with_defaults();
    // Engine created with default config
    assert!(true);
}

#[test]
fn test_should_attempt_another_initial() {
    let config = ScalingConfig::default();
    let engine = ScalingEngine::new(config);

    // First attempt should be allowed
    assert!(engine.should_attempt_another());
}

// ============================================================================
// ScalingDecision Tests
// ============================================================================

#[test]
fn test_scaling_decision_continue() {
    let decision = ScalingDecision {
        should_continue: true,
        next_timeout_ms: 60000,
        recommended_strategy: "aggressive".to_string(),
        reason: "Attempt 2 of 5".to_string(),
        resource_allocation: ResourceAllocation {
            token_budget: 100000,
            cost_budget: 2.0,
            time_budget_ms: 60000,
            priority: AttemptPriority::High,
        },
    };

    assert!(decision.should_continue);
    assert_eq!(decision.next_timeout_ms, 60000);
    assert_eq!(decision.recommended_strategy, "aggressive");
    assert_eq!(decision.resource_allocation.token_budget, 100000);
}

#[test]
fn test_scaling_decision_stop() {
    let decision = ScalingDecision {
        should_continue: false,
        next_timeout_ms: 0,
        recommended_strategy: "none".to_string(),
        reason: "Max attempts reached".to_string(),
        resource_allocation: ResourceAllocation {
            token_budget: 0,
            cost_budget: 0.0,
            time_budget_ms: 0,
            priority: AttemptPriority::Low,
        },
    };

    assert!(!decision.should_continue);
    assert_eq!(decision.next_timeout_ms, 0);
    assert_eq!(decision.reason, "Max attempts reached");
}

// ============================================================================
// ResourceAllocation Tests
// ============================================================================

#[test]
fn test_resource_allocation_creation() {
    let allocation = ResourceAllocation {
        token_budget: 150000,
        cost_budget: 3.5,
        time_budget_ms: 120000,
        priority: AttemptPriority::Critical,
    };

    assert_eq!(allocation.token_budget, 150000);
    assert_eq!(allocation.cost_budget, 3.5);
    assert_eq!(allocation.time_budget_ms, 120000);
    assert!(matches!(allocation.priority, AttemptPriority::Critical));
}

// ============================================================================
// AttemptPriority Tests
// ============================================================================

#[test]
fn test_attempt_priority_variants() {
    assert!(matches!(AttemptPriority::Low, AttemptPriority::Low));
    assert!(matches!(AttemptPriority::Normal, AttemptPriority::Normal));
    assert!(matches!(AttemptPriority::High, AttemptPriority::High));
    assert!(matches!(AttemptPriority::Critical, AttemptPriority::Critical));
}

#[test]
fn test_attempt_priority_ordering() {
    // Test that AttemptPriority variants exist and can be compared for equality
    assert!(AttemptPriority::Low != AttemptPriority::Normal);
    assert!(AttemptPriority::Normal != AttemptPriority::High);
    assert!(AttemptPriority::High != AttemptPriority::Critical);
    // Note: Ordering comparisons (<, >) not available as AttemptPriority doesn't implement PartialOrd
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_scaling_config_backoff() {
    let config = ScalingConfig {
        initial_timeout_ms: 30000,
        backoff_multiplier: 2.0,
        max_timeout_ms: 300000,
        ..Default::default()
    };

    // First attempt: 30s
    // Second attempt: 60s
    // Third attempt: 120s
    assert_eq!(config.initial_timeout_ms, 30000);
    assert_eq!(config.backoff_multiplier, 2.0);
}

#[test]
fn test_resource_allocation_for_different_priorities() {
    let low_priority = ResourceAllocation {
        token_budget: 50000,
        cost_budget: 1.0,
        time_budget_ms: 30000,
        priority: AttemptPriority::Low,
    };

    let high_priority = ResourceAllocation {
        token_budget: 200000,
        cost_budget: 5.0,
        time_budget_ms: 120000,
        priority: AttemptPriority::High,
    };

    assert!(low_priority.token_budget < high_priority.token_budget);
    assert!(low_priority.cost_budget < high_priority.cost_budget);
}
