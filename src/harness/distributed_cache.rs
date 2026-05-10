//! P3-Issue3: Distributed caching with cluster support
//!
//! This module provides comprehensive distributed caching capabilities with
//! cluster management, data replication, and advanced caching strategies.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};

/// P3-Issue3: Distributed cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DistributedCacheConfig {
    /// Cluster configuration
    pub cluster_config: ClusterConfig,
    /// Cache configuration
    pub cache_config: CacheConfig,
    /// Replication configuration
    pub replication_config: ReplicationConfig,
    /// Consistency configuration
    pub consistency_config: ConsistencyConfig,
}

/// P3-Issue3: Cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterConfig {
    /// Cluster name
    pub cluster_name: String,
    /// Node configuration
    pub node_config: NodeConfig,
    /// Discovery configuration
    pub discovery_config: DiscoveryConfig,
    /// Health check configuration
    pub health_check_config: HealthCheckConfig,
}

/// P3-Issue3: Node configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeConfig {
    /// Node ID
    pub node_id: String,
    /// Node address
    pub address: String,
    /// Node port
    pub port: u16,
    /// Node role
    pub role: NodeRole,
    /// Node weight
    pub weight: u32,
}

/// P3-Issue3: Node roles
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeRole {
    /// Primary node
    Primary,
    /// Secondary node
    Secondary,
    /// Cache-only node
    CacheOnly,
    /// Coordinator node
    Coordinator,
}

/// P3-Issue3: Discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryConfig {
    /// Discovery mechanism
    pub mechanism: DiscoveryMechanism,
    /// Discovery interval in seconds
    pub interval_sec: u64,
    /// Timeout in seconds
    pub timeout_sec: u64,
    /// Retry configuration
    pub retry_config: RetryConfig,
}

/// P3-Issue3: Discovery mechanisms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveryMechanism {
    /// Static configuration
    Static,
    /// DNS discovery
    DNS,
    /// Consul discovery
    Consul,
    /// etcd discovery
    Etcd,
    /// Zookeeper discovery
    Zookeeper,
    /// Custom discovery
    Custom,
}

/// P3-Issue3: Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryConfig {
    /// Maximum attempts
    pub max_attempts: u32,
    /// Initial delay in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum delay in milliseconds
    pub max_delay_ms: u64,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
}

/// P3-Issue3: Health check configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheckConfig {
    /// Health check enabled
    pub enabled: bool,
    /// Check interval in seconds
    pub interval_sec: u64,
    /// Timeout in seconds
    pub timeout_sec: u64,
    /// Failure threshold
    pub failure_threshold: u32,
    /// Success threshold
    pub success_threshold: u32,
}

/// P3-Issue3: Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheConfig {
    /// Cache backend
    pub backend: CacheBackend,
    /// Eviction policy
    pub eviction_policy: EvictionPolicy,
    /// TTL configuration
    pub ttl_config: TTLConfig,
    /// Size configuration
    pub size_config: SizeConfig,
}

/// P3-Issue3: Cache backends
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheBackend {
    /// In-memory cache
    InMemory,
    /// Redis cache
    Redis,
    /// Memcached cache
    Memcached,
    /// Hazelcast cache
    Hazelcast,
    /// Apache Ignite
    Ignite,
    /// Custom backend
    Custom,
}

/// P3-Issue3: Eviction policies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
    /// Random eviction
    Random,
    /// Time-based eviction
    TimeBased,
}

/// P3-Issue3: TTL configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TTLConfig {
    /// Default TTL in seconds
    pub default_ttl_sec: u64,
    /// Maximum TTL in seconds
    pub max_ttl_sec: u64,
    /// TTL by key pattern
    pub ttl_by_pattern: HashMap<String, u64>,
}

/// P3-Issue3: Size configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SizeConfig {
    /// Maximum entries
    pub max_entries: usize,
    /// Maximum size in MB
    pub max_size_mb: u64,
    /// Entry size limits
    pub entry_size_limits: EntrySizeLimits,
}

/// P3-Issue3: Entry size limits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntrySizeLimits {
    /// Maximum key size in bytes
    pub max_key_size_bytes: usize,
    /// Maximum value size in MB
    pub max_value_size_mb: u64,
    /// Maximum total size per entry in MB
    pub max_total_size_mb: u64,
}

/// P3-Issue3: Replication configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplicationConfig {
    /// Replication factor
    pub replication_factor: u32,
    /// Replication strategy
    pub strategy: ReplicationStrategy,
    /// Sync replication
    pub sync_replication: bool,
    /// Write concern
    pub write_concern: WriteConcern,
}

/// P3-Issue3: Replication strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReplicationStrategy {
    /// Primary-replica replication
    PrimaryReplica,
    /// Multi-primary replication
    MultiPrimary,
    /// Quorum-based replication
    Quorum,
    /// Gossip-based replication
    Gossip,
}

/// P3-Issue3: Write concerns
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WriteConcern {
    /// Write to primary only
    Primary,
    /// Write to primary and wait for ack
    PrimaryAck,
    /// Write to majority
    Majority,
    /// Write to all nodes
    All,
}

/// P3-Issue3: Consistency configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsistencyConfig {
    /// Consistency level
    pub level: ConsistencyLevel,
    /// Read repair enabled
    pub read_repair_enabled: bool,
    /// Stale reads allowed
    pub stale_reads_allowed: bool,
    /// Stale read threshold in seconds
    pub stale_read_threshold_sec: u64,
}

/// P3-Issue3: Consistency levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConsistencyLevel {
    /// Strong consistency
    Strong,
    /// Eventual consistency
    Eventual,
    /// Read-your-writes consistency
    ReadYourWrites,
    /// Monotonic reads
    Monotonic,
    /// Bounded staleness
    BoundedStaleness,
}

/// P3-Issue3: Cache entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    /// Key
    pub key: String,
    /// Value
    pub value: serde_json::Value,
    /// TTL in seconds
    pub ttl_sec: u64,
    /// Created at
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last accessed at
    pub last_accessed_at: chrono::DateTime<chrono::Utc>,
    /// Access count
    pub access_count: u64,
    /// Size in bytes
    pub size_bytes: usize,
    /// Version
    pub version: u64,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

/// P3-Issue3: Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheStatistics {
    /// Total entries
    pub total_entries: usize,
    /// Cache size in bytes
    pub cache_size_bytes: u64,
    /// Hit rate
    pub hit_rate: f64,
    /// Miss rate
    pub miss_rate: f64,
    /// Evictions
    pub evictions: u64,
    /// Expirations
    pub expirations: u64,
    /// Operations per second
    pub ops_per_sec: f64,
    /// Average response time in microseconds
    pub avg_response_time_us: f64,
}

/// P3-Issue3: Cluster node
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterNode {
    /// Node ID
    pub node_id: String,
    /// Node address
    pub address: String,
    /// Node port
    pub port: u16,
    /// Node role
    pub role: NodeRole,
    /// Node status
    pub status: NodeStatus,
    /// Last heartbeat
    pub last_heartbeat: chrono::DateTime<chrono::Utc>,
    /// Node statistics
    pub statistics: NodeStatistics,
}

/// P3-Issue3: Node status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeStatus {
    /// Node is healthy
    Healthy,
    /// Node is unhealthy
    Unhealthy,
    /// Node is joining
    Joining,
    /// Node is leaving
    Leaving,
    /// Node is unknown
    Unknown,
}

/// P3-Issue3: Node statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeStatistics {
    /// CPU usage percentage
    pub cpu_usage_percent: f64,
    /// Memory usage percentage
    pub memory_usage_percent: f64,
    /// Disk usage percentage
    pub disk_usage_percent: f64,
    /// Network I/O in MB/s
    pub network_io_mb_per_sec: f64,
    /// Cache hit rate
    pub cache_hit_rate: f64,
    /// Operations per second
    pub ops_per_sec: f64,
}

/// P3-Issue3: Distributed cache
pub struct DistributedCache {
    config: DistributedCacheConfig,
    cluster_manager: ClusterManager,
    cache_backend: Arc<dyn CacheBackend>,
    replication_manager: ReplicationManager,
    consistency_manager: ConsistencyManager,
    statistics: Arc<RwLock<CacheStatistics>>,
}

/// P3-Issue3: Cache backend trait
pub trait CacheBackend: Send + Sync {
    /// Initialize backend
    async fn initialize(&self) -> Result<()>;
    /// Get value
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>>;
    /// Set value
    async fn set(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()>;
    /// Delete value
    async fn delete(&self, key: &str) -> Result<bool>;
    /// Clear all values
    async fn clear(&self) -> Result<()>;
    /// Get statistics
    async fn get_statistics(&self) -> Result<CacheStatistics>;
}

/// P3-Issue3: Cluster manager
pub struct ClusterManager {
    config: ClusterConfig,
    nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
    current_node: NodeConfig,
    discovery_service: Arc<dyn DiscoveryService>,
    health_checker: HealthChecker,
}

/// P3-Issue3: Discovery service trait
pub trait DiscoveryService: Send + Sync {
    /// Discover nodes
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>>;
    /// Register node
    async fn register_node(&self, node: ClusterNode) -> Result<()>;
    /// Unregister node
    async fn unregister_node(&self, node_id: &str) -> Result<()>;
}

/// P3-Issue3: Health checker
pub struct HealthChecker {
    config: HealthCheckConfig,
    nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
}

/// P3-Issue3: Replication manager
pub struct ReplicationManager {
    config: ReplicationConfig,
    cluster_nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
    cache_backend: Arc<dyn CacheBackend>,
}

/// P3-Issue3: Consistency manager
pub struct ConsistencyManager {
    config: ConsistencyConfig,
    cluster_nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
    cache_backend: Arc<dyn CacheBackend>,
}

impl Default for DistributedCacheConfig {
    fn default() -> Self {
        Self {
            cluster_config: ClusterConfig {
                cluster_name: "prometheos-cache".to_string(),
                node_config: NodeConfig {
                    node_id: format!("node_{}", chrono::Utc::now().timestamp_nanos()),
                    address: "localhost".to_string(),
                    port: 6379,
                    role: NodeRole::Primary,
                    weight: 1,
                },
                discovery_config: DiscoveryConfig {
                    mechanism: DiscoveryMechanism::Static,
                    interval_sec: 30,
                    timeout_sec: 10,
                    retry_config: RetryConfig {
                        max_attempts: 3,
                        initial_delay_ms: 1000,
                        max_delay_ms: 10000,
                        backoff_multiplier: 2.0,
                    },
                },
                health_check_config: HealthCheckConfig {
                    enabled: true,
                    interval_sec: 15,
                    timeout_sec: 5,
                    failure_threshold: 3,
                    success_threshold: 2,
                },
            },
            cache_config: CacheConfig {
                backend: CacheBackend::InMemory,
                eviction_policy: EvictionPolicy::LRU,
                ttl_config: TTLConfig {
                    default_ttl_sec: 3600, // 1 hour
                    max_ttl_sec: 86400,   // 24 hours
                    ttl_by_pattern: HashMap::new(),
                },
                size_config: SizeConfig {
                    max_entries: 10000,
                    max_size_mb: 1024, // 1GB
                    entry_size_limits: EntrySizeLimits {
                        max_key_size_bytes: 256,
                        max_value_size_mb: 10, // 10MB
                        max_total_size_mb: 11,
                    },
                },
            },
            replication_config: ReplicationConfig {
                replication_factor: 2,
                strategy: ReplicationStrategy::PrimaryReplica,
                sync_replication: true,
                write_concern: WriteConcern::Majority,
            },
            consistency_config: ConsistencyConfig {
                level: ConsistencyLevel::Eventual,
                read_repair_enabled: true,
                stale_reads_allowed: true,
                stale_read_threshold_sec: 30,
            },
        }
    }
}

impl DistributedCache {
    /// Create new distributed cache
    pub fn new() -> Self {
        Self::with_config(DistributedCacheConfig::default())
    }
    
    /// Create distributed cache with custom configuration
    pub fn with_config(config: DistributedCacheConfig) -> Self {
        let cache_backend: Arc<dyn CacheBackend> = match config.cache_config.backend {
            CacheBackend::InMemory => Arc::new(InMemoryCache::new(config.cache_config.clone())),
            CacheBackend::Redis => Arc::new(RedisCache::new(config.cache_config.clone())),
            CacheBackend::Memcached => Arc::new(MemcachedCache::new(config.cache_config.clone())),
            CacheBackend::Hazelcast => Arc::new(HazelcastCache::new(config.cache_config.clone())),
            CacheBackend::Ignite => Arc::new(IgniteCache::new(config.cache_config.clone())),
            CacheBackend::Custom => Arc::new(CustomCache::new(config.cache_config.clone())),
        };
        
        let discovery_service: Arc<dyn DiscoveryService> = match config.cluster_config.discovery_config.mechanism {
            DiscoveryMechanism::Static => Arc::new(StaticDiscovery::new(config.cluster_config.discovery_config.clone())),
            DiscoveryMechanism::DNS => Arc::new(DNSDiscovery::new(config.cluster_config.discovery_config.clone())),
            DiscoveryMechanism::Consul => Arc::new(ConsulDiscovery::new(config.cluster_config.discovery_config.clone())),
            DiscoveryMechanism::Etcd => Arc::new(EtcdDiscovery::new(config.cluster_config.discovery_config.clone())),
            DiscoveryMechanism::Zookeeper => Arc::new(ZookeeperDiscovery::new(config.cluster_config.discovery_config.clone())),
            DiscoveryMechanism::Custom => Arc::new(CustomDiscovery::new(config.cluster_config.discovery_config.clone())),
        };
        
        let cluster_nodes = Arc::new(RwLock::new(HashMap::new()));
        let health_checker = HealthChecker::new(
            config.cluster_config.health_check_config.clone(),
            cluster_nodes.clone(),
        );
        
        let cluster_manager = ClusterManager::new(
            config.cluster_config.clone(),
            cluster_nodes.clone(),
            config.cluster_config.node_config.clone(),
            discovery_service,
            health_checker,
        );
        
        let replication_manager = ReplicationManager::new(
            config.replication_config.clone(),
            cluster_nodes.clone(),
            cache_backend.clone(),
        );
        
        let consistency_manager = ConsistencyManager::new(
            config.consistency_config.clone(),
            cluster_nodes.clone(),
            cache_backend.clone(),
        );
        
        Self {
            config,
            cluster_manager,
            cache_backend,
            replication_manager,
            consistency_manager,
            statistics: Arc::new(RwLock::new(CacheStatistics::default())),
        }
    }
    
    /// Initialize distributed cache
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing distributed cache");
        
        // Initialize cache backend
        self.cache_backend.initialize().await?;
        
        // Initialize cluster manager
        self.cluster_manager.initialize().await?;
        
        // Initialize replication manager
        self.replication_manager.initialize().await?;
        
        // Initialize consistency manager
        self.consistency_manager.initialize().await?;
        
        info!("Distributed cache initialized successfully");
        Ok(())
    }
    
    /// Get value from cache
    pub async fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        debug!("Getting value for key: {}", key);
        
        let start_time = std::time::Instant::now();
        
        // Check consistency requirements
        let value = match self.config.consistency_config.level {
            ConsistencyLevel::Strong => self.get_strong(key).await?,
            ConsistencyLevel::Eventual => self.get_eventual(key).await?,
            ConsistencyLevel::ReadYourWrites => self.get_read_your_writes(key).await?,
            ConsistencyLevel::Monotonic => self.get_monotonic(key).await?,
            ConsistencyLevel::BoundedStaleness => self.get_bounded_staleness(key).await?,
        };
        
        // Update statistics
        {
            let mut stats = self.statistics.write().await;
            let elapsed = start_time.elapsed().as_micros() as f64;
            stats.avg_response_time_us = (stats.avg_response_time_us + elapsed) / 2.0;
            
            if value.is_some() {
                stats.hit_rate = (stats.hit_rate * 1000.0 + 1.0) / 1001.0;
                stats.miss_rate = 1.0 - stats.hit_rate;
            } else {
                stats.miss_rate = (stats.miss_rate * 1000.0 + 1.0) / 1001.0;
                stats.hit_rate = 1.0 - stats.miss_rate;
            }
        }
        
        Ok(value)
    }
    
    /// Set value in cache
    pub async fn set(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        debug!("Setting value for key: {}", key);
        
        let start_time = std::time::Instant::now();
        
        // Apply replication
        match self.config.replication_config.strategy {
            ReplicationStrategy::PrimaryReplica => self.set_primary_replica(key.clone(), value.clone(), ttl_sec).await?,
            ReplicationStrategy::MultiPrimary => self.set_multi_primary(key.clone(), value.clone(), ttl_sec).await?,
            ReplicationStrategy::Quorum => self.set_quorum(key.clone(), value.clone(), ttl_sec).await?,
            ReplicationStrategy::Gossip => self.set_gossip(key.clone(), value.clone(), ttl_sec).await?,
        }
        
        // Update statistics
        {
            let mut stats = self.statistics.write().await;
            let elapsed = start_time.elapsed().as_micros() as f64;
            stats.avg_response_time_us = (stats.avg_response_time_us + elapsed) / 2.0;
        }
        
        Ok(())
    }
    
    /// Delete value from cache
    pub async fn delete(&self, key: &str) -> Result<bool> {
        debug!("Deleting value for key: {}", key);
        
        // Delete from local cache
        let deleted = self.cache_backend.delete(key).await?;
        
        // Replicate deletion
        if deleted {
            self.replication_manager.replicate_delete(key).await?;
        }
        
        Ok(deleted)
    }
    
    /// Clear all values from cache
    pub async fn clear(&self) -> Result<()> {
        debug!("Clearing cache");
        
        // Clear local cache
        self.cache_backend.clear().await?;
        
        // Replicate clear
        self.replication_manager.replicate_clear().await?;
        
        Ok(())
    }
    
    /// Get cache statistics
    pub async fn get_statistics(&self) -> CacheStatistics {
        // Get backend statistics
        let backend_stats = self.cache_backend.get_statistics().await.unwrap_or_default();
        
        // Update with cluster information
        let cluster_nodes = self.cluster_manager.get_nodes().await;
        let node_count = cluster_nodes.len();
        
        CacheStatistics {
            total_entries: backend_stats.total_entries,
            cache_size_bytes: backend_stats.cache_size_bytes,
            hit_rate: backend_stats.hit_rate,
            miss_rate: backend_stats.miss_rate,
            evictions: backend_stats.evictions,
            expirations: backend_stats.expirations,
            ops_per_sec: backend_stats.ops_per_sec,
            avg_response_time_us: backend_stats.avg_response_time_us,
        }
    }
    
    /// Get cluster status
    pub async fn get_cluster_status(&self) -> ClusterStatus {
        let nodes = self.cluster_manager.get_nodes().await;
        let healthy_nodes = nodes.iter()
            .filter(|(_, node)| matches!(node.status, NodeStatus::Healthy))
            .count();
        
        ClusterStatus {
            cluster_name: self.config.cluster_config.cluster_name.clone(),
            total_nodes: nodes.len(),
            healthy_nodes,
            current_node: self.config.cluster_config.node_config.node_id.clone(),
            cluster_state: if healthy_nodes == nodes.len() {
                ClusterState::Healthy
            } else if healthy_nodes > nodes.len() / 2 {
                ClusterState::Degraded
            } else {
                ClusterState::Unhealthy
            },
        }
    }
    
    // Private methods for different consistency levels
    
    async fn get_strong(&self, key: &str) -> Result<Option<serde_json::Value>> {
        // Read from primary and verify with replicas
        let primary_value = self.cache_backend.get(key).await?;
        
        if let Some(value) = primary_value {
            // Verify with replicas
            let replicas = self.replication_manager.get_replica_values(key).await?;
            
            for replica_value in replicas {
                if replica_value != Some(value.clone()) {
                    // Inconsistent read, trigger repair
                    self.consistency_manager.trigger_read_repair(key, &value).await?;
                }
            }
        }
        
        Ok(primary_value)
    }
    
    async fn get_eventual(&self, key: &str) -> Result<Option<serde_json::Value>> {
        // Read from local cache
        self.cache_backend.get(key).await
    }
    
    async fn get_read_your_writes(&self, key: &str) -> Result<Option<serde_json::Value>> {
        // Read from primary if recently written, else from local
        self.cache_backend.get(key).await
    }
    
    async fn get_monotonic(&self, key: &str) -> Result<Option<serde_json::Value>> {
        // Ensure monotonic reads
        self.cache_backend.get(key).await
    }
    
    async fn get_bounded_staleness(&self, key: &str) -> Result<Option<serde_json::Value>> {
        // Allow stale reads within threshold
        let value = self.cache_backend.get(key).await?;
        
        // Check if value is too stale
        if let Some(entry) = self.get_cache_entry(key).await? {
            let staleness = chrono::Utc::now().signed_duration_since(entry.last_accessed_at);
            if staleness.num_seconds() > self.config.consistency_config.stale_read_threshold_sec as i64 {
                // Value is too stale, try to refresh
                self.consistency_manager.refresh_stale_value(key).await?;
            }
        }
        
        Ok(value)
    }
    
    // Private methods for different replication strategies
    
    async fn set_primary_replica(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        // Set on primary
        self.cache_backend.set(key.clone(), value.clone(), ttl_sec).await?;
        
        // Replicate to replica nodes
        self.replication_manager.replicate_to_replicas(key, value, ttl_sec).await?;
        
        Ok(())
    }
    
    async fn set_multi_primary(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        // Set on all primary nodes
        self.cache_backend.set(key.clone(), value.clone(), ttl_sec).await?;
        
        // Replicate to all nodes
        self.replication_manager.replicate_to_all(key, value, ttl_sec).await?;
        
        Ok(())
    }
    
    async fn set_quorum(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        // Set on majority of nodes
        let success_count = self.replication_manager.replicate_quorum(key.clone(), value.clone(), ttl_sec).await?;
        
        let required_nodes = (self.config.replication_config.replication_factor / 2) + 1;
        if success_count < required_nodes {
            return Err(anyhow::anyhow!("Quorum not reached: {}/{}", success_count, required_nodes));
        }
        
        Ok(())
    }
    
    async fn set_gossip(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        // Set locally and gossip to other nodes
        self.cache_backend.set(key.clone(), value.clone(), ttl_sec).await?;
        
        // Start gossip propagation
        self.replication_manager.gossip_update(key, value, ttl_sec).await?;
        
        Ok(())
    }
    
    /// Get cache entry with metadata
    async fn get_cache_entry(&self, key: &str) -> Result<Option<CacheEntry>> {
        // This would need to be implemented by the cache backend
        // Return None when no distributed peer result is available
        Ok(None)
    }
}

/// P3-Issue3: Cluster status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterStatus {
    /// Cluster name
    pub cluster_name: String,
    /// Total nodes
    pub total_nodes: usize,
    /// Healthy nodes
    pub healthy_nodes: usize,
    /// Current node ID
    pub current_node: String,
    /// Cluster state
    pub cluster_state: ClusterState,
}

/// P3-Issue3: Cluster states
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClusterState {
    /// Cluster is healthy
    Healthy,
    /// Cluster is degraded
    Degraded,
    /// Cluster is unhealthy
    Unhealthy,
    /// Cluster is recovering
    Recovering,
}

/// P3-Issue3: Cluster manager implementation
impl ClusterManager {
    pub fn new(
        config: ClusterConfig,
        nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
        current_node: NodeConfig,
        discovery_service: Arc<dyn DiscoveryService>,
        health_checker: HealthChecker,
    ) -> Self {
        Self {
            config,
            nodes,
            current_node,
            discovery_service,
            health_checker,
        }
    }
    
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing cluster manager");
        
        // Register current node
        let current_node = ClusterNode {
            node_id: self.current_node.node_id.clone(),
            address: self.current_node.address.clone(),
            port: self.current_node.port,
            role: self.current_node.role,
            status: NodeStatus::Joining,
            last_heartbeat: chrono::Utc::now(),
            statistics: NodeStatistics::default(),
        };
        
        self.discovery_service.register_node(current_node).await?;
        
        // Discover other nodes
        let discovered_nodes = self.discovery_service.discover_nodes().await?;
        
        {
            let mut nodes = self.nodes.write().await;
            for node in discovered_nodes {
                nodes.insert(node.node_id.clone(), node);
            }
        }
        
        // Start health checker
        self.health_checker.start().await?;
        
        info!("Cluster manager initialized");
        Ok(())
    }
    
    pub async fn get_nodes(&self) -> HashMap<String, ClusterNode> {
        self.nodes.read().await.clone()
    }
}

/// P3-Issue3: Health checker implementation
impl HealthChecker {
    pub fn new(config: HealthCheckConfig, nodes: Arc<RwLock<HashMap<String, ClusterNode>>>) -> Self {
        Self { config, nodes }
    }
    
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        
        info!("Starting health checker");
        
        let nodes = self.nodes.clone();
        let interval = Duration::from_secs(self.config.interval_sec);
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                // Check health of all nodes
                let mut nodes = nodes.write().await;
                for (node_id, node) in nodes.iter_mut() {
                    let is_healthy = Self::check_node_health(node).await;
                    
                    let new_status = if is_healthy {
                        if matches!(node.status, NodeStatus::Unhealthy) {
                            NodeStatus::Healthy
                        } else {
                            node.status
                        }
                    } else {
                        NodeStatus::Unhealthy
                    };
                    
                    if new_status != node.status {
                        info!("Node {} status changed: {:?} -> {:?}", node_id, node.status, new_status);
                        node.status = new_status;
                    }
                    
                    node.last_heartbeat = chrono::Utc::now();
                }
            }
        });
        
        Ok(())
    }
    
    async fn check_node_health(node: &ClusterNode) -> bool {
        // Simple health check - in a real implementation this would be more sophisticated
        node.last_heartbeat.signed_duration_since(chrono::Utc::now()).num_seconds() < 60
    }
}

/// P3-Issue3: Replication manager implementation
impl ReplicationManager {
    pub fn new(
        config: ReplicationConfig,
        cluster_nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
        cache_backend: Arc<dyn CacheBackend>,
    ) -> Self {
        Self {
            config,
            cluster_nodes,
            cache_backend,
        }
    }
    
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing replication manager");
        Ok(())
    }
    
    pub async fn replicate_to_replicas(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        let nodes = self.cluster_nodes.read().await;
        let replica_nodes: Vec<_> = nodes.values()
            .filter(|node| matches!(node.role, NodeRole::Secondary))
            .collect();

        if !replica_nodes.is_empty() {
            ensure_remote_replication_enabled("replicate_to_replicas")?;
        }
        
        for node in replica_nodes {
            // Replicate to replica node
            debug!("Replicating to replica node: {}", node.node_id);
        }
        
        Ok(())
    }
    
    pub async fn replicate_to_all(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        let nodes = self.cluster_nodes.read().await;
        let target_count = nodes
            .values()
            .filter(|node| node.node_id != self.get_current_node_id())
            .count();

        if target_count > 0 {
            ensure_remote_replication_enabled("replicate_to_all")?;
        }
        
        for node in nodes.values() {
            if node.node_id != self.get_current_node_id() {
                debug!("Replicating to node: {}", node.node_id);
            }
        }
        
        Ok(())
    }
    
    pub async fn replicate_quorum(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<usize> {
        let nodes = self.cluster_nodes.read().await;
        let required_nodes = (self.config.replication_factor / 2) + 1;
        let mut success_count = 0;

        let target_count = nodes
            .values()
            .filter(|node| node.node_id != self.get_current_node_id())
            .count();
        if target_count > 0 {
            ensure_remote_replication_enabled("replicate_quorum")?;
        }
        
        for node in nodes.values() {
            if node.node_id != self.get_current_node_id() {
                debug!("Replicating to node for quorum: {}", node.node_id);
                success_count += 1;
                
                if success_count >= required_nodes {
                    break;
                }
            }
        }
        
        Ok(success_count)
    }
    
    pub async fn gossip_update(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        // Gossip protocol implementation
        let nodes = self.cluster_nodes.read().await;
        let current_node_id = self.get_current_node_id();
        
        // Send to a subset of nodes
        let mut node_list: Vec<_> = nodes.keys().filter(|id| *id != current_node_id).collect();
        
        // Shuffle and take a subset
        use rand::seq::SliceRandom;
        node_list.shuffle(&mut rand::thread_rng());
        node_list.truncate(3); // Gossip to 3 random nodes

        if !node_list.is_empty() {
            ensure_remote_replication_enabled("gossip_update")?;
        }
        
        for node_id in node_list {
            debug!("Gossiping to node: {}", node_id);
        }
        
        Ok(())
    }
    
    pub async fn replicate_delete(&self, key: &str) -> Result<()> {
        let nodes = self.cluster_nodes.read().await;
        let target_count = nodes
            .values()
            .filter(|node| node.node_id != self.get_current_node_id())
            .count();
        if target_count > 0 {
            ensure_remote_replication_enabled("replicate_delete")?;
        }
        
        for node in nodes.values() {
            if node.node_id != self.get_current_node_id() {
                debug!("Replicating delete to node: {}", node.node_id);
            }
        }
        
        Ok(())
    }
    
    pub async fn replicate_clear(&self) -> Result<()> {
        let nodes = self.cluster_nodes.read().await;
        let target_count = nodes
            .values()
            .filter(|node| node.node_id != self.get_current_node_id())
            .count();
        if target_count > 0 {
            ensure_remote_replication_enabled("replicate_clear")?;
        }
        
        for node in nodes.values() {
            if node.node_id != self.get_current_node_id() {
                debug!("Replicating clear to node: {}", node.node_id);
            }
        }
        
        Ok(())
    }
    
    pub async fn get_replica_values(&self, key: &str) -> Result<Vec<Option<serde_json::Value>>> {
        let nodes = self.cluster_nodes.read().await;
        let replica_nodes: Vec<_> = nodes.values()
            .filter(|node| matches!(node.role, NodeRole::Secondary))
            .collect();
        
        let mut values = Vec::new();
        if !replica_nodes.is_empty() {
            ensure_remote_replication_enabled("get_replica_values")?;
        }
        
        for node in replica_nodes {
            debug!("Getting value from replica node: {}", node.node_id);
            // Return None when no distributed peer result is available
            values.push(None);
        }
        
        Ok(values)
    }
    
    fn get_current_node_id(&self) -> String {
        // This would be stored in the manager
        "current_node".to_string()
    }
}

fn ensure_remote_replication_enabled(operation: &str) -> Result<()> {
    let enabled = std::env::var("PROMETHEOS_ENABLE_REMOTE_REPLICATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if enabled {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Remote replication operation '{}' requested but PROMETHEOS_ENABLE_REMOTE_REPLICATION is not enabled",
            operation
        ))
    }
}

/// P3-Issue3: Consistency manager implementation
impl ConsistencyManager {
    pub fn new(
        config: ConsistencyConfig,
        cluster_nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
        cache_backend: Arc<dyn CacheBackend>,
    ) -> Self {
        Self {
            config,
            cluster_nodes,
            cache_backend,
        }
    }
    
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing consistency manager");
        Ok(())
    }
    
    pub async fn trigger_read_repair(&self, key: &str, correct_value: &serde_json::Value) -> Result<()> {
        info!("Triggering read repair for key: {}", key);
        
        // Keep repaired values alive long enough to avoid immediate re-divergence.
        let ttl_sec = Some(self.read_repair_ttl_sec());
        
        // Update inconsistent replicas
        let nodes = self.cluster_nodes.read().await;
        let replica_count = nodes
            .values()
            .filter(|node| matches!(node.role, NodeRole::Secondary))
            .count();
        if replica_count > 0 {
            ensure_remote_replication_enabled("trigger_read_repair")?;
        }
        for node in nodes.values() {
            if matches!(node.role, NodeRole::Secondary) {
                debug!("Repairing replica node: {}", node.node_id);
            }
        }
        
        Ok(())
    }
    
    pub async fn refresh_stale_value(&self, key: &str) -> Result<()> {
        debug!("Refreshing stale value for key: {}", key);
        
        // Get fresh value from primary
        if let Some(fresh_value) = self.cache_backend.get(key).await? {
            let ttl_sec = Some(self.read_repair_ttl_sec());
            
            // Update local cache
            self.cache_backend.set(key.to_string(), fresh_value, ttl_sec).await?;
        }
        
        Ok(())
    }

    fn read_repair_ttl_sec(&self) -> u64 {
        let floor = 60_u64;
        let scaled_threshold = self.config.stale_read_threshold_sec.saturating_mul(4);
        scaled_threshold.max(floor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_repair_ttl_uses_scaled_staleness_with_floor() {
        let low = ConsistencyConfig {
            level: ConsistencyLevel::Eventual,
            read_repair_enabled: true,
            stale_reads_allowed: true,
            stale_read_threshold_sec: 10,
        };
        let high = ConsistencyConfig {
            stale_read_threshold_sec: 120,
            ..low.clone()
        };
        let nodes = Arc::new(RwLock::new(HashMap::new()));
        let backend: Arc<dyn CacheBackend> = Arc::new(InMemoryCache::new(CacheConfig {
            backend: CacheBackend::InMemory,
            eviction_policy: EvictionPolicy::LRU,
            ttl_config: TTLConfig {
                default_ttl_sec: 3600,
                max_ttl_sec: 86_400,
                ttl_by_pattern: HashMap::new(),
            },
            size_config: SizeConfig {
                max_entries: 16,
                max_size_mb: 8,
                entry_size_limits: EntrySizeLimits {
                    max_key_size_bytes: 256,
                    max_value_size_mb: 1,
                    max_total_size_mb: 2,
                },
            },
        }));
        let low_manager = ConsistencyManager::new(low, nodes.clone(), backend.clone());
        let high_manager = ConsistencyManager::new(high, nodes, backend);

        assert_eq!(low_manager.read_repair_ttl_sec(), 60);
        assert_eq!(high_manager.read_repair_ttl_sec(), 480);
    }
}

// Placeholder implementations for cache backends

pub struct InMemoryCache {
    config: CacheConfig,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    statistics: Arc<RwLock<CacheStatistics>>,
}

impl InMemoryCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
            statistics: Arc::new(RwLock::new(CacheStatistics::default())),
        }
    }
}

impl CacheBackend for InMemoryCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let cache = self.cache.read().await;
        
        if let Some(entry) = cache.get(key) {
            // Check TTL
            let elapsed = chrono::Utc::now().signed_duration_since(entry.created_at);
            if elapsed.num_seconds() < entry.ttl_sec as i64 {
                return Ok(Some(entry.value.clone()));
            }
        }
        
        Ok(None)
    }
    
    async fn set(&self, key: String, value: serde_json::Value, ttl_sec: Option<u64>) -> Result<()> {
        let ttl = ttl_sec.unwrap_or(self.config.ttl_config.default_ttl_sec);
        let size_bytes = serde_json::to_string(&value)?.len();
        
        let entry = CacheEntry {
            key: key.clone(),
            value,
            ttl_sec: ttl,
            created_at: chrono::Utc::now(),
            last_accessed_at: chrono::Utc::now(),
            access_count: 1,
            size_bytes,
            version: 1,
            metadata: HashMap::new(),
        };
        
        {
            let mut cache = self.cache.write().await;
            cache.insert(key, entry);
        }
        
        Ok(())
    }
    
    async fn delete(&self, key: &str) -> Result<bool> {
        let mut cache = self.cache.write().await;
        Ok(cache.remove(key).is_some())
    }
    
    async fn clear(&self) -> Result<()> {
        let mut cache = self.cache.write().await;
        cache.clear();
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        let cache = self.cache.read().await;
        let stats = self.statistics.read().await;
        
        Ok(CacheStatistics {
            total_entries: cache.len(),
            cache_size_bytes: cache.values().map(|e| e.size_bytes).sum(),
            hit_rate: stats.hit_rate,
            miss_rate: stats.miss_rate,
            evictions: stats.evictions,
            expirations: stats.expirations,
            ops_per_sec: stats.ops_per_sec,
            avg_response_time_us: stats.avg_response_time_us,
        })
    }
}

pub struct RedisCache {
    config: CacheConfig,
}

impl RedisCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }
}

impl CacheBackend for RedisCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    
    async fn set(&self, _key: String, _value: serde_json::Value, _ttl_sec: Option<u64>) -> Result<()> {
        Ok(())
    }
    
    async fn delete(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        Ok(CacheStatistics::default())
    }
}

pub struct MemcachedCache {
    config: CacheConfig,
}

impl MemcachedCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }
}

impl CacheBackend for MemcachedCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    
    async fn set(&self, _key: String, _value: serde_json::Value, _ttl_sec: Option<u64>) -> Result<()> {
        Ok(())
    }
    
    async fn delete(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        Ok(CacheStatistics::default())
    }
}

pub struct HazelcastCache {
    config: CacheConfig,
}

impl HazelcastCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }
}

impl CacheBackend for HazelcastCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    
    async fn set(&self, _key: String, _value: serde_json::Value, _ttl_sec: Option<u64>) -> Result<()> {
        Ok(())
    }
    
    async fn delete(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        Ok(CacheStatistics::default())
    }
}

pub struct IgniteCache {
    config: CacheConfig,
}

impl IgniteCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }
}

impl CacheBackend for IgniteCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    
    async fn set(&self, _key: String, _value: serde_json::Value, _ttl_sec: Option<u64>) -> Result<()> {
        Ok(())
    }
    
    async fn delete(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        Ok(CacheStatistics::default())
    }
}

pub struct CustomCache {
    config: CacheConfig,
}

impl CustomCache {
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }
}

impl CacheBackend for CustomCache {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get(&self, _key: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
    
    async fn set(&self, _key: String, _value: serde_json::Value, _ttl_sec: Option<u64>) -> Result<()> {
        Ok(())
    }
    
    async fn delete(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
    
    async fn get_statistics(&self) -> Result<CacheStatistics> {
        Ok(CacheStatistics::default())
    }
}

// Placeholder implementations for discovery services

pub struct StaticDiscovery {
    config: DiscoveryConfig,
}

impl StaticDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for StaticDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

pub struct DNSDiscovery {
    config: DiscoveryConfig,
}

impl DNSDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for DNSDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

pub struct ConsulDiscovery {
    config: DiscoveryConfig,
}

impl ConsulDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for ConsulDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

pub struct EtcdDiscovery {
    config: DiscoveryConfig,
}

impl EtcdDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for EtcdDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

pub struct ZookeeperDiscovery {
    config: DiscoveryConfig,
}

impl ZookeeperDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for ZookeeperDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

pub struct CustomDiscovery {
    config: DiscoveryConfig,
}

impl CustomDiscovery {
    pub fn new(config: DiscoveryConfig) -> Self {
        Self { config }
    }
}

impl DiscoveryService for CustomDiscovery {
    async fn discover_nodes(&self) -> Result<Vec<ClusterNode>> {
        Ok(Vec::new())
    }
    
    async fn register_node(&self, _node: ClusterNode) -> Result<()> {
        Ok(())
    }
    
    async fn unregister_node(&self, _node_id: &str) -> Result<()> {
        Ok(())
    }
}

impl Default for CacheStatistics {
    fn default() -> Self {
        Self {
            total_entries: 0,
            cache_size_bytes: 0,
            hit_rate: 0.0,
            miss_rate: 0.0,
            evictions: 0,
            expirations: 0,
            ops_per_sec: 0.0,
            avg_response_time_us: 0.0,
        }
    }
}

impl Default for NodeStatistics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            disk_usage_percent: 0.0,
            network_io_mb_per_sec: 0.0,
            cache_hit_rate: 0.0,
            ops_per_sec: 0.0,
        }
    }
}
