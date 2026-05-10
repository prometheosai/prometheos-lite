use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheKey {
    pub scope: String,
    pub key: String,
}

impl CacheKey {
    pub fn new(scope: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            key: key.into(),
        }
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        format!("{}:{}", self.scope, self.key).hash(state);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheEntry {
    pub key: CacheKey,
    pub value: CacheValue,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub access_count: u32,
    pub last_accessed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CacheValue {
    String(String),
    Bytes(Vec<u8>),
    Json(serde_json::Value),
    Number(i64),
    Bool(bool),
    List(Vec<CacheValue>),
    Map(HashMap<String, CacheValue>),
}

impl CacheValue {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            CacheValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match self {
            CacheValue::Json(v) => Some(v),
            _ => None,
        }
    }

    pub fn estimate_size(&self) -> usize {
        match self {
            CacheValue::String(s) => s.len(),
            CacheValue::Bytes(b) => b.len(),
            CacheValue::Json(v) => v.to_string().len(),
            CacheValue::Number(_) => 8,
            CacheValue::Bool(_) => 1,
            CacheValue::List(items) => items.iter().map(|i| i.estimate_size()).sum(),
            CacheValue::Map(m) => m.iter().map(|(k, v)| k.len() + v.estimate_size()).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheScope {
    Task,
    Project,
    Global,
}

impl CacheScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheScope::Task => "task",
            CacheScope::Project => "project",
            CacheScope::Global => "global",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub entries_count: usize,
    pub total_size_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub hit_rate: f64,
}

pub struct TaskLocalKnowledgeCache {
    scopes: Arc<RwLock<HashMap<String, HashMap<String, CacheEntry>>>>,
    stats: Arc<RwLock<CacheStats>>,
    persistence_path: Option<PathBuf>,
    max_entries: usize,
    default_ttl: Option<Duration>,
    task_id: String,
}

impl TaskLocalKnowledgeCache {
    pub fn new(task_id: String, max_entries: usize, persistence_path: Option<PathBuf>) -> Self {
        Self {
            scopes: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(CacheStats {
                entries_count: 0,
                total_size_bytes: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
                hit_rate: 0.0,
            })),
            persistence_path,
            max_entries,
            default_ttl: Some(Duration::from_secs(3600)), // 1 hour default
            task_id,
        }
    }

    pub async fn get(&self, scope: CacheScope, key: &str) -> Option<CacheValue> {
        let scope_str = scope.as_str().to_string();
        let mut scopes = self.scopes.write().await;
        let mut stats = self.stats.write().await;

        let now = self.now();

        if let Some(scope_entries) = scopes.get_mut(&scope_str) {
            if let Some(entry) = scope_entries.get_mut(key) {
                if let Some(expires) = entry.expires_at {
                    if now > expires {
                        scope_entries.remove(key);
                        stats.entries_count = stats.entries_count.saturating_sub(1);
                        stats.misses += 1;
                        self.update_hit_rate(&mut stats);
                        return None;
                    }
                }

                entry.access_count += 1;
                entry.last_accessed = now;
                stats.hits += 1;
                self.update_hit_rate(&mut stats);

                return Some(entry.value.clone());
            }
        }

        stats.misses += 1;
        self.update_hit_rate(&mut stats);
        None
    }

    pub async fn set(
        &self,
        scope: CacheScope,
        key: impl Into<String>,
        value: CacheValue,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let scope_str = scope.as_str().to_string();
        let key_str = key.into();
        let now = self.now();

        let expires_at = ttl.or(self.default_ttl).map(|d| now + d.as_secs() as u64);

        let entry = CacheEntry {
            key: CacheKey::new(scope_str.clone(), key_str.clone()),
            value,
            created_at: now,
            expires_at,
            access_count: 0,
            last_accessed: now,
        };

        let mut scopes = self.scopes.write().await;
        let mut stats = self.stats.write().await;

        let scope_entries = scopes.entry(scope_str.clone()).or_default();

        if scope_entries.len() >= self.max_entries && !scope_entries.contains_key(&key_str) {
            self.evict_oldest(scope_entries, &mut stats);
        }

        let size = entry.value.estimate_size();
        scope_entries.insert(key_str, entry);
        stats.entries_count += 1;
        stats.total_size_bytes += size;

        drop(scopes);
        drop(stats);

        self.persist_if_needed().await?;

        Ok(())
    }

    pub async fn set_json<T: Serialize>(
        &self,
        scope: CacheScope,
        key: impl Into<String>,
        value: &T,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let json_value = serde_json::to_value(value)?;
        self.set(scope, key, CacheValue::Json(json_value), ttl)
            .await
    }

    pub async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        scope: CacheScope,
        key: &str,
    ) -> Result<Option<T>> {
        if let Some(value) = self.get(scope, key).await {
            if let Some(json) = value.as_json() {
                return Ok(Some(serde_json::from_value(json.clone())?));
            }
        }
        Ok(None)
    }

    pub async fn delete(&self, scope: CacheScope, key: &str) -> bool {
        let scope_str = scope.as_str().to_string();
        let mut scopes = self.scopes.write().await;
        let mut stats = self.stats.write().await;

        if let Some(scope_entries) = scopes.get_mut(&scope_str) {
            if let Some(entry) = scope_entries.remove(key) {
                let size = entry.value.estimate_size();
                stats.entries_count = stats.entries_count.saturating_sub(1);
                stats.total_size_bytes = stats.total_size_bytes.saturating_sub(size);
                return true;
            }
        }

        false
    }

    pub async fn clear(&self, scope: Option<CacheScope>) -> Result<()> {
        let mut scopes = self.scopes.write().await;
        let mut stats = self.stats.write().await;

        if let Some(s) = scope {
            let scope_str = s.as_str().to_string();
            if let Some(entries) = scopes.remove(&scope_str) {
                stats.entries_count = stats.entries_count.saturating_sub(entries.len());
            }
        } else {
            stats.entries_count = 0;
            stats.total_size_bytes = 0;
            scopes.clear();
        }

        drop(scopes);
        drop(stats);

        self.persist_if_needed().await?;
        Ok(())
    }

    pub async fn get_stats(&self) -> CacheStats {
        self.stats.read().await.clone()
    }

    pub async fn keys(&self, scope: CacheScope) -> Vec<String> {
        let scope_str = scope.as_str().to_string();
        let scopes = self.scopes.read().await;

        if let Some(entries) = scopes.get(&scope_str) {
            entries.keys().cloned().collect()
        } else {
            vec![]
        }
    }

    pub async fn export(&self, path: &Path) -> Result<()> {
        let scopes = self.scopes.read().await;
        let export_data = serde_json::to_string_pretty(&*scopes)?;
        tokio::fs::write(path, export_data).await?;
        Ok(())
    }

    pub async fn import(&self, path: &Path) -> Result<()> {
        let data = tokio::fs::read_to_string(path).await?;
        let imported: HashMap<String, HashMap<String, CacheEntry>> = serde_json::from_str(&data)?;

        let mut scopes = self.scopes.write().await;
        let mut stats = self.stats.write().await;

        for (scope, entries) in imported {
            let scope_entries = scopes.entry(scope).or_default();
            for (key, entry) in entries {
                let size = entry.value.estimate_size();
                scope_entries.insert(key, entry);
                stats.entries_count += 1;
                stats.total_size_bytes += size;
            }
        }

        Ok(())
    }

    fn evict_oldest(&self, entries: &mut HashMap<String, CacheEntry>, stats: &mut CacheStats) {
        if let Some((oldest_key, oldest_entry)) = entries
            .iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, v)| (k.clone(), v.clone()))
        {
            entries.remove(&oldest_key);
            let size = oldest_entry.value.estimate_size();
            stats.entries_count = stats.entries_count.saturating_sub(1);
            stats.total_size_bytes = stats.total_size_bytes.saturating_sub(size);
            stats.evictions += 1;
        }
    }

    fn update_hit_rate(&self, stats: &mut CacheStats) {
        let total = stats.hits + stats.misses;
        if total > 0 {
            stats.hit_rate = stats.hits as f64 / total as f64;
        }
    }

    fn now(&self) -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    async fn persist_if_needed(&self) -> Result<()> {
        if let Some(ref path) = self.persistence_path {
            let cache_path = path.join(format!("task_cache_{}.json", self.task_id));
            self.export(&cache_path).await?;
        }
        Ok(())
    }
}

pub struct KnowledgeCacheManager {
    caches: Arc<RwLock<HashMap<String, Arc<TaskLocalKnowledgeCache>>>>,
    default_max_entries: usize,
    persistence_root: Option<PathBuf>,
}

impl KnowledgeCacheManager {
    pub fn new(default_max_entries: usize, persistence_root: Option<PathBuf>) -> Self {
        Self {
            caches: Arc::new(RwLock::new(HashMap::new())),
            default_max_entries,
            persistence_root,
        }
    }

    pub async fn get_or_create(&self, task_id: &str) -> Arc<TaskLocalKnowledgeCache> {
        let mut caches = self.caches.write().await;

        if let Some(cache) = caches.get(task_id) {
            return cache.clone();
        }

        let cache = Arc::new(TaskLocalKnowledgeCache::new(
            task_id.to_string(),
            self.default_max_entries,
            self.persistence_root.clone(),
        ));

        caches.insert(task_id.to_string(), cache.clone());
        cache
    }

    pub async fn remove(&self, task_id: &str) -> Result<()> {
        let mut caches = self.caches.write().await;

        if let Some(cache) = caches.remove(task_id) {
            cache.clear(None).await?;
        }

        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<usize> {
        let caches = self.caches.read().await;
        let mut total_cleaned = 0;

        for (_task_id, cache) in caches.iter() {
            let keys = cache.keys(CacheScope::Task).await;
            let now = cache.now();

            for key in keys {
                let should_delete = {
                    let scopes = cache.scopes.read().await;
                    if let Some(task_entries) = scopes.get("task") {
                        if let Some(entry) = task_entries.get(&key) {
                            entry.expires_at.map(|exp| now > exp).unwrap_or(false)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if should_delete {
                    cache.delete(CacheScope::Task, &key).await;
                    total_cleaned += 1;
                }
            }
        }

        Ok(total_cleaned)
    }
}

pub fn create_default_cache(task_id: &str) -> TaskLocalKnowledgeCache {
    TaskLocalKnowledgeCache::new(
        task_id.to_string(),
        1000,
        Some(PathBuf::from(".cache/knowledge")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_cache_operations() {
        let cache = TaskLocalKnowledgeCache::new("test-task".into(), 100, None);

        cache
            .set(
                CacheScope::Task,
                "key1",
                CacheValue::String("value1".into()),
                None,
            )
            .await
            .unwrap();

        let value = cache.get(CacheScope::Task, "key1").await;
        assert!(value.is_some());
        assert_eq!(value.unwrap().as_string(), Some("value1"));

        let stats = cache.get_stats().await;
        assert_eq!(stats.entries_count, 1);
        assert_eq!(stats.hits, 1);
    }

    #[tokio::test]
    async fn test_json_cache() {
        let cache = TaskLocalKnowledgeCache::new("test-task".into(), 100, None);

        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct TestData {
            name: String,
            count: i32,
        }

        let data = TestData {
            name: "test".into(),
            count: 42,
        };

        cache
            .set_json(CacheScope::Task, "data", &data, None)
            .await
            .unwrap();

        let retrieved: Option<TestData> = cache.get_json(CacheScope::Task, "data").await.unwrap();
        assert_eq!(retrieved, Some(data));
    }
}
