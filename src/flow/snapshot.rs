//! Flow snapshot and versioning for reproducible execution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Flow snapshot for versioning and reproducible execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSnapshot {
    /// Flow name
    pub flow_name: String,
    /// Flow version
    pub flow_version: String,
    /// Source hash for validation
    pub source_hash: String,
    /// Original flow source text
    pub source_text: String,
    /// Timestamp when snapshot was created
    pub created_at: DateTime<Utc>,
}

impl FlowSnapshot {
    /// Create a new flow snapshot
    pub fn new(flow_name: String, flow_version: String, source_text: String) -> Self {
        let source_hash = Self::compute_hash(&source_text);

        Self {
            flow_name,
            flow_version,
            source_hash,
            source_text,
            created_at: Utc::now(),
        }
    }

    /// Compute a hash for the flow source
    pub fn compute_hash(source: &str) -> String {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Verify that the source matches the stored hash
    pub fn verify_hash(&self, source: &str) -> bool {
        let computed_hash = Self::compute_hash(source);
        computed_hash == self.source_hash
    }

    /// Check if this snapshot matches another
    pub fn matches(&self, other: &FlowSnapshot) -> bool {
        self.source_hash == other.source_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_snapshot_creation() {
        let source = "nodes:\n  - id: test\n    type: llm";
        let snapshot = FlowSnapshot::new(
            "test_flow".to_string(),
            "1.0.0".to_string(),
            source.to_string(),
        );

        assert_eq!(snapshot.flow_name, "test_flow");
        assert_eq!(snapshot.flow_version, "1.0.0");
        assert_eq!(snapshot.source_text, source);
        assert!(!snapshot.source_hash.is_empty());
    }

    #[test]
    fn test_hash_computation() {
        let source1 = "nodes:\n  - id: test";
        let source2 = "nodes:\n  - id: test";
        let source3 = "nodes:\n  - id: different";

        let hash1 = FlowSnapshot::compute_hash(source1);
        let hash2 = FlowSnapshot::compute_hash(source2);
        let hash3 = FlowSnapshot::compute_hash(source3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_hash_verification() {
        let source = "nodes:\n  - id: test";
        let snapshot = FlowSnapshot::new(
            "test_flow".to_string(),
            "1.0.0".to_string(),
            source.to_string(),
        );

        assert!(snapshot.verify_hash(source));
        assert!(!snapshot.verify_hash("different source"));
    }

    #[test]
    fn test_snapshot_matching() {
        let source = "nodes:\n  - id: test";
        let snapshot1 = FlowSnapshot::new(
            "test_flow".to_string(),
            "1.0.0".to_string(),
            source.to_string(),
        );
        let snapshot2 = FlowSnapshot::new(
            "test_flow".to_string(),
            "1.0.0".to_string(),
            source.to_string(),
        );
        let snapshot3 = FlowSnapshot::new(
            "test_flow".to_string(),
            "1.0.0".to_string(),
            "different source".to_string(),
        );

        assert!(snapshot1.matches(&snapshot2));
        assert!(!snapshot1.matches(&snapshot3));
    }
}
