//! Structured repo-aware coding tools
//!
//! This module provides tools for code analysis, navigation, and manipulation
//! that are aware of repository structure and can perform structured operations.

use crate::flow::Tool;
use crate::tools::{ToolContext, ToolMetadata};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Read file tool - reads a file from the repository
pub struct ReadFileTool {
    repo_path: PathBuf,
}

impl ReadFileTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> String {
        "read_file".to_string()
    }

    fn description(&self) -> String {
        "Reads a file from the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file relative to repository root"
                }
            },
            "required": ["file_path"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let file_path = input.get("file_path")
            .and_then(|v| v.as_str())
            .context("Missing file_path")?;

        let full_path = self.repo_path.join(file_path);
        
        if !full_path.exists() {
            return Ok(serde_json::json!({
                "error": format!("File not found: {}", full_path.display()),
                "success": false
            }));
        }

        let content = tokio::fs::read_to_string(&full_path).await
            .context("Failed to read file")?;

        Ok(serde_json::json!({
            "content": content,
            "file_path": file_path,
            "success": true
        }))
    }
}

/// Search code tool - searches for patterns across the codebase
pub struct SearchCodeTool {
    repo_path: PathBuf,
}

impl SearchCodeTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for SearchCodeTool {
    fn name(&self) -> String {
        "search_code".to_string()
    }

    fn description(&self) -> String {
        "Searches for a pattern across the codebase".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Pattern to search for"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Optional file pattern to filter (e.g., '*.rs')"
                }
            },
            "required": ["pattern"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let pattern = input.get("pattern")
            .and_then(|v| v.as_str())
            .context("Missing pattern")?;

        let file_pattern = input.get("file_pattern")
            .and_then(|v| v.as_str());

        let mut results = Vec::new();
        self.search_repo(pattern, file_pattern, &mut results).await?;

        Ok(serde_json::json!({
            "pattern": pattern,
            "results": results,
            "count": results.len(),
            "success": true
        }))
    }
}

impl SearchCodeTool {
    async fn search_repo(&self, pattern: &str, file_pattern: Option<&str>, results: &mut Vec<serde_json::Value>) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.repo_path).await
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
                        if let Some(file_pattern) = file_pattern {
                            if !self.matches_pattern(&sub_path, file_pattern) {
                                continue;
                            }
                        }
                        self.search_file(&sub_path, pattern, results).await?;
                    }
                }
            } else if path.is_file() {
                if let Some(file_pattern) = file_pattern {
                    if !self.matches_pattern(&path, file_pattern) {
                        continue;
                    }
                }
                self.search_file(&path, pattern, results).await?;
            }
        }
        
        Ok(())
    }

    async fn search_file(&self, path: &Path, pattern: &str, results: &mut Vec<serde_json::Value>) -> Result<()> {
        let content = tokio::fs::read_to_string(path).await
            .context("Failed to read file")?;
        
        for (line_num, line) in content.lines().enumerate() {
            if line.contains(pattern) {
                results.push(serde_json::json!({
                    "file": path.to_string_lossy(),
                    "line": line_num + 1,
                    "content": line,
                    "pattern": pattern
                }));
            }
        }
        
        Ok(())
    }

    fn matches_pattern(&self, path: &Path, pattern: &str) -> bool {
        if let Some(extension) = path.extension() {
            let pattern_ext = pattern.trim_start_matches('*').trim_start_matches('.');
            return extension.to_str() == Some(pattern_ext);
        }
        false
    }
}

/// List files tool - lists files in the repository
pub struct ListFilesTool {
    repo_path: PathBuf,
}

impl ListFilesTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> String {
        "list_files".to_string()
    }

    fn description(&self) -> String {
        "Lists files in the repository".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to list (relative to repo root, empty for root)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Whether to list recursively",
                    "default": false
                }
            }
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let directory = input.get("directory")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let recursive = input.get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let base_path = if directory.is_empty() {
            self.repo_path.clone()
        } else {
            self.repo_path.join(directory)
        };

        let mut files = Vec::new();
        
        if recursive {
            self.list_recursive(&base_path, &mut files).await?;
        } else {
            self.list_directory(&base_path, &mut files).await?;
        }

        Ok(serde_json::json!({
            "directory": directory,
            "files": files,
            "count": files.len(),
            "success": true
        }))
    }
}

impl ListFilesTool {
    async fn list_directory(&self, path: &Path, files: &mut Vec<serde_json::Value>) -> Result<()> {
        let mut entries = tokio::fs::read_dir(path).await
            .context("Failed to read directory")?;
        
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let is_dir = entry_path.is_dir();
            
            files.push(serde_json::json!({
                "name": entry.file_name().to_string_lossy().to_string(),
                "path": entry_path.to_string_lossy().to_string(),
                "is_directory": is_dir
            }));
        }
        
        Ok(())
    }

    async fn list_recursive(&self, path: &Path, files: &mut Vec<serde_json::Value>) -> Result<()> {
        let mut entries = tokio::fs::read_dir(path).await
            .context("Failed to read directory")?;
        
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let is_dir = entry_path.is_dir();
            
            files.push(serde_json::json!({
                "name": entry.file_name().to_string_lossy().to_string(),
                "path": entry_path.to_string_lossy().to_string(),
                "is_directory": is_dir
            }));
            
            if is_dir {
                if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                    if name != "target" && name != "node_modules" && name != ".git" {
                        Box::pin(self.list_recursive(&entry_path, files)).await?;
                    }
                }
            }
        }
        
        Ok(())
    }
}

/// Get file info tool - gets metadata about a file
pub struct GetFileInfoTool {
    repo_path: PathBuf,
}

impl GetFileInfoTool {
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

#[async_trait::async_trait]
impl Tool for GetFileInfoTool {
    fn name(&self) -> String {
        "get_file_info".to_string()
    }

    fn description(&self) -> String {
        "Gets metadata about a file".to_string()
    }

    fn input_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file relative to repository root"
                }
            },
            "required": ["file_path"]
        }))
    }

    async fn call(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let file_path = input.get("file_path")
            .and_then(|v| v.as_str())
            .context("Missing file_path")?;

        let full_path = self.repo_path.join(file_path);
        
        if !full_path.exists() {
            return Ok(serde_json::json!({
                "error": format!("File not found: {}", full_path.display()),
                "success": false
            }));
        }

        let metadata = tokio::fs::metadata(&full_path).await
            .context("Failed to get file metadata")?;

        let extension = full_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");

        let language = self.detect_language(extension);

        Ok(serde_json::json!({
            "file_path": file_path,
            "size_bytes": metadata.len(),
            "is_file": metadata.is_file(),
            "is_directory": metadata.is_dir(),
            "extension": extension,
            "language": language,
            "success": true
        }))
    }
}

impl GetFileInfoTool {
    fn detect_language(&self, extension: &str) -> &str {
        match extension {
            "rs" => "rust",
            "py" => "python",
            "js" | "jsx" | "ts" | "tsx" => "javascript",
            "go" => "go",
            "java" => "java",
            "cpp" | "cc" | "cxx" | "h" | "hpp" => "cpp",
            "c" => "c",
            "md" => "markdown",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            _ => "unknown",
        }
    }
}
