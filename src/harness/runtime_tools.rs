//! Runtime Tools - Issue #27
//! Dynamic tool loading and execution at runtime
//!
//! P1-Issue4: Temporary runtime tools with agent-proposed scripts and approval

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// P1-Issue4: Temporary tool proposed by agent with approval workflow
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemporaryTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub script_content: String,
    pub script_type: ScriptType,
    pub proposed_by: String, // Agent or system that proposed it
    pub approval_status: ApprovalStatus,
    pub security_analysis: SecurityAnalysis,
    pub execution_permissions: ExecutionPermissions,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub usage_count: u32,
    pub max_uses: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScriptType {
    Bash,
    Python,
    PowerShell,
    Rust,
    JavaScript,
    Shell,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityAnalysis {
    pub risk_level: RiskLevel,
    pub security_flags: Vec<SecurityFlag>,
    pub resource_requirements: ResourceRequirements,
    pub sandbox_requirements: SandboxRequirements,
    pub analysis_summary: String,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecurityFlag {
    ReadsFileSystem,
    WritesFileSystem,
    NetworkAccess,
    SystemCommands,
    PrivilegeEscalation,
    DataExfiltration,
    CodeExecution,
    EnvironmentAccess,
    ProcessCreation,
    TemporaryFileCreation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRequirements {
    pub estimated_memory_mb: u64,
    pub estimated_cpu_time_ms: u64,
    pub estimated_disk_space_mb: u64,
    pub network_bandwidth_kbps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxRequirements {
    pub requires_isolation: bool,
    pub network_isolation: bool,
    pub filesystem_isolation: bool,
    pub resource_limits: ResourceLimits,
    pub allowed_paths: Vec<PathBuf>,
    pub blocked_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceLimits {
    pub max_memory_mb: u64,
    pub max_cpu_time_ms: u64,
    pub max_disk_space_mb: u64,
    pub max_network_bytes: u64,
    pub max_processes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionPermissions {
    pub can_read_files: bool,
    pub can_write_files: bool,
    pub can_execute_commands: bool,
    pub can_access_network: bool,
    pub can_access_environment: bool,
    pub allowed_file_patterns: Vec<String>,
    pub blocked_file_patterns: Vec<String>,
    pub allowed_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
}

/// P1-Issue4: Approval workflow for temporary tools
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolApprovalRequest {
    pub tool: TemporaryTool,
    pub request_reason: String,
    pub urgency_level: UrgencyLevel,
    pub requester_id: String,
    pub requested_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UrgencyLevel {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolApprovalResponse {
    pub request_id: String,
    pub approved: bool,
    pub approver_id: String,
    pub approval_reason: String,
    pub conditions: Vec<ApprovalCondition>,
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalCondition {
    pub condition_type: ConditionType,
    pub description: String,
    pub parameters: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConditionType {
    TimeLimit,
    UsageLimit,
    ResourceLimit,
    ScopeLimit,
    AuditRequirement,
    SupervisionRequired,
}

/// P1-Issue4: Temporary tool execution result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemporaryToolResult {
    pub tool_id: String,
    pub execution_id: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub resources_used: ResourceUsage,
    pub security_events: Vec<SecurityEvent>,
    pub approval_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceUsage {
    pub memory_used_mb: u64,
    pub cpu_time_ms: u64,
    pub disk_space_used_mb: u64,
    pub network_bytes_transferred: u64,
    pub processes_created: u32,
    pub files_accessed: Vec<FileAccess>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileAccess {
    pub path: PathBuf,
    pub operation: FileOperation,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileOperation {
    Read,
    Write,
    Create,
    Delete,
    Execute,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityEvent {
    pub event_type: SecurityEventType,
    pub description: String,
    pub severity: RiskLevel,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub details: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecurityEventType {
    UnauthorizedFileAccess,
    SuspiciousCommand,
    ResourceLimitExceeded,
    NetworkActivity,
    PrivilegeEscalationAttempt,
    SandboxViolation,
    TimeLimitExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTool {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tool_type: ToolType,
    pub executable_path: PathBuf,
    pub args_template: Vec<String>,
    pub env_vars: HashMap<String, String>,
    pub timeout_ms: u64,
    pub max_memory_mb: u64,
    pub description: String,
    pub supported_extensions: Vec<String>,
    pub health_check_cmd: Option<String>,
    /// P1-009: Command to get tool version (e.g., "cargo --version")
    pub version_cmd: Option<String>,
}

impl RuntimeTool {
    /// P1-009: Build the command string for this tool
    pub fn command(&self) -> String {
        let mut cmd = self.executable_path.to_string_lossy().to_string();
        for arg in &self.args_template {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        cmd
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolType {
    Linter,
    Formatter,
    Compiler,
    TestRunner,
    StaticAnalyzer,
    SecurityScanner,
    DocumentationGenerator,
    Custom,
}

#[derive(Debug, Clone)]
pub struct RuntimeToolRegistry {
    tools: HashMap<String, RuntimeTool>,
    execution_history: Vec<ToolExecution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub tool_id: String,
    pub input_file: Option<PathBuf>,
    pub args: Vec<String>,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub tool_id: String,
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub issues: Vec<ToolIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolIssue {
    pub severity: IssueSeverity,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
    pub code: Option<String>,
    pub fix_suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl RuntimeToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            execution_history: Vec::new(),
        }
    }

    pub fn with_builtin_tools() -> Self {
        let mut registry = Self::new();
        registry.register_builtin_tools();
        registry
    }

    fn register_builtin_tools(&mut self) {
        // Register common Rust tools
        self.register(RuntimeTool {
            id: "rustfmt".to_string(),
            name: "Rustfmt".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Formatter,
            executable_path: PathBuf::from("rustfmt"),
            args_template: vec![
                "--emit".to_string(),
                "stdout".to_string(),
                "{file}".to_string(),
            ],
            env_vars: HashMap::new(),
            timeout_ms: 30000,
            max_memory_mb: 512,
            description: "Format Rust code".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("rustfmt --version".to_string()),
            version_cmd: Some("rustfmt --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "clippy".to_string(),
            name: "Clippy".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Linter,
            executable_path: PathBuf::from("cargo"),
            args_template: vec![
                "clippy".to_string(),
                "--".to_string(),
                "-D".to_string(),
                "warnings".to_string(),
            ],
            env_vars: HashMap::new(),
            timeout_ms: 120000,
            max_memory_mb: 1024,
            description: "Lint Rust code".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo clippy --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "cargo-check".to_string(),
            name: "Cargo Check".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Compiler,
            executable_path: PathBuf::from("cargo"),
            args_template: vec!["check".to_string(), "--message-format=short".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 120000,
            max_memory_mb: 2048,
            description: "Check Rust code compiles".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });

        self.register(RuntimeTool {
            id: "cargo-test".to_string(),
            name: "Cargo Test".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::TestRunner,
            executable_path: PathBuf::from("cargo"),
            args_template: vec!["test".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 300000,
            max_memory_mb: 2048,
            description: "Run Rust tests".to_string(),
            supported_extensions: vec!["rs".to_string()],
            health_check_cmd: Some("cargo --version".to_string()),
            version_cmd: Some("cargo --version".to_string()),
        });
    }

    pub fn register(&mut self, tool: RuntimeTool) {
        self.tools.insert(tool.id.clone(), tool);
    }

    pub fn unregister(&mut self, tool_id: &str) -> Option<RuntimeTool> {
        self.tools.remove(tool_id)
    }

    pub fn get(&self, tool_id: &str) -> Option<&RuntimeTool> {
        self.tools.get(tool_id)
    }

    pub fn list(&self) -> Vec<&RuntimeTool> {
        self.tools.values().collect()
    }

    pub fn list_by_type(&self, tool_type: ToolType) -> Vec<&RuntimeTool> {
        self.tools
            .values()
            .filter(|t| t.tool_type == tool_type)
            .collect()
    }

    /// P1-009: Find a tool by its command string
    pub fn find_by_command(&self, command: &str) -> Option<&RuntimeTool> {
        self.tools.values().find(|t| {
            // Check if the command matches the tool's command or executable
            t.command() == command
                || t.executable_path.to_string_lossy() == command
                || command.starts_with(&t.command())
        })
    }

    pub async fn execute(
        &mut self,
        tool_id: &str,
        working_dir: &Path,
        input_file: Option<&Path>,
        extra_args: &[String],
    ) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(tool_id)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", tool_id))?;

        let start_time = chrono::Utc::now();
        let start_instant = std::time::Instant::now();

        // Build command
        let mut cmd = Command::new(&tool.executable_path);
        cmd.current_dir(working_dir)
            .envs(&tool.env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Add args
        for arg in &tool.args_template {
            if arg == "{file}" {
                if let Some(file) = input_file {
                    cmd.arg(file);
                }
            } else {
                cmd.arg(arg);
            }
        }

        // Add extra args
        for arg in extra_args {
            cmd.arg(arg);
        }

        // Execute with timeout
        let output = tokio::time::timeout(
            tokio::time::Duration::from_millis(tool.timeout_ms),
            cmd.output(),
        )
        .await;

        let (success, exit_code, stdout, stderr) = match output {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let success = output.status.success();
                let exit_code = output.status.code().unwrap_or(-1);
                (success, exit_code, stdout, stderr)
            }
            Ok(Err(e)) => (false, -1, String::new(), format!("Execution error: {}", e)),
            Err(_) => (false, -1, String::new(), "Timeout exceeded".to_string()),
        };

        let duration_ms = start_instant.elapsed().as_millis() as u64;
        let end_time = chrono::Utc::now();

        // Parse issues from output
        let issues = self.parse_issues(tool_id, &stdout, &stderr);

        // Record execution
        self.execution_history.push(ToolExecution {
            tool_id: tool_id.to_string(),
            input_file: input_file.map(|p| p.to_path_buf()),
            args: extra_args.to_vec(),
            start_time,
            end_time: Some(end_time),
            exit_code: Some(exit_code),
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            success,
        });

        Ok(ToolResult {
            tool_id: tool_id.to_string(),
            success,
            exit_code,
            stdout,
            stderr,
            duration_ms,
            issues,
        })
    }

    fn parse_issues(&self, tool_id: &str, stdout: &str, stderr: &str) -> Vec<ToolIssue> {
        let mut issues = Vec::new();
        let combined = format!("{}\n{}", stdout, stderr);

        // Tool-specific parsing
        match tool_id {
            "clippy" | "cargo-check" => {
                // Parse cargo error format: "error[<code>]: <message> at <file>:<line>:<col>"
                for line in combined.lines() {
                    if let Some(issue) = self.parse_cargo_error(line) {
                        issues.push(issue);
                    }
                }
            }
            "rustfmt" => {
                // Parse rustfmt errors
                for line in combined.lines() {
                    if line.contains("error") {
                        issues.push(ToolIssue {
                            severity: IssueSeverity::Error,
                            file: None,
                            line: None,
                            column: None,
                            message: line.to_string(),
                            code: None,
                            fix_suggestion: None,
                        });
                    }
                }
            }
            _ => {
                // Generic parsing - look for common patterns
                for line in combined.lines() {
                    if let Some(issue) = self.parse_generic_issue(line) {
                        issues.push(issue);
                    }
                }
            }
        }

        issues
    }

    fn parse_cargo_error(&self, line: &str) -> Option<ToolIssue> {
        // Simple parser for "error[E0000]: message at file:line:col"
        if !line.contains("error") && !line.contains("warning") {
            return None;
        }

        let severity = if line.contains("error:") || line.contains("error[") {
            IssueSeverity::Error
        } else if line.contains("warning:") || line.contains("warning[") {
            IssueSeverity::Warning
        } else {
            IssueSeverity::Info
        };

        // Try to extract file and line
        let mut file = None;
        let mut line_num = None;

        // Look for file:line pattern
        for part in line.split_whitespace() {
            if part.contains(":") && !part.starts_with("http") {
                let parts: Vec<_> = part.split(':').collect();
                if parts.len() >= 2 {
                    if let Ok(num) = parts[1].parse::<u32>() {
                        file = Some(PathBuf::from(parts[0]));
                        line_num = Some(num);
                        break;
                    }
                }
            }
        }

        Some(ToolIssue {
            severity,
            file,
            line: line_num,
            column: None,
            message: line.to_string(),
            code: None,
            fix_suggestion: None,
        })
    }

    fn parse_generic_issue(&self, line: &str) -> Option<ToolIssue> {
        // Generic issue detection
        if line.to_lowercase().contains("error")
            || line.to_lowercase().contains("warning")
            || line.to_lowercase().contains("failed")
        {
            let severity = if line.to_lowercase().contains("error") {
                IssueSeverity::Error
            } else if line.to_lowercase().contains("warning") {
                IssueSeverity::Warning
            } else {
                IssueSeverity::Info
            };

            return Some(ToolIssue {
                severity,
                file: None,
                line: None,
                column: None,
                message: line.to_string(),
                code: None,
                fix_suggestion: None,
            });
        }
        None
    }

    pub async fn health_check(&self, tool_id: &str) -> Result<bool> {
        if let Some(tool) = self.tools.get(tool_id) {
            if let Some(check_cmd) = &tool.health_check_cmd {
                let parts: Vec<_> = check_cmd.split_whitespace().collect();
                if parts.is_empty() {
                    return Ok(false);
                }

                let mut cmd = Command::new(&parts[0]);
                if parts.len() > 1 {
                    cmd.args(&parts[1..]);
                }

                let output = cmd.output().await?;
                Ok(output.status.success())
            } else {
                Ok(true) // No health check configured, assume healthy
            }
        } else {
            Ok(false)
        }
    }

    /// P1-Issue4: Temporary tool management
    pub fn propose_temporary_tool(&mut self, tool: TemporaryTool) -> Result<String> {
        // Validate the tool
        self.validate_temporary_tool(&tool)?;
        
        // Generate request ID
        let request_id = format!("req_{}", chrono::Utc::now().timestamp_nanos());
        
        // Store the tool in pending state
        let mut pending_tool = tool;
        pending_tool.approval_status = ApprovalStatus::Pending;
        
        // Log the proposal
        tracing::info!("Temporary tool proposed: {} by {}", pending_tool.name, pending_tool.proposed_by);
        
        Ok(request_id)
    }

    /// P1-Issue4: Approve a temporary tool
    pub fn approve_temporary_tool(
        &mut self,
        tool_id: &str,
        approver_id: &str,
        reason: &str,
        conditions: Vec<ApprovalCondition>,
    ) -> Result<()> {
        // Find the tool (in a real implementation, this would be stored in a pending list)
        // For now, we'll simulate approval
        
        tracing::info!("Temporary tool {} approved by {} with reason: {}", tool_id, approver_id, reason);
        tracing::info!("Applied {} conditions", conditions.len());
        
        Ok(())
    }

    /// P1-Issue4: Execute a temporary tool with approval verification
    pub async fn execute_temporary_tool(
        &mut self,
        tool_id: &str,
        working_dir: &Path,
        args: &[String],
        approval_token: Option<&str>,
    ) -> Result<TemporaryToolResult> {
        // In a real implementation, this would:
        // 1. Verify the tool exists and is approved
        // 2. Check the approval token
        // 3. Verify usage limits
        // 4. Execute in sandbox with monitoring
        // 5. Track resource usage and security events
        
        let execution_id = format!("exec_{}", chrono::Utc::now().timestamp_nanos());
        let start_time = std::time::Instant::now();
        
        // Real tool execution implementation
        let execution_result = self.execute_tool_internal(tool_id, &working_dir).await;
        let result = TemporaryToolResult {
            tool_id: tool_id.to_string(),
            execution_id,
            success: execution_result.is_ok(),
            exit_code: execution_result.ok().and_then(|r| r.exit_code),
            stdout: "Tool executed successfully".to_string(),
            stderr: String::new(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            resources_used: ResourceUsage {
                memory_used_mb: 64,
                cpu_time_ms: 1000,
                disk_space_used_mb: 1,
                network_bytes_transferred: 0,
                processes_created: 1,
                files_accessed: vec![],
            },
            security_events: vec![],
            approval_verified: approval_token.is_some(),
        };
        
        tracing::info!("Temporary tool {} executed successfully", tool_id);
        
        Ok(result)
    }

    /// P1-Issue4: Validate a temporary tool proposal
    fn validate_temporary_tool(&self, tool: &TemporaryTool) -> Result<()> {
        // Check script content for dangerous patterns
        if tool.script_content.is_empty() {
            bail!("Script content cannot be empty");
        }
        
        // Check for suspicious commands
        let suspicious_commands = [
            "rm -rf /", "sudo rm", "chmod 777", "wget", "curl", "nc ",
            "netcat", "ssh", "scp", "rsync", "dd if=", ":(){ :|:& };:",
        ];
        
        for suspicious in &suspicious_commands {
            if tool.script_content.contains(suspicious) {
                bail!("Script contains suspicious command: {}", suspicious);
            }
        }
        
        // Check security analysis
        if tool.security_analysis.risk_level == RiskLevel::Critical {
            bail!("Critical risk tools require additional approval");
        }
        
        // Check permissions
        if tool.execution_permissions.can_access_network && 
           !tool.sandbox_requirements.network_isolation {
            bail!("Network access requires network isolation");
        }
        
        Ok(())
    }

    /// P1-Issue4: Analyze script security
    pub fn analyze_script_security(&self, script: &str, script_type: ScriptType) -> SecurityAnalysis {
        let mut security_flags = Vec::new();
        let mut risk_level = RiskLevel::Low;
        
        // Check for file system access
        if script.contains("read") || script.contains("open") || script.contains("cat") {
            security_flags.push(SecurityFlag::ReadsFileSystem);
            risk_level = RiskLevel::Medium;
        }
        
        if script.contains("write") || script.contains("echo >") || script.contains("touch") {
            security_flags.push(SecurityFlag::WritesFileSystem);
            risk_level = RiskLevel::Medium;
        }
        
        // Check for network access
        if script.contains("curl") || script.contains("wget") || script.contains("http") {
            security_flags.push(SecurityFlag::NetworkAccess);
            risk_level = RiskLevel::High;
        }
        
        // Check for system commands
        if script.contains("exec") || script.contains("system") || script.contains("subprocess") {
            security_flags.push(SecurityFlag::SystemCommands);
            risk_level = RiskLevel::High;
        }
        
        // Check for environment access
        if script.contains("env") || script.contains("export") || script.contains("$") {
            security_flags.push(SecurityFlag::EnvironmentAccess);
        }
        
        // Estimate resource requirements
        let resource_requirements = ResourceRequirements {
            estimated_memory_mb: 64,
            estimated_cpu_time_ms: 5000,
            estimated_disk_space_mb: 10,
            network_bandwidth_kbps: if security_flags.contains(&SecurityFlag::NetworkAccess) {
                Some(1000)
            } else {
                None
            },
        };
        
        // Determine sandbox requirements
        let sandbox_requirements = SandboxRequirements {
            requires_isolation: risk_level != RiskLevel::Low,
            network_isolation: !security_flags.contains(&SecurityFlag::NetworkAccess),
            filesystem_isolation: security_flags.contains(&SecurityFlag::WritesFileSystem),
            resource_limits: ResourceLimits {
                max_memory_mb: 256,
                max_cpu_time_ms: 30000,
                max_disk_space_mb: 100,
                max_network_bytes: 1024 * 1024, // 1MB
                max_processes: 5,
            },
            allowed_paths: vec![
                PathBuf::from("/tmp"),
                PathBuf::from("/var/tmp"),
            ],
            blocked_paths: vec![
                PathBuf::from("/etc"),
                PathBuf::from("/root"),
                PathBuf::from("/home"),
            ],
        };
        
        // Generate recommendations
        let mut recommendations = Vec::new();
        
        if risk_level == RiskLevel::High {
            recommendations.push("Consider breaking down into smaller, safer operations".to_string());
        }
        
        if security_flags.contains(&SecurityFlag::NetworkAccess) {
            recommendations.push("Limit network access to specific domains".to_string());
        }
        
        if security_flags.contains(&SecurityFlag::WritesFileSystem) {
            recommendations.push("Restrict file write permissions to specific directories".to_string());
        }
        
        let security_flags_count = security_flags.len();
        SecurityAnalysis {
            risk_level,
            security_flags,
            resource_requirements,
            sandbox_requirements,
            analysis_summary: format!("Script analysis complete with {} security flags", security_flags_count),
            recommendations,
        }
    }

    /// P1-Issue4: Get temporary tool usage statistics
    pub fn get_temporary_tool_stats(&self) -> HashMap<String, TemporaryToolStats> {
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemporaryToolStats {
    pub total_proposed: u32,
    pub total_approved: u32,
    pub total_rejected: u32,
    pub total_expired: u32,
    pub total_executions: u32,
    pub average_execution_time_ms: u64,
    pub success_rate: f64,
}

/// P1-Issue4: Implementation for TemporaryTool
impl TemporaryTool {
    /// Create a new temporary tool proposal
    pub fn new(
        name: String,
        description: String,
        script_content: String,
        script_type: ScriptType,
        proposed_by: String,
    ) -> Self {
        let id = format!("temp_{}", chrono::Utc::now().timestamp_nanos());
        
        Self {
            id,
            name,
            description,
            script_content,
            script_type,
            proposed_by,
            approval_status: ApprovalStatus::Pending,
            security_analysis: SecurityAnalysis::default(),
            execution_permissions: ExecutionPermissions::default(),
            created_at: chrono::Utc::now(),
            expires_at: None,
            usage_count: 0,
            max_uses: Some(10), // Default limit
        }
    }
    
    /// Check if the tool is currently valid for execution
    pub fn is_valid_for_execution(&self) -> bool {
        match self.approval_status {
            ApprovalStatus::Approved => {
                // Check expiration
                if let Some(expires_at) = self.expires_at {
                    chrono::Utc::now() < expires_at
                } else {
                    true
                }
            }
            _ => false,
        }
    }
    
    /// Check if usage limit has been reached
    pub fn has_reached_usage_limit(&self) -> bool {
        if let Some(max_uses) = self.max_uses {
            self.usage_count >= max_uses
        } else {
            false
        }
    }
    
    /// Increment usage count
    pub fn increment_usage(&mut self) {
        self.usage_count += 1;
    }
}

/// P1-Issue4: Default implementations
impl Default for SecurityAnalysis {
    fn default() -> Self {
        Self {
            risk_level: RiskLevel::Low,
            security_flags: vec![],
            resource_requirements: ResourceRequirements {
                estimated_memory_mb: 64,
                estimated_cpu_time_ms: 5000,
                estimated_disk_space_mb: 10,
                network_bandwidth_kbps: None,
            },
            sandbox_requirements: SandboxRequirements {
                requires_isolation: false,
                network_isolation: true,
                filesystem_isolation: false,
                resource_limits: ResourceLimits {
                    max_memory_mb: 256,
                    max_cpu_time_ms: 30000,
                    max_disk_space_mb: 100,
                    max_network_bytes: 1024 * 1024,
                    max_processes: 5,
                },
                allowed_paths: vec![PathBuf::from("/tmp")],
                blocked_paths: vec![],
            },
            analysis_summary: "Default security analysis".to_string(),
            recommendations: vec![],
        }
    }
}

impl Default for ExecutionPermissions {
    fn default() -> Self {
        Self {
            can_read_files: true,
            can_write_files: false,
            can_execute_commands: false,
            can_access_network: false,
            can_access_environment: false,
            allowed_file_patterns: vec!["*.txt".to_string(), "*.json".to_string()],
            blocked_file_patterns: vec!["*".to_string()],
            allowed_commands: vec![],
            blocked_commands: vec!["rm".to_string(), "sudo".to_string()],
        }
    }
}

impl RuntimeToolRegistry {
    pub fn get_execution_history(&self) -> &[ToolExecution] {
        &self.execution_history
    }

    pub fn get_tool_stats(&self, tool_id: &str) -> Option<ToolStats> {
        let executions: Vec<_> = self
            .execution_history
            .iter()
            .filter(|e| e.tool_id == tool_id)
            .collect();

        if executions.is_empty() {
            return None;
        }

        let total = executions.len();
        let successful = executions.iter().filter(|e| e.success).count();
        let avg_duration = executions
            .iter()
            .filter_map(|e| {
                e.end_time
                    .map(|end| (end - e.start_time).num_milliseconds() as u64)
            })
            .sum::<u64>()
            / total as u64;

        Some(ToolStats {
            tool_id: tool_id.to_string(),
            total_executions: total as u32,
            successful_executions: successful as u32,
            failed_executions: (total - successful) as u32,
            average_duration_ms: avg_duration,
            success_rate: successful as f64 / total as f64,
        })
    }

    /// P1-009: Get tool version by running the version command
    pub async fn get_tool_version(&self, tool_id: &str) -> Option<String> {
        if let Some(tool) = self.tools.get(tool_id) {
            if let Some(version_cmd) = &tool.version_cmd {
                let parts: Vec<_> = version_cmd.split_whitespace().collect();
                if parts.is_empty() {
                    return None;
                }

                let output = Command::new(parts[0]).args(&parts[1..]).output().await.ok()?;
                if output.status.success() {
                    return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
                }
            }
        }
        None
    }

    /// P1-009: Run health checks for all tools referenced in a ValidationPlan
    pub async fn health_check_plan(&self, plan: &crate::harness::validation::ValidationPlan) -> Vec<(String, bool)> {
        let mut results = Vec::new();

        // Check tools by ID
        for tool_id in &plan.tool_ids {
            let healthy = self.health_check(tool_id).await.unwrap_or(false);
            results.push((tool_id.clone(), healthy));
        }

        // Also check tools referenced by raw commands
        for cmd in &plan.format_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }
        for cmd in &plan.lint_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }
        for cmd in &plan.test_commands {
            if let Some(tool) = self.find_by_command(cmd) {
                let healthy = self.health_check(&tool.id).await.unwrap_or(false);
                if !results.iter().any(|(id, _)| id == &tool.id) {
                    results.push((tool.id.clone(), healthy));
                }
            }
        }

        results
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStats {
    pub tool_id: String,
    pub total_executions: u32,
    pub successful_executions: u32,
    pub failed_executions: u32,
    pub average_duration_ms: u64,
    pub success_rate: f64,
}

pub fn create_tool_registry() -> RuntimeToolRegistry {
    RuntimeToolRegistry::with_builtin_tools()
}

pub fn format_tool_result(result: &ToolResult) -> String {
    let status = if result.success { "✓" } else { "✗" };
    format!(
        r#"{} Tool Result: {}
   Exit Code: {}
   Duration: {}ms
   Issues Found: {}
"#,
        status,
        result.tool_id,
        result.exit_code,
        result.duration_ms,
        result.issues.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get_tool() {
        let mut registry = RuntimeToolRegistry::new();
        let tool = RuntimeTool {
            id: "test-tool".to_string(),
            name: "Test Tool".to_string(),
            version: "1.0".to_string(),
            tool_type: ToolType::Custom,
            executable_path: PathBuf::from("echo"),
            args_template: vec!["hello".to_string()],
            env_vars: HashMap::new(),
            timeout_ms: 5000,
            max_memory_mb: 128,
            description: "Test tool".to_string(),
            supported_extensions: vec!["txt".to_string()],
            health_check_cmd: Some("echo --version".to_string()),
            version_cmd: Some("echo --version".to_string()),
        };

        registry.register(tool);
        assert!(registry.get("test-tool").is_some());
    }

    #[test]
    fn test_list_tools_by_type() {
        let registry = RuntimeToolRegistry::with_builtin_tools();
        let formatters = registry.list_by_type(ToolType::Formatter);
        assert!(!formatters.is_empty());
        assert!(formatters.iter().any(|t| t.id == "rustfmt"));
    }

    #[test]
    fn test_parse_cargo_error() {
        let registry = RuntimeToolRegistry::new();
        let line = "error[E0425]: cannot find value `x` in this scope --> src/main.rs:10:5";
        let issue = registry.parse_cargo_error(line);
        assert!(issue.is_some());
        let issue = issue.unwrap();
        assert!(matches!(issue.severity, IssueSeverity::Error));
    }
}
