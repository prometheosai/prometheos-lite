//! Repo tooling layer - safe, deterministic access to filesystem and repo
//!
//! This module provides tools for repository operations including:
//! - list_tree: List files and directories
//! - read_file: Read file contents
//! - search_files: Search for patterns in files
//! - write_file: Write file contents
//! - patch_file: Apply patches to files with validation
//! - git_diff: Get git diff output

use crate::flow::Tool;
use crate::tools::PathGuard;
use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::io::FromRawHandle;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::Mutex;

/// Fail-closed open anchored at the filesystem root of `full_path`; strictly
/// stronger than a bare pathname open. Exercises the same production
/// fail-closed write path used by `WriteFileTool` after validation (reparse
/// refusal, verified-open-then-truncate). Not a public API contract; retained
/// so the in-crate post-validation boundary test can exercise it directly.
#[cfg(test)]
fn open_write_fail_closed(full_path: &Path) -> Result<std::fs::File> {
    let root = full_path
        .ancestors()
        .last()
        .context("failed to determine filesystem root")?
        .to_path_buf();
    open_write_verified(&root, full_path)
}

/// RepoTool trait for repository operations
pub trait RepoTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value>;
}

/// List tree tool - lists files and directories in a repository
pub struct ListTreeTool {
    repo_path: PathBuf,
    path_guard: PathGuard,
}

impl ListTreeTool {
    pub fn new(repo_path: PathBuf) -> Self {
        // Use repo_path as base directory for PathGuard
        let base_dir = repo_path.to_string_lossy().to_string();
        Self {
            repo_path,
            path_guard: PathGuard::new(base_dir),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ListTreeTool {
    fn name(&self) -> String {
        "list_tree".to_string()
    }

    fn description(&self) -> String {
        "Lists files and directories in the repository with optional depth control".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "root": {
                    "type": "string",
                    "description": "Root directory to list (relative to repo root, empty for root)"
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth to traverse (null for unlimited)",
                    "default": null
                }
            }
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let root = input.get("root").and_then(|v| v.as_str()).unwrap_or("");

        // Validate path with PathGuard (canonical, symlink-escape-proof)
        let base_path = if root.is_empty() {
            self.repo_path.clone()
        } else {
            match self.path_guard.validate_path(root) {
                Ok(p) => std::path::PathBuf::from(p),
                Err(_) => {
                    return Ok(serde_json::json!({
                        "error": format!("Path validation failed: {}", root),
                        "success": false
                    }));
                }
            }
        };

        let depth = input
            .get("depth")
            .and_then(|v| v.as_u64())
            .map(|d| d as u32);

        let mut files = Vec::new();
        let mut dirs = Vec::new();

        if let Some(max_depth) = depth {
            self.list_tree_recursive(&base_path, 0, max_depth, &mut files, &mut dirs)
                .await?;
        } else {
            self.list_tree_recursive(&base_path, 0, u32::MAX, &mut files, &mut dirs)
                .await?;
        }

        Ok(serde_json::json!({
            "files": files,
            "dirs": dirs,
            "success": true
        }))
    }
}

impl ListTreeTool {
    async fn list_tree_recursive(
        &self,
        path: &Path,
        current_depth: u32,
        max_depth: u32,
        files: &mut Vec<String>,
        dirs: &mut Vec<String>,
    ) -> Result<()> {
        if current_depth > max_depth {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(path)
            .await
            .context("Failed to read directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();

            // Never follow symlinks/junctions: a repo-internal link pointing
            // outside the repository must not be recursed into.
            let file_type = entry.file_type().await?;
            if file_type.is_symlink() {
                continue;
            }

            // Skip common ignored directories
            if let Some(name) = entry_path.file_name().and_then(|n| n.to_str())
                && (name == "target" || name == "node_modules" || name == ".git" || name == ".next")
            {
                continue;
            }

            if entry_path.is_dir() {
                dirs.push(entry_path.to_string_lossy().to_string());
                Box::pin(self.list_tree_recursive(
                    &entry_path,
                    current_depth + 1,
                    max_depth,
                    files,
                    dirs,
                ))
                .await?;
            } else {
                files.push(entry_path.to_string_lossy().to_string());
            }
        }

        Ok(())
    }
}

/// Read file tool - reads file contents
pub struct RepoReadFileTool {
    repo_path: PathBuf,
    path_guard: PathGuard,
}

impl RepoReadFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        let base_dir = repo_path.to_string_lossy().to_string();
        Self {
            repo_path,
            path_guard: PathGuard::new(base_dir),
        }
    }
}

#[async_trait::async_trait]
impl Tool for RepoReadFileTool {
    fn name(&self) -> String {
        "read_file".to_string()
    }

    fn description(&self) -> String {
        "Reads the contents of a file from the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file relative to repository root"
                }
            },
            "required": ["path"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        // Validate path with PathGuard
        let safe_path = match self.path_guard.validate_path(path) {
            Ok(p) => p,
            Err(_) => {
                return Ok(serde_json::json!({
                    "error": format!("Path validation failed: {}", path),
                    "success": false
                }));
            }
        };

        let full_path = std::path::PathBuf::from(&safe_path);
        let base = self.repo_path.clone();

        // Effect decided on an opened handle, never on a re-checked pathname.
        let file = match open_read_verified(&base, &full_path) {
            Ok(file) => file,
            Err(err) => {
                return Ok(serde_json::json!({
                    "error": format!("Fail-closed read refused: {err:#}"),
                    "success": false
                }));
            }
        };
        let mut file = file;
        let mut content = String::new();
        use std::io::Read as _;
        file.read_to_string(&mut content)
            .context("Failed to read file")?;

        Ok(serde_json::json!({
            "content": content,
            "path": path,
            "success": true
        }))
    }
}

/// Search files tool - searches for patterns across files
pub struct SearchFilesTool {
    repo_path: PathBuf,
}

impl SearchFilesTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> String {
        "search_files".to_string()
    }

    fn description(&self) -> String {
        "Searches for a pattern across files in the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Pattern to search for"
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob pattern to filter files (e.g., '*.rs')"
                }
            },
            "required": ["query"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .context("Missing query")?;

        let glob = input.get("glob").and_then(|v| v.as_str());

        let mut results = Vec::new();
        self.search_repo(query, glob, &mut results).await?;

        Ok(serde_json::json!({
            "query": query,
            "results": results,
            "count": results.len(),
            "success": true
        }))
    }
}

impl SearchFilesTool {
    async fn search_repo(
        &self,
        query: &str,
        glob: Option<&str>,
        results: &mut Vec<serde_json::Value>,
    ) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.repo_path)
            .await
            .context("Failed to read repo directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Never follow symlinks/junctions out of the repository.
            let file_type = entry.file_type().await?;
            if file_type.is_symlink() {
                continue;
            }

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && (name == "target" || name == "node_modules" || name == ".git")
                {
                    continue;
                }

                let mut sub_entries = tokio::fs::read_dir(&path).await?;
                while let Some(sub_entry) = sub_entries.next_entry().await? {
                    let sub_path = sub_entry.path();
                    let sub_type = sub_entry.file_type().await?;
                    if sub_type.is_symlink() {
                        // Skip symlinks/junctions: never read outside the repo.
                        continue;
                    }
                    if sub_type.is_file() {
                        if let Some(glob_pattern) = glob
                            && !self.matches_glob(&sub_path, glob_pattern)
                        {
                            continue;
                        }
                        self.search_file(&sub_path, query, results).await?;
                    }
                }
            } else if path.is_file() {
                if let Some(glob_pattern) = glob
                    && !self.matches_glob(&path, glob_pattern)
                {
                    continue;
                }
                self.search_file(&path, query, results).await?;
            }
        }

        Ok(())
    }

    async fn search_file(
        &self,
        path: &Path,
        query: &str,
        results: &mut Vec<serde_json::Value>,
    ) -> Result<()> {
        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read file")?;

        for (line_num, line) in content.lines().enumerate() {
            if line.contains(query) {
                results.push(serde_json::json!({
                    "file": path.to_string_lossy().to_string(),
                    "line": line_num + 1,
                    "content": line,
                    "query": query
                }));
            }
        }

        Ok(())
    }

    fn matches_glob(&self, path: &Path, glob: &str) -> bool {
        let glob_pattern = glob.trim_start_matches('*').trim_start_matches('.');
        if let Some(extension) = path.extension() {
            return extension.to_str() == Some(glob_pattern);
        }
        false
    }
}

/// Write file tool - writes content to a file
pub struct WriteFileTool {
    repo_path: PathBuf,
    path_guard: PathGuard,
}

impl WriteFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        let base_dir = repo_path.to_string_lossy().to_string();
        Self {
            repo_path,
            path_guard: PathGuard::new(base_dir),
        }
    }
}

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> String {
        "write_file".to_string()
    }

    fn description(&self) -> String {
        "Writes content to a file in the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file relative to repository root"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .context("Missing content")?;

        // Validate path with PathGuard
        let safe_path = match self.path_guard.validate_path(path) {
            Ok(p) => p,
            Err(_) => {
                return Ok(serde_json::json!({
                    "error": format!("Path validation failed: {}", path),
                    "success": false
                }));
            }
        };

        let full_path = std::path::PathBuf::from(&safe_path);
        let base = self.repo_path.clone();

        // Effect decided on an opened handle, never on a re-checked pathname.
        let file = match open_write_verified(&base, &full_path) {
            Ok(file) => file,
            Err(err) => {
                return Ok(serde_json::json!({
                    "error": format!("Fail-closed write refused: {err:#}"),
                    "success": false
                }));
            }
        };
        use std::io::Write as _;
        let mut file = file;
        file.write_all(content.as_bytes())
            .context("Failed to write file")?;

        Ok(serde_json::json!({
            "path": path,
            "bytes_written": content.len(),
            "success": true
        }))
    }
}

/// Patch file tool - applies a diff to a file with validation
pub struct PatchFileTool {
    repo_path: PathBuf,
    path_guard: PathGuard,
}

impl PatchFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        let base_dir = repo_path.to_string_lossy().to_string();
        Self {
            repo_path,
            path_guard: PathGuard::new(base_dir),
        }
    }

    /// Retained for API compatibility: historically this allowed the
    /// simplified, in-repo diff applier. The tool now always uses the
    /// bounded, self-contained unified-diff applier; both constructors
    /// behave identically.
    pub fn with_fallback_allowed(repo_path: PathBuf) -> Self {
        Self::new(repo_path)
    }
}

#[async_trait::async_trait]
impl Tool for PatchFileTool {
    fn name(&self) -> String {
        "patch_file".to_string()
    }

    fn description(&self) -> String {
        "Applies a unified diff patch to a file with validation".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file relative to repository root"
                },
                "diff": {
                    "type": "string",
                    "description": "Unified diff format patch to apply"
                }
            },
            "required": ["path", "diff"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        let diff = input
            .get("diff")
            .and_then(|v| v.as_str())
            .context("Missing diff")?;

        // Validate path with PathGuard
        let safe_path = match self.path_guard.validate_path(path) {
            Ok(p) => p,
            Err(_) => {
                return Ok(serde_json::json!({
                    "error": format!("Path validation failed: {}", path),
                    "success": false
                }));
            }
        };

        let full_path = std::path::PathBuf::from(&safe_path);
        let base = self.repo_path.clone();

        // Validate diff target and format against the requested path.
        if let Err(err) = self.validate_diff_for_path(diff, path) {
            return Ok(serde_json::json!({
                "error": err.to_string(),
                "success": false,
                "validation": "failed"
            }));
        }

        // Verified read through a handled, reparse-free chain.
        let original_content = {
            let file = match open_read_verified(&base, &full_path) {
                Ok(file) => file,
                Err(err) => {
                    return Ok(serde_json::json!({
                        "error": format!("Fail-closed read refused: {err:#}"),
                        "success": false,
                        "validation": "failed"
                    }));
                }
            };
            let mut file = file;
            let mut content = String::new();
            use std::io::Read as _;
            file.read_to_string(&mut content)
                .context("Failed to read original file")?;
            content
        };

        // Apply the unified diff in memory (no external patch(1)).
        let patched_content = match apply_unified_diff(&original_content, diff) {
            Ok(patched) => patched,
            Err(err) => {
                return Ok(serde_json::json!({
                    "error": format!("Patch refused: {err:#}"),
                    "success": false,
                    "validation": "failed"
                }));
            }
        };

        // Verified, self-contained write through the same containment.
        use std::io::Write as _;
        let mut file = match open_write_verified(&base, &full_path) {
            Ok(file) => file,
            Err(err) => {
                return Ok(serde_json::json!({
                    "error": format!("Fail-closed write refused: {err:#}"),
                    "success": false,
                    "validation": "failed"
                }));
            }
        };
        file.write_all(patched_content.as_bytes())
            .context("Failed to write patched file")?;

        Ok(serde_json::json!({
            "path": path,
            "diff": diff,
            "validation": "passed",
            "success": true,
            "lines_changed": self.count_changes(diff)
        }))
    }
}

impl PatchFileTool {
    fn validate_diff_for_path(&self, diff: &str, requested_path: &str) -> Result<()> {
        // Basic validation: check for hunk headers.
        if !diff.lines().any(|line| line.starts_with("@@")) {
            anyhow::bail!("Invalid diff format: missing hunk header");
        }

        let targets = self.extract_diff_targets(diff)?;
        if targets.len() != 1 {
            anyhow::bail!("patch_file requires exactly one target file in diff headers");
        }

        let requested = Self::normalize_diff_path(requested_path)?;
        let target = Self::normalize_diff_path(&targets[0])?;

        if requested != target {
            anyhow::bail!(
                "Diff target '{}' does not match requested path '{}'",
                targets[0],
                requested_path
            );
        }

        if self.path_guard.validate_path(&target).is_err() {
            anyhow::bail!("Diff target path validation failed: {}", target);
        }

        Ok(())
    }

    fn extract_diff_targets(&self, diff: &str) -> Result<Vec<String>> {
        let mut old_targets = Vec::new();
        let mut new_targets = Vec::new();

        for line in diff.lines() {
            if let Some(path) = line.strip_prefix("--- ") {
                old_targets.push(Self::parse_diff_header_path(path)?);
            } else if let Some(path) = line.strip_prefix("+++ ") {
                new_targets.push(Self::parse_diff_header_path(path)?);
            }
        }

        if old_targets.is_empty() || new_targets.is_empty() {
            anyhow::bail!("Invalid diff format: missing ---/+++ headers");
        }
        if old_targets.len() != new_targets.len() {
            anyhow::bail!("Invalid diff format: mismatched ---/+++ header count");
        }

        let mut targets = Vec::new();
        for (old_path, new_path) in old_targets.iter().zip(new_targets.iter()) {
            if old_path == "/dev/null" || new_path == "/dev/null" {
                anyhow::bail!("Create/delete patches are not supported by patch_file");
            }

            let old_norm = Self::normalize_diff_path(old_path)?;
            let new_norm = Self::normalize_diff_path(new_path)?;
            if old_norm != new_norm {
                anyhow::bail!(
                    "Rename/multi-target patch is not supported: '{}' -> '{}'",
                    old_path,
                    new_path
                );
            }
            targets.push(old_norm);
        }

        Ok(targets)
    }

    fn parse_diff_header_path(header_value: &str) -> Result<String> {
        let candidate = header_value
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Invalid diff header path"))?;
        if candidate.is_empty() {
            anyhow::bail!("Invalid empty diff header path");
        }
        Ok(candidate.to_string())
    }

    fn normalize_diff_path(path: &str) -> Result<String> {
        let trimmed = path.trim();
        let stripped = trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed);

        if stripped.is_empty() {
            anyhow::bail!("Invalid empty diff target path");
        }
        if stripped.starts_with('/') || stripped.starts_with('\\') {
            anyhow::bail!("Absolute diff target paths are not allowed");
        }
        if stripped.contains(':') {
            anyhow::bail!("Drive-letter or URI-like diff target paths are not allowed");
        }
        if stripped.split('/').any(|seg| seg == "..") || stripped.split('\\').any(|seg| seg == "..")
        {
            anyhow::bail!("Path traversal in diff target is not allowed");
        }

        Ok(stripped.replace('\\', "/"))
    }

    fn count_changes(&self, diff: &str) -> usize {
        diff.lines()
            .filter(|line| {
                (line.starts_with('+') && !line.starts_with("+++"))
                    || (line.starts_with('-') && !line.starts_with("---"))
            })
            .count()
    }
}

/// Git diff tool - gets git diff output
pub struct GitDiffTool {
    repo_path: PathBuf,
}

impl GitDiffTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> String {
        "git_diff".to_string()
    }

    fn description(&self) -> String {
        "Gets git diff output for the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "cached": {
                    "type": "boolean",
                    "description": "Whether to show staged changes (--cached)",
                    "default": false
                }
            }
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let cached = input
            .get("cached")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = Command::new("git");
        cmd.arg("diff");
        if cached {
            cmd.arg("--cached");
        }
        cmd.current_dir(&self.repo_path);

        let output = cmd.output().context("Failed to execute git diff")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(serde_json::json!({
                "error": format!("git diff failed: {}", stderr),
                "success": false
            }));
        }

        let diff = String::from_utf8_lossy(&output.stdout);

        Ok(serde_json::json!({
            "diff": diff.to_string(),
            "cached": cached,
            "success": true
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_list_tree() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // Create test structure
        tokio::fs::create_dir_all(repo_path.join("src"))
            .await
            .unwrap();
        tokio::fs::write(repo_path.join("src/main.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(repo_path.join("README.md"), "# Test")
            .await
            .unwrap();

        let tool = ListTreeTool::new(repo_path.to_path_buf());
        let result = tool.call(serde_json::json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(!result["files"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        tokio::fs::write(repo_path.join("test.txt"), "Hello, World!")
            .await
            .unwrap();

        let tool = RepoReadFileTool::new(repo_path.to_path_buf());
        let result = tool
            .call(serde_json::json!({"path": "test.txt"}))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["content"].as_str().unwrap(), "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = WriteFileTool::new(repo_path.to_path_buf());
        let result = tool
            .call(serde_json::json!({
                "path": "new_file.txt",
                "content": "New content"
            }))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["bytes_written"].as_u64().unwrap(), 11);

        // Verify file was written
        let content = tokio::fs::read_to_string(repo_path.join("new_file.txt"))
            .await
            .unwrap();
        assert_eq!(content, "New content");
    }

    #[tokio::test]
    async fn test_search_files() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        tokio::fs::write(
            repo_path.join("test.rs"),
            "fn main() { println!(\"hello\"); }",
        )
        .await
        .unwrap();
        tokio::fs::write(repo_path.join("test.txt"), "hello world")
            .await
            .unwrap();

        let tool = SearchFilesTool::new(repo_path.to_path_buf());
        let result = tool
            .call(serde_json::json!({"query": "hello"}))
            .await
            .unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["count"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_patch_file_validation() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = PatchFileTool::new(repo_path.to_path_buf());

        // Invalid diff format
        assert!(
            tool.validate_diff_for_path("not a diff", "test.txt")
                .is_err()
        );

        // Valid diff format
        let valid_diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,1 +1,1 @@\n-old\n+new";
        assert!(tool.validate_diff_for_path(valid_diff, "test.txt").is_ok());
    }

    #[test]
    fn test_patch_file_rejects_mismatched_target() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();
        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let diff = "--- a/other.txt\n+++ b/other.txt\n@@ -1,1 +1,1 @@\n-old\n+new";
        assert!(tool.validate_diff_for_path(diff, "test.txt").is_err());
    }

    #[test]
    fn test_patch_file_rejects_multifile_diff() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();
        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let diff = "--- a/one.txt\n+++ b/one.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n--- a/two.txt\n+++ b/two.txt\n@@ -1,1 +1,1 @@\n-c\n+d";
        assert!(tool.validate_diff_for_path(diff, "one.txt").is_err());
    }

    #[test]
    fn test_patch_file_rejects_traversal_or_absolute_targets() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();
        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let traversal = "--- a/../secret.txt\n+++ b/../secret.txt\n@@ -1,1 +1,1 @@\n-a\n+b";
        assert!(
            tool.validate_diff_for_path(traversal, "../secret.txt")
                .is_err()
        );

        let absolute = "--- /tmp/secret.txt\n+++ /tmp/secret.txt\n@@ -1,1 +1,1 @@\n-a\n+b";
        assert!(tool.validate_diff_for_path(absolute, "secret.txt").is_err());
    }

    #[test]
    fn test_patch_file_allows_timestamped_headers() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();
        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let diff = "--- a/test.txt\t2026-05-11 12:00:00\n+++ b/test.txt\t2026-05-11 12:01:00\n@@ -1,1 +1,1 @@\n-old\n+new";
        assert!(tool.validate_diff_for_path(diff, "test.txt").is_ok());
    }

    #[tokio::test]
    #[cfg(not(windows))]
    async fn test_patch_file_system_patch_updates_file_content() {
        let patch_available = std::process::Command::new("sh")
            .arg("-lc")
            .arg("command -v patch >/dev/null 2>&1")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !patch_available {
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();
        tokio::fs::write(repo_path.join("test.txt"), "old content\n")
            .await
            .unwrap();

        let tool = PatchFileTool::new(repo_path.to_path_buf());
        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,1 +1,1 @@\n-old content\n+new content\n";
        let result = tool
            .call(serde_json::json!({
                "path": "test.txt",
                "diff": diff
            }))
            .await
            .unwrap();
        assert!(result["success"].as_bool().unwrap());

        let content = tokio::fs::read_to_string(repo_path.join("test.txt"))
            .await
            .unwrap();
        assert_eq!(content.trim_end_matches('\n'), "new content");
        assert!(!content.contains("patching file"));
    }

    #[test]
    fn test_count_changes() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n+another";
        assert_eq!(tool.count_changes(diff), 3);
    }

    fn reparse_entry_present(link: &std::path::Path) -> bool {
        std::fs::symlink_metadata(link)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
    }

    fn swap_leaf_with_file_reparse(link: &std::path::Path, target: &std::path::Path) -> bool {
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(target, link).is_ok()
                && reparse_entry_present(link)
            {
                return true;
            }
            let out = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output();
            if let Ok(out) = out
                && out.status.success()
                && reparse_entry_present(link)
            {
                return true;
            }
            false
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok() && reparse_entry_present(link)
        }
    }

    // Post-validation boundary: the same production fail-closed open that
    // WriteFileTool calls after validation and before the filesystem effect.
    #[tokio::test]
    async fn security_repo_swap_after_validation_write_fails_closed() {
        let base = TempDir::new().unwrap();
        let repo = base.path().join("repo");
        let outside = TempDir::new().unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "outside-of-repo").unwrap();

        let tool = WriteFileTool::new(repo.clone());
        let guard = crate::tools::PathGuard::new(repo.to_string_lossy().into_owned());
        let validated = guard
            .validate_path("swapped.txt")
            .expect("a fresh target inside the repo validates");

        // The attacker wins the race: AFTER validation, the leaf is swapped
        // for a reparse point to the outside victim, BEFORE the write effect.
        let link = repo.join("swapped.txt");
        if !swap_leaf_with_file_reparse(&link, &victim) {
            eprintln!(
                "SKIP: reparse point creation not permitted on this host; security assertions NOT exercised"
            );
            return;
        }
        assert!(
            reparse_entry_present(&link),
            "the swapped leaf must be observable as a reparse point after the swap"
        );

        let out = tool
            .call(serde_json::json!({"path": "swapped.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "a swapped path must also fail the tool's validation: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "outside-of-repo",
            "the outside victim must be unchanged after the refused call"
        );

        let boundary_result =
            open_write_fail_closed(std::path::PathBuf::from(&validated).as_path());
        assert!(
            boundary_result.is_err(),
            "the post-validation fail-closed open must refuse the swapped leaf"
        );

        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "outside-of-repo",
            "the outside victim file is untouched by the post-validation refusal"
        );
        assert!(
            reparse_entry_present(&link),
            "the swap must leave no effect behind other than the attacker's own link"
        );
    }
}

// ---------------------------------------------------------------------------
// Bounded unified-diff application
// ---------------------------------------------------------------------------

/// A parsed hunk: `-<start>[,<count>]`, its body lines with prefixes stripped,
/// and `+<start>[,<count>]`'s body lines with prefixes stripped.
#[derive(Debug, Clone)]
struct Hunk {
    old_start: usize,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

/// Parse a unified diff for exactly one file.
///
/// Accepted grammar (deliberately strict): optional `--- a/<path>` and
/// `+++ b/<path>` headers, then one or more `@@ -old[,n] +new[,m] @@` hunk
/// headers, each followed by `<n>` body lines: `' '` context, `'-'` removal,
/// `'+'` addition, and `'\'` ("No newline at end of file", ignored). Anything
/// else is refused. Empty diffs are refused.
fn apply_unified_diff(original: &str, diff: &str) -> Result<String> {
    if diff.trim().is_empty() {
        bail!("empty diff");
    }
    let original_trailing_newline = original.ends_with('\n');
    let original_lines: Vec<String> = original.lines().map(str::to_string).collect();
    let diff_lines: Vec<&str> = diff.lines().collect();
    let mut index = 0;
    // Skip the optional --- / +++ header lines.
    while index < diff_lines.len()
        && (diff_lines[index].starts_with("--- ") || diff_lines[index].starts_with("+++ "))
    {
        index += 1;
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    while index < diff_lines.len() {
        let header = diff_lines[index];
        if !header.starts_with("@@ ") {
            bail!(
                "unexpected line before hunk header: {:?}",
                header.chars().take(80).collect::<String>()
            );
        }
        let (old_start, mut old_count, new_count) = parse_hunk_header(header)?;
        index += 1;
        let mut old_lines = Vec::with_capacity(old_count);
        let mut new_lines = Vec::with_capacity(new_count);
        let mut old_seen = 0usize;
        while index < diff_lines.len() && !diff_lines[index].starts_with("@@ ") {
            let line = diff_lines[index];
            if line.starts_with("\\") {
                // "\ No newline at end of file" marker — tolerated, ignored.
                index += 1;
                continue;
            }
            if line.is_empty() {
                // Tolerate trailing newline-free lines inside a hunk body.
                bail!("empty line inside hunk body; use a explicit '-'/+'/' ' line");
            }
            match line.as_bytes()[0] {
                b' ' => {
                    let body = line[1..].to_string();
                    old_lines.push(body.clone());
                    new_lines.push(body);
                    old_seen += 1;
                }
                b'-' => {
                    old_lines.push(line[1..].to_string());
                    old_seen += 1;
                }
                b'+' => {
                    new_lines.push(line[1..].to_string());
                }
                _ => bail!(
                    "invalid line inside hunk body: {:?}",
                    line.chars().take(80).collect::<String>()
                ),
            }
            index += 1;
        }
        if old_count == 0 && old_seen == 0 {
            // A pure insertion hunk has no context; keep the single position.
            old_count = 0;
        }
        if old_seen != old_count {
            bail!(
                "hunk header announces {old_count} removed/context lines but body has {old_seen}"
            );
        }
        if new_lines.len() != new_count {
            bail!(
                "hunk header announces {new_count} added/context lines but body has {}",
                new_lines.len()
            );
        }
        hunks.push(Hunk {
            old_start,
            old_lines,
            new_lines,
        });
    }
    if hunks.is_empty() {
        bail!("diff contains no hunks");
    }

    // Apply hunks sequentially, allowing the file to grow/shrink via delta.
    let mut out: Vec<String> = Vec::with_capacity(original_lines.len());
    let mut cursor = 0usize;
    let mut delta: isize = 0;
    for hunk in &hunks {
        // Clamp defensively: a crafted header (e.g. start beyond isize::MAX)
        // must yield an out-of-range sentinel, never a wrapped index that can
        // panic the parser. A preferred position before the cursor is not
        // trusted; find_position rejects it instead of letting the caller
        // slice `original_lines[cursor..pos]` backwards.
        let preferred = hunk
            .old_start
            .saturating_sub(1)
            .saturating_add(delta.max(0) as usize);
        let pos = find_position(&original_lines, cursor, preferred, &hunk.old_lines)?;
        out.extend_from_slice(&original_lines[cursor..pos]);
        cursor = pos + hunk.old_lines.len();
        let grown = hunk.new_lines.len() as isize - hunk.old_lines.len() as isize;
        delta = delta
            .checked_add(grown)
            .context("unified-diff arithmetic overflow")?;
        out.extend(hunk.new_lines.iter().cloned());
    }
    out.extend_from_slice(&original_lines[cursor..]);
    let mut result = out.join("\n");
    // Preserve the original file's trailing-newline convention; a unified
    // diff carries line semantics, not whole-file EOL semantics, so the
    // faithful default is to keep whatever the original had.
    if original_trailing_newline && !out.is_empty() {
        result.push('\n');
    }
    Ok(result)
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize)> {
    let inner = header
        .strip_prefix("@@ ")
        .and_then(|rest| rest.split(" @@").next())
        .ok_or_else(|| anyhow::anyhow!("invalid hunk header: {header:?}"))?;
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() != 2 {
        bail!("invalid hunk header: {header:?}");
    }
    let old_start = parse_hunk_range(parts[0], '-')?;
    let new_count = parse_hunk_count(parts[1], '+')?;
    let old_count = parse_hunk_count(parts[0], '-')?;
    Ok((old_start, old_count, new_count))
}

fn parse_hunk_range(part: &str, sign: char) -> Result<usize> {
    let stripped = part
        .strip_prefix(sign)
        .ok_or_else(|| anyhow::anyhow!("invalid hunk range {part:?}"))?;
    let start = stripped.split(',').next().unwrap_or("");
    start
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid hunk start {start:?}"))
}

fn parse_hunk_count(part: &str, sign: char) -> Result<usize> {
    let stripped = part
        .strip_prefix(sign)
        .ok_or_else(|| anyhow::anyhow!("invalid hunk range {part:?}"))?;
    match stripped.split(',').nth(1) {
        None => Ok(1),
        Some(count) => count
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid hunk count {count:?}")),
    }
}

fn find_position(
    original: &[String],
    cursor: usize,
    preferred: usize,
    needle: &[String],
) -> Result<usize> {
    // Checked match test: a preferred or scanned position beyond the file
    // (or past an arithmetic edge) is simply "not a match", so a crafted
    // hunk header can never panic the parser on bounds arithmetic.
    let matches = |pos: usize| -> bool {
        let Some(end) = pos.checked_add(needle.len()) else {
            return false;
        };
        end <= original.len() && &original[pos..end] == needle
    };
    if preferred >= cursor && matches(preferred) {
        return Ok(preferred);
    }
    let end = original.len().saturating_sub(needle.len());
    for pos in cursor..=end {
        if matches(pos) {
            return Ok(pos);
        }
    }
    bail!(
        "hunk does not apply: expected lines not found at any position at or after line {}",
        cursor + 1
    )
}

/// Classification of a path's leaf component. Inspection errors other than
/// `NotFound` are fatal, never silently treated as nonexistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafClass {
    Ordinary,
    ReparsePoint,
    NotFound,
}

fn leaf_class(path: &Path) -> Result<LeafClass> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                Ok(LeafClass::ReparsePoint)
            } else {
                Ok(LeafClass::Ordinary)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LeafClass::NotFound),
        Err(e) => Err(e).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

/// TEST-ONLY deterministic interposition between validation and the
/// effect. Not for production use. The hook is one-shot and consumed only
/// when the observed path matches the armed name.
#[cfg(test)]
type HookEntry = (String, Box<dyn FnOnce(&Path) + Send>);

#[cfg(test)]
static HOOKS: Mutex<Vec<HookEntry>> = Mutex::new(Vec::new());

/// Arm a one-shot hook for a leaf file name. The next interposition
/// whose path ends with `leaf_name` runs the armed closure exactly once.
/// Multiple armed hooks may coexist; each fires only for its own leaf.
#[cfg(test)]
fn arm(leaf_name: &str, hook: impl FnOnce(&Path) + Send + 'static) {
    HOOKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((leaf_name.to_string(), Box::new(hook)));
}

/// If an armed hook matches `path`, run it exactly once and drop it.
#[cfg(test)]
fn consume_matching_hook(path: &Path) {
    let matched = {
        let mut hooks = HOOKS.lock().unwrap_or_else(|e| e.into_inner());
        let position = hooks
            .iter()
            .position(|(leaf, _)| path.ends_with(leaf.as_str()));
        position.map(|position| hooks.swap_remove(position))
    };
    if let Some((_, hook)) = matched {
        hook(path);
    }
}

#[cfg(test)]
fn interpose_after_validation(path: &Path) {
    consume_matching_hook(path);
}

#[cfg(not(test))]
fn interpose_after_validation(_path: &Path) {}

/// TEST-ONLY interposition between the creation decision and the anchored
/// creation of a missing directory component. Not for production use: the
/// production creation is anchored to the parent handle by construction.
#[cfg(test)]
fn interpose_at_creation(path: &Path) {
    consume_matching_hook(path);
}

#[cfg(not(test))]
fn interpose_at_creation(_path: &Path) {}

/// Split `full_path` into its canonical base plus the normal components
/// below it. Rejects absolute/prefix/parent components (defense in depth).
fn split_relative(base: &Path, full_path: &Path) -> Result<(PathBuf, Vec<std::ffi::OsString>)> {
    let base_canon = base
        .canonicalize()
        .with_context(|| format!("base {} is not resolvable", base.display()))?;
    let rel = full_path
        .strip_prefix(base)
        .with_context(|| format!("{} escapes base {}", full_path.display(), base.display()))?;
    let mut out = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(seg) => out.push(seg.to_os_string()),
            Component::CurDir => {}
            other => bail!("unsupported path component {other:?} in target"),
        }
    }
    Ok((base_canon, out))
}

/// Walk every component of `segments` from the base handle, opening each
/// one and verifying it is a plain, non-reparse directory. Returns the chain
/// of open handles (kept alive so the resolution cannot drift) and the path
/// of the deepest opened directory.
fn walk_dir_chain(
    base: &Path,
    segments: &[std::ffi::OsString],
) -> Result<(Vec<std::fs::File>, PathBuf)> {
    let canon = base.canonicalize()?;
    let mut handles: Vec<std::fs::File> = vec![open_dir(&canon)?];
    let mut cur = canon.clone();
    for segment in segments {
        let child = cur.join(segment);
        let md = std::fs::symlink_metadata(&child)
            .with_context(|| format!("failed to inspect component {}", child.display()))?;
        if md.file_type().is_symlink() {
            bail!(
                "component {} is a symlink/reparse point; refusing",
                child.display()
            );
        }
        if !md.file_type().is_dir() {
            bail!("component {} is not a directory; refusing", child.display());
        }
        let handle = open_child_dir(handles.last().expect("base handle"), &cur, segment)
            .with_context(|| format!("failed to open {}", child.display()))?;
        verify_plain_directory(&handle, &child)?;
        cur = child;
        handles.push(handle);
    }
    Ok((handles, cur))
}

/// Create missing directories along `segments` relative to the verified base.
///
/// Every directory we create is anchored to a parent handle that is already
/// verified as a plain, non-reparse directory: on Windows the child is
/// created exclusively with `NtCreateFile` (`FILE_CREATE`) against that
/// handle, so a reparse point raced in at the segment cannot redirect
/// creation — the create lands as a new plain directory inside the verified
/// chain or fails closed. On Unix the child is created with `mkdirat(2)`
/// against the verified parent directory fd and reopened with `openat`
/// (`O_NOFOLLOW|O_DIRECTORY`), so creation is anchored to the parent fd and
/// cannot be redirected by namespace swaps at or above the component; the
/// outside-creation race is closed on the fd-relative path. After creation
/// the chain is re-verified through live handles before any byte is written.
fn create_dir_chain(
    base: &Path,
    segments: &[std::ffi::OsString],
) -> Result<(Vec<std::fs::File>, PathBuf)> {
    let canon = base.canonicalize()?;
    let mut handles: Vec<std::fs::File> = vec![open_dir(&canon)?];
    let mut cur = canon.clone();
    for segment in segments {
        let child = cur.join(segment);
        match std::fs::symlink_metadata(&child) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    bail!(
                        "component {} is a symlink/reparse point; refusing",
                        child.display()
                    );
                }
                if !meta.file_type().is_dir() {
                    bail!("component {} is not a directory; refusing", child.display());
                }
                let handle = open_child_dir(handles.last().expect("base handle"), &cur, segment)?;
                verify_plain_directory(&handle, &child)?;
                cur = child;
                handles.push(handle);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let handle =
                    create_child_dir(handles.last().expect("base handle"), &cur, segment, &child)?;
                cur = child;
                handles.push(handle);
            }
            Err(e) => {
                bail!("failed to inspect component {}: {e}", child.display());
            }
        }
    }
    Ok((handles, cur))
}

/// Windows: create `segment` exclusively against the verified parent handle,
/// returning the pinned directory handle after verifying the created object
/// is a plain directory. Refuses on any non-success status or reparse mark.
#[cfg(windows)]
fn create_child_dir(
    parent: &std::fs::File,
    parent_path: &Path,
    segment: &std::ffi::OsStr,
    child_path: &Path,
) -> Result<std::fs::File> {
    interpose_at_creation(child_path);
    let name = nt_string(segment);
    let parent_raw = parent.as_raw_handle() as winapi::shared::ntdef::HANDLE;
    let (status, raw) = nt_create_file(
        &name,
        parent_raw,
        GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
        FILE_CREATE,
        true,
    );
    let Some(raw_handle) = raw else {
        bail!(
            "directory creation refused (anchored under {}; status {:#x})",
            parent_path.display(),
            status
        );
    };
    let handle = from_nt_handle(raw_handle, child_path)?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!(
            "anchored directory creation landed on a reparse point: {}",
            child_path.display()
        );
    }
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!(
            "anchored directory creation produced a non-directory object: {}",
            child_path.display()
        );
    }
    Ok(handle)
}

/// Unix: create `segment` exclusively against the verified parent directory
/// fd using `mkdirat(2)`, then open the new directory fd-relative with
/// `openat` (`O_NOFOLLOW|O_DIRECTORY`). Creation is anchored to the parent
/// fd, so a raced namespace swap cannot redirect the mkdir to create outside
/// the verified chain; a symlink/reparse raced in at the leaf is refused by
/// O_NOFOLLOW / mkdirat semantics. This closes the check/create/recheck race
/// on unauthorized directory creation.
#[cfg(unix)]
fn create_child_dir(
    parent: &std::fs::File,
    parent_path: &Path,
    segment: &std::ffi::OsStr,
    child_path: &Path,
) -> Result<std::fs::File> {
    interpose_at_creation(child_path);
    unix_fd::create_child_dir(parent, parent_path, segment, child_path)
}

/// Open `full_path` for writing under `base`, verifying every ancestor and
/// the leaf on handles. Fresh leaves are created atomically; existing leaves
/// are truncated only after the opened handle is verified as a plain file.
/// The returned handle is pinned to the verified object for its lifetime.
fn open_write_verified(base: &Path, full_path: &Path) -> Result<std::fs::File> {
    interpose_after_validation(full_path);
    let (base_canon, segments) = split_relative(base, full_path)?;
    let mut dir_segments = segments.clone();
    let leaf = dir_segments
        .pop()
        .context("open_write_verified requires a leaf component")?;

    let (_handles, leaf_parent) = create_dir_chain(&base_canon, &dir_segments)?;
    let leaf_path = leaf_parent.join(&leaf);

    match leaf_class(&leaf_path)? {
        LeafClass::ReparsePoint => bail!(
            "leaf {} is a symlink/reparse point; refusing to write",
            leaf_path.display()
        ),
        LeafClass::Ordinary => open_write_ordinary(&base_canon, &dir_segments, &leaf),
        LeafClass::NotFound => open_fresh_leaf_write(&base_canon, &dir_segments, &leaf),
    }
}

/// Open `full_path` for reading under `base`, verifying every component on
/// handles. The leaf must be a plain, non-reparse regular file.
fn open_read_verified(base: &Path, full_path: &Path) -> Result<std::fs::File> {
    interpose_after_validation(full_path);
    let (base_canon, segments) = split_relative(base, full_path)?;
    let mut dir_segments = segments.clone();
    let leaf = dir_segments
        .pop()
        .context("open_read_verified requires a leaf component")?;
    let (handles, _leaf_parent) = walk_dir_chain(&base_canon, &dir_segments)?;
    let _ = handles;
    open_leaf_read(&base_canon, &dir_segments, &leaf)
}

#[cfg(test)]
mod engine_unit_tests {
    use super::*;

    #[test]
    fn leaf_class_reports_not_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("absent.txt");
        assert_eq!(leaf_class(&path).expect("classify"), LeafClass::NotFound);
    }

    #[test]
    fn split_relative_rejects_parent_components() {
        let base = tempfile::tempdir().expect("tempdir");
        let target = base.path().join("..").join("escape.txt");
        assert!(split_relative(base.path(), &target).is_err());
    }
}

#[cfg(windows)]
const FILE_ATTRIBUTE_DIRECTORY: u32 = winapi::um::winnt::FILE_ATTRIBUTE_DIRECTORY;

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = winapi::um::winnt::FILE_ATTRIBUTE_REPARSE_POINT;

#[cfg(windows)]
const GENERIC_READ: u32 = 0x80000000;

#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x40000000;

#[cfg(windows)]
const SYNCHRONIZE: u32 = 0x00100000;

#[cfg(windows)]
const FILE_OPEN_REPARSE_POINT: u32 = 0x00200000;

#[cfg(windows)]
const FILE_DIRECTORY_FILE: u32 = 0x00000001;

#[cfg(windows)]
const FILE_NON_DIRECTORY_FILE: u32 = 0x00000040;

#[cfg(windows)]
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x00000020;

#[cfg(windows)]
const FILE_OPEN: u32 = 0x00000001;

#[cfg(windows)]
const FILE_CREATE: u32 = 0x00000002;

#[cfg(windows)]
const FILE_SHARE_READ: u32 = 0x00000001;

#[cfg(windows)]
const FILE_SHARE_WRITE: u32 = 0x00000002;

#[cfg(windows)]
const FILE_SHARE_DELETE: u32 = 0x00000004;

// NtCreateFile signature (x64). Declared here because winapi does not
// ship an NtCreateFile binding; no dependency is added.
#[repr(C)]
#[allow(non_snake_case)]
#[cfg(windows)]
struct IO_STATUS_BLOCK {
    Status: winapi::ctypes::c_long,
    Information: usize,
}

#[link(name = "ntdll")]
#[cfg(windows)]
unsafe extern "system" {
    fn NtCreateFile(
        FileHandle: *mut winapi::shared::ntdef::HANDLE,
        DesiredAccess: winapi::ctypes::c_ulong,
        ObjectAttributes: *mut winapi::shared::ntdef::OBJECT_ATTRIBUTES,
        IoStatusBlock: *mut IO_STATUS_BLOCK,
        AllocationSize: winapi::shared::ntdef::PLARGE_INTEGER,
        FileAttributes: winapi::ctypes::c_ulong,
        ShareAccess: winapi::ctypes::c_ulong,
        CreateDisposition: winapi::ctypes::c_ulong,
        CreateOptions: winapi::ctypes::c_ulong,
        EaBuffer: *mut winapi::ctypes::c_void,
        EaLength: winapi::ctypes::c_ulong,
    ) -> winapi::ctypes::c_long;
}

#[cfg(windows)]
const STATUS_SUCCESS: winapi::ctypes::c_long = 0;

#[cfg(windows)]
const STATUS_NOT_FOUND: winapi::ctypes::c_long = 0xC0000034u32 as winapi::ctypes::c_long;

#[cfg(windows)]
fn nt_string(segment: &std::ffi::OsStr) -> Vec<u16> {
    segment.encode_wide().collect()
}

/// Convert an absolute Windows path into an absolute NT path (`\??\C:...`).
#[cfg(windows)]
fn to_nt_path(path: &Path) -> Result<Vec<u16>> {
    let absolute = path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))?;
    let absolute = absolute
        .to_str()
        .with_context(|| format!("absolute path {} is not valid UTF-8", absolute.display()))?;
    // canonicalize() on Windows returns `\\?\C:\...`; strip that prefix so
    // the result is `\??\C:\...` exactly once.
    let absolute = absolute
        .strip_prefix(r"\\?\")
        .context("canonical path lacks the \\?\\ prefix")?;
    let nt = format!("\\??\\{absolute}");
    Ok(nt.encode_utf16().collect())
}

#[cfg(windows)]
fn attrs(handle: &std::fs::File) -> Result<u32> {
    let mut info: winapi::um::fileapi::BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let raw_handle = handle.as_raw_handle() as *mut winapi::ctypes::c_void;
    let ok = unsafe { winapi::um::fileapi::GetFileInformationByHandle(raw_handle, &mut info) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("GetFileInformationByHandle failed");
    }
    Ok(info.dwFileAttributes)
}

#[cfg(windows)]
const SHARE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

#[cfg(windows)]
fn nt_create_file(
    name: &[u16],
    root: winapi::shared::ntdef::HANDLE,
    desired_access: u32,
    create_disposition: u32,
    is_directory: bool,
) -> (
    winapi::ctypes::c_long,
    Option<winapi::shared::ntdef::HANDLE>,
) {
    let mut unicode = winapi::shared::ntdef::UNICODE_STRING {
        Length: (name.len() * 2) as u16,
        MaximumLength: (name.len() * 2) as u16,
        Buffer: name.as_ptr() as *mut u16,
    };
    let mut attrs: winapi::shared::ntdef::OBJECT_ATTRIBUTES = unsafe { std::mem::zeroed() };
    unsafe {
        winapi::shared::ntdef::InitializeObjectAttributes(
            &mut attrs,
            &mut unicode,
            winapi::shared::ntdef::OBJ_CASE_INSENSITIVE,
            root,
            std::ptr::null_mut(),
        );
    }
    let mut io = IO_STATUS_BLOCK {
        Status: 0,
        Information: 0,
    };
    let mut handle: winapi::shared::ntdef::HANDLE = std::ptr::null_mut();
    let opts = if is_directory {
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT
    } else {
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT
    };
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &mut attrs,
            &mut io,
            std::ptr::null_mut(),
            0,
            SHARE,
            create_disposition,
            opts,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return (status, None);
    }
    (status, Some(handle))
}

#[cfg(windows)]
fn from_nt_handle(raw: winapi::shared::ntdef::HANDLE, path: &Path) -> Result<std::fs::File> {
    if raw.is_null() || raw as usize == usize::MAX {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("nt handle invalid for {}", path.display()));
    }
    // SAFETY: `raw` is owned by the caller (nt_create_file success) and the
    // File takes ownership.
    Ok(unsafe { std::fs::File::from_raw_handle(raw as _) })
}

#[cfg(windows)]
fn open_dir(path: &Path) -> Result<std::fs::File> {
    let nt = to_nt_path(path)?;
    // Root open: directory, FILE_READ_DATA | SYNCHRONIZE.
    let (status, raw) = nt_create_file(
        &nt,
        std::ptr::null_mut(),
        GENERIC_READ | SYNCHRONIZE,
        FILE_OPEN,
        true,
    );
    if status != STATUS_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status))
            .with_context(|| format!("failed to open base directory {}", path.display()));
    }
    let handle = from_nt_handle(raw.unwrap(), path)?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("base {} is a reparse point; refusing", path.display());
    }
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!("base {} is not a directory; refusing", path.display());
    }
    Ok(handle)
}

#[cfg(windows)]
fn open_child_dir(
    parent: &std::fs::File,
    _parent_path: &Path,
    segment: &std::ffi::OsStr,
) -> Result<std::fs::File> {
    let name = nt_string(segment);
    let parent_raw = parent.as_raw_handle() as winapi::shared::ntdef::HANDLE;
    let (status, raw) = nt_create_file(
        &name,
        parent_raw,
        GENERIC_READ | SYNCHRONIZE,
        FILE_OPEN,
        true,
    );
    if status != STATUS_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status))
            .with_context(|| format!("open {} failed", segment.to_string_lossy()));
    }
    let handle = from_nt_handle(
        raw.unwrap(),
        Path::new(segment.to_str().unwrap_or("<non-unicode>")),
    )?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!(
            "component {} is a reparse point; refusing",
            segment.to_string_lossy()
        );
    }
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!(
            "component {} is not a directory; refusing",
            segment.to_string_lossy()
        );
    }
    Ok(handle)
}

#[cfg(windows)]
fn verify_plain_directory(handle: &std::fs::File, path: &Path) -> Result<()> {
    let a = attrs(handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("component {} is a reparse point; refusing", path.display());
    }
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!("component {} is not a directory; refusing", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn open_relative_file(
    parent: &std::fs::File,
    leaf: &std::ffi::OsStr,
    desired_access: u32,
    create_disposition: u32,
) -> Result<std::fs::File> {
    let name = nt_string(leaf);
    let parent_raw = parent.as_raw_handle() as winapi::shared::ntdef::HANDLE;
    let (status, raw) = nt_create_file(
        &name,
        parent_raw,
        desired_access | SYNCHRONIZE,
        create_disposition,
        false,
    );
    if status != STATUS_SUCCESS {
        if status == STATUS_NOT_FOUND {
            return Err(std::io::Error::from_raw_os_error(status))
                .with_context(|| format!("leaf {} not found", leaf.to_string_lossy()));
        }
        return Err(std::io::Error::from_raw_os_error(status))
            .with_context(|| format!("open {} failed", leaf.to_string_lossy()));
    }
    from_nt_handle(raw.unwrap(), Path::new(leaf))
}

#[cfg(windows)]
fn open_leaf_read(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    let (handles, _) = walk_dir_chain(base, dir)?;
    let parent = handles.last().expect("walk yields base handle");
    let handle = open_relative_file(parent, leaf.as_os_str(), GENERIC_READ, FILE_OPEN)?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("leaf is a reparse point; refusing to read");
    }
    if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
        bail!("leaf is a directory; refusing to read as file");
    }
    Ok(handle)
}

#[cfg(windows)]
fn open_write_ordinary(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    let (handles, _) = walk_dir_chain(base, dir)?;
    let parent = handles.last().expect("walk yields base handle");
    let handle = open_relative_file(
        parent,
        leaf.as_os_str(),
        GENERIC_READ | GENERIC_WRITE,
        FILE_OPEN,
    )?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("leaf is a reparse point; refusing to write");
    }
    if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
        bail!("leaf is a directory; refusing to write");
    }
    // Truncate only after the object has been verified.
    handle.set_len(0).context("truncate after verified open")?;
    Ok(handle)
}

#[cfg(windows)]
fn open_fresh_leaf_write(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    let (handles, _) = walk_dir_chain(base, dir)?;
    let parent = handles.last().expect("walk yields base handle");
    let handle = open_relative_file(
        parent,
        leaf.as_os_str(),
        GENERIC_READ | GENERIC_WRITE,
        FILE_CREATE,
    )?;
    let a = attrs(&handle)?;
    if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("leaf is a reparse point; refusing to write");
    }
    if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
        bail!("leaf is a directory; refusing to write");
    }
    Ok(handle)
}

#[cfg(unix)]
mod unix_fd {
    use anyhow::{Context, Result, anyhow, bail};
    use std::ffi::{CString, OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::path::{Path, PathBuf};

    fn cseg(segment: &OsStr) -> Result<CString> {
        // A single path component must not contain NUL; embedded separators
        // are impossible here because components are split already.
        CString::new(segment.as_bytes())
            .map_err(|_| anyhow!("path component contains NUL; refusing"))
    }

    fn errno(msg: &str, path: impl std::fmt::Display) -> anyhow::Error {
        anyhow::Error::from(std::io::Error::last_os_error()).context(format!("{msg}: {path}"))
    }

    /// Open an existing directory by absolute path (base/root of the walk).
    /// Path-based only for the initial anchor; subsequent steps are fd-relative.
    pub fn open_dir(path: &Path) -> Result<std::fs::File> {
        let before = std::fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect base {}", path.display()))?;
        if before.file_type().is_symlink() {
            bail!("base {} is a symlink; refusing", path.display());
        }
        if !before.file_type().is_dir() {
            bail!("base {} is not a directory; refusing", path.display());
        }
        let c = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| anyhow!("base path contains NUL; refusing"))?;
        let fd = unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(errno("failed to open base directory", path.display()));
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if meta.dev() != before.dev() || meta.ino() != before.ino() {
            bail!("base {} was swapped between check and open", path.display());
        }
        if !meta.file_type().is_dir() {
            bail!("base {} is not a directory; refusing", path.display());
        }
        Ok(file)
    }

    /// Open an existing child directory relative to `parent` without following
    /// a symlink/reparse point at the leaf. The operation is anchored to the
    /// verified parent fd: it cannot be redirected by swapping pathnames above
    /// or at the component during the open.
    pub fn open_child_dir(
        parent: &std::fs::File,
        _parent_path: &Path,
        segment: &OsStr,
        child_path: &Path,
    ) -> Result<std::fs::File> {
        let name = cseg(segment)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(errno(
                "failed to open directory component",
                child_path.display(),
            ));
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if !meta.file_type().is_dir() {
            bail!(
                "component {} is not a directory; refusing",
                child_path.display()
            );
        }
        Ok(file)
    }

    /// Create `segment` as a new directory relative to the verified `parent`
    /// fd, then open and return a pinned handle to it. `mkdirat` on a single
    /// component creates inside the directory referenced by `parent`'s fd and
    /// fails (EEXIST/ELOOP/ENOTDIR, etc.) rather than following or creating
    /// through a raced symlink at the leaf. This prevents unauthorized
    /// directory creation outside the verified chain even if namespace paths
    /// are swapped around the operation.
    pub fn create_child_dir(
        parent: &std::fs::File,
        _parent_path: &Path,
        segment: &OsStr,
        child_path: &Path,
    ) -> Result<std::fs::File> {
        let name = cseg(segment)?;
        let rc = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o777) };
        if rc != 0 {
            return Err(errno(
                "failed to create directory (fd-anchored)",
                child_path.display(),
            ));
        }
        // Open the directory we just created, anchored to the same parent fd
        // and refusing to follow a symlink at the leaf.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(errno(
                "failed to open newly created directory",
                child_path.display(),
            ));
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if !meta.file_type().is_dir() {
            bail!(
                "anchored directory creation produced a non-directory object: {}",
                child_path.display()
            );
        }
        Ok(file)
    }

    pub fn verify_plain_directory(handle: &std::fs::File, _path: &Path) -> Result<()> {
        let meta = handle.metadata()?;
        if !meta.file_type().is_dir() {
            bail!("component is not a directory; refusing");
        }
        Ok(())
    }

    fn open_leaf_fd(
        parent: &std::fs::File,
        leaf: &OsStr,
        flags: libc::c_int,
        mode: libc::mode_t,
    ) -> Result<std::fs::File> {
        let name = cseg(leaf)?;
        // Always refuse to follow a symlink at the final component.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode,
            )
        };
        if fd < 0 {
            return Err(errno("failed to open leaf", name.to_string_lossy()));
        }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    pub fn open_leaf_read(base: &Path, dir: &[OsString], leaf: &OsString) -> Result<std::fs::File> {
        let (handles, _leaf_parent) = super::walk_dir_chain(base, dir)?;
        let parent = handles.last().expect("base handle");
        let file = open_leaf_fd(parent, leaf.as_os_str(), libc::O_RDONLY, 0)
            .with_context(|| format!("failed to open leaf for read: {}", leaf.to_string_lossy()))?;
        let meta = file.metadata()?;
        if !meta.file_type().is_file() {
            bail!("leaf is not a regular file; refusing to read");
        }
        Ok(file)
    }

    pub fn open_write_ordinary(
        base: &Path,
        dir: &[OsString],
        leaf: &OsString,
    ) -> Result<std::fs::File> {
        let (handles, _leaf_parent) = super::walk_dir_chain(base, dir)?;
        let parent = handles.last().expect("base handle");
        let file = open_leaf_fd(parent, leaf.as_os_str(), libc::O_RDWR, 0).with_context(|| {
            format!("failed to open leaf for write: {}", leaf.to_string_lossy())
        })?;
        let meta = file.metadata()?;
        if !meta.file_type().is_file() {
            bail!("leaf is not a regular file; refusing to write");
        }
        file.set_len(0).context("truncate after verified open")?;
        Ok(file)
    }

    pub fn open_fresh_leaf_write(
        base: &Path,
        dir: &[OsString],
        leaf: &OsString,
    ) -> Result<std::fs::File> {
        let (handles, _leaf_parent) = super::walk_dir_chain(base, dir)?;
        let parent = handles.last().expect("base handle");
        let file = open_leaf_fd(
            parent,
            leaf.as_os_str(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o666,
        )
        .with_context(|| {
            format!(
                "failed to create leaf for write: {}",
                leaf.to_string_lossy()
            )
        })?;
        let meta = file.metadata()?;
        if !meta.file_type().is_file() {
            bail!("leaf is not a regular file; refusing to write");
        }
        Ok(file)
    }

    // Keep the PathBuf-building walk helpers available for callers that need
    // the leaf_parent path for messages; the fd-relative operations above are
    // the containment boundary.
    pub fn _leaf_parent_path(base: &Path, dir: &[OsString]) -> Result<PathBuf> {
        let (_h, p) = super::walk_dir_chain(base, dir)?;
        Ok(p)
    }
}

#[cfg(unix)]
fn open_dir(path: &Path) -> Result<std::fs::File> {
    unix_fd::open_dir(path)
}

#[cfg(unix)]
fn open_child_dir(
    parent: &std::fs::File,
    parent_path: &Path,
    segment: &std::ffi::OsStr,
) -> Result<std::fs::File> {
    let child_path = parent_path.join(segment);
    unix_fd::open_child_dir(parent, parent_path, segment, &child_path)
}

#[cfg(unix)]
fn verify_plain_directory(handle: &std::fs::File, path: &Path) -> Result<()> {
    unix_fd::verify_plain_directory(handle, path)
}

#[cfg(unix)]
fn open_leaf_read(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    unix_fd::open_leaf_read(base, dir, leaf)
}

#[cfg(unix)]
fn open_write_ordinary(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    unix_fd::open_write_ordinary(base, dir, leaf)
}

#[cfg(unix)]
fn open_fresh_leaf_write(
    base: &Path,
    dir: &[std::ffi::OsString],
    leaf: &std::ffi::OsString,
) -> Result<std::fs::File> {
    unix_fd::open_fresh_leaf_write(base, dir, leaf)
}

// ---------------------------------------------------------------------------
// Adversarial validation-effect swap interposition tests (the review's live
// race). These exercise the deterministic test-only hook between tool
// validation and the filesystem effect; they exist inside the crate so no
// new public surface is required. The outside sentinel must never change.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use crate::flow::Tool;

    /// Non-dereferencing check: is there a directory entry at `link` and is it a
    /// symlink/junction (reparse point)? Never follows the target.
    fn reparse_entry_present(link: &Path) -> bool {
        std::fs::symlink_metadata(link)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
    }

    /// Plant a reparse point at `link` targeting `target`. Prefers a true file
    /// symlink; falls back to a directory junction (`mklink /J`) where file
    /// symlinks are not permitted (Windows without elevation). Returns false
    /// when neither is permitted; callers then skip loudly instead of counting
    /// the test as evidence.
    fn junction_or_symlink(source: &Path, target: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, source).is_ok() && reparse_entry_present(source)
        }
        #[cfg(windows)]
        {
            if target.is_dir() {
                if std::os::windows::fs::symlink_dir(target, source).is_ok()
                    && reparse_entry_present(source)
                {
                    return true;
                }
                let out = std::process::Command::new("cmd")
                    .args(["/c", "mklink", "/J"])
                    .arg(source)
                    .arg(target)
                    .output();
                if let Ok(out) = out
                    && out.status.success()
                    && reparse_entry_present(source)
                {
                    return true;
                }
                false
            } else {
                std::os::windows::fs::symlink_file(target, source).is_ok()
                    && reparse_entry_present(source)
            }
        }
    }

    #[tokio::test]
    async fn existing_leaf_swap_during_validation_write_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("existing-write.txt"), "repo-original").unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();
        std::fs::write(victim_dir.join("sentinel.txt"), "outside-original").unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("existing-write.txt", move |_path| {
            let victim = outside_clone.join("victim-dir");
            let link = repo_clone.join("existing-write.txt");
            let _ = std::fs::remove_file(&link);
            let _ = junction_or_symlink(&link, &victim);
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "existing-write.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "an existing leaf swapped mid-effect must fail closed: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("victim-dir").join("sentinel.txt"))
                .unwrap(),
            "outside-original",
            "the outside sentinel file must not be written through the swapped link"
        );
    }

    #[tokio::test]
    async fn fresh_leaf_swap_during_validation_write_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();
        std::fs::write(victim_dir.join("sentinel.txt"), "outside-original").unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("fresh.txt", move |_path| {
            let victim = outside_clone.join("victim-dir");
            let link = repo_clone.join("fresh.txt");
            let _ = std::fs::remove_file(&link);
            let _ = junction_or_symlink(&link, &victim);
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "fresh.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "a fresh leaf swapped mid-effect must fail closed: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("victim-dir").join("sentinel.txt"))
                .unwrap(),
            "outside-original",
            "the outside victim file must not receive the written content"
        );
    }

    #[tokio::test]
    async fn ancestor_swap_during_validation_write_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();
        std::fs::write(victim_dir.join("sentinel.txt"), "outside-original").unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("victim.txt", move |_path| {
            let sub_orig = repo_clone.join("sub-orig");
            let _ = std::fs::rename(repo_clone.join("sub"), &sub_orig);
            let _ = junction_or_symlink(&repo_clone.join("sub"), &outside_clone);
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "sub/victim.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "a swapped ancestor must fail closed before the write effect: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("victim-dir").join("sentinel.txt"))
                .unwrap(),
            "outside-original",
            "the outside sentinel file must not be written through the swapped ancestor"
        );
        assert!(
            !outside.path().join("child.txt").exists(),
            "no file must be created outside the repo through the swapped ancestor"
        );
    }

    #[tokio::test]
    async fn patch_swap_during_validation_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("existing-patch.txt"), "repo-original").unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();
        std::fs::write(victim_dir.join("sentinel.txt"), "outside-original").unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("existing-patch.txt", move |_path| {
            let victim = outside_clone.join("victim-dir");
            let link = repo_clone.join("existing-patch.txt");
            let _ = std::fs::remove_file(&link);
            let _ = junction_or_symlink(&link, &victim);
        });

        let tool = PatchFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({
                "path": "existing-patch.txt",
                "diff": "--- a/existing-patch.txt\n+++ b/existing-patch.txt\n@@ -1 +1 @@\n-repo-original\n+owned\n"
            }))
            .await
            .expect("patch");
        assert_eq!(
            out["success"], false,
            "a leaf swapped mid-effect must fail closed for patch too: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("victim-dir").join("sentinel.txt"))
                .unwrap(),
            "outside-original",
            "the outside sentinel file must not be patched through the swapped link"
        );
    }

    #[tokio::test]
    async fn read_swap_during_validation_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("read-target.txt"), "repo-content").unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();
        std::fs::write(victim_dir.join("sentinel.txt"), "outside-original").unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("read-target.txt", move |_path| {
            let victim = outside_clone.join("victim-dir");
            let link = repo_clone.join("read-target.txt");
            let _ = std::fs::remove_file(&link);
            let _ = junction_or_symlink(&link, &victim);
        });

        let tool = RepoReadFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "read-target.txt"}))
            .await
            .expect("read");
        assert_eq!(
            out["success"], false,
            "a leaf swapped mid-effect must fail closed for reads too: {out}"
        );
    }

    #[tokio::test]
    async fn dangling_final_component_during_validation_write_fails_closed() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        // Target deliberately absent (dangling junction — target directory
        // never created). mklink /J handles a nonexistent target.
        let link = repo.join("dangling.txt");
        let absent_target = outside.path().join("never-created-dir");
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&absent_target)
            .output();
        match out {
            Ok(o) if o.status.success() => {}
            _ => panic!(
                "reparse point creation not permitted on this host; host cannot produce the required fixture"
            ),
        }
        assert!(
            std::fs::symlink_metadata(&link).is_ok(),
            "the junction itself must exist after creation"
        );

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "dangling.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "a dangling final-component link must fail closed: {out}"
        );
        assert!(
            !outside.path().join("never-created-dir").exists(),
            "the dangling outside directory must never be created by a refused write"
        );
    }

    /// FIX A regression (Windows): a reparse point raced in *at directory
    /// creation time* must not create directories outside the repo. The
    /// name-based `create_dir_all` used previously followed the junction and
    /// created the directory through it; anchored creation refuses.
    #[cfg(windows)]
    #[tokio::test]
    async fn write_during_ancestor_swap_at_creation_creates_nothing_outside_repo() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let victim_dir = outside.path().join("victim-dir");
        std::fs::create_dir_all(&victim_dir).unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        // Fires once, between the stat that says "missing" and the anchored
        // creation of "a" (the write target is sub/a/file.txt).
        arm("a", move |_path| {
            let sub_orig = repo_clone.join("sub-orig");
            let _ = std::fs::rename(repo_clone.join("sub"), &sub_orig);
            let _ = junction_or_symlink(&repo_clone.join("sub"), &outside_clone.join("victim-dir"));
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "sub/a/file.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "a write through a raced ancestor must fail closed: {out}"
        );
        assert!(
            std::fs::read_dir(&victim_dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false),
            "no directory may be created outside the repo through a raced ancestor: {}",
            victim_dir.display()
        );
        assert!(
            !outside.path().join("victim-dir/a").exists(),
            "the segment under the raced ancestor must not be created outside the repo"
        );
    }

    /// FIX B regression: crafted headers with extreme start values must not
    /// wrap the delta arithmetic or panic the parser; extremes that cannot
    /// apply are rejected or applied cleanly, but the call must not panic.
    #[test]
    fn parser_is_panic_free_on_extreme_hunk_starts() {
        let diff = "--- a/f\n+++ b/f\n@@ -18446744073709551615,1 +1,1 @@\n existing\n";
        assert_eq!(
            apply_unified_diff("existing\n", diff).unwrap(),
            "existing\n"
        );
        let diff = "--- a/f\n+++ b/f\n@@ -9223372036854775806,1 +1,1 @@\n b\n";
        assert_eq!(apply_unified_diff("a\nb\n", diff).unwrap(), "a\nb\n");
    }

    /// FIX B regression: hunks whose preferred position falls behind the
    /// cursor panic the old parser (`original_lines[cursor..pos]` with
    /// pos < cursor); they must be rejected as "hunk does not apply".
    #[test]
    fn parser_rejects_out_of_order_hunks_without_panicking() {
        let diff = "--- a/f\n+++ b/f\n@@ -2,1 +2,1 @@\n b\n@@ -2,1 +2,1 @@\n b\n";
        assert!(
            apply_unified_diff("a\nb\n", diff).is_err(),
            "out-of-order hunk must be rejected, not panic"
        );
    }

    /// FIX B regression: a large negative delta followed by an early start
    /// used to produce a wrapped `usize::MAX - delta` preferred index that
    /// panicked on bounds arithmetic; now the parser rejects cleanly.
    #[test]
    fn parser_rejects_hunks_before_file_start_without_panicking() {
        let diff = "--- a/f\n+++ b/f\n@@ -1,2 +1,0 @@\n-a\n-b\n@@ -1,1 +1,1 @@\n c\n";
        assert!(
            apply_unified_diff("a\nb\n", diff).is_err(),
            "hunk before the file start must be rejected, not panic"
        );
    }

    /// FIX B regression: malformed grammar variants are all rejected with
    /// typed errors.
    #[test]
    fn parser_rejects_malformed_diffs() {
        for diff in [
            "",
            "\n",
            "garbage",
            "--- a/f\n",
            "@@ -1 +1 @@\nextra-junk\n",
            "@@ -1,1 +1,1 @@\n",
            "@@ -1,1 +1,2 @@\n-x\n+y\n",
            "@@ -1,1 +1,1 @@\n empty body first line is not empty; this hunk lacks a leading space\n",
        ] {
            assert!(
                apply_unified_diff("a\nb\n", diff).is_err(),
                "expected rejection: {diff:?}"
            );
        }
    }

    /// FIX B regression: bounded property probe - thousands of deterministic
    /// pseudo-random odd diffs must not panic. Panics fail the test itself;
    /// rejections are expected and ignored.
    #[test]
    fn parser_has_no_panic_on_randomish_diffs() {
        let mut state: u64 = 0x8f3a_1c92_b4d5_7e6f;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let hunk = format!("@@ -{} +{} @@\n", next() % 10, next() % 10);
            let body: String = (0..3)
                .map(|_| match next() % 3 {
                    0 => " context\n",
                    1 => "-del\n",
                    _ => "+add\n",
                })
                .collect();
            let diff = format!("--- a/f\n+++ b/f\n{hunk}{body}");
            let _ = apply_unified_diff("a\nb\nc\n", &diff);
        }
    }

    /// Dispatch 318: ancestor swapped immediately before mkdir (fd-relative
    /// creation). Creation must be anchored to the verified parent fd so no
    /// directory is created outside the repo, even if a namespace ancestor is
    /// replaced with a symlink to outside at the creation interposition point.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_ancestor_swapped_immediately_before_mkdir_creates_nothing_outside_repo() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let victim = outside.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("a", move |_path| {
            let sub_orig = repo_clone.join("sub-orig");
            let _ = std::fs::rename(repo_clone.join("sub"), &sub_orig);
            let _ = junction_or_symlink(&repo_clone.join("sub"), &outside_clone.join("victim"));
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "sub/a/file.txt", "content": "owned"}))
            .await
            .expect("write");
        // The operation must not create anything outside the repo. The call
        // itself may fail closed (namespace changed under a held fd context)
        // or succeed only inside the verified directory object; either way the
        // outside sentinel stays empty.
        assert!(
            std::fs::read_dir(&victim)
                .map(|mut e| e.next().is_none())
                .unwrap_or(false),
            "no directory may be created outside the repo: {out}"
        );
        assert!(
            !outside.path().join("victim/a").exists(),
            "segment must not appear outside repo: {out}"
        );
        let _ = out; // success/failure is implementation-dependent; containment is the invariant
    }

    /// Dispatch 318: ancestor swapped during creation (same interposition
    /// window, fd-anchored mkdirat). Outside directory must remain absent.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_ancestor_swapped_during_creation_creates_nothing_outside_repo() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.join("p/q")).unwrap();
        let victim = outside.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();

        let repo_clone = repo.clone();
        let outside_clone = outside.path().to_path_buf();
        arm("newdir", move |_path| {
            // Swap an ancestor (p) to point outside while creating "newdir".
            let p_orig = repo_clone.join("p-orig");
            let _ = std::fs::rename(repo_clone.join("p"), &p_orig);
            let _ = junction_or_symlink(&repo_clone.join("p"), &outside_clone.join("victim"));
        });

        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "p/q/newdir/file.txt", "content": "owned"}))
            .await
            .expect("write");
        assert!(
            std::fs::read_dir(&victim)
                .map(|mut e| e.next().is_none())
                .unwrap_or(false),
            "no directory may be created outside the repo during creation race: {out}"
        );
        assert!(
            !outside.path().join("victim/q").exists()
                && !outside.path().join("victim/newdir").exists(),
            "no segment may appear outside repo: {out}"
        );
    }

    /// Dispatch 318: existing symlink ancestor on the directory-creation path
    /// must be refused (no traversal, no creation through it).
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_existing_symlink_ancestor_refused_on_nested_create() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(outside.path().join("victim")).unwrap();
        assert!(
            junction_or_symlink(&repo.join("link"), &outside.path().join("victim")),
            "symlink creation must be permitted in this environment"
        );
        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "link/newdir/file.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "existing symlink ancestor must fail closed: {out}"
        );
        assert!(
            !outside.path().join("victim/newdir").exists(),
            "must not create through existing symlink ancestor"
        );
    }

    /// Dispatch 318: dangling symlink ancestor must be refused and must not
    /// create the dangling target or any outside directory.
    #[cfg(unix)]
    #[tokio::test]
    async fn unix_dangling_symlink_ancestor_refused_on_nested_create() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(&repo).unwrap();
        // dangling: target does not exist
        std::os::unix::fs::symlink(outside.path().join("nope"), repo.join("dang")).unwrap();
        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "dang/newdir/file.txt", "content": "owned"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], false,
            "dangling symlink ancestor must fail closed: {out}"
        );
        assert!(
            !outside.path().join("nope").exists(),
            "dangling target must never be created"
        );
    }

    /// Dispatch 318: ordinary authorized nested directory creation still works
    /// (supported path, no symlink/race).
    #[tokio::test]
    async fn ordinary_authorized_nested_directory_creation_still_works() {
        let base = tempfile::tempdir().expect("tempdir");
        let repo = base.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let tool = WriteFileTool::new(repo.clone());
        let out = tool
            .call(serde_json::json!({"path": "a/b/c/file.txt", "content": "owned-nested"}))
            .await
            .expect("write");
        assert_eq!(
            out["success"], true,
            "ordinary nested creation must work: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("a/b/c/file.txt")).unwrap(),
            "owned-nested"
        );
    }
}
