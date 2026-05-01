//! Coding harness nodes - repo-aware nodes for code operations
//!
//! This module provides flow nodes that are aware of repository structure
//! and can perform code analysis, navigation, and manipulation operations.

use crate::flow::SharedState;
use crate::flow::node::{Node, NodeConfig};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Code analysis node - analyzes code structure and dependencies
pub struct CodeAnalysisNode {
    config: NodeConfig,
    repo_path: PathBuf,
}

impl CodeAnalysisNode {
    pub fn new(config: NodeConfig, repo_path: PathBuf) -> Self {
        Self { config, repo_path }
    }
}

#[async_trait::async_trait]
impl Node for CodeAnalysisNode {
    fn id(&self) -> String {
        "code_analysis".to_string()
    }

    fn kind(&self) -> &str {
        "code_analysis"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let file_path = state
            .get_input("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(json!({
            "file_path": file_path,
            "repo_path": self.repo_path.to_string_lossy()
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let file_path = input
            .get("file_path")
            .and_then(|v| v.as_str())
            .context("Missing file_path in input")?;

        let full_path = self.repo_path.join(file_path);

        if !full_path.exists() {
            return Ok(json!({
                "error": format!("File not found: {}", full_path.display()),
                "status": "error"
            }));
        }

        let content = tokio::fs::read_to_string(&full_path)
            .await
            .context("Failed to read file")?;

        let extension = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");

        let line_count = content.lines().count();
        let language = self.detect_language(extension);

        Ok(json!({
            "file_path": file_path,
            "language": language,
            "line_count": line_count,
            "size_bytes": content.len(),
            "status": "success"
        }))
    }

    fn post(&self, _state: &mut SharedState, output: serde_json::Value) -> String {
        if output.get("status").and_then(|v| v.as_str()) == Some("error") {
            "error".to_string()
        } else {
            "continue".to_string()
        }
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

impl CodeAnalysisNode {
    fn detect_language(&self, extension: &str) -> &str {
        match extension {
            "rs" => "rust",
            "py" => "python",
            "js" | "jsx" | "ts" | "tsx" => "javascript",
            "go" => "go",
            "java" => "java",
            "cpp" | "cc" | "cxx" | "h" | "hpp" => "cpp",
            "c" => "c",
            _ => "unknown",
        }
    }
}

/// Symbol resolution node - resolves symbols across the codebase
pub struct SymbolResolutionNode {
    config: NodeConfig,
    repo_path: PathBuf,
}

impl SymbolResolutionNode {
    pub fn new(config: NodeConfig, repo_path: PathBuf) -> Self {
        Self { config, repo_path }
    }
}

#[async_trait::async_trait]
impl Node for SymbolResolutionNode {
    fn id(&self) -> String {
        "symbol_resolution".to_string()
    }

    fn kind(&self) -> &str {
        "symbol_resolution"
    }

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let symbol = state
            .get_input("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let file_path = state
            .get_input("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(json!({
            "symbol": symbol,
            "file_path": file_path,
            "repo_path": self.repo_path.to_string_lossy()
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let symbol = input
            .get("symbol")
            .and_then(|v| v.as_str())
            .context("Missing symbol in input")?;

        let file_path = input.get("file_path").and_then(|v| v.as_str());

        let mut definitions = Vec::new();

        if let Some(fp) = file_path {
            let full_path = self.repo_path.join(fp);
            if full_path.exists() {
                if let Some(def) = self.search_file_for_symbol(&full_path, symbol).await? {
                    definitions.push(def);
                }
            }
        }

        Ok(json!({
            "symbol": symbol,
            "definitions": definitions,
            "status": "success"
        }))
    }

    fn post(&self, _state: &mut SharedState, output: serde_json::Value) -> String {
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

impl SymbolResolutionNode {
    async fn search_file_for_symbol(
        &self,
        path: &Path,
        symbol: &str,
    ) -> Result<Option<serde_json::Value>> {
        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read file")?;

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            if line.contains(symbol)
                && (line.starts_with("fn ")
                    || line.starts_with("pub fn ")
                    || line.starts_with("struct ")
                    || line.starts_with("pub struct ")
                    || line.starts_with("type ")
                    || line.starts_with("def ")
                    || line.starts_with("class ")
                    || line.starts_with("function "))
            {
                return Ok(Some(json!({
                    "file": path.to_string_lossy(),
                    "line": line_num + 1,
                    "content": line,
                    "language": extension
                })));
            }
        }

        Ok(None)
    }
}

/// Dependency analysis node - analyzes project dependencies
pub struct DependencyAnalysisNode {
    config: NodeConfig,
    repo_path: PathBuf,
}

impl DependencyAnalysisNode {
    pub fn new(config: NodeConfig, repo_path: PathBuf) -> Self {
        Self { config, repo_path }
    }
}

#[async_trait::async_trait]
impl Node for DependencyAnalysisNode {
    fn id(&self) -> String {
        "dependency_analysis".to_string()
    }

    fn kind(&self) -> &str {
        "dependency_analysis"
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(json!({
            "repo_path": self.repo_path.to_string_lossy()
        }))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        let mut dependencies = Vec::new();

        // Check for Cargo.toml (Rust)
        let cargo_toml = self.repo_path.join("Cargo.toml");
        if cargo_toml.exists() {
            dependencies.push("rust: Cargo.toml detected".to_string());
        }

        // Check for package.json (JavaScript/TypeScript)
        let package_json = self.repo_path.join("package.json");
        if package_json.exists() {
            dependencies.push("javascript: package.json detected".to_string());
        }

        // Check for requirements.txt (Python)
        let requirements_txt = self.repo_path.join("requirements.txt");
        if requirements_txt.exists() {
            dependencies.push("python: requirements.txt detected".to_string());
        }

        // Check for go.mod (Go)
        let go_mod = self.repo_path.join("go.mod");
        if go_mod.exists() {
            dependencies.push("go: go.mod detected".to_string());
        }

        Ok(json!({
            "dependencies": dependencies,
            "status": "success"
        }))
    }

    fn post(&self, _state: &mut SharedState, output: serde_json::Value) -> String {
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}
