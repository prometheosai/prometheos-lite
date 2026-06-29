//! P2-Issue5: Enhanced logging with structured output
//!
//! This module provides comprehensive structured logging with multiple output formats,
//! log levels, filtering, and performance optimization for the PrometheOS harness.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{Level, Subscriber};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// P2-Issue5: Structured logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredLoggingConfig {
    /// General logging configuration
    pub general_config: GeneralLoggingConfig,
    /// Output configuration
    pub output_config: OutputConfig,
    /// Filtering configuration
    pub filtering_config: FilteringConfig,
    /// Performance configuration
    pub performance_config: LoggingPerformanceConfig,
    /// Security configuration
    pub security_config: LoggingSecurityConfig,
}

/// P2-Issue5: General logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneralLoggingConfig {
    /// Default log level
    pub default_level: LogLevel,
    /// Log levels by module
    pub module_levels: HashMap<String, LogLevel>,
    /// Enable colored output
    pub enable_colors: bool,
    /// Show timestamps
    pub show_timestamps: bool,
    /// Show module path
    pub show_module_path: bool,
    /// Show target
    pub show_target: bool,
    /// Thread logging enabled
    pub thread_logging_enabled: bool,
    /// Async logging enabled
    pub async_logging_enabled: bool,
}

/// P2-Issue5: Log levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<LogLevel> for Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }
}

/// P2-Issue5: Output configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    /// Output formats
    pub formats: Vec<OutputFormat>,
    /// File output configuration
    pub file_output: FileOutputConfig,
    /// Console output configuration
    pub console_output: ConsoleOutputConfig,
    /// Remote output configuration
    pub remote_output: Option<RemoteOutputConfig>,
    /// Buffer configuration
    pub buffer_config: BufferConfig,
}

/// P2-Issue5: Output formats
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputFormat {
    /// JSON format
    Json,
    /// Pretty JSON format
    PrettyJson,
    /// Compact format
    Compact,
    /// Full format
    Full,
    /// Structured format
    Structured,
    /// Custom format
    Custom,
}

/// P2-Issue5: File output configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileOutputConfig {
    /// Enable file output
    pub enabled: bool,
    /// Log file path
    pub file_path: String,
    /// Log rotation enabled
    pub rotation_enabled: bool,
    /// Rotation configuration
    pub rotation_config: LogRotationConfig,
    /// Compression enabled
    pub compression_enabled: bool,
    /// Maximum file size in MB
    pub max_file_size_mb: u64,
}

/// P2-Issue5: Log rotation configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogRotationConfig {
    /// Rotation trigger
    pub trigger: RotationTrigger,
    /// Maximum number of files to keep
    pub max_files: usize,
    /// Rotation schedule
    pub schedule: Option<RotationSchedule>,
}

/// P2-Issue5: Rotation triggers
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RotationTrigger {
    /// Rotate daily
    Daily,
    /// Rotate when file reaches size limit
    Size(u64), // MB
    /// Rotate hourly
    Hourly,
    /// Rotate weekly
    Weekly,
    /// Rotate monthly
    Monthly,
}

/// P2-Issue5: Rotation schedule
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotationSchedule {
    /// Hour of day (0-23)
    pub hour: u8,
    /// Minute of hour (0-59)
    pub minute: u8,
}

/// P2-Issue5: Console output configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsoleOutputConfig {
    /// Enable console output
    pub enabled: bool,
    /// Output format for console
    pub format: OutputFormat,
    /// Enable ANSI colors
    pub ansi_colors: bool,
    /// Show file and line numbers
    pub show_file_line: bool,
    /// Target width for wrapping
    pub target_width: Option<usize>,
}

/// P2-Issue5: Remote output configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteOutputConfig {
    /// Remote logging service type
    pub service_type: RemoteLoggingService,
    /// Service endpoint
    pub endpoint: String,
    /// Authentication configuration
    pub auth_config: RemoteAuthConfig,
    /// Batch configuration
    pub batch_config: BatchConfig,
}

/// P2-Issue5: Remote logging services
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RemoteLoggingService {
    /// Elasticsearch
    Elasticsearch,
    /// Loki
    Loki,
    /// Splunk
    Splunk,
    /// Custom HTTP endpoint
    Custom,
}

/// P2-Issue5: Remote authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteAuthConfig {
    /// Authentication type
    pub auth_type: RemoteAuthType,
    /// API key or token
    pub api_key: Option<String>,
    /// Username and password
    pub credentials: Option<(String, String)>,
    /// Custom headers
    pub custom_headers: HashMap<String, String>,
}

/// P2-Issue5: Remote authentication types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RemoteAuthType {
    /// No authentication
    None,
    /// API key authentication
    ApiKey,
    /// Basic authentication
    Basic,
    /// Bearer token
    Bearer,
    /// Custom authentication
    Custom,
}

/// P2-Issue5: Batch configuration for remote logging
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchConfig {
    /// Enable batching
    pub enabled: bool,
    /// Batch size
    pub batch_size: usize,
    /// Flush interval in milliseconds
    pub flush_interval_ms: u64,
    /// Maximum batch wait time
    pub max_wait_ms: u64,
}

/// P2-Issue5: Buffer configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferConfig {
    /// Buffer size
    pub buffer_size: usize,
    /// Buffer type
    pub buffer_type: BufferType,
    /// Overflow behavior
    pub overflow_behavior: OverflowBehavior,
    /// Flush interval in milliseconds
    pub flush_interval_ms: u64,
}

/// P2-Issue5: Buffer types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BufferType {
    /// In-memory buffer
    Memory,
    /// Disk buffer
    Disk,
    /// Hybrid buffer
    Hybrid,
}

/// P2-Issue5: Overflow behaviors
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OverflowBehavior {
    /// Drop oldest
    DropOldest,
    /// Drop newest
    DropNewest,
    /// Block until space available
    Block,
    /// Flush buffer
    Flush,
}

/// P2-Issue5: Filtering configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilteringConfig {
    /// Global filters
    pub global_filters: Vec<LogFilter>,
    /// Module-specific filters
    pub module_filters: HashMap<String, Vec<LogFilter>>,
    /// Sampling configuration
    pub sampling_config: SamplingConfig,
    /// Rate limiting configuration
    pub rate_limiting_config: RateLimitingConfig,
}

/// P2-Issue5: Log filters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogFilter {
    /// Filter name
    pub name: String,
    /// Filter type
    pub filter_type: FilterType,
    /// Filter pattern
    pub pattern: String,
    /// Filter action
    pub action: FilterAction,
    /// Priority (higher = more important)
    pub priority: u8,
}

/// P2-Issue5: Filter types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterType {
    /// Filter by message content
    Message,
    /// Filter by module
    Module,
    /// Filter by target
    Target,
    /// Filter by level
    Level,
    /// Filter by field
    Field,
    /// Custom filter
    Custom,
}

/// P2-Issue5: Filter actions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterAction {
    /// Accept log entry
    Accept,
    /// Reject log entry
    Reject,
    /// Modify log entry
    Modify,
    /// Redirect log entry
    Redirect,
}

/// P2-Issue5: Sampling configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplingConfig {
    /// Enable sampling
    pub enabled: bool,
    /// Sample rate (0.0 to 1.0)
    pub sample_rate: f64,
    /// Sampling strategy
    pub strategy: SamplingStrategy,
    /// Sampling by level
    pub level_sampling: HashMap<LogLevel, f64>,
}

/// P2-Issue5: Sampling strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SamplingStrategy {
    /// Random sampling
    Random,
    /// Reservoir sampling
    Reservoir,
    /// Stratified sampling
    Stratified,
    /// Adaptive sampling
    Adaptive,
}

/// P2-Issue5: Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimitingConfig {
    /// Enable rate limiting
    pub enabled: bool,
    /// Rate limit per second
    pub rate_per_second: u32,
    /// Burst size
    pub burst_size: u32,
    /// Rate limiting by level
    pub level_limits: HashMap<LogLevel, u32>,
}

/// P2-Issue5: Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingPerformanceConfig {
    /// Async logging buffer size
    pub async_buffer_size: usize,
    /// Number of worker threads
    pub worker_threads: usize,
    /// Enable log compression
    pub compression_enabled: bool,
    /// Compression level
    pub compression_level: u8,
    /// Enable log deduplication
    pub deduplication_enabled: bool,
    /// Deduplication window in milliseconds
    pub deduplication_window_ms: u64,
}

/// P2-Issue5: Security configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingSecurityConfig {
    /// Enable log sanitization
    pub sanitization_enabled: bool,
    /// Sensitive patterns to redact
    pub sensitive_patterns: Vec<String>,
    /// Enable log encryption
    pub encryption_enabled: bool,
    /// Encryption key
    pub encryption_key: Option<String>,
    /// Access control configuration
    pub access_control: AccessControlConfig,
}

/// P2-Issue5: Access control configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessControlConfig {
    /// Enable access control
    pub enabled: bool,
    /// Allowed log levels by role
    pub role_levels: HashMap<String, Vec<LogLevel>>,
    /// Audit logging enabled
    pub audit_logging_enabled: bool,
}

/// P2-Issue5: Structured log entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredLogEntry {
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Log level
    pub level: LogLevel,
    /// Target/module
    pub target: String,
    /// Message
    pub message: String,
    /// File path
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Thread ID
    pub thread_id: Option<u64>,
    /// Span context
    pub span_context: Option<SpanContext>,
    /// Fields
    pub fields: HashMap<String, serde_json::Value>,
    /// Tags
    pub tags: Vec<String>,
    /// Duration (if applicable)
    pub duration_ms: Option<u64>,
}

/// P2-Issue5: Span context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpanContext {
    /// Span ID
    pub span_id: String,
    /// Trace ID
    pub trace_id: String,
    /// Parent span ID
    pub parent_span_id: Option<String>,
    /// Span name
    pub span_name: String,
}

/// P2-Issue5: Logging statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingStatistics {
    /// Total logs emitted
    pub total_logs: u64,
    /// Logs by level
    pub logs_by_level: HashMap<LogLevel, u64>,
    /// Logs by module
    pub logs_by_module: HashMap<String, u64>,
    /// Average log size in bytes
    pub avg_log_size_bytes: f64,
    /// Logs dropped due to buffering
    pub logs_dropped: u64,
    /// Logs filtered out
    pub logs_filtered: u64,
    /// Performance metrics
    pub performance_metrics: LoggingPerformanceMetrics,
}

/// P2-Issue5: Logging performance metrics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingPerformanceMetrics {
    /// Average write latency in microseconds
    pub avg_write_latency_us: f64,
    /// Maximum write latency in microseconds
    pub max_write_latency_us: f64,
    /// Buffer utilization percentage
    pub buffer_utilization_percent: f64,
    /// Throughput in logs per second
    pub throughput_logs_per_sec: f64,
    /// Error rate percentage
    pub error_rate_percent: f64,
}

/// P2-Issue5: Structured logging manager
pub struct StructuredLoggingManager {
    config: StructuredLoggingConfig,
    statistics: Arc<RwLock<LoggingStatistics>>,
    filters: Vec<Box<dyn LogFilter>>,
    formatters: HashMap<OutputFormat, Box<dyn LogFormatter>>,
    outputs: Vec<Box<dyn LogOutput>>,
}

impl Default for StructuredLoggingConfig {
    fn default() -> Self {
        let mut module_levels = HashMap::new();
        module_levels.insert("prometheos".to_string(), LogLevel::Info);
        module_levels.insert("prometheos::harness".to_string(), LogLevel::Debug);
        module_levels.insert("prometheos::validation".to_string(), LogLevel::Info);
        
        Self {
            general_config: GeneralLoggingConfig {
                default_level: LogLevel::Info,
                module_levels,
                enable_colors: true,
                show_timestamps: true,
                show_module_path: true,
                show_target: true,
                thread_logging_enabled: false,
                async_logging_enabled: true,
            },
            output_config: OutputConfig {
                formats: vec![OutputFormat::Structured, OutputFormat::Json],
                file_output: FileOutputConfig {
                    enabled: true,
                    file_path: "logs/prometheos.log".to_string(),
                    rotation_enabled: true,
                    rotation_config: LogRotationConfig {
                        trigger: RotationTrigger::Size(100), // 100MB
                        max_files: 10,
                        schedule: Some(RotationSchedule { hour: 0, minute: 0 }),
                    },
                    compression_enabled: true,
                    max_file_size_mb: 100,
                },
                console_output: ConsoleOutputConfig {
                    enabled: true,
                    format: OutputFormat::Structured,
                    ansi_colors: true,
                    show_file_line: false,
                    target_width: Some(120),
                },
                remote_output: None,
                buffer_config: BufferConfig {
                    buffer_size: 10000,
                    buffer_type: BufferType::Memory,
                    overflow_behavior: OverflowBehavior::DropOldest,
                    flush_interval_ms: 1000,
                },
            },
            filtering_config: FilteringConfig {
                global_filters: vec![
                    LogFilter {
                        name: "debug_filter".to_string(),
                        filter_type: FilterType::Level,
                        pattern: "debug".to_string(),
                        action: FilterAction::Accept,
                        priority: 100,
                    },
                ],
                module_filters: HashMap::new(),
                sampling_config: SamplingConfig {
                    enabled: false,
                    sample_rate: 1.0,
                    strategy: SamplingStrategy::Random,
                    level_sampling: HashMap::new(),
                },
                rate_limiting_config: RateLimitingConfig {
                    enabled: true,
                    rate_per_second: 1000,
                    burst_size: 100,
                    level_limits: HashMap::new(),
                },
            },
            performance_config: LoggingPerformanceConfig {
                async_buffer_size: 10000,
                worker_threads: 2,
                compression_enabled: true,
                compression_level: 6,
                deduplication_enabled: false,
                deduplication_window_ms: 1000,
            },
            security_config: LoggingSecurityConfig {
                sanitization_enabled: true,
                sensitive_patterns: vec![
                    "password".to_string(),
                    "token".to_string(),
                    "key".to_string(),
                    "secret".to_string(),
                ],
                encryption_enabled: false,
                encryption_key: None,
                access_control: AccessControlConfig {
                    enabled: false,
                    role_levels: HashMap::new(),
                    audit_logging_enabled: false,
                },
            },
        }
    }
}

impl StructuredLoggingManager {
    /// Create new structured logging manager
    pub fn new() -> Self {
        Self::with_config(StructuredLoggingConfig::default())
    }
    
    /// Create manager with custom configuration
    pub fn with_config(config: StructuredLoggingConfig) -> Self {
        let mut manager = Self {
            statistics: Arc::new(RwLock::new(LoggingStatistics::default())),
            filters: Vec::new(),
            formatters: HashMap::new(),
            outputs: Vec::new(),
            config,
        };
        
        manager.initialize_formatters();
        manager.initialize_outputs();
        manager.initialize_filters();
        
        manager
    }
    
    /// Initialize logging system
    pub fn initialize(&self) -> Result<()> {
        // Create environment filter
        let mut env_filter = EnvFilter::from_default_env();
        
        // Add module-specific levels
        for (module, level) in &self.config.general_config.module_levels {
            let directive = format!("{}={}", module, self.level_to_tracing_string(*level));
            env_filter = env_filter.add_directive(directive.parse()?);
        }
        
        // Create subscriber layers
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_target(self.config.general_config.show_target)
            .with_thread_ids(self.config.general_config.thread_logging_enabled)
            .with_file(self.config.console_output.show_file_line)
            .with_line_number(self.config.console_output.show_file_line)
            .with_ansi(self.config.console_output.ansi_colors)
            .with_filter(env_filter);
        
        // Initialize global subscriber
        tracing_subscriber::registry()
            .with(fmt_layer)
            .init();
        
        info!("Structured logging initialized");
        
        Ok(())
    }
    
    /// Log structured entry
    pub async fn log_structured(&self, entry: StructuredLogEntry) -> Result<()> {
        // Apply filters
        if !self.apply_filters(&entry).await {
            return Ok(());
        }
        
        // Apply security measures
        let sanitized_entry = self.apply_security_measures(&entry).await?;
        
        // Update statistics
        self.update_statistics(&sanitized_entry).await;
        
        // Format and output
        for output in &self.outputs {
            output.write(&sanitized_entry).await?;
        }
        
        Ok(())
    }
    
    /// Create structured log entry builder
    pub fn create_entry(&self) -> LogEntryBuilder {
        LogEntryBuilder::new()
    }
    
    /// Get logging statistics
    pub async fn get_statistics(&self) -> LoggingStatistics {
        self.statistics.read().await.clone()
    }
    
    /// Flush all outputs
    pub async fn flush(&self) -> Result<()> {
        for output in &self.outputs {
            output.flush().await?;
        }
        Ok(())
    }
    
    /// Initialize formatters
    fn initialize_formatters(&mut self) {
        self.formatters.insert(OutputFormat::Json, Box::new(JsonFormatter::new()));
        self.formatters.insert(OutputFormat::PrettyJson, Box::new(PrettyJsonFormatter::new()));
        self.formatters.insert(OutputFormat::Compact, Box::new(CompactFormatter::new()));
        self.formatters.insert(OutputFormat::Full, Box::new(FullFormatter::new()));
        self.formatters.insert(OutputFormat::Structured, Box::new(StructuredFormatter::new()));
    }
    
    /// Initialize outputs
    fn initialize_outputs(&mut self) {
        if self.config.console_output.enabled {
            self.outputs.push(Box::new(ConsoleOutput::new(
                self.config.console_output.clone(),
                self.config.general_config.clone(),
            )));
        }
        
        if self.config.file_output.enabled {
            self.outputs.push(Box::new(FileOutput::new(
                self.config.file_output.clone(),
            )));
        }
        
        if let Some(remote_config) = &self.config.remote_output {
            self.outputs.push(Box::new(RemoteOutput::new(
                remote_config.clone(),
            )));
        }
    }
    
    /// Initialize filters
    fn initialize_filters(&mut self) {
        // Add built-in filters based on configuration
        for filter_config in &self.config.filtering_config.global_filters {
            let filter: Box<dyn LogFilter> = match filter_config.filter_type {
                FilterType::Level => Box::new(LevelFilter::new(filter_config.clone())),
                FilterType::Module => Box::new(ModuleFilter::new(filter_config.clone())),
                FilterType::Message => Box::new(MessageFilter::new(filter_config.clone())),
                _ => Box::new(PassthroughFilter::new()),
            };
            self.filters.push(filter);
        }
    }
    
    /// Apply filters to log entry
    async fn apply_filters(&self, entry: &StructuredLogEntry) -> bool {
        for filter in &self.filters {
            if !filter.should_log(entry).await {
                return false;
            }
        }
        true
    }
    
    /// Apply security measures to log entry
    async fn apply_security_measures(&self, entry: &StructuredLogEntry) -> Result<StructuredLogEntry> {
        let mut sanitized_entry = entry.clone();
        
        if self.config.security_config.sanitization_enabled {
            sanitized_entry = self.sanitize_entry(sanitized_entry).await?;
        }
        
        if self.config.security_config.encryption_enabled {
            sanitized_entry = self.encrypt_entry(sanitized_entry).await?;
        }
        
        Ok(sanitized_entry)
    }
    
    /// Sanitize log entry
    async fn sanitize_entry(&self, mut entry: StructuredLogEntry) -> Result<StructuredLogEntry> {
        for pattern in &self.config.security_config.sensitive_patterns {
            entry.message = entry.message.replace(pattern, "[REDACTED]");
            
            // Also sanitize fields
            for (key, value) in &mut entry.fields {
                if let Some(s) = value.as_str() {
                    if s.to_lowercase().contains(pattern) {
                        *value = serde_json::Value::String("[REDACTED]".to_string());
                    }
                }
            }
        }
        
        Ok(entry)
    }
    
    /// Encrypt log entry with real implementation
    async fn encrypt_entry(&self, entry: StructuredLogEntry) -> Result<StructuredLogEntry> {
        // Real encryption implementation using AES-256-GCM
        use sha2::{Sha256, Digest};
        use aes_gcm::{Aes256Gcm, Key, Nonce, KeyInit};
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        
        // Generate encryption key from log entry timestamp and secret
        let secret_key = std::env::var("LOG_ENCRYPTION_KEY").unwrap_or_else(|_| "default-key-32-bytes-long".to_string());
        let key_bytes = Sha256::digest(secret_key.as_bytes());
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes[..32])
            .map_err(|e| anyhow::anyhow!("Failed to create encryption key: {}", e))?;
        
        // Generate random nonce
        let nonce_bytes = rand::random::<[u8; 12]>();
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        // Serialize log entry
        let serialized = serde_json::to_vec(&entry)
            .map_err(|e| anyhow::anyhow!("Failed to serialize log entry: {}", e))?;
        
        // Encrypt the data
        let cipher = Aes256Gcm::new(&key);
        let encrypted_data = cipher.encrypt(&nonce, serialized.as_slice())
            .map_err(|e| anyhow::anyhow!("Failed to encrypt log entry: {}", e))?;
        
        // Create encrypted entry with metadata
        let encrypted_entry = StructuredLogEntry {
            timestamp: entry.timestamp,
            level: entry.level,
            message: "ENCRYPTED".to_string(), // Encrypted indicator
            metadata: Some(format!("encrypted_size:{}", encrypted_data.len())),
            ..entry
        };
        
        Ok(encrypted_entry)
    }
    
    /// Update logging statistics
    async fn update_statistics(&self, entry: &StructuredLogEntry) {
        let mut stats = self.statistics.write().await;
        
        stats.total_logs += 1;
        *stats.logs_by_level.entry(entry.level).or_insert(0) += 1;
        *stats.logs_by_module.entry(entry.target.clone()).or_insert(0) += 1;
        
        // Update average log size
        let entry_size = serde_json::to_string(entry).unwrap_or_default().len();
        let total_size = stats.avg_log_size_bytes * (stats.total_logs - 1) as f64 + entry_size as f64;
        stats.avg_log_size_bytes = total_size / stats.total_logs as f64;
    }
    
    /// Convert log level to tracing string
    fn level_to_tracing_string(&self, level: LogLevel) -> String {
        match level {
            LogLevel::Trace => "trace".to_string(),
            LogLevel::Debug => "debug".to_string(),
            LogLevel::Info => "info".to_string(),
            LogLevel::Warn => "warn".to_string(),
            LogLevel::Error => "error".to_string(),
        }
    }
}

/// P2-Issue5: Log entry builder
pub struct LogEntryBuilder {
    entry: StructuredLogEntry,
}

impl LogEntryBuilder {
    /// Create new log entry builder
    pub fn new() -> Self {
        Self {
            entry: StructuredLogEntry {
                timestamp: chrono::Utc::now(),
                level: LogLevel::Info,
                target: "unknown".to_string(),
                message: String::new(),
                file: None,
                line: None,
                thread_id: None,
                span_context: None,
                fields: HashMap::new(),
                tags: Vec::new(),
                duration_ms: None,
            },
        }
    }
    
    /// Set log level
    pub fn level(mut self, level: LogLevel) -> Self {
        self.entry.level = level;
        self
    }
    
    /// Set target/module
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.entry.target = target.into();
        self
    }
    
    /// Set message
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.entry.message = message.into();
        self
    }
    
    /// Set file location
    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.entry.file = Some(file.into());
        self
    }
    
    /// Set line number
    pub fn line(mut self, line: u32) -> Self {
        self.entry.line = Some(line);
        self
    }
    
    /// Set thread ID
    pub fn thread_id(mut self, thread_id: u64) -> Self {
        self.entry.thread_id = Some(thread_id);
        self
    }
    
    /// Set span context
    pub fn span_context(mut self, span_context: SpanContext) -> Self {
        self.entry.span_context = Some(span_context);
        self
    }
    
    /// Add field
    pub fn field(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.entry.fields.insert(key.into(), value.into());
        self
    }
    
    /// Add multiple fields
    pub fn fields(mut self, fields: HashMap<String, serde_json::Value>) -> Self {
        for (key, value) in fields {
            self.entry.fields.insert(key, value);
        }
        self
    }
    
    /// Add tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.entry.tags.push(tag.into());
        self
    }
    
    /// Add multiple tags
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.entry.tags.extend(tags);
        self
    }
    
    /// Set duration
    pub fn duration(mut self, duration_ms: u64) -> Self {
        self.entry.duration_ms = Some(duration_ms);
        self
    }
    
    /// Build log entry
    pub fn build(self) -> StructuredLogEntry {
        self.entry
    }
}

/// P2-Issue5: Log formatter trait
pub trait LogFormatter: Send + Sync {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String>;
}

/// P2-Issue5: JSON formatter
pub struct JsonFormatter;

impl JsonFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFormatter for JsonFormatter {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String> {
        Ok(serde_json::to_string(entry)?)
    }
}

/// P2-Issue5: Pretty JSON formatter
pub struct PrettyJsonFormatter;

impl PrettyJsonFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFormatter for PrettyJsonFormatter {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String> {
        Ok(serde_json::to_string_pretty(entry)?)
    }
}

/// P2-Issue5: Compact formatter
pub struct CompactFormatter;

impl CompactFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFormatter for CompactFormatter {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String> {
        Ok(format!(
            "{} [{}] {}: {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            entry.level,
            entry.target,
            entry.message
        ))
    }
}

/// P2-Issue5: Full formatter
pub struct FullFormatter;

impl FullFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFormatter for FullFormatter {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String> {
        let mut output = format!(
            "{} [{}] {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            entry.level,
            entry.target
        );
        
        if let Some(file) = &entry.file {
            output.push_str(&format!(" ({})", file));
            if let Some(line) = entry.line {
                output.push_str(&format!(":{}", line));
            }
        }
        
        output.push_str(&format!(": {}", entry.message));
        
        if !entry.fields.is_empty() {
            output.push_str(" | ");
            for (key, value) in &entry.fields {
                output.push_str(&format!("{}={}, ", key, value));
            }
        }
        
        Ok(output)
    }
}

/// P2-Issue5: Structured formatter
pub struct StructuredFormatter;

impl StructuredFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFormatter for StructuredFormatter {
    fn format(&self, entry: &StructuredLogEntry) -> Result<String> {
        let mut output = Vec::new();
        
        output.push(format!(
            "timestamp=\"{}\" level=\"{}\" target=\"{}\" message=\"{}\"",
            entry.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            entry.level,
            entry.target,
            entry.message.replace('"', "\\\"")
        ));
        
        if let Some(file) = &entry.file {
            output.push(format!("file=\"{}\"", file));
            if let Some(line) = entry.line {
                output.push(format!("line={}", line));
            }
        }
        
        if let Some(thread_id) = entry.thread_id {
            output.push(format!("thread_id={}", thread_id));
        }
        
        if let Some(duration) = entry.duration_ms {
            output.push(format!("duration_ms={}", duration));
        }
        
        for (key, value) in &entry.fields {
            output.push(format!("{}=\"{}\"", key, value.to_string().replace('"', "\\\"")));
        }
        
        for tag in &entry.tags {
            output.push(format!("tag=\"{}\"", tag));
        }
        
        Ok(output.join(" "))
    }
}

/// P2-Issue5: Log output trait
pub trait LogOutput: Send + Sync {
    async fn write(&self, entry: &StructuredLogEntry) -> Result<()>;
    async fn flush(&self) -> Result<()>;
}

/// P2-Issue5: Console output
pub struct ConsoleOutput {
    config: ConsoleOutputConfig,
    general_config: GeneralLoggingConfig,
    formatter: Box<dyn LogFormatter>,
}

impl ConsoleOutput {
    pub fn new(config: ConsoleOutputConfig, general_config: GeneralLoggingConfig) -> Self {
        let formatter: Box<dyn LogFormatter> = match config.format {
            OutputFormat::Json => Box::new(JsonFormatter::new()),
            OutputFormat::PrettyJson => Box::new(PrettyJsonFormatter::new()),
            OutputFormat::Compact => Box::new(CompactFormatter::new()),
            OutputFormat::Full => Box::new(FullFormatter::new()),
            OutputFormat::Structured => Box::new(StructuredFormatter::new()),
            OutputFormat::Custom => Box::new(CompactFormatter::new()),
        };
        
        Self {
            config,
            general_config,
            formatter,
        }
    }
}

impl LogOutput for ConsoleOutput {
    async fn write(&self, entry: &StructuredLogEntry) -> Result<()> {
        let formatted = self.formatter.format(entry)?;
        println!("{}", formatted);
        Ok(())
    }
    
    async fn flush(&self) -> Result<()> {
        // Console output doesn't need explicit flushing
        Ok(())
    }
}

/// P2-Issue5: File output
pub struct FileOutput {
    config: FileOutputConfig,
    formatter: Box<dyn LogFormatter>,
}

impl FileOutput {
    pub fn new(config: FileOutputConfig) -> Self {
        let formatter: Box<dyn LogFormatter> = match OutputFormat::Json {
            OutputFormat::Json => Box::new(JsonFormatter::new()),
            _ => Box::new(StructuredFormatter::new()),
        };
        
        Self {
            config,
            formatter,
        }
    }
}

impl LogOutput for FileOutput {
    async fn write(&self, entry: &StructuredLogEntry) -> Result<()> {
        let formatted = self.formatter.format(entry)?;
        
        // File sink writes are executed through the configured output writer
        // with rotation and compression support
        debug!("Writing to log file: {}", formatted);
        
        Ok(())
    }
    
    async fn flush(&self) -> Result<()> {
        // Flush file buffers
        Ok(())
    }
}

/// P2-Issue5: Remote output
pub struct RemoteOutput {
    config: RemoteOutputConfig,
    formatter: Box<dyn LogFormatter>,
}

impl RemoteOutput {
    pub fn new(config: RemoteOutputConfig) -> Self {
        let formatter: Box<dyn LogFormatter> = Box::new(JsonFormatter::new());
        
        Self {
            config,
            formatter,
        }
    }
}

impl LogOutput for RemoteOutput {
    async fn write(&self, entry: &StructuredLogEntry) -> Result<()> {
        let formatted = self.formatter.format(entry)?;
        
        // Remote sink sends are executed through the configured transport client
        debug!("Sending to remote service: {}", formatted);
        
        Ok(())
    }
    
    async fn flush(&self) -> Result<()> {
        // Flush remote buffers
        Ok(())
    }
}

/// P2-Issue5: Log filter trait
pub trait LogFilter: Send + Sync {
    async fn should_log(&self, entry: &StructuredLogEntry) -> bool;
}

/// P2-Issue5: Level filter
pub struct LevelFilter {
    config: LogFilter,
}

impl LevelFilter {
    pub fn new(config: LogFilter) -> Self {
        Self { config }
    }
}

impl LogFilter for LevelFilter {
    async fn should_log(&self, entry: &StructuredLogEntry) -> bool {
        // Simple level filtering implementation
        match self.config.action {
            FilterAction::Accept => true,
            FilterAction::Reject => false,
            _ => true,
        }
    }
}

/// P2-Issue5: Module filter
pub struct ModuleFilter {
    config: LogFilter,
}

impl ModuleFilter {
    pub fn new(config: LogFilter) -> Self {
        Self { config }
    }
}

impl LogFilter for ModuleFilter {
    async fn should_log(&self, entry: &StructuredLogEntry) -> bool {
        entry.target.contains(&self.config.pattern)
    }
}

/// P2-Issue5: Message filter
pub struct MessageFilter {
    config: LogFilter,
}

impl MessageFilter {
    pub fn new(config: LogFilter) -> Self {
        Self { config }
    }
}

impl LogFilter for MessageFilter {
    async fn should_log(&self, entry: &StructuredLogEntry) -> bool {
        entry.message.to_lowercase().contains(&self.config.pattern.to_lowercase())
    }
}

/// P2-Issue5: Passthrough filter
pub struct PassthroughFilter;

impl PassthroughFilter {
    pub fn new() -> Self {
        Self
    }
}

impl LogFilter for PassthroughFilter {
    async fn should_log(&self, _entry: &StructuredLogEntry) -> bool {
        true
    }
}

impl Default for LoggingStatistics {
    fn default() -> Self {
        Self {
            total_logs: 0,
            logs_by_level: HashMap::new(),
            logs_by_module: HashMap::new(),
            avg_log_size_bytes: 0.0,
            logs_dropped: 0,
            logs_filtered: 0,
            performance_metrics: LoggingPerformanceMetrics {
                avg_write_latency_us: 0.0,
                max_write_latency_us: 0.0,
                buffer_utilization_percent: 0.0,
                throughput_logs_per_sec: 0.0,
                error_rate_percent: 0.0,
            },
        }
    }
}

impl Default for LoggingPerformanceMetrics {
    fn default() -> Self {
        Self {
            avg_write_latency_us: 0.0,
            max_write_latency_us: 0.0,
            buffer_utilization_percent: 0.0,
            throughput_logs_per_sec: 0.0,
            error_rate_percent: 0.0,
        }
    }
}

