use crate::harness::sandbox::SandboxRuntime;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ValidationPlan {
    pub format_commands: Vec<String>,
    pub lint_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub repro_commands: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationResult {
    pub passed: bool,
    pub command_results: Vec<CommandResult>,
    pub errors: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}
pub async fn run_validation(
    root: &Path,
    plan: &ValidationPlan,
    sandbox: &dyn SandboxRuntime,
) -> anyhow::Result<ValidationResult> {
    let mut rs = Vec::new();
    let mut errors = Vec::new();
    for c in plan
        .format_commands
        .iter()
        .chain(plan.lint_commands.iter())
        .chain(plan.test_commands.iter())
        .chain(plan.repro_commands.iter())
    {
        let r = sandbox.run_command(root, c, 120000).await?;
        if r.exit_code != Some(0) {
            errors.push(c.clone())
        }
        rs.push(r)
    }
    Ok(ValidationResult {
        passed: errors.is_empty(),
        command_results: rs,
        errors,
    })
}
