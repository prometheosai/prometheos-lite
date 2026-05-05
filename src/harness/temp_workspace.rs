pub use crate::harness::mode_policy::WorkspaceStrategy;

use crate::harness::{
    edit_protocol::EditOperation,
    file_control::{FilePolicy, FileSet},
    patch_applier::{PatchResult, apply_patch_temp_only},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

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
        // Use UUID + timestamp for unique, collision-safe naming
        let workspace_name = format!(
            "prometheos_validate_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
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

        // Apply edits in temp workspace (safe - temp_only version)
        let temp_policy = FilePolicy::default_for_repo(temp_root.clone());
        let patch_result = apply_patch_temp_only(edits, file_set, &temp_policy)
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
            "yarn.lock", "pnpm-lock.yaml", "bun.lockb",
            "pyproject.toml", "requirements.txt", "requirements-dev.txt", "setup.py", "setup.cfg",
            "go.mod", "go.sum", "go.work", "go.work.sum",
            "pom.xml", "build.gradle", "gradle.properties", "settings.gradle",
            "tsconfig.json", "jsconfig.json", ".babelrc", ".eslintrc", ".prettierrc",
            ".gitignore", ".dockerignore", "Dockerfile",
            "Makefile", "justfile", "Justfile",
            ".cargo/config.toml", ".cargo/config",
        ];

        for config in &config_files {
            let src = original_root.join(config);
            let dst = temp_root.join(config);
            if src.exists() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).await.ok();
                }
                fs::copy(&src, &dst).await.ok();
            }
        }

        // Copy test directories and fixtures
        let important_dirs = [
            "tests", "test", "__tests__", "spec", "specs",
            "fixtures", "fixture", "testdata", "test_data", "data",
            "examples", "example", "demo", "demos",
            "benches", "benchmarks", "bench",
            "migrations", "migration", "alembic",
            "scripts", "script", "bin",
            "static", "assets", "public", "resources",
            "templates", "template",
            "proto", "protobuf", "protos",
            "thrift", "idl",
        ];

        for dir_name in &important_dirs {
            let src_dir = original_root.join(dir_name);
            if src_dir.exists() && src_dir.is_dir() {
                let dst_dir = temp_root.join(dir_name);
                Self::copy_directory(&src_dir, &dst_dir).await.ok();
            }
        }

        // Copy Cargo workspace members if this is a Rust project
        Self::copy_workspace_members(original_root, temp_root).await.ok();

        Ok(())
    }

    /// Recursively copy a directory
    async fn copy_directory(src: &Path, dst: &Path) -> Result<()> {
        for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let relative = path.strip_prefix(src)?;
                let dst_path = dst.join(relative);

                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent).await.ok();
                }
                fs::copy(path, dst_path).await.ok();
            }
        }
        Ok(())
    }

    /// Copy Cargo workspace member directories
    async fn copy_workspace_members(original_root: &Path, temp_root: &Path) -> Result<()> {
        let cargo_toml = original_root.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Ok(());
        }

        // Parse workspace members from Cargo.toml
        let content = fs::read_to_string(&cargo_toml).await.ok();
        if let Some(content) = content {
            // Simple parsing for workspace.members array
            if content.contains("[workspace]") {
                // Try to find member directories
                for entry in walkdir::WalkDir::new(original_root)
                    .max_depth(2)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let path = entry.path();
                    if path.is_dir() && path.join("Cargo.toml").exists() {
                        let relative = path.strip_prefix(original_root)?;
                        let dst_path = temp_root.join(relative);
                        Self::copy_directory(path, &dst_path).await.ok();
                    }
                }
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
