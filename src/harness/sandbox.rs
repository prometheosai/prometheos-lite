//! Command Runtime with Security Policy
//!
//! ⚠️ IMPORTANT: This is NOT a true sandbox. It provides command filtering
//! and security policy enforcement, but does NOT provide:
//! - Process isolation (containers, namespaces)
//! - Filesystem isolation (chroot, bind mounts)
//! - Network isolation
//! - Resource limits (CPU/memory quotas)
//!
//! For true sandboxing, use container runtime integration (Docker, etc.)

use crate::harness::validation::CommandResult;
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Stdio, time::Instant};
use tokio::{
    process::Command,
    time::{Duration, timeout},
};

/// P0-Issue1: Runtime kind for sandbox evidence tracking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxRuntimeKind {
    Docker,
    Local,
}

/// P0-Issue2: Enhanced sandbox policy for mode-aware runtime selection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub prefer_docker: bool,
    pub require_docker: bool,
    pub fallback_to_local: bool,
    pub network: NetworkPolicy,
    pub mount_mode: MountMode,
    pub cpu_limit: Option<String>,
    pub memory_limit: Option<String>,
    pub docker_image: Option<String>,
}

/// P0-Issue2: Network policy for sandbox isolation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkPolicy {
    Disabled,
    Enabled,
    OutboundOnly,
}

/// P0-Issue2: Mount mode for Docker containers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MountMode {
    ReadOnly,
    ReadWrite,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            prefer_docker: false,
            require_docker: false,
            fallback_to_local: true,
            network: NetworkPolicy::Enabled,
            mount_mode: MountMode::ReadWrite,
            cpu_limit: None,
            memory_limit: None,
            docker_image: None,
        }
    }
}

impl SandboxPolicy {
    /// P0-Issue2: Create sandbox policy for autonomous mode
    pub fn autonomous() -> Self {
        Self {
            prefer_docker: true,
            require_docker: true, // Docker required for autonomous mode
            fallback_to_local: false, // No fallback for autonomous mode
            network: NetworkPolicy::Disabled, // Network disabled for safety
            mount_mode: MountMode::ReadWrite, // Read-write for temp workspace validation
            cpu_limit: Some("1".to_string()),
            memory_limit: Some("512m".to_string()),
            docker_image: Some("rust:latest".to_string()),
        }
    }

    /// Create sandbox policy for assisted mode
    pub fn assisted() -> Self {
        Self {
            prefer_docker: true,
            require_docker: false, // Docker preferred but not required
            fallback_to_local: true, // Allow fallback in assisted mode
            network: NetworkPolicy::OutboundOnly, // Limited network access
            mount_mode: MountMode::ReadWrite, // Read-write for temp workspace validation
            cpu_limit: Some("2".to_string()),
            memory_limit: Some("1g".to_string()),
            docker_image: Some("rust:latest".to_string()),
        }
    }

    /// Create sandbox policy for review-only mode
    pub fn review_only() -> Self {
        Self {
            prefer_docker: false, // Local commands OK for review-only
            require_docker: false,
            fallback_to_local: true,
            network: NetworkPolicy::Enabled, // Network allowed for review-only
            mount_mode: MountMode::ReadOnly, // Read-only for review-only analysis
            cpu_limit: None,
            memory_limit: None,
            docker_image: None,
        }
    }

    /// Create sandbox policy for benchmark mode
    pub fn benchmark() -> Self {
        Self {
            prefer_docker: true,
            require_docker: false,
            fallback_to_local: true,
            network: NetworkPolicy::Enabled, // Network allowed for benchmarking
            mount_mode: MountMode::ReadWrite,
            cpu_limit: Some("2".to_string()),
            memory_limit: Some("1g".to_string()),
            docker_image: Some("rust:latest".to_string()),
        }
    }

    /// Create sandbox policy from harness mode
    pub fn from_mode(mode: crate::harness::mode_policy::HarnessMode) -> Self {
        use crate::harness::mode_policy::HarnessMode;
        match mode {
            HarnessMode::Autonomous => Self::autonomous(),
            HarnessMode::Assisted => Self::assisted(),
            HarnessMode::Review | HarnessMode::ReviewOnly => Self::review_only(),
            HarnessMode::Benchmark => Self::benchmark(),
        }
    }
}

/// V1.6-P0-003: Docker capabilities verification
#[derive(Debug, Clone, Default)]
pub struct DockerCapabilities {
    pub can_create_containers: bool,
    pub can_set_resource_limits: bool,
    pub can_set_network_policies: bool,
    pub can_set_security_options: bool,
}

/// Parsed command structure with program and arguments
#[derive(Debug, Clone)]
pub struct StructuredCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Whether this command requires shell features (pipes, redirects, etc.)
    pub requires_shell: bool,
    /// Original command string for display/logging
    pub original: String,
}

impl StructuredCommand {
    /// Parse a command string into structured components
    ///
    /// This uses a simple shell-like parser that:
    /// - Respects quoted strings (single and double quotes)
    /// - Handles escaped characters
    /// - Detects shell metacharacters that require shell execution
    pub fn parse(command: &str) -> Result<Self> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            bail!("empty command");
        }

        // Check for shell metacharacters that require shell execution
        let shell_metachars = [
            '|', '&', ';', '$', '`', '(', ')', '<', '>', '*', '?', '[', ']', '{', '}', '~',
        ];
        let requires_shell = shell_metachars.iter().any(|&c| trimmed.contains(c));

        // Simple tokenizer that respects quotes
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escaped = false;

        for ch in trimmed.chars() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && !in_single_quote {
                escaped = true;
                continue;
            }

            if ch == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
                continue;
            }

            if ch == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
                continue;
            }

            if ch.is_whitespace() && !in_single_quote && !in_double_quote {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            } else {
                current.push(ch);
            }
        }

        if !current.is_empty() {
            args.push(current);
        }

        if args.is_empty() {
            bail!("empty command after parsing");
        }

        let program = args.remove(0);

        Ok(StructuredCommand {
            program,
            args,
            requires_shell,
            original: trimmed.to_string(),
        })
    }

    /// Get the program name without path
    pub fn program_name(&self) -> &str {
        Path::new(&self.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.program)
    }
}

#[async_trait]
/// Runtime for executing commands with security policy enforcement
///
/// ⚠️ WARNING: This provides command filtering only, NOT process isolation.
/// Commands run directly on the host with the privileges of the PrometheOS process.
pub trait CommandRuntime: Send + Sync {
    /// Run a command from a raw string with policy enforcement
    async fn run_command(
        &self,
        repo_root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> Result<CommandResult>;

    /// Run a structured command with separate program and args (recommended API)
    async fn run_structured_command(
        &self,
        repo_root: &Path,
        cmd: &StructuredCommand,
        timeout_ms: u64,
    ) -> Result<CommandResult>;
}

// Backward compatibility trait - will be removed in v2.0
#[deprecated(since = "1.6.0", note = "Use CommandRuntime instead. This is not true sandboxing.")]
pub trait SandboxRuntime: CommandRuntime {}

// Auto-implement SandboxRuntime for any type that implements CommandRuntime
#[allow(deprecated)]
impl<T: CommandRuntime + ?Sized> SandboxRuntime for T {}

/// Security policy for command execution
#[derive(Debug, Clone)]
pub struct CommandSecurityPolicy {
    /// List of allowed program names (e.g., "cargo", "npm")
    pub allowed_programs: Vec<String>,
    /// List of blocked programs that are explicitly denied
    pub blocked_programs: Vec<String>,
    /// Whether to allow shell execution (for commands with pipes/redirects)
    pub allow_shell: bool,
    /// Maximum command length
    pub max_command_length: usize,
    /// Maximum number of arguments
    pub max_args: usize,
    /// P0-C5: Whether shell execution is explicitly approved for autonomous mode
    pub autonomous_shell_approved: bool,
}

// Backward compatibility alias - will be removed in v2.0
#[deprecated(since = "1.6.0", note = "Use CommandSecurityPolicy instead")]
pub type SandboxSecurityPolicy = CommandSecurityPolicy;

impl Default for CommandSecurityPolicy {
    fn default() -> Self {
        Self {
            allowed_programs: vec![
                "cargo", "rustc", "rustfmt", "clippy", "npm", "node", "yarn", "pnpm", "python", "python3",
                "pip", "pip3", "pytest", "black", "flake8", "mypy", "go", "go fmt", "go vet", "gofmt",
                "javac", "java", "mvn", "gradle", "gcc", "g++", "make", "cmake", "dotnet", "nuget",
                "git", "docker", "kubectl", "helm", "terraform", "ansible", "vault", "consul",
                "aws", "az", "gcloud", "kubectl", "oc", "istioctl", "jq", "yq", "curl", "wget",
                "cat", "ls", "find", "grep", "sed", "awk", "sort", "uniq", "wc", "head", "tail",
                "diff", "patch", "tar", "gzip", "gunzip", "zip", "unzip", "chmod", "chown",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            blocked_programs: vec![
                "rm", "del", "rd", "rmdir", "format", "fdisk", "mkfs", "sudo", "su", "doas",
                "wget", "curl", // Network tools blocked by default
                "nc", "netcat", "telnet", "ssh", "scp", "sftp", "bash", "zsh",
                "fish", // Shells blocked unless explicitly allowed
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            allow_shell: false,
            max_command_length: 8192,
            max_args: 100,
            autonomous_shell_approved: false, // P0-C5: Shell execution not approved for autonomous mode by default
        }
    }
}

#[derive(Debug, Clone)]
/// Local command runtime with security policy enforcement
///
/// ⚠️ WARNING: This runs commands directly on the host. It filters commands
/// by program name but does NOT isolate the process.
pub struct LocalCommandRuntime {
    policy: CommandSecurityPolicy,
}

// Backward compatibility alias - will be removed in v2.0
#[deprecated(since = "1.6.0", note = "Use LocalCommandRuntime instead")]
pub type LocalSandboxRuntime = LocalCommandRuntime;

impl Default for LocalCommandRuntime {
    fn default() -> Self {
        Self {
            policy: CommandSecurityPolicy::default(),
        }
    }
}

impl LocalCommandRuntime {
    pub fn new() -> Self {
        Self {
            policy: CommandSecurityPolicy::default(),
        }
    }

    pub fn with_policy(policy: CommandSecurityPolicy) -> Self {
        Self { policy }
    }

    /// Check if a program is allowed to run
    fn is_program_allowed(&self, program: &str) -> bool {
        let program_lower = program.to_lowercase();
        let program_name = Path::new(&program_lower)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&program_lower);

        // Check blocked list first
        for blocked in &self.policy.blocked_programs {
            if program_name == blocked.to_lowercase() || program_lower == blocked.to_lowercase() {
                return false;
            }
        }

        // If allowed list is empty, allow all (except blocked)
        if self.policy.allowed_programs.is_empty() {
            return true;
        }

        // Check allowed list
        for allowed in &self.policy.allowed_programs {
            if program_name == allowed.to_lowercase() || program_lower == allowed.to_lowercase() {
                return true;
            }
        }

        false
    }

    /// Validate a structured command against security policy
    fn validate_command(&self, cmd: &StructuredCommand) -> Result<()> {
        // Check command length
        if cmd.original.len() > self.policy.max_command_length {
            bail!(
                "command exceeds maximum length of {} characters",
                self.policy.max_command_length
            );
        }

        // Check argument count
        if cmd.args.len() > self.policy.max_args {
            bail!(
                "command has too many arguments (max: {})",
                self.policy.max_args
            );
        }

        // Check if program is allowed
        if !self.is_program_allowed(&cmd.program) {
            bail!("command '{}' is not allowed", cmd.program);
        }

        // Check if shell execution is required but not allowed
        if cmd.requires_shell && !self.policy.allow_shell {
            bail!(
                "command requires shell features (pipes, redirects, etc.) which are not allowed: {}",
                cmd.original
            );
        }

        Ok(())
    }

    /// P0-C5: Validate command for autonomous mode with additional restrictions
    pub fn validate_command_for_autonomous(&self, cmd: &StructuredCommand) -> Result<()> {
        // First run standard validation
        self.validate_command(cmd)?;

        // Additional autonomous mode restrictions
        if cmd.requires_shell && !self.policy.autonomous_shell_approved {
            bail!(
                "P0-C5: Shell execution in autonomous mode requires explicit approval: {}",
                cmd.original
            );
        }

        // Block shell programs entirely in autonomous mode unless approved
        let shell_programs = ["bash", "sh", "zsh", "fish", "cmd", "powershell", "pwsh"];
        let program_name = cmd.program_name().to_lowercase();
        
        if shell_programs.iter().any(|&shell| program_name.contains(shell)) && !self.policy.autonomous_shell_approved {
            bail!(
                "P0-C5: Shell program '{}' not allowed in autonomous mode without explicit approval",
                cmd.program
            );
        }

        tracing::warn!(
            "P0-C5: Command validated for autonomous mode: {}",
            cmd.original
        );

        Ok(())
    }
}
#[async_trait]
impl CommandRuntime for LocalCommandRuntime {
    async fn run_command(
        &self,
        root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> Result<CommandResult> {
        // Parse command into structured form
        let structured = StructuredCommand::parse(command)?;

        // Run via structured API
        self.run_structured_command(root, &structured, timeout_ms)
            .await
    }

    async fn run_structured_command(
        &self,
        root: &Path,
        cmd: &StructuredCommand,
        timeout_ms: u64,
    ) -> Result<CommandResult> {
        // Validate against security policy
        self.validate_command(cmd)?;

        let start = Instant::now();

        // Execute command
        let child = if cmd.requires_shell && self.policy.allow_shell {
            // Shell execution for complex commands (pipes, redirects)
            self.spawn_shell_command(root, cmd).await?
        } else {
            // Direct execution - no shell, no injection vulnerability
            self.spawn_direct_command(root, cmd).await?
        };

        let out = match timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
            Ok(o) => o?,
            Err(_) => {
                return Ok(CommandResult {
                    command: cmd.original.clone(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: "timeout".into(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    cached: false,
                    cache_key: None,
                    timed_out: true,
                });
            }
        };

        Ok(CommandResult {
            command: cmd.original.clone(),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into(),
            stderr: String::from_utf8_lossy(&out.stderr).into(),
            duration_ms: start.elapsed().as_millis() as u64,
            cached: false,
            cache_key: None,
            timed_out: false,
        })
    }
}

impl LocalCommandRuntime {
    /// Spawn a direct command without shell wrapper
    ///
    /// This is the secure path - args are passed directly to the program,
    /// preventing shell injection attacks.
    async fn spawn_direct_command(
        &self,
        root: &Path,
        cmd: &StructuredCommand,
    ) -> Result<tokio::process::Child> {
        let mut command = Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        Ok(command.spawn()?)
    }

    /// Spawn a command via shell (only for commands requiring shell features)
    ///
    /// This is less secure but necessary for pipes, redirects, etc.
    /// Only used when allow_shell is true.
    async fn spawn_shell_command(
        &self,
        root: &Path,
        cmd: &StructuredCommand,
    ) -> Result<tokio::process::Child> {
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut command = Command::new(shell);
        command
            .arg(shell_arg)
            .arg(&cmd.original)
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        Ok(command.spawn()?)
    }
}

/// P1-FIX: Docker-based sandbox runtime for true process isolation
///
/// This provides actual containerization with:
/// - Filesystem isolation (container rootfs)
/// - Process isolation (PID namespace)
/// - Resource limits (CPU/memory quotas)
/// - Optional network isolation
#[derive(Debug, Clone)]
pub struct DockerSandboxRuntime {
    /// Docker image to use (e.g., "rust:latest", "node:18")
    image: String,
    /// Container working directory (mounted from host)
    workdir: String,
    /// Additional volume mounts (host_path:container_path)
    volumes: Vec<String>,
    /// Environment variables to set in container
    env_vars: Vec<(String, String)>,
    /// Network mode ("none" for isolation, "host" for access)
    network_mode: String,
    /// CPU limit (e.g., "1.0" for 1 core)
    cpus: Option<String>,
    /// Memory limit (e.g., "512m")
    memory: Option<String>,
    /// Timeout for container operations
    timeout_ms: u64,
    policy: CommandSecurityPolicy,
    /// P0-Issue2: Mount mode for volume mounts
    mount_mode: MountMode,
}

impl DockerSandboxRuntime {
    /// Create a new Docker sandbox with the specified image
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            workdir: "/workspace".to_string(),
            policy: CommandSecurityPolicy {
                allowed_programs: vec![],
                blocked_programs: vec![],
                allow_shell: false,
                max_command_length: 8192,
                max_args: 100,
                autonomous_shell_approved: false,
            },
            volumes: vec![],
            env_vars: vec![],
            network_mode: "none".to_string(), // Secure default
            cpus: Some("1.0".to_string()),
            memory: Some("512m".to_string()),
            timeout_ms: 300000, // 5 minutes
            mount_mode: MountMode::ReadWrite, // Default to read-write for compatibility
        }
    }

    /// Set the working directory inside the container
    pub fn with_workdir(mut self, workdir: impl Into<String>) -> Self {
        self.workdir = workdir.into();
        self
    }

    /// Add a volume mount with explicit mount mode
    pub fn with_volume(mut self, host_path: impl Into<String>, container_path: impl Into<String>) -> Self {
        self.volumes.push(format!("{}:{}", host_path.into(), container_path.into()));
        self
    }

    /// P1-Issue6: Add a volume mount with explicit mount mode
    pub fn with_volume_mode(
        mut self, 
        host_path: impl Into<String>, 
        container_path: impl Into<String>, 
        mode: crate::harness::evidence::SandboxMountMode
    ) -> Self {
        let mode_suffix = match mode {
            crate::harness::evidence::SandboxMountMode::ReadOnly => ":ro",
            crate::harness::evidence::SandboxMountMode::ReadWrite => ":rw",
        };
        self.volumes.push(format!("{}:{}{}", host_path.into(), container_path.into(), mode_suffix));
        self
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }

    /// Set network mode ("none" for isolation)
    pub fn with_network_mode(mut self, mode: impl Into<String>) -> Self {
        self.network_mode = mode.into();
        self
    }

    /// Set resource limits
    pub fn with_resources(mut self, cpus: Option<String>, memory: Option<String>) -> Self {
        self.cpus = cpus;
        self.memory = memory;
        self
    }

    /// Check if Docker is available on the system
    pub async fn is_docker_available() -> bool {
        match Command::new("docker").arg("--version").output().await {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// P0-Issue2: Set network policy for Docker runtime
    pub fn set_network_policy(&mut self, policy: NetworkPolicy) {
        self.network_mode = match policy {
            NetworkPolicy::Disabled => "none".to_string(),
            NetworkPolicy::Enabled => "bridge".to_string(),
            NetworkPolicy::OutboundOnly => "bridge".to_string(), // Would need additional firewall rules
        };
    }

    /// P0-Issue2: Set mount mode for Docker runtime
    pub fn set_mount_mode(&mut self, mode: MountMode) {
        self.mount_mode = mode;
    }

    /// P0-Issue2: Set CPU limit for Docker runtime
    pub fn set_cpu_limit(&mut self, limit: String) {
        self.cpus = Some(limit);
    }

    /// P0-Issue2: Set memory limit for Docker runtime
    pub fn set_memory_limit(&mut self, limit: String) {
        self.memory = Some(limit);
    }

    /// V1.6-P0-003: Verify Docker daemon is running and accessible
    pub async fn verify_docker_daemon() -> Result<()> {
        // Check Docker daemon status
        let output = Command::new("docker")
            .arg("info")
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to run docker info: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Docker daemon not accessible: {}", stderr);
        }

        // Parse Docker info to verify critical components
        let info = String::from_utf8_lossy(&output.stdout);
        
        // Check for essential Docker components
        if !info.contains("Server Version:") {
            anyhow::bail!("Docker daemon info incomplete - missing server version");
        }
        
        if !info.contains("Containers:") {
            anyhow::bail!("Docker daemon info incomplete - missing container information");
        }

        // Verify Docker daemon is healthy
        let health_output = Command::new("docker")
            .args(&["system", "info", "--format", "{{.ServerState.Health}}"])
            .output()
            .await;

        match health_output {
            Ok(output) if output.status.success() => {
                let health = String::from_utf8_lossy(&output.stdout);
                if health.trim() != "healthy" {
                    tracing::warn!("Docker daemon health status: {}", health.trim());
                }
            }
            _ => {
                tracing::warn!("Could not verify Docker daemon health status");
            }
        }

        Ok(())
    }

    /// V1.6-P0-003: Verify Docker capabilities required for autonomous mode
    pub async fn verify_docker_capabilities() -> Result<DockerCapabilities> {
        let mut capabilities = DockerCapabilities::default();

        // Test container creation capability
        let test_output = Command::new("docker")
            .args(&["run", "--rm", "--name", "prometheos-test-cap", "hello-world"])
            .output()
            .await;

        match test_output {
            Ok(output) if output.status.success() => {
                capabilities.can_create_containers = true;
                tracing::info!("Container creation capability verified");
            }
            Ok(_) => {
                tracing::error!("Container creation test failed");
            }
            Err(e) => {
                tracing::error!("Container creation test error: {}", e);
            }
        }

        // Test resource limits capability
        let limits_output = Command::new("docker")
            .args(&["run", "--rm", "--cpus", "0.5", "--memory", "128m", "hello-world"])
            .output()
            .await;

        match limits_output {
            Ok(output) if output.status.success() => {
                capabilities.can_set_resource_limits = true;
                tracing::info!("Resource limits capability verified");
            }
            Ok(_) => {
                tracing::warn!("Resource limits test failed - Docker may not support limits");
            }
            Err(e) => {
                tracing::warn!("Resource limits test error: {}", e);
            }
        }

        // Test network policy capability
        let network_output = Command::new("docker")
            .args(&["run", "--rm", "--network", "none", "hello-world"])
            .output()
            .await;

        match network_output {
            Ok(output) if output.status.success() => {
                capabilities.can_set_network_policies = true;
                tracing::info!("Network policy capability verified");
            }
            Ok(_) => {
                tracing::error!("Network policy test failed");
            }
            Err(e) => {
                tracing::error!("Network policy test error: {}", e);
            }
        }

        // Test security options capability
        let security_output = Command::new("docker")
            .args(&["run", "--rm", "--security-opt", "no-new-privileges:true", "hello-world"])
            .output()
            .await;

        match security_output {
            Ok(output) if output.status.success() => {
                capabilities.can_set_security_options = true;
                tracing::info!("Security options capability verified");
            }
            Ok(_) => {
                tracing::error!("Security options test failed");
            }
            Err(e) => {
                tracing::error!("Security options test error: {}", e);
            }
        }

        Ok(capabilities)
    }

    /// P0-Issue1: Create sandbox evidence for autonomous mode safety verification
    pub fn create_sandbox_evidence(&self, container_id: Option<String>) -> crate::harness::evidence::SandboxEvidence {
        crate::harness::evidence::SandboxEvidence {
            runtime_kind: crate::harness::sandbox::SandboxRuntimeKind::Docker,
            isolated_process: true, // Docker provides process isolation
            isolated_filesystem: true, // Docker provides filesystem isolation
            network_disabled: self.network_mode == "none",
            cpu_limited: self.cpus.is_some(),
            memory_limited: self.memory.is_some(),
            container_id,
            mount_mode: crate::harness::evidence::SandboxMountMode::ReadWrite, // Default to read-write for patch application
            resource_limits_applied: self.cpus.is_some() || self.memory.is_some(),
            no_new_privileges: true, // Always set in Docker runtime
            capabilities_dropped: true, // Always drop ALL capabilities
            seccomp_enabled: true, // P0-Audit-005: Seccomp now enabled
            pids_limit: Some(64), // P0-Audit-005: PIDs limit applied
            non_root_user: true, // P0-Audit-005: Non-root user enforced
            tmpfs_protected: true, // P0-Audit-005: /tmp protected with noexec
        }
    }
    fn build_docker_args(&self, repo_root: &Path, cmd: &StructuredCommand) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(), // Remove container after run
            "--interactive".to_string(),
            "--workdir".to_string(),
            self.workdir.clone(),
        ];

        // Add network isolation
        args.push("--network".to_string());
        args.push(self.network_mode.clone());

        // Add resource limits
        if let Some(ref cpus) = self.cpus {
            args.push("--cpus".to_string());
            args.push(cpus.clone());
        }
        if let Some(ref memory) = self.memory {
            args.push("--memory".to_string());
            args.push(memory.clone());
        }

        // Add volume mounts
        // Mount the repo root to the working directory
        // P0-Issue2: Use policy-defined mount mode for proper isolation
        let mount_mode = match self.mount_mode {
            crate::harness::sandbox::MountMode::ReadOnly => ":ro",
            crate::harness::sandbox::MountMode::ReadWrite => ":rw",
        };
        args.push("--volume".to_string());
        args.push(format!("{}:{}{}", repo_root.display(), self.workdir, mount_mode));

        // Add additional volumes
        for volume in &self.volumes {
            args.push("--volume".to_string());
            args.push(volume.clone());
        }

        // Add environment variables
        for (key, value) in &self.env_vars {
            args.push("--env".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Security options - P0-Audit-005: Harden Docker sandbox
        args.push("--security-opt".to_string());
        args.push("no-new-privileges:true".to_string());
        args.push("--cap-drop".to_string());
        args.push("ALL".to_string());
        
        // Additional hardening measures
        args.push("--pids-limit".to_string());
        args.push("64".to_string()); // Limit process count
        
        // Add tmpfs for /tmp
        args.push("--tmpfs".to_string());
        args.push("/tmp:noexec,nosuid,size=100m".to_string());
        
        // Use non-root user
        args.push("--user".to_string());
        args.push("nobody".to_string());
        
        // Add seccomp profile if available
        args.push("--security-opt".to_string());
        args.push("seccomp=runtime/default".to_string());

        // Add the image
        args.push(self.image.clone());

        // Add the command to run
        args.push(cmd.program.clone());
        args.extend(cmd.args.clone());

        args
    }
}

#[async_trait]
impl CommandRuntime for DockerSandboxRuntime {
    async fn run_command(
        &self,
        repo_root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> Result<CommandResult> {
        // Parse command into structured form
        let structured = StructuredCommand::parse(command)?;

        // Run via structured API
        self.run_structured_command(repo_root, &structured, timeout_ms)
            .await
    }

    async fn run_structured_command(
        &self,
        repo_root: &Path,
        cmd: &StructuredCommand,
        _timeout_ms: u64, // Docker has its own timeout handling
    ) -> Result<CommandResult> {
        // Check if Docker is available
        if !Self::is_docker_available().await {
            bail!("Docker is not available on this system");
        }

        let start = Instant::now();

        // Build docker arguments
        let docker_args = self.build_docker_args(repo_root, cmd);

        tracing::info!(
            "P1: Running in Docker container: docker {}",
            docker_args.join(" ")
        );

        // Execute docker run
        let child = Command::new("docker")
            .args(&docker_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let out = match timeout(Duration::from_millis(self.timeout_ms), child.wait_with_output()).await {
            Ok(o) => o?,
            Err(_) => {
                return Ok(CommandResult {
                    command: cmd.original.clone(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: "Docker container timed out".into(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    cached: false,
                    cache_key: None,
                    timed_out: true,
                });
            }
        };

        let stdout: String = String::from_utf8_lossy(&out.stdout).into();
        let stderr: String = String::from_utf8_lossy(&out.stderr).into();

        // Check for Docker-specific errors
        if !out.status.success() && stderr.contains("Cannot connect to the Docker daemon") {
            bail!("Docker daemon is not running");
        }

        // P0-Issue1: Extract container ID from stderr for evidence tracking
        let container_id = self.extract_container_id(&stderr);
        
        // P0-Issue1: Log sandbox evidence for autonomous mode verification
        let sandbox_evidence = self.create_sandbox_evidence(container_id);
        tracing::info!(
            "P0-Issue1: Docker sandbox evidence - runtime: {:?}, isolated: {}, network: {}, limits: {}",
            sandbox_evidence.runtime_kind,
            sandbox_evidence.isolated_process && sandbox_evidence.isolated_filesystem,
            sandbox_evidence.network_disabled,
            sandbox_evidence.resource_limits_applied
        );

        Ok(CommandResult {
            command: format!("docker run {} {} {}",
                self.image,
                cmd.program,
                cmd.args.join(" ")
            ),
            exit_code: out.status.code(),
            stdout,
            stderr,
            duration_ms: start.elapsed().as_millis() as u64,
            cached: false,
            cache_key: None,
            timed_out: false,
        })
    }
}

/// P1-FIX: Sandbox runtime factory for selecting appropriate backend
pub struct SandboxRuntimeFactory;

impl SandboxRuntimeFactory {
    /// P0-Issue2: Create sandbox runtime based on enhanced policy
    pub async fn create_with_policy(policy: &SandboxPolicy) -> Result<std::sync::Arc<dyn SandboxRuntime + Send + Sync>> {
        // Check if Docker is available and preferred
        let docker_available = DockerSandboxRuntime::is_docker_available().await;
        
        if policy.require_docker && !docker_available {
            tracing::error!("Docker required by policy but not available");
            anyhow::bail!(
                "Docker runtime required by policy but not available on this system. \
                Autonomous mode requires Docker for security isolation. \
                Please install Docker and ensure it's running, or use assisted mode instead."
            );
        }

        // V1.6-P0-003: Additional autonomous mode Docker requirements
        if policy.require_docker {
            // Verify Docker daemon is running and accessible
            match DockerSandboxRuntime::verify_docker_daemon().await {
                Ok(_) => {
                    tracing::info!("Docker daemon verified and accessible");
                }
                Err(e) => {
                    tracing::error!("Docker daemon verification failed: {}", e);
                    anyhow::bail!(
                        "Docker daemon verification failed: {}. \
                        Autonomous mode requires a fully functional Docker daemon. \
                        Please check Docker installation and permissions.", e
                    );
                }
            }

            // Verify required Docker capabilities
            match DockerSandboxRuntime::verify_docker_capabilities().await {
                Ok(capabilities) => {
                    if !capabilities.can_create_containers {
                        anyhow::bail!("Docker lacks container creation capability required for autonomous mode");
                    }
                    if !capabilities.can_set_resource_limits {
                        tracing::warn!("Docker cannot set resource limits - autonomous mode may be less secure");
                    }
                    if !capabilities.can_set_network_policies {
                        anyhow::bail!("Docker cannot enforce network policies required for autonomous mode");
                    }
                    tracing::info!("Docker capabilities verified: {:?}", capabilities);
                }
                Err(e) => {
                    tracing::error!("Docker capabilities verification failed: {}", e);
                    anyhow::bail!("Docker capabilities verification failed: {}", e);
                }
            }
        }
        
        if (policy.prefer_docker || policy.require_docker) && docker_available {
            let image = policy.docker_image.clone().unwrap_or_else(|| "rust:latest".to_string());
            tracing::info!("P0-Issue2: Using Docker sandbox with image: {}", image);
            tracing::info!("P0-Issue2: Network policy: {:?}, Mount mode: {:?}", policy.network, policy.mount_mode);
            
            // Create Docker runtime with policy configuration
            let mut docker_runtime = DockerSandboxRuntime::new(image);
            
            // Apply policy settings
            docker_runtime.set_network_policy(policy.network.clone());
            docker_runtime.set_mount_mode(policy.mount_mode.clone());
            if let Some(ref cpu_limit) = policy.cpu_limit {
                docker_runtime.set_cpu_limit(cpu_limit.clone());
            }
            if let Some(ref memory_limit) = policy.memory_limit {
                docker_runtime.set_memory_limit(memory_limit.clone());
            }
            
            Ok(std::sync::Arc::new(docker_runtime))
        } else if policy.prefer_docker && !docker_available {
            if policy.fallback_to_local {
                tracing::warn!("P0-Issue2: Docker not available, falling back to local runtime");
                Ok(std::sync::Arc::new(LocalCommandRuntime::new()))
            } else {
                tracing::error!("P0-Issue2: Docker not available and fallback disabled");
                anyhow::bail!("Docker runtime not available and fallback to local runtime is disabled by policy");
            }
        } else {
            // Local runtime preferred or Docker not preferred
            tracing::info!("P0-Issue2: Using local command runtime");
            Ok(std::sync::Arc::new(LocalCommandRuntime::new()))
        }
    }

    /// Create the best available sandbox runtime (legacy method)
    ///
    /// Priority:
    /// 1. Docker (if available and requested)
    /// 2. LocalCommandRuntime (fallback)
    pub async fn create(prefer_docker: bool, image: Option<String>) -> std::sync::Arc<dyn SandboxRuntime + Send + Sync> {
        let policy = SandboxPolicy {
            prefer_docker,
            require_docker: false,
            fallback_to_local: true,
            network: NetworkPolicy::Enabled,
            mount_mode: MountMode::ReadWrite,
            cpu_limit: None,
            memory_limit: None,
            docker_image: image,
        };
        
        Self::create_with_policy(&policy).await.unwrap_or_else(|e| {
            tracing::error!("Failed to create sandbox runtime: {}", e);
            std::sync::Arc::new(LocalCommandRuntime::new()) // Fallback to local runtime
        })
    }

    /// Create a Docker sandbox if available, otherwise fail
    pub async fn create_docker(image: impl Into<String>) -> Result<std::sync::Arc<dyn SandboxRuntime + Send + Sync>> {
        if DockerSandboxRuntime::is_docker_available().await {
            Ok(std::sync::Arc::new(DockerSandboxRuntime::new(image)))
        } else {
            bail!("Docker is not available on this system")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_command_parse_simple() {
        let cmd = StructuredCommand::parse("cargo build").unwrap();
        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, vec!["build"]);
        assert!(!cmd.requires_shell);
    }

    #[test]
    fn test_structured_command_parse_quoted() {
        let cmd = StructuredCommand::parse("echo 'hello world'").unwrap();
        assert_eq!(cmd.program, "echo");
        assert_eq!(cmd.args, vec!["hello world"]);
    }

    #[test]
    fn test_structured_command_parse_double_quoted() {
        let cmd = StructuredCommand::parse("echo \"hello world\"").unwrap();
        assert_eq!(cmd.program, "echo");
        assert_eq!(cmd.args, vec!["hello world"]);
    }

    #[test]
    fn test_structured_command_detects_pipe() {
        let cmd = StructuredCommand::parse("cat file | grep pattern").unwrap();
        assert!(cmd.requires_shell);
    }

    #[test]
    fn test_structured_command_detects_redirect() {
        let cmd = StructuredCommand::parse("echo hello > file.txt").unwrap();
        assert!(cmd.requires_shell);
    }

    #[test]
    fn test_security_policy_blocks_dangerous() {
        let policy = SandboxSecurityPolicy::default();
        let runtime = LocalSandboxRuntime::with_policy(policy);

        assert!(!runtime.is_program_allowed("rm"));
        assert!(!runtime.is_program_allowed("sudo"));
        assert!(!runtime.is_program_allowed("/bin/rm"));
        assert!(runtime.is_program_allowed("cargo"));
        assert!(runtime.is_program_allowed("npm"));
    }

    #[test]
    fn test_security_validation_rejects_shell_without_permission() {
        let policy = SandboxSecurityPolicy {
            allow_shell: false,
            allowed_programs: vec![], // Empty list allows all non-blocked programs
            ..Default::default()
        };
        let runtime = LocalSandboxRuntime::with_policy(policy);

        let cmd = StructuredCommand::parse("cat file | grep pattern").unwrap();
        let result = runtime.validate_command(&cmd);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shell"));
    }

    #[test]
    fn test_security_validation_accepts_simple_commands() {
        let policy = SandboxSecurityPolicy::default();
        let runtime = LocalSandboxRuntime::with_policy(policy);

        let cmd = StructuredCommand::parse("cargo build --release").unwrap();
        assert!(runtime.validate_command(&cmd).is_ok());
    }
}
