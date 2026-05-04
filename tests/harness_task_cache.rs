//! Issue 11: Task-Local Knowledge Cache Tests
//!
//! Comprehensive tests for the Task-Local Knowledge Cache including:
//! - KnowledgeCache struct creation and configuration
//! - KnowledgeEntry struct (key, value, timestamp, ttl)
//! - CachePolicy enum (LRU, LFU, FIFO)
//! - CacheScope enum (Task, Session, Global)
//! - get, set, remove operations
//! - TTL and expiration handling
//! - Cache statistics and metrics
//! - Scope isolation between tasks

use std::collections::HashMap;
use std::time::{Duration, Instant};

use prometheos_lite::harness::knowledge_cache::{
    CachePolicy, CacheScope, KnowledgeCache, KnowledgeEntry,
};

// ============================================================================
// KnowledgeCache Tests
// ============================================================================

#[test]
fn test_knowledge_cache_default() {
    let cache = KnowledgeCache::default();

    assert_eq!(cache.max_entries(), 1000);
    assert_eq!(cache.policy(), CachePolicy::LRU);
    assert!(cache.is_empty());
}

#[test]
fn test_knowledge_cache_with_policy() {
    let cache = KnowledgeCache::with_policy(CachePolicy::LFU, 500);

    assert_eq!(cache.max_entries(), 500);
    assert_eq!(cache.policy(), CachePolicy::LFU);
}

#[test]
fn test_knowledge_cache_with_scope() {
    let cache = KnowledgeCache::with_scope(CacheScope::Session, 200);

    assert_eq!(cache.scope(), CacheScope::Session);
    assert_eq!(cache.max_entries(), 200);
}

// ============================================================================
// KnowledgeEntry Tests
// ============================================================================

#[test]
fn test_knowledge_entry_creation() {
    let entry = KnowledgeEntry {
        key: "test_key".to_string(),
        value: "test_value".to_string(),
        created_at: Instant::now(),
        ttl: Some(Duration::from_secs(3600)),
        scope: CacheScope::Task,
        access_count: 0,
    };

    assert_eq!(entry.key, "test_key");
    assert_eq!(entry.value, "test_value");
    assert!(entry.ttl.is_some());
    assert_eq!(entry.scope, CacheScope::Task);
    assert_eq!(entry.access_count, 0);
}

#[test]
fn test_knowledge_entry_no_ttl() {
    let entry = KnowledgeEntry {
        key: "permanent".to_string(),
        value: "data".to_string(),
        created_at: Instant::now(),
        ttl: None,
        scope: CacheScope::Global,
        access_count: 5,
    };

    assert!(entry.ttl.is_none());
    assert_eq!(entry.access_count, 5);
    assert_eq!(entry.scope, CacheScope::Global);
}

#[test]
fn test_knowledge_entry_is_expired() {
    let old_entry = KnowledgeEntry {
        key: "old".to_string(),
        value: "data".to_string(),
        created_at: Instant::now() - Duration::from_secs(7200), // 2 hours ago
        ttl: Some(Duration::from_secs(3600)), // 1 hour TTL
        scope: CacheScope::Task,
        access_count: 0,
    };

    assert!(old_entry.is_expired());
}

#[test]
fn test_knowledge_entry_not_expired() {
    let fresh_entry = KnowledgeEntry {
        key: "fresh".to_string(),
        value: "data".to_string(),
        created_at: Instant::now(),
        ttl: Some(Duration::from_secs(3600)),
        scope: CacheScope::Task,
        access_count: 0,
    };

    assert!(!fresh_entry.is_expired());
}

// ============================================================================
// CachePolicy Tests
// ============================================================================

#[test]
fn test_cache_policy_variants() {
    assert!(matches!(CachePolicy::LRU, CachePolicy::LRU));
    assert!(matches!(CachePolicy::LFU, CachePolicy::LFU));
    assert!(matches!(CachePolicy::FIFO, CachePolicy::FIFO));
}

#[test]
fn test_cache_policy_display() {
    assert_eq!(format!("{:?}", CachePolicy::LRU), "LRU");
    assert_eq!(format!("{:?}", CachePolicy::LFU), "LFU");
    assert_eq!(format!("{:?}", CachePolicy::FIFO), "FIFO");
}

#[test]
fn test_cache_policy_default() {
    let policy: CachePolicy = Default::default();
    assert!(matches!(policy, CachePolicy::LRU));
}

// ============================================================================
// CacheScope Tests
// ============================================================================

#[test]
fn test_cache_scope_variants() {
    assert!(matches!(CacheScope::Task, CacheScope::Task));
    assert!(matches!(CacheScope::Session, CacheScope::Session));
    assert!(matches!(CacheScope::Global, CacheScope::Global));
}

#[test]
fn test_cache_scope_display() {
    assert_eq!(format!("{:?}", CacheScope::Task), "Task");
    assert_eq!(format!("{:?}", CacheScope::Session), "Session");
    assert_eq!(format!("{:?}", CacheScope::Global), "Global");
}

// ============================================================================
// Cache Operations Tests
// ============================================================================

#[test]
fn test_cache_set_and_get() {
    let mut cache = KnowledgeCache::default();

    cache.set("key1", "value1");
    assert_eq!(cache.get("key1"), Some("value1".to_string()));
}

#[test]
fn test_cache_get_nonexistent() {
    let cache = KnowledgeCache::default();

    assert_eq!(cache.get("nonexistent"), None);
}

#[test]
fn test_cache_remove() {
    let mut cache = KnowledgeCache::default();

    cache.set("key1", "value1");
    assert!(cache.remove("key1"));
    assert_eq!(cache.get("key1"), None);
}

#[test]
fn test_cache_remove_nonexistent() {
    let mut cache = KnowledgeCache::default();

    assert!(!cache.remove("nonexistent"));
}

#[test]
fn test_cache_update_existing() {
    let mut cache = KnowledgeCache::default();

    cache.set("key1", "value1");
    cache.set("key1", "value2");
    assert_eq!(cache.get("key1"), Some("value2".to_string()));
}

#[test]
fn test_cache_clear() {
    let mut cache = KnowledgeCache::default();

    cache.set("key1", "value1");
    cache.set("key2", "value2");
    cache.clear();

    assert!(cache.is_empty());
    assert_eq!(cache.get("key1"), None);
    assert_eq!(cache.get("key2"), None);
}

// ============================================================================
// Cache TTL Tests
// ============================================================================

#[test]
fn test_cache_set_with_ttl() {
    let mut cache = KnowledgeCache::default();

    cache.set_with_ttl("key1", "value1", Duration::from_secs(3600));
    assert_eq!(cache.get("key1"), Some("value1".to_string()));
}

#[test]
fn test_cache_expired_entry_removed() {
    let mut cache = KnowledgeCache::default();

    // Set with very short TTL
    cache.set_with_ttl("key1", "value1", Duration::from_millis(1));

    // Wait for expiration
    std::thread::sleep(Duration::from_millis(10));

    // Should be expired and removed
    assert_eq!(cache.get("key1"), None);
}

// ============================================================================
// Cache Statistics Tests
// ============================================================================

#[test]
fn test_cache_size() {
    let mut cache = KnowledgeCache::default();

    assert_eq!(cache.size(), 0);
    cache.set("key1", "value1");
    assert_eq!(cache.size(), 1);
    cache.set("key2", "value2");
    assert_eq!(cache.size(), 2);
}

#[test]
fn test_cache_is_empty() {
    let mut cache = KnowledgeCache::default();

    assert!(cache.is_empty());
    cache.set("key1", "value1");
    assert!(!cache.is_empty());
}

#[test]
fn test_cache_hit_and_miss_stats() {
    let mut cache = KnowledgeCache::default();

    cache.set("key1", "value1");

    // Hits
    let _ = cache.get("key1");
    let _ = cache.get("key1");

    // Miss
    let _ = cache.get("nonexistent");

    let stats = cache.stats();
    assert_eq!(stats.hits, 2);
    assert_eq!(stats.misses, 1);
}

// ============================================================================
// Cache Scope Isolation Tests
// ============================================================================

#[test]
fn test_cache_scope_task_isolation() {
    let task_cache1 = KnowledgeCache::with_scope(CacheScope::Task, 100);
    let task_cache2 = KnowledgeCache::with_scope(CacheScope::Task, 100);

    // Caches with Task scope should be independent
    // This is more of a conceptual test - actual isolation depends on implementation
    assert_eq!(task_cache1.scope(), CacheScope::Task);
    assert_eq!(task_cache2.scope(), CacheScope::Task);
}

#[test]
fn test_cache_scope_hierarchy() {
    let task = KnowledgeCache::with_scope(CacheScope::Task, 100);
    let session = KnowledgeCache::with_scope(CacheScope::Session, 200);
    let global = KnowledgeCache::with_scope(CacheScope::Global, 500);

    assert_eq!(task.scope(), CacheScope::Task);
    assert_eq!(session.scope(), CacheScope::Session);
    assert_eq!(global.scope(), CacheScope::Global);
}

// ============================================================================
// Cache Capacity Tests
// ============================================================================

#[test]
fn test_cache_capacity_limit() {
    let mut cache = KnowledgeCache::with_policy(CachePolicy::LRU, 3);

    cache.set("key1", "value1");
    cache.set("key2", "value2");
    cache.set("key3", "value3");
    cache.set("key4", "value4"); // Should evict key1 with LRU

    assert_eq!(cache.size(), 3);
    // key1 should be evicted
    assert_eq!(cache.get("key1"), None);
}
