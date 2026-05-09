//! Tool context for enforcing guardrails at tool execution boundaries

use crate::tools::permissions::ToolPolicy;
use serde::{Deserialize, Serialize};

/// Trust level for tools and sources
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Fully trusted (built-in core tools)
    Trusted,
    /// Local tools and flows
    Local,
    /// Community-contributed tools
    Community,
    /// External downloaded tools
    External,
    /// Unknown or untrusted tools
    Untrusted,
}

impl Default for TrustLevel {
    fn default() -> Self {
        TrustLevel::Local
    }
}

/// Approval policy for tool execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalPolicy {
    /// No approval required
    Auto,
    /// Approval required for all tools
    RequireForTools,
    /// Approval required for side-effecting tools
    RequireForSideEffects,
    /// Approval required for untrusted tools
    RequireForUntrusted,
    /// Manual approval for everything
    ManualAll,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        ApprovalPolicy::Auto
    }
}

/// Execution context for tool calls with guardrail information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    /// Run identifier
    pub run_id: String,
    /// Trace identifier
    pub trace_id: String,
    /// Node identifier
    pub node_id: String,
    /// Tool name being executed
    pub tool_name: String,
    /// Tool policy for permission checks
    pub policy: ToolPolicy,
    /// Trust level of the tool
    pub trust_level: TrustLevel,
    /// Approval policy for this execution
    pub approval_policy: ApprovalPolicy,
    /// Optional idempotency key for preventing duplicate side effects
    pub idempotency_key: Option<String>,
}

impl ToolContext {
    /// Create a new tool context
    pub fn new(
        run_id: String,
        trace_id: String,
        node_id: String,
        tool_name: String,
        policy: ToolPolicy,
    ) -> Self {
        Self {
            run_id,
            trace_id,
            node_id,
            tool_name,
            policy,
            trust_level: TrustLevel::default(),
            approval_policy: ApprovalPolicy::default(),
            idempotency_key: None,
        }
    }

    /// Set the trust level
    pub fn with_trust_level(mut self, level: TrustLevel) -> Self {
        self.trust_level = level;
        self
    }

    /// Set the approval policy
    pub fn with_approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.approval_policy = policy;
        self
    }

    /// Set the idempotency key
    pub fn with_idempotency_key(mut self, key: String) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    /// Check if approval is required based on policy and trust level
    pub fn requires_approval(&self) -> bool {
        match self.approval_policy {
            ApprovalPolicy::Auto => false,
            ApprovalPolicy::ManualAll => true,
            ApprovalPolicy::RequireForTools => true,
            ApprovalPolicy::RequireForUntrusted => {
                matches!(
                    self.trust_level,
                    TrustLevel::Untrusted | TrustLevel::External
                )
            }
            ApprovalPolicy::RequireForSideEffects => {
                // This will be checked based on tool permissions
                self.policy
                    .is_allowed(crate::tools::permissions::ToolPermission::FileWrite)
                    || self
                        .policy
                        .is_allowed(crate::tools::permissions::ToolPermission::Shell)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::permissions::{ToolPermission, ToolPolicy};

    #[test]
    fn test_tool_context_creation() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        );

        assert_eq!(context.run_id, "run1");
        assert_eq!(context.trace_id, "trace1");
        assert_eq!(context.node_id, "node1");
        assert_eq!(context.tool_name, "test_tool");
    }

    #[test]
    fn test_tool_context_with_trust_level() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_trust_level(TrustLevel::Trusted);

        assert_eq!(context.trust_level, TrustLevel::Trusted);
    }

    #[test]
    fn test_tool_context_with_approval_policy() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_approval_policy(ApprovalPolicy::ManualAll);

        assert_eq!(context.approval_policy, ApprovalPolicy::ManualAll);
    }

    #[test]
    fn test_requires_approval_auto() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_approval_policy(ApprovalPolicy::Auto);

        assert!(!context.requires_approval());
    }

    #[test]
    fn test_requires_approval_manual_all() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_approval_policy(ApprovalPolicy::ManualAll);

        assert!(context.requires_approval());
    }

    #[test]
    fn test_requires_approval_for_untrusted() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_trust_level(TrustLevel::Untrusted)
        .with_approval_policy(ApprovalPolicy::RequireForUntrusted);

        assert!(context.requires_approval());
    }

    #[test]
    fn test_requires_approval_for_trusted() {
        let policy = ToolPolicy::new();
        let context = ToolContext::new(
            "run1".to_string(),
            "trace1".to_string(),
            "node1".to_string(),
            "test_tool".to_string(),
            policy,
        )
        .with_trust_level(TrustLevel::Trusted)
        .with_approval_policy(ApprovalPolicy::RequireForUntrusted);

        assert!(!context.requires_approval());
    }
}
