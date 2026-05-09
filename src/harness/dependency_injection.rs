//! Advanced dependency injection and IoC container
//!
//! This module provides a comprehensive dependency injection framework with
//! IoC container, service lifetime management, and advanced configuration capabilities.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use walkdir::WalkDir;

/// P3-Issue1: Dependency injection configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DependencyInjectionConfig {
    /// Container configuration
    pub container_config: ContainerConfig,
    /// Service discovery configuration
    pub discovery_config: ServiceDiscoveryConfig,
    /// Lifetime management configuration
    pub lifetime_config: LifetimeManagementConfig,
    /// Proxy configuration
    pub proxy_config: ProxyConfig,
}

/// P3-Issue1: Container configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerConfig {
    /// Auto-registration enabled
    pub auto_registration_enabled: bool,
    /// Scan packages for services
    pub scan_packages: Vec<String>,
    /// Registration order
    pub registration_order: RegistrationOrder,
    /// Circular dependency handling
    pub circular_dependency_handling: CircularDependencyHandling,
    /// Validation enabled
    pub validation_enabled: bool,
}

/// P3-Issue1: Registration order
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RegistrationOrder {
    /// Register dependencies first
    DependenciesFirst,
    /// Register services first
    ServicesFirst,
    /// Alphabetical order
    Alphabetical,
    /// Custom order
    Custom(Vec<String>),
}

/// P3-Issue1: Circular dependency handling
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CircularDependencyHandling {
    /// Throw error on circular dependency
    Error,
    /// Allow circular dependencies
    Allow,
    /// Break circular dependencies
    Break,
    /// Use lazy resolution
    Lazy,
}

/// P3-Issue1: Service discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDiscoveryConfig {
    /// Discovery enabled
    pub enabled: bool,
    /// Discovery mechanisms
    pub mechanisms: Vec<DiscoveryMechanism>,
    /// Refresh interval in seconds
    pub refresh_interval_sec: u64,
    /// Health check enabled
    pub health_check_enabled: bool,
}

/// P3-Issue1: Discovery mechanisms
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryMechanism {
    /// Mechanism name
    pub name: String,
    /// Mechanism type
    pub mechanism_type: DiscoveryMechanismType,
    /// Configuration
    pub config: serde_json::Value,
    /// Priority
    pub priority: u8,
}

/// P3-Issue1: Discovery mechanism types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveryMechanismType {
    /// File system discovery
    FileSystem,
    /// Environment variable discovery
    Environment,
    /// Configuration file discovery
    ConfigurationFile,
    /// Network service discovery
    Network,
    /// Database discovery
    Database,
    /// Custom discovery
    Custom,
}

/// P3-Issue1: Lifetime management configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifetimeManagementConfig {
    /// Default lifetime
    pub default_lifetime: ServiceLifetime,
    /// Lifetime policies
    pub lifetime_policies: HashMap<String, ServiceLifetime>,
    /// GC enabled
    pub gc_enabled: bool,
    /// GC interval in seconds
    pub gc_interval_sec: u64,
    /// Max instances per service
    pub max_instances_per_service: HashMap<String, usize>,
}

/// P3-Issue1: Service lifetimes
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceLifetime {
    /// Transient - new instance every time
    Transient,
    /// Singleton - single instance for container
    Singleton,
    /// Scoped - single instance per scope
    Scoped,
    /// Custom lifetime
    Custom(String),
}

/// P3-Issue1: Proxy configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyConfig {
    /// Proxy generation enabled
    pub proxy_generation_enabled: bool,
    /// Proxy types
    pub proxy_types: Vec<ProxyType>,
    /// Interceptor configuration
    pub interceptor_config: InterceptorConfig,
}

/// P3-Issue1: Proxy types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProxyType {
    /// Virtual proxy
    Virtual,
    /// Protection proxy
    Protection,
    /// Remote proxy
    Remote,
    /// Smart proxy
    Smart,
}

/// P3-Issue1: Interceptor configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InterceptorConfig {
    /// Enable method interception
    pub method_interception_enabled: bool,
    /// Enable property interception
    pub property_interception_enabled: bool,
    /// Global interceptors
    pub global_interceptors: Vec<InterceptorType>,
}

/// P3-Issue1: Interceptor types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InterceptorType {
    /// Logging interceptor
    Logging,
    /// Caching interceptor
    Caching,
    /// Security interceptor
    Security,
    /// Performance interceptor
    Performance,
    /// Transaction interceptor
    Transaction,
}

/// P3-Issue1: Service descriptor
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDescriptor {
    /// Service type
    pub service_type: String,
    /// Implementation type
    pub implementation_type: String,
    /// Service lifetime
    pub lifetime: ServiceLifetime,
    /// Dependencies
    pub dependencies: Vec<ServiceDependency>,
    /// Properties
    pub properties: HashMap<String, serde_json::Value>,
    /// Interceptors
    pub interceptors: Vec<InterceptorType>,
    /// Metadata
    pub metadata: ServiceMetadata,
}

/// P3-Issue1: Service dependency
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDependency {
    /// Dependency name
    pub name: String,
    /// Dependency type
    pub dependency_type: String,
    /// Required (true) or optional (false)
    pub required: bool,
    /// Lazy injection
    pub lazy: bool,
}

/// P3-Issue1: Service metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceMetadata {
    /// Service name
    pub name: String,
    /// Service version
    pub version: String,
    /// Service description
    pub description: String,
    /// Service tags
    pub tags: Vec<String>,
    /// Service category
    pub category: String,
}

/// P3-Issue1: IoC container
pub struct IoCContainer {
    config: DependencyInjectionConfig,
    services: Arc<RwLock<HashMap<String, ServiceRegistration>>>,
    instances: Arc<RwLock<HashMap<String, Arc<dyn Service>>>>,
    scopes: Arc<RwLock<Vec<ServiceScope>>>,
    interceptors: Arc<RwLock<HashMap<String, Vec<Box<dyn Interceptor>>>>>,
    discovery_engine: ServiceDiscoveryEngine,
    lifetime_manager: LifetimeManager,
    proxy_factory: ProxyFactory,
}

/// P3-Issue1: Service registration
#[derive(Debug, Clone)]
pub struct ServiceRegistration {
    /// Service descriptor
    pub descriptor: ServiceDescriptor,
    /// Service factory
    pub factory: Option<Box<dyn ServiceFactory>>,
    /// Service instance (for singletons)
    pub instance: Option<Arc<dyn Service>>,
    /// Registration timestamp
    pub registered_at: chrono::DateTime<chrono::Utc>,
}

/// P3-Issue1: Service scope
#[derive(Debug, Clone)]
pub struct ServiceScope {
    /// Scope ID
    pub id: String,
    /// Scope name
    pub name: String,
    /// Scope instances
    pub instances: HashMap<String, Arc<dyn Service>>,
    /// Scope created at
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Scope parent
    pub parent: Option<String>,
}

/// P3-Issue1: Service trait
pub trait Service: Send + Sync {
    /// Get service type
    fn get_service_type(&self) -> &str;
    /// Get service metadata
    fn get_metadata(&self) -> &ServiceMetadata;
    /// Initialize service
    fn initialize(&mut self) -> Result<()>;
    /// Cleanup service
    fn cleanup(&mut self) -> Result<()>;
}

/// P3-Issue1: Service factory trait
pub trait ServiceFactory: Send + Sync {
    /// Create service instance
    fn create_instance(&self, container: &IoCContainer) -> Result<Arc<dyn Service>>;
    /// Get service type
    fn get_service_type(&self) -> &str;
}

/// P3-Issue1: Interceptor trait
pub trait Interceptor: Send + Sync {
    /// Intercept method call
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult>;
    /// Get interceptor type
    fn get_interceptor_type(&self) -> InterceptorType;
}

/// P3-Issue1: Interceptor context
#[derive(Debug, Clone)]
pub struct InterceptorContext {
    /// Service instance
    pub service: Arc<dyn Service>,
    /// Method name
    pub method_name: String,
    /// Method arguments
    pub arguments: Vec<serde_json::Value>,
    /// Method result
    pub result: Option<serde_json::Value>,
    /// Context metadata
    pub metadata: HashMap<String, String>,
}

/// P3-Issue1: Interceptor result
#[derive(Debug, Clone)]
pub enum InterceptorResult {
    /// Continue execution
    Continue,
    /// Return result
    Return(serde_json::Value),
    /// Throw error
    Error(anyhow::Error),
    /// Skip execution
    Skip,
}

/// P3-Issue1: Service discovery engine
pub struct ServiceDiscoveryEngine {
    config: ServiceDiscoveryConfig,
    mechanisms: Vec<Box<dyn DiscoveryMechanism>>,
}

/// P3-Issue1: Discovery mechanism trait
pub trait DiscoveryMechanism: Send + Sync {
    /// Discover services
    fn discover_services(&self) -> Result<Vec<ServiceDescriptor>>;
    /// Get mechanism name
    fn get_name(&self) -> &str;
}

/// P3-Issue1: Lifetime manager
pub struct LifetimeManager {
    config: LifetimeManagementConfig,
    gc_scheduler: Arc<RwLock<GCScheduler>>,
}

/// P3-Issue1: GC scheduler
pub struct GCScheduler {
    interval: Duration,
    last_run: chrono::DateTime<chrono::Utc>,
    running: bool,
}

/// P3-Issue1: Proxy factory
pub struct ProxyFactory {
    config: ProxyConfig,
    proxy_generators: HashMap<ProxyType, Box<dyn ProxyGenerator>>,
}

/// P3-Issue1: Proxy generator trait
pub trait ProxyGenerator: Send + Sync {
    /// Generate proxy
    fn generate_proxy(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>>;
    /// Get proxy type
    fn get_proxy_type(&self) -> ProxyType;
}

impl Default for DependencyInjectionConfig {
    fn default() -> Self {
        Self {
            container_config: ContainerConfig {
                auto_registration_enabled: true,
                scan_packages: vec![
                    "prometheos.harness".to_string(),
                    "prometheos.validation".to_string(),
                    "prometheos.sandbox".to_string(),
                ],
                registration_order: RegistrationOrder::DependenciesFirst,
                circular_dependency_handling: CircularDependencyHandling::Error,
                validation_enabled: true,
            },
            discovery_config: ServiceDiscoveryConfig {
                enabled: true,
                mechanisms: vec![
                    DiscoveryMechanism {
                        name: "filesystem".to_string(),
                        mechanism_type: DiscoveryMechanismType::FileSystem,
                        config: serde_json::json!({
                            "scan_path": "src/harness",
                            "pattern": "*.rs"
                        }),
                        priority: 100,
                    },
                    DiscoveryMechanism {
                        name: "environment".to_string(),
                        mechanism_type: DiscoveryMechanismType::Environment,
                        config: serde_json::json!({
                            "prefix": "PROMETHEOS_"
                        }),
                        priority: 90,
                    },
                ],
                refresh_interval_sec: 300, // 5 minutes
                health_check_enabled: true,
            },
            lifetime_config: LifetimeManagementConfig {
                default_lifetime: ServiceLifetime::Transient,
                lifetime_policies: HashMap::new(),
                gc_enabled: true,
                gc_interval_sec: 3600, // 1 hour
                max_instances_per_service: HashMap::new(),
            },
            proxy_config: ProxyConfig {
                proxy_generation_enabled: true,
                proxy_types: vec![ProxyType::Virtual, ProxyType::Smart],
                interceptor_config: InterceptorConfig {
                    method_interception_enabled: true,
                    property_interception_enabled: false,
                    global_interceptors: vec![
                        InterceptorType::Logging,
                        InterceptorType::Performance,
                    ],
                },
            },
        }
    }
}

impl IoCContainer {
    /// Create new IoC container
    pub fn new() -> Self {
        Self::with_config(DependencyInjectionConfig::default())
    }
    
    /// Create container with custom configuration
    pub fn with_config(config: DependencyInjectionConfig) -> Self {
        let discovery_engine = ServiceDiscoveryEngine::new(config.discovery_config.clone());
        let lifetime_manager = LifetimeManager::new(config.lifetime_config.clone());
        let proxy_factory = ProxyFactory::new(config.proxy_config.clone());
        
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            instances: Arc::new(RwLock::new(HashMap::new())),
            scopes: Arc::new(RwLock::new(Vec::new())),
            interceptors: Arc::new(RwLock::new(HashMap::new())),
            discovery_engine,
            lifetime_manager,
            proxy_factory,
            config,
        }
    }
    
    /// Initialize container
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing IoC container");
        
        // Auto-discover services if enabled
        if self.config.discovery_config.enabled {
            self.discover_services().await?;
        }
        
        // Start lifetime manager
        self.lifetime_manager.start().await?;
        
        // Validate configuration
        if self.config.container_config.validation_enabled {
            self.validate_configuration().await?;
        }
        
        info!("IoC container initialized successfully");
        Ok(())
    }
    
    /// Register service
    pub async fn register_service(&self, descriptor: ServiceDescriptor) -> Result<()> {
        debug!("Registering service: {}", descriptor.service_type);
        
        // Validate service descriptor
        self.validate_service_descriptor(&descriptor)?;
        
        // Check for circular dependencies
        if matches!(self.config.container_config.circular_dependency_handling, CircularDependencyHandling::Error) {
            self.check_circular_dependencies(&descriptor).await?;
        }
        
        let registration = ServiceRegistration {
            descriptor,
            factory: None,
            instance: None,
            registered_at: chrono::Utc::now(),
        };
        
        {
            let mut services = self.services.write().await;
            services.insert(registration.descriptor.service_type.clone(), registration);
        }
        
        Ok(())
    }
    
    /// Register service with factory
    pub async fn register_service_factory(
        &self,
        service_type: String,
        factory: Box<dyn ServiceFactory>,
    ) -> Result<()> {
        debug!("Registering service factory: {}", service_type);
        
        let descriptor = ServiceDescriptor {
            service_type: service_type.clone(),
            implementation_type: factory.get_service_type().to_string(),
            lifetime: self.config.lifetime_config.default_lifetime,
            dependencies: Vec::new(),
            properties: HashMap::new(),
            interceptors: Vec::new(),
            metadata: ServiceMetadata {
                name: service_type.clone(),
                version: "1.0.0".to_string(),
                description: "Service registered via factory".to_string(),
                tags: Vec::new(),
                category: "unknown".to_string(),
            },
        };
        
        let registration = ServiceRegistration {
            descriptor,
            factory: Some(factory),
            instance: None,
            registered_at: chrono::Utc::now(),
        };
        
        {
            let mut services = self.services.write().await;
            services.insert(service_type, registration);
        }
        
        Ok(())
    }
    
    /// Resolve service
    pub async fn resolve_service<T: Service + 'static>(&self) -> Result<Arc<T>> {
        let service_type = std::any::type_name::<T>();
        let service = self.resolve_service_by_type(service_type).await?;
        
        // Downcast to specific type
        service.downcast::<T>()
            .map_err(|_| anyhow::anyhow!("Failed to downcast service to type {}", service_type))
    }
    
    /// Resolve service by type
    pub async fn resolve_service_by_type(&self, service_type: &str) -> Result<Arc<dyn Service>> {
        debug!("Resolving service: {}", service_type);
        
        // Check if service is registered
        let registration = {
            let services = self.services.read().await;
            services.get(service_type)
                .ok_or_else(|| anyhow::anyhow!("Service {} is not registered", service_type))?
                .clone()
        };
        
        // Resolve based on lifetime
        match registration.descriptor.lifetime {
            ServiceLifetime::Singleton => self.resolve_singleton(service_type, &registration).await,
            ServiceLifetime::Transient => self.resolve_transient(service_type, &registration).await,
            ServiceLifetime::Scoped => self.resolve_scoped(service_type, &registration).await,
            ServiceLifetime::Custom(_) => self.resolve_custom(service_type, &registration).await,
        }
    }
    
    /// Resolve singleton service
    async fn resolve_singleton(&self, service_type: &str, registration: &ServiceRegistration) -> Result<Arc<dyn Service>> {
        // Check if instance already exists
        {
            let instances = self.instances.read().await;
            if let Some(instance) = instances.get(service_type) {
                return Ok(instance.clone());
            }
        }
        
        // Create new instance
        let instance = self.create_service_instance(registration).await?;
        
        // Store instance
        {
            let mut instances = self.instances.write().await;
            instances.insert(service_type.to_string(), instance.clone());
        }
        
        Ok(instance)
    }
    
    /// Resolve transient service
    async fn resolve_transient(&self, service_type: &str, registration: &ServiceRegistration) -> Result<Arc<dyn Service>> {
        self.create_service_instance(registration).await
    }
    
    /// Resolve scoped service
    async fn resolve_scoped(&self, service_type: &str, registration: &ServiceRegistration) -> Result<Arc<dyn Service>> {
        // Use the global scope by default until request-scoped context is provided.
        let scope_id = "global".to_string();
        
        // Check if instance exists in scope
        {
            let scopes = self.scopes.read().await;
            if let Some(scope) = scopes.iter().find(|s| s.id == scope_id) {
                if let Some(instance) = scope.instances.get(service_type) {
                    return Ok(instance.clone());
                }
            }
        }
        
        // Create new instance
        let instance = self.create_service_instance(registration).await?;
        
        // Store in scope
        {
            let mut scopes = self.scopes.write().await;
            if let Some(scope) = scopes.iter_mut().find(|s| s.id == scope_id) {
                scope.instances.insert(service_type.to_string(), instance.clone());
            } else {
                // Create new scope
                let mut new_scope = ServiceScope {
                    id: scope_id.clone(),
                    name: "Global Scope".to_string(),
                    instances: HashMap::new(),
                    created_at: chrono::Utc::now(),
                    parent: None,
                };
                new_scope.instances.insert(service_type.to_string(), instance.clone());
                scopes.push(new_scope);
            }
        }
        
        Ok(instance)
    }
    
    /// Resolve custom lifetime service
    async fn resolve_custom(
        &self,
        service_type: &str,
        registration: &ServiceRegistration,
    ) -> Result<Arc<dyn Service>> {
        let custom_name = match &registration.descriptor.lifetime {
            ServiceLifetime::Custom(name) => name.as_str(),
            _ => "unknown",
        };

        Err(anyhow::anyhow!(
            "Custom service lifetime '{}' for '{}' is not configured. Register an explicit factory or use Singleton/Transient/Scoped lifetime.",
            custom_name,
            service_type
        ))
    }
    
    /// Create service instance
    async fn create_service_instance(&self, registration: &ServiceRegistration) -> Result<Arc<dyn Service>> {
        let instance = if let Some(factory) = &registration.factory {
            factory.create_instance(self).await?
        } else {
            // Fallback to registered or built-in typed constructors.
            self.create_instance_by_type(&registration.descriptor.implementation_type).await?
        };
        
        // Initialize service
        let mut service = (*instance).clone();
        service.initialize()?;
        
        // Apply interceptors if enabled
        if self.config.proxy_config.proxy_generation_enabled {
            self.apply_interceptors(instance).await
        } else {
            Ok(instance)
        }
    }
    
    /// Create instance by type using dynamic service factory
    async fn create_instance_by_type(&self, implementation_type: &str) -> Result<Arc<dyn Service>> {
        // Use service factory registry to create instances
        let factory_registry = ServiceFactoryRegistry::instance();
        
        if let Some(factory) = factory_registry.get_factory(implementation_type) {
            factory.create_instance(self).await
        } else {
            // Try to create instance using built-in service constructors
            self.create_builtin_service(implementation_type).await
        }
    }
    
    /// Create built-in service instances
    async fn create_builtin_service(&self, service_type: &str) -> Result<Arc<dyn Service>> {
        match service_type {
            "ValidationService" => Ok(Arc::new(crate::harness::validation::ValidationService::new())),
            "SecurityService" => Ok(Arc::new(crate::harness::advanced_security::SecurityService::new())),
            "CacheService" => Ok(Arc::new(crate::harness::distributed_cache::CacheService::new())),
            "EventService" => Ok(Arc::new(crate::harness::event_system::EventService::new())),
            _ => Err(anyhow::anyhow!("Unknown service type: {}", service_type))
        }
    }
    
    /// Apply interceptors to service
    async fn apply_interceptors(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>> {
        let service_type = service.get_service_type();
        
        // Get interceptors for this service
        let interceptors = {
            let interceptors_map = self.interceptors.read().await;
            interceptors_map.get(service_type).cloned().unwrap_or_default()
        };
        
        // Apply global interceptors
        let mut all_interceptors = self.config.proxy_config.interceptor_config.global_interceptors.clone();
        
        // Add service-specific interceptors
        for interceptor_type in &interceptors {
            all_interceptors.push(*interceptor_type);
        }
        
        // Apply interceptors in order
        let mut current_service = service;
        for interceptor_type in all_interceptors {
            current_service = self.apply_interceptor(current_service, interceptor_type).await?;
        }
        
        Ok(current_service)
    }
    
    /// Apply single interceptor
    async fn apply_interceptor(&self, service: Arc<dyn Service>, interceptor_type: InterceptorType) -> Result<Arc<dyn Service>> {
        match interceptor_type {
            InterceptorType::Logging => {
                let interceptor = LoggingInterceptor::new();
                self.create_proxy_with_interceptor(service, Box::new(interceptor)).await
            }
            InterceptorType::Performance => {
                let interceptor = PerformanceInterceptor::new();
                self.create_proxy_with_interceptor(service, Box::new(interceptor)).await
            }
            InterceptorType::Caching => {
                let interceptor = CachingInterceptor::new();
                self.create_proxy_with_interceptor(service, Box::new(interceptor)).await
            }
            InterceptorType::Security => {
                let interceptor = SecurityInterceptor::new();
                self.create_proxy_with_interceptor(service, Box::new(interceptor)).await
            }
            InterceptorType::Transaction => {
                let interceptor = TransactionInterceptor::new();
                self.create_proxy_with_interceptor(service, Box::new(interceptor)).await
            }
        }
    }
    
    /// Create proxy with interceptor
    async fn create_proxy_with_interceptor(
        &self,
        service: Arc<dyn Service>,
        interceptor: Box<dyn Interceptor>,
    ) -> Result<Arc<dyn Service>> {
        // Dynamic proxy behavior is implemented by ProxyService wrapping
        // the original service with the provided interceptor chain.
        Ok(Arc::new(ProxyService::new(service, interceptor)))
    }
    
    /// Discover services
    async fn discover_services(&self) -> Result<()> {
        info!("Discovering services");
        
        let discovered_services = self.discovery_engine.discover_services().await?;
        
        for descriptor in discovered_services {
            if let Err(e) = self.register_service(descriptor).await {
                warn!("Failed to register discovered service: {}", e);
            }
        }
        
        info!("Service discovery completed");
        Ok(())
    }
    
    /// Validate service descriptor
    fn validate_service_descriptor(&self, descriptor: &ServiceDescriptor) -> Result<()> {
        if descriptor.service_type.is_empty() {
            return Err(anyhow::anyhow!("Service type cannot be empty"));
        }
        
        if descriptor.implementation_type.is_empty() {
            return Err(anyhow::anyhow!("Implementation type cannot be empty"));
        }
        
        Ok(())
    }
    
    /// Check for circular dependencies
    async fn check_circular_dependencies(&self, descriptor: &ServiceDescriptor) -> Result<()> {
        let mut visited = std::collections::HashSet::new();
        let mut stack = Vec::new();
        
        self.check_circular_dependencies_recursive(
            &descriptor.service_type,
            &mut visited,
            &mut stack,
        )?;
        
        Ok(())
    }
    
    /// Recursive circular dependency check
    fn check_circular_dependencies_recursive(
        &self,
        service_type: &str,
        visited: &mut std::collections::HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if stack.contains(&service_type.to_string()) {
            return Err(anyhow::anyhow!("Circular dependency detected: {:?}", stack));
        }
        
        if visited.contains(service_type) {
            return Ok(());
        }
        
        visited.insert(service_type.to_string());
        stack.push(service_type.to_string());
        
        // Check dependencies recursively
        if let Ok(services) = self.services.try_read() {
            if let Some(registration) = services.get(service_type) {
                for dependency in &registration.descriptor.dependencies {
                    if !dependency.lazy {
                        self.check_circular_dependencies_recursive(
                            &dependency.dependency_type,
                            visited,
                            stack,
                        )?;
                    }
                }
            }
        }
        
        stack.pop();
        Ok(())
    }
    
    /// Validate configuration
    async fn validate_configuration(&self) -> Result<()> {
        // Validate service registrations
        let services = self.services.read().await;
        
        for (service_type, registration) in services.iter() {
            // Check that dependencies are registered
            for dependency in &registration.descriptor.dependencies {
                if !services.contains_key(&dependency.dependency_type) && dependency.required {
                    return Err(anyhow::anyhow!(
                        "Service {} depends on unregistered service {}",
                        service_type,
                        dependency.dependency_type
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Create new scope
    pub async fn create_scope(&self, name: String) -> Result<String> {
        let scope_id = format!("scope_{}", chrono::Utc::now().timestamp_nanos());
        
        let scope = ServiceScope {
            id: scope_id.clone(),
            name,
            instances: HashMap::new(),
            created_at: chrono::Utc::now(),
            parent: None,
        };
        
        {
            let mut scopes = self.scopes.write().await;
            scopes.push(scope);
        }
        
        Ok(scope_id)
    }
    
    /// Destroy scope
    pub async fn destroy_scope(&self, scope_id: &str) -> Result<()> {
        let mut scopes = self.scopes.write().await;
        
        if let Some(index) = scopes.iter().position(|s| s.id == scope_id) {
            let scope = scopes.remove(index);
            
            // Cleanup instances in scope
            for (_, instance) in scope.instances {
                let mut service = (*instance).clone();
                if let Err(e) = service.cleanup() {
                    warn!("Error cleaning up service: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Get container statistics
    pub async fn get_statistics(&self) -> ContainerStatistics {
        let services = self.services.read().await;
        let instances = self.instances.read().await;
        let scopes = self.scopes.read().await;
        
        ContainerStatistics {
            registered_services: services.len(),
            active_instances: instances.len(),
            active_scopes: scopes.len(),
            memory_usage: self.estimate_memory_usage().await,
        }
    }
    
    /// Estimate memory usage based on actual container state
    async fn estimate_memory_usage(&self) -> u64 {
        let services = self.services.read().await;
        let instances = self.instances.read().await;
        let scopes = self.scopes.read().await;
        
        // Base memory for container structures
        let mut total_memory = 1024 * 512; // 512KB base
        
        // Memory for service registrations
        total_memory += services.len() * 1024; // 1KB per service registration
        
        // Memory for active instances (estimated)
        total_memory += instances.len() * 4096; // 4KB per instance
        
        // Memory for scopes
        total_memory += scopes.len() * 2048; // 2KB per scope
        
        // Memory for interceptors
        let interceptors = self.interceptors.read().await;
        total_memory += interceptors.len() * 512; // 512B per interceptor
        
        total_memory as u64
    }
}

/// P3-Issue1: Container statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerStatistics {
    /// Number of registered services
    pub registered_services: usize,
    /// Number of active instances
    pub active_instances: usize,
    /// Number of active scopes
    pub active_scopes: usize,
    /// Memory usage in bytes
    pub memory_usage: u64,
}

/// Service factory registry for dynamic service creation
pub struct ServiceFactoryRegistry {
    factories: HashMap<String, Arc<dyn ServiceFactory>>,
}

impl ServiceFactoryRegistry {
    pub fn instance() -> Arc<Self> {
        static INSTANCE: std::sync::OnceLock<Arc<ServiceFactoryRegistry>> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| {
            Arc::new(Self {
                factories: HashMap::new(),
            })
        }).clone()
    }
    
    pub fn register_factory(&mut self, service_type: String, factory: Arc<dyn ServiceFactory>) {
        self.factories.insert(service_type, factory);
    }
    
    pub fn get_factory(&self, service_type: &str) -> Option<&Arc<dyn ServiceFactory>> {
        self.factories.get(service_type)
    }
    
    pub fn list_factories(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}

/// P3-Issue1: Proxy service
pub struct ProxyService {
    target: Arc<dyn Service>,
    interceptor: Box<dyn Interceptor>,
}

impl ProxyService {
    pub fn new(target: Arc<dyn Service>, interceptor: Box<dyn Interceptor>) -> Self {
        Self { target, interceptor }
    }
}

impl Service for ProxyService {
    fn get_service_type(&self) -> &str {
        self.target.get_service_type()
    }
    
    fn get_metadata(&self) -> &ServiceMetadata {
        self.target.get_metadata()
    }
    
    fn initialize(&mut self) -> Result<()> {
        let mut context = InterceptorContext {
            service: self.target.clone(),
            method_name: "initialize".to_string(),
            arguments: Vec::new(),
            result: None,
            metadata: HashMap::new(),
        };
        
        match self.interceptor.intercept(&mut context)? {
            InterceptorResult::Continue => {
                let mut target = (*self.target).clone();
                target.initialize()?;
                Ok(())
            }
            InterceptorResult::Return(_) => Ok(()),
            InterceptorResult::Error(e) => Err(e),
            InterceptorResult::Skip => Ok(()),
        }
    }
    
    fn cleanup(&mut self) -> Result<()> {
        let mut context = InterceptorContext {
            service: self.target.clone(),
            method_name: "cleanup".to_string(),
            arguments: Vec::new(),
            result: None,
            metadata: HashMap::new(),
        };
        
        match self.interceptor.intercept(&mut context)? {
            InterceptorResult::Continue => {
                let mut target = (*self.target).clone();
                target.cleanup()?;
                Ok(())
            }
            InterceptorResult::Return(_) => Ok(()),
            InterceptorResult::Error(e) => Err(e),
            InterceptorResult::Skip => Ok(()),
        }
    }
}

/// P3-Issue1: Logging interceptor
pub struct LoggingInterceptor;

impl LoggingInterceptor {
    pub fn new() -> Self {
        Self
    }
}

impl Interceptor for LoggingInterceptor {
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult> {
        info!("Calling method {} on service {}", 
            context.method_name, 
            context.service.get_service_type());
        
        Ok(InterceptorResult::Continue)
    }
    
    fn get_interceptor_type(&self) -> InterceptorType {
        InterceptorType::Logging
    }
}

/// P3-Issue1: Performance interceptor
pub struct PerformanceInterceptor {
    start_time: std::time::Instant,
}

impl PerformanceInterceptor {
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
        }
    }
}

impl Interceptor for PerformanceInterceptor {
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult> {
        let elapsed = self.start_time.elapsed();
        debug!("Method {} executed in {:?}", context.method_name, elapsed);
        
        Ok(InterceptorResult::Continue)
    }
    
    fn get_interceptor_type(&self) -> InterceptorType {
        InterceptorType::Performance
    }
}

/// P3-Issue1: Caching interceptor
pub struct CachingInterceptor;

impl CachingInterceptor {
    pub fn new() -> Self {
        Self
    }
}

impl Interceptor for CachingInterceptor {
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult> {
        // Simple caching logic - in a real implementation this would be more sophisticated
        if context.method_name == "get_data" {
            // Check cache first
            if let Some(cached_result) = self.check_cache(context) {
                return Ok(InterceptorResult::Return(cached_result));
            }
        }
        
        Ok(InterceptorResult::Continue)
    }
    
    fn get_interceptor_type(&self) -> InterceptorType {
        InterceptorType::Caching
    }
}

impl CachingInterceptor {
    fn check_cache(&self, context: &InterceptorContext) -> Option<serde_json::Value> {
        // Create cache key from service type and method
        let cache_key = format!("{}:{}", context.service.get_service_type(), context.method_name);
        
        // In a real implementation, this would check a distributed cache
        // For now, implement simple in-memory caching
        static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, (serde_json::Value, DateTime<Utc>)>>> = 
            std::sync::OnceLock::new();
        
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
        
        if let Ok(cache_guard) = cache.lock() {
            if let Some((value, timestamp)) = cache_guard.get(&cache_key) {
                // Cache entries valid for 5 minutes
                if Utc::now().signed_duration_since(*timestamp).num_minutes() < 5 {
                    return Some(value.clone());
                }
            }
        }
        
        None
    }
    
    fn store_cache(&self, context: &InterceptorContext, result: &serde_json::Value) {
        let cache_key = format!("{}:{}", context.service.get_service_type(), context.method_name);
        
        static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, (serde_json::Value, DateTime<Utc>)>>> = 
            std::sync::OnceLock::new();
        
        let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
        
        if let Ok(mut cache_guard) = cache.lock() {
            cache_guard.insert(cache_key, (result.clone(), Utc::now()));
        }
    }
}

/// P3-Issue1: Security interceptor
pub struct SecurityInterceptor;

impl SecurityInterceptor {
    pub fn new() -> Self {
        Self
    }
}

impl Interceptor for SecurityInterceptor {
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult> {
        // Simple security check - in a real implementation this would be more sophisticated
        if context.method_name.contains("admin") {
            return Err(anyhow::anyhow!("Access denied to admin method"));
        }
        
        Ok(InterceptorResult::Continue)
    }
    
    fn get_interceptor_type(&self) -> InterceptorType {
        InterceptorType::Security
    }
}

/// P3-Issue1: Transaction interceptor
pub struct TransactionInterceptor;

impl TransactionInterceptor {
    pub fn new() -> Self {
        Self
    }
}

impl Interceptor for TransactionInterceptor {
    fn intercept(&self, context: &mut InterceptorContext) -> Result<InterceptorResult> {
        // Simple transaction logic - in a real implementation this would be more sophisticated
        if context.method_name.starts_with("update_") {
            debug!("Starting transaction for method {}", context.method_name);
            // Begin transaction
        }
        
        Ok(InterceptorResult::Continue)
    }
    
    fn get_interceptor_type(&self) -> InterceptorType {
        InterceptorType::Transaction
    }
}

impl ServiceDiscoveryEngine {
    pub fn new(config: ServiceDiscoveryConfig) -> Self {
        let mut mechanisms: Vec<Box<dyn DiscoveryMechanism>> = Vec::new();
        
        for mechanism_config in &config.mechanisms {
            let mechanism: Box<dyn DiscoveryMechanism> = match mechanism_config.mechanism_type {
                DiscoveryMechanismType::FileSystem => Box::new(FileSystemDiscovery::new()),
                DiscoveryMechanismType::Environment => Box::new(EnvironmentDiscovery::new()),
                DiscoveryMechanismType::ConfigurationFile => Box::new(ConfigurationFileDiscovery::new()),
                DiscoveryMechanismType::Network
                | DiscoveryMechanismType::Database
                | DiscoveryMechanismType::Custom => Box::new(UnsupportedDiscovery::new(
                    mechanism_config.name.clone(),
                    mechanism_config.mechanism_type,
                )),
            };
            mechanisms.push(mechanism);
        }
        
        Self {
            config,
            mechanisms,
        }
    }
    
    pub async fn discover_services(&self) -> Result<Vec<ServiceDescriptor>> {
        let mut all_services = Vec::new();
        
        for mechanism in &self.mechanisms {
            match mechanism.discover_services() {
                Ok(mut services) => {
                    all_services.append(&mut services);
                }
                Err(e) => {
                    warn!("Discovery mechanism {} failed: {}", mechanism.get_name(), e);
                }
            }
        }
        
        Ok(all_services)
    }
}

impl LifetimeManager {
    pub fn new(config: LifetimeManagementConfig) -> Self {
        Self {
            gc_scheduler: Arc::new(RwLock::new(GCScheduler {
                interval: Duration::from_secs(config.gc_interval_sec),
                last_run: chrono::Utc::now(),
                running: false,
            })),
        }
    }
    
    /// Clean up expired cache entries
    async fn cleanup_expired_cache_entries(&self) {
        // This would integrate with the distributed cache system
        debug!("Cleaning up expired cache entries");
    }
    
    /// Clean up unused service instances
    async fn cleanup_unused_instances(&self) {
        // This would check reference counts and clean up unused instances
        debug!("Cleaning up unused service instances");
    }
    
    pub async fn start(&self) -> Result<()> {
        if self.gc_scheduler.read().await.running {
            return Ok(());
        }
        
        info!("Starting lifetime manager");
        
        let gc_scheduler = self.gc_scheduler.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            
            loop {
                interval.tick().await;
                
                let mut scheduler = gc_scheduler.write().await;
                if scheduler.last_run.elapsed() >= scheduler.interval {
                    // Run garbage collection
                    debug!("Running garbage collection");
                    
                    // Clean up expired cache entries
                    self.cleanup_expired_cache_entries().await;
                    
                    // Clean up unused service instances
                    self.cleanup_unused_instances().await;
                    
                    scheduler.last_run = chrono::Utc::now();
                }
            }
        });
        
        {
            let mut scheduler = self.gc_scheduler.write().await;
            scheduler.running = true;
        }
        
        Ok(())
    }
}

impl ProxyFactory {
    pub fn new(config: ProxyConfig) -> Self {
        let mut proxy_generators: HashMap<ProxyType, Box<dyn ProxyGenerator>> = HashMap::new();
        
        for proxy_type in &config.proxy_types {
            let generator: Box<dyn ProxyGenerator> = match proxy_type {
                ProxyType::Virtual => Box::new(VirtualProxyGenerator::new()),
                ProxyType::Protection => Box::new(ProtectionProxyGenerator::new()),
                ProxyType::Remote => Box::new(RemoteProxyGenerator::new()),
                ProxyType::Smart => Box::new(SmartProxyGenerator::new()),
            };
            proxy_generators.insert(*proxy_type, generator);
        }
        
        Self {
            config,
            proxy_generators,
        }
    }
}

// Placeholder implementations for discovery mechanisms and proxy generators

pub struct FileSystemDiscovery;

impl FileSystemDiscovery {
    pub fn new() -> Self {
        Self
    }
}

impl DiscoveryMechanism for FileSystemDiscovery {
    fn discover_services(&self) -> Result<Vec<ServiceDescriptor>> {
        let mut services = Vec::new();
        
        // Scan for service files in the harness directory
        let scan_path = Path::new("src/harness");
        if scan_path.exists() {
            for entry in WalkDir::new(scan_path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().extension().map(|ext| ext == "rs").unwrap_or(false)
                }) {
                
                // Try to extract service information from the file
                if let Some(service) = self.extract_service_from_file(entry.path())? {
                    services.push(service);
                }
            }
        }
        
        Ok(services)
    }
    
    fn get_name(&self) -> &str {
        "filesystem"
    }
}

impl FileSystemDiscovery {
    fn extract_service_from_file(&self, file_path: &Path) -> Result<Option<ServiceDescriptor>> {
        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;
        
        // Look for service implementations
        if content.contains("impl Service for") {
            // Extract service name from file
            let file_stem = file_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            
            let service_type = format!("crate::harness::{}::{}Service", 
                file_stem, file_stem);
            
            Ok(Some(ServiceDescriptor {
                service_type: service_type.clone(),
                implementation_type: service_type,
                lifetime: ServiceLifetime::Transient,
                dependencies: Vec::new(),
                properties: HashMap::new(),
                interceptors: vec![InterceptorType::Logging],
                metadata: ServiceMetadata {
                    name: format!("{} Service", file_stem),
                    version: "1.0.0".to_string(),
                    description: format!("Service discovered from {}", file_path.display()),
                    tags: vec!["discovered".to_string(), "filesystem".to_string()],
                    category: "discovered".to_string(),
                },
            }))
        } else {
            Ok(None)
        }
    }
}

pub struct EnvironmentDiscovery;

impl EnvironmentDiscovery {
    pub fn new() -> Self {
        Self
    }
}

impl DiscoveryMechanism for EnvironmentDiscovery {
    fn discover_services(&self) -> Result<Vec<ServiceDescriptor>> {
        let mut services = Vec::new();
        
        // Look for service configurations in environment variables
        for (key, value) in std::env::vars() {
            if key.starts_with("PROMETHEOS_SERVICE_") {
                if let Some(service_info) = self.parse_service_env_var(&key, &value)? {
                    services.push(service_info);
                }
            }
        }
        
        Ok(services)
    }
    
    fn get_name(&self) -> &str {
        "environment"
    }
}

impl EnvironmentDiscovery {
    fn parse_service_env_var(&self, key: &str, value: &str) -> Result<Option<ServiceDescriptor>> {
        // Parse PROMETHEOS_SERVICE_{TYPE}_CONFIG format
        let parts: Vec<&str> = key.split('_').collect();
        if parts.len() < 4 || parts[3] != "CONFIG" {
            return Ok(None);
        }
        
        let service_type = parts[2];
        
        // Parse JSON configuration
        let config: serde_json::Value = serde_json::from_str(value)
            .with_context(|| format!("Invalid JSON in environment variable {}: {}", key, value))?;
        
        Ok(Some(ServiceDescriptor {
            service_type: service_type.to_string(),
            implementation_type: config.get("implementation")
                .and_then(|v| v.as_str())
                .unwrap_or(service_type)
                .to_string(),
            lifetime: config.get("lifetime")
                .and_then(|v| v.as_str())
                .and_then(|s| match s {
                    "singleton" => Some(ServiceLifetime::Singleton),
                    "scoped" => Some(ServiceLifetime::Scoped),
                    _ => Some(ServiceLifetime::Transient),
                })
                .unwrap_or(ServiceLifetime::Transient),
            dependencies: Vec::new(),
            properties: HashMap::new(),
            interceptors: vec![InterceptorType::Logging],
            metadata: ServiceMetadata {
                name: format!("{} Service", service_type),
                version: config.get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("1.0.0")
                    .to_string(),
                description: config.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Service discovered from environment")
                    .to_string(),
                tags: vec!["discovered".to_string(), "environment".to_string()],
                category: "discovered".to_string(),
            },
        }))
    }
}

pub struct ConfigurationFileDiscovery;

impl ConfigurationFileDiscovery {
    pub fn new() -> Self {
        Self
    }
}

impl DiscoveryMechanism for ConfigurationFileDiscovery {
    fn discover_services(&self) -> Result<Vec<ServiceDescriptor>> {
        let config_path = Path::new("prometheos-services.json");
        
        if !config_path.exists() {
            return Ok(Vec::new());
        }
        
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read service configuration file: {}", config_path.display()))?;
        
        let config: ServiceConfigFile = serde_json::from_str(&content)
            .with_context(|| "Invalid service configuration file format")?;
        
        Ok(config.services)
    }
    
    fn get_name(&self) -> &str {
        "configuration_file"
    }
}

/// Service configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceConfigFile {
    services: Vec<ServiceDescriptor>,
}

pub struct UnsupportedDiscovery {
    name: String,
    mechanism_type: DiscoveryMechanismType,
}

impl UnsupportedDiscovery {
    pub fn new(name: String, mechanism_type: DiscoveryMechanismType) -> Self {
        Self {
            name,
            mechanism_type,
        }
    }
}

impl DiscoveryMechanism for UnsupportedDiscovery {
    fn discover_services(&self) -> Result<Vec<ServiceDescriptor>> {
        Err(anyhow::anyhow!(
            "Discovery mechanism '{}' ({:?}) is configured but not implemented",
            self.name,
            self.mechanism_type
        ))
    }
    
    fn get_name(&self) -> &str {
        &self.name
    }
}

pub struct VirtualProxyGenerator;

impl VirtualProxyGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ProxyGenerator for VirtualProxyGenerator {
    fn generate_proxy(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>> {
        // Create a virtual proxy that defers method calls
        Ok(Arc::new(VirtualProxyService::new(service)))
    }
    
    fn get_proxy_type(&self) -> ProxyType {
        ProxyType::Virtual
    }
}

/// Virtual proxy service that defers method calls
pub struct VirtualProxyService {
    target: Arc<dyn Service>,
    call_count: std::sync::atomic::AtomicU64,
}

impl VirtualProxyService {
    pub fn new(target: Arc<dyn Service>) -> Self {
        Self {
            target,
            call_count: std::sync::atomic::AtomicU64::new(0),
        }
    }
    
    pub fn get_call_count(&self) -> u64 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Service for VirtualProxyService {
    fn get_service_type(&self) -> &str {
        self.target.get_service_type()
    }
    
    fn get_metadata(&self) -> &ServiceMetadata {
        self.target.get_metadata()
    }
    
    fn initialize(&mut self) -> Result<()> {
        debug!("Initializing virtual proxy for {}", self.target.get_service_type());
        self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        
        // Defer to target service
        let mut target = (*self.target).clone();
        target.initialize()
    }
    
    fn cleanup(&mut self) -> Result<()> {
        debug!("Cleaning up virtual proxy for {}", self.target.get_service_type());
        
        // Defer to target service
        let mut target = (*self.target).clone();
        target.cleanup()
    }
}

pub struct ProtectionProxyGenerator;

impl ProtectionProxyGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ProxyGenerator for ProtectionProxyGenerator {
    fn generate_proxy(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>> {
        // Create a protection proxy with access control
        Ok(Arc::new(ProtectionProxyService::new(service)))
    }
    
    fn get_proxy_type(&self) -> ProxyType {
        ProxyType::Protection
    }
}

/// Protection proxy service with access control
pub struct ProtectionProxyService {
    target: Arc<dyn Service>,
    access_policy: AccessPolicy,
}

#[derive(Debug, Clone)]
struct AccessPolicy {
    allow_admin_methods: bool,
    allowed_callers: HashSet<String>,
}

impl ProtectionProxyService {
    pub fn new(target: Arc<dyn Service>) -> Self {
        let mut allowed_callers = HashSet::new();
        allowed_callers.insert("system".to_string());
        Self {
            target,
            access_policy: AccessPolicy {
                allow_admin_methods: false,
                allowed_callers,
            },
        }
    }
    
    pub fn with_policy(target: Arc<dyn Service>, policy: AccessPolicy) -> Self {
        Self { target, access_policy: policy }
    }
    
    fn check_access(&self, method_name: &str) -> Result<()> {
        // Check if method is admin method
        if method_name.contains("admin") && !self.access_policy.allow_admin_methods {
            return Err(anyhow::anyhow!("Access denied: admin methods not allowed"));
        }

        // Enforce caller allowlist for all operations.
        let caller_id = std::env::var("PROMETHEOS_CALLER_ID").unwrap_or_else(|_| "system".to_string());
        if !self.access_policy.allowed_callers.contains(&caller_id) {
            return Err(anyhow::anyhow!(
                "Access denied: caller '{}' is not in allowed_callers policy",
                caller_id
            ));
        }

        Ok(())
    }
}

impl Service for ProtectionProxyService {
    fn get_service_type(&self) -> &str {
        format!("Protected:{}", self.target.get_service_type()).leak()
    }
    
    fn get_metadata(&self) -> &ServiceMetadata {
        self.target.get_metadata()
    }
    
    fn initialize(&mut self) -> Result<()> {
        self.check_access("initialize")?;
        
        debug!("Initializing protection proxy for {}", self.target.get_service_type());
        
        // Defer to target service
        let mut target = (*self.target).clone();
        target.initialize()
    }
    
    fn cleanup(&mut self) -> Result<()> {
        self.check_access("cleanup")?;
        
        debug!("Cleaning up protection proxy for {}", self.target.get_service_type());
        
        // Defer to target service
        let mut target = (*self.target).clone();
        target.cleanup()
    }
}

pub struct RemoteProxyGenerator;

impl RemoteProxyGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ProxyGenerator for RemoteProxyGenerator {
    fn generate_proxy(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>> {
        // Create a remote proxy that forwards calls to a remote service
        Ok(Arc::new(RemoteProxyService::new(service)))
    }
    
    fn get_proxy_type(&self) -> ProxyType {
        ProxyType::Remote
    }
}

/// Remote proxy service that forwards calls to remote endpoints
pub struct RemoteProxyService {
    target: Arc<dyn Service>,
    endpoint: String,
    client: reqwest::Client,
}

impl RemoteProxyService {
    pub fn new(target: Arc<dyn Service>) -> Self {
        Self {
            target,
            endpoint: std::env::var("PROMETHEOS_REMOTE_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            client: reqwest::Client::new(),
        }
    }
    
    pub fn with_endpoint(target: Arc<dyn Service>, endpoint: String) -> Self {
        Self {
            target,
            endpoint,
            client: reqwest::Client::new(),
        }
    }
    
    async fn call_remote(&self, method: &str, args: &[serde_json::Value]) -> Result<serde_json::Value> {
        let request_body = serde_json::json!({
            "service": self.target.get_service_type(),
            "method": method,
            "arguments": args
        });
        
        let response = self.client
            .post(&format!("{}/api/service/call", self.endpoint))
            .json(&request_body)
            .send()
            .await
            .context("Failed to call remote service")?;
        
        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse remote service response")?;
        
        Ok(result)
    }
}

impl Service for RemoteProxyService {
    fn get_service_type(&self) -> &str {
        format!("Remote:{}", self.target.get_service_type()).leak()
    }
    
    fn get_metadata(&self) -> &ServiceMetadata {
        self.target.get_metadata()
    }
    
    fn initialize(&mut self) -> Result<()> {
        debug!("Initializing remote proxy for {}", self.target.get_service_type());

        Err(anyhow::anyhow!(
            "Remote proxy initialization must be performed through async remote call path; sync local fallback is disabled for service '{}'",
            self.target.get_service_type()
        ))
    }
    
    fn cleanup(&mut self) -> Result<()> {
        debug!("Cleaning up remote proxy for {}", self.target.get_service_type());

        Err(anyhow::anyhow!(
            "Remote proxy cleanup must be performed through async remote call path; sync local fallback is disabled for service '{}'",
            self.target.get_service_type()
        ))
    }
}

pub struct SmartProxyGenerator;

impl SmartProxyGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ProxyGenerator for SmartProxyGenerator {
    fn generate_proxy(&self, service: Arc<dyn Service>) -> Result<Arc<dyn Service>> {
        // Create a smart proxy with adaptive behavior
        Ok(Arc::new(SmartProxyService::new(service)))
    }
    
    fn get_proxy_type(&self) -> ProxyType {
        ProxyType::Smart
    }
}

/// Smart proxy service with adaptive behavior and caching
pub struct SmartProxyService {
    target: Arc<dyn Service>,
    cache: Arc<RwLock<HashMap<String, (serde_json::Value, DateTime<Utc>)>>>,
    performance_metrics: Arc<RwLock<PerformanceMetrics>>,
}

#[derive(Debug, Clone, Default)]
struct PerformanceMetrics {
    call_count: u64,
    total_duration: Duration,
    cache_hits: u64,
    cache_misses: u64,
}

impl SmartProxyService {
    pub fn new(target: Arc<dyn Service>) -> Self {
        Self {
            target,
            cache: Arc::new(RwLock::new(HashMap::new())),
            performance_metrics: Arc::new(RwLock::new(PerformanceMetrics::default())),
        }
    }
    
    async fn get_cached_result(&self, cache_key: &str) -> Option<serde_json::Value> {
        let cache = self.cache.read().await;
        if let Some((value, timestamp)) = cache.get(cache_key) {
            // Cache entries valid for 10 minutes
            if Utc::now().signed_duration_since(*timestamp).num_minutes() < 10 {
                // Update cache hit metric
                if let Ok(mut metrics) = self.performance_metrics.try_write() {
                    metrics.cache_hits += 1;
                }
                return Some(value.clone());
            }
        }
        
        // Update cache miss metric
        if let Ok(mut metrics) = self.performance_metrics.try_write() {
            metrics.cache_misses += 1;
        }
        
        None
    }
    
    async fn store_cached_result(&self, cache_key: String, result: serde_json::Value) {
        let mut cache = self.cache.write().await;
        cache.insert(cache_key, (result, Utc::now()));
    }
    
    async fn update_metrics(&self, duration: Duration) {
        if let Ok(mut metrics) = self.performance_metrics.try_write() {
            metrics.call_count += 1;
            metrics.total_duration += duration;
        }
    }
    
    pub async fn get_performance_metrics(&self) -> PerformanceMetrics {
        self.performance_metrics.read().await.clone()
    }
}

impl Service for SmartProxyService {
    fn get_service_type(&self) -> &str {
        format!("Smart:{}", self.target.get_service_type()).leak()
    }
    
    fn get_metadata(&self) -> &ServiceMetadata {
        self.target.get_metadata()
    }
    
    fn initialize(&mut self) -> Result<()> {
        debug!("Initializing smart proxy for {}", self.target.get_service_type());
        
        // Initialize target service
        let mut target = (*self.target).clone();
        target.initialize()
    }
    
    fn cleanup(&mut self) -> Result<()> {
        debug!("Cleaning up smart proxy for {}", self.target.get_service_type());
        
        // Cleanup target service
        let mut target = (*self.target).clone();
        target.cleanup()
    }
}
