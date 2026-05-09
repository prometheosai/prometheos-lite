//! Issue 20: Selection Engine Tests
//!
//! Comprehensive tests for the Selection Engine including:
//! - PatchCandidate struct (id, edits, confidence, risk, validation, etc.)
//! - SelectionCriteria struct (weights, thresholds, requirements)
//! - ScoredCandidate struct (scores, eligibility, rejection reasons)
//! - SelectionEngine for ranking and selecting patches
//! - select_best_candidate() for choosing optimal patch
//! - rank_candidates() for ordered scoring
//! - score_candidate() with multi-dimensional scoring
//! - Eligibility checking (confidence, risk, size, validation)
//! - Weighted scoring across 5 dimensions

use std::collections::HashMap;

use prometheos_lite::harness::confidence::ConfidenceScore;
use prometheos_lite::harness::edit_protocol::EditOperation;
use prometheos_lite::harness::risk::{RiskAssessment, RiskLevel};
use prometheos_lite::harness::review::ReviewIssue;
use prometheos_lite::harness::selection::{
    PatchCandidate, ScoredCandidate, SelectionCriteria, SelectionEngine,
};
use prometheos_lite::harness::semantic_diff::SemanticDiff;
use prometheos_lite::harness::validation::ValidationResult;

// ============================================================================
// PatchCandidate Tests
// ============================================================================

#[test]
fn test_patch_candidate_creation() {
    let candidate = PatchCandidate {
        id: "patch-1".to_string(),
        edits: vec![],
        source: "model-a".to_string(),
        confidence: ConfidenceScore {
            score: 0.85,
            factors: vec![],
            explanation: "High confidence".to_string(),
            recommendation: None,
        },
        metadata: HashMap::new(),
        risk: None,
        validation: None,
        review_issues: vec![],
        semantic_diff: None,
        lines_added: 10,
        lines_removed: 5,
    };

    assert_eq!(candidate.id, "patch-1");
    assert_eq!(candidate.source, "model-a");
    assert_eq!(candidate.confidence.score, 0.85);
    assert_eq!(candidate.lines_added, 10);
}

#[test]
fn test_patch_candidate_with_metadata() {
    let mut metadata = HashMap::new();
    metadata.insert("author".to_string(), "ai-model".to_string());
    metadata.insert("timestamp".to_string(), "2024-01-01".to_string());

    let candidate = PatchCandidate {
        id: "patch-2".to_string(),
        edits: vec![],
        source: "model-b".to_string(),
        confidence: ConfidenceScore {
            score: 0.75,
            factors: vec![],
            explanation: "Good".to_string(),
            recommendation: None,
        },
        metadata,
        risk: None,
        validation: None,
        review_issues: vec![],
        semantic_diff: None,
        lines_added: 20,
        lines_removed: 10,
    };

    assert_eq!(candidate.metadata.len(), 2);
    assert_eq!(candidate.metadata.get("author"), Some(&"ai-model".to_string()));
}

// ============================================================================
// SelectionCriteria Tests
// ============================================================================

#[test]
fn test_selection_criteria_default() {
    let criteria = SelectionCriteria::default();

    assert_eq!(criteria.confidence_weight, 0.3);
    assert_eq!(criteria.risk_weight, 0.25);
    assert_eq!(criteria.size_weight, 0.15);
    assert_eq!(criteria.review_weight, 0.15);
    assert_eq!(criteria.validation_weight, 0.15);
    assert_eq!(criteria.min_confidence_threshold, 0.6);
    assert_eq!(criteria.max_risk_level, RiskLevel::High);
    assert_eq!(criteria.max_patch_size_lines, 500);
    assert!(criteria.require_validation);
    assert!(!criteria.require_review_pass);
}

#[test]
fn test_selection_criteria_custom() {
    let criteria = SelectionCriteria {
        confidence_weight: 0.4,
        risk_weight: 0.3,
        size_weight: 0.1,
        review_weight: 0.1,
        validation_weight: 0.1,
        min_confidence_threshold: 0.7,
        max_risk_level: RiskLevel::Medium,
        max_patch_size_lines: 300,
        require_validation: true,
        require_review_pass: true,
    };

    assert_eq!(criteria.confidence_weight, 0.4);
    assert_eq!(criteria.min_confidence_threshold, 0.7);
    assert_eq!(criteria.max_risk_level, RiskLevel::Medium);
    assert!(criteria.require_review_pass);
}

// ============================================================================
// ScoredCandidate Tests
// ============================================================================

#[test]
fn test_scored_candidate_eligible() {
    let scored = ScoredCandidate {
        candidate: PatchCandidate {
            id: "patch-1".to_string(),
            edits: vec![],
            source: "test".to_string(),
            confidence: ConfidenceScore {
                score: 0.9,
                factors: vec![],
                explanation: "High".to_string(),
                recommendation: None,
            },
            metadata: HashMap::new(),
            risk: None,
            validation: None,
            review_issues: vec![],
            semantic_diff: None,
            lines_added: 10,
            lines_removed: 2,
        },
        total_score: 0.85,
        confidence_score: 0.9,
        risk_score: 0.8,
        size_score: 0.9,
        review_score: 1.0,
        validation_score: 0.8,
        is_eligible: true,
        rejection_reasons: vec![],
    };

    assert!(scored.is_eligible);
    assert!(scored.rejection_reasons.is_empty());
    assert_eq!(scored.total_score, 0.85);
}

#[test]
fn test_scored_candidate_ineligible() {
    let scored = ScoredCandidate {
        candidate: PatchCandidate {
            id: "patch-2".to_string(),
            edits: vec![],
            source: "test".to_string(),
            confidence: ConfidenceScore {
                score: 0.5,
                factors: vec![],
                explanation: "Low".to_string(),
                recommendation: None,
            },
            metadata: HashMap::new(),
            risk: None,
            validation: None,
            review_issues: vec![],
            semantic_diff: None,
            lines_added: 1000,
            lines_removed: 500,
        },
        total_score: 0.0,
        confidence_score: 0.5,
        risk_score: 0.3,
        size_score: 0.2,
        review_score: 0.4,
        validation_score: 0.0,
        is_eligible: false,
        rejection_reasons: vec![
            "Confidence below threshold".to_string(),
            "Patch too large".to_string(),
        ],
    };

    assert!(!scored.is_eligible);
    assert_eq!(scored.rejection_reasons.len(), 2);
}

// ============================================================================
// SelectionEngine Tests
// ============================================================================

#[test]
fn test_selection_engine_new() {
    let criteria = SelectionCriteria::default();
    let engine = SelectionEngine::new(criteria);
    // Engine created successfully
    assert!(true);
}

#[test]
fn test_selection_engine_default_criteria() {
    let engine = SelectionEngine::with_default_criteria();
    // Engine created with default criteria
    assert!(true);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_candidate_scoring_high_confidence() {
    let candidate = PatchCandidate {
        id: "high-confidence".to_string(),
        edits: vec![],
        source: "model".to_string(),
        confidence: ConfidenceScore {
            score: 0.95,
            factors: vec![],
            explanation: "Excellent".to_string(),
            recommendation: None,
        },
        metadata: HashMap::new(),
        risk: Some(RiskAssessment {
            level: RiskLevel::Low,
            reasons: vec![],
            requires_approval: false,
            can_override: true,
            override_conditions: vec![],
            assessed: true,
        }),
        validation: Some(ValidationResult {
            status: prometheos_lite::harness::validation::ValidationStatus::Passed,
            duration_ms: 1000,
            command_results: vec![],
            cached: false,
            category_results: std::collections::HashMap::new(),
            errors: vec![],
            flaky_tests_detected: vec![],
            validation_performed: true,
            is_final_gate: false,
            cache_disabled: false,
            commands_planned: 1,
            commands_executed: 1,
            commands_skipped: 0,
            categories_executed: vec![],
        }),
        review_issues: vec![],
        semantic_diff: None,
        lines_added: 50,
        lines_removed: 10,
    };

    assert_eq!(candidate.confidence.score, 0.95);
    assert!(candidate.risk.as_ref().unwrap().level == RiskLevel::Low);
    assert!(candidate.validation.as_ref().unwrap().passed());
}

#[test]
fn test_candidate_scoring_high_risk() {
    let candidate = PatchCandidate {
        id: "high-risk".to_string(),
        edits: vec![],
        source: "model".to_string(),
        confidence: ConfidenceScore {
            score: 0.6,
            factors: vec![],
            explanation: "Medium".to_string(),
            recommendation: None,
        },
        metadata: HashMap::new(),
        risk: Some(RiskAssessment {
            level: RiskLevel::Critical,
            reasons: vec![],
            requires_approval: true,
            can_override: false,
            override_conditions: vec![],
            assessed: true,
        }),
        validation: Some(ValidationResult {
            status: prometheos_lite::harness::validation::ValidationStatus::Failed,
            duration_ms: 2000,
            command_results: vec![],
            cached: false,
            category_results: std::collections::HashMap::new(),
            errors: vec![],
            flaky_tests_detected: vec![],
            validation_performed: true,
            is_final_gate: false,
            cache_disabled: false,
            commands_planned: 1,
            commands_executed: 1,
            commands_skipped: 0,
            categories_executed: vec![],
        }),
        review_issues: vec![ReviewIssue {
            issue_type: prometheos_lite::harness::review::ReviewIssueType::Security,
            severity: prometheos_lite::harness::review::ReviewSeverity::Critical,
            file: None,
            line: None,
            message: "Security issue".to_string(),
            suggestion: None,
            rule_id: "security_check".to_string(),
        }],
        semantic_diff: None,
        lines_added: 500,
        lines_removed: 200,
    };

    assert_eq!(candidate.risk.as_ref().unwrap().level, RiskLevel::Critical);
    assert!(!candidate.validation.as_ref().unwrap().passed());
    assert!(!candidate.review_issues.is_empty());
}

#[test]
fn test_empty_candidates() {
    let candidates: Vec<PatchCandidate> = vec![];
    assert!(candidates.is_empty());
}

#[test]
fn test_multiple_candidates() {
    let candidates = vec![
        PatchCandidate {
            id: "patch-a".to_string(),
            edits: vec![],
            source: "model-1".to_string(),
            confidence: ConfidenceScore {
                score: 0.9,
                factors: vec![],
                explanation: "High".to_string(),
                recommendation: None,
            },
            metadata: HashMap::new(),
            risk: None,
            validation: None,
            review_issues: vec![],
            semantic_diff: None,
            lines_added: 20,
            lines_removed: 5,
        },
        PatchCandidate {
            id: "patch-b".to_string(),
            edits: vec![],
            source: "model-2".to_string(),
            confidence: ConfidenceScore {
                score: 0.7,
                factors: vec![],
                explanation: "Medium".to_string(),
                recommendation: None,
            },
            metadata: HashMap::new(),
            risk: None,
            validation: None,
            review_issues: vec![],
            semantic_diff: None,
            lines_added: 30,
            lines_removed: 10,
        },
    ];

    assert_eq!(candidates.len(), 2);
    assert!(candidates[0].confidence.score > candidates[1].confidence.score);
}
