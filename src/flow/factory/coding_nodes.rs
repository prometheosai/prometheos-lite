//! Coding harness nodes - Heuristic-based code analysis and navigation
//!
//! This module provides flow nodes that use heuristic-based code analysis
//! for extracting functions, classes, imports, and symbols across multiple languages.
//! Full AST-based analysis using tree-sitter is planned for future enhancement.

use crate::flow::SharedState;
use crate::flow::node::{Node, NodeConfig};
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Language configuration for code analysis
#[derive(Debug, Clone, Copy)]
pub enum Language {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Cpp,
    Java,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Language::Rust),
            "js" => Some(Language::JavaScript),
            "jsx" => Some(Language::JavaScript),
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::TypeScript),
            "py" => Some(Language::Python),
            "go" => Some(Language::Go),
            "cpp" | "cc" | "cxx" | "h" | "hpp" => Some(Language::Cpp),
            "c" => Some(Language::Cpp),
            "java" => Some(Language::Java),
            _ => None,
        }
    }
}

/// Heuristic-based code parser
pub struct AstParser;

impl AstParser {
    /// Extract function names from code using language-specific heuristics
    pub fn extract_functions(content: &str, language: Language) -> Vec<String> {
        let mut functions = Vec::new();

        match language {
            Language::Rust => {
                for line in content.lines() {
                    if (line.trim().starts_with("pub fn ") || line.trim().starts_with("fn "))
                        && let Some(name) = line.split('(').next()
                    {
                        let name = name.split("fn ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            functions.push(name.to_string());
                        }
                    }
                }
            }
            Language::Python => {
                for line in content.lines() {
                    if line.trim().starts_with("def ")
                        && let Some(name) = line.split('(').next()
                    {
                        let name = name.split("def ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            functions.push(name.to_string());
                        }
                    }
                }
            }
            Language::JavaScript | Language::TypeScript => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("function ")
                        && let Some(name) = trimmed.split('(').next()
                    {
                        let name = name.split("function ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            functions.push(name.to_string());
                        }
                    }
                }
            }
            Language::Go => {
                for line in content.lines() {
                    if line.trim().starts_with("func ")
                        && let Some(name) = line.split('(').next()
                    {
                        let name = name.split("func ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            functions.push(name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }

        functions
    }

    /// Extract class/struct names from code using heuristics
    pub fn extract_classes(content: &str, language: Language) -> Vec<String> {
        let mut classes = Vec::new();

        match language {
            Language::Rust => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if (trimmed.starts_with("pub struct ") || trimmed.starts_with("struct "))
                        && let Some(name) = trimmed.split('{').next()
                    {
                        let name = name.split("struct ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            classes.push(name.to_string());
                        }
                    }
                }
            }
            Language::Python => {
                for line in content.lines() {
                    if line.trim().starts_with("class ")
                        && let Some(name) =
                            line.split('(').next().or_else(|| line.split(':').next())
                    {
                        let name = name.split("class ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            classes.push(name.to_string());
                        }
                    }
                }
            }
            Language::JavaScript | Language::TypeScript => {
                for line in content.lines() {
                    if line.trim().starts_with("class ")
                        && let Some(name) = line.split('{').next()
                    {
                        let name = name.split("class ").last().unwrap_or(name).trim();
                        if !name.is_empty() {
                            classes.push(name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }

        classes
    }

    /// Extract imports from code using heuristics
    pub fn extract_imports(content: &str, language: Language) -> Vec<String> {
        let mut imports = Vec::new();

        match language {
            Language::Rust => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("use ") {
                        let import = trimmed.split(';').next().unwrap_or(trimmed);
                        let import = import.split("use ").last().unwrap_or(import).trim();
                        if !import.is_empty() {
                            imports.push(import.to_string());
                        }
                    }
                }
            }
            Language::Python => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("import ") {
                        let import = trimmed.split("import ").last().unwrap_or(trimmed).trim();
                        if !import.is_empty() {
                            imports.push(import.to_string());
                        }
                    } else if trimmed.starts_with("from ") {
                        let import = trimmed.split(';').next().unwrap_or(trimmed);
                        if !import.is_empty() {
                            imports.push(import.to_string());
                        }
                    }
                }
            }
            Language::JavaScript | Language::TypeScript => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("import ") {
                        let import = trimmed.split(';').next().unwrap_or(trimmed);
                        if !import.is_empty() {
                            imports.push(import.to_string());
                        }
                    }
                }
            }
            _ => {}
        }

        imports
    }

    /// Find symbol definition using text search
    pub fn find_symbol_definition(
        content: &str,
        symbol: &str,
        _language: Language,
    ) -> Option<(usize, String)> {
        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(symbol) {
                let is_def = trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("struct ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("def ")
                    || trimmed.starts_with("function ");

                if is_def {
                    return Some((line_num + 1, trimmed.to_string()));
                }
            }
        }
        None
    }
}

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
        let language_str = self.detect_language(extension);

        // Heuristic-based analysis
        let (functions, classes, imports) = if let Some(lang) = Language::from_extension(extension)
        {
            let funcs = AstParser::extract_functions(&content, lang);
            let cls = AstParser::extract_classes(&content, lang);
            let imps = AstParser::extract_imports(&content, lang);
            (funcs, cls, imps)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        Ok(json!({
            "file_path": file_path,
            "language": language_str,
            "line_count": line_count,
            "size_bytes": content.len(),
            "functions": functions,
            "classes": classes,
            "imports": imports,
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

/// Dependency parser for extracting actual dependency information
pub struct DependencyParser;

impl DependencyParser {
    /// Parse Cargo.toml for Rust dependencies
    pub fn parse_cargo_toml(content: &str) -> Vec<String> {
        let mut deps = Vec::new();

        // Simple parsing - look for [dependencies] section
        let mut in_deps_section = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("[dependencies]") {
                in_deps_section = true;
                continue;
            }

            // Exit section on next bracket line
            if in_deps_section && trimmed.starts_with('[') && !trimmed.starts_with("[[") {
                break;
            }

            if in_deps_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Extract dependency name (before = or space)
                if let Some(name) = trimmed.split('=').next() {
                    let name = name.trim();
                    if !name.is_empty() && !name.starts_with('#') {
                        deps.push(name.to_string());
                    }
                }
            }
        }

        deps
    }

    /// Parse package.json for JavaScript dependencies
    pub fn parse_package_json(content: &str) -> Vec<String> {
        let mut deps = Vec::new();

        // Look for "dependencies" or "devDependencies" keys
        let mut in_deps = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("\"dependencies\"") || trimmed.starts_with("\"devDependencies\"")
            {
                in_deps = true;
                continue;
            }

            if in_deps {
                if trimmed == "}" || trimmed.starts_with('}') {
                    in_deps = false;
                    continue;
                }

                // Extract package name (between quotes)
                if let Some(start) = trimmed.find('"')
                    && let Some(end) = trimmed[start + 1..].find('"')
                {
                    let name = &trimmed[start + 1..start + 1 + end];
                    if !name.is_empty() {
                        deps.push(name.to_string());
                    }
                }
            }
        }

        deps
    }

    /// Parse requirements.txt for Python dependencies
    pub fn parse_requirements_txt(content: &str) -> Vec<String> {
        content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
                    return None;
                }
                // Extract package name (before ==, >=, <=, ~=, etc.)
                trimmed
                    .split(&['=', '<', '>', '~', '!', ';'])
                    .next()
                    .map(|s| s.trim().to_string())
            })
            .collect()
    }

    /// Parse go.mod for Go dependencies
    pub fn parse_go_mod(content: &str) -> Vec<String> {
        let mut deps = Vec::new();
        let mut in_require = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("require (") {
                in_require = true;
                continue;
            }

            if in_require && trimmed == ")" {
                in_require = false;
                continue;
            }

            // Single-line require: require package version
            if trimmed.starts_with("require ") && !trimmed.contains('(') {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    deps.push(parts[1].to_string());
                }
            }

            // Inside require block
            if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if !parts.is_empty() {
                    deps.push(parts[0].to_string());
                }
            }
        }

        deps
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
            if full_path.exists()
                && let Some(def) = self.search_file_for_symbol(&full_path, symbol).await?
            {
                definitions.push(def);
            }
        }

        Ok(json!({
            "symbol": symbol,
            "definitions": definitions,
            "status": "success"
        }))
    }

    fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

impl SymbolResolutionNode {
    /// Heuristic-based symbol search - finds symbol definitions
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

        // Try heuristic-based resolution first
        if let Some(lang) = Language::from_extension(extension)
            && let Some((line_num, line_content)) =
                AstParser::find_symbol_definition(&content, symbol, lang)
        {
            return Ok(Some(json!({
                "file": path.to_string_lossy(),
                "line": line_num,
                "content": line_content.trim(),
                "language": extension,
                "method": "heuristic"
            })));
        }

        // Fallback: Simple text search if heuristic fails or language not supported
        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(symbol) {
                return Ok(Some(json!({
                    "file": path.to_string_lossy(),
                    "line": line_num + 1,
                    "content": trimmed,
                    "language": extension,
                    "method": "text_search"
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

    fn prep(&self, state: &SharedState) -> Result<serde_json::Value> {
        let manifest_path = state
            .get_input("manifest_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(json!({
            "manifest_path": manifest_path,
            "repo_path": self.repo_path.to_string_lossy()
        }))
    }

    async fn exec(&self, input: serde_json::Value) -> Result<serde_json::Value> {
        let manifest_path = input
            .get("manifest_path")
            .and_then(|v| v.as_str())
            .context("Missing manifest_path in input")?;

        let full_path = self.repo_path.join(manifest_path);

        if !full_path.exists() {
            return Ok(json!({
                "error": format!("Manifest not found: {}", full_path.display()),
                "status": "error"
            }));
        }

        let content = tokio::fs::read_to_string(&full_path)
            .await
            .context("Failed to read manifest file")?;

        let file_name = full_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let dependencies = match file_name {
            "Cargo.toml" => DependencyParser::parse_cargo_toml(&content),
            "package.json" => DependencyParser::parse_package_json(&content),
            "requirements.txt" => DependencyParser::parse_requirements_txt(&content),
            "go.mod" => DependencyParser::parse_go_mod(&content),
            _ => Vec::new(),
        };

        Ok(json!({
            "manifest_path": manifest_path,
            "manifest_type": file_name,
            "dependency_count": dependencies.len(),
            "dependencies": dependencies,
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
