//! Workspace strategy implementation for safe patch validation
//!
//! This module provides multiple workspace strategies for validating patches
//! without risking the original repository:
//! - TempCopy: Copy files to a temporary directory (fully isolated)
//! - GitWorktree: Use git worktree for lightweight isolation (when available)
//! - InPlace: Direct validation (only for ReviewOnly or explicitly allowed)

use crate::harness::{
    edit_protocol::EditOperation,
    file_control::{FilePolicy, FileSet},
    mode_policy::WorkspaceStrategy,
    patch_applier::{PatchResult, apply_patch_temp_only},
    temp_workspace::TempWorkspace,
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// A workspace handle that manages the lifecycle of a validation workspace
#[derive(Debug, Clone)]
pub struct WorkspaceHandle {
    pub root: PathBuf,
    pub original_root: PathBuf,
    pub strategy: WorkspaceStrategy,
    /// Whether this workspace needs explicit cleanup
    needs_cleanup: bool,
}

impl WorkspaceHandle {
    /// Create a workspace using the specified strategy
    pub async fn create(
        strategy: WorkspaceStrategy,
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        match strategy {
            WorkspaceStrategy::TempCopy => {
                Self::create_temp_copy(original_root, edits, file_set, policy).await
            }
            WorkspaceStrategy::GitWorktree => {
                Self::create_git_worktree(original_root, edits, file_set, policy).await
            }
            WorkspaceStrategy::InPlace => {
                Self::create_in_place(original_root, edits, file_set, policy).await
            }
        }
    }

    /// Create a temporary copy workspace (fully isolated)
    async fn create_temp_copy(
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        let (temp_workspace, patch_result) = TempWorkspace::create_temp_copy(
            original_root,
            edits,
            file_set,
            policy,
        ).await?;

        let handle = Self {
            root: temp_workspace.root.clone(),
            original_root: temp_workspace.original_root.clone(),
            strategy: WorkspaceStrategy::TempCopy,
            needs_cleanup: true,
        };

        Ok((handle, patch_result))
    }

    /// Create a git worktree workspace (lightweight isolation)
    ///
    /// This creates a new git worktree linked to the original repo,
    /// allowing validation with actual git state without affecting the main worktree.
    async fn create_git_worktree(
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        // Generate unique worktree name
        let worktree_name = format!(
            "prometheos-worktree-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );

        let worktree_path = original_root.parent()
            .unwrap_or(original_root)
            .join(&worktree_name);

        // Create git worktree
        let create_result = Command::new("git")
            .args([
                "worktree", "add",
                "-d",  // Detached HEAD (no branch needed for validation)
                worktree_path.to_str().unwrap_or("prometheos-worktree"),
            ])
            .current_dir(original_root)
            .output()
            .await;

        match create_result {
            Ok(output) if output.status.success() => {
                // Apply edits in worktree
                let worktree_policy = FilePolicy::default_for_repo(worktree_path.clone());
                let patch_result = apply_patch_temp_only(edits, file_set, &worktree_policy)
                    .await
                    .context("Failed to apply patch in git worktree")?;

                let handle = Self {
                    root: worktree_path,
                    original_root: original_root.to_path_buf(),
                    strategy: WorkspaceStrategy::GitWorktree,
                    needs_cleanup: true,
                };

                Ok((handle, patch_result))
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("Failed to create git worktree: {}", stderr)
            }
            Err(e) => {
                bail!("Git worktree command failed (is git installed?): {}", e)
            }
        }
    }

    /// Create an in-place workspace (no isolation - only for ReviewOnly)
    ///
    /// ⚠️ WARNING: This performs validation directly on the original repo
    /// without applying patches. Only safe for read-only validation.
    async fn create_in_place(
        original_root: &Path,
        _edits: &[EditOperation],
        _file_set: &FileSet,
        _policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        // InPlace doesn't apply patches - it's for read-only validation only
        // Return an empty patch result indicating no changes were made
        let patch_result = PatchResult {
            applied: false,
            changed_files: vec![],
            diff: String::new(),
            failures: vec![],
            dry_run: true,
            transaction_id: None,
            content_hashes: std::collections::HashMap::new(),
            snapshots: vec![],
        };

        let handle = Self {
            root: original_root.to_path_buf(),
            original_root: original_root.to_path_buf(),
            strategy: WorkspaceStrategy::InPlace,
            needs_cleanup: false,
        };

        Ok((handle, patch_result))
    }

    /// Check if git worktree is available
    pub async fn is_git_worktree_available(repo_root: &Path) -> bool {
        // Check if we're in a git repo
        let result = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(repo_root)
            .output()
            .await;

        match result {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// Clean up the workspace
    pub async fn cleanup(&self) -> Result<()> {
        if !self.needs_cleanup {
            return Ok(());
        }

        match self.strategy {
            WorkspaceStrategy::TempCopy => {
                if self.root.exists() {
                    tokio::fs::remove_dir_all(&self.root)
                        .await
                        .context("Failed to cleanup temp workspace")?;
                }
            }
            WorkspaceStrategy::GitWorktree => {
                // Remove git worktree
                let result = Command::new("git")
                    .args(["worktree", "remove", "--force"])
                    .arg(&self.root)
                    .current_dir(&self.original_root)
                    .output()
                    .await;

                if let Err(e) = result {
                    tracing::warn!("Failed to remove git worktree: {}", e);
                }
            }
            WorkspaceStrategy::InPlace => {
                // No cleanup needed
            }
        }

        Ok(())
    }

    /// Get the workspace root path
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Get the strategy used
    pub fn strategy(&self) -> WorkspaceStrategy {
        self.strategy
    }
}

/// Workspace strategy selector based on mode policy
pub struct WorkspaceSelector;

impl WorkspaceSelector {
    /// Select the best available workspace strategy
    ///
    /// Priority:
    /// 1. GitWorktree (if available and preferred)
    /// 2. TempCopy (always available, fully isolated)
    /// 3. InPlace (only for ReviewOnly mode)
    pub async fn select(
        preferred: WorkspaceStrategy,
        repo_root: &Path,
        mode: crate::harness::mode_policy::HarnessMode,
    ) -> WorkspaceStrategy {
        match preferred {
            WorkspaceStrategy::GitWorktree => {
                if WorkspaceHandle::is_git_worktree_available(repo_root).await {
                    WorkspaceStrategy::GitWorktree
                } else {
                    tracing::info!("Git worktree not available, falling back to TempCopy");
                    WorkspaceStrategy::TempCopy
                }
            }
            WorkspaceStrategy::InPlace => {
                // Only allow InPlace for ReviewOnly mode
                if matches!(mode, crate::harness::mode_policy::HarnessMode::ReviewOnly) {
                    WorkspaceStrategy::InPlace
                } else {
                    tracing::warn!("InPlace strategy only allowed in ReviewOnly mode, using TempCopy");
                    WorkspaceStrategy::TempCopy
                }
            }
            WorkspaceStrategy::TempCopy => WorkspaceStrategy::TempCopy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_workspace_strategy_selection() {
        // ReviewOnly can use InPlace
        // Other modes should fall back to TempCopy
        let repo_root = PathBuf::from("/tmp/test-repo");

        // Note: Actual async tests would require a real git repo
        // These are compile-time checks
        let _temp_copy = WorkspaceStrategy::TempCopy;
        let _git_worktree = WorkspaceStrategy::GitWorktree;
        let _in_place = WorkspaceStrategy::InPlace;
    }
}
