pub use crate::harness::mode_policy::WorkspaceStrategy;

use crate::harness::{
    edit_protocol::EditOperation,
    file_control::{FilePolicy, FileSet},
    patch_applier::{PatchResult, apply_patch_temp_only},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

/// P0-C4: Resource and file limits for temp workspace creation
#[derive(Debug, Clone)]
pub struct TempWorkspaceLimits {
    pub max_files: usize,
    pub max_size_mb: usize,
    pub max_subdirectories: usize,
    pub max_file_size_mb: usize,
}

impl Default for TempWorkspaceLimits {
    fn default() -> Self {
        Self {
            max_files: 1000,           // Maximum 1000 files
            max_size_mb: 100,          // Maximum 100MB total
            max_subdirectories: 50,    // Maximum 50 subdirectories
            max_file_size_mb: 10,      // Maximum 10MB per file
        }
    }
}

/// A temporary workspace for validation without mutating the real repo
#[derive(Debug, Clone)]
pub struct TempWorkspace {
    pub root: PathBuf,
    pub original_root: PathBuf,
    pub strategy: WorkspaceStrategy,
    pub limits: TempWorkspaceLimits,
}

impl TempWorkspace {
    /// Create a new temp workspace using the TempCopy strategy
    pub async fn create_temp_copy(
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
    ) -> Result<(Self, PatchResult)> {
        Self::create_temp_copy_with_limits(original_root, edits, file_set, policy, TempWorkspaceLimits::default()).await
    }

    /// P0-C4: Create a new temp workspace with explicit resource/file limits
    pub async fn create_temp_copy_with_limits(
        original_root: &Path,
        edits: &[EditOperation],
        file_set: &FileSet,
        policy: &FilePolicy,
        limits: TempWorkspaceLimits,
    ) -> Result<(Self, PatchResult)> {
        let temp_dir = std::env::temp_dir();
        // Use UUID + timestamp for unique, collision-safe naming
        let workspace_name = format!(
            "prometheos_validate_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );
        let temp_root = temp_dir.join(&workspace_name);

        // P0-C4: Check file set against limits before creating workspace
        Self::check_file_set_limits(file_set, &limits)?;

        // Create temp directory
        fs::create_dir_all(&temp_root)
            .await
            .context("Failed to create temp workspace directory")?;

        // Copy relevant files to temp workspace with limits enforcement
        Self::copy_repo_files_with_limits(original_root, &temp_root, file_set, &limits)
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
            limits,
        };

        Ok((workspace, patch_result))
    }

    /// P0-C4: Check file set against resource limits before workspace creation
    fn check_file_set_limits(file_set: &FileSet, limits: &TempWorkspaceLimits) -> Result<()> {
        if file_set.editable.len() > limits.max_files {
            bail!(
                "File set exceeds maximum files limit: {} > {}",
                file_set.editable.len(),
                limits.max_files
            );
        }

        // Check if any file is too large (quick check using metadata)
        for file_path in &file_set.editable {
            if let Ok(metadata) = std::fs::metadata(file_path) {
                let file_size_mb = metadata.len() / (1024 * 1024);
                if file_size_mb > limits.max_file_size_mb as u64 {
                    bail!(
                        "File {} exceeds maximum size limit: {}MB > {}MB",
                        file_path.display(),
                        file_size_mb,
                        limits.max_file_size_mb
                    );
                }
            }
        }

        tracing::info!(
            "P0-C4: File set passed limits check: {} files, max {} allowed",
            file_set.editable.len(),
            limits.max_files
        );

        Ok(())
    }

    /// P0-C4: Copy relevant files from original repo to temp workspace with limits enforcement
    async fn copy_repo_files_with_limits(
        original_root: &Path,
        temp_root: &Path,
        file_set: &FileSet,
        limits: &TempWorkspaceLimits,
    ) -> Result<()> {
        let mut files_copied = 0;
        let mut total_size_mb = 0;
        let mut subdirectories_created = 0;

        for file_path in &file_set.editable {
            // Check file count limit
            if files_copied >= limits.max_files {
                bail!(
                    "Reached maximum file limit during copy: {} files",
                    files_copied
                );
            }

            // Normalize path to be relative to original root
            let relative_path = file_path
                .strip_prefix(original_root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| {
                    // If strip_prefix fails, use the original path as relative
                    file_path.to_path_buf()
                });

            let target_path = temp_root.join(&relative_path);

            // Create parent directories if needed
            if let Some(parent) = target_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).await
                        .context("Failed to create parent directory in temp workspace")?;
                    
                    subdirectories_created += 1;
                    if subdirectories_created > limits.max_subdirectories {
                        bail!(
                            "Reached maximum subdirectory limit during copy: {} directories",
                            subdirectories_created
                        );
                    }
                }
            }

            // Copy file with size check
            let file_size = fs::metadata(file_path).await
                .context("Failed to get file metadata")?
                .len();
            
            let file_size_mb = file_size / (1024 * 1024);
            total_size_mb += file_size_mb;

            if total_size_mb > limits.max_size_mb as u64 {
                bail!(
                    "Reached maximum total size limit during copy: {}MB > {}MB",
                    total_size_mb,
                    limits.max_size_mb
                );
            }

            if file_size_mb > limits.max_file_size_mb as u64 {
                bail!(
                    "File {} exceeds maximum size limit: {}MB > {}MB",
                    file_path.display(),
                    file_size_mb,
                    limits.max_file_size_mb
                );
            }

            fs::copy(file_path, &target_path).await
                .with_context(|| format!("Failed to copy file {} to temp workspace", file_path.display()))?;

            files_copied += 1;
        }

        tracing::info!(
            "P0-C4: Successfully copied {} files ({}MB, {} directories) to temp workspace",
            files_copied,
            total_size_mb,
            subdirectories_created
        );

        Ok(())
    }

    /// Copy relevant files from original repo to temp workspace
    ///
    /// P0-FIX: Correctly handles absolute paths in FileSet by normalizing to relative
    /// before joining with temp_root. Fails loudly on copy errors for required files.
    async fn copy_repo_files(
        original_root: &Path,
        temp_root: &Path,
        file_set: &FileSet,
    ) -> Result<()> {
        // Canonicalize original_root for consistent path handling
        let canonical_original = original_root.canonicalize()
            .context("Failed to canonicalize original root")?;

        // Copy editable files
        for file in &file_set.editable {
            // P0-FIX: Normalize file path to be relative to original_root
            let relative_path = normalize_to_relative(&canonical_original, file)
                .context(format!("Failed to normalize path: {:?}", file))?;

            let src = canonical_original.join(&relative_path);
            let dst = temp_root.join(&relative_path);

            // P0-FIX: Fail loudly if source doesn't exist (required file)
            if !src.exists() {
                anyhow::bail!(
                    "Required file does not exist: {:?} (normalized from {:?})",
                    src, file
                );
            }

            // Create parent directories
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).await
                    .context(format!("Failed to create parent directories for {:?}", dst))?;
            }

            // P0-FIX: Fail loudly on copy errors for required files
            fs::copy(&src, &dst).await
                .context(format!("Failed to copy file from {:?} to {:?}", src, dst))?;
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

/// P0-FIX: Normalize a path to be relative to the base directory
///
/// Handles both absolute and relative paths correctly:
/// - If path is absolute and under base, returns relative path
/// - If path is already relative, validates it's under base and returns as-is
/// - If path is absolute and outside base, returns error
fn normalize_to_relative(base: &Path, path: &Path) -> Result<PathBuf> {
    let canonical_path = if path.is_absolute() {
        path.canonicalize()
            .context(format!("Failed to canonicalize path: {:?}", path))?
    } else {
        base.join(path).canonicalize()
            .context(format!("Failed to canonicalize relative path: {:?}", path))?
    };

    // Ensure the path is under the base directory (security check)
    if !canonical_path.starts_with(base) {
        anyhow::bail!(
            "Path {:?} is outside base directory {:?}",
            canonical_path, base
        );
    }

    // Strip the base prefix to get relative path
    Ok(canonical_path.strip_prefix(base)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(".")))
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
