//! Tool permission system

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Permission types for tools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolPermission {
    /// Network access (HTTP requests, etc.)
    Network,
    /// File read access
    FileRead,
    /// File write access
    FileWrite,
    /// Shell command execution
    Shell,
    /// Environment variable access
    Env,
}

/// Tool policy defining allowed permissions and approval requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    /// Set of allowed permissions
    pub allowed_permissions: HashSet<ToolPermission>,
    /// Whether approval is required for tool execution
    pub require_approval: bool,
    /// Restricted file write paths (if FileWrite is allowed)
    pub restricted_write_paths: Vec<String>,
}

impl ToolPolicy {
    /// Create a new tool policy
    pub fn new() -> Self {
        Self {
            allowed_permissions: HashSet::new(),
            require_approval: false,
            restricted_write_paths: Vec::new(),
        }
    }

    /// Add an allowed permission
    pub fn with_permission(mut self, permission: ToolPermission) -> Self {
        self.allowed_permissions.insert(permission);
        self
    }

    /// Set whether approval is required
    pub fn with_approval(mut self, require: bool) -> Self {
        self.require_approval = require;
        self
    }

    /// Add a restricted write path
    pub fn with_restricted_write_path(mut self, path: String) -> Self {
        self.restricted_write_paths.push(path);
        self
    }

    /// Check if a permission is allowed
    pub fn is_allowed(&self, permission: ToolPermission) -> bool {
        self.allowed_permissions.contains(&permission)
    }

    /// Create a conservative default policy (safe defaults)
    pub fn conservative() -> Self {
        Self::new()
            .with_permission(ToolPermission::FileRead)
            .with_restricted_write_path("prometheos-output/".to_string())
    }

    /// Create a permissive policy (for development/testing)
    pub fn permissive() -> Self {
        Self::new()
            .with_permission(ToolPermission::Network)
            .with_permission(ToolPermission::FileRead)
            .with_permission(ToolPermission::FileWrite)
            .with_permission(ToolPermission::Shell)
            .with_permission(ToolPermission::Env)
    }
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self::conservative()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_policy_creation() {
        let policy = ToolPolicy::new();
        assert!(policy.allowed_permissions.is_empty());
        assert!(!policy.require_approval);
        assert!(policy.restricted_write_paths.is_empty());
    }

    #[test]
    fn test_tool_policy_with_permission() {
        let policy = ToolPolicy::new().with_permission(ToolPermission::Network);

        assert!(policy.is_allowed(ToolPermission::Network));
        assert!(!policy.is_allowed(ToolPermission::Shell));
    }

    #[test]
    fn test_tool_policy_with_approval() {
        let policy = ToolPolicy::new().with_approval(true);

        assert!(policy.require_approval);
    }

    #[test]
    fn test_tool_policy_conservative() {
        let policy = ToolPolicy::conservative();

        assert!(policy.is_allowed(ToolPermission::FileRead));
        assert!(!policy.is_allowed(ToolPermission::Network));
        assert!(!policy.is_allowed(ToolPermission::Shell));
        assert!(!policy.is_allowed(ToolPermission::FileWrite));
        assert!(
            policy
                .restricted_write_paths
                .contains(&"prometheos-output/".to_string())
        );
    }

    #[test]
    fn test_tool_policy_permissive() {
        let policy = ToolPolicy::permissive();

        assert!(policy.is_allowed(ToolPermission::Network));
        assert!(policy.is_allowed(ToolPermission::FileRead));
        assert!(policy.is_allowed(ToolPermission::FileWrite));
        assert!(policy.is_allowed(ToolPermission::Shell));
        assert!(policy.is_allowed(ToolPermission::Env));
    }

    #[test]
    fn test_tool_policy_default() {
        let policy = ToolPolicy::default();

        assert!(policy.is_allowed(ToolPermission::FileRead));
        assert!(!policy.is_allowed(ToolPermission::Network));
    }
}
