//! P2-Issue1: Enhanced error recovery with automatic retry mechanisms
//!
//! This module provides comprehensive error recovery strategies including
//! automatic retry with exponential backoff, circuit breaker patterns,
//! and intelligent error classification for optimal recovery.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// P2-Issue1: Enhanced error recovery configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRecoveryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial backoff duration
    pub initial_backoff_ms: u64,
    /// Backoff multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Maximum backoff duration
    pub max_backoff_ms: u64,
    /// Jitter factor for backoff randomization
    pub jitter_factor: f64,
    /// Circuit breaker configuration
    pub circuit_breaker: CircuitBreakerConfig,
    /// Retry policies by error type
    pub retry_policies: HashMap<String, RetryPolicy>,
    /// Recovery strategies
    pub recovery_strategies: Vec<RecoveryStrategy>,
}

/// P2-Issue1: Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitBreakerConfig {
    /// Failure threshold to open circuit
    pub failure_threshold: u32,
    /// Recovery timeout in milliseconds
    pub recovery_timeout_ms: u64,
    /// Half-open max calls
    pub half_open_max_calls: u32,
    /// Success threshold to close circuit
    pub success_threshold: u32,
}

/// P2-Issue1: Retry policy for specific error types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    /// Whether this error type is retryable
    pub retryable: bool,
    /// Maximum retries for this error type
    pub max_retries: Option<u32>,
    /// Backoff strategy for this error type
    pub backoff_strategy: BackoffStrategy,
    /// Recovery actions for this error type
    pub recovery_actions: Vec<RecoveryAction>,
}

/// P2-Issue1: Backoff strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Exponential backoff
    Exponential,
    /// Linear backoff
    Linear,
    /// Custom backoff function
    Custom,
}

/// P2-Issue1: Recovery action types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Clear caches
    ClearCache,
    /// Reset connections
    ResetConnections,
    /// Restart services
    RestartServices,
    /// Reinitialize components
    Reinitialize,
    /// Fallback to alternative
    Fallback,
    /// Escalate to human
    Escalate,
}

/// P2-Issue1: Recovery strategy configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecoveryStrategy {
    /// Strategy name
    pub name: String,
    /// Error patterns this strategy applies to
    pub error_patterns: Vec<String>,
    /// Recovery actions in order
    pub actions: Vec<RecoveryAction>,
    /// Maximum attempts for this strategy
    pub max_attempts: u32,
    /// Success criteria
    pub success_criteria: SuccessCriteria,
}

/// P2-Issue1: Success criteria for recovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuccessCriteria {
    /// Minimum success rate
    pub min_success_rate: f64,
    /// Maximum error rate
    pub max_error_rate: f64,
    /// Minimum consecutive successes
    pub min_consecutive_successes: u32,
}

/// P2-Issue1: Circuit breaker state
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// P2-Issue1: Circuit breaker implementation
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    last_failure_time: Option<Instant>,
    config: CircuitBreakerConfig,
}

/// P2-Issue1: Retry attempt information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryAttempt {
    /// Attempt number
    pub attempt_number: u32,
    /// Timestamp of attempt
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Error that occurred
    pub error: String,
    /// Backoff duration used
    pub backoff_duration_ms: u64,
    /// Recovery actions taken
    pub recovery_actions: Vec<RecoveryAction>,
}

/// P2-Issue1: Error classification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorClassification {
    /// Error type
    pub error_type: String,
    /// Error category
    pub category: ErrorCategory,
    /// Severity level
    pub severity: ErrorSeverity,
    /// Whether error is transient
    pub transient: bool,
    /// Whether error is retryable
    pub retryable: bool,
    /// Suggested recovery actions
    pub suggested_actions: Vec<RecoveryAction>,
}

/// P2-Issue1: Error categories
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCategory {
    Network,
    FileSystem,
    Process,
    Memory,
    Configuration,
    Validation,
    Timeout,
    Authentication,
    Permission,
    Resource,
    Unknown,
}

/// P2-Issue1: Error severity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ErrorSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// P2-Issue1: Error recovery result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRecoveryResult {
    /// Whether recovery was successful
    pub success: bool,
    /// Total number of attempts
    pub total_attempts: u32,
    /// Retry attempts
    pub retry_attempts: Vec<RetryAttempt>,
    /// Recovery actions taken
    pub recovery_actions: Vec<RecoveryAction>,
    /// Final error if recovery failed
    pub final_error: Option<String>,
    /// Total time spent on recovery
    pub total_duration_ms: u64,
    /// Circuit breaker state changes
    pub circuit_breaker_events: Vec<CircuitBreakerEvent>,
}

/// P2-Issue1: Circuit breaker event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitBreakerEvent {
    /// Event type
    pub event_type: CircuitBreakerEventType,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Previous state
    pub previous_state: CircuitState,
    /// New state
    pub new_state: CircuitState,
    /// Reason for state change
    pub reason: String,
}

/// P2-Issue1: Circuit breaker event types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CircuitBreakerEventType {
    Opened,
    Closed,
    HalfOpened,
    Tripped,
    Reset,
}

/// P2-Issue1: Error recovery engine
pub struct ErrorRecoveryEngine {
    config: ErrorRecoveryConfig,
    circuit_breakers: HashMap<String, CircuitBreaker>,
    error_classifier: ErrorClassifier,
}

impl Default for ErrorRecoveryConfig {
    fn default() -> Self {
        let mut retry_policies = HashMap::new();
        
        // Network errors - retryable with exponential backoff
        retry_policies.insert("network".to_string(), RetryPolicy {
            retryable: true,
            max_retries: Some(5),
            backoff_strategy: BackoffStrategy::Exponential,
            recovery_actions: vec![RecoveryAction::ResetConnections],
        });
        
        // File system errors - retryable with linear backoff
        retry_policies.insert("filesystem".to_string(), RetryPolicy {
            retryable: true,
            max_retries: Some(3),
            backoff_strategy: BackoffStrategy::Linear,
            recovery_actions: vec![RecoveryAction::ClearCache],
        });
        
        // Process errors - not retryable
        retry_policies.insert("process".to_string(), RetryPolicy {
            retryable: false,
            max_retries: None,
            backoff_strategy: BackoffStrategy::Fixed,
            recovery_actions: vec![RecoveryAction::RestartServices],
        });
        
        // Timeout errors - retryable with exponential backoff
        retry_policies.insert("timeout".to_string(), RetryPolicy {
            retryable: true,
            max_retries: Some(3),
            backoff_strategy: BackoffStrategy::Exponential,
            recovery_actions: vec![RecoveryAction::Fallback],
        });
        
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            backoff_multiplier: 2.0,
            max_backoff_ms: 30000,
            jitter_factor: 0.1,
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 5,
                recovery_timeout_ms: 60000,
                half_open_max_calls: 3,
                success_threshold: 3,
            },
            retry_policies,
            recovery_strategies: vec![
                RecoveryStrategy {
                    name: "network_recovery".to_string(),
                    error_patterns: vec!["connection".to_string(), "timeout".to_string()],
                    actions: vec![RecoveryAction::ResetConnections, RecoveryAction::Fallback],
                    max_attempts: 3,
                    success_criteria: SuccessCriteria {
                        min_success_rate: 0.8,
                        max_error_rate: 0.2,
                        min_consecutive_successes: 2,
                    },
                },
                RecoveryStrategy {
                    name: "resource_recovery".to_string(),
                    error_patterns: vec!["memory".to_string(), "disk".to_string()],
                    actions: vec![RecoveryAction::ClearCache, RecoveryAction::Reinitialize],
                    max_attempts: 2,
                    success_criteria: SuccessCriteria {
                        min_success_rate: 0.9,
                        max_error_rate: 0.1,
                        min_consecutive_successes: 1,
                    },
                },
            ],
        }
    }
}

impl ErrorRecoveryEngine {
    /// Create new error recovery engine
    pub fn new() -> Self {
        Self::with_config(ErrorRecoveryConfig::default())
    }
    
    /// Create error recovery engine with custom config
    pub fn with_config(config: ErrorRecoveryConfig) -> Self {
        Self {
            circuit_breakers: HashMap::new(),
            error_classifier: ErrorClassifier::new(),
            config,
        }
    }
    
    /// Execute operation with automatic retry and recovery
    pub async fn execute_with_recovery<F, T, E>(
        &mut self,
        operation_name: &str,
        operation: F,
    ) -> Result<T>
    where
        F: Fn() -> Result<T, E>,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        let start_time = Instant::now();
        let mut retry_attempts = Vec::new();
        let mut recovery_actions = Vec::new();
        let mut circuit_breaker_events = Vec::new();
        
        // Get or create circuit breaker for this operation
        let circuit_breaker = self.circuit_breakers
            .entry(operation_name.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.config.circuit_breaker.clone()));
        
        // Check circuit breaker state
        if !circuit_breaker.can_execute() {
            return Err(anyhow::anyhow!("Circuit breaker is open for operation: {}", operation_name));
        }
        
        let mut attempt = 0;
        let max_retries = self.config.max_retries;
        
        loop {
            attempt += 1;
            
            // Execute operation
            match operation() {
                Ok(result) => {
                    // Record success
                    circuit_breaker.record_success();
                    
                    let total_duration = start_time.elapsed().as_millis() as u64;
                    
                    info!("Operation {} succeeded after {} attempts", operation_name, attempt);
                    
                    return Ok(result);
                }
                Err(error) => {
                    let error_string = error.to_string();
                    
                    // Classify error
                    let classification = self.error_classifier.classify_error(&error_string);
                    
                    // Record failure
                    circuit_breaker.record_failure();
                    
                    // Check if we should retry
                    let should_retry = self.should_retry(&classification, attempt, max_retries);
                    
                    if !should_retry {
                        let total_duration = start_time.elapsed().as_millis() as u64;
                        
                        error!("Operation {} failed after {} attempts: {}", operation_name, attempt, error_string);
                        
                        return Err(anyhow::anyhow!("Operation failed: {}", error_string));
                    }
                    
                    // Calculate backoff duration
                    let backoff_duration = self.calculate_backoff_duration(attempt, &classification);
                    
                    // Apply recovery actions
                    let applied_actions = self.apply_recovery_actions(&classification).await?;
                    recovery_actions.extend(applied_actions);
                    
                    // Record retry attempt
                    retry_attempts.push(RetryAttempt {
                        attempt_number: attempt,
                        timestamp: chrono::Utc::now(),
                        error: error_string.clone(),
                        backoff_duration_ms: backoff_duration.as_millis() as u64,
                        recovery_actions: recovery_actions.clone(),
                    });
                    
                    warn!(
                        "Operation {} attempt {} failed: {}, retrying in {}ms",
                        operation_name, attempt, error_string, backoff_duration.as_millis()
                    );
                    
                    // Wait before retry
                    sleep(backoff_duration).await;
                }
            }
        }
    }
    
    /// Execute operation with circuit breaker only (no retry)
    pub async fn execute_with_circuit_breaker<F, T, E>(
        &mut self,
        operation_name: &str,
        operation: F,
    ) -> Result<T>
    where
        F: Fn() -> Result<T, E>,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        // Get or create circuit breaker
        let circuit_breaker = self.circuit_breakers
            .entry(operation_name.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.config.circuit_breaker.clone()));
        
        // Check circuit breaker state
        if !circuit_breaker.can_execute() {
            return Err(anyhow::anyhow!("Circuit breaker is open for operation: {}", operation_name));
        }
        
        // Execute operation
        match operation() {
            Ok(result) => {
                circuit_breaker.record_success();
                Ok(result)
            }
            Err(error) => {
                circuit_breaker.record_failure();
                Err(anyhow::anyhow!("Operation failed: {}", error))
            }
        }
    }
    
    /// Determine if operation should be retried
    fn should_retry(
        &self,
        classification: &ErrorClassification,
        attempt: u32,
        max_retries: u32,
    ) -> bool {
        if attempt > max_retries {
            return false;
        }
        
        if !classification.retryable {
            return false;
        }
        
        // Check specific retry policy for this error type
        if let Some(policy) = self.config.retry_policies.get(&classification.error_type) {
            if !policy.retryable {
                return false;
            }
            
            if let Some(policy_max_retries) = policy.max_retries {
                if attempt > policy_max_retries {
                    return false;
                }
            }
        }
        
        true
    }
    
    /// Calculate backoff duration
    fn calculate_backoff_duration(&self, attempt: u32, classification: &ErrorClassification) -> Duration {
        let base_duration = self.config.initial_backoff_ms;
        
        // Get backoff strategy for this error type
        let strategy = if let Some(policy) = self.config.retry_policies.get(&classification.error_type) {
            policy.backoff_strategy
        } else {
            BackoffStrategy::Exponential
        };
        
        let duration_ms = match strategy {
            BackoffStrategy::Fixed => base_duration,
            BackoffStrategy::Exponential => {
                let exponential = base_duration as f64 * self.config.backoff_multiplier.powi(attempt as i32 - 1);
                exponential.min(self.config.max_backoff_ms as f64) as u64
            }
            BackoffStrategy::Linear => {
                let linear = base_duration * attempt;
                linear.min(self.config.max_backoff_ms)
            }
            BackoffStrategy::Custom => {
                // Custom backoff logic would go here
                base_duration
            }
        };
        
        // Add jitter
        let jitter = (duration_ms as f64 * self.config.jitter_factor * rand::random::<f64>()) as u64;
        let final_duration = duration_ms + jitter;
        
        Duration::from_millis(final_duration)
    }
    
    /// Apply recovery actions for error classification
    async fn apply_recovery_actions(&self, classification: &ErrorClassification) -> Result<Vec<RecoveryAction>> {
        let mut applied_actions = Vec::new();
        
        // Get recovery strategy for this error
        for strategy in &self.config.recovery_strategies {
            if strategy.error_patterns.iter().any(|pattern| {
                classification.error_type.contains(pattern) || 
                classification.error_type.to_lowercase().contains(&pattern.to_lowercase())
            }) {
                for action in &strategy.actions {
                    if self.apply_recovery_action(action).await? {
                        applied_actions.push(*action);
                    }
                }
                break;
            }
        }
        
        // Apply suggested actions from classification
        for action in &classification.suggested_actions {
            if self.apply_recovery_action(action).await? {
                applied_actions.push(*action);
            }
        }
        
        Ok(applied_actions)
    }
    
    /// Apply individual recovery action
    async fn apply_recovery_action(&self, action: &RecoveryAction) -> Result<bool> {
        match action {
            RecoveryAction::ClearCache => {
                debug!("Clearing caches as recovery action");
                // Implementation would clear relevant caches
                Ok(true)
            }
            RecoveryAction::ResetConnections => {
                debug!("Resetting connections as recovery action");
                // Implementation would reset network connections
                Ok(true)
            }
            RecoveryAction::RestartServices => {
                warn!("Restarting services as recovery action");
                // Implementation would restart relevant services
                Ok(true)
            }
            RecoveryAction::Reinitialize => {
                info!("Reinitializing components as recovery action");
                // Implementation would reinitialize components
                Ok(true)
            }
            RecoveryAction::Fallback => {
                info!("Using fallback as recovery action");
                // Implementation would switch to fallback mechanism
                Ok(true)
            }
            RecoveryAction::Escalate => {
                error!("Escalating to human intervention as recovery action");
                // Implementation would notify human operators
                Ok(true)
            }
        }
    }
    
    /// Get circuit breaker status for all operations
    pub fn get_circuit_breaker_status(&self) -> HashMap<String, CircuitBreakerStatus> {
        self.circuit_breakers
            .iter()
            .map(|(name, breaker)| {
                (name.clone(), breaker.get_status())
            })
            .collect()
    }
    
    /// Reset all circuit breakers
    pub fn reset_all_circuit_breakers(&mut self) {
        for breaker in self.circuit_breakers.values_mut() {
            breaker.reset();
        }
    }
    
    /// Reset specific circuit breaker
    pub fn reset_circuit_breaker(&mut self, operation_name: &str) {
        if let Some(breaker) = self.circuit_breakers.get_mut(operation_name) {
            breaker.reset();
        }
    }
}

/// P2-Issue1: Circuit breaker status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitBreakerStatus {
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_failure_time: Option<chrono::DateTime<chrono::Utc>>,
    pub can_execute: bool,
}

impl CircuitBreaker {
    /// Create new circuit breaker
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
            config,
        }
    }
    
    /// Check if operation can be executed
    pub fn can_execute(&self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if let Some(last_failure) = self.last_failure_time {
                    let elapsed = last_failure.elapsed();
                    elapsed.as_millis() >= self.config.recovery_timeout_ms as u128
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => self.success_count < self.config.half_open_max_calls,
        }
    }
    
    /// Record successful operation
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.config.success_threshold {
                    self.state = CircuitState::Closed;
                    self.failure_count = 0;
                    self.success_count = 0;
                }
            }
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::Open => {
                // Should not happen, but handle gracefully
                self.state = CircuitState::Closed;
                self.failure_count = 0;
            }
        }
    }
    
    /// Record failed operation
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(chrono::Utc::now());
        
        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open;
                self.success_count = 0;
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }
    
    /// Get current status
    pub fn get_status(&self) -> CircuitBreakerStatus {
        CircuitBreakerStatus {
            state: self.state,
            failure_count: self.failure_count,
            success_count: self.success_count,
            last_failure_time: self.last_failure_time,
            can_execute: self.can_execute(),
        }
    }
    
    /// Reset circuit breaker
    pub fn reset(&mut self) {
        self.state = CircuitState::Closed;
        self.failure_count = 0;
        self.success_count = 0;
        self.last_failure_time = None;
    }
}

/// P2-Issue1: Error classifier
pub struct ErrorClassifier {
    patterns: Vec<ErrorPattern>,
}

/// P2-Issue1: Error pattern for classification
#[derive(Debug, Clone)]
struct ErrorPattern {
    pattern: String,
    error_type: String,
    category: ErrorCategory,
    severity: ErrorSeverity,
    transient: bool,
    retryable: bool,
    suggested_actions: Vec<RecoveryAction>,
}

impl ErrorClassifier {
    /// Create new error classifier
    pub fn new() -> Self {
        let patterns = vec![
            ErrorPattern {
                pattern: "connection".to_string(),
                error_type: "network".to_string(),
                category: ErrorCategory::Network,
                severity: ErrorSeverity::Medium,
                transient: true,
                retryable: true,
                suggested_actions: vec![RecoveryAction::ResetConnections],
            },
            ErrorPattern {
                pattern: "timeout".to_string(),
                error_type: "timeout".to_string(),
                category: ErrorCategory::Timeout,
                severity: ErrorSeverity::Medium,
                transient: true,
                retryable: true,
                suggested_actions: vec![RecoveryAction::Fallback],
            },
            ErrorPattern {
                pattern: "permission".to_string(),
                error_type: "permission".to_string(),
                category: ErrorCategory::Permission,
                severity: ErrorSeverity::High,
                transient: false,
                retryable: false,
                suggested_actions: vec![RecoveryAction::Escalate],
            },
            ErrorPattern {
                pattern: "memory".to_string(),
                error_type: "memory".to_string(),
                category: ErrorCategory::Memory,
                severity: ErrorSeverity::High,
                transient: true,
                retryable: true,
                suggested_actions: vec![RecoveryAction::ClearCache, RecoveryAction::Reinitialize],
            },
            ErrorPattern {
                pattern: "disk".to_string(),
                error_type: "disk".to_string(),
                category: ErrorCategory::Resource,
                severity: ErrorSeverity::High,
                transient: false,
                retryable: false,
                suggested_actions: vec![RecoveryAction::ClearCache],
            },
            ErrorPattern {
                pattern: "file".to_string(),
                error_type: "filesystem".to_string(),
                category: ErrorCategory::FileSystem,
                severity: ErrorSeverity::Medium,
                transient: true,
                retryable: true,
                suggested_actions: vec![RecoveryAction::ClearCache],
            },
            ErrorPattern {
                pattern: "process".to_string(),
                error_type: "process".to_string(),
                category: ErrorCategory::Process,
                severity: ErrorSeverity::High,
                transient: false,
                retryable: false,
                suggested_actions: vec![RecoveryAction::RestartServices],
            },
        ];
        
        Self { patterns }
    }
    
    /// Classify error based on message
    pub fn classify_error(&self, error_message: &str) -> ErrorClassification {
        let lower_message = error_message.to_lowercase();
        
        // Find matching pattern
        for pattern in &self.patterns {
            if lower_message.contains(&pattern.pattern.to_lowercase()) {
                return ErrorClassification {
                    error_type: pattern.error_type.clone(),
                    category: pattern.category,
                    severity: pattern.severity,
                    transient: pattern.transient,
                    retryable: pattern.retryable,
                    suggested_actions: pattern.suggested_actions.clone(),
                };
            }
        }
        
        // Default classification
        ErrorClassification {
            error_type: "unknown".to_string(),
            category: ErrorCategory::Unknown,
            severity: ErrorSeverity::Medium,
            transient: true,
            retryable: false,
            suggested_actions: vec![RecoveryAction::Escalate],
        }
    }
}

/// P2-Issue1: Retry wrapper for common operations
pub struct RetryWrapper {
    engine: ErrorRecoveryEngine,
}

impl RetryWrapper {
    /// Create new retry wrapper
    pub fn new() -> Self {
        Self {
            engine: ErrorRecoveryEngine::new(),
        }
    }
    
    /// Create retry wrapper with custom config
    pub fn with_config(config: ErrorRecoveryConfig) -> Self {
        Self {
            engine: ErrorRecoveryEngine::with_config(config),
        }
    }
    
    /// Execute async operation with retry
    pub async fn execute_async<F, T, E>(
        &mut self,
        operation_name: &str,
        operation: F,
    ) -> Result<T>
    where
        F: Fn() -> Result<T, E>,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        self.engine.execute_with_recovery(operation_name, operation).await
    }
    
    /// Get engine status
    pub fn get_status(&self) -> HashMap<String, CircuitBreakerStatus> {
        self.engine.get_circuit_breaker_status()
    }
    
    /// Reset engine
    pub fn reset(&mut self) {
        self.engine.reset_all_circuit_breakers();
    }
}
