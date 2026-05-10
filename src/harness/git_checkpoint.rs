use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitCheckpoint {
    pub id: String,
    pub work_context_id: String,
    pub branch_name: String,
    pub before_head: Option<String>,
    pub after_head: Option<String>,
    pub dirty_files: Vec<PathBuf>,
    pub touched_files: Vec<PathBuf>,
    pub diff_before: String,
    pub diff_after: String,
    pub committed: bool,
    pub commit_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct GitCheckpointManager {
    repo_root: PathBuf,
    branch_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointResult {
    pub success: bool,
    pub checkpoint: Option<GitCheckpoint>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RollbackStrategy {
    HardReset,
    SoftReset,
    RevertCommit,
    StashAndReset,
}

impl GitCheckpointManager {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            branch_prefix: "harness".to_string(),
        }
    }

    pub fn with_prefix(repo_root: PathBuf, prefix: String) -> Self {
        Self {
            repo_root,
            branch_prefix: prefix,
        }
    }

    pub fn is_git_repo(&self) -> bool {
        let top = match self.run_git(&["rev-parse", "--show-toplevel"]) {
            Ok(t) => t,
            Err(_) => return false,
        };
        let reported = std::path::PathBuf::from(top.trim());
        let reported = reported.canonicalize().unwrap_or(reported);
        let root = self
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| self.repo_root.clone());
        reported == root
    }

    pub async fn create_checkpoint(&self, work_context_id: &str) -> Result<GitCheckpoint> {
        // Check if this is a git repo
        if !self.is_git_repo() {
            bail!("Not a git repository: {}", self.repo_root.display());
        }

        // Get current HEAD
        let before_head = self.get_head_hash()?;

        // Check for dirty state
        let dirty_files = self.get_dirty_files()?;

        // Get diff of dirty files
        let diff_before = self.get_diff()?;

        // Create branch name
        let timestamp = Utc::now().timestamp();
        let branch_name = format!("{}-{}-{}", self.branch_prefix, work_context_id, timestamp);

        // Create new branch
        self.run_git(&["checkout", "-b", &branch_name])
            .context("Failed to create checkpoint branch")?;

        let id = format!(
            "{}-{}-{}",
            work_context_id,
            timestamp,
            before_head.as_deref().unwrap_or("unknown")
        );

        Ok(GitCheckpoint {
            id,
            work_context_id: work_context_id.to_string(),
            branch_name,
            before_head,
            after_head: None,
            dirty_files,
            touched_files: vec![],
            diff_before,
            diff_after: String::new(),
            committed: false,
            commit_message: None,
            created_at: Utc::now(),
        })
    }

    pub async fn commit_changes(
        &self,
        checkpoint: &mut GitCheckpoint,
        message: &str,
    ) -> Result<()> {
        // Stage only touched files (not unrelated dirty files)
        for file in &checkpoint.touched_files {
            if file.exists() {
                self.run_git(&["add", &file.to_string_lossy()])
                    .context(format!("Failed to stage {}", file.display()))?;
            }
        }

        // Commit
        let full_message = format!("[harness-{}] {}", checkpoint.work_context_id, message);
        self.run_git(&["commit", "-m", &full_message])
            .context("Failed to commit changes")?;

        // Get new HEAD
        checkpoint.after_head = self.get_head_hash()?;
        checkpoint.commit_message = Some(message.to_string());
        checkpoint.committed = true;

        Ok(())
    }

    pub async fn rollback(
        &self,
        checkpoint: &GitCheckpoint,
        strategy: RollbackStrategy,
    ) -> Result<()> {
        match strategy {
            RollbackStrategy::HardReset => {
                // Switch to original branch and delete checkpoint branch
                if let Some(ref before_head) = checkpoint.before_head {
                    self.run_git(&["checkout", before_head])?;
                    self.run_git(&["branch", "-D", &checkpoint.branch_name])?;
                }
            }
            RollbackStrategy::SoftReset => {
                // Just switch back without deleting
                if let Some(ref before_head) = checkpoint.before_head {
                    self.run_git(&["checkout", before_head])?;
                }
            }
            RollbackStrategy::RevertCommit => {
                if checkpoint.committed {
                    // Revert the commit
                    if let Some(ref after_head) = checkpoint.after_head {
                        self.run_git(&["revert", "--no-commit", after_head])?;
                        self.run_git(&[
                            "commit",
                            "-m",
                            &format!("Revert harness checkpoint {}", checkpoint.id),
                        ])?;
                    }
                }
                // Switch back to original
                if let Some(ref before_head) = checkpoint.before_head {
                    self.run_git(&["checkout", before_head])?;
                }
            }
            RollbackStrategy::StashAndReset => {
                // Stash any current changes
                self.run_git(&["stash", "push", "-m", "harness-rollback-stash"])?;
                // Reset to before state
                if let Some(ref before_head) = checkpoint.before_head {
                    self.run_git(&["checkout", before_head])?;
                }
            }
        }

        Ok(())
    }

    pub fn get_current_branch(&self) -> Result<String> {
        let output = self.run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        Ok(output.trim().to_string())
    }

    pub fn get_head_hash(&self) -> Result<Option<String>> {
        match self.run_git(&["rev-parse", "HEAD"]) {
            Ok(output) => Ok(Some(output.trim().to_string())),
            Err(_) => Ok(None),
        }
    }

    pub fn get_dirty_files(&self) -> Result<Vec<PathBuf>> {
        let output = self.run_git(&["status", "--porcelain"])?;
        let mut files = vec![];

        for line in output.lines() {
            if line.len() > 3 {
                let file = &line[3..]; // Skip status markers
                files.push(PathBuf::from(file));
            }
        }

        Ok(files)
    }

    pub fn get_diff(&self) -> Result<String> {
        self.run_git(&["diff", "HEAD"])
    }

    pub fn get_diff_for_file(&self, file: &Path) -> Result<String> {
        self.run_git(&["diff", "HEAD", "--", &file.to_string_lossy()])
    }

    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        let output = self.run_git(&["status", "--porcelain"])?;
        Ok(!output.trim().is_empty())
    }

    pub fn stage_file(&self, file: &Path) -> Result<()> {
        self.run_git(&["add", &file.to_string_lossy()]).map(|_| ())
    }

    pub fn unstage_file(&self, file: &Path) -> Result<()> {
        self.run_git(&["reset", "HEAD", &file.to_string_lossy()])
            .map(|_| ())
    }

    fn run_git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git command")?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Git command failed: {}", stderr)
        }
    }
}

// Legacy API compatibility
pub async fn create_pre_task_checkpoint(root: &Path) -> Result<GitCheckpoint> {
    let manager = GitCheckpointManager::new(root.to_path_buf());
    manager.create_checkpoint("legacy").await
}

pub async fn commit_success(root: &Path, message: &str) -> Result<GitCheckpoint> {
    let manager = GitCheckpointManager::new(root.to_path_buf());
    let mut checkpoint = manager.create_checkpoint("success").await?;
    manager.commit_changes(&mut checkpoint, message).await?;
    Ok(checkpoint)
}

pub async fn rollback_to_checkpoint(root: &Path, checkpoint: &GitCheckpoint) -> Result<()> {
    let manager = GitCheckpointManager::new(root.to_path_buf());
    manager
        .rollback(checkpoint, RollbackStrategy::HardReset)
        .await
}

pub fn is_git_repo(root: &Path) -> bool {
    let manager = GitCheckpointManager::new(root.to_path_buf());
    manager.is_git_repo()
}

pub fn get_repo_info(root: &Path) -> Result<RepoInfo> {
    let manager = GitCheckpointManager::new(root.to_path_buf());

    Ok(RepoInfo {
        is_git_repo: manager.is_git_repo(),
        current_branch: manager.get_current_branch().ok(),
        head_hash: manager.get_head_hash()?.unwrap_or_default(),
        has_uncommitted_changes: manager.has_uncommitted_changes()?,
        dirty_files: manager.get_dirty_files()?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub is_git_repo: bool,
    pub current_branch: Option<String>,
    pub head_hash: String,
    pub has_uncommitted_changes: bool,
    pub dirty_files: Vec<PathBuf>,
}
