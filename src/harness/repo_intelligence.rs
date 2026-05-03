use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoContext {
    pub root: PathBuf,
    pub ranked_files: Vec<RankedFile>,
    pub symbols: Vec<CodeSymbol>,
    pub relationships: Vec<SymbolEdge>,
    pub compressed_context: String,
    pub token_estimate: usize,
    pub language_breakdown: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedFile {
    pub path: PathBuf,
    pub score: f32,
    pub reason: String,
    pub symbol_count: usize,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
                score: 100.0,
                reason: "explicitly mentioned".into(),
                symbol_count: 0,
                language: lang,
            });
        }
    }

    ranked_files.sort_by(|a, b| b.score.total_cmp(&a.score));

    let compressed_context = build_compressed_context(&ranked_files, &all_symbols, token_budget);
    let token_estimate = compressed_context.len() / 4;

    Ok(RepoContext {
        root,
        ranked_files,
        symbols: all_symbols,
        relationships: all_relationships,
        compressed_context,
        token_estimate,
        language_breakdown,
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
        "typescript" | "tsx" => tree_sitter_javascript::LANGUAGE.into(),
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

        let mut score: f32 = 0.1;
        let mut reasons: Vec<String> = Vec::new();

        if path_str.contains("test") {
            score += 4.0;
            reasons.push("test file".to_string());
        }

        for keyword in &task_keywords {
            if keyword.len() > 2 {
                if path_str.contains(keyword) {
                    score += 12.0;
                    reasons.push("path match".to_string());
                }

                if let Some(matching) = symbol_index.get(*keyword) {
                    let file_matches: Vec<_> = matching.iter().filter(|s| s.file == file).collect();

                    for sym in file_matches {
                        score += 20.0;
                        reasons.push(format!("symbol match: {}", sym.name));

                        if mentioned_set.contains(&sym.name) {
                            score += 50.0;
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
            score += 2.0;
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

    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    ranked
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
