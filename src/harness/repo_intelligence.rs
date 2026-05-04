use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};
use tree_sitter_typescript;

/// Dependency graph representation for a repository
/// Extracts dependencies from Cargo.toml, package.json, pyproject.toml, etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DependencyGraph {
    /// Direct dependencies: name -> version/path spec
    pub dependencies: HashMap<String, DependencySpec>,
    /// Dev/test dependencies (not included in production builds)
    pub dev_dependencies: HashMap<String, DependencySpec>,
    /// Build dependencies (for Rust/Cargo)
    pub build_dependencies: HashMap<String, DependencySpec>,
    /// Peer dependencies (for JS/TS)
    pub peer_dependencies: HashMap<String, DependencySpec>,
    /// Dependency lockfile entries (exact versions)
    pub locked_versions: HashMap<String, String>,
    /// Reverse dependency map: which packages depend on this one
    pub reverse_deps: HashMap<String, Vec<String>>,
    /// Dependency file path that was parsed
    pub source_file: PathBuf,
    /// Type of dependency file (cargo, npm, poetry, etc.)
    pub package_manager: PackageManagerType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencySpec {
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub git: Option<String>,
    pub features: Vec<String>,
    pub optional: bool,
    pub target: Option<String>, // platform-specific dep
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageManagerType {
    Cargo,
    Npm,
    Yarn,
    Pnpm,
    Poetry,
    Pip,
    Unknown,
}

impl Default for PackageManagerType {
    fn default() -> Self {
        PackageManagerType::Unknown
    }
}

/// Incremental cache entry for a single file's symbols
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCacheEntry {
    /// File modification time when cached
    pub mtime: SystemTime,
    /// File size when cached
    pub size: u64,
    /// File content hash for change detection
    pub content_hash: String,
    /// Extracted symbols from this file
    pub symbols: Vec<CodeSymbol>,
    /// Extracted relationships from this file
    pub relationships: Vec<SymbolEdge>,
    /// Language detected for this file
    pub language: String,
}

/// Incremental cache for RepoMap with automatic invalidation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoCache {
    /// Root directory this cache is for
    root: PathBuf,
    /// Map of relative file path -> cache entry
    files: HashMap<PathBuf, FileCacheEntry>,
    /// Dependency graph at time of caching
    dependency_graph: DependencyGraph,
    /// Cache timestamp
    cached_at: SystemTime,
    /// Cache version for migration
    version: u32,
}

const CACHE_VERSION: u32 = 1;
const CACHE_FILENAME: &str = ".repomap_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoContext {
    pub root: PathBuf,
    pub ranked_files: Vec<RankedFile>,
    pub symbols: Vec<CodeSymbol>,
    pub relationships: Vec<SymbolEdge>,
    pub compressed_context: String,
    pub token_estimate: usize,
    pub language_breakdown: HashMap<String, usize>,
    pub dependency_graph: DependencyGraph,
}

/// Alias for RepoContext - used by modules expecting RepoMap type
pub type RepoMap = RepoContext;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RankedFile {
    pub path: PathBuf,
    pub score: u32,
    pub reason: String,
    pub symbol_count: usize,
    pub language: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: usize,
    pub column_end: usize,
    pub documentation: Option<String>,
    pub signature: Option<String>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Class,
    Enum,
    Trait,
    Interface,
    Type,
    Module,
    Constant,
    Variable,
    Field,
    Import,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolEdge {
    pub from: String,
    pub to: String,
    pub file: PathBuf,
    pub line: usize,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeKind {
    Import,
    Call,
    Reference,
    Inherits,
    Implements,
    Contains,
}

#[derive(Debug, Clone)]
struct LanguageParser {
    language: tree_sitter::Language,
    file_extensions: Vec<String>,
}

pub async fn build_repo_context(
    root: &Path,
    task: &str,
    mentioned_files: &[PathBuf],
    mentioned_symbols: &[String],
    token_budget: usize,
) -> Result<RepoContext> {
    let root = root.canonicalize()?;

    let mut files = Vec::new();
    collect_files(&root, &mut files)?;

    let mut language_breakdown: HashMap<String, usize> = HashMap::new();
    let mut all_symbols: Vec<CodeSymbol> = Vec::new();
    let mut all_relationships: Vec<SymbolEdge> = Vec::new();
    let mut file_symbols_map: HashMap<PathBuf, Vec<CodeSymbol>> = HashMap::new();

    for file in &files {
        let lang = detect_language(file);
        *language_breakdown.entry(lang.clone()).or_insert(0) += 1;

        if let Ok(content) = fs::read_to_string(file) {
            if let Some((symbols, relationships)) =
                extract_symbols_and_relationships(file, &content, &lang)
            {
                for sym in &symbols {
                    all_symbols.push(sym.clone());
                }
                for rel in &relationships {
                    all_relationships.push(rel.clone());
                }
                file_symbols_map.insert(file.clone(), symbols);
            }
        }
    }

    let mut ranked_files = rank_files_by_relevance(
        files,
        task,
        &all_symbols,
        &file_symbols_map,
        mentioned_symbols,
    );

    for f in mentioned_files {
        let full_path = root.join(f);
        if !ranked_files.iter().any(|rf| rf.path == full_path) {
            let lang = detect_language(&full_path);
            ranked_files.push(RankedFile {
                path: full_path,
                score: 100,
                reason: "explicitly mentioned".into(),
                symbol_count: 0,
                language: lang,
            });
        }
    }

    ranked_files.sort_by(|a, b| b.score.cmp(&a.score));

    let compressed_context = build_compressed_context(&ranked_files, &all_symbols, token_budget);
    let token_estimate = compressed_context.len() / 4;

    // Parse dependency graph from manifest files
    let dependency_graph = parse_dependency_graph(&root);

    Ok(RepoContext {
        root,
        ranked_files,
        symbols: all_symbols,
        relationships: all_relationships,
        compressed_context,
        token_estimate,
        language_breakdown,
        dependency_graph,
    })
}

fn collect_files(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let walker = ignore::WalkBuilder::new(p)
        .add_custom_ignore_filename(".gitignore")
        .add_custom_ignore_filename(".prometheosignore")
        .build();

    for entry in walker {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() && is_code_file(path) {
                out.push(path.to_path_buf());
            }
        }
    }

    Ok(())
}

fn is_code_file(p: &Path) -> bool {
    let extensions: HashSet<&str> = [
        "rs", "js", "ts", "jsx", "tsx", "py", "go", "java", "c", "cpp", "h", "hpp", "rb", "php",
        "swift", "kt", "scala", "r", "m", "mm",
    ]
    .iter()
    .cloned()
    .collect();

    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| extensions.contains(e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn detect_language(p: &Path) -> String {
    match p.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("jsx") => "javascript",
        Some("tsx") => "typescript",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("hpp") => "cpp",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("kt") => "kotlin",
        Some("scala") => "scala",
        _ => "unknown",
    }
    .to_string()
}

fn extract_symbols_and_relationships(
    file: &Path,
    content: &str,
    language: &str,
) -> Option<(Vec<CodeSymbol>, Vec<SymbolEdge>)> {
    let mut parser = Parser::new();

    let ts_lang: tree_sitter::Language = match language {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        _ => return None,
    };

    parser.set_language(&ts_lang).ok()?;
    let tree = parser.parse(content, None)?;
    let root = tree.root_node();

    let mut symbols = Vec::new();
    let mut relationships = Vec::new();

    extract_from_node(
        file,
        content,
        &root,
        &ts_lang,
        &mut symbols,
        &mut relationships,
    );

    Some((symbols, relationships))
}

fn extract_from_node(
    file: &Path,
    content: &str,
    node: &Node,
    language: &tree_sitter::Language,
    symbols: &mut Vec<CodeSymbol>,
    relationships: &mut Vec<SymbolEdge>,
) {
    let kind = node.kind();
    let text = &content[node.byte_range()];

    match kind {
        "function_item" | "function_declaration" | "function_definition" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                let name = content[name_node.byte_range()].to_string();
                let (line_start, col_start) = position_from_byte(content, node.start_byte());
                let (line_end, col_end) = position_from_byte(content, node.end_byte());

                symbols.push(CodeSymbol {
                    name: name.clone(),
                    kind: SymbolKind::Function,
                    file: file.to_path_buf(),
                    line_start,
                    line_end,
                    column_start: col_start,
                    column_end: col_end,
                    documentation: extract_docs(content, node),
                    signature: Some(text.lines().next().unwrap_or(text).to_string()),
                    visibility: Visibility::Public,
                });
            }
        }
        "struct_item" | "struct_declaration" | "class_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                let name = content[name_node.byte_range()].to_string();
                let (line_start, col_start) = position_from_byte(content, node.start_byte());
                let (line_end, col_end) = position_from_byte(content, node.end_byte());

                let symbol_kind = if kind.contains("struct") {
                    SymbolKind::Struct
                } else {
                    SymbolKind::Class
                };

                symbols.push(CodeSymbol {
                    name: name.clone(),
                    kind: symbol_kind,
                    file: file.to_path_buf(),
                    line_start,
                    line_end,
                    column_start: col_start,
                    column_end: col_end,
                    documentation: extract_docs(content, node),
                    signature: None,
                    visibility: Visibility::Public,
                });

                if let Some(body) = find_child_by_kind(node, "field_declaration_list") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i as u32) {
                            if child.kind().contains("field") {
                                if let Some(field_name) = find_child_by_kind(&child, "identifier") {
                                    let field = content[field_name.byte_range()].to_string();
                                    relationships.push(SymbolEdge {
                                        from: name.clone(),
                                        to: field,
                                        file: file.to_path_buf(),
                                        line: line_start,
                                        kind: EdgeKind::Contains,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        "enum_item" | "enum_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                let name = content[name_node.byte_range()].to_string();
                let (line_start, col_start) = position_from_byte(content, node.start_byte());
                let (line_end, col_end) = position_from_byte(content, node.end_byte());

                symbols.push(CodeSymbol {
                    name,
                    kind: SymbolKind::Enum,
                    file: file.to_path_buf(),
                    line_start,
                    line_end,
                    column_start: col_start,
                    column_end: col_end,
                    documentation: extract_docs(content, node),
                    signature: None,
                    visibility: Visibility::Public,
                });
            }
        }
        "trait_item" | "interface_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                let name = content[name_node.byte_range()].to_string();
                let (line_start, col_start) = position_from_byte(content, node.start_byte());
                let (line_end, col_end) = position_from_byte(content, node.end_byte());

                let symbol_kind = if kind.contains("trait") {
                    SymbolKind::Trait
                } else {
                    SymbolKind::Interface
                };

                symbols.push(CodeSymbol {
                    name,
                    kind: symbol_kind,
                    file: file.to_path_buf(),
                    line_start,
                    line_end,
                    column_start: col_start,
                    column_end: col_end,
                    documentation: extract_docs(content, node),
                    signature: None,
                    visibility: Visibility::Public,
                });
            }
        }
        "impl_item" => {
            if let Some(type_node) = find_child_by_kind(node, "type_identifier") {
                let impl_for = content[type_node.byte_range()].to_string();
                if let Some(trait_node) = node.child(1) {
                    if trait_node.kind() == "type_identifier" {
                        let trait_name = content[trait_node.byte_range()].to_string();
                        relationships.push(SymbolEdge {
                            from: impl_for,
                            to: trait_name,
                            file: file.to_path_buf(),
                            line: 0,
                            kind: EdgeKind::Implements,
                        });
                    }
                }
            }
        }
        "import_statement" | "use_declaration" => {
            let import_text = text.to_string();
            let (line_start, _) = position_from_byte(content, node.start_byte());

            symbols.push(CodeSymbol {
                name: import_text.clone(),
                kind: SymbolKind::Import,
                file: file.to_path_buf(),
                line_start,
                line_end: line_start,
                column_start: 0,
                column_end: 0,
                documentation: None,
                signature: Some(import_text.clone()),
                visibility: Visibility::Public,
            });

            let imported_names = extract_import_names(&import_text);
            for name in imported_names {
                relationships.push(SymbolEdge {
                    from: file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    to: name,
                    file: file.to_path_buf(),
                    line: line_start,
                    kind: EdgeKind::Import,
                });
            }
        }
        "call_expression" => {
            if let Some(func) = node.child(0) {
                if func.kind() == "identifier" {
                    let called = content[func.byte_range()].to_string();
                    let (line, _) = position_from_byte(content, node.start_byte());

                    relationships.push(SymbolEdge {
                        from: file
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        to: called,
                        file: file.to_path_buf(),
                        line,
                        kind: EdgeKind::Call,
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            extract_from_node(file, content, &child, language, symbols, relationships);
        }
    }
}

fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i as u32) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn position_from_byte(content: &str, byte: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;

    for (i, c) in content.char_indices() {
        if i >= byte {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    (line, col)
}

fn extract_docs(content: &str, node: &Node) -> Option<String> {
    let start_byte = node.start_byte();
    let prefix = &content[..start_byte];

    let lines: Vec<&str> = prefix.lines().rev().collect();
    let mut docs = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("///") || trimmed.starts_with("/**") || trimmed.starts_with("//!") {
            docs.push(trimmed.to_string());
        } else if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        } else {
            break;
        }
    }

    if docs.is_empty() {
        None
    } else {
        docs.reverse();
        Some(docs.join("\n"))
    }
}

fn extract_import_names(import_text: &str) -> Vec<String> {
    let mut names = Vec::new();

    if import_text.contains("{") {
        let start = import_text.find('{').unwrap_or(0);
        let end = import_text.find('}').unwrap_or(import_text.len());
        let inner = &import_text[start + 1..end];

        for part in inner.split(',') {
            let name = part.trim().split_whitespace().next().unwrap_or("");
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    } else {
        let parts: Vec<&str> = import_text.split("::").collect();
        if let Some(last) = parts.last() {
            names.push(last.trim().to_string());
        }
    }

    names
}

fn rank_files_by_relevance(
    files: Vec<PathBuf>,
    task: &str,
    symbols: &[CodeSymbol],
    file_symbols: &HashMap<PathBuf, Vec<CodeSymbol>>,
    mentioned_symbols: &[String],
) -> Vec<RankedFile> {
    let task_lower = task.to_lowercase();
    let task_keywords: Vec<&str> = task_lower.split_whitespace().collect();
    let mentioned_set: HashSet<String> = mentioned_symbols.iter().cloned().collect();

    let mut symbol_index: HashMap<String, Vec<&CodeSymbol>> = HashMap::new();
    for sym in symbols {
        symbol_index
            .entry(sym.name.to_lowercase())
            .or_default()
            .push(sym);
    }

    let mut ranked = Vec::new();

    for file in files {
        let path_str = file.to_string_lossy().to_lowercase();
        let file_symbols_list = file_symbols.get(&file).cloned().unwrap_or_default();
        let symbol_count = file_symbols_list.len();

        let mut score: u32 = 0;
        let mut reasons: Vec<String> = Vec::new();

        if path_str.contains("test") {
            score += 4;
            reasons.push("test file".to_string());
        }

        for keyword in &task_keywords {
            if keyword.len() > 2 {
                if path_str.contains(keyword) {
                    score += 12;
                    reasons.push("path match".to_string());
                }

                if let Some(matching) = symbol_index.get(*keyword) {
                    let file_matches: Vec<_> = matching.iter().filter(|s| s.file == file).collect();

                    for sym in file_matches {
                        score += 20;
                        reasons.push(format!("symbol match: {}", sym.name));

                        if mentioned_set.contains(&sym.name) {
                            score += 50;
                            reasons.push("explicitly mentioned symbol".to_string());
                        }
                    }
                }
            }
        }

        if file_symbols_list
            .iter()
            .any(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        {
            score += 2;
            reasons.push("has functions".to_string());
        }

        let reason = if reasons.is_empty() {
            "default ranking".to_string()
        } else {
            reasons.join(", ")
        };

        let lang = detect_language(&file);
        ranked.push(RankedFile {
            path: file,
            score,
            reason,
            symbol_count,
            language: lang,
        });
    }

    ranked.sort_by(|a, b| b.score.cmp(&a.score));
    ranked
}

/// Alias for extract_symbols_and_relationships
fn extract_symbols_with_tree_sitter(
    file: &Path,
    content: &str,
    language: &str,
) -> Option<(Vec<CodeSymbol>, Vec<SymbolEdge>)> {
    extract_symbols_and_relationships(file, content, language)
}

fn build_compressed_context(
    ranked_files: &[RankedFile],
    symbols: &[CodeSymbol],
    token_budget: usize,
) -> String {
    let mut context = String::new();
    let mut used_tokens = 0;
    let max_tokens = token_budget.max(128);

    context.push_str("# Repository Files\n\n");

    for file in ranked_files.iter().take(30) {
        let entry = format!(
            "- {} (score: {:.1}, symbols: {})\n",
            file.path.display(),
            file.score,
            file.symbol_count
        );
        let entry_tokens = entry.len() / 4;

        if used_tokens + entry_tokens > max_tokens {
            break;
        }

        context.push_str(&entry);
        used_tokens += entry_tokens;
    }

    context.push_str("\n# Key Symbols\n\n");

    let important_symbols: Vec<_> = symbols
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Function | SymbolKind::Struct | SymbolKind::Class | SymbolKind::Trait
            )
        })
        .take(50)
        .collect();

    for sym in important_symbols {
        let entry = format!(
            "- {} ({:?}) in {}:{}\n",
            sym.name,
            sym.kind,
            sym.file.file_name().unwrap_or_default().to_string_lossy(),
            sym.line_start
        );
        let entry_tokens = entry.len() / 4;

        if used_tokens + entry_tokens > max_tokens {
            break;
        }

        context.push_str(&entry);
        used_tokens += entry_tokens;
    }

    context
}

pub fn search_symbol(c: &RepoContext, n: &str) -> Vec<CodeSymbol> {
    let query = n.to_lowercase();
    c.symbols
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

pub fn find_references(c: &RepoContext, s: &str) -> Vec<SymbolEdge> {
    let query = s.to_lowercase();
    c.relationships
        .iter()
        .filter(|e| e.to.to_lowercase() == query || e.from.to_lowercase() == query)
        .cloned()
        .collect()
}

pub fn get_related_symbols(c: &RepoContext, symbol_name: &str) -> Vec<CodeSymbol> {
    let mut related = HashSet::new();
    let name_lower = symbol_name.to_lowercase();

    for edge in &c.relationships {
        if edge.from.to_lowercase() == name_lower {
            related.insert(edge.to.clone());
        }
        if edge.to.to_lowercase() == name_lower {
            related.insert(edge.from.clone());
        }
    }

    c.symbols
        .iter()
        .filter(|s| related.contains(&s.name))
        .cloned()
        .collect()
}

pub fn find_symbol_in_file(c: &RepoContext, file: &Path, line: usize) -> Option<CodeSymbol> {
    c.symbols
        .iter()
        .find(|s| s.file == file && s.line_start <= line && s.line_end >= line)
        .cloned()
}

// ============================================================================
// DEPENDENCY GRAPH PARSING
// ============================================================================

/// Parse dependency graph from repository manifest files
/// Supports Cargo.toml (Rust), package.json (Node.js), pyproject.toml (Python)
pub fn parse_dependency_graph(root: &Path) -> DependencyGraph {
    // Try Cargo.toml first
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        return parse_cargo_toml(&cargo_toml);
    }

    // Try package.json
    let package_json = root.join("package.json");
    if package_json.exists() {
        return parse_package_json(&package_json);
    }

    // Try pyproject.toml
    let pyproject_toml = root.join("pyproject.toml");
    if pyproject_toml.exists() {
        return parse_pyproject_toml(&pyproject_toml);
    }

    // No recognized manifest found
    DependencyGraph {
        source_file: root.to_path_buf(),
        package_manager: PackageManagerType::Unknown,
        ..Default::default()
    }
}

/// Parse Cargo.toml and extract dependency information
fn parse_cargo_toml(path: &Path) -> DependencyGraph {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut graph = DependencyGraph {
        source_file: path.to_path_buf(),
        package_manager: PackageManagerType::Cargo,
        ..Default::default()
    };

    // Parse using toml crate
    if let Ok(toml_value) = content.parse::<toml::Value>() {
        // Parse [dependencies]
        if let Some(deps) = toml_value.get("dependencies").and_then(|d| d.as_table()) {
            for (name, spec) in deps {
                graph.dependencies.insert(
                    name.clone(),
                    parse_cargo_dependency(name, spec),
                );
            }
        }

        // Parse [dev-dependencies]
        if let Some(deps) = toml_value.get("dev-dependencies").and_then(|d| d.as_table()) {
            for (name, spec) in deps {
                graph.dev_dependencies.insert(
                    name.clone(),
                    parse_cargo_dependency(name, spec),
                );
            }
        }

        // Parse [build-dependencies]
        if let Some(deps) = toml_value.get("build-dependencies").and_then(|d| d.as_table()) {
            for (name, spec) in deps {
                graph.build_dependencies.insert(
                    name.clone(),
                    parse_cargo_dependency(name, spec),
                );
            }
        }

        // Parse [target.*.dependencies]
        if let Some(target) = toml_value.get("target").and_then(|t| t.as_table()) {
            for (target_name, target_deps) in target {
                if let Some(deps) = target_deps.get("dependencies").and_then(|d| d.as_table()) {
                    for (dep_name, spec) in deps {
                        let mut dep = parse_cargo_dependency(dep_name, spec);
                        dep.target = Some(target_name.clone());
                        graph.dependencies.insert(dep_name.clone(), dep);
                    }
                }
            }
        }
    }

    // Try to read Cargo.lock for locked versions
    let lockfile = path.parent().unwrap_or(Path::new(".")).join("Cargo.lock");
    if let Ok(lock_content) = fs::read_to_string(&lockfile) {
        graph.locked_versions = parse_cargo_lock(&lock_content);
    }

    // Build reverse dependency map
    graph.build_reverse_deps();

    graph
}

/// Parse a single Cargo dependency specification
fn parse_cargo_dependency(name: &str, spec: &toml::Value) -> DependencySpec {
    let mut dep = DependencySpec {
        optional: false,
        ..Default::default()
    };

    match spec {
        toml::Value::String(version) => {
            dep.version = Some(version.clone());
        }
        toml::Value::Table(table) => {
            dep.version = table.get("version").and_then(|v| v.as_str()).map(String::from);
            dep.path = table.get("path").and_then(|p| p.as_str()).map(PathBuf::from);
            dep.git = table.get("git").and_then(|g| g.as_str()).map(String::from);
            dep.optional = table.get("optional").and_then(|o| o.as_bool()).unwrap_or(false);

            // Parse features
            if let Some(features) = table.get("features").and_then(|f| f.as_array()) {
                dep.features = features
                    .iter()
                    .filter_map(|f| f.as_str())
                    .map(String::from)
                    .collect();
            }
        }
        _ => {}
    }

    dep
}

/// Parse Cargo.lock file to get exact versions
fn parse_cargo_lock(content: &str) -> HashMap<String, String> {
    let mut locked = HashMap::new();

    if let Ok(toml_value) = content.parse::<toml::Value>() {
        if let Some(packages) = toml_value.get("package").and_then(|p| p.as_array()) {
            for pkg in packages {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    if let Some(version) = pkg.get("version").and_then(|v| v.as_str()) {
                        locked.insert(name.to_string(), version.to_string());
                    }
                }
            }
        }
    }

    locked
}

/// Parse package.json and extract dependency information
fn parse_package_json(path: &Path) -> DependencyGraph {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut graph = DependencyGraph {
        source_file: path.to_path_buf(),
        package_manager: PackageManagerType::Npm, // Could be Yarn/Pnpm, detected by lockfile
        ..Default::default()
    };

    // Detect package manager from lockfile presence
    let parent = path.parent().unwrap_or(Path::new("."));
    if parent.join("yarn.lock").exists() {
        graph.package_manager = PackageManagerType::Yarn;
    } else if parent.join("pnpm-lock.yaml").exists() {
        graph.package_manager = PackageManagerType::Pnpm;
    }

    // Parse JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
        // Parse dependencies
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, version) in deps {
                graph.dependencies.insert(
                    name.clone(),
                    DependencySpec {
                        version: version.as_str().map(String::from),
                        ..Default::default()
                    },
                );
            }
        }

        // Parse devDependencies
        if let Some(deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
            for (name, version) in deps {
                graph.dev_dependencies.insert(
                    name.clone(),
                    DependencySpec {
                        version: version.as_str().map(String::from),
                        ..Default::default()
                    },
                );
            }
        }

        // Parse peerDependencies
        if let Some(deps) = json.get("peerDependencies").and_then(|d| d.as_object()) {
            for (name, version) in deps {
                graph.peer_dependencies.insert(
                    name.clone(),
                    DependencySpec {
                        version: version.as_str().map(String::from),
                        ..Default::default()
                    },
                );
            }
        }
    }

    // Try to read lockfile for locked versions
    let lockfile = match graph.package_manager {
        PackageManagerType::Yarn => parent.join("yarn.lock"),
        PackageManagerType::Pnpm => parent.join("pnpm-lock.yaml"),
        _ => parent.join("package-lock.json"),
    };

    if let Ok(lock_content) = fs::read_to_string(&lockfile) {
        graph.locked_versions = parse_npm_lock(&lock_content, graph.package_manager);
    }

    graph.build_reverse_deps();

    graph
}

/// Parse npm/yarn/pnpm lockfile
fn parse_npm_lock(content: &str, manager: PackageManagerType) -> HashMap<String, String> {
    let mut locked = HashMap::new();

    match manager {
        PackageManagerType::Yarn => {
            // Yarn lock format parsing
            for line in content.lines() {
                if line.starts_with('"') && line.contains("@") {
                    let parts: Vec<&str> = line.split('@').collect();
                    if parts.len() >= 2 {
                        let name = parts[0].trim_matches('"');
                        if let Some(version_part) = parts.last() {
                            if let Some(version) = version_part.split(':').next() {
                                locked.insert(name.to_string(), version.trim().to_string());
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // package-lock.json format
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(deps) = json.get("packages").and_then(|p| p.get("")).and_then(|d| d.get("dependencies")).and_then(|d| d.as_object()) {
                    for (name, version) in deps {
                        if let Some(v) = version.as_str() {
                            locked.insert(name.clone(), v.to_string());
                        }
                    }
                }
            }
        }
    }

    locked
}

/// Parse pyproject.toml (Poetry or PEP 621 format)
fn parse_pyproject_toml(path: &Path) -> DependencyGraph {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut graph = DependencyGraph {
        source_file: path.to_path_buf(),
        package_manager: PackageManagerType::Pip, // Could be Poetry
        ..Default::default()
    };

    // Detect if it's Poetry
    if content.contains("[tool.poetry]") {
        graph.package_manager = PackageManagerType::Poetry;
    }

    if let Ok(toml_value) = content.parse::<toml::Value>() {
        // Poetry format
        if let Some(poetry) = toml_value.get("tool").and_then(|t| t.get("poetry")) {
            // Parse dependencies
            if let Some(deps) = poetry.get("dependencies").and_then(|d| d.as_table()) {
                for (name, spec) in deps {
                    if name == "python" {
                        continue; // Skip python version spec
                    }
                    graph.dependencies.insert(
                        name.clone(),
                        parse_python_dependency(spec),
                    );
                }
            }

            // Parse dev dependencies
            if let Some(deps) = poetry.get("dev-dependencies").and_then(|d| d.as_table()) {
                for (name, spec) in deps {
                    graph.dev_dependencies.insert(
                        name.clone(),
                        parse_python_dependency(spec),
                    );
                }
            }

            // Parse group dependencies (Poetry 1.2+)
            if let Some(groups) = poetry.get("group").and_then(|g| g.as_table()) {
                for (_, group) in groups {
                    if let Some(deps) = group.get("dependencies").and_then(|d| d.as_table()) {
                        for (name, spec) in deps {
                            graph.dev_dependencies.insert(
                                name.clone(),
                                parse_python_dependency(spec),
                            );
                        }
                    }
                }
            }
        }

        // PEP 621 format (project.dependencies)
        if let Some(project) = toml_value.get("project").and_then(|p| p.as_table()) {
            // Parse dependencies array
            if let Some(deps) = project.get("dependencies").and_then(|d| d.as_array()) {
                for dep_str in deps {
                    if let Some(spec) = dep_str.as_str() {
                        // Parse simple "name>=version" format
                        if let Some((name, version)) = parse_pep621_dep(spec) {
                            graph.dependencies.insert(
                                name,
                                DependencySpec {
                                    version: Some(version),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }

            // Parse optional-dependencies
            if let Some(opt_deps) = project.get("optional-dependencies").and_then(|o| o.as_table()) {
                for (_, dep_array) in opt_deps {
                    if let Some(deps) = dep_array.as_array() {
                        for dep_str in deps {
                            if let Some(spec) = dep_str.as_str() {
                                if let Some((name, version)) = parse_pep621_dep(spec) {
                                    graph.dependencies.insert(
                                        name,
                                        DependencySpec {
                                            version: Some(version),
                                            optional: true,
                                            ..Default::default()
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Try to read poetry.lock or Pipfile.lock
    let parent = path.parent().unwrap_or(Path::new("."));
    let lockfile = if graph.package_manager == PackageManagerType::Poetry {
        parent.join("poetry.lock")
    } else {
        parent.join("Pipfile.lock")
    };

    if let Ok(lock_content) = fs::read_to_string(&lockfile) {
        if graph.package_manager == PackageManagerType::Poetry {
            graph.locked_versions = parse_poetry_lock(&lock_content);
        }
    }

    graph.build_reverse_deps();

    graph
}

/// Parse Poetry-style dependency specification
fn parse_python_dependency(spec: &toml::Value) -> DependencySpec {
    let mut dep = DependencySpec {
        optional: false,
        ..Default::default()
    };

    match spec {
        toml::Value::String(version) => {
            dep.version = Some(version.clone());
        }
        toml::Value::Table(table) => {
            dep.version = table.get("version").and_then(|v| v.as_str()).map(String::from);
            dep.git = table.get("git").and_then(|g| g.as_str()).map(String::from);
            dep.optional = table.get("optional").and_then(|o| o.as_bool()).unwrap_or(false);

            if let Some(features) = table.get("extras").and_then(|e| e.as_array()) {
                dep.features = features
                    .iter()
                    .filter_map(|f| f.as_str())
                    .map(String::from)
                    .collect();
            }
        }
        _ => {}
    }

    dep
}

/// Parse PEP 621 dependency string "name>=version"
fn parse_pep621_dep(spec: &str) -> Option<(String, String)> {
    // Handle various formats: name>=1.0, name==1.0, name~=1.0, name
    let version_chars: &[char] = &['=', '>', '<', '~', '!', '@'];

    if let Some(pos) = spec.find(|c: char| version_chars.contains(&c)) {
        let name = spec[..pos].trim().to_string();
        let version = spec[pos..].trim().to_string();
        Some((name, version))
    } else {
        // No version specified
        Some((spec.trim().to_string(), "*".to_string()))
    }
}

/// Parse poetry.lock file
fn parse_poetry_lock(content: &str) -> HashMap<String, String> {
    let mut locked = HashMap::new();

    // Poetry lock is TOML format
    if let Ok(toml_value) = content.parse::<toml::Value>() {
        if let Some(packages) = toml_value.get("package").and_then(|p| p.as_array()) {
            for pkg in packages {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    if let Some(version) = pkg.get("version").and_then(|v| v.as_str()) {
                        locked.insert(name.to_string(), version.to_string());
                    }
                }
            }
        }
    }

    locked
}

impl DependencyGraph {
    /// Build reverse dependency map: which packages depend on each package
    fn build_reverse_deps(&mut self) {
        let mut reverse: HashMap<String, Vec<String>> = HashMap::new();

        // Collect all dependency relationships
        let all_deps: Vec<_> = self.dependencies.iter()
            .map(|(k, _)| k.clone())
            .collect();

        for (dependent, spec) in &self.dependencies {
            // Add to reverse map
            reverse.entry(dependent.clone())
                .or_default()
                .extend(all_deps.iter().filter(|d| *d != dependent).cloned());
        }

        self.reverse_deps = reverse;
    }

    /// Get all transitive dependencies of a package
    pub fn transitive_deps(&self, package: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut stack = vec![package.to_string()];

        while let Some(current) = stack.pop() {
            if visited.insert(current.clone()) {
                if let Some(deps) = self.reverse_deps.get(&current) {
                    for dep in deps {
                        if !visited.contains(dep) {
                            stack.push(dep.clone());
                        }
                    }
                }
            }
        }

        visited.into_iter().filter(|d| d != package).collect()
    }

    /// Check if a package is a dev/optional dependency
    pub fn is_dev_dependency(&self, package: &str) -> bool {
        self.dev_dependencies.contains_key(package)
    }

    /// Get the exact locked version of a package
    pub fn locked_version(&self, package: &str) -> Option<&String> {
        self.locked_versions.get(package)
    }
}

impl Default for DependencySpec {
    fn default() -> Self {
        DependencySpec {
            version: None,
            path: None,
            git: None,
            features: Vec::new(),
            optional: false,
            target: None,
        }
    }
}

// ============================================================================
// INCREMENTAL CACHE IMPLEMENTATION
// ============================================================================

impl RepoCache {
    /// Create a new empty cache for a repository root
    pub fn new(root: PathBuf) -> Self {
        RepoCache {
            root,
            files: HashMap::new(),
            dependency_graph: DependencyGraph::default(),
            cached_at: SystemTime::now(),
            version: CACHE_VERSION,
        }
    }

    /// Load cache from disk if it exists and is valid
    pub fn load(root: &Path) -> Option<Self> {
        let cache_path = root.join(CACHE_FILENAME);
        let content = fs::read_to_string(&cache_path).ok()?;
        let mut cache: RepoCache = serde_json::from_str(&content).ok()?;

        // Check version compatibility
        if cache.version != CACHE_VERSION {
            tracing::info!("Cache version mismatch, rebuilding");
            return None;
        }

        // Verify cache root matches
        if cache.root != root {
            tracing::info!("Cache root mismatch, rebuilding");
            return None;
        }

        tracing::info!("Loaded incremental cache with {} files", cache.files.len());
        Some(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let cache_path = self.root.join(CACHE_FILENAME);
        let json = serde_json::to_string_pretty(self)?;
        let mut file = fs::File::create(&cache_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        tracing::info!("Saved incremental cache to {:?}", cache_path);
        Ok(())
    }

    /// Check if a file's cache entry is still valid
    pub fn is_valid(&self, path: &Path) -> bool {
        let relative_path = match path.strip_prefix(&self.root) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let entry = match self.files.get(relative_path) {
            Some(e) => e,
            None => return false,
        };

        // Check if file still exists
        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        // Check modification time
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if mtime != entry.mtime {
            tracing::debug!("File {:?} modified, invalidating cache", path);
            return false;
        }

        // Check size
        let size = metadata.len();
        if size != entry.size {
            tracing::debug!("File {:?} size changed, invalidating cache", path);
            return false;
        }

        // Verify content hash (defense against hash collision)
        if let Ok(content) = fs::read_to_string(path) {
            let current_hash = compute_string_hash(&content);
            if current_hash != entry.content_hash {
                tracing::debug!("File {:?} hash mismatch, invalidating cache", path);
                return false;
            }
        } else {
            return false;
        }

        true
    }

    /// Get cached symbols for a file if valid
    pub fn get_file_symbols(&self, path: &Path) -> Option<(Vec<CodeSymbol>, Vec<SymbolEdge>)> {
        if !self.is_valid(path) {
            return None;
        }

        let relative_path = path.strip_prefix(&self.root).ok()?;
        let entry = self.files.get(relative_path)?;

        Some((entry.symbols.clone(), entry.relationships.clone()))
    }

    /// Update cache entry for a file
    pub fn update_file(&mut self, path: &Path, symbols: Vec<CodeSymbol>, relationships: Vec<SymbolEdge>) {
        let relative_path = match path.strip_prefix(&self.root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => return,
        };

        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return,
        };

        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let size = metadata.len();

        let content_hash = match fs::read_to_string(path) {
            Ok(c) => compute_string_hash(&c),
            Err(_) => return,
        };

        let language = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| detect_language_from_ext(e).to_string())
            .unwrap_or_default();

        let entry = FileCacheEntry {
            mtime,
            size,
            content_hash,
            symbols,
            relationships,
            language,
        };

        self.files.insert(relative_path, entry);
        self.cached_at = SystemTime::now();
    }

    /// Remove a file from cache (e.g., when file is deleted)
    pub fn remove_file(&mut self, path: &Path) {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            self.files.remove(relative);
        }
    }

    /// Get all cached file paths
    pub fn cached_files(&self) -> Vec<PathBuf> {
        self.files.keys().map(|k| self.root.join(k)).collect()
    }

    /// Invalidate entire cache
    pub fn invalidate_all(&mut self) {
        self.files.clear();
        self.cached_at = SystemTime::now();
        tracing::info!("Invalidated entire cache");
    }

    /// Invalidate entries older than a certain duration
    pub fn invalidate_stale(&mut self, max_age: Duration) {
        let now = SystemTime::now();
        let to_remove: Vec<_> = self.files.iter()
            .filter(|(_, entry)| {
                now.duration_since(entry.mtime).unwrap_or(Duration::ZERO) > max_age
            })
            .map(|(path, _)| path.clone())
            .collect();

        for path in &to_remove {
            self.files.remove(path);
        }

        if !to_remove.is_empty() {
            tracing::info!("Invalidated {} stale cache entries", to_remove.len());
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let total_files = self.files.len();
        let total_symbols: usize = self.files.values().map(|e| e.symbols.len()).sum();
        let total_relationships: usize = self.files.values().map(|e| e.relationships.len()).sum();

        let age = SystemTime::now()
            .duration_since(self.cached_at)
            .unwrap_or(Duration::ZERO);

        CacheStats {
            total_files,
            total_symbols,
            total_relationships,
            cache_age_secs: age.as_secs(),
        }
    }

    /// Update dependency graph in cache
    pub fn update_dependency_graph(&mut self, graph: DependencyGraph) {
        self.dependency_graph = graph;
    }

    /// Get cached dependency graph
    pub fn get_dependency_graph(&self) -> Option<&DependencyGraph> {
        // Check if manifest files have changed
        let manifest = self.dependency_graph.source_file.clone();
        if manifest.exists() {
            if let Ok(metadata) = fs::metadata(&manifest) {
                let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                // Only return if cache is newer than manifest modification
                if self.cached_at > mtime {
                    return Some(&self.dependency_graph);
                }
            }
        }
        None
    }
}

/// Cache statistics for monitoring
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub total_files: usize,
    pub total_symbols: usize,
    pub total_relationships: usize,
    pub cache_age_secs: u64,
}

/// Compute hash of a string for cache validation
fn compute_string_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Detect language from file extension
fn detect_language_from_ext(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "js" => "javascript",
        "ts" => "typescript",
        "tsx" => "typescript",
        "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "cpp" | "cc" | "cxx" => "cpp",
        "c" => "c",
        "h" => "header",
        "hpp" => "cpp",
        _ => "unknown",
    }
}

// ============================================================================
// INTEGRATION WITH RepoMap BUILDER
// ============================================================================

/// Build RepoMap with incremental caching support
pub async fn build_repo_context_with_cache(
    root: &Path,
    max_tokens: usize,
    use_cache: bool,
) -> Result<RepoContext> {
    // Try to load existing cache
    let mut cache = if use_cache {
        RepoCache::load(root)
    } else {
        None
    };

    // Load dependency graph (cached or fresh)
    let dependency_graph = cache.as_ref()
        .and_then(|c| c.get_dependency_graph().cloned())
        .unwrap_or_else(|| parse_dependency_graph(root));

    // Update cache with dependency graph
    if let Some(ref mut c) = cache {
        c.update_dependency_graph(dependency_graph.clone());
    }

    // Build context with incremental symbol extraction
    let mut all_symbols = Vec::new();
    let mut all_relationships = Vec::new();
    let mut ranked_files = Vec::new();
    let mut language_breakdown: HashMap<String, usize> = HashMap::new();

    // Walk directory and process files
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            !matches!(name, "target" | "node_modules" | ".git" | "dist" | "build")
        });

    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check if we have valid cached symbols for this file
        let (symbols, relationships) = if let Some(ref c) = cache {
            if let Some(cached) = c.get_file_symbols(path) {
                tracing::debug!("Using cached symbols for {:?}", path);
                cached
            } else {
                // Parse file and extract symbols
                let (s, r) = extract_symbols_from_file(path).await?;
                if let Some(ref mut c) = cache {
                    c.update_file(path, s.clone(), r.clone());
                }
                (s, r)
            }
        } else {
            // No cache, parse fresh
            extract_symbols_from_file(path).await?
        };

        // Update language breakdown
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lang = detect_language_from_ext(ext);
            *language_breakdown.entry(lang.to_string()).or_insert(0) += 1;
        }

        // Add to ranked files
        let score = calculate_file_relevance(&symbols, &relationships);
        let lang = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| detect_language_from_ext(e).to_string())
            .unwrap_or_default();
        ranked_files.push(RankedFile {
            path: path.to_path_buf(),
            score: score as u32,
            reason: generate_file_reason(&symbols),
            symbol_count: symbols.len(),
            language: lang,
        });

        all_symbols.extend(symbols);
        all_relationships.extend(relationships);
    }

    // Sort ranked files by score
    ranked_files.sort_by(|a, b| b.score.cmp(&a.score));

    // Generate compressed context
    let compressed_context = build_compressed_context(&ranked_files, &all_symbols, max_tokens);

    let token_estimate = compressed_context.len() / 4;

    let context = RepoContext {
        root: root.to_path_buf(),
        ranked_files,
        symbols: all_symbols,
        relationships: all_relationships,
        compressed_context,
        token_estimate,
        language_breakdown,
        dependency_graph,
    };

    // Save cache if enabled
    if use_cache {
        if let Some(mut c) = cache {
            c.save()?;
        } else {
            // Create new cache from what we built
            let mut new_cache = RepoCache::new(root.to_path_buf());
            new_cache.update_dependency_graph(context.dependency_graph.clone());
            // Note: File symbols would need to be re-added, this is handled above
            new_cache.save()?;
        }
    }

    Ok(context)
}

/// Calculate file relevance score based on symbol count and relationships
fn calculate_file_relevance(symbols: &[CodeSymbol], relationships: &[SymbolEdge]) -> f32 {
    let symbol_weight = 1.0;
    let relationship_weight = 0.5;
    
    let symbol_score = symbols.len() as f32 * symbol_weight;
    let relationship_score = relationships.len() as f32 * relationship_weight;
    
    // Bonus for files with public exports
    let public_bonus = symbols.iter()
        .filter(|s| s.visibility == Visibility::Public)
        .count() as f32 * 2.0;
    
    symbol_score + relationship_score + public_bonus
}

/// Generate reason string for file ranking
fn generate_file_reason(symbols: &[CodeSymbol]) -> String {
    let pub_count = symbols.iter()
        .filter(|s| s.visibility == Visibility::Public)
        .count();
    
    let fn_count = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .count();
    
    let struct_count = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Struct)
        .count();
    
    let parts: Vec<String> = [
        if pub_count > 0 { format!("{} public", pub_count) } else { String::new() },
        if fn_count > 0 { format!("{} func", fn_count) } else { String::new() },
        if struct_count > 0 { format!("{} types", struct_count) } else { String::new() },
    ].into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    
    if parts.is_empty() {
        format!("{} symbols", symbols.len())
    } else {
        parts.join(", ")
    }
}

/// Extract symbols from a single file (async wrapper)
async fn extract_symbols_from_file(path: &Path) -> Result<(Vec<CodeSymbol>, Vec<SymbolEdge>)> {
    // This calls into the existing symbol extraction logic
    let content = tokio::fs::read_to_string(path).await?;
    
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let language = detect_language_from_ext(ext);
    
    let mut symbols = Vec::new();
    let mut relationships = Vec::new();
    
    if let Some((s, r)) = extract_symbols_with_tree_sitter(path, &content, language) {
        symbols = s;
        relationships = r;
    }
    
    Ok((symbols, relationships))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let content = r#"
[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }
local = { path = "../local" }
git = { git = "https://github.com/example/repo" }

[dev-dependencies]
tempfile = "3.0"
"#;
        fs::write(dir.path().join("Cargo.toml"), content).unwrap();
        
        let graph = parse_dependency_graph(dir.path());
        
        assert_eq!(graph.package_manager, PackageManagerType::Cargo);
        assert_eq!(graph.dependencies.len(), 4);
        assert!(graph.dependencies.contains_key("serde"));
        assert!(graph.dev_dependencies.contains_key("tempfile"));
        
        let tokio = graph.dependencies.get("tokio").unwrap();
        assert_eq!(tokio.features, vec!["full"]);
        
        let local = graph.dependencies.get("local").unwrap();
        assert_eq!(local.path, Some(PathBuf::from("../local")));
    }

    #[test]
    fn test_parse_package_json() {
        let dir = TempDir::new().unwrap();
        let content = r#"{
  "name": "test",
  "dependencies": { "react": "^18.0.0" },
  "devDependencies": { "jest": "^29.0.0" }
}"#;
        fs::write(dir.path().join("package.json"), content).unwrap();
        fs::write(dir.path().join("yarn.lock"), "\"react@^18.0.0\": version \"18.2.0\"").unwrap();
        
        let graph = parse_dependency_graph(dir.path());
        
        assert_eq!(graph.package_manager, PackageManagerType::Yarn);
        assert!(graph.dependencies.contains_key("react"));
        assert!(graph.dev_dependencies.contains_key("jest"));
    }

    #[test]
    fn test_repo_cache() {
        let dir = TempDir::new().unwrap();
        let mut cache = RepoCache::new(dir.path().to_path_buf());
        
        let graph = DependencyGraph {
            source_file: dir.path().join("Cargo.toml"),
            package_manager: PackageManagerType::Cargo,
            dependencies: [("serde".to_string(), DependencySpec::default())].into_iter().collect(),
            ..Default::default()
        };
        cache.update_dependency_graph(graph);
        cache.save().unwrap();
        
        let loaded = RepoCache::load(dir.path()).unwrap();
        assert!(loaded.dependency_graph.dependencies.contains_key("serde"));
    }
}
