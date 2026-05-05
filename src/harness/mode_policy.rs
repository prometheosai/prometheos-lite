//! Harness Mode Policy - Issue 1.3
//! Explicit execution mode state machine for safe side-effect management

use serde::{Deserialize, Serialize};

/// Execution mode for the harness
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarnessMode {
    /// Legacy review mode - same as ReviewOnly, kept for backward compatibility
    Review,
    /// Review-only mode: never modify repo, generate reports only
    ReviewOnly,
    /// Assisted mode: apply only if dry-run passes and no critical issues
    Assisted,
    /// Autonomous mode: apply if dry-run passes and risk is acceptable
    Autonomous,
    /// Benchmark mode: apply if dry-run passed (disposable workspace)
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
            HarnessMode::Review | HarnessMode::ReviewOnly => Self::review_only(),
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

/// Hard policy gate that enforces execution rules
/// This is the single point of authority for side-effect decisions
pub struct HarnessPolicyGate {
    mode: HarnessMode,
    policy: HarnessModePolicy,
}

/// Gate decision result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Block(String),
    RequireApproval(String),
}

impl HarnessPolicyGate {
    /// Create a new policy gate for the given mode
    pub fn for_mode(mode: HarnessMode) -> Self {
        let policy = HarnessModePolicy::for_mode(mode);
        Self { mode, policy }
    }

    /// Check if dry-run is required before real patch application
    pub fn require_dry_run(&self) -> bool {
        // All modes except ReviewOnly require dry-run before real patch
        !matches!(self.mode, HarnessMode::ReviewOnly)
    }

    /// Check if review is required before patch application
    pub fn require_review(&self) -> bool {
        // All modes require review before patch application
        // ReviewOnly still runs review for reporting
        true
    }

    /// Check if risk assessment is required before patch application  
    pub fn require_risk_assessment(&self) -> bool {
        // All modes require risk assessment
        true
    }

    /// Check if validation is required after patch application
    pub fn require_validation(&self) -> bool {
        // All modes except ReviewOnly require validation
        !matches!(self.mode, HarnessMode::ReviewOnly)
    }

    /// Gate check: Can we apply a real patch given the current state?
    pub fn check_patch_application(
        &self,
        dry_run_passed: bool,
        has_critical_review_issues: bool,
        risk_level: crate::harness::risk::RiskLevel,
        has_rollback: bool,
    ) -> GateDecision {
        // P0: ReviewOnly never applies real patches
        if matches!(self.mode, HarnessMode::ReviewOnly) {
            return GateDecision::Block("ReviewOnly mode: real patch application is disabled".into());
        }

        // P0: Dry-run must pass before real patch application
        if self.require_dry_run() && !dry_run_passed {
            return GateDecision::Block("Dry-run failed: cannot apply real patch".into());
        }

        // P0: Critical review issues block application in all modes
        if has_critical_review_issues {
            return GateDecision::Block("Critical review issues found: cannot apply patch".into());
        }

        // P0: Check risk level against mode policy
        if !self.policy.should_apply_for_risk(risk_level) {
            return match self.mode {
                HarnessMode::Assisted => {
                    GateDecision::RequireApproval(format!("High risk ({:?}) requires explicit approval in Assisted mode", risk_level))
                }
                HarnessMode::Autonomous => {
                    GateDecision::Block(format!("Critical risk ({:?}) blocked in Autonomous mode", risk_level))
                }
                _ => GateDecision::Block(format!("Risk level ({:?}) exceeds mode policy", risk_level)),
            };
        }

        // P0: Rollback handle is required for real repo patching (except Benchmark with BestEffort)
        if matches!(self.policy.checkpoint_policy, CheckpointPolicy::Required) && !has_rollback {
            return GateDecision::Block("Rollback handle required for real repo patching but not available".into());
        }

        GateDecision::Allow
    }

    /// Gate check: Can we proceed without validation?
    pub fn check_validation_bypass(&self, reason: &str) -> GateDecision {
        if self.require_validation() {
            GateDecision::Block(format!("Validation cannot be bypassed in {:?} mode: {}", self.mode, reason))
        } else {
            GateDecision::Allow
        }
    }

    /// Get the mode
    pub fn mode(&self) -> HarnessMode {
        self.mode
    }

    /// Get the underlying policy
    pub fn policy(&self) -> &HarnessModePolicy {
        &self.policy
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

    #[test]
    fn test_policy_gate_review_only_blocks_real_patch() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::ReviewOnly);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Low,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::Block(_)));
    }

    #[test]
    fn test_policy_gate_assisted_allows_low_risk() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Assisted);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Low,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::Allow));
    }

    #[test]
    fn test_policy_gate_assisted_blocks_high_risk() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Assisted);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::High,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::RequireApproval(_)));
    }

    #[test]
    fn test_policy_gate_autonomous_blocks_critical_risk() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Autonomous);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Critical,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::Block(_)));
    }

    #[test]
    fn test_policy_gate_blocks_failed_dry_run() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Autonomous);
        let decision = gate.check_patch_application(
            false, // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Low,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::Block(_)));
    }

    #[test]
    fn test_policy_gate_blocks_critical_review_issues() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Autonomous);
        let decision = gate.check_patch_application(
            true, // dry_run_passed
            true, // has_critical_issues
            crate::harness::risk::RiskLevel::Low,
            true, // has_rollback
        );
        assert!(matches!(decision, GateDecision::Block(_)));
    }

    #[test]
    fn test_policy_gate_requires_rollback() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Autonomous);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Low,
            false, // NO rollback
        );
        assert!(matches!(decision, GateDecision::Block(_)));
    }

    #[test]
    fn test_policy_gate_benchmark_allows_critical_risk() {
        let gate = HarnessPolicyGate::for_mode(HarnessMode::Benchmark);
        let decision = gate.check_patch_application(
            true,  // dry_run_passed
            false, // has_critical_issues
            crate::harness::risk::RiskLevel::Critical,
            true,  // has_rollback
        );
        assert!(matches!(decision, GateDecision::Allow));
    }

    #[test]
    fn test_policy_gate_require_validation() {
        let review_gate = HarnessPolicyGate::for_mode(HarnessMode::ReviewOnly);
        let autonomous_gate = HarnessPolicyGate::for_mode(HarnessMode::Autonomous);

        // ReviewOnly doesn't require validation
        assert!(!review_gate.require_validation());

        // Autonomous requires validation
        assert!(autonomous_gate.require_validation());
    }
}
