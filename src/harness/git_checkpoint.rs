use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitCheckpoint {
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub diff: String,
    pub committed: bool,
    pub commit_message: Option<String>,
}
pub async fn create_pre_task_checkpoint(root: &Path) -> Result<GitCheckpoint> {
    let out = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(root)
        .output()
        .ok();
    Ok(GitCheckpoint {
        before_hash: out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()),
        after_hash: None,
        diff: String::new(),
        committed: false,
        commit_message: None,
    })
}
pub async fn commit_success(root: &Path, message: &str) -> Result<GitCheckpoint> {
    Ok(GitCheckpoint {
        commit_message: Some(message.into()),
        ..create_pre_task_checkpoint(root).await?
    })
}
pub async fn rollback_to_checkpoint(_: &Path, _: &GitCheckpoint) -> Result<()> {
    bail!("automatic rollback disabled")
}
