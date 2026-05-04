pub use crate::harness::mode_policy::WorkspaceStrategy;

use crate::harness::{
    edit_protocol::EditOperation,
    file_control::{FilePolicy, FileSet},
    patch_applier::{PatchResult, apply_patch},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

/// A temporary workspace for validation without mutating the real repo
#[derive(Debug, Clone)]
pub struct TempWorkspace {
    pub root: PathBuf,
    pub original_root: PathBuf,
    pub strategy: WorkspaceStrategy,
}

impl TempWorkspace {
    /// Create a new temp workspace using the TempCopy strategy
    pub async fn create_temp_copy(
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        let temp_dir = std::env::temp_dir();
        let workspace_name = format!(
            "prometheos_validate_{}",
            std::process::id()
        );
        let temp_root = temp_dir.join(&workspace_name);

        // Create temp directory
        fs::create_dir_all(&temp_root)
            .await
            .context("Failed to create temp workspace directory")?;

        // Copy relevant files to temp workspace
        Self::copy_repo_files(original_root, &temp_root, file_set)
            .await
            .context("Failed to copy repo files to temp workspace")?;

        // Apply edits in temp workspace
        let temp_policy = FilePolicy::default_for_repo(temp_root.clone());
        let patch_result = apply_patch(edits, file_set, &temp_policy)
            .await
            .context("Failed to apply patch in temp workspace")?;

        let workspace = Self {
            root: temp_root,
            original_root: original_root.to_path_buf(),
            strategy: WorkspaceStrategy::TempCopy,
        };

        Ok((workspace, patch_result))
    }

    /// Copy relevant files from original repo to temp workspace
    async fn copy_repo_files(
        original_root: &Path,
        temp_root: &Path,
        file_set: &FileSet,
    ) -> Result<()> {
        // Copy editable files
        for file in &file_set.editable {
            let src = original_root.join(file);
            let dst = temp_root.join(file);

            if src.exists() {
                // Create parent directories
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).await.ok();
                }
                fs::copy(&src, &dst).await.ok();
            }
        }

        // Copy important config files
        let config_files = [
            "Cargo.toml", "Cargo.lock", "package.json", "package-lock.json",
            "pyproject.toml", "requirements.txt", "setup.py", "go.mod", "go.sum",
            "pom.xml", "build.gradle", "tsconfig.json", ".gitignore",
        ];

        for config in &config_files {
            let src = original_root.join(config);
            let dst = temp_root.join(config);
            if src.exists() {
                fs::copy(&src, &dst).await.ok();
            }
        }

        Ok(())
    }

    /// Clean up the temp workspace
    pub async fn cleanup(&self) -> Result<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)
                .await
                .context("Failed to cleanup temp workspace")?;
        }
        Ok(())
    }
}

/// Determine the validation target based on execution mode and patch state
#[derive(Debug, Clone)]
pub enum ValidationTarget {
    /// Validate the real repo (patch was applied)
    RealRepo(PathBuf),
    /// Validate a temp workspace (patch not applied to real repo)
    TempWorkspace(TempWorkspace),
    /// No validation needed
    None,
}

impl ValidationTarget {
    /// Get the root path for validation
    pub fn path(&self) -> Option<&Path> {
        match self {
            ValidationTarget::RealRepo(path) => Some(path),
            ValidationTarget::TempWorkspace(ws) => Some(&ws.root),
            ValidationTarget::None => None,
        }
    }
}

/// Create appropriate validation target based on execution state
pub async fn create_validation_target(
    repo_root: &Path,
    should_apply: bool,
    patch_was_applied: bool,
    edits: &[EditOperation],
    file_set: &FileSet,
    policy: &FilePolicy,
) -> Result<ValidationTarget> {
    if patch_was_applied {
        // Patch was applied to real repo, validate in-place
        Ok(ValidationTarget::RealRepo(repo_root.to_path_buf()))
    } else if !edits.is_empty() {
        // Patch wasn't applied but edits exist - create temp workspace
        let (workspace, _result) = TempWorkspace::create_temp_copy(
            repo_root,
            edits,
            file_set,
            policy,
        ).await?;
        Ok(ValidationTarget::TempWorkspace(workspace))
    } else {
        // No edits to validate
        Ok(ValidationTarget::None)
    }
}
