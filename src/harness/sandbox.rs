use crate::harness::validation::CommandResult;
use anyhow::{Result, bail};
use async_trait::async_trait;
use std::{path::Path, process::Stdio, time::Instant};
use tokio::{
    process::Command,
    time::{Duration, timeout},
};
#[async_trait]
pub trait SandboxRuntime: Send + Sync {
    async fn run_command(
        &self,
        repo_root: &Path,
        command: &str,
        timeout_ms: u64,
    ) -> Result<CommandResult>;
}
#[derive(Debug, Clone)]
pub struct LocalSandboxRuntime {
    allowed: Vec<String>,
}
impl Default for LocalSandboxRuntime {
    fn default() -> Self {
        Self {
            allowed: vec![
                "cargo",
                "npm",
                "pnpm",
                "yarn",
                "python",
                "go",
                "make",
                "git",
                "cmd",
                "powershell",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}
impl LocalSandboxRuntime {
    pub fn new(allowed: Vec<String>) -> Self {
        Self { allowed }
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
        let program = command
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;
        if !self.allowed.iter().any(|a| a == program) {
            bail!("command denied")
        };
        let start = Instant::now();
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        let child = cmd
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let out = match timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
            Ok(o) => o?,
            Err(_) => {
                return Ok(CommandResult {
                    command: command.into(),
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
            command: command.into(),
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
