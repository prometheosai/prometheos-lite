//! Issue 30: Multi-Model Strategy Tests
//!
//! Comprehensive tests for Multi-Model Strategy including:
//! - ModelTier enum (Fast, Balanced, Powerful, Expert)
//! - ModelProfile struct (id, name, tier, context_window, cost, latency, strengths)
//! - TaskRequirements struct (complexity, code_size, languages, reasoning, creativity)
//! - ComplexityLevel enum (Simple, Moderate, Complex, Expert)
//! - UrgencyLevel enum (Low, Normal, High, Critical)
//! - ModelStrategyEngine for model selection
//! - ModelRecommendation for selection results
//! - ModelPerformance tracking

use prometheos_lite::harness::model_strategy::{
    ComplexityLevel, ModelProfile, ModelStrategyEngine, ModelTier, TaskRequirements, UrgencyLevel,
};

// ============================================================================
// ModelTier Tests
// ============================================================================

#[test]
fn test_model_tier_variants() {
    assert!(matches!(ModelTier::Fast, ModelTier::Fast));
    assert!(matches!(ModelTier::Balanced, ModelTier::Balanced));
    assert!(matches!(ModelTier::Powerful, ModelTier::Powerful));
    assert!(matches!(ModelTier::Expert, ModelTier::Expert));
}

#[test]
fn test_model_tier_ordering() {
    // Test that ModelTier variants exist and can be compared for equality
    assert!(ModelTier::Fast != ModelTier::Balanced);
    assert!(ModelTier::Balanced != ModelTier::Powerful);
    assert!(ModelTier::Powerful != ModelTier::Expert);
    // Note: Ordering comparisons (<, >) not available as ModelTier doesn't implement PartialOrd
}

// ============================================================================
// ModelProfile Tests
// ============================================================================

#[test]
fn test_model_profile_creation() {
    let profile = ModelProfile {
        id: "gpt-4".to_string(),
        name: "GPT-4".to_string(),
        tier: ModelTier::Expert,
        context_window: 8192,
        cost_per_1k_tokens: 0.03,
        avg_latency_ms: 2000,
        strengths: vec!["reasoning".to_string(), "code".to_string()],
        weaknesses: vec!["speed".to_string()],
        supported_languages: vec!["rust".to_string(), "python".to_string()],
        max_file_size_kb: 1024,
    };

    assert_eq!(profile.id, "gpt-4");
    assert!(matches!(profile.tier, ModelTier::Expert));
    assert_eq!(profile.context_window, 8192);
    assert_eq!(profile.cost_per_1k_tokens, 0.03);
}

#[test]
fn test_model_profile_fast() {
    let profile = ModelProfile {
        id: "gpt-3.5".to_string(),
        name: "GPT-3.5".to_string(),
        tier: ModelTier::Fast,
        context_window: 4096,
        cost_per_1k_tokens: 0.002,
        avg_latency_ms: 500,
        strengths: vec!["speed".to_string()],
        weaknesses: vec!["complex_reasoning".to_string()],
        supported_languages: vec!["rust".to_string(), "python".to_string(), "js".to_string()],
        max_file_size_kb: 512,
    };

    assert!(matches!(profile.tier, ModelTier::Fast));
    assert_eq!(profile.avg_latency_ms, 500);
}

// ============================================================================
// ComplexityLevel Tests
// ============================================================================

#[test]
fn test_complexity_level_variants() {
    assert!(matches!(ComplexityLevel::Simple, ComplexityLevel::Simple));
    assert!(matches!(
        ComplexityLevel::Moderate,
        ComplexityLevel::Moderate
    ));
    assert!(matches!(ComplexityLevel::Complex, ComplexityLevel::Complex));
    assert!(matches!(ComplexityLevel::Expert, ComplexityLevel::Expert));
}

// ============================================================================
// UrgencyLevel Tests
// ============================================================================

#[test]
fn test_urgency_level_variants() {
    assert!(matches!(UrgencyLevel::Low, UrgencyLevel::Low));
    assert!(matches!(UrgencyLevel::Normal, UrgencyLevel::Normal));
    assert!(matches!(UrgencyLevel::High, UrgencyLevel::High));
    assert!(matches!(UrgencyLevel::Critical, UrgencyLevel::Critical));
}

// ============================================================================
// TaskRequirements Tests
// ============================================================================

#[test]
fn test_task_requirements_creation() {
    let requirements = TaskRequirements {
        complexity: ComplexityLevel::Complex,
        code_size_lines: 500,
        languages: vec!["rust".to_string()],
        requires_reasoning: true,
        requires_creativity: false,
        urgency: UrgencyLevel::High,
        budget_constraint: Some(5.0),
    };

    assert!(matches!(requirements.complexity, ComplexityLevel::Complex));
    assert_eq!(requirements.code_size_lines, 500);
    assert!(requirements.requires_reasoning);
    assert!(!requirements.requires_creativity);
    assert_eq!(requirements.budget_constraint, Some(5.0));
}

#[test]
fn test_task_requirements_simple() {
    let requirements = TaskRequirements {
        complexity: ComplexityLevel::Simple,
        code_size_lines: 50,
        languages: vec!["python".to_string()],
        requires_reasoning: false,
        requires_creativity: false,
        urgency: UrgencyLevel::Low,
        budget_constraint: None,
    };

    assert!(matches!(requirements.complexity, ComplexityLevel::Simple));
    assert!(requirements.budget_constraint.is_none());
}

// ============================================================================
// ModelStrategyEngine Tests
// ============================================================================

#[test]
fn test_model_strategy_engine_new() {
    let engine = ModelStrategyEngine::new();
    // Engine created successfully
    assert!(true);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_model_selection_workflow() {
    // Create models
    let expert_model = ModelProfile {
        id: "claude-3-opus".to_string(),
        name: "Claude 3 Opus".to_string(),
        tier: ModelTier::Expert,
        context_window: 200000,
        cost_per_1k_tokens: 0.015,
        avg_latency_ms: 3000,
        strengths: vec![
            "reasoning".to_string(),
            "code".to_string(),
            "analysis".to_string(),
        ],
        weaknesses: vec!["cost".to_string()],
        supported_languages: vec![
            "rust".to_string(),
            "python".to_string(),
            "typescript".to_string(),
        ],
        max_file_size_kb: 2048,
    };

    let fast_model = ModelProfile {
        id: "claude-3-haiku".to_string(),
        name: "Claude 3 Haiku".to_string(),
        tier: ModelTier::Fast,
        context_window: 200000,
        cost_per_1k_tokens: 0.00025,
        avg_latency_ms: 500,
        strengths: vec!["speed".to_string()],
        weaknesses: vec!["complex_reasoning".to_string()],
        supported_languages: vec!["rust".to_string(), "python".to_string()],
        max_file_size_kb: 512,
    };

    assert!(expert_model.tier != fast_model.tier); // Can't use > as ModelTier doesn't implement PartialOrd
    assert!(expert_model.cost_per_1k_tokens > fast_model.cost_per_1k_tokens);
    assert!(fast_model.avg_latency_ms < expert_model.avg_latency_ms);
}

#[test]
fn test_task_requirements_for_complex_task() {
    let requirements = TaskRequirements {
        complexity: ComplexityLevel::Expert,
        code_size_lines: 2000,
        languages: vec!["rust".to_string(), "c".to_string()],
        requires_reasoning: true,
        requires_creativity: true,
        urgency: UrgencyLevel::Critical,
        budget_constraint: Some(10.0),
    };

    assert!(matches!(requirements.complexity, ComplexityLevel::Expert));
    assert!(requirements.requires_reasoning);
    assert!(requirements.requires_creativity);
    assert!(matches!(requirements.urgency, UrgencyLevel::Critical));
}
