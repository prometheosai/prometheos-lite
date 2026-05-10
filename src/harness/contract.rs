//! Canonical harness execution contract (V1.6.1 alignment).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::harness::completion::CompletionDecision;
use crate::harness::mode_policy::HarnessMode;
use crate::harness::risk::RiskLevel;
use crate::harness::verification::VerificationStrength;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkContextBudget {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_total_tokens: u32,
    pub max_cost_cents: u32,
}

impl Default for WorkContextBudget {
    fn default() -> Self {
        Self {
            max_input_tokens: 16_000,
            max_output_tokens: 4_000,
            max_total_tokens: 32_000,
            max_cost_cents: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessRequest {
    pub work_context_id: String,
    pub repo_root: PathBuf,
    pub task: String,
    pub acceptance_criteria: Vec<String>,
    pub mode: HarnessMode,
    pub budget: WorkContextBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessResult {
    pub run_id: String,
    pub evidence_log_id: String,
    pub completion_decision: CompletionDecision,
    pub artifact_summary: String,
    pub risk_level: Option<RiskLevel>,
    pub verification_strength: Option<VerificationStrength>,
    pub token_usage: Option<u64>,
}
