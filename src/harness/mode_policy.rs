//! Harness Mode Policy - Issue 1.3
//! Explicit execution mode state machine for safe side-effect management

use serde::{Deserialize, Serialize};

/// Execution mode for the harness
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarnessMode {
    /// Review-only mode: never modify repo, generate reports only
    ReviewOnly,
    /// Assisted mode: apply only if dry-run passes and no critical issues
    Assisted,
    /// Autonomous mode: apply if dry-run passes and risk is acceptable
    Autonomous,
    /// Benchmark mode: apply if dry-run passes (disposable workspace)
    Benchmark,
}

/// Strategy for creating temporary workspaces for validation
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceStrategy {
    /// Validate in-place (patch must be applied)
    InPlace,
    /// Create a git worktree for validation
    GitWorktree,
    /// Create a temp copy of the repo
    TempCopy,
}

/// Where validation should run
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidationTarget {
    /// Validate the real repository
    RealRepo,
    /// Validate a temporary workspace
    TempWorkspace,
    /// No validation needed
    None,
}

/// Policy for checkpoint requirements
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckpointPolicy {
    /// Checkpoint is required - side effects blocked if checkpoint fails
    Required,
    /// Try to create checkpoint but allow side effects if it fails
    BestEffort,
    /// No checkpoint needed
    Disabled,
}

/// Policy object that defines behavior for a harness execution mode
/// This provides explicit, testable rules for side effects
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessModePolicy {
    /// Whether patches may be applied to the real repository
    pub may_apply_real_patch: bool,
    /// Whether patches may be applied to a temp workspace
    pub may_apply_temp_patch: bool,
    /// Whether user approval is required before side effects
    pub requires_user_approval: bool,
    /// Checkpoint creation policy
    pub checkpoint_policy: CheckpointPolicy,
    /// Where validation should run
    pub validation_target: ValidationTarget,
    /// Whether to allow high-risk patches without explicit approval
    pub allow_high_risk: bool,
    /// Whether to allow critical-risk patches
    pub allow_critical_risk: bool,
    /// Workspace strategy for creating temp copies
    pub workspace_strategy: WorkspaceStrategy,
}

impl HarnessModePolicy {
    /// Policy for ReviewOnly mode - never modifies repo
    pub fn review_only() -> Self {
        Self {
            may_apply_real_patch: false,
            may_apply_temp_patch: true, // Allow temp workspace for validation
            requires_user_approval: false,
            checkpoint_policy: CheckpointPolicy::Disabled,
            validation_target: ValidationTarget::TempWorkspace,
            allow_high_risk: false,
            allow_critical_risk: false,
            workspace_strategy: WorkspaceStrategy::TempCopy,
        }
    }

    /// Policy for Assisted mode - requires approval for high risk
    pub fn assisted() -> Self {
        Self {
            may_apply_real_patch: true,
            may_apply_temp_patch: true,
            requires_user_approval: true,
            checkpoint_policy: CheckpointPolicy::Required,
            validation_target: ValidationTarget::RealRepo,
            allow_high_risk: false, // Requires explicit approval
            allow_critical_risk: false,
            workspace_strategy: WorkspaceStrategy::GitWorktree,
        }
    }

    /// Policy for Autonomous mode - applies if acceptable risk
    pub fn autonomous() -> Self {
        Self {
            may_apply_real_patch: true,
            may_apply_temp_patch: true,
            requires_user_approval: false,
            checkpoint_policy: CheckpointPolicy::Required,
            validation_target: ValidationTarget::RealRepo,
            allow_high_risk: true,
            allow_critical_risk: false, // Never auto-apply critical risk
            workspace_strategy: WorkspaceStrategy::GitWorktree,
        }
    }

    /// Policy for Benchmark mode - disposable workspace
    pub fn benchmark() -> Self {
        Self {
            may_apply_real_patch: true,
            may_apply_temp_patch: true,
            requires_user_approval: false,
            checkpoint_policy: CheckpointPolicy::BestEffort,
            validation_target: ValidationTarget::RealRepo,
            allow_high_risk: true,
            allow_critical_risk: true, // In benchmark, we want to test everything
            workspace_strategy: WorkspaceStrategy::TempCopy,
        }
    }

    /// Get policy for a given mode
    pub fn for_mode(mode: HarnessMode) -> Self {
        match mode {
            HarnessMode::ReviewOnly => Self::review_only(),
            HarnessMode::Assisted => Self::assisted(),
            HarnessMode::Autonomous => Self::autonomous(),
            HarnessMode::Benchmark => Self::benchmark(),
        }
    }

    /// Check if patch should be applied based on risk level
    pub fn should_apply_for_risk(&self, risk_level: crate::harness::risk::RiskLevel) -> bool {
        match risk_level {
            crate::harness::risk::RiskLevel::None | crate::harness::risk::RiskLevel::Low => true,
            crate::harness::risk::RiskLevel::Medium => true,
            crate::harness::risk::RiskLevel::High => self.allow_high_risk,
            crate::harness::risk::RiskLevel::Critical => self.allow_critical_risk,
        }
    }

    /// Check if checkpoint failure should block side effects
    pub fn checkpoint_failure_blocks(&self) -> bool {
        matches!(self.checkpoint_policy, CheckpointPolicy::Required)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_review_only_policy() {
        let policy = HarnessModePolicy::review_only();
        assert!(!policy.may_apply_real_patch);
        assert!(policy.may_apply_temp_patch);
        assert!(!policy.requires_user_approval);
        assert!(matches!(policy.checkpoint_policy, CheckpointPolicy::Disabled));
        assert!(matches!(policy.validation_target, ValidationTarget::TempWorkspace));
    }

    #[test]
    fn test_assisted_policy() {
        let policy = HarnessModePolicy::assisted();
        assert!(policy.may_apply_real_patch);
        assert!(policy.requires_user_approval);
        assert!(matches!(policy.checkpoint_policy, CheckpointPolicy::Required));
        assert!(policy.checkpoint_failure_blocks());
    }

    #[test]
    fn test_autonomous_policy() {
        let policy = HarnessModePolicy::autonomous();
        assert!(policy.may_apply_real_patch);
        assert!(!policy.requires_user_approval);
        assert!(matches!(policy.checkpoint_policy, CheckpointPolicy::Required));
        assert!(policy.allow_high_risk);
        assert!(!policy.allow_critical_risk);
    }

    #[test]
    fn test_benchmark_policy() {
        let policy = HarnessModePolicy::benchmark();
        assert!(policy.may_apply_real_patch);
        assert!(matches!(policy.checkpoint_policy, CheckpointPolicy::BestEffort));
        assert!(!policy.checkpoint_failure_blocks());
        assert!(policy.allow_critical_risk);
    }

    #[test]
    fn test_risk_level_checks() {
        let autonomous = HarnessModePolicy::autonomous();
        assert!(autonomous.should_apply_for_risk(crate::harness::risk::RiskLevel::Low));
        assert!(autonomous.should_apply_for_risk(crate::harness::risk::RiskLevel::High));
        assert!(!autonomous.should_apply_for_risk(crate::harness::risk::RiskLevel::Critical));

        let assisted = HarnessModePolicy::assisted();
        assert!(assisted.should_apply_for_risk(crate::harness::risk::RiskLevel::Medium));
        assert!(!assisted.should_apply_for_risk(crate::harness::risk::RiskLevel::High));
    }
}
