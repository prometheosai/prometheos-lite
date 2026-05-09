//! P1-Issue6: Full validation artifacts storage
//!
//! This module provides comprehensive storage of validation artifacts including
//! stdout/stderr, truncated summaries, file paths, and execution metadata.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use chrono::{DateTime, Utc};

/// P1-Issue6: Complete validation artifacts storage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationArtifacts {
    /// Unique identifier for this validation run
    pub validation_id: String,
    /// Timestamp when validation started
    pub started_at: DateTime<Utc>,
    /// Timestamp when validation completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Total duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Repository root path
    pub repo_root: PathBuf,
    /// Validation plan that was executed
    pub plan: crate::harness::validation::ValidationPlan,
    /// Individual command artifacts
    pub command_artifacts: Vec<CommandArtifacts>,
    /// Summary statistics
    pub summary: ValidationSummary,
    /// File system state changes
    pub file_changes: Vec<FileChange>,
    /// Performance metrics
    pub performance_metrics: PerformanceMetrics,
    /// Error analysis
    pub error_analysis: ErrorAnalysis,
}

/// P1-Issue6: Artifacts for a single validation command
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandArtifacts {
    /// Unique identifier for this command execution
    pub command_id: String,
    /// The command that was executed
    pub command: String,
    /// Working directory where command was executed
    pub working_dir: PathBuf,
    /// Command arguments
    pub args: Vec<String>,
    /// Environment variables used
    pub env_vars: HashMap<String, String>,
    /// Execution category (format, lint, test, repro)
    pub category: crate::harness::validation::ValidationCategory,
    /// Start timestamp
    pub started_at: DateTime<Utc>,
    /// End timestamp
    pub completed_at: Option<DateTime<Utc>>,
    /// Duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Full stdout output
    pub stdout: FullOutput,
    /// Full stderr output
    pub stderr: FullOutput,
    /// Success status
    pub success: bool,
    /// Timeout status
    pub timed_out: bool,
    /// Resource usage
    pub resource_usage: ResourceUsage,
    /// Files accessed during execution
    pub files_accessed: Vec<FileAccess>,
    /// Issues detected
    pub issues: Vec<ValidationIssue>,
    /// Truncated summary for quick viewing
    pub summary: CommandSummary,
}

/// P1-Issue6: Full output with truncation metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FullOutput {
    /// Complete output content
    pub content: String,
    /// Total size in bytes
    pub size_bytes: usize,
    /// Number of lines
    pub line_count: usize,
    /// Whether output was truncated
    pub truncated: bool,
    /// Truncation strategy used
    pub truncation_strategy: TruncationStrategy,
    /// Original size before truncation (if truncated)
    pub original_size: Option<usize>,
    /// Checksum for integrity verification
    pub checksum: String,
}

/// P1-Issue6: Truncation strategies for large outputs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TruncationStrategy {
    /// No truncation
    None,
    /// Truncate by line count
    ByLineCount { max_lines: usize },
    /// Truncate by byte size
    ByByteSize { max_bytes: usize },
    /// Smart truncation (keep important lines)
    Smart { max_bytes: usize },
    /// Head and tail truncation
    HeadTail { head_bytes: usize, tail_bytes: usize },
}

/// P1-Issue6: Resource usage during command execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUsage {
    /// Memory usage in MB
    pub memory_mb: f64,
    /// CPU time in milliseconds
    pub cpu_time_ms: u64,
    /// Disk space used in MB
    pub disk_space_mb: f64,
    /// Network bytes transferred
    pub network_bytes: u64,
    /// Number of processes created
    pub processes_created: u32,
    /// Peak memory usage in MB
    pub peak_memory_mb: f64,
}

/// P1-Issue6: File access record
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileAccess {
    /// File path
    pub path: PathBuf,
    /// Type of access
    pub access_type: FileAccessType,
    /// Timestamp of access
    pub timestamp: DateTime<Utc>,
    /// Success status
    pub success: bool,
    /// Error message if any
    pub error: Option<String>,
    /// File size if applicable
    pub file_size: Option<u64>,
}

/// P1-Issue6: Types of file access
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileAccessType {
    Read,
    Write,
    Create,
    Delete,
    Execute,
    Stat,
}

/// P1-Issue6: Validation issue detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationIssue {
    /// Issue identifier
    pub id: String,
    /// Issue severity
    pub severity: ValidationIssueSeverity,
    /// Issue category
    pub category: String,
    /// File where issue occurred
    pub file: Option<PathBuf>,
    /// Line number
    pub line: Option<u32>,
    /// Column number
    pub column: Option<u32>,
    /// Issue message
    pub message: String,
    /// Issue code or identifier
    pub code: Option<String>,
    /// Suggested fix
    pub fix_suggestion: Option<String>,
    /// Context around the issue
    pub context: Option<String>,
    /// Tool that detected the issue
    pub detected_by: String,
    /// Timestamp when detected
    pub detected_at: DateTime<Utc>,
}

/// P1-Issue6: Issue severity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ValidationIssueSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// P1-Issue6: Command summary for quick viewing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandSummary {
    /// Brief status description
    pub status: String,
    /// Key metrics
    pub metrics: HashMap<String, String>,
    /// Top issues (up to 5)
    pub top_issues: Vec<String>,
    /// Performance summary
    pub performance: String,
    /// File summary
    pub file_summary: String,
}

/// P1-Issue6: Overall validation summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationSummary {
    /// Total commands executed
    pub total_commands: usize,
    /// Commands passed
    pub passed_commands: usize,
    /// Commands failed
    pub failed_commands: usize,
    /// Commands timed out
    pub timed_out_commands: usize,
    /// Total issues found
    pub total_issues: usize,
    /// Issues by severity
    pub issues_by_severity: HashMap<ValidationIssueSeverity, usize>,
    /// Issues by category
    pub issues_by_category: HashMap<String, usize>,
    /// Files affected
    pub files_affected: usize,
    /// Most affected files
    pub most_affected_files: Vec<PathBuf>,
    /// Overall success status
    pub success: bool,
    /// Execution summary
    pub execution_summary: String,
}

/// P1-Issue6: File system changes during validation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChange {
    /// File path
    pub path: PathBuf,
    /// Type of change
    pub change_type: FileChangeType,
    /// Timestamp of change
    pub timestamp: DateTime<Utc>,
    /// File size before change
    pub size_before: Option<u64>,
    /// File size after change
    pub size_after: Option<u64>,
    /// Checksum before change
    pub checksum_before: Option<String>,
    /// Checksum after change
    pub checksum_after: Option<String>,
    /// Reason for change
    pub reason: Option<String>,
}

/// P1-Issue6: Types of file changes
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed,
    PermissionChanged,
}

/// P1-Issue6: Performance metrics for validation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceMetrics {
    /// Total execution time in milliseconds
    pub total_time_ms: u64,
    /// Average command time in milliseconds
    pub avg_command_time_ms: f64,
    /// Slowest command time in milliseconds
    pub slowest_command_ms: u64,
    /// Fastest command time in milliseconds
    pub fastest_command_ms: u64,
    /// Memory usage statistics
    pub memory_stats: MemoryStats,
    /// Disk I/O statistics
    pub disk_io: DiskIoStats,
    /// Network I/O statistics
    pub network_io: NetworkIoStats,
}

/// P1-Issue6: Memory usage statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryStats {
    /// Peak memory usage in MB
    pub peak_mb: f64,
    /// Average memory usage in MB
    pub average_mb: f64,
    /// Minimum memory usage in MB
    pub minimum_mb: f64,
    /// Memory usage trend
    pub trend: MemoryTrend,
}

/// P1-Issue6: Memory usage trend
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryTrend {
    Increasing,
    Decreasing,
    Stable,
    Fluctuating,
}

/// P1-Issue6: Disk I/O statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskIoStats {
    /// Total bytes read
    pub bytes_read: u64,
    /// Total bytes written
    pub bytes_written: u64,
    /// Number of read operations
    pub read_ops: u64,
    /// Number of write operations
    pub write_ops: u64,
    /// Files accessed
    pub files_accessed: u64,
}

/// P1-Issue6: Network I/O statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkIoStats {
    /// Total bytes received
    pub bytes_received: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Number of network connections
    pub connections: u64,
    /// DNS queries made
    pub dns_queries: u64,
}

/// P1-Issue6: Error analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorAnalysis {
    /// Common error patterns
    pub common_patterns: Vec<ErrorPattern>,
    /// Error frequency analysis
    pub frequency_analysis: ErrorFrequencyAnalysis,
    /// Error correlation analysis
    pub correlation_analysis: ErrorCorrelationAnalysis,
    /// Recommendations for fixing errors
    pub recommendations: Vec<String>,
}

/// P1-Issue6: Error pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorPattern {
    /// Pattern identifier
    pub id: String,
    /// Pattern description
    pub description: String,
    /// Regular expression to match pattern
    pub regex: String,
    /// Number of occurrences
    pub occurrences: usize,
    /// Commands where pattern appears
    pub commands: Vec<String>,
    /// Suggested fix
    pub suggested_fix: Option<String>,
}

/// P1-Issue6: Error frequency analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorFrequencyAnalysis {
    /// Total errors
    pub total_errors: usize,
    /// Errors by hour
    pub errors_by_hour: HashMap<String, usize>,
    /// Errors by command type
    pub errors_by_command: HashMap<String, usize>,
    /// Most frequent errors
    pub most_frequent: Vec<(String, usize)>,
}

/// P1-Issue6: Error correlation analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorCorrelationAnalysis {
    /// Correlated errors (errors that often occur together)
    pub correlated_errors: Vec<(String, String, f64)>,
    /// Error chains (sequences of errors)
    pub error_chains: Vec<Vec<String>>,
    /// Root cause candidates
    pub root_cause_candidates: Vec<String>,
}

/// P1-Issue6: Validation artifacts storage manager
pub struct ValidationArtifactsManager {
    /// Storage backend
    storage: Box<dyn ArtifactStorage>,
    /// Maximum artifact size in bytes
    max_artifact_size: usize,
    /// Retention policy
    retention_policy: RetentionPolicy,
}

/// P1-Issue6: Artifact storage trait
#[async_trait::async_trait]
pub trait ArtifactStorage {
    /// Store validation artifacts
    async fn store_artifacts(&self, artifacts: &ValidationArtifacts) -> Result<String>;
    
    /// Retrieve validation artifacts by ID
    async fn retrieve_artifacts(&self, id: &str) -> Result<ValidationArtifacts>;
    
    /// List stored artifacts
    async fn list_artifacts(&self, filter: Option<&ArtifactFilter>) -> Result<Vec<ArtifactMetadata>>;
    
    /// Delete artifacts by ID
    async fn delete_artifacts(&self, id: &str) -> Result<()>;
    
    /// Clean up expired artifacts
    async fn cleanup_expired(&self) -> Result<usize>;
}

/// P1-Issue6: Artifact metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactMetadata {
    /// Artifact ID
    pub id: String,
    /// Repository root
    pub repo_root: PathBuf,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Artifact size in bytes
    pub size_bytes: usize,
    /// Number of commands
    pub command_count: usize,
    /// Success status
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Tags
    pub tags: Vec<String>,
}

/// P1-Issue6: Artifact filter for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactFilter {
    /// Filter by repository root
    pub repo_root: Option<PathBuf>,
    /// Filter by date range
    pub date_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Filter by success status
    pub success: Option<bool>,
    /// Filter by tags
    pub tags: Vec<String>,
    /// Limit number of results
    pub limit: Option<usize>,
    /// Sort order
    pub sort_by: ArtifactSortBy,
}

/// P1-Issue6: Sort options for artifact listing
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactSortBy {
    CreatedAt,
    Duration,
    Success,
    Size,
}

/// P1-Issue6: Retention policy for artifacts
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetentionPolicy {
    /// Maximum age in days
    pub max_age_days: u32,
    /// Maximum number of artifacts to keep
    pub max_count: usize,
    /// Maximum total size in MB
    pub max_size_mb: u64,
    /// Whether to keep failed validations
    pub keep_failed: bool,
    /// Whether to keep successful validations
    pub keep_successful: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            max_count: 1000,
            max_size_mb: 1024, // 1GB
            keep_failed: true,
            keep_successful: true,
        }
    }
}

impl ValidationArtifactsManager {
    /// Create a new artifacts manager
    pub fn new(storage: Box<dyn ArtifactStorage>) -> Self {
        Self {
            storage,
            max_artifact_size: 100 * 1024 * 1024, // 100MB
            retention_policy: RetentionPolicy::default(),
        }
    }
    
    /// Store validation artifacts
    pub async fn store(&self, artifacts: &ValidationArtifacts) -> Result<String> {
        // Validate artifact size
        let artifact_size = self.calculate_artifact_size(artifacts)?;
        if artifact_size > self.max_artifact_size {
            anyhow::bail!("Artifact size {} exceeds maximum {}", artifact_size, self.max_artifact_size);
        }
        
        self.storage.store_artifacts(artifacts).await
    }
    
    /// Retrieve validation artifacts
    pub async fn retrieve(&self, id: &str) -> Result<ValidationArtifacts> {
        self.storage.retrieve_artifacts(id).await
    }
    
    /// List artifacts with optional filtering
    pub async fn list(&self, filter: Option<&ArtifactFilter>) -> Result<Vec<ArtifactMetadata>> {
        self.storage.list_artifacts(filter).await
    }
    
    /// Delete artifacts
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.storage.delete_artifacts(id).await
    }
    
    /// Clean up expired artifacts
    pub async fn cleanup(&self) -> Result<usize> {
        self.storage.cleanup_expired().await
    }
    
    /// Calculate artifact size
    fn calculate_artifact_size(&self, artifacts: &ValidationArtifacts) -> Result<usize> {
        // Simple size calculation - in a real implementation this would be more accurate
        let serialized = serde_json::to_string(artifacts)?;
        Ok(serialized.len())
    }
}

impl FullOutput {
    /// Create new full output from content
    pub fn new(content: String, max_size: usize) -> Self {
        let size_bytes = content.len();
        let line_count = content.lines().count();
        
        let (truncated_content, truncated, original_size, strategy) = if size_bytes > max_size {
            // Apply smart truncation
            let truncated_content = Self::smart_truncate(&content, max_size);
            (
                truncated_content,
                true,
                Some(size_bytes),
                TruncationStrategy::Smart { max_bytes: max_size },
            )
        } else {
            (content, false, None, TruncationStrategy::None)
        };
        
        let checksum = Self::calculate_checksum(&truncated_content);
        
        Self {
            content: truncated_content,
            size_bytes: truncated_content.len(),
            line_count: truncated_content.lines().count(),
            truncated,
            truncation_strategy: strategy,
            original_size,
            checksum,
        }
    }
    
    /// Smart truncation that keeps important lines
    fn smart_truncate(content: &str, max_bytes: usize) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut important_lines = Vec::new();
        let mut current_size = 0;
        
        // Prioritize error lines, warnings, and important patterns
        for line in &lines {
            let line_size = line.len() + 1; // +1 for newline
            
            // Check if line is important
            let is_important = line.contains("error") || 
                             line.contains("Error") ||
                             line.contains("ERROR") ||
                             line.contains("warning") ||
                             line.contains("Warning") ||
                             line.contains("failed") ||
                             line.contains("FAILED") ||
                             line.starts_with("error:") ||
                             line.starts_with("warning:");
            
            if is_important || current_size + line_size < max_bytes {
                important_lines.push(*line);
                current_size += line_size;
                
                if current_size >= max_bytes {
                    break;
                }
            }
        }
        
        // If we still have room, add more lines from the beginning
        if current_size < max_bytes {
            for line in &lines {
                if !important_lines.contains(line) {
                    let line_size = line.len() + 1;
                    if current_size + line_size > max_bytes {
                        break;
                    }
                    important_lines.push(*line);
                    current_size += line_size;
                }
            }
        }
        
        important_lines.join("\n")
    }
    
    /// Calculate checksum for content
    fn calculate_checksum(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

impl ValidationArtifacts {
    /// Create new validation artifacts
    pub fn new(
        validation_id: String,
        repo_root: PathBuf,
        plan: crate::harness::validation::ValidationPlan,
    ) -> Self {
        Self {
            validation_id,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            repo_root,
            plan,
            command_artifacts: Vec::new(),
            summary: ValidationSummary::default(),
            file_changes: Vec::new(),
            performance_metrics: PerformanceMetrics::default(),
            error_analysis: ErrorAnalysis::default(),
        }
    }
    
    /// Add command artifacts
    pub fn add_command_artifacts(&mut self, artifacts: CommandArtifacts) {
        self.command_artifacts.push(artifacts);
    }
    
    /// Mark validation as completed
    pub fn mark_completed(&mut self) {
        self.completed_at = Some(Utc::now());
        if let Some(started) = self.started_at {
            self.duration_ms = Some(self.completed_at.unwrap().signed_duration_since(started).num_milliseconds() as u64);
        }
        self.update_summary();
        self.update_performance_metrics();
        self.analyze_errors();
    }
    
    /// Update validation summary
    fn update_summary(&mut self) {
        let total_commands = self.command_artifacts.len();
        let passed_commands = self.command_artifacts.iter().filter(|c| c.success).count();
        let failed_commands = self.command_artifacts.iter().filter(|c| !c.success && !c.timed_out).count();
        let timed_out_commands = self.command_artifacts.iter().filter(|c| c.timed_out).count();
        
        let mut total_issues = 0;
        let mut issues_by_severity = HashMap::new();
        let mut issues_by_category = HashMap::new();
        let mut files_affected = std::collections::HashSet::new();
        
        for command in &self.command_artifacts {
            total_issues += command.issues.len();
            for issue in &command.issues {
                *issues_by_severity.entry(issue.severity).or_insert(0) += 1;
                *issues_by_category.entry(issue.category.clone()).or_insert(0) += 1;
                if let Some(file) = &issue.file {
                    files_affected.insert(file.clone());
                }
            }
        }
        
        let most_affected_files = files_affected.into_iter().take(10).collect();
        
        self.summary = ValidationSummary {
            total_commands,
            passed_commands,
            failed_commands,
            timed_out_commands,
            total_issues,
            issues_by_severity,
            issues_by_category,
            files_affected: files_affected.len(),
            most_affected_files,
            success: failed_commands == 0 && timed_out_commands == 0,
            execution_summary: format!(
                "Executed {} commands: {} passed, {} failed, {} timed out",
                total_commands, passed_commands, failed_commands, timed_out_commands
            ),
        };
    }
    
    /// Update performance metrics
    fn update_performance_metrics(&mut self) {
        let durations: Vec<u64> = self.command_artifacts
            .iter()
            .filter_map(|c| c.duration_ms)
            .collect();
        
        if !durations.is_empty() {
            let total_time = durations.iter().sum::<u64>();
            let avg_time = total_time as f64 / durations.len() as f64;
            let slowest = *durations.iter().max().unwrap_or(&0);
            let fastest = *durations.iter().min().unwrap_or(&0);
            
            self.performance_metrics = PerformanceMetrics {
                total_time_ms: total_time,
                avg_command_time_ms: avg_time,
                slowest_command_ms: slowest,
                fastest_command_ms: fastest,
                memory_stats: MemoryStats::default(),
                disk_io: DiskIoStats::default(),
                network_io: NetworkIoStats::default(),
            };
        }
    }
    
    /// Analyze errors
    fn analyze_errors(&mut self) {
        let mut common_patterns = Vec::new();
        let mut error_frequency = ErrorFrequencyAnalysis::default();
        let mut correlation_analysis = ErrorCorrelationAnalysis::default();
        let mut recommendations = Vec::new();
        
        // Analyze error patterns (simplified)
        for command in &self.command_artifacts {
            if !command.success {
                // Add pattern detection logic here
                error_frequency.total_errors += 1;
            }
        }
        
        self.error_analysis = ErrorAnalysis {
            common_patterns,
            frequency_analysis: error_frequency,
            correlation_analysis,
            recommendations,
        };
    }
}

impl Default for ValidationSummary {
    fn default() -> Self {
        Self {
            total_commands: 0,
            passed_commands: 0,
            failed_commands: 0,
            timed_out_commands: 0,
            total_issues: 0,
            issues_by_severity: HashMap::new(),
            issues_by_category: HashMap::new(),
            files_affected: 0,
            most_affected_files: Vec::new(),
            success: false,
            execution_summary: String::new(),
        }
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            total_time_ms: 0,
            avg_command_time_ms: 0.0,
            slowest_command_ms: 0,
            fastest_command_ms: 0,
            memory_stats: MemoryStats::default(),
            disk_io: DiskIoStats::default(),
            network_io: NetworkIoStats::default(),
        }
    }
}

impl Default for MemoryStats {
    fn default() -> Self {
        Self {
            peak_mb: 0.0,
            average_mb: 0.0,
            minimum_mb: 0.0,
            trend: MemoryTrend::Stable,
        }
    }
}

impl Default for DiskIoStats {
    fn default() -> Self {
        Self {
            bytes_read: 0,
            bytes_written: 0,
            read_ops: 0,
            write_ops: 0,
            files_accessed: 0,
        }
    }
}

impl Default for NetworkIoStats {
    fn default() -> Self {
        Self {
            bytes_received: 0,
            bytes_sent: 0,
            connections: 0,
            dns_queries: 0,
        }
    }
}

impl Default for ErrorAnalysis {
    fn default() -> Self {
        Self {
            common_patterns: Vec::new(),
            frequency_analysis: ErrorFrequencyAnalysis::default(),
            correlation_analysis: ErrorCorrelationAnalysis::default(),
            recommendations: Vec::new(),
        }
    }
}

impl Default for ErrorFrequencyAnalysis {
    fn default() -> Self {
        Self {
            total_errors: 0,
            errors_by_hour: HashMap::new(),
            errors_by_command: HashMap::new(),
            most_frequent: Vec::new(),
        }
    }
}

impl Default for ErrorCorrelationAnalysis {
    fn default() -> Self {
        Self {
            correlated_errors: Vec::new(),
            error_chains: Vec::new(),
            root_cause_candidates: Vec::new(),
        }
    }
}
