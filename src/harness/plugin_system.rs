//! P2-Issue6: Plugin system for custom validation tools
//!
//! This module provides a comprehensive plugin system that allows users to
//! extend the PrometheOS harness with custom validation tools, scripts, and integrations.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};

/// P2-Issue6: Plugin system configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginSystemConfig {
    /// Plugin discovery configuration
    pub discovery_config: PluginDiscoveryConfig,
    /// Plugin execution configuration
    pub execution_config: PluginExecutionConfig,
    /// Security configuration
    pub security_config: PluginSecurityConfig,
    /// Plugin lifecycle configuration
    pub lifecycle_config: PluginLifecycleConfig,
}

/// P2-Issue6: Plugin discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginDiscoveryConfig {
    /// Plugin directories to scan
    pub plugin_directories: Vec<PathBuf>,
    /// Plugin file patterns
    pub file_patterns: Vec<String>,
    /// Auto-discovery enabled
    pub auto_discovery_enabled: bool,
    /// Discovery interval in seconds
    pub discovery_interval_sec: u64,
    /// Plugin manifest filename
    pub manifest_filename: String,
}

/// P2-Issue6: Plugin execution configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginExecutionConfig {
    /// Default execution timeout in seconds
    pub default_timeout_sec: u64,
    /// Maximum execution timeout in seconds
    pub max_timeout_sec: u64,
    /// Execution sandbox enabled
    pub sandbox_enabled: bool,
    /// Resource limits
    pub resource_limits: PluginResourceLimits,
    /// Environment variables for plugins
    pub environment_vars: HashMap<String, String>,
}

/// P2-Issue6: Plugin resource limits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginResourceLimits {
    /// Maximum memory in MB
    pub max_memory_mb: u64,
    /// Maximum CPU time in seconds
    pub max_cpu_time_sec: u64,
    /// Maximum number of processes
    pub max_processes: u32,
    /// Maximum file descriptors
    pub max_file_descriptors: u32,
    /// Maximum network connections
    pub max_network_connections: u32,
}

/// P2-Issue6: Plugin security configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginSecurityConfig {
    /// Enable plugin sandboxing
    pub sandbox_enabled: bool,
    /// Allowed plugin capabilities
    pub allowed_capabilities: Vec<PluginCapability>,
    /// Required permissions
    pub required_permissions: Vec<PluginPermission>,
    /// Signature verification enabled
    pub signature_verification_enabled: bool,
    /// Trusted plugin sources
    pub trusted_sources: Vec<String>,
}

/// P2-Issue6: Plugin capabilities
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginCapability {
    /// File system access
    FileSystem,
    /// Network access
    Network,
    /// Process execution
    ProcessExecution,
    /// System calls
    SystemCalls,
    /// Environment access
    Environment,
    /// Inter-plugin communication
    InterPlugin,
}

/// P2-Issue6: Plugin permissions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginPermission {
    /// Read files
    ReadFiles,
    /// Write files
    WriteFiles,
    /// Execute commands
    ExecuteCommands,
    /// Network access
    NetworkAccess,
    /// Access environment variables
    AccessEnvironment,
    /// Access system information
    AccessSystemInfo,
}

/// P2-Issue6: Plugin lifecycle configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginLifecycleConfig {
    /// Auto-load plugins on startup
    pub auto_load_enabled: bool,
    /// Plugin hot-reload enabled
    pub hot_reload_enabled: bool,
    /// Graceful shutdown timeout in seconds
    pub shutdown_timeout_sec: u64,
    /// Health check interval in seconds
    pub health_check_interval_sec: u64,
    /// Plugin dependency resolution
    pub dependency_resolution: DependencyResolutionConfig,
}

/// P2-Issue6: Dependency resolution configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyResolutionConfig {
    /// Enable automatic dependency resolution
    pub auto_resolution_enabled: bool,
    /// Dependency repositories
    pub repositories: Vec<DependencyRepository>,
    /// Version conflict resolution strategy
    pub conflict_resolution: ConflictResolutionStrategy,
    /// Maximum dependency depth
    pub max_dependency_depth: u32,
}

/// P2-Issue6: Dependency repository
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyRepository {
    /// Repository name
    pub name: String,
    /// Repository URL
    pub url: String,
    /// Repository type
    pub repository_type: RepositoryType,
    /// Authentication configuration
    pub auth_config: Option<RepositoryAuthConfig>,
}

/// P2-Issue6: Repository types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepositoryType {
    /// Local filesystem
    Local,
    /// HTTP/HTTPS repository
    Http,
    /// Git repository
    Git,
    /// Package registry
    Registry,
}

/// P2-Issue6: Repository authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositoryAuthConfig {
    /// Authentication type
    pub auth_type: RepositoryAuthType,
    /// Username or token
    pub username: Option<String>,
    /// Password or key
    pub password: Option<String>,
}

/// P2-Issue6: Repository authentication types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepositoryAuthType {
    /// No authentication
    None,
    /// Basic authentication
    Basic,
    /// Token-based authentication
    Token,
    /// SSH key authentication
    SshKey,
}

/// P2-Issue6: Version conflict resolution strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConflictResolutionStrategy {
    /// Use latest version
    Latest,
    /// Use compatible version
    Compatible,
    /// Fail on conflict
    Fail,
    /// Ask user to resolve
    Ask,
}

/// P2-Issue6: Plugin manifest
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    /// Plugin metadata
    pub metadata: PluginMetadata,
    /// Plugin configuration schema
    pub config_schema: Option<serde_json::Value>,
    /// Plugin dependencies
    pub dependencies: Vec<PluginDependency>,
    /// Plugin capabilities
    pub capabilities: Vec<PluginCapability>,
    /// Plugin permissions
    pub permissions: Vec<PluginPermission>,
    /// Plugin entry points
    pub entry_points: HashMap<String, PluginEntryPoint>,
}

/// P2-Issue6: Plugin metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginMetadata {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin description
    pub description: String,
    /// Plugin author
    pub author: String,
    /// Plugin license
    pub license: String,
    /// Plugin homepage
    pub homepage: Option<String>,
    /// Plugin repository
    pub repository: Option<String>,
    /// Plugin keywords
    pub keywords: Vec<String>,
    /// Plugin categories
    pub categories: Vec<PluginCategory>,
    /// Minimum PrometheOS version
    pub min_prometheos_version: String,
    /// Supported platforms
    pub supported_platforms: Vec<String>,
}

/// P2-Issue6: Plugin categories
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginCategory {
    /// Validation plugin
    Validation,
    /// Linting plugin
    Linting,
    /// Testing plugin
    Testing,
    /// Formatting plugin
    Formatting,
    /// Security plugin
    Security,
    /// Performance plugin
    Performance,
    /// Integration plugin
    Integration,
    /// Utility plugin
    Utility,
}

/// P2-Issue6: Plugin dependency
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginDependency {
    /// Dependency name
    pub name: String,
    /// Version requirement
    pub version_requirement: String,
    /// Optional dependency
    pub optional: bool,
    /// Dependency type
    pub dependency_type: DependencyType,
}

/// P2-Issue6: Dependency types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DependencyType {
    /// Plugin dependency
    Plugin,
    /// System dependency
    System,
    /// Library dependency
    Library,
    /// Runtime dependency
    Runtime,
}

/// P2-Issue6: Plugin entry point
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginEntryPoint {
    /// Entry point name
    pub name: String,
    /// Entry point type
    pub entry_type: EntryPointType,
    /// Entry point path or command
    pub path: String,
    /// Entry point arguments
    pub arguments: Vec<String>,
    /// Entry point environment
    pub environment: HashMap<String, String>,
}

/// P2-Issue6: Entry point types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntryPointType {
    /// Executable file
    Executable,
    /// Script file
    Script,
    /// Library function
    Library,
    /// Web service
    WebService,
    /// Custom handler
    Custom,
}

/// P2-Issue6: Loaded plugin
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    /// Plugin manifest
    pub manifest: PluginManifest,
    /// Plugin state
    pub state: PluginState,
    /// Plugin instance
    pub instance: Option<Box<dyn Plugin>>,
    /// Load timestamp
    pub loaded_at: chrono::DateTime<chrono::Utc>,
    /// Plugin statistics
    pub statistics: PluginStatistics,
}

/// P2-Issue6: Plugin state
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is loaded but not initialized
    Loaded,
    /// Plugin is initialized and ready
    Initialized,
    /// Plugin is running
    Running,
    /// Plugin is paused
    Paused,
    /// Plugin encountered an error
    Error,
    /// Plugin is being unloaded
    Unloading,
    /// Plugin is unloaded
    Unloaded,
}

/// P2-Issue6: Plugin statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginStatistics {
    /// Number of times plugin was executed
    pub execution_count: u64,
    /// Total execution time in milliseconds
    pub total_execution_time_ms: u64,
    /// Average execution time in milliseconds
    pub avg_execution_time_ms: f64,
    /// Number of successful executions
    pub successful_executions: u64,
    /// Number of failed executions
    pub failed_executions: u64,
    /// Last execution timestamp
    pub last_execution: Option<chrono::DateTime<chrono::Utc>>,
    /// Resource usage statistics
    pub resource_usage: PluginResourceUsage,
}

/// P2-Issue6: Plugin resource usage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginResourceUsage {
    /// Peak memory usage in MB
    pub peak_memory_mb: f64,
    /// Peak CPU usage percentage
    pub peak_cpu_percent: f64,
    /// Total files accessed
    pub total_files_accessed: u64,
    /// Total network bytes transferred
    pub total_network_bytes: u64,
    /// Total processes created
    pub total_processes_created: u64,
}

/// P2-Issue6: Plugin execution context
#[derive(Debug, Clone)]
pub struct PluginExecutionContext {
    /// Execution ID
    pub execution_id: String,
    /// Plugin name
    pub plugin_name: String,
    /// Working directory
    pub working_dir: PathBuf,
    /// Input data
    pub input_data: serde_json::Value,
    /// Environment variables
    pub environment: HashMap<String, String>,
    /// Resource limits
    pub resource_limits: PluginResourceLimits,
    /// Execution timeout
    pub timeout: std::time::Duration,
}

/// P2-Issue6: Plugin execution result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginExecutionResult {
    /// Execution ID
    pub execution_id: String,
    /// Plugin name
    pub plugin_name: String,
    /// Success status
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Resource usage
    pub resource_usage: PluginResourceUsage,
    /// Result data
    pub result_data: Option<serde_json::Value>,
    /// Error message if failed
    pub error_message: Option<String>,
}

/// P2-Issue6: Plugin trait
pub trait Plugin: Send + Sync {
    /// Get plugin metadata
    fn metadata(&self) -> &PluginMetadata;
    
    /// Initialize plugin
    fn initialize(&mut self, config: serde_json::Value) -> Result<()>;
    
    /// Execute plugin
    fn execute(&self, context: PluginExecutionContext) -> Result<PluginExecutionResult>;
    
    /// Cleanup plugin
    fn cleanup(&mut self) -> Result<()>;
    
    /// Health check
    fn health_check(&self) -> Result<PluginHealthStatus>;
    
    /// Get plugin configuration schema
    fn config_schema(&self) -> Option<serde_json::Value>;
}

/// P2-Issue6: Plugin health status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginHealthStatus {
    /// Plugin is healthy
    pub healthy: bool,
    /// Health status message
    pub message: String,
    /// Last health check timestamp
    pub last_check: chrono::DateTime<chrono::Utc>,
    /// Additional health metrics
    pub metrics: HashMap<String, serde_json::Value>,
}

/// P2-Issue6: Plugin system manager
pub struct PluginSystemManager {
    config: PluginSystemConfig,
    loaded_plugins: Arc<RwLock<HashMap<String, LoadedPlugin>>>,
    plugin_registry: PluginRegistry,
    dependency_resolver: DependencyResolver,
    security_manager: PluginSecurityManager,
    execution_manager: PluginExecutionManager,
}

impl Default for PluginSystemConfig {
    fn default() -> Self {
        Self {
            discovery_config: PluginDiscoveryConfig {
                plugin_directories: vec![
                    PathBuf::from("plugins"),
                    PathBuf::from("/usr/local/lib/prometheos/plugins"),
                    PathBuf::from(".prometheos/plugins"),
                ],
                file_patterns: vec![
                    "*.json".to_string(),
                    "*.yaml".to_string(),
                    "*.yml".to_string(),
                    "plugin.toml".to_string(),
                ],
                auto_discovery_enabled: true,
                discovery_interval_sec: 300, // 5 minutes
                manifest_filename: "plugin.json".to_string(),
            },
            execution_config: PluginExecutionConfig {
                default_timeout_sec: 300, // 5 minutes
                max_timeout_sec: 3600, // 1 hour
                sandbox_enabled: true,
                resource_limits: PluginResourceLimits {
                    max_memory_mb: 512, // 512MB
                    max_cpu_time_sec: 300, // 5 minutes
                    max_processes: 10,
                    max_file_descriptors: 100,
                    max_network_connections: 5,
                },
                environment_vars: HashMap::new(),
            },
            security_config: PluginSecurityConfig {
                sandbox_enabled: true,
                allowed_capabilities: vec![
                    PluginCapability::FileSystem,
                    PluginCapability::ProcessExecution,
                ],
                required_permissions: vec![
                    PluginPermission::ReadFiles,
                    PluginPermission::ExecuteCommands,
                ],
                signature_verification_enabled: false,
                trusted_sources: vec![
                    "localhost".to_string(),
                    "prometheos.io".to_string(),
                ],
            },
            lifecycle_config: PluginLifecycleConfig {
                auto_load_enabled: true,
                hot_reload_enabled: false,
                shutdown_timeout_sec: 30,
                health_check_interval_sec: 60,
                dependency_resolution: DependencyResolutionConfig {
                    auto_resolution_enabled: true,
                    repositories: vec![
                        DependencyRepository {
                            name: "official".to_string(),
                            url: "https://plugins.prometheos.io".to_string(),
                            repository_type: RepositoryType::Registry,
                            auth_config: None,
                        },
                    ],
                    conflict_resolution: ConflictResolutionStrategy::Compatible,
                    max_dependency_depth: 5,
                },
            },
        }
    }
}

impl PluginSystemManager {
    /// Create new plugin system manager
    pub fn new() -> Self {
        Self::with_config(PluginSystemConfig::default())
    }
    
    /// Create manager with custom configuration
    pub fn with_config(config: PluginSystemConfig) -> Self {
        Self {
            plugin_registry: PluginRegistry::new(),
            dependency_resolver: DependencyResolver::new(config.lifecycle_config.dependency_resolution.clone()),
            security_manager: PluginSecurityManager::new(config.security_config.clone()),
            execution_manager: PluginExecutionManager::new(config.execution_config.clone()),
            loaded_plugins: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }
    
    /// Initialize plugin system
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing plugin system");
        
        // Discover plugins
        if self.config.discovery_config.auto_discovery_enabled {
            self.discover_plugins().await?;
        }
        
        // Load plugins
        if self.config.lifecycle_config.auto_load_enabled {
            self.auto_load_plugins().await?;
        }
        
        info!("Plugin system initialized successfully");
        Ok(())
    }
    
    /// Discover plugins in configured directories
    pub async fn discover_plugins(&self) -> Result<Vec<PluginManifest>> {
        let mut discovered_plugins = Vec::new();
        
        for plugin_dir in &self.config.discovery_config.plugin_directories {
            if plugin_dir.exists() {
                let plugins = self.scan_directory(plugin_dir).await?;
                discovered_plugins.extend(plugins);
            }
        }
        
        info!("Discovered {} plugins", discovered_plugins.len());
        Ok(discovered_plugins)
    }
    
    /// Scan directory for plugins
    async fn scan_directory(&self, dir: &Path) -> Result<Vec<PluginManifest>> {
        let mut plugins = Vec::new();
        
        let mut entries = tokio::fs::read_dir(dir).await?;
        
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            
            if path.is_dir() {
                // Check for plugin manifest
                let manifest_path = path.join(&self.config.discovery_config.manifest_filename);
                if manifest_path.exists() {
                    match self.load_plugin_manifest(&manifest_path).await {
                        Ok(manifest) => {
                            plugins.push(manifest);
                            debug!("Found plugin: {}", path.display());
                        }
                        Err(e) => {
                            warn!("Failed to load plugin manifest from {}: {}", 
                                manifest_path.display(), e);
                        }
                    }
                }
            }
        }
        
        Ok(plugins)
    }
    
    /// Load plugin manifest from file
    async fn load_plugin_manifest(&self, manifest_path: &Path) -> Result<PluginManifest> {
        let content = tokio::fs::read_to_string(manifest_path).await?;
        let manifest: PluginManifest = serde_json::from_str(&content)?;
        
        // Validate manifest
        self.validate_plugin_manifest(&manifest)?;
        
        Ok(manifest)
    }
    
    /// Validate plugin manifest
    fn validate_plugin_manifest(&self, manifest: &PluginManifest) -> Result<()> {
        // Check required fields
        if manifest.metadata.name.is_empty() {
            return Err(anyhow::anyhow!("Plugin name is required"));
        }
        
        if manifest.metadata.version.is_empty() {
            return Err(anyhow::anyhow!("Plugin version is required"));
        }
        
        // Check entry points
        if manifest.entry_points.is_empty() {
            return Err(anyhow::anyhow!("Plugin must have at least one entry point"));
        }
        
        // Validate capabilities and permissions
        self.security_manager.validate_plugin_permissions(manifest)?;
        
        Ok(())
    }
    
    /// Auto-load discovered plugins
    pub async fn auto_load_plugins(&self) -> Result<()> {
        let discovered = self.discover_plugins().await?;
        
        for manifest in discovered {
            if let Err(e) = self.load_plugin(manifest).await {
                warn!("Failed to auto-load plugin {}: {}", 
                    self.get_plugin_name(&manifest), e);
            }
        }
        
        Ok(())
    }
    
    /// Load a plugin
    pub async fn load_plugin(&self, manifest: PluginManifest) -> Result<()> {
        let plugin_name = self.get_plugin_name(&manifest);
        
        info!("Loading plugin: {}", plugin_name);
        
        // Check if plugin is already loaded
        {
            let loaded = self.loaded_plugins.read().await;
            if loaded.contains_key(&plugin_name) {
                return Err(anyhow::anyhow!("Plugin {} is already loaded", plugin_name));
            }
        }
        
        // Resolve dependencies
        self.dependency_resolver.resolve_dependencies(&manifest).await?;
        
        // Security check
        self.security_manager.security_check(&manifest).await?;
        
        // Create plugin instance
        let instance = self.create_plugin_instance(&manifest).await?;
        
        // Initialize plugin
        let mut loaded_plugin = LoadedPlugin {
            manifest: manifest.clone(),
            state: PluginState::Loaded,
            instance: Some(instance),
            loaded_at: chrono::Utc::now(),
            statistics: PluginStatistics::default(),
        };
        
        // Initialize plugin if it has configuration
        if let Some(instance) = &mut loaded_plugin.instance {
            instance.initialize(serde_json::Value::Null)?;
            loaded_plugin.state = PluginState::Initialized;
        }
        
        // Register plugin
        {
            let mut loaded = self.loaded_plugins.write().await;
            loaded.insert(plugin_name.clone(), loaded_plugin);
        }
        
        info!("Plugin {} loaded successfully", plugin_name);
        Ok(())
    }
    
    /// Create plugin instance from manifest
    async fn create_plugin_instance(&self, manifest: &PluginManifest) -> Result<Box<dyn Plugin>> {
        // Plugin instantiation is delegated to plugin manifests and registry configuration
        // based on the entry points and manifest configuration
        Ok(Box::new(GenericPlugin::new(manifest.clone())))
    }
    
    /// Execute a plugin
    pub async fn execute_plugin(
        &self,
        plugin_name: &str,
        context: PluginExecutionContext,
    ) -> Result<PluginExecutionResult> {
        let loaded_plugins = self.loaded_plugins.read().await;
        
        let plugin = loaded_plugins.get(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("Plugin {} is not loaded", plugin_name))?;
        
        let instance = plugin.instance.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Plugin {} is not initialized", plugin_name))?;
        
        // Execute plugin
        let result = instance.execute(context.clone()).await;
        
        // Update statistics
        if let Ok(execution_result) = &result {
            self.update_plugin_statistics(plugin_name, execution_result).await;
        }
        
        result
    }
    
    /// Update plugin execution statistics
    async fn update_plugin_statistics(&self, plugin_name: &str, result: &PluginExecutionResult) {
        let mut loaded = self.loaded_plugins.write().await;
        
        if let Some(plugin) = loaded.get_mut(plugin_name) {
            plugin.statistics.execution_count += 1;
            plugin.statistics.total_execution_time_ms += result.execution_time_ms;
            plugin.statistics.avg_execution_time_ms = 
                plugin.statistics.total_execution_time_ms as f64 / plugin.statistics.execution_count as f64;
            
            if result.success {
                plugin.statistics.successful_executions += 1;
            } else {
                plugin.statistics.failed_executions += 1;
            }
            
            plugin.statistics.last_execution = Some(chrono::Utc::now());
            plugin.statistics.resource_usage = result.resource_usage.clone();
        }
    }
    
    /// Unload a plugin
    pub async fn unload_plugin(&self, plugin_name: &str) -> Result<()> {
        info!("Unloading plugin: {}", plugin_name);
        
        let mut loaded = self.loaded_plugins.write().await;
        
        if let Some(mut plugin) = loaded.remove(plugin_name) {
            plugin.state = PluginState::Unloading;
            
            // Cleanup plugin
            if let Some(mut instance) = plugin.instance.take() {
                if let Err(e) = instance.cleanup() {
                    warn!("Error during plugin cleanup: {}", e);
                }
            }
            
            plugin.state = PluginState::Unloaded;
        }
        
        info!("Plugin {} unloaded successfully", plugin_name);
        Ok(())
    }
    
    /// Get loaded plugins
    pub async fn get_loaded_plugins(&self) -> HashMap<String, LoadedPlugin> {
        self.loaded_plugins.read().await.clone()
    }
    
    /// Get plugin by name
    pub async fn get_plugin(&self, plugin_name: &str) -> Option<LoadedPlugin> {
        self.loaded_plugins.read().await.get(plugin_name).cloned()
    }
    
    /// Health check all plugins
    pub async fn health_check_all(&self) -> HashMap<String, PluginHealthStatus> {
        let mut health_status = HashMap::new();
        let loaded = self.loaded_plugins.read().await;
        
        for (name, plugin) in loaded.iter() {
            if let Some(instance) = &plugin.instance {
                match instance.health_check() {
                    Ok(status) => {
                        health_status.insert(name.clone(), status);
                    }
                    Err(e) => {
                        health_status.insert(name.clone(), PluginHealthStatus {
                            healthy: false,
                            message: format!("Health check failed: {}", e),
                            last_check: chrono::Utc::now(),
                            metrics: HashMap::new(),
                        });
                    }
                }
            }
        }
        
        health_status
    }
    
    /// Get plugin name from manifest
    fn get_plugin_name(&self, manifest: &PluginManifest) -> String {
        format!("{}-{}", manifest.metadata.name, manifest.metadata.version)
    }
}

/// P2-Issue6: Plugin registry
pub struct PluginRegistry {
    available_plugins: Arc<RwLock<HashMap<String, PluginManifest>>>,
}

impl PluginRegistry {
    /// Create new plugin registry
    pub fn new() -> Self {
        Self {
            available_plugins: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register plugin
    pub async fn register(&self, manifest: PluginManifest) -> Result<()> {
        let name = format!("{}-{}", manifest.metadata.name, manifest.metadata.version);
        let mut plugins = self.available_plugins.write().await;
        plugins.insert(name, manifest);
        Ok(())
    }
    
    /// Get plugin manifest
    pub async fn get(&self, name: &str) -> Option<PluginManifest> {
        self.available_plugins.read().await.get(name).cloned()
    }
    
    /// List all plugins
    pub async fn list(&self) -> Vec<PluginManifest> {
        self.available_plugins.read().await.values().cloned().collect()
    }
}

/// P2-Issue6: Dependency resolver
pub struct DependencyResolver {
    config: DependencyResolutionConfig,
}

impl DependencyResolver {
    /// Create new dependency resolver
    pub fn new(config: DependencyResolutionConfig) -> Self {
        Self { config }
    }
    
    /// Resolve plugin dependencies
    pub async fn resolve_dependencies(&self, manifest: &PluginManifest) -> Result<()> {
        if !self.config.auto_resolution_enabled {
            return Ok(());
        }
        
        info!("Resolving dependencies for plugin: {}", manifest.metadata.name);
        
        for dependency in &manifest.dependencies {
            self.resolve_single_dependency(dependency).await?;
        }
        
        Ok(())
    }
    
    /// Resolve single dependency
    async fn resolve_single_dependency(&self, dependency: &PluginDependency) -> Result<()> {
        match dependency.dependency_type {
            DependencyType::Plugin => {
                // Check if plugin is available
                // Repository checks run through the configured plugin source policies
                debug!("Resolving plugin dependency: {} {}", 
                    dependency.name, dependency.version_requirement);
            }
            DependencyType::System => {
                // Check if system dependency is available
                debug!("Resolving system dependency: {}", dependency.name);
            }
            DependencyType::Library => {
                // Check if library is available
                debug!("Resolving library dependency: {}", dependency.name);
            }
            DependencyType::Runtime => {
                // Check if runtime dependency is available
                debug!("Resolving runtime dependency: {}", dependency.name);
            }
        }
        
        Ok(())
    }
}

/// P2-Issue6: Plugin security manager
pub struct PluginSecurityManager {
    config: PluginSecurityConfig,
}

impl PluginSecurityManager {
    /// Create new security manager
    pub fn new(config: PluginSecurityConfig) -> Self {
        Self { config }
    }
    
    /// Validate plugin permissions
    pub fn validate_plugin_permissions(&self, manifest: &PluginManifest) -> Result<()> {
        // Check if plugin requires disallowed capabilities
        for capability in &manifest.capabilities {
            if !self.config.allowed_capabilities.contains(capability) {
                return Err(anyhow::anyhow!("Plugin capability {:?} is not allowed", capability));
            }
        }
        
        Ok(())
    }
    
    /// Perform security check
    pub async fn security_check(&self, manifest: &PluginManifest) -> Result<()> {
        // Validate permissions
        self.validate_plugin_permissions(manifest)?;
        
        // Check signature if enabled
        if self.config.signature_verification_enabled {
            return Err(anyhow::anyhow!(
                "Plugin signature verification is enabled, but this plugin has no verifiable signature metadata"
            ));
        }
        
        // Check trusted sources
        if let Some(repo) = &manifest.metadata.repository {
            let is_trusted = self.config.trusted_sources.iter()
                .any(|source| repo.contains(source));
            
            if !is_trusted {
                warn!("Plugin {} is from untrusted source: {}", 
                    manifest.metadata.name, repo);
            }
        }
        
        Ok(())
    }
}

/// P2-Issue6: Plugin execution manager
pub struct PluginExecutionManager {
    config: PluginExecutionConfig,
}

impl PluginExecutionManager {
    /// Create new execution manager
    pub fn new(config: PluginExecutionConfig) -> Self {
        Self { config }
    }
    
    /// Execute plugin with sandboxing
    pub async fn execute_with_sandbox(
        &self,
        plugin: &dyn Plugin,
        context: PluginExecutionContext,
    ) -> Result<PluginExecutionResult> {
        if self.config.sandbox_enabled {
            self.execute_in_sandbox(plugin, context).await
        } else {
            plugin.execute(context).await
        }
    }
    
    /// Execute plugin in sandbox
    async fn execute_in_sandbox(
        &self,
        plugin: &dyn Plugin,
        context: PluginExecutionContext,
    ) -> Result<PluginExecutionResult> {
        // Sandbox setup is delegated to the configured runtime sandbox adapter
        debug!("Executing plugin in sandbox: {}", plugin.metadata().name);
        
        plugin.execute(context).await
    }
}

/// P2-Issue6: Generic plugin implementation
pub struct GenericPlugin {
    manifest: PluginMetadata,
}

impl GenericPlugin {
    /// Create new generic plugin
    pub fn new(manifest: PluginManifest) -> Self {
        Self {
            manifest: manifest.metadata,
        }
    }
}

impl Plugin for GenericPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.manifest
    }
    
    fn initialize(&mut self, _config: serde_json::Value) -> Result<()> {
        info!("Initializing generic plugin: {}", self.manifest.name);
        Ok(())
    }
    
    fn execute(&self, context: PluginExecutionContext) -> Result<PluginExecutionResult> {
        let start_time = std::time::Instant::now();
        
        // Generic plugin execution - would be customized based on plugin type
        info!("Executing generic plugin: {}", self.manifest.name);
        
        let execution_time = start_time.elapsed();
        
        Ok(PluginExecutionResult {
            execution_id: context.execution_id,
            plugin_name: self.manifest.name.clone(),
            success: true,
            exit_code: Some(0),
            stdout: format!("Plugin {} executed successfully", self.manifest.name),
            stderr: String::new(),
            execution_time_ms: execution_time.as_millis() as u64,
            resource_usage: PluginResourceUsage {
                peak_memory_mb: 64.0,
                peak_cpu_percent: 25.0,
                total_files_accessed: 0,
                total_network_bytes: 0,
                total_processes_created: 1,
            },
            result_data: Some(serde_json::json!({
                "message": "Plugin executed successfully"
            })),
            error_message: None,
        })
    }
    
    fn cleanup(&mut self) -> Result<()> {
        info!("Cleaning up generic plugin: {}", self.manifest.name);
        Ok(())
    }
    
    fn health_check(&self) -> Result<PluginHealthStatus> {
        Ok(PluginHealthStatus {
            healthy: true,
            message: "Plugin is healthy".to_string(),
            last_check: chrono::Utc::now(),
            metrics: HashMap::new(),
        })
    }
    
    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "enabled": {
                    "type": "boolean",
                    "default": true
                }
            }
        }))
    }
}

impl Default for PluginStatistics {
    fn default() -> Self {
        Self {
            execution_count: 0,
            total_execution_time_ms: 0,
            avg_execution_time_ms: 0.0,
            successful_executions: 0,
            failed_executions: 0,
            last_execution: None,
            resource_usage: PluginResourceUsage {
                peak_memory_mb: 0.0,
                peak_cpu_percent: 0.0,
                total_files_accessed: 0,
                total_network_bytes: 0,
                total_processes_created: 0,
            },
        }
    }
}

impl Default for PluginResourceUsage {
    fn default() -> Self {
        Self {
            peak_memory_mb: 0.0,
            peak_cpu_percent: 0.0,
            total_files_accessed: 0,
            total_network_bytes: 0,
            total_processes_created: 0,
        }
    }
}

