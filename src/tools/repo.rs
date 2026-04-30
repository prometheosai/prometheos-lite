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
use crate::tools::{ToolContext, ToolMetadata};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// RepoTool trait for repository operations
pub trait RepoTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, input: serde_json::Value) -> Result<serde_json::Value>;
}

/// List tree tool - lists files and directories in a repository
pub struct ListTreeTool {
    repo_path: PathBuf,
}

impl ListTreeTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
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
        let root = input.get("root")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let depth = input.get("depth")
            .and_then(|v| v.as_u64())
            .map(|d| d as u32);

        let base_path = if root.is_empty() {
            self.repo_path.clone()
        } else {
            self.repo_path.join(root)
        };

        let mut files = Vec::new();
        let mut dirs = Vec::new();

        if let Some(max_depth) = depth {
            self.list_tree_recursive(&base_path, 0, max_depth, &mut files, &mut dirs).await?;
        } else {
            self.list_tree_recursive(&base_path, 0, u32::MAX, &mut files, &mut dirs).await?;
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
            
            // Skip common ignored directories
            if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                if name == "target" || name == "node_modules" || name == ".git" || name == ".next" {
                    continue;
                }
            }

            if entry_path.is_dir() {
                dirs.push(entry_path.to_string_lossy().to_string());
                Box::pin(self.list_tree_recursive(&entry_path, current_depth + 1, max_depth, files, dirs)).await?;
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
}

impl RepoReadFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
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
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        let full_path = self.repo_path.join(path);

        if !full_path.exists() {
            return Ok(serde_json::json!({
                "error": format!("File not found: {}", full_path.display()),
                "success": false
            }));
        }

        let content = tokio::fs::read_to_string(&full_path)
            .await
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
        let query = input.get("query")
            .and_then(|v| v.as_str())
            .context("Missing query")?;

        let glob = input.get("glob")
            .and_then(|v| v.as_str());

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
    async fn search_repo(&self, query: &str, glob: Option<&str>, results: &mut Vec<serde_json::Value>) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.repo_path)
            .await
            .context("Failed to read repo directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "target" || name == "node_modules" || name == ".git" {
                        continue;
                    }
                }

                let mut sub_entries = tokio::fs::read_dir(&path).await?;
                while let Some(sub_entry) = sub_entries.next_entry().await? {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file() {
                        if let Some(glob_pattern) = glob {
                            if !self.matches_glob(&sub_path, glob_pattern) {
                                continue;
                            }
                        }
                        self.search_file(&sub_path, query, results).await?;
                    }
                }
            } else if path.is_file() {
                if let Some(glob_pattern) = glob {
                    if !self.matches_glob(&path, glob_pattern) {
                        continue;
                    }
                }
                self.search_file(&path, query, results).await?;
            }
        }

        Ok(())
    }

    async fn search_file(&self, path: &Path, query: &str, results: &mut Vec<serde_json::Value>) -> Result<()> {
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
}

impl WriteFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
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
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        let content = input.get("content")
            .and_then(|v| v.as_str())
            .context("Missing content")?;

        let full_path = self.repo_path.join(path);

        // Create parent directories if they don't exist
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create parent directories")?;
        }

        tokio::fs::write(&full_path, content)
            .await
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
}

impl PatchFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
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
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .context("Missing path")?;

        let diff = input.get("diff")
            .and_then(|v| v.as_str())
            .context("Missing diff")?;

        let full_path = self.repo_path.join(path);

        // Validate file exists
        if !full_path.exists() {
            return Ok(serde_json::json!({
                "error": format!("File not found: {}", full_path.display()),
                "success": false,
                "validation": "failed"
            }));
        }

        // Validate diff format
        if !self.validate_diff_format(diff) {
            return Ok(serde_json::json!({
                "error": "Invalid diff format",
                "success": false,
                "validation": "failed"
            }));
        }

        // Read original content
        let original_content = tokio::fs::read_to_string(&full_path)
            .await
            .context("Failed to read original file")?;

        // Apply patch
        let patched_content = self.apply_patch(&original_content, diff)
            .context("Failed to apply patch")?;

        // Write patched content
        tokio::fs::write(&full_path, &patched_content)
            .await
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
    fn validate_diff_format(&self, diff: &str) -> bool {
        // Basic validation: check for diff headers
        let lines: Vec<&str> = diff.lines().collect();
        if lines.is_empty() {
            return false;
        }

        // Check for diff header
        let has_header = lines.iter().any(|line| line.starts_with("---") || line.starts_with("+++"));
        // Check for hunk headers
        let has_hunk = lines.iter().any(|line| line.starts_with("@@"));

        has_header && has_hunk
    }

    fn apply_patch(&self, original: &str, diff: &str) -> Result<String> {
        // Simple diff application - for production, use a proper diff library
        // This is a simplified implementation that handles basic unified diffs
        let mut result_lines: Vec<&str> = original.lines().collect();
        let diff_lines: Vec<&str> = diff.lines().collect();
        let mut i = 0;

        while i < diff_lines.len() {
            let line = diff_lines[i];

            if line.starts_with("@@") {
                // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let old_info = parts[1]; // -old_start,old_count
                    let new_info = parts[2]; // +new_start,new_count

                    let old_parts: Vec<&str> = old_info.trim_start_matches('-').split(',').collect();
                    let old_start: usize = old_parts[0].parse().unwrap_or(1);
                    let old_count: usize = if old_parts.len() > 1 { old_parts[1].parse().unwrap_or(1) } else { 1 };

                    let new_parts: Vec<&str> = new_info.trim_start_matches('+').split(',').collect();
                    let new_start: usize = new_parts[0].parse().unwrap_or(1);

                    let mut old_idx = old_start - 1;
                    let mut new_lines = Vec::new();

                    i += 1;
                    while i < diff_lines.len() && !diff_lines[i].starts_with("@@") && !diff_lines[i].starts_with("---") && !diff_lines[i].starts_with("+++") {
                        let diff_line = diff_lines[i];

                        if diff_line.starts_with(' ') {
                            // Context line - keep original
                            if old_idx < result_lines.len() {
                                new_lines.push(result_lines[old_idx]);
                            }
                            old_idx += 1;
                        } else if diff_line.starts_with('-') {
                            // Remove line
                            old_idx += 1;
                        } else if diff_line.starts_with('+') {
                            // Add line
                            new_lines.push(&diff_line[1..]);
                        }

                        i += 1;
                    }

                    // Replace the old lines with new lines
                    let replace_start = old_start - 1;
                    if replace_start + old_count <= result_lines.len() {
                        result_lines.splice(replace_start..replace_start + old_count, new_lines);
                    }

                    continue;
                }
            }

            i += 1;
        }

        Ok(result_lines.join("\n"))
    }

    fn count_changes(&self, diff: &str) -> usize {
        diff.lines()
            .filter(|line| line.starts_with('+') || line.starts_with('-'))
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
        let cached = input.get("cached")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = Command::new("git");
        cmd.arg("diff");
        if cached {
            cmd.arg("--cached");
        }
        cmd.current_dir(&self.repo_path);

        let output = cmd.output()
            .context("Failed to execute git diff")?;

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
        tokio::fs::create_dir_all(repo_path.join("src")).await.unwrap();
        tokio::fs::write(repo_path.join("src/main.rs"), "fn main() {}").await.unwrap();
        tokio::fs::write(repo_path.join("README.md"), "# Test").await.unwrap();

        let tool = ListTreeTool::new(repo_path.to_path_buf());
        let result = tool.call(serde_json::json!({})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["files"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        tokio::fs::write(repo_path.join("test.txt"), "Hello, World!").await.unwrap();

        let tool = RepoReadFileTool::new(repo_path.to_path_buf());
        let result = tool.call(serde_json::json!({"path": "test.txt"})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["content"].as_str().unwrap(), "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = WriteFileTool::new(repo_path.to_path_buf());
        let result = tool.call(serde_json::json!({
            "path": "new_file.txt",
            "content": "New content"
        })).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert_eq!(result["bytes_written"].as_u64().unwrap(), 11);

        // Verify file was written
        let content = tokio::fs::read_to_string(repo_path.join("new_file.txt")).await.unwrap();
        assert_eq!(content, "New content");
    }

    #[tokio::test]
    async fn test_search_files() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        tokio::fs::write(repo_path.join("test.rs"), "fn main() { println!(\"hello\"); }").await.unwrap();
        tokio::fs::write(repo_path.join("test.txt"), "hello world").await.unwrap();

        let tool = SearchFilesTool::new(repo_path.to_path_buf());
        let result = tool.call(serde_json::json!({"query": "hello"})).await.unwrap();

        assert!(result["success"].as_bool().unwrap());
        assert!(result["count"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_patch_file_validation() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = PatchFileTool::new(repo_path.to_path_buf());

        // Invalid diff format
        assert!(!tool.validate_diff_format("not a diff"));

        // Valid diff format
        let valid_diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,1 +1,1 @@\n-old\n+new";
        assert!(tool.validate_diff_format(valid_diff));
    }

    #[test]
    fn test_count_changes() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let tool = PatchFileTool::new(repo_path.to_path_buf());

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n+another";
        assert_eq!(tool.count_changes(diff), 3);
    }
}
