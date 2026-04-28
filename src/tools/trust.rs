//! Trust policy for classifying tools and sources

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::TrustLevel;

/// Trust policy for a tool or source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPolicy {
    /// Source identifier (e.g., "builtin", "local", "community", "external")
    pub source: String,
    /// Trust level
    pub level: TrustLevel,
    /// Whether approval is required
    pub require_approval: bool,
}

impl TrustPolicy {
    /// Create a new trust policy
    pub fn new(source: String, level: TrustLevel) -> Self {
        let require_approval = matches!(level, TrustLevel::Untrusted | TrustLevel::External);
        Self {
            source,
            level,
            require_approval,
        }
    }

    /// Set whether approval is required
    pub fn with_approval(mut self, require: bool) -> Self {
        self.require_approval = require;
        self
    }

    /// Get the default trust policy for built-in tools
    pub fn builtin() -> Self {
        Self::new("builtin".to_string(), TrustLevel::Trusted)
    }

    /// Get the default trust policy for local tools
    pub fn local() -> Self {
        Self::new("local".to_string(), TrustLevel::Local)
    }

    /// Get the default trust policy for community tools
    pub fn community() -> Self {
        Self::new("community".to_string(), TrustLevel::Community)
    }

    /// Get the default trust policy for external tools
    pub fn external() -> Self {
        Self::new("external".to_string(), TrustLevel::External)
    }

    /// Get the default trust policy for unknown tools
    pub fn unknown() -> Self {
        Self::new("unknown".to_string(), TrustLevel::Untrusted)
    }
}

/// Trust policy registry for managing tool trust levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRegistry {
    /// Map of source to trust policy
    policies: HashMap<String, TrustPolicy>,
}

impl TrustRegistry {
    /// Create a new trust registry with default policies
    pub fn new() -> Self {
        let mut policies = HashMap::new();
        policies.insert("builtin".to_string(), TrustPolicy::builtin());
        policies.insert("local".to_string(), TrustPolicy::local());
        policies.insert("community".to_string(), TrustPolicy::community());
        policies.insert("external".to_string(), TrustPolicy::external());
        policies.insert("unknown".to_string(), TrustPolicy::unknown());

        Self { policies }
    }

    /// Get the trust policy for a source
    pub fn get_policy(&self, source: &str) -> Option<&TrustPolicy> {
        self.policies.get(source)
    }

    /// Set or update a trust policy
    pub fn set_policy(&mut self, policy: TrustPolicy) {
        self.policies.insert(policy.source.clone(), policy);
    }

    /// Get the trust level for a source
    pub fn get_trust_level(&self, source: &str) -> TrustLevel {
        self.policies
            .get(source)
            .map(|p| p.level)
            .unwrap_or(TrustLevel::Untrusted)
    }

    /// Check if a source requires approval
    pub fn requires_approval(&self, source: &str) -> bool {
        self.policies
            .get(source)
            .map(|p| p.require_approval)
            .unwrap_or(true)
    }

    /// List all registered sources
    pub fn list_sources(&self) -> Vec<String> {
        self.policies.keys().cloned().collect()
    }
}

impl Default for TrustRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_policy_creation() {
        let policy = TrustPolicy::new("test".to_string(), TrustLevel::Trusted);
        assert_eq!(policy.source, "test");
        assert_eq!(policy.level, TrustLevel::Trusted);
        assert!(!policy.require_approval);
    }

    #[test]
    fn test_trust_policy_with_approval() {
        let policy = TrustPolicy::new("test".to_string(), TrustLevel::Trusted)
            .with_approval(true);
        assert!(policy.require_approval);
    }

    #[test]
    fn test_trust_policy_defaults() {
        assert_eq!(TrustPolicy::builtin().level, TrustLevel::Trusted);
        assert_eq!(TrustPolicy::local().level, TrustLevel::Local);
        assert_eq!(TrustPolicy::community().level, TrustLevel::Community);
        assert_eq!(TrustPolicy::external().level, TrustLevel::External);
        assert_eq!(TrustPolicy::unknown().level, TrustLevel::Untrusted);
    }

    #[test]
    fn test_trust_registry_creation() {
        let registry = TrustRegistry::new();
        assert_eq!(registry.get_trust_level("builtin"), TrustLevel::Trusted);
        assert_eq!(registry.get_trust_level("local"), TrustLevel::Local);
        assert_eq!(registry.get_trust_level("unknown"), TrustLevel::Untrusted);
    }

    #[test]
    fn test_trust_registry_set_policy() {
        let mut registry = TrustRegistry::new();
        let policy = TrustPolicy::new("custom".to_string(), TrustLevel::Trusted);
        registry.set_policy(policy);

        assert_eq!(registry.get_trust_level("custom"), TrustLevel::Trusted);
    }

    #[test]
    fn test_trust_registry_requires_approval() {
        let registry = TrustRegistry::new();
        assert!(!registry.requires_approval("builtin"));
        assert!(registry.requires_approval("unknown"));
    }

    #[test]
    fn test_trust_registry_list_sources() {
        let registry = TrustRegistry::new();
        let sources = registry.list_sources();
        assert!(sources.contains(&"builtin".to_string()));
        assert!(sources.contains(&"local".to_string()));
    }
}
