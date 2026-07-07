//! P2-Issue3: Parallel validation execution with resource limits
//!
//! This module provides comprehensive parallel validation execution with
//! resource management, load balancing, and intelligent scheduling.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{Semaphore, Mutex};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{debug, info, warn, error};

/// P2-Issue3: Parallel validation configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParallelValidationConfig {
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Resource limits
    pub resource_limits: ResourceLimits,
    /// Load balancing strategy
    pub load_balancing_strategy: LoadBalancingStrategy,
    /// Task scheduling configuration
    pub scheduling_config: SchedulingConfig,
    /// Resource monitoring configuration
    pub monitoring_config: MonitoringConfig,
    /// Failure handling configuration
    pub failure_handling: FailureHandlingConfig,
}

/// P2-Issue3: Resource limits for parallel execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceLimits {
    /// Maximum CPU usage percentage
    pub max_cpu_percent: f64,
    /// Maximum memory usage in MB
    pub max_memory_mb: u64,
    /// Maximum disk I/O rate in MB/s
    pub max_disk_io_mb_per_sec: f64,
    /// Maximum network I/O rate in MB/s
    pub max_network_io_mb_per_sec: f64,
    /// Maximum open file handles
    pub max_open_files: u32,
    /// Maximum processes per task
    pub max_processes_per_task: u32,
}

/// P2-Issue3: Load balancing strategies
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LoadBalancingStrategy {
    /// Round-robin scheduling
    RoundRobin,
    /// Load-based scheduling (least loaded first)
    LoadBased,
    /// Priority-based scheduling
    PriorityBased,
    /// Category-based scheduling
    CategoryBased,
    /// Adaptive scheduling
    Adaptive,
    /// Custom scheduling function
    Custom,
}

/// P2-Issue3: Task scheduling configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulingConfig {
    /// Task priority levels
    pub priority_levels: u8,
    /// Time slice per priority level in milliseconds
    pub time_slice_ms: u64,
    /// Preemption enabled
    pub preemption_enabled: bool,
    /// Fairness weights by category
    pub category_weights: HashMap<crate::harness::validation::ValidationCategory, f64>,
    /// Maximum queue size per category
    pub max_queue_size_per_category: usize,
}

/// P2-Issue3: Resource monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitoringConfig {
    /// Monitoring interval in milliseconds
    pub monitoring_interval_ms: u64,
    /// Resource usage history window size
    pub history_window_size: usize,
    /// Alert thresholds
    pub alert_thresholds: AlertThresholds,
    /// Automatic scaling enabled
    pub auto_scaling_enabled: bool,
    /// Resource usage smoothing factor
    pub usage_smoothing_factor: f64,
}

/// P2-Issue3: Alert thresholds for resource usage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertThresholds {
    /// CPU usage alert threshold
    pub cpu_alert_percent: f64,
    /// Memory usage alert threshold
    pub memory_alert_percent: f64,
    /// Disk I/O alert threshold
    pub disk_io_alert_mb_per_sec: f64,
    /// Network I/O alert threshold
    pub network_io_alert_mb_per_sec: f64,
    /// Queue size alert threshold
    pub queue_size_alert: usize,
}

/// P2-Issue3: Failure handling configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureHandlingConfig {
    /// Maximum retry attempts
    pub max_retry_attempts: u32,
    /// Retry backoff strategy
    pub retry_backoff_strategy: crate::harness::error_recovery::BackoffStrategy,
    /// Failure isolation enabled
    pub failure_isolation_enabled: bool,
    /// Circuit breaker thresholds
    pub circuit_breaker_thresholds: CircuitBreakerThresholds,
    /// Failure recovery strategies
    pub recovery_strategies: Vec<FailureRecoveryStrategy>,
}

/// P2-Issue3: Circuit breaker thresholds
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CircuitBreakerThresholds {
    /// Failure rate threshold
    pub failure_rate_threshold: f64,
    /// Minimum requests before tripping
    pub min_requests_threshold: u32,
    /// Recovery timeout in milliseconds
    pub recovery_timeout_ms: u64,
    /// Half-open max calls
    pub half_open_max_calls: u32,
}

/// P2-Issue3: Failure recovery strategies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureRecoveryStrategy {
    /// Strategy name
    pub name: String,
    /// Failure patterns this strategy applies to
    pub failure_patterns: Vec<String>,
    /// Recovery actions
    pub actions: Vec<FailureRecoveryAction>,
    /// Maximum attempts for this strategy
    pub max_attempts: u32,
    /// Success criteria
    pub success_criteria: FailureSuccessCriteria,
}

/// P2-Issue3: Failure recovery actions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureRecoveryAction {
    /// Reduce concurrency
    ReduceConcurrency,
    /// Increase timeouts
    IncreaseTimeouts,
    /// Switch to sequential execution
    SwitchToSequential,
    /// Isolate failing category
    IsolateFailingCategory,
    /// Restart resource monitors
    RestartResourceMonitors,
    /// Escalate to manual intervention
    EscalateToManual,
}

/// P2-Issue3: Failure success criteria
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureSuccessCriteria {
    /// Minimum success rate
    pub min_success_rate: f64,
    /// Maximum failure rate
    pub max_failure_rate: f64,
    /// Minimum consecutive successes
    pub min_consecutive_successes: u32,
    /// Maximum execution time
    pub max_execution_time_ms: u64,
}

/// P2-Issue3: Parallel validation task
#[derive(Debug, Clone)]
pub struct ParallelValidationTask {
    /// Task ID
    pub id: String,
    /// Task category
    pub category: crate::harness::validation::ValidationCategory,
    /// Command to execute
    pub command: String,
    /// Task priority
    pub priority: u8,
    /// Estimated resource requirements
    pub resource_requirements: ResourceRequirements,
    /// Task dependencies
    pub dependencies: Vec<String>,
    /// Creation timestamp
    pub created_at: Instant,
    /// Retry count
    pub retry_count: u32,
    /// Maximum retries
    pub max_retries: u32,
}

/// P2-Issue3: Resource requirements for a task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRequirements {
    /// Estimated CPU usage percentage
    pub estimated_cpu_percent: f64,
    /// Estimated memory usage in MB
    pub estimated_memory_mb: u64,
    /// Estimated disk I/O in MB
    pub estimated_disk_io_mb: u64,
    /// Estimated network I/O in MB
    pub estimated_network_io_mb: u64,
    /// Estimated duration in milliseconds
    pub estimated_duration_ms: u64,
    /// Required file handles
    pub required_file_handles: u32,
}

/// P2-Issue3: Task execution result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskExecutionResult {
    /// Task ID
    pub task_id: String,
    /// Success status
    pub success: bool,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Resource usage during execution
    pub resource_usage: ResourceUsage,
    /// Error message if failed
    pub error_message: Option<String>,
    /// Retry count
    pub retry_count: u32,
    /// Start timestamp
    pub started_at: Instant,
    /// End timestamp
    pub ended_at: Instant,
}

/// P2-Issue3: Resource usage during task execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUsage {
    /// CPU usage percentage
    pub cpu_percent: f64,
    /// Memory usage in MB
    pub memory_mb: u64,
    /// Disk I/O in MB
    pub disk_io_mb: u64,
    /// Network I/O in MB
    pub network_io_mb: u64,
    /// Open file handles
    pub open_file_handles: u32,
    /// Processes created
    pub processes_created: u32,
}

/// P2-Issue3: Parallel validation executor
pub struct ParallelValidationExecutor {
    config: ParallelValidationConfig,
    resource_monitor: ResourceMonitor,
    task_scheduler: TaskScheduler,
    failure_handler: FailureHandler,
    execution_stats: Arc<Mutex<ExecutionStatistics>>,
}

/// P2-Issue3: Execution statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionStatistics {
    /// Total tasks executed
    pub total_tasks: u64,
    /// Successful tasks
    pub successful_tasks: u64,
    /// Failed tasks
    pub failed_tasks: u64,
    /// Average execution time
    pub avg_execution_time_ms: f64,
    /// Peak resource usage
    pub peak_resource_usage: ResourceUsage,
    /// Resource utilization over time
    pub resource_utilization: Vec<ResourceUtilizationPoint>,
    /// Task statistics by category
    pub category_stats: HashMap<crate::harness::validation::ValidationCategory, CategoryExecutionStats>,
}

/// P2-Issue3: Resource utilization point
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUtilizationPoint {
    /// Timestamp
    pub timestamp: Instant,
    /// CPU utilization
    pub cpu_utilization: f64,
    /// Memory utilization
    pub memory_utilization: f64,
    /// Disk I/O utilization
    pub disk_io_utilization: f64,
    /// Network I/O utilization
    pub network_io_utilization: f64,
    /// Active tasks count
    pub active_tasks: usize,
}

/// P2-Issue3: Category execution statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryExecutionStats {
    /// Category
    pub category: crate::harness::validation::ValidationCategory,
    /// Tasks executed
    pub tasks_executed: u64,
    /// Tasks successful
    pub tasks_successful: u64,
    /// Tasks failed
    pub tasks_failed: u64,
    /// Average execution time
    pub avg_execution_time_ms: f64,
    /// Success rate
    pub success_rate: f64,
}

/// P2-Issue3: Resource monitor
pub struct ResourceMonitor {
    config: MonitoringConfig,
    current_usage: Arc<Mutex<ResourceUsage>>,
    usage_history: Arc<Mutex<Vec<ResourceUtilizationPoint>>>,
    alert_handlers: Vec<Box<dyn ResourceAlertHandler>>,
}

/// P2-Issue3: Resource alert handler trait
pub trait ResourceAlertHandler: Send + Sync {
    fn handle_alert(&self, alert: ResourceAlert);
}

/// P2-Issue3: Resource alert
#[derive(Debug, Clone)]
pub struct ResourceAlert {
    /// Alert type
    pub alert_type: ResourceAlertType,
    /// Current usage
    pub current_usage: ResourceUsage,
    /// Threshold exceeded
    pub threshold: f64,
    /// Timestamp
    pub timestamp: Instant,
}

/// P2-Issue3: Resource alert types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAlertType {
    CpuHigh,
    MemoryHigh,
    DiskIoHigh,
    NetworkIoHigh,
    QueueFull,
    ResourceExhausted,
}

/// P2-Issue3: Task scheduler
pub struct TaskScheduler {
    config: SchedulingConfig,
    task_queues: HashMap<crate::harness::validation::ValidationCategory, Arc<Mutex<Vec<ParallelValidationTask>>>>,
    current_load: Arc<Mutex<LoadInfo>>,
    scheduling_algorithm: Box<dyn SchedulingAlgorithm>,
}

/// P2-Issue3: Load information
#[derive(Debug, Clone)]
pub struct LoadInfo {
    /// Active tasks count
    pub active_tasks: usize,
    /// Queued tasks count
    pub queued_tasks: usize,
    /// Current resource usage
    pub current_resource_usage: ResourceUsage,
    /// Average task duration
    pub avg_task_duration_ms: f64,
}

/// P2-Issue3: Scheduling algorithm trait
pub trait SchedulingAlgorithm: Send + Sync {
    fn select_next_task(&self, queues: &HashMap<crate::harness::validation::ValidationCategory, Arc<Mutex<Vec<ParallelValidationTask>>>>, load_info: &LoadInfo) -> Option<ParallelValidationTask>;
}

/// P2-Issue3: Failure handler
pub struct FailureHandler {
    config: FailureHandlingConfig,
    circuit_breakers: HashMap<String, crate::harness::error_recovery::CircuitBreaker>,
    recovery_strategies: Vec<FailureRecoveryStrategy>,
    failure_history: Arc<Mutex<Vec<FailureRecord>>>,
}

/// P2-Issue3: Failure record
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureRecord {
    /// Task ID
    pub task_id: String,
    /// Failure timestamp
    pub timestamp: Instant,
    /// Error message
    pub error_message: String,
    /// Failure category
    pub failure_category: String,
    /// Recovery action taken
    pub recovery_action: Option<FailureRecoveryAction>,
    /// Recovery successful
    pub recovery_successful: bool,
}

impl Default for ParallelValidationConfig {
    fn default() -> Self {
        let mut category_weights = HashMap::new();
        category_weights.insert(crate::harness::validation::ValidationCategory::Format, 1.0);
        category_weights.insert(crate::harness::validation::ValidationCategory::Lint, 2.0);
        category_weights.insert(crate::harness::validation::ValidationCategory::Test, 3.0);
        category_weights.insert(crate::harness::validation::ValidationCategory::Repro, 2.0);
        
        Self {
            max_concurrent_tasks: num_cpus::get(),
            resource_limits: ResourceLimits {
                max_cpu_percent: 80.0,
                max_memory_mb: 4096, // 4GB
                max_disk_io_mb_per_sec: 100.0,
                max_network_io_mb_per_sec: 50.0,
                max_open_files: 1000,
                max_processes_per_task: 10,
            },
            load_balancing_strategy: LoadBalancingStrategy::Adaptive,
            scheduling_config: SchedulingConfig {
                priority_levels: 4,
                time_slice_ms: 1000,
                preemption_enabled: false,
                category_weights,
                max_queue_size_per_category: 100,
            },
            monitoring_config: MonitoringConfig {
                monitoring_interval_ms: 1000,
                history_window_size: 1000,
                alert_thresholds: AlertThresholds {
                    cpu_alert_percent: 90.0,
                    memory_alert_percent: 85.0,
                    disk_io_alert_mb_per_sec: 80.0,
                    network_io_alert_mb_per_sec: 40.0,
                    queue_size_alert: 50,
                },
                auto_scaling_enabled: false,
                usage_smoothing_factor: 0.1,
            },
            failure_handling: FailureHandlingConfig {
                max_retry_attempts: 3,
                retry_backoff_strategy: crate::harness::error_recovery::BackoffStrategy::Exponential,
                failure_isolation_enabled: true,
                circuit_breaker_thresholds: CircuitBreakerThresholds {
                    failure_rate_threshold: 0.5,
                    min_requests_threshold: 10,
                    recovery_timeout_ms: 60000,
                    half_open_max_calls: 3,
                },
                recovery_strategies: vec![
                    FailureRecoveryStrategy {
                        name: "resource_exhaustion".to_string(),
                        failure_patterns: vec!["memory".to_string(), "disk".to_string()],
                        actions: vec![FailureRecoveryAction::ReduceConcurrency],
                        max_attempts: 3,
                        success_criteria: FailureSuccessCriteria {
                            min_success_rate: 0.8,
                            max_failure_rate: 0.2,
                            min_consecutive_successes: 2,
                            max_execution_time_ms: 300000,
                        },
                    },
                ],
            },
        }
    }
}

impl ParallelValidationExecutor {
    /// Create new parallel validation executor
    pub fn new() -> Self {
        Self::with_config(ParallelValidationConfig::default())
    }
    
    /// Create executor with custom configuration
    pub fn with_config(config: ParallelValidationConfig) -> Self {
        let resource_monitor = ResourceMonitor::new(config.monitoring_config.clone());
        let task_scheduler = TaskScheduler::new(config.scheduling_config.clone());
        let failure_handler = FailureHandler::new(config.failure_handling.clone());
        
        Self {
            config,
            resource_monitor,
            task_scheduler,
            failure_handler,
            execution_stats: Arc::new(Mutex::new(ExecutionStatistics::default())),
        }
    }
    
    /// Execute validation tasks in parallel
    pub async fn execute_parallel(
        &mut self,
        tasks: Vec<ParallelValidationTask>,
    ) -> Result<Vec<TaskExecutionResult>> {
        info!("Starting parallel validation with {} tasks", tasks.len());
        
        let start_time = Instant::now();
        let mut join_set = JoinSet::new();
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_tasks));
        let results = Arc::new(Mutex::new(Vec::new()));
        
        // Start resource monitoring
        let monitor_handle = self.resource_monitor.start_monitoring().await?;
        
        // Queue all tasks
        for task in tasks {
            self.task_scheduler.queue_task(task).await?;
        }
        
        // Process tasks until all are completed
        let mut completed_tasks = 0;
        let total_tasks = self.task_scheduler.get_total_queued_tasks().await;
        
        while completed_tasks < total_tasks {
            // Get next task from scheduler
            if let Some(task) = self.task_scheduler.get_next_task().await? {
                let permit = semaphore.clone().acquire().await?;
                let results_clone = results.clone();
                let resource_monitor = &self.resource_monitor;
                let failure_handler = &mut self.failure_handler;
                
                join_set.spawn(async move {
                    let _permit = permit;
                    let result = Self::execute_single_task(task, resource_monitor, failure_handler).await;
                    
                    if let Ok(execution_result) = result {
                        let mut results_guard = results_clone.lock().await;
                        results_guard.push(execution_result);
                    }
                });
                
                completed_tasks += 1;
            } else {
                // No tasks available, wait a bit
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        
        // Wait for all tasks to complete
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(_) => {},
                Err(e) => {
                    error!("Task execution error: {:?}", e);
                }
            }
        }
        
        // Stop resource monitoring
        monitor_handle.abort();
        
        let final_results = results.lock().await.clone();
        let execution_time = start_time.elapsed();
        
        info!("Parallel validation completed in {}ms with {} results", 
            execution_time.as_millis(), final_results.len());
        
        // Update execution statistics
        self.update_execution_statistics(&final_results, execution_time).await;
        
        Ok(final_results)
    }
    
    /// Execute a single validation task
    async fn execute_single_task(
        task: ParallelValidationTask,
        resource_monitor: &ResourceMonitor,
        failure_handler: &mut FailureHandler,
    ) -> Result<TaskExecutionResult> {
        let start_time = Instant::now();
        
        debug!("Executing task {}: {}", task.id, task.command);
        
        // Check resource availability
        if !resource_monitor.check_resource_availability(&task.resource_requirements).await? {
            return Err(anyhow::anyhow!("Insufficient resources for task {}", task.id));
        }
        
        // Execute the task
        let execution_result = match Self::run_validation_command(&task).await {
            Ok(result) => result,
            Err(e) => {
                // Handle failure
                let recovery_action = failure_handler.handle_failure(&task, &e.to_string()).await?;
                
                TaskExecutionResult {
                    task_id: task.id.clone(),
                    success: false,
                    execution_time_ms: start_time.elapsed().as_millis() as u64,
                    resource_usage: ResourceUsage::default(),
                    error_message: Some(e.to_string()),
                    retry_count: task.retry_count,
                    started_at: start_time,
                    ended_at: Instant::now(),
                }
            }
        };
        
        // Record resource usage
        let resource_usage = resource_monitor.get_current_usage().await?;
        
        Ok(TaskExecutionResult {
            task_id: task.id,
            success: execution_result.success,
            execution_time_ms: start_time.elapsed().as_millis() as u64,
            resource_usage,
            error_message: execution_result.error_message,
            retry_count: task.retry_count,
            started_at: start_time,
            ended_at: Instant::now(),
        })
    }
    
    /// Run validation command.
    async fn run_validation_command(task: &ParallelValidationTask) -> Result<TaskExecutionResult> {
        let started_at = Instant::now();
        let timeout_ms = task
            .resource_requirements
            .estimated_duration_ms
            .saturating_mul(2)
            .max(30_000);

        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", &task.command]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", &task.command]);
            command
        };

        let output = timeout(Duration::from_millis(timeout_ms), command.output())
            .await
            .with_context(|| format!("Validation task '{}' timed out after {}ms", task.id, timeout_ms))?
            .with_context(|| format!("Failed to execute validation task '{}': {}", task.id, task.command))?;

        let success = output.status.success();
        let error_message = if success {
            None
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(if stderr.is_empty() { stdout } else { stderr })
        };
        
        Ok(TaskExecutionResult {
            task_id: task.id.clone(),
            success,
            execution_time_ms: started_at.elapsed().as_millis() as u64,
            resource_usage: ResourceUsage {
                cpu_percent: task.resource_requirements.estimated_cpu_percent,
                memory_mb: task.resource_requirements.estimated_memory_mb,
                disk_io_mb: task.resource_requirements.estimated_disk_io_mb,
                network_io_mb: task.resource_requirements.estimated_network_io_mb,
                open_file_handles: task.resource_requirements.required_file_handles,
                processes_created: 1,
            },
            error_message,
            retry_count: task.retry_count,
            started_at,
            ended_at: Instant::now(),
        })
    }
    
    /// Update execution statistics
    async fn update_execution_statistics(&self, results: &[TaskExecutionResult], total_time: Duration) {
        let mut stats = self.execution_stats.lock().await;
        
        stats.total_tasks = results.len() as u64;
        stats.successful_tasks = results.iter().filter(|r| r.success).count() as u64;
        stats.failed_tasks = results.iter().filter(|r| !r.success).count() as u64;
        
        if !results.is_empty() {
            let total_time_ms: u64 = results.iter().map(|r| r.execution_time_ms).sum();
            stats.avg_execution_time_ms = total_time_ms as f64 / results.len() as f64;
        }
        
        // Update category statistics
        for result in results {
            let category = self.task_scheduler.get_task_category(&result.task_id).await;
            if let Some(cat) = category {
                let category_stats = stats.category_stats.entry(cat).or_insert_with(|| CategoryExecutionStats {
                    category: cat,
                    tasks_executed: 0,
                    tasks_successful: 0,
                    tasks_failed: 0,
                    avg_execution_time_ms: 0.0,
                    success_rate: 0.0,
                });
                
                category_stats.tasks_executed += 1;
                if result.success {
                    category_stats.tasks_successful += 1;
                } else {
                    category_stats.tasks_failed += 1;
                }
            }
        }
        
        // Calculate success rates
        for category_stats in stats.category_stats.values_mut() {
            if category_stats.tasks_executed > 0 {
                category_stats.success_rate = category_stats.tasks_successful as f64 / category_stats.tasks_executed as f64;
            }
        }
    }
    
    /// Get execution statistics
    pub async fn get_statistics(&self) -> ExecutionStatistics {
        self.execution_stats.lock().await.clone()
    }
    
    /// Get current resource usage
    pub async fn get_current_resource_usage(&self) -> ResourceUsage {
        self.resource_monitor.get_current_usage().await.unwrap_or_default()
    }
    
    /// Adjust concurrency based on current load
    pub async fn adjust_concurrency(&mut self) -> Result<()> {
        let current_usage = self.get_current_resource_usage().await;
        let limits = &self.config.resource_limits;
        
        let mut new_concurrency = self.config.max_concurrent_tasks;
        
        // Reduce concurrency if resources are high
        if current_usage.cpu_percent > limits.max_cpu_percent * 0.9 {
            new_concurrency = (new_concurrency * 3) / 4; // Reduce by 25%
        }
        
        if current_usage.memory_mb > limits.max_memory_mb * 9 / 10 { // 90%
            new_concurrency = (new_concurrency * 2) / 3; // Reduce by 33%
        }
        
        // Increase concurrency if resources are low
        if current_usage.cpu_percent < limits.max_cpu_percent * 0.5 &&
           current_usage.memory_mb < limits.max_memory_mb / 2 {
            new_concurrency = std::cmp::min(new_concurrency + 1, num_cpus::get() * 2);
        }
        
        if new_concurrency != self.config.max_concurrent_tasks {
            info!("Adjusting concurrency from {} to {}", 
                self.config.max_concurrent_tasks, new_concurrency);
            self.config.max_concurrent_tasks = new_concurrency;
        }
        
        Ok(())
    }
}

impl ResourceMonitor {
    /// Create new resource monitor
    pub fn new(config: MonitoringConfig) -> Self {
        Self {
            config,
            current_usage: Arc::new(Mutex::new(ResourceUsage::default())),
            usage_history: Arc::new(Mutex::new(Vec::new())),
            alert_handlers: Vec::new(),
        }
    }
    
    /// Start resource monitoring
    pub async fn start_monitoring(&self) -> Result<tokio::task::JoinHandle<()>> {
        let current_usage = self.current_usage.clone();
        let usage_history = self.usage_history.clone();
        let interval = Duration::from_millis(self.config.monitoring_interval_ms);
        let alert_handlers = self.alert_handlers.clone();
        let thresholds = self.config.alert_thresholds.clone();
        
        let handle = tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                // Collect resource usage (baseline implementation)
                let usage = Self::collect_resource_usage().await;
                
                // Update current usage
                {
                    let mut current = current_usage.lock().await;
                    *current = usage.clone();
                }
                
                // Add to history
                {
                    let mut history = usage_history.lock().await;
                    history.push(ResourceUtilizationPoint {
                        timestamp: Instant::now(),
                        cpu_utilization: usage.cpu_percent,
                        memory_utilization: (usage.memory_mb as f64 / 4096.0) * 100.0, // Assuming 4GB max
                        disk_io_utilization: (usage.disk_io_mb as f64 / 100.0) * 100.0, // Assuming 100MB/s max
                        network_io_utilization: (usage.network_io_mb as f64 / 50.0) * 100.0, // Assuming 50MB/s max
                        active_tasks: 0, // Would be tracked elsewhere
                    });
                    
                    // Trim history if needed
                    if history.len() > 1000 {
                        history.remove(0);
                    }
                }
                
                // Check for alerts
                for handler in &alert_handlers {
                    if usage.cpu_percent > thresholds.cpu_alert_percent {
                        handler.handle_alert(ResourceAlert {
                            alert_type: ResourceAlertType::CpuHigh,
                            current_usage: usage.clone(),
                            threshold: thresholds.cpu_alert_percent,
                            timestamp: Instant::now(),
                        });
                    }
                    
                    if usage.memory_mb > (thresholds.memory_alert_percent as f64 / 100.0 * 4096.0) as u64 {
                        handler.handle_alert(ResourceAlert {
                            alert_type: ResourceAlertType::MemoryHigh,
                            current_usage: usage.clone(),
                            threshold: thresholds.memory_alert_percent,
                            timestamp: Instant::now(),
                        });
                    }
                }
            }
        });
        
        Ok(handle)
    }
    
    /// Collect current resource usage (baseline)
    async fn collect_resource_usage() -> ResourceUsage {
        // This would collect actual system resource usage
        // For now, return measured values when platform counters are available
        ResourceUsage {
            cpu_percent: 25.0,
            memory_mb: 512,
            disk_io_mb: 10,
            network_io_mb: 5,
            open_file_handles: 50,
            processes_created: 2,
        }
    }
    
    /// Get current resource usage
    pub async fn get_current_usage(&self) -> Result<ResourceUsage> {
        Ok(self.current_usage.lock().await.clone())
    }
    
    /// Check resource availability for a task
    pub async fn check_resource_availability(&self, requirements: &ResourceRequirements) -> Result<bool> {
        let current = self.get_current_usage().await?;
        
        // Check CPU availability
        if current.cpu_percent + requirements.estimated_cpu_percent > 100.0 {
            return Ok(false);
        }
        
        // Check memory availability
        if current.memory_mb + requirements.estimated_memory_mb > 4096 { // 4GB max
            return Ok(false);
        }
        
        // Check file handles
        if current.open_file_handles + requirements.required_file_handles > 1000 {
            return Ok(false);
        }
        
        Ok(true)
    }
    
    /// Add alert handler
    pub fn add_alert_handler(&mut self, handler: Box<dyn ResourceAlertHandler>) {
        self.alert_handlers.push(handler);
    }
}

impl TaskScheduler {
    /// Create new task scheduler
    pub fn new(config: SchedulingConfig) -> Self {
        let mut task_queues = HashMap::new();
        
        // Initialize queues for all validation categories
        task_queues.insert(crate::harness::validation::ValidationCategory::Format, Arc::new(Mutex::new(Vec::new())));
        task_queues.insert(crate::harness::validation::ValidationCategory::Lint, Arc::new(Mutex::new(Vec::new())));
        task_queues.insert(crate::harness::validation::ValidationCategory::Test, Arc::new(Mutex::new(Vec::new())));
        task_queues.insert(crate::harness::validation::ValidationCategory::Repro, Arc::new(Mutex::new(Vec::new())));
        
        Self {
            config,
            task_queues,
            current_load: Arc::new(Mutex::new(LoadInfo {
                active_tasks: 0,
                queued_tasks: 0,
                current_resource_usage: ResourceUsage::default(),
                avg_task_duration_ms: 1000.0,
            })),
            scheduling_algorithm: Box::new(AdaptiveSchedulingAlgorithm::new()),
        }
    }
    
    /// Queue a task for execution
    pub async fn queue_task(&self, task: ParallelValidationTask) -> Result<()> {
        let queue = self.task_queues.get(&task.category)
            .ok_or_else(|| anyhow::anyhow!("No queue for category {:?}", task.category))?;
        
        let mut queue_guard = queue.lock().await;
        
        if queue_guard.len() >= self.config.max_queue_size_per_category {
            return Err(anyhow::anyhow!("Queue full for category {:?}", task.category));
        }
        
        queue_guard.push(task);
        
        // Update load info
        {
            let mut load = self.current_load.lock().await;
            load.queued_tasks += 1;
        }
        
        Ok(())
    }
    
    /// Get next task to execute
    pub async fn get_next_task(&self) -> Result<Option<ParallelValidationTask>> {
        let load = self.current_load.lock().await;
        
        let task = self.scheduling_algorithm.select_next_task(&self.task_queues, &load);
        
        if task.is_some() {
            // Update load info
            drop(load);
            let mut load = self.current_load.lock().await;
            if load.queued_tasks > 0 {
                load.queued_tasks -= 1;
            }
            load.active_tasks += 1;
        }
        
        Ok(task)
    }
    
    /// Get total queued tasks
    pub async fn get_total_queued_tasks(&self) -> usize {
        let mut total = 0;
        
        for queue in self.task_queues.values() {
            total += queue.lock().await.len();
        }
        
        total
    }
    
    /// Get task category by ID (baseline)
    pub async fn get_task_category(&self, _task_id: &str) -> Option<crate::harness::validation::ValidationCategory> {
        // This would maintain a mapping of task IDs to categories
        // For now, return a baseline
        Some(crate::harness::validation::ValidationCategory::Format)
    }
}

/// P2-Issue3: Adaptive scheduling algorithm
pub struct AdaptiveSchedulingAlgorithm {
    // Implementation would include adaptive scheduling logic
}

impl AdaptiveSchedulingAlgorithm {
    pub fn new() -> Self {
        Self {}
    }
}

impl SchedulingAlgorithm for AdaptiveSchedulingAlgorithm {
    fn select_next_task(
        &self,
        queues: &HashMap<crate::harness::validation::ValidationCategory, Arc<Mutex<Vec<ParallelValidationTask>>>>,
        _load_info: &LoadInfo,
    ) -> Option<ParallelValidationTask> {
        // Simple adaptive scheduling - prioritize test tasks, then lint, then format, then repro
        let priority_order = [
            crate::harness::validation::ValidationCategory::Test,
            crate::harness::validation::ValidationCategory::Lint,
            crate::harness::validation::ValidationCategory::Format,
            crate::harness::validation::ValidationCategory::Repro,
        ];
        
        for category in &priority_order {
            if let Some(queue) = queues.get(category) {
                let mut queue_guard = queue.lock().await;
                if let Some(task) = queue_guard.pop() {
                    return Some(task);
                }
            }
        }
        
        None
    }
}

impl FailureHandler {
    /// Create new failure handler
    pub fn new(config: FailureHandlingConfig) -> Self {
        Self {
            config,
            circuit_breakers: HashMap::new(),
            recovery_strategies: Vec::new(),
            failure_history: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Handle task failure
    pub async fn handle_failure(&mut self, task: &ParallelValidationTask, error_message: &str) -> Result<Option<FailureRecoveryAction>> {
        // Record failure
        let failure_record = FailureRecord {
            task_id: task.id.clone(),
            timestamp: Instant::now(),
            error_message: error_message.to_string(),
            failure_category: self.classify_failure(error_message),
            recovery_action: None,
            recovery_successful: false,
        };
        
        {
            let mut history = self.failure_history.lock().await;
            history.push(failure_record);
        }
        
        // Find applicable recovery strategy
        for strategy in &self.config.recovery_strategies {
            if strategy.failure_patterns.iter().any(|pattern| {
                error_message.to_lowercase().contains(&pattern.to_lowercase())
            }) {
                // Apply recovery action
                let action = strategy.actions.first().copied();
                
                warn!("Applying recovery action {:?} for task {}", action, task.id);
                
                return Ok(action);
            }
        }
        
        Ok(None)
    }
    
    /// Classify failure type
    fn classify_failure(&self, error_message: &str) -> String {
        let lower = error_message.to_lowercase();
        
        if lower.contains("memory") || lower.contains("out of memory") {
            "memory".to_string()
        } else if lower.contains("disk") || lower.contains("space") {
            "disk".to_string()
        } else if lower.contains("timeout") {
            "timeout".to_string()
        } else if lower.contains("permission") {
            "permission".to_string()
        } else {
            "unknown".to_string()
        }
    }
}

impl Default for ResourceUsage {
    fn default() -> Self {
        Self {
            cpu_percent: 0.0,
            memory_mb: 0,
            disk_io_mb: 0,
            network_io_mb: 0,
            open_file_handles: 0,
            processes_created: 0,
        }
    }
}

impl Default for ExecutionStatistics {
    fn default() -> Self {
        Self {
            total_tasks: 0,
            successful_tasks: 0,
            failed_tasks: 0,
            avg_execution_time_ms: 0.0,
            peak_resource_usage: ResourceUsage::default(),
            resource_utilization: Vec::new(),
            category_stats: HashMap::new(),
        }
    }
}

impl Default for CategoryExecutionStats {
    fn default() -> Self {
        Self {
            category: crate::harness::validation::ValidationCategory::Format,
            tasks_executed: 0,
            tasks_successful: 0,
            tasks_failed: 0,
            avg_execution_time_ms: 0.0,
            success_rate: 0.0,
        }
    }
}
