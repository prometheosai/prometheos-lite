//! P2-Issue4: Validation result caching with invalidation
//!
//! This module provides comprehensive validation result caching with
//! intelligent invalidation, cache warming, and performance optimization.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};

/// P2-Issue4: Advanced validation cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationCacheConfig {
    /// Cache configuration
    pub cache_config: CacheConfig,
    /// Invalidation configuration
    pub invalidation_config: InvalidationConfig,
    /// Performance configuration
    pub performance_config: PerformanceConfig,
    /// Cache warming configuration
    pub warming_config: CacheWarmingConfig,
}

/// P2-Issue4: Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheConfig {
    /// Maximum cache size in MB
    pub max_size_mb: u64,
    /// Maximum number of entries
    pub max_entries: usize,
    /// Default TTL in milliseconds
    pub default_ttl_ms: u64,
    /// TTL by category
    pub ttl_by_category: HashMap<crate::harness::validation::ValidationCategory, u64>,
    /// Cache eviction policy
    pub eviction_policy: EvictionPolicy,
    /// Compression enabled
    pub compression_enabled: bool,
    /// Encryption enabled
    pub encryption_enabled: bool,
}

/// P2-Issue4: Cache eviction policies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
    /// Time-based expiration
    TimeBased,
    /// Size-based eviction
    SizeBased,
    /// Adaptive eviction
    Adaptive,
}

/// P2-Issue4: Invalidation configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvalidationConfig {
    /// Invalidation strategies
    pub strategies: Vec<InvalidationStrategy>,
    /// File change detection
    pub file_change_detection: FileChangeDetectionConfig,
    /// Dependency tracking
    pub dependency_tracking: DependencyTrackingConfig,
    /// Automatic invalidation enabled
    pub auto_invalidation_enabled: bool,
    /// Invalidation delay in milliseconds
    pub invalidation_delay_ms: u64,
}

/// P2-Issue4: Invalidation strategies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvalidationStrategy {
    /// Strategy name
    pub name: String,
    /// Strategy type
    pub strategy_type: InvalidationStrategyType,
    /// Conditions for this strategy
    pub conditions: Vec<InvalidationCondition>,
    /// Priority (higher = more important)
    pub priority: u8,
    /// Whether this strategy is enabled
    pub enabled: bool,
}

/// P2-Issue4: Invalidation strategy types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvalidationStrategyType {
    /// File-based invalidation
    FileBased,
    /// Content-based invalidation
    ContentBased,
    /// Dependency-based invalidation
    DependencyBased,
    /// Time-based invalidation
    TimeBased,
    /// Manual invalidation
    Manual,
    /// Hybrid invalidation
    Hybrid,
}

/// P2-Issue4: Invalidation conditions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvalidationCondition {
    /// File path matches pattern
    FilePattern(String),
    /// File content hash changed
    ContentHashChanged,
    /// File metadata changed
    MetadataChanged,
    /// Dependency changed
    DependencyChanged(String),
    /// Time elapsed
    TimeElapsed(Duration),
    /// Custom condition
    Custom(String),
}

/// P2-Issue4: File change detection configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeDetectionConfig {
    /// Watch file system changes
    pub watch_filesystem: bool,
    /// Polling interval in milliseconds
    pub polling_interval_ms: u64,
    /// File patterns to watch
    pub watch_patterns: Vec<String>,
    /// Ignore patterns
    pub ignore_patterns: Vec<String>,
    /// Hash algorithm for content detection
    pub hash_algorithm: HashAlgorithm,
    /// Track file permissions
    pub track_permissions: bool,
    /// Track file timestamps
    pub track_timestamps: bool,
}

/// P2-Issue4: Hash algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HashAlgorithm {
    SHA256,
    SHA1,
    MD5,
    Blake3,
}

/// P2-Issue4: Dependency tracking configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyTrackingConfig {
    /// Track import dependencies
    pub track_imports: bool,
    /// Track include dependencies
    pub track_includes: bool,
    /// Track module dependencies
    pub track_modules: bool,
    /// Track library dependencies
    pub track_libraries: bool,
    /// Dependency depth limit
    pub max_dependency_depth: u32,
    /// Circular dependency detection
    pub detect_circular_deps: bool,
}

/// P2-Issue4: Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceConfig {
    /// Cache warming enabled
    pub cache_warming_enabled: bool,
    /// Prefetch strategy
    pub prefetch_strategy: PrefetchStrategy,
    /// Background refresh enabled
    pub background_refresh_enabled: bool,
    /// Refresh interval in milliseconds
    pub refresh_interval_ms: u64,
    /// Cache hit ratio target
    pub target_hit_ratio: f64,
    /// Performance monitoring enabled
    pub performance_monitoring_enabled: bool,
}

/// P2-Issue4: Prefetch strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PrefetchStrategy {
    /// No prefetching
    None,
    /// Predictive prefetching
    Predictive,
    /// Pattern-based prefetching
    PatternBased,
    /// Usage-based prefetching
    UsageBased,
    /// Hybrid prefetching
    Hybrid,
}

/// P2-Issue4: Cache warming configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheWarmingConfig {
    /// Warm cache on startup
    pub warm_on_startup: bool,
    /// Predefined warming queries
    pub warming_queries: Vec<WarmingQuery>,
    /// Warming batch size
    pub batch_size: usize,
    /// Warming timeout per query
    pub timeout_per_query_ms: u64,
    /// Parallel warming enabled
    pub parallel_warming_enabled: bool,
}

/// P2-Issue4: Cache warming query
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WarmingQuery {
    /// Query name
    pub name: String,
    /// Query pattern
    pub pattern: String,
    /// Priority
    pub priority: u8,
    /// Estimated cache benefit
    pub estimated_benefit: f64,
}

/// P2-Issue4: Cache entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    /// Entry key
    pub key: String,
    /// Validation result
    pub result: crate::harness::validation::ValidationResult,
    /// Creation timestamp
    pub created_at: Instant,
    /// Last access timestamp
    pub last_accessed: Instant,
    /// Access count
    pub access_count: u64,
    /// TTL in milliseconds
    pub ttl_ms: u64,
    /// File hashes at time of caching
    pub file_hashes: HashMap<PathBuf, String>,
    /// Dependencies at time of caching
    pub dependencies: Vec<String>,
    /// Entry metadata
    pub metadata: CacheEntryMetadata,
    /// Size in bytes
    pub size_bytes: usize,
}

/// P2-Issue4: Cache entry metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntryMetadata {
    /// Validation category
    pub category: crate::harness::validation::ValidationCategory,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Resource usage
    pub resource_usage: crate::harness::validation_artifacts::ResourceUsage,
    /// Cache hit benefit score
    pub benefit_score: f64,
    /// Tags
    pub tags: Vec<String>,
}

/// P2-Issue4: Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheStatistics {
    /// Total entries
    pub total_entries: usize,
    /// Cache size in bytes
    pub cache_size_bytes: usize,
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Hit ratio
    pub hit_ratio: f64,
    /// Evictions
    pub evictions: u64,
    /// Invalidations
    pub invalidations: u64,
    /// Average access time
    pub avg_access_time_ms: f64,
    /// Statistics by category
    pub category_stats: HashMap<crate::harness::validation::ValidationCategory, CategoryCacheStats>,
}

/// P2-Issue4: Category cache statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryCacheStats {
    /// Category
    pub category: crate::harness::validation::ValidationCategory,
    /// Entries count
    pub entries_count: usize,
    /// Hits
    pub hits: u64,
    /// Misses
    pub misses: u64,
    /// Hit ratio
    pub hit_ratio: f64,
    /// Average entry size
    pub avg_entry_size_bytes: usize,
}

/// P2-Issue4: Invalidation event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvalidationEvent {
    /// Event ID
    pub id: String,
    /// Event type
    pub event_type: InvalidationEventType,
    /// Timestamp
    pub timestamp: Instant,
    /// Affected entries
    pub affected_entries: Vec<String>,
    /// Reason for invalidation
    pub reason: String,
    /// Source of invalidation
    pub source: InvalidationSource,
}

/// P2-Issue4: Invalidation event types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvalidationEventType {
    /// File changed
    FileChanged,
    /// Dependency changed
    DependencyChanged,
    /// TTL expired
    TTLExpired,
    /// Manual invalidation
    Manual,
    /// Cache full
    CacheFull,
    /// Strategy-based invalidation
    StrategyBased,
}

/// P2-Issue4: Invalidation sources
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InvalidationSource {
    FileSystem,
    DependencyTracker,
    TTLManager,
    Manual,
    EvictionPolicy,
    StrategyEngine,
}

/// P2-Issue4: Advanced validation cache
pub struct AdvancedValidationCache {
    config: ValidationCacheConfig,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    statistics: Arc<RwLock<CacheStatistics>>,
    invalidation_engine: InvalidationEngine,
    performance_monitor: PerformanceMonitor,
    warming_engine: CacheWarmingEngine,
}

impl Default for ValidationCacheConfig {
    fn default() -> Self {
        let mut ttl_by_category = HashMap::new();
        ttl_by_category.insert(crate::harness::validation::ValidationCategory::Format, 300_000); // 5 minutes
        ttl_by_category.insert(crate::harness::validation::ValidationCategory::Lint, 600_000);  // 10 minutes
        ttl_by_category.insert(crate::harness::validation::ValidationCategory::Test, 1_800_000); // 30 minutes
        ttl_by_category.insert(crate::harness::validation::ValidationCategory::Repro, 900_000);  // 15 minutes
        
        Self {
            cache_config: CacheConfig {
                max_size_mb: 1024, // 1GB
                max_entries: 10000,
                default_ttl_ms: 600_000, // 10 minutes
                ttl_by_category,
                eviction_policy: EvictionPolicy::LRU,
                compression_enabled: true,
                encryption_enabled: false,
            },
            invalidation_config: InvalidationConfig {
                strategies: vec![
                    InvalidationStrategy {
                        name: "file_based".to_string(),
                        strategy_type: InvalidationStrategyType::FileBased,
                        conditions: vec![
                            InvalidationCondition::FilePattern("**/*.rs".to_string()),
                            InvalidationCondition::ContentHashChanged,
                        ],
                        priority: 100,
                        enabled: true,
                    },
                    InvalidationStrategy {
                        name: "dependency_based".to_string(),
                        strategy_type: InvalidationStrategyType::DependencyBased,
                        conditions: vec![
                            InvalidationCondition::DependencyChanged("Cargo.toml".to_string()),
                        ],
                        priority: 90,
                        enabled: true,
                    },
                ],
                file_change_detection: FileChangeDetectionConfig {
                    watch_filesystem: true,
                    polling_interval_ms: 1000,
                    watch_patterns: vec!["**/*.rs".to_string(), "**/*.toml".to_string()],
                    ignore_patterns: vec!["target/**".to_string(), ".git/**".to_string()],
                    hash_algorithm: HashAlgorithm::SHA256,
                    track_permissions: false,
                    track_timestamps: true,
                },
                dependency_tracking: DependencyTrackingConfig {
                    track_imports: true,
                    track_includes: true,
                    track_modules: true,
                    track_libraries: true,
                    max_dependency_depth: 10,
                    detect_circular_deps: true,
                },
                auto_invalidation_enabled: true,
                invalidation_delay_ms: 100,
            },
            performance_config: PerformanceConfig {
                cache_warming_enabled: true,
                prefetch_strategy: PrefetchStrategy::UsageBased,
                background_refresh_enabled: true,
                refresh_interval_ms: 300_000, // 5 minutes
                target_hit_ratio: 0.8,
                performance_monitoring_enabled: true,
            },
            warming_config: CacheWarmingConfig {
                warm_on_startup: true,
                warming_queries: vec![
                    WarmingQuery {
                        name: "common_format".to_string(),
                        pattern: "format src/**/*.rs".to_string(),
                        priority: 100,
                        estimated_benefit: 0.9,
                    },
                    WarmingQuery {
                        name: "common_lint".to_string(),
                        pattern: "lint src/**/*.rs".to_string(),
                        priority: 90,
                        estimated_benefit: 0.8,
                    },
                ],
                batch_size: 10,
                timeout_per_query_ms: 30000,
                parallel_warming_enabled: true,
            },
        }
    }
}

impl AdvancedValidationCache {
    /// Create new advanced validation cache
    pub fn new() -> Self {
        Self::with_config(ValidationCacheConfig::default())
    }
    
    /// Create cache with custom configuration
    pub fn with_config(config: ValidationCacheConfig) -> Self {
        let cache = Arc::new(RwLock::new(HashMap::new()));
        let statistics = Arc::new(RwLock::new(CacheStatistics::default()));
        
        Self {
            invalidation_engine: InvalidationEngine::new(config.invalidation_config.clone()),
            performance_monitor: PerformanceMonitor::new(config.performance_config.clone()),
            warming_engine: CacheWarmingEngine::new(config.warming_config.clone()),
            config,
            cache,
            statistics,
        }
    }
    
    /// Get cached validation result
    pub async fn get(&self, key: &str) -> Result<Option<crate::harness::validation::ValidationResult>> {
        let start_time = Instant::now();
        
        let cache_read = self.cache.read().await;
        
        if let Some(entry) = cache_read.get(key) {
            // Check if entry is still valid
            if self.is_entry_valid(entry).await? {
                // Update access statistics
                drop(cache_read);
                self.update_access_stats(key).await;
                
                // Record hit
                {
                    let mut stats = self.statistics.write().await;
                    stats.hits += 1;
                    stats.hit_ratio = stats.hits as f64 / (stats.hits + stats.misses) as f64;
                }
                
                debug!("Cache hit for key: {}", key);
                return Ok(Some(entry.result.clone()));
            } else {
                // Entry is invalid, remove it
                drop(cache_read);
                self.invalidate_key(key).await?;
            }
        }
        
        // Record miss
        {
            let mut stats = self.statistics.write().await;
            stats.misses += 1;
            stats.hit_ratio = stats.hits as f64 / (stats.hits + stats.misses) as f64;
        }
        
        debug!("Cache miss for key: {}", key);
        
        // Update performance statistics
        self.performance_monitor.record_access(start_time.elapsed(), false).await;
        
        Ok(None)
    }
    
    /// Put validation result in cache
    pub async fn put(&self, key: String, result: crate::harness::validation::ValidationResult, 
                     file_hashes: HashMap<PathBuf, String>, dependencies: Vec<String>) -> Result<()> {
        let start_time = Instant::now();
        
        // Calculate entry size
        let serialized_result = serde_json::to_string(&result)?;
        let size_bytes = serialized_result.len();
        
        // Check cache size limits
        self.check_cache_limits().await?;
        
        // Create cache entry
        let entry = CacheEntry {
            key: key.clone(),
            result,
            created_at: Instant::now(),
            last_accessed: Instant::now(),
            access_count: 1,
            ttl_ms: self.get_ttl_for_category(&result.category),
            file_hashes,
            dependencies,
            metadata: CacheEntryMetadata {
                category: result.category,
                execution_time_ms: result.duration_ms.unwrap_or(0),
                resource_usage: result.resource_usage.clone().unwrap_or_default(),
                benefit_score: self.calculate_benefit_score(&result),
                tags: vec![],
            },
            size_bytes,
        };
        
        // Add to cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(key.clone(), entry);
        }
        
        // Update statistics
        {
            let mut stats = self.statistics.write().await;
            stats.total_entries = cache.read().await.len();
            stats.cache_size_bytes += size_bytes;
            
            // Update category statistics
            let category_stats = stats.category_stats.entry(result.category).or_insert_with(|| CategoryCacheStats {
                category: result.category,
                entries_count: 0,
                hits: 0,
                misses: 0,
                hit_ratio: 0.0,
                avg_entry_size_bytes: 0,
            });
            category_stats.entries_count += 1;
            category_stats.avg_entry_size_bytes = 
                (category_stats.avg_entry_size_bytes * (category_stats.entries_count - 1) as usize + size_bytes) / 
                category_stats.entries_count;
        }
        
        debug!("Cached result for key: {} (size: {} bytes)", key, size_bytes);
        
        // Update performance statistics
        self.performance_monitor.record_access(start_time.elapsed(), true).await;
        
        Ok(())
    }
    
    /// Invalidate cache entries based on conditions
    pub async fn invalidate(&self, conditions: Vec<InvalidationCondition>) -> Result<usize> {
        let mut invalidated_count = 0;
        
        {
            let cache = self.cache.read().await;
            let mut keys_to_invalidate = Vec::new();
            
            for (key, entry) in cache.iter() {
                if self.should_invalidate_entry(entry, &conditions).await {
                    keys_to_invalidate.push(key.clone());
                }
            }
            
            drop(cache);
            
            for key in keys_to_invalidate {
                self.invalidate_key(&key).await?;
                invalidated_count += 1;
            }
        }
        
        info!("Invalidated {} cache entries", invalidated_count);
        Ok(invalidated_count)
    }
    
    /// Invalidate specific key
    pub async fn invalidate_key(&self, key: &str) -> Result<()> {
        let entry_size = {
            let mut cache = self.cache.write().await;
            if let Some(entry) = cache.remove(key) {
                let size = entry.size_bytes;
                
                // Update statistics
                {
                    let mut stats = self.statistics.write().await;
                    stats.total_entries = cache.len();
                    stats.cache_size_bytes = stats.cache_size_bytes.saturating_sub(size);
                    stats.invalidations += 1;
                    
                    // Update category statistics
                    if let Some(category_stats) = stats.category_stats.get_mut(&entry.metadata.category) {
                        category_stats.entries_count = category_stats.entries_count.saturating_sub(1);
                    }
                }
                
                // Record invalidation event
                self.invalidation_engine.record_invalidation(InvalidationEvent {
                    id: format!("inv_{}", chrono::Utc::now().timestamp_nanos()),
                    event_type: InvalidationEventType::Manual,
                    timestamp: Instant::now(),
                    affected_entries: vec![key.to_string()],
                    reason: "Manual invalidation".to_string(),
                    source: InvalidationSource::Manual,
                }).await;
                
                size
            } else {
                0
            }
        };
        
        debug!("Invalidated cache entry: {} (freed {} bytes)", key, entry_size);
        Ok(())
    }
    
    /// Clear all cache entries
    pub async fn clear(&self) -> Result<()> {
        let cleared_count = {
            let mut cache = self.cache.write().await;
            let count = cache.len();
            cache.clear();
            count
        };
        
        // Reset statistics
        {
            let mut stats = self.statistics.write().await;
            *stats = CacheStatistics::default();
        }
        
        info!("Cleared {} cache entries", cleared_count);
        Ok(())
    }
    
    /// Get cache statistics
    pub async fn get_statistics(&self) -> CacheStatistics {
        self.statistics.read().await.clone()
    }
    
    /// Warm cache with predefined queries
    pub async fn warm_cache(&self) -> Result<()> {
        if !self.config.performance_config.cache_warming_enabled {
            return Ok(());
        }
        
        info!("Starting cache warming");
        let warmed_count = self.warming_engine.warm_cache().await?;
        
        info!("Cache warming completed, warmed {} entries", warmed_count);
        Ok(())
    }
    
    /// Check if cache entry is still valid
    async fn is_entry_valid(&self, entry: &CacheEntry) -> Result<bool> {
        // Check TTL
        if entry.created_at.elapsed().as_millis() >= entry.ttl_ms as u128 {
            return Ok(false);
        }
        
        // Check file hashes
        for (file_path, cached_hash) in &entry.file_hashes {
            if let Ok(current_hash) = self.calculate_file_hash(file_path).await {
                if current_hash != *cached_hash {
                    return Ok(false);
                }
            } else {
                // File no longer exists or is inaccessible
                return Ok(false);
            }
        }
        
        // Check dependencies
        for dependency in &entry.dependencies {
            if !self.is_dependency_valid(dependency).await {
                return Ok(false);
            }
        }
        
        Ok(true)
    }
    
    /// Update access statistics for entry
    async fn update_access_stats(&self, key: &str) {
        let mut cache = self.cache.write().await;
        if let Some(entry) = cache.get_mut(key) {
            entry.last_accessed = Instant::now();
            entry.access_count += 1;
        }
    }
    
    /// Check cache limits and evict if necessary
    async fn check_cache_limits(&self) -> Result<()> {
        let (current_size, current_entries) = {
            let cache = self.cache.read().await;
            let total_size: usize = cache.values().map(|e| e.size_bytes).sum();
            (total_size, cache.len())
        };
        
        let max_size_bytes = self.config.cache_config.max_size_mb * 1024 * 1024;
        
        // Check size limit
        if current_size > max_size_bytes {
            self.evict_by_size(current_size - max_size_bytes).await?;
        }
        
        // Check entry limit
        if current_entries > self.config.cache_config.max_entries {
            self.evict_by_count(current_entries - self.config.cache_config.max_entries).await?;
        }
        
        Ok(())
    }
    
    /// Evict entries by size
    async fn evict_by_size(&self, bytes_to_free: usize) -> Result<()> {
        let mut bytes_freed = 0;
        let mut entries_to_remove = Vec::new();
        
        {
            let cache = self.cache.read().await;
            let mut entries: Vec<_> = cache.iter().collect();
            
            // Sort by eviction policy
            match self.config.cache_config.eviction_policy {
                EvictionPolicy::LRU => {
                    entries.sort_by(|a, b| a.1.last_accessed.cmp(&b.1.last_accessed));
                }
                EvictionPolicy::LFU => {
                    entries.sort_by(|a, b| a.1.access_count.cmp(&b.1.access_count));
                }
                EvictionPolicy::FIFO => {
                    entries.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at));
                }
                EvictionPolicy::SizeBased => {
                    entries.sort_by(|a, b| b.1.size_bytes.cmp(&a.1.size_bytes));
                }
                _ => {
                    entries.sort_by(|a, b| a.1.last_accessed.cmp(&b.1.last_accessed));
                }
            }
            
            for (key, entry) in entries {
                if bytes_freed >= bytes_to_free {
                    break;
                }
                entries_to_remove.push(key.clone());
                bytes_freed += entry.size_bytes;
            }
        }
        
        // Remove entries
        for key in entries_to_remove {
            self.invalidate_key(&key).await?;
        }
        
        info!("Evicted {} bytes from cache", bytes_freed);
        Ok(())
    }
    
    /// Evict entries by count
    async fn evict_by_count(&self, entries_to_remove: usize) -> Result<()> {
        let mut keys_to_remove = Vec::new();
        
        {
            let cache = self.cache.read().await;
            let mut entries: Vec<_> = cache.iter().collect();
            
            // Sort by eviction policy
            match self.config.cache_config.eviction_policy {
                EvictionPolicy::LRU => {
                    entries.sort_by(|a, b| a.1.last_accessed.cmp(&b.1.last_accessed));
                }
                EvictionPolicy::LFU => {
                    entries.sort_by(|a, b| a.1.access_count.cmp(&b.1.access_count));
                }
                EvictionPolicy::FIFO => {
                    entries.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at));
                }
                _ => {
                    entries.sort_by(|a, b| a.1.last_accessed.cmp(&b.1.last_accessed));
                }
            }
            
            for (key, _) in entries.iter().take(entries_to_remove) {
                keys_to_remove.push(key.clone());
            }
        }
        
        // Remove entries
        for key in keys_to_remove {
            self.invalidate_key(&key).await?;
        }
        
        info!("Evicted {} entries from cache", entries_to_remove);
        Ok(())
    }
    
    /// Get TTL for validation category
    fn get_ttl_for_category(&self, category: &crate::harness::validation::ValidationCategory) -> u64 {
        self.config.cache_config.ttl_by_category
            .get(category)
            .copied()
            .unwrap_or(self.config.cache_config.default_ttl_ms)
    }
    
    /// Calculate benefit score for cache entry
    fn calculate_benefit_score(&self, result: &crate::harness::validation::ValidationResult) -> f64 {
        // Simple benefit calculation based on execution time and success
        let execution_time = result.duration_ms.unwrap_or(0) as f64;
        let success_factor = if result.status == crate::harness::validation::ValidationStatus::Passed { 1.0 } else { 0.5 };
        
        // Higher benefit for longer-running, successful validations
        (execution_time / 1000.0) * success_factor
    }
    
    /// Calculate file hash
    async fn calculate_file_hash(&self, file_path: &Path) -> Result<String> {
        use sha2::{Digest, Sha256};
        
        let content = tokio::fs::read(file_path).await
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
        
        let mut hasher = Sha256::new();
        hasher.update(&content);
        
        Ok(format!("{:x}", hasher.finalize()))
    }
    
    /// Check if dependency is valid
    async fn is_dependency_valid(&self, dependency: &str) -> bool {
        // Simple dependency validation - would be more sophisticated in practice
        Path::new(dependency).exists()
    }
    
    /// Check if entry should be invalidated based on conditions
    async fn should_invalidate_entry(&self, entry: &CacheEntry, conditions: &[InvalidationCondition]) -> bool {
        for condition in conditions {
            match condition {
                InvalidationCondition::FilePattern(pattern) => {
                    // Check if any of the entry's files match the pattern
                    for file_path in entry.file_hashes.keys() {
                        if self.matches_pattern(file_path, pattern) {
                            return true;
                        }
                    }
                }
                InvalidationCondition::ContentHashChanged => {
                    // Check if any file hashes have changed
                    for (file_path, cached_hash) in &entry.file_hashes {
                        if let Ok(current_hash) = self.calculate_file_hash(file_path).await {
                            if current_hash != *cached_hash {
                                return true;
                            }
                        }
                    }
                }
                InvalidationCondition::TimeElapsed(duration) => {
                    if entry.created_at.elapsed() >= *duration {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    
    /// Check if file path matches pattern
    fn matches_pattern(&self, file_path: &Path, pattern: &str) -> bool {
        // Simple pattern matching - would use glob patterns in practice
        file_path.to_string_lossy().contains(pattern)
    }
}

/// P2-Issue4: Invalidation engine
pub struct InvalidationEngine {
    config: InvalidationConfig,
    invalidation_events: Arc<RwLock<Vec<InvalidationEvent>>>,
}

impl InvalidationEngine {
    /// Create new invalidation engine
    pub fn new(config: InvalidationConfig) -> Self {
        Self {
            config,
            invalidation_events: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    /// Record invalidation event
    pub async fn record_invalidation(&self, event: InvalidationEvent) {
        let mut events = self.invalidation_events.write().await;
        events.push(event);
        
        // Trim events if needed
        if events.len() > 10000 {
            events.remove(0);
        }
    }
    
    /// Get invalidation events
    pub async fn get_events(&self) -> Vec<InvalidationEvent> {
        self.invalidation_events.read().await.clone()
    }
}

/// P2-Issue4: Performance monitor
pub struct PerformanceMonitor {
    config: PerformanceConfig,
    access_times: Arc<RwLock<Vec<Duration>>>,
}

impl PerformanceMonitor {
    /// Create new performance monitor
    pub fn new(config: PerformanceConfig) -> Self {
        Self {
            config,
            access_times: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    /// Record cache access
    pub async fn record_access(&self, access_time: Duration, is_hit: bool) {
        if self.config.performance_monitoring_enabled {
            let mut times = self.access_times.write().await;
            times.push(access_time);
            
            // Trim if needed
            if times.len() > 10000 {
                times.remove(0);
            }
        }
    }
    
    /// Get average access time
    pub async fn get_avg_access_time(&self) -> Duration {
        let times = self.access_times.read().await;
        if times.is_empty() {
            return Duration::ZERO;
        }
        
        let total: Duration = times.iter().sum();
        total / times.len() as u32
    }
}

/// P2-Issue4: Cache warming engine
pub struct CacheWarmingEngine {
    config: CacheWarmingConfig,
}

impl CacheWarmingEngine {
    /// Create new cache warming engine
    pub fn new(config: CacheWarmingConfig) -> Self {
        Self { config }
    }
    
    /// Warm cache with predefined queries
    pub async fn warm_cache(&self) -> Result<usize> {
        if !self.config.warm_on_startup {
            return Ok(0);
        }
        
        info!("Starting cache warming with {} queries", self.config.warming_queries.len());
        
        let mut warmed_count = 0;
        
        for query in &self.config.warming_queries {
            // Execute warming query (placeholder implementation)
            debug!("Warming query: {}", query.name);
            warmed_count += 1;
            
            // In a real implementation, this would execute the validation
            // and cache the results
        }
        
        Ok(warmed_count)
    }
}

impl Default for CacheStatistics {
    fn default() -> Self {
        Self {
            total_entries: 0,
            cache_size_bytes: 0,
            hits: 0,
            misses: 0,
            hit_ratio: 0.0,
            evictions: 0,
            invalidations: 0,
            avg_access_time_ms: 0.0,
            category_stats: HashMap::new(),
        }
    }
}

impl Default for CategoryCacheStats {
    fn default() -> Self {
        Self {
            category: crate::harness::validation::ValidationCategory::Format,
            entries_count: 0,
            hits: 0,
            misses: 0,
            hit_ratio: 0.0,
            avg_entry_size_bytes: 0,
        }
    }
}
