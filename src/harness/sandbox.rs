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
use std::{path::Path, process::Stdio, time::Instant};
use tokio::{
    process::Command,
    time::{Duration, timeout},
};

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
        }
    }

    /// Set the working directory inside the container
    pub fn with_workdir(mut self, workdir: impl Into<String>) -> Self {
        self.workdir = workdir.into();
        self
    }

    /// Add a volume mount
    pub fn with_volume(mut self, host_path: impl Into<String>, container_path: impl Into<String>) -> Self {
        self.volumes.push(format!("{}:{}", host_path.into(), container_path.into()));
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

    /// Build docker run command arguments
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
        // P0-C2: Support read-only mounts for validation scenarios
        args.push("--volume".to_string());
        args.push(format!("{}:{}", repo_root.display(), self.workdir));

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

        // Security options
        args.push("--security-opt".to_string());
        args.push("no-new-privileges:true".to_string());
        args.push("--cap-drop".to_string());
        args.push("ALL".to_string());

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
    /// Create the best available sandbox runtime
    ///
    /// Priority:
    /// 1. Docker (if available and requested)
    /// 2. LocalCommandRuntime (fallback)
    pub async fn create(prefer_docker: bool, image: Option<String>) -> std::sync::Arc<dyn SandboxRuntime + Send + Sync> {
        if prefer_docker && DockerSandboxRuntime::is_docker_available().await {
            let image = image.unwrap_or_else(|| "rust:latest".to_string());
            tracing::info!("P1: Using Docker sandbox with image: {}", image);
            std::sync::Arc::new(DockerSandboxRuntime::new(image))
        } else {
            tracing::info!("P1: Using local command runtime");
            std::sync::Arc::new(LocalCommandRuntime::new())
        }
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
