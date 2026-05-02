//! Selection Engine - Issue #9
//! Multi-factor patch selection and ranking system

use crate::harness::{
    confidence::ConfidenceScore,
    review::{ReviewIssue, ReviewSeverity},
    risk::{RiskAssessment, RiskLevel},
    semantic_diff::SemanticDiff,
    validation::ValidationResult,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchCandidate {
    pub id: String,
    pub patch_content: String,
    pub files_changed: Vec<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub confidence: ConfidenceScore,
    pub risk: RiskAssessment,
    pub validation: Option<ValidationResult>,
    pub review_issues: Vec<ReviewIssue>,
    pub semantic_diff: SemanticDiff,
    pub generation_strategy: String,
    pub attempt_number: u32,
    pub generation_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectionCriteria {
    pub confidence_weight: f32,
    pub risk_weight: f32,
    pub size_weight: f32,
    pub review_weight: f32,
    pub validation_weight: f32,
    pub min_confidence_threshold: f32,
    pub max_risk_level: RiskLevel,
    pub max_patch_size_lines: usize,
    pub require_validation: bool,
    pub require_review_pass: bool,
}

impl Default for SelectionCriteria {
    fn default() -> Self {
        Self {
            confidence_weight: 0.3,
            risk_weight: 0.25,
            size_weight: 0.15,
            review_weight: 0.15,
            validation_weight: 0.15,
            min_confidence_threshold: 0.6,
            max_risk_level: RiskLevel::High,
            max_patch_size_lines: 500,
            require_validation: true,
            require_review_pass: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoredCandidate {
    pub candidate: PatchCandidate,
    pub total_score: f32,
    pub confidence_score: f32,
    pub risk_score: f32,
    pub size_score: f32,
    pub review_score: f32,
    pub validation_score: f32,
    pub is_eligible: bool,
    pub rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SelectionEngine {
    criteria: SelectionCriteria,
    scoring_history: Vec<SelectionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SelectionRecord {
    selected_candidate_id: String,
    scores: HashMap<String, f32>,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl SelectionEngine {
    pub fn new(criteria: SelectionCriteria) -> Self {
        Self {
            criteria,
            scoring_history: Vec::new(),
        }
    }

    pub fn with_default_criteria() -> Self {
        Self::new(SelectionCriteria::default())
    }

    pub fn select_best_candidate(&mut self, candidates: Vec<PatchCandidate>) -> Result<Option<ScoredCandidate>> {
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut scored: Vec<ScoredCandidate> = candidates
            .into_iter()
            .map(|c| self.score_candidate(c))
            .collect();

        // Sort by total score (descending)
        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());

        // Return highest-scoring eligible candidate
        let selected = scored.iter().find(|s| s.is_eligible).cloned();

        if let Some(ref s) = selected {
            self.scoring_history.push(SelectionRecord {
                selected_candidate_id: s.candidate.id.clone(),
                scores: self.extract_scores(s),
                timestamp: chrono::Utc::now(),
            });
        }

        Ok(selected)
    }

    pub fn rank_candidates(&self, candidates: Vec<PatchCandidate>) -> Vec<ScoredCandidate> {
        let mut scored: Vec<ScoredCandidate> = candidates
            .into_iter()
            .map(|c| self.score_candidate(c))
            .collect();

        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());
        scored
    }

    fn score_candidate(&self, candidate: PatchCandidate) -> ScoredCandidate {
        let mut rejection_reasons = Vec::new();

        // Calculate individual scores
        let confidence_score = self.calculate_confidence_score(&candidate);
        let risk_score = self.calculate_risk_score(&candidate);
        let size_score = self.calculate_size_score(&candidate);
        let review_score = self.calculate_review_score(&candidate);
        let validation_score = self.calculate_validation_score(&candidate);

        // Check eligibility criteria
        let is_eligible = self.check_eligibility(&candidate, &mut rejection_reasons);

        // Calculate weighted total
        let total_score = if is_eligible {
            confidence_score * self.criteria.confidence_weight
                + risk_score * self.criteria.risk_weight
                + size_score * self.criteria.size_weight
                + review_score * self.criteria.review_weight
                + validation_score * self.criteria.validation_weight
        } else {
            0.0
        };

        ScoredCandidate {
            candidate,
            total_score,
            confidence_score,
            risk_score,
            size_score,
            review_score,
            validation_score,
            is_eligible,
            rejection_reasons,
        }
    }

    fn calculate_confidence_score(&self, candidate: &PatchCandidate) -> f32 {
        candidate.confidence.score
    }

    fn calculate_risk_score(&self, candidate: &PatchCandidate) -> f32 {
        // Higher score for lower risk
        match candidate.risk.level {
            RiskLevel::None => 1.0,
            RiskLevel::Critical => 0.0,
            RiskLevel::High => 0.4,
            RiskLevel::Medium => 0.7,
            RiskLevel::Low => 1.0,
        }
    }

    fn calculate_size_score(&self, candidate: &PatchCandidate) -> f32 {
        let total_lines = candidate.lines_added + candidate.lines_removed;
        if total_lines > self.criteria.max_patch_size_lines {
            0.0
        } else {
            1.0 - (total_lines as f32 / self.criteria.max_patch_size_lines as f32)
        }
    }

    fn calculate_review_score(&self, candidate: &PatchCandidate) -> f32 {
        let critical_count = candidate
            .review_issues
            .iter()
            .filter(|i| i.severity == ReviewSeverity::Critical)
            .count();
        let high_count = candidate
            .review_issues
            .iter()
            .filter(|i| i.severity == ReviewSeverity::High)
            .count();

        if critical_count > 0 {
            0.0
        } else if high_count > 2 {
            0.3
        } else if high_count > 0 {
            0.6
        } else {
            1.0
        }
    }

    fn calculate_validation_score(&self, candidate: &PatchCandidate) -> f32 {
        match &candidate.validation {
            None => 0.0,
            Some(v) => {
                if !v.passed {
                    0.0
                } else {
                    let command_count = v.command_results.len();
                    if command_count >= 3 {
                        1.0
                    } else {
                        0.5 + (command_count as f32 * 0.17)
                    }
                }
            }
        }
    }

    fn check_eligibility(&self, candidate: &PatchCandidate, reasons: &mut Vec<String>) -> bool {
        let mut eligible = true;

        // Check confidence threshold
        if candidate.confidence.score < self.criteria.min_confidence_threshold {
            reasons.push(format!(
                "Confidence {:.0}% below threshold {:.0}%",
                candidate.confidence.score * 100.0,
                self.criteria.min_confidence_threshold * 100.0
            ));
            eligible = false;
        }

        // Check risk level
        let risk_level_value = match candidate.risk.level {
            RiskLevel::None => 0,
            RiskLevel::Low => 1,
            RiskLevel::Medium => 2,
            RiskLevel::High => 3,
            RiskLevel::Critical => 4,
        };
        let max_risk_value = match self.criteria.max_risk_level {
            RiskLevel::None => 0,
            RiskLevel::Low => 1,
            RiskLevel::Medium => 2,
            RiskLevel::High => 3,
            RiskLevel::Critical => 4,
        };

        if risk_level_value > max_risk_value {
            reasons.push(format!(
                "Risk level {:?} exceeds maximum {:?}",
                candidate.risk.level, self.criteria.max_risk_level
            ));
            eligible = false;
        }

        // Check patch size
        let total_lines = candidate.lines_added + candidate.lines_removed;
        if total_lines > self.criteria.max_patch_size_lines {
            reasons.push(format!(
                "Patch size {} lines exceeds maximum {}",
                total_lines, self.criteria.max_patch_size_lines
            ));
            eligible = false;
        }

        // Check validation requirement
        if self.criteria.require_validation && candidate.validation.is_none() {
            reasons.push("Validation required but not performed".to_string());
            eligible = false;
        }

        // Check review pass requirement
        if self.criteria.require_review_pass {
            let has_critical = candidate
                .review_issues
                .iter()
                .any(|i| i.severity == ReviewSeverity::Critical);
            if has_critical {
                reasons.push("Critical review issues found".to_string());
                eligible = false;
            }
        }

        eligible
    }

    fn extract_scores(&self, scored: &ScoredCandidate) -> HashMap<String, f32> {
        let mut scores = HashMap::new();
        scores.insert("total".to_string(), scored.total_score);
        scores.insert("confidence".to_string(), scored.confidence_score);
        scores.insert("risk".to_string(), scored.risk_score);
        scores.insert("size".to_string(), scored.size_score);
        scores.insert("review".to_string(), scored.review_score);
        scores.insert("validation".to_string(), scored.validation_score);
        scores
    }

    pub fn get_selection_history(&self) -> &[SelectionRecord] {
        &self.scoring_history
    }

    pub fn clear_history(&mut self) {
        self.scoring_history.clear();
    }
}

pub fn select_best_patch(
    candidates: Vec<PatchCandidate>,
    criteria: SelectionCriteria,
) -> Result<Option<ScoredCandidate>> {
    let mut engine = SelectionEngine::new(criteria);
    engine.select_best_candidate(candidates)
}

pub fn rank_patches(candidates: Vec<PatchCandidate>) -> Vec<ScoredCandidate> {
    let engine = SelectionEngine::with_default_criteria();
    engine.rank_candidates(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::review::ReviewIssueType;

    fn create_test_candidate(id: &str, confidence: f32) -> PatchCandidate {
        PatchCandidate {
            id: id.to_string(),
            patch_content: "test".to_string(),
            files_changed: vec![],
            lines_added: 10,
            lines_removed: 5,
            confidence: ConfidenceScore {
                score: confidence,
                factors: vec![],
            },
            risk: RiskAssessment {
                level: RiskLevel::Low,
                reasons: vec![],
                requires_approval: false,
            },
            validation: Some(ValidationResult {
                passed: true,
                command_results: vec![],
                total_duration_ms: 1000,
            }),
            review_issues: vec![],
            semantic_diff: SemanticDiff::default(),
            generation_strategy: "test".to_string(),
            attempt_number: 1,
            generation_time_ms: 100,
        }
    }

    #[test]
    fn test_selection_engine_ranks_by_confidence() {
        let candidates = vec![
            create_test_candidate("low", 0.5),
            create_test_candidate("high", 0.9),
            create_test_candidate("med", 0.7),
        ];

        let ranked = rank_patches(candidates);
        assert_eq!(ranked[0].candidate.id, "high");
        assert_eq!(ranked[1].candidate.id, "med");
        assert_eq!(ranked[2].candidate.id, "low");
    }

    #[test]
    fn test_eligibility_threshold() {
        let mut criteria = SelectionCriteria::default();
        criteria.min_confidence_threshold = 0.8;

        let mut engine = SelectionEngine::new(criteria);
        let candidates = vec![
            create_test_candidate("below", 0.7),
            create_test_candidate("above", 0.9),
        ];

        let selected = engine.select_best_candidate(candidates).unwrap();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().candidate.id, "above");
    }
}
