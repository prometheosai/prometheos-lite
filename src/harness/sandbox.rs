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
        let shell_metachars = ['|', '&', ';', '$', '`', '(', ')', '<', '>', '*', '?', '[', ']', '{', '}', '~'];
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
pub trait SandboxRuntime: Send + Sync {
    /// Run a command from a raw string (legacy API, now secure)
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
/// Security policy for command execution
#[derive(Debug, Clone)]
pub struct SandboxSecurityPolicy {
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
}

impl Default for SandboxSecurityPolicy {
    fn default() -> Self {
        Self {
            allowed_programs: vec![
                "cargo", "npm", "pnpm", "yarn", "python", "python3",
                "go", "make", "git", "node", "deno", "bun",
                "rustc", "clang", "gcc", "g++",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            blocked_programs: vec![
                "rm", "del", "rd", "rmdir",
                "format", "fdisk", "mkfs",
                "sudo", "su", "doas",
                "wget", "curl", // Network tools blocked by default
                "nc", "netcat", "telnet",
                "ssh", "scp", "sftp",
                "bash", "zsh", "fish", // Shells blocked unless explicitly allowed
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            allow_shell: false,
            max_command_length: 8192,
            max_args: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalSandboxRuntime {
    policy: SandboxSecurityPolicy,
}

impl Default for LocalSandboxRuntime {
    fn default() -> Self {
        Self {
            policy: SandboxSecurityPolicy::default(),
        }
    }
}

impl LocalSandboxRuntime {
    pub fn new(allowed: Vec<String>) -> Self {
        Self {
            policy: SandboxSecurityPolicy {
                allowed_programs: allowed,
                ..Default::default()
            },
        }
    }
    
    pub fn with_policy(policy: SandboxSecurityPolicy) -> Self {
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
}
#[async_trait]
impl SandboxRuntime for LocalSandboxRuntime {
    async fn run_command(
        &self,
        root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> Result<CommandResult> {
        // Parse command into structured form
        let structured = StructuredCommand::parse(command)?;
        
        // Run via structured API
        self.run_structured_command(root, &structured, timeout_ms).await
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

impl LocalSandboxRuntime {
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
