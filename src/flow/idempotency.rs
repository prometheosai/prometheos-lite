//! Idempotency keys for preventing duplicate side effects

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Version tag carried by durable canonical keys. The explicit prefix lets a
/// reader distinguish a v2 key from a legacy `DefaultHasher` key instead of
/// silently treating the two encodings as interchangeable.
const CANONICAL_KEY_VERSION: &str = "idem-v2";

/// Stable, platform-independent 64-bit FNV-1a. Unlike `DefaultHasher`, its
/// output is specified and does not change with the Rust toolchain version.
fn stable_fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

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

    /// Compute the canonical durable key from run_id, node_id and operation_hash.
    ///
    /// The preimage is length-prefixed and version-tagged, so no two distinct
    /// triples can share it. The digest is a specified, stable FNV-1a over the
    /// canonical bytes; the version prefix makes a legacy key distinguishable.
    /// This is an identity encoding, not a security or atomic-admission control.
    pub fn compute_key(run_id: &str, node_id: &str, operation_hash: &str) -> String {
        let mut preimage = String::from(CANONICAL_KEY_VERSION);
        for field in [run_id, node_id, operation_hash] {
            preimage.push('|');
            preimage.push_str(&field.len().to_string());
            preimage.push(':');
            preimage.push_str(field);
        }
        format!(
            "{}-{:016x}",
            CANONICAL_KEY_VERSION,
            stable_fnv1a64(preimage.as_bytes())
        )
    }

    /// Reproduce the pre-v2 key for an already persisted outbox entry.
    ///
    /// Kept only so existing `tool_outbox` rows remain readable; new durable
    /// keys must use [`IdempotencyKey::compute_key`]. The colon-joined preimage
    /// is ambiguous and this function is explicitly deprecated for new writes.
    #[deprecated(note = "legacy read compatibility only; use compute_key for new keys")]
    pub fn compute_legacy_key(run_id: &str, node_id: &str, operation_hash: &str) -> String {
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

    /// RED regression (#343): the legacy colon-joined preimage is ambiguous.
    /// ("a:b", "c", "d") and ("a", "b:c", "d") share the same preimage, so a
    /// hash over that string cannot distinguish two different operations.
    #[test]
    fn distinct_tuples_with_colliding_colon_preimage_have_distinct_keys() {
        let first = IdempotencyKey::compute_key("a:b", "c", "d");
        let second = IdempotencyKey::compute_key("a", "b:c", "d");
        assert_ne!(
            first, second,
            "ambiguous tuple encoding collapsed two distinct operations"
        );
    }

    #[test]
    fn canonical_key_is_versioned_and_stable() {
        let first = IdempotencyKey::compute_key("run1", "node1", "hash123");
        let second = IdempotencyKey::compute_key("run1", "node1", "hash123");
        assert_eq!(first, second);
        assert!(
            first.starts_with("idem-v2-"),
            "expected explicit version prefix: {first}"
        );
        let digest = first.strip_prefix("idem-v2-").expect("version prefix");
        assert_eq!(digest.len(), 16, "stable fixed-width digest: {digest}");
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn empty_and_delimiter_inputs_do_not_collapse() {
        let empty = IdempotencyKey::compute_key("", "", "");
        let delimited = IdempotencyKey::compute_key("|", "", "");
        let shifted = IdempotencyKey::compute_key("", "|", "");
        assert_ne!(empty, delimited);
        assert_ne!(empty, shifted);
        assert_ne!(delimited, shifted);
    }

    #[test]
    fn operation_hash_is_independent_of_json_object_key_order() {
        let first = serde_json::json!({"alpha": 1, "beta": 2});
        let second = serde_json::json!({"beta": 2, "alpha": 1});
        assert_eq!(
            IdempotencyKey::compute_operation_hash("node", &first),
            IdempotencyKey::compute_operation_hash("node", &second)
        );
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_key_is_retained_only_for_old_outbox_reads() {
        let legacy = IdempotencyKey::compute_legacy_key("a:b", "c", "d");
        let canonical = IdempotencyKey::compute_key("a:b", "c", "d");
        assert!(!legacy.starts_with("idem-v2-"));
        assert!(canonical.starts_with("idem-v2-"));
        assert_ne!(legacy, canonical);
        // The legacy encoding remains exactly reproducible for persisted rows.
        assert_eq!(legacy, IdempotencyKey::compute_legacy_key("a:b", "c", "d"));
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
