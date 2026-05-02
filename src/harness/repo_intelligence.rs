use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoContext {
    pub root: PathBuf,
    pub ranked_files: Vec<RankedFile>,
    pub symbols: Vec<CodeSymbol>,
    pub relationships: Vec<SymbolEdge>,
    pub compressed_context: String,
    pub token_estimate: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedFile {
    pub path: PathBuf,
    pub score: f32,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Class,
    Enum,
    Trait,
    Interface,
    Module,
    Constant,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolEdge {
    pub from: String,
    pub to: String,
    pub file: PathBuf,
    pub line: usize,
    pub kind: String,
}
pub async fn build_repo_context(
    root: &Path,
    task: &str,
    mentioned_files: &[PathBuf],
    _: &[String],
    token_budget: usize,
) -> Result<RepoContext> {
    let root = root.canonicalize()?;
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut ranked_files = rank_files_by_relevance(files, task, &[]);
    for f in mentioned_files {
        ranked_files.push(RankedFile {
            path: root.join(f),
            score: 100.0,
            reason: "explicitly mentioned".into(),
        })
    }
    let mut compressed_context = ranked_files
        .iter()
        .take(50)
        .map(|r| format!("{}\n", r.path.display()))
        .collect::<String>();
    compressed_context.truncate((token_budget.max(128)) * 4);
    let token_estimate = compressed_context.len() / 4;
    Ok(RepoContext {
        root,
        ranked_files,
        symbols: vec![],
        relationships: vec![],
        compressed_context,
        token_estimate,
    })
}
fn collect(p: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            let s = path.to_string_lossy();
            if s.contains(".git") || s.contains("target") || s.contains("node_modules") {
                continue;
            }
            if path.is_dir() {
                collect(&path, out)
            } else if path.is_file() {
                out.push(path)
            }
        }
    }
}
pub fn search_symbol(c: &RepoContext, n: &str) -> Vec<CodeSymbol> {
    c.symbols
        .iter()
        .filter(|s| s.name.contains(n))
        .cloned()
        .collect()
}
pub fn find_references(c: &RepoContext, s: &str) -> Vec<SymbolEdge> {
    c.relationships
        .iter()
        .filter(|e| e.to == s)
        .cloned()
        .collect()
}
pub fn rank_files_by_relevance(
    files: Vec<PathBuf>,
    task: &str,
    _: &[CodeSymbol],
) -> Vec<RankedFile> {
    let t = task.to_lowercase();
    let mut v = files
        .into_iter()
        .map(|p| {
            let s = p.to_string_lossy().to_lowercase();
            let mut score = 0.1;
            if s.contains("test") {
                score += 4.0
            }
            if t.split_whitespace().any(|w| w.len() > 2 && s.contains(w)) {
                score += 12.0
            }
            RankedFile {
                path: p,
                score,
                reason: "ranked by path/task match".into(),
            }
        })
        .collect::<Vec<_>>();
    v.sort_by(|a, b| b.score.total_cmp(&a.score));
    v
}
