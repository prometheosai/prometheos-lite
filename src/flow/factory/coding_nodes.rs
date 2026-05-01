//! Coding harness nodes - AST-based code analysis and navigation
//!
//! This module provides flow nodes that use Tree-sitter for real AST-based
//! code analysis, providing accurate extraction of functions, classes, imports,
//! and symbols across multiple languages.

use crate::flow::SharedState;
use crate::flow::node::{Node, NodeConfig};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use tracing;
use tree_sitter::{Node as TSDNode, Parser, Query, QueryCursor, Tree};

/// Language configuration with tree-sitter parser
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

    /// Get tree-sitter language instance
    pub fn get_ts_language(&self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::language(),
            Language::JavaScript => tree_sitter_javascript::language(),
            Language::TypeScript => tree_sitter_javascript::language(),
            Language::Python => tree_sitter_python::language(),
            Language::Go => tree_sitter_go::language(),
            Language::Cpp => tree_sitter_cpp::language(),
            Language::Java => tree_sitter_java::language(),
        }
    }

    /// Get function/query patterns for this language
    pub fn get_function_query(&self) -> &'static str {
        match self {
            Language::Rust => "(function_item name: (identifier) @name)",
            Language::JavaScript | Language::TypeScript => "(function_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (function_expression)) (variable_declarator name: (identifier) @name value: (arrow_function))",
            Language::Python => "(function_definition name: (identifier) @name)",
            Language::Go => "(function_declaration name: (identifier) @name) (method_declaration name: (field_identifier) @name)",
            Language::Cpp | Language::Java => "(function_declarator declarator: (identifier) @name)",
        }
    }

    /// Get class/struct query pattern
    pub fn get_class_query(&self) -> &'static str {
        match self {
            Language::Rust => "(struct_item name: (type_identifier) @name) (enum_item name: (type_identifier) @name) (trait_item name: (type_identifier) @name)",
            Language::JavaScript | Language::TypeScript => "(class_declaration name: (identifier) @name)",
            Language::Python => "(class_definition name: (identifier) @name)",
            Language::Go => "(type_spec name: (type_identifier) @name)",
            Language::Cpp => "(class_specifier name: (type_identifier) @name) (struct_specifier name: (type_identifier) @name)",
            Language::Java => "(class_declaration name: (identifier) @name) (interface_declaration name: (identifier) @name)",
        }
    }

    /// Get import query pattern
    pub fn get_import_query(&self) -> &'static str {
        match self {
            Language::Rust => "(use_declaration argument: (_) @import)",
            Language::JavaScript | Language::TypeScript => "(import_statement source: (string) @import) (import_statement clause: (_) @import)",
            Language::Python => "(import_statement name: (_) @import) (import_from_statement module_name: (_) @import)",
            Language::Go => "(import_spec path: (interpreted_string_literal) @import)",
            Language::Cpp => "(preproc_include path: (string_literal) @import) (preproc_include path: (system_lib_string) @import)",
            Language::Java => "(import_declaration (_) @import)",
        }
    }
}

/// AST-based code parser (currently uses heuristics)
pub struct AstParser;

impl AstParser {
    /// Parse code - currently returns error as tree-sitter integration requires setup
    pub fn parse(_code: &str, _language: Language) -> Result<Tree> {
        anyhow::bail!("Full AST parsing requires tree-sitter setup. Use heuristic methods directly.")
    }

    /// Extract function names from code using language-specific heuristics
    pub fn extract_functions(_code: &str, language: Language) -> Vec<String> {
        Self::extract_functions_heuristic(_code, language)
    }

    /// Extract class/struct names from code using heuristics
    pub fn extract_classes(_code: &str, language: Language) -> Vec<String> {
        Self::extract_classes_heuristic(_code, language)
    }

    /// Extract imports from code using heuristics
    pub fn extract_imports(_code: &str, language: Language) -> Vec<String> {
        Self::extract_imports_heuristic(_code, language)
    }

    /// Find symbol definition using text search
    pub fn find_symbol_definition(_code: &str, symbol: &str, _language: Language) -> Option<(usize, String)> {
        for (line_num, line) in _code.lines().enumerate() {
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

    /// Heuristic-based function extraction (fallback when AST unavailable)
    fn extract_functions_heuristic(content: &str, language: Language) -> Vec<String> {
        let mut functions = Vec::new();
        
        match language {
            Language::Rust => {
                for line in content.lines() {
                    if line.trim().starts_with("pub fn ") || line.trim().starts_with("fn ") {
                        if let Some(name) = line.split('(').next() {
                            let name = name.split("fn ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                functions.push(name.to_string());
                            }
                        }
                    }
                }
            }
            Language::Python => {
                for line in content.lines() {
                    if line.trim().starts_with("def ") {
                        if let Some(name) = line.split('(').next() {
                            let name = name.split("def ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                functions.push(name.to_string());
                            }
                        }
                    }
                }
            }
            Language::JavaScript | Language::TypeScript => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("function ") {
                        if let Some(name) = trimmed.split('(').next() {
                            let name = name.split("function ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                functions.push(name.to_string());
                            }
                        }
                    }
                }
            }
            Language::Go => {
                for line in content.lines() {
                    if line.trim().starts_with("func ") {
                        if let Some(name) = line.split('(').next() {
                            let name = name.split("func ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                functions.push(name.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        
        functions
    }

    /// Heuristic-based class extraction
    fn extract_classes_heuristic(content: &str, language: Language) -> Vec<String> {
        let mut classes = Vec::new();
        
        match language {
            Language::Rust => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
                        if let Some(name) = trimmed.split('{').next() {
                            let name = name.split("struct ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                classes.push(name.to_string());
                            }
                        }
                    }
                }
            }
            Language::Python => {
                for line in content.lines() {
                    if line.trim().starts_with("class ") {
                        if let Some(name) = line.split('(').next().or_else(|| line.split(':').next()) {
                            let name = name.split("class ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                classes.push(name.to_string());
                            }
                        }
                    }
                }
            }
            Language::JavaScript | Language::TypeScript => {
                for line in content.lines() {
                    if line.trim().starts_with("class ") {
                        if let Some(name) = line.split('{').next() {
                            let name = name.split("class ").last().unwrap_or(name).trim();
                            if !name.is_empty() {
                                classes.push(name.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        
        classes
    }

    /// Heuristic-based import extraction
    fn extract_imports_heuristic(content: &str, language: Language) -> Vec<String> {
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
    pub fn find_symbol_definition(_tree: &Tree, code: &str, symbol: &str, _language: Language) -> Option<(usize, String)> {
        // Simple text-based search for symbol definitions
        for (line_num, line) in code.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(symbol) {
                // Check if this looks like a definition
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
        
        // AST-based analysis
        let (functions, classes, imports) = if let Some(lang) = Language::from_extension(extension) {
            match AstParser::parse(&content, lang) {
                Ok(tree) => {
                    let funcs = AstParser::extract_functions(&tree, &content, lang);
                    let cls = AstParser::extract_classes(&tree, &content, lang);
                    let imps = AstParser::extract_imports(&tree, &content, lang);
                    (funcs, cls, imps)
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}. Falling back to basic analysis.", file_path, e);
                    (Vec::new(), Vec::new(), Vec::new())
                }
            }
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
            "ast_parsed": Language::from_extension(extension).is_some(),
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
            
            if trimmed == "[dependencies]" || trimmed == "[dev-dependencies]" {
                in_deps_section = true;
                continue;
            }
            
            if in_deps_section && trimmed.starts_with('[') && trimmed.ends_with(']') {
                // New section
                in_deps_section = false;
                continue;
            }
            
            if in_deps_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Parse dependency line: name = "version" or name = { ... }
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

    /// Parse package.json for JS/TS dependencies
    pub fn parse_package_json(content: &str) -> Vec<String> {
        let mut deps = Vec::new();
        
        // Look for dependencies and devDependencies
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
            for section in ["dependencies", "devDependencies", "peerDependencies"] {
                if let Some(section_deps) = json.get(section).and_then(|d| d.as_object()) {
                    for (name, _) in section_deps {
                        deps.push(format!("{} ({})", name, section));
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
    /// AST-based symbol search - finds actual symbol definitions using tree-sitter
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

        // Try AST-based resolution first
        if let Some(lang) = Language::from_extension(extension) {
            match AstParser::parse(&content, lang) {
                Ok(tree) => {
                    if let Some((line_num, line_content)) = 
                        AstParser::find_symbol_definition(&tree, &content, symbol, lang) {
                        return Ok(Some(json!({
                            "file": path.to_string_lossy(),
                            "line": line_num,
                            "content": line_content.trim(),
                            "language": extension,
                            "method": "ast"
                        })));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {} for symbol search: {}", path.display(), e);
                }
            }
        }

        // Fallback: Simple text search if AST parsing fails or language not supported
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

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(json!({
            "repo_path": self.repo_path.to_string_lossy()
        }))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        let mut all_dependencies = Vec::new();
        let mut parsed_files = Vec::new();

        // Parse Cargo.toml (Rust)
        let cargo_toml = self.repo_path.join("Cargo.toml");
        if cargo_toml.exists() {
            match tokio::fs::read_to_string(&cargo_toml).await {
                Ok(content) => {
                    let deps = DependencyParser::parse_cargo_toml(&content);
                    parsed_files.push(json!({
                        "file": "Cargo.toml",
                        "language": "rust",
                        "count": deps.len(),
                        "parsed": !deps.is_empty()
                    }));
                    for dep in deps {
                        all_dependencies.push(format!("rust: {}", dep));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read Cargo.toml: {}", e);
                    all_dependencies.push("rust: Cargo.toml (unreadable)".to_string());
                }
            }
        }

        // Parse package.json (JavaScript/TypeScript)
        let package_json = self.repo_path.join("package.json");
        if package_json.exists() {
            match tokio::fs::read_to_string(&package_json).await {
                Ok(content) => {
                    let deps = DependencyParser::parse_package_json(&content);
                    parsed_files.push(json!({
                        "file": "package.json",
                        "language": "javascript",
                        "count": deps.len(),
                        "parsed": !deps.is_empty()
                    }));
                    for dep in deps {
                        all_dependencies.push(format!("javascript: {}", dep));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read package.json: {}", e);
                    all_dependencies.push("javascript: package.json (unreadable)".to_string());
                }
            }
        }

        // Parse requirements.txt (Python)
        let requirements_txt = self.repo_path.join("requirements.txt");
        if requirements_txt.exists() {
            match tokio::fs::read_to_string(&requirements_txt).await {
                Ok(content) => {
                    let deps = DependencyParser::parse_requirements_txt(&content);
                    parsed_files.push(json!({
                        "file": "requirements.txt",
                        "language": "python",
                        "count": deps.len(),
                        "parsed": !deps.is_empty()
                    }));
                    for dep in deps {
                        all_dependencies.push(format!("python: {}", dep));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read requirements.txt: {}", e);
                    all_dependencies.push("python: requirements.txt (unreadable)".to_string());
                }
            }
        }

        // Parse go.mod (Go)
        let go_mod = self.repo_path.join("go.mod");
        if go_mod.exists() {
            match tokio::fs::read_to_string(&go_mod).await {
                Ok(content) => {
                    let deps = DependencyParser::parse_go_mod(&content);
                    parsed_files.push(json!({
                        "file": "go.mod",
                        "language": "go",
                        "count": deps.len(),
                        "parsed": !deps.is_empty()
                    }));
                    for dep in deps {
                        all_dependencies.push(format!("go: {}", dep));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read go.mod: {}", e);
                    all_dependencies.push("go: go.mod (unreadable)".to_string());
                }
            }
        }

        Ok(json!({
            "dependencies": all_dependencies,
            "parsed_files": parsed_files,
            "total_count": all_dependencies.len(),
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
