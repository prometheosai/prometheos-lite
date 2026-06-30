//! Idempotency keys for preventing duplicate side effects

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Idempotency key for preventing duplicate side effects
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyKey {
    /// Unique key for the operation
    pub key: String,
    /// Run identifier
    pub run_id: String,
    /// Node identifier
    pub node_id: String,
    /// Hash of the operation (content + parameters)
    pub operation_hash: String,
}

impl IdempotencyKey {
    /// Create a new idempotency key
    pub fn new(run_id: String, node_id: String, operation_hash: String) -> Self {
        let key = Self::compute_key(&run_id, &node_id, &operation_hash);

        Self {
            key,
            run_id,
            node_id,
            operation_hash,
        }
    }

    /// Compute a unique key from run_id, node_id, and operation_hash
    pub fn compute_key(run_id: &str, node_id: &str, operation_hash: &str) -> String {
        let combined = format!("{}:{}:{}", run_id, node_id, operation_hash);
        let mut hasher = DefaultHasher::new();
        combined.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Compute an operation hash from content and parameters
    pub fn compute_operation_hash(content: &str, parameters: &serde_json::Value) -> String {
        let combined = format!(
            "{}:{}",
            content,
            serde_json::to_string(parameters).unwrap_or_default()
        );
        let mut hasher = DefaultHasher::new();
        combined.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Check if this key matches another
    pub fn matches(&self, other: &IdempotencyKey) -> bool {
        self.key == other.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idempotency_key_creation() {
        let key = IdempotencyKey::new(
            "run1".to_string(),
            "node1".to_string(),
            "hash123".to_string(),
        );

        assert_eq!(key.run_id, "run1");
        assert_eq!(key.node_id, "node1");
        assert_eq!(key.operation_hash, "hash123");
        assert!(!key.key.is_empty());
    }

    #[test]
    fn test_key_computation() {
        let key1 = IdempotencyKey::compute_key("run1", "node1", "hash123");
        let key2 = IdempotencyKey::compute_key("run1", "node1", "hash123");
        let key3 = IdempotencyKey::compute_key("run1", "node1", "hash456");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_operation_hash_computation() {
        let content = "write file";
        let params1 = serde_json::json!({"path": "test.txt", "content": "hello"});
        let params2 = serde_json::json!({"path": "test.txt", "content": "hello"});
        let params3 = serde_json::json!({"path": "test.txt", "content": "world"});

        let hash1 = IdempotencyKey::compute_operation_hash(content, &params1);
        let hash2 = IdempotencyKey::compute_operation_hash(content, &params2);
        let hash3 = IdempotencyKey::compute_operation_hash(content, &params3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_key_matching() {
        let key1 = IdempotencyKey::new(
            "run1".to_string(),
            "node1".to_string(),
            "hash123".to_string(),
        );
        let key2 = IdempotencyKey::new(
            "run1".to_string(),
            "node1".to_string(),
            "hash123".to_string(),
        );
        let key3 = IdempotencyKey::new(
            "run1".to_string(),
            "node1".to_string(),
            "hash456".to_string(),
        );

        assert!(key1.matches(&key2));
        assert!(!key1.matches(&key3));
    }
}
