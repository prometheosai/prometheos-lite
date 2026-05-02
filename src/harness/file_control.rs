use crate::harness::repo_intelligence::RepoContext;
use anyhow::{Result, bail, Context};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
use ignore::gitignore::GitignoreBuilder;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FileSet {
    pub editable: Vec<PathBuf>,
    pub readonly: Vec<PathBuf>,
    pub generated: Vec<PathBuf>,
    pub artifacts: Vec<PathBuf>,
    pub denied: Vec<(PathBuf, DenyReason)>,
    pub binary: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DenyReason {
    OutsideRepo,
    DeniedPath,
    BinaryFile,
    TooLarge,
    SensitiveFile,
    Generated,
    NotTracked,
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenyReason::OutsideRepo => write!(f, "file is outside repository root"),
            DenyReason::DeniedPath => write!(f, "path is in deny list"),
            DenyReason::BinaryFile => write!(f, "binary files cannot be edited"),
            DenyReason::TooLarge => write!(f, "file exceeds size limit"),
            DenyReason::SensitiveFile => write!(f, "sensitive file detected"),
            DenyReason::Generated => write!(f, "generated file should not be edited directly"),
            DenyReason::NotTracked => write!(f, "file not tracked by git or in editable set"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilePolicy {
    pub repo_root: PathBuf,
    pub allowed_write_paths: Vec<PathBuf>,
    pub denied_paths: Vec<PathBuf>,
    pub denied_patterns: Vec<String>,
    pub max_file_size_bytes: u64,
    pub allow_delete: bool,
    pub allow_rename: bool,
    pub allow_generated_edits: bool,
    pub respect_gitignore: bool,
    pub allow_binary_edit: bool,
    pub sensitive_file_patterns: Vec<String>,
    pub generated_file_patterns: Vec<String>,
}

impl FilePolicy {
    pub fn default_for_repo(root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: root.into(),
            allowed_write_paths: vec![PathBuf::from(".")],
            denied_paths: vec![
                PathBuf::from(".git"),
                PathBuf::from(".env"),
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
                PathBuf::from("dist"),
                PathBuf::from("build"),
                PathBuf::from("__pycache__"),
                PathBuf::from(".pytest_cache"),
                PathBuf::from("coverage"),
                PathBuf::from(".coverage"),
            ],
            denied_patterns: vec![
                "*.lock".into(),
                "*-lock.*".into(),
                ".DS_Store".into(),
                "Thumbs.db".into(),
            ],
            max_file_size_bytes: 1_000_000, // 1MB default
            allow_delete: false,
            allow_rename: false,
            allow_generated_edits: false,
            respect_gitignore: true,
            allow_binary_edit: false,
            sensitive_file_patterns: vec![
                ".env*".into(),
                "*.key".into(),
                "*.pem".into(),
                "*.p12".into(),
                "*.pfx".into(),
                "*.crt".into(),
                "credentials*".into(),
                "*secret*".into(),
                "*password*".into(),
                "*token*".into(),
                ".aws/".into(),
                ".ssh/".into(),
            ],
            generated_file_patterns: vec![
                "*.min.js".into(),
                "*.min.css".into(),
                "*.generated.*".into(),
                "*.gen.*".into(),
                "*_gen.*".into(),
                "*generated*/".into(),
                "dist/".into(),
                "build/".into(),
                "target/".into(),
                "out/".into(),
            ],
        }
    }

    pub fn strict() -> Self {
        let mut policy = Self::default_for_repo(".");
        policy.allow_delete = false;
        policy.allow_rename = false;
        policy.allow_generated_edits = false;
        policy.allow_binary_edit = false;
        policy.max_file_size_bytes = 500_000; // 500KB
        policy
    }

    pub fn relaxed(root: impl Into<PathBuf>) -> Self {
        let mut policy = Self::default_for_repo(root);
        policy.allow_delete = true;
        policy.allow_rename = true;
        policy.allow_generated_edits = true;
        policy.max_file_size_bytes = 5_000_000; // 5MB
        policy
    }
}

#[derive(Debug, Clone)]
struct FileClassification {
    path: PathBuf,
    is_binary: bool,
    is_sensitive: bool,
    is_generated: bool,
    size: u64,
    is_gitignored: bool,
}

pub fn build_file_set(
    ctx: &RepoContext,
    mentioned_files: &[PathBuf],
    policy: &FilePolicy,
) -> Result<FileSet> {
    let mut file_set = FileSet::default();
    let mut gitignore = build_gitignore(&policy.repo_root)?;
    
    let mut classifications: HashMap<PathBuf, FileClassification> = HashMap::new();
    
    for ranked_file in &ctx.ranked_files {
        let path = &ranked_file.path;
        
        let classification = classify_file(path, &mut gitignore, policy)?;
        classifications.insert(path.clone(), classification);
    }
    
    for mentioned in mentioned_files {
        let full_path = normalize_path(&policy.repo_root, mentioned)?;
        if !classifications.contains_key(&full_path) {
            if let Ok(classification) = classify_file(&full_path, &mut gitignore, policy) {
                classifications.insert(full_path.clone(), classification);
            }
        }
    }
    
    for (path, classification) in classifications {
        let category = categorize_file(&path, &classification, policy)?;
        
        match category {
            FileCategory::Editable => file_set.editable.push(path),
            FileCategory::Readonly => file_set.readonly.push(path),
            FileCategory::Generated => file_set.generated.push(path),
            FileCategory::Binary => file_set.binary.push(path),
            FileCategory::Denied(reason) => file_set.denied.push((path, reason)),
        }
    }
    
    file_set.editable.sort();
    file_set.readonly.sort();
    file_set.generated.sort();
    file_set.binary.sort();
    file_set.denied.sort_by(|a, b| a.0.cmp(&b.0));
    
    Ok(file_set)
}

#[derive(Debug, Clone)]
enum FileCategory {
    Editable,
    Readonly,
    Generated,
    Binary,
    Denied(DenyReason),
}

fn classify_file(
    path: &Path,
    gitignore: &mut Option<ignore::gitignore::Gitignore>,
    policy: &FilePolicy,
) -> Result<FileClassification> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?;
    
    let size = metadata.len();
    let is_binary = is_binary_file(path)?;
    let is_sensitive = is_sensitive_file(path, policy);
    let is_generated = is_generated_file(path, policy);
    
    let is_gitignored = if policy.respect_gitignore {
        if let Some(ref gi) = gitignore {
            gi.matched(path, metadata.is_dir()).is_ignore()
        } else {
            false
        }
    } else {
        false
    };
    
    Ok(FileClassification {
        path: path.to_path_buf(),
        is_binary,
        is_sensitive,
        is_generated,
        size,
        is_gitignored,
    })
}

fn categorize_file(
    path: &Path,
    classification: &FileClassification,
    policy: &FilePolicy,
) -> Result<FileCategory> {
    if is_path_denied(path, policy)? {
        return Ok(FileCategory::Denied(DenyReason::DeniedPath));
    }
    
    if classification.is_sensitive {
        return Ok(FileCategory::Denied(DenyReason::SensitiveFile));
    }
    
    if classification.size > policy.max_file_size_bytes {
        return Ok(FileCategory::Denied(DenyReason::TooLarge));
    }
    
    if classification.is_binary && !policy.allow_binary_edit {
        return Ok(FileCategory::Binary);
    }
    
    if classification.is_generated && !policy.allow_generated_edits {
        return Ok(FileCategory::Generated);
    }
    
    if classification.is_gitignored {
        return Ok(FileCategory::Readonly);
    }
    
    Ok(FileCategory::Editable)
}

fn build_gitignore(repo_root: &Path) -> Result<Option<ignore::gitignore::Gitignore>> {
    let gitignore_path = repo_root.join(".gitignore");
    
    if !gitignore_path.exists() {
        return Ok(None);
    }
    
    let mut builder = GitignoreBuilder::new(repo_root);
    
    if let Some(e) = builder.add(&gitignore_path) {
        eprintln!("Warning: Failed to parse .gitignore: {}", e);
    }
    
    let global_gitignore = dirs::home_dir().map(|h| h.join(".gitignore_global"));
    if let Some(ref global) = global_gitignore {
        if global.exists() {
            if let Some(e) = builder.add(global) {
                eprintln!("Warning: Failed to parse global .gitignore: {}", e);
            }
        }
    }
    
    Ok(Some(builder.build()?))
}

fn is_binary_file(path: &Path) -> Result<bool> {
    const SAMPLE_SIZE: usize = 8192;
    const MAX_TEXT_RATIO: f64 = 0.10;
    
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    
    let text_extensions: HashSet<&str> = [
        "txt", "md", "rs", "js", "ts", "jsx", "tsx", "py", "go", "java", "c", "cpp", "h",
        "hpp", "rb", "php", "swift", "kt", "scala", "r", "m", "mm", "html", "css", "scss",
        "sass", "less", "json", "yaml", "yml", "toml", "xml", "sql", "sh", "bash", "zsh",
        "fish", "ps1", "bat", "cmd", "dockerfile", "makefile", "cmake", "graphql", "gql",
    ].iter().cloned().collect();
    
    if let Some(ref ext) = extension {
        if text_extensions.contains(ext.as_str()) {
            return Ok(false);
        }
    }
    
    let binary_extensions: HashSet<&str> = [
        "exe", "dll", "so", "dylib", "bin", "obj", "o", "a", "lib", "pyc", "class",
        "jar", "war", "ear", "zip", "tar", "gz", "bz2", "7z", "rar", "jpg", "jpeg",
        "png", "gif", "bmp", "ico", "svg", "pdf", "doc", "docx", "xls", "xlsx", "ppt",
        "pptx", "mp3", "mp4", "avi", "mov", "wav", "ogg", "webm", "ttf", "otf", "woff",
        "woff2", "eot", "swf", "fla", "db", "sqlite", "sqlite3", "mdb", "accdb",
    ].iter().cloned().collect();
    
    if let Some(ref ext) = extension {
        if binary_extensions.contains(ext.as_str()) {
            return Ok(true);
        }
    }
    
    let mut file = fs::File::open(path)
        .with_context(|| format!("Failed to open {} for binary detection", path.display()))?;
    
    let mut buffer = vec![0u8; SAMPLE_SIZE];
    let bytes_read = file.read(&mut buffer)
        .with_context(|| format!("Failed to read from {}", path.display()))?;
    
    buffer.truncate(bytes_read);
    
    let null_byte_count = buffer.iter().filter(|&&b| b == 0).count();
    let null_ratio = null_byte_count as f64 / bytes_read as f64;
    
    if null_ratio > MAX_TEXT_RATIO {
        return Ok(true);
    }
    
    let control_char_count = buffer.iter()
        .filter(|&&b| b < 32 && b != 0x09 && b != 0x0A && b != 0x0D)
        .count();
    let control_ratio = control_char_count as f64 / bytes_read as f64;
    
    Ok(control_ratio > MAX_TEXT_RATIO)
}

fn is_sensitive_file(path: &Path, policy: &FilePolicy) -> bool {
    let path_str = path.to_string_lossy();
    let file_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    
    for pattern in &policy.sensitive_file_patterns {
        if glob_match(pattern, &path_str) || glob_match(pattern, file_name) {
            return true;
        }
    }
    
    let sensitive_content_indicators = [
        "password", "secret", "token", "key", "credential", "private",
        "api_key", "apikey", "auth_token", "access_token", "bearer",
    ];
    
    let lower_path = path_str.to_lowercase();
    for indicator in &sensitive_content_indicators {
        if lower_path.contains(indicator) {
            return true;
        }
    }
    
    false
}

fn is_generated_file(path: &Path, policy: &FilePolicy) -> bool {
    let path_str = path.to_string_lossy();
    let file_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    
    for pattern in &policy.generated_file_patterns {
        if glob_match(pattern, &path_str) || glob_match(pattern, file_name) {
            return true;
        }
    }
    
    if let Ok(content) = fs::read_to_string(path) {
        let first_lines: String = content.lines().take(5).collect::<Vec<_>>().join("\n");
        if first_lines.contains("@generated") ||
           first_lines.contains("GENERATED") ||
           first_lines.contains("Auto-generated") ||
           first_lines.contains("This file was generated") {
            return true;
        }
    }
    
    false
}

fn is_path_denied(path: &Path, policy: &FilePolicy) -> Result<bool> {
    let canonical = path.canonicalize()?;
    let repo_root = policy.repo_root.canonicalize()?;
    
    if !canonical.starts_with(&repo_root) {
        return Ok(true);
    }
    
    let relative = canonical.strip_prefix(&repo_root)
        .map_err(|_| anyhow::anyhow!("Failed to get relative path"))?;
    
    for denied in &policy.denied_paths {
        if relative.starts_with(denied) {
            return Ok(true);
        }
    }
    
    let path_str = path.to_string_lossy();
    for pattern in &policy.denied_patterns {
        if glob_match(pattern, &path_str) {
            return Ok(true);
        }
    }
    
    Ok(false)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern.ends_with('/') {
        let dir_pattern = &pattern[..pattern.len() - 1];
        return text.contains(&format!("{}/", dir_pattern)) ||
               text.ends_with(dir_pattern);
    }
    
    if pattern.starts_with("*") && pattern.ends_with("*") {
        let middle = &pattern[1..pattern.len() - 1];
        return text.contains(middle);
    }
    
    if pattern.starts_with("*") {
        let suffix = &pattern[1..];
        return text.ends_with(suffix);
    }
    
    if pattern.ends_with("*") {
        let prefix = &pattern[..pattern.len() - 1];
        return text.starts_with(prefix);
    }
    
    text == pattern
}

pub fn assert_edit_allowed(path: &Path, set: &FileSet, policy: &FilePolicy) -> Result<()> {
    let normalized = normalize_path(&policy.repo_root, path)?;
    
    if let Some((_, reason)) = set.denied.iter().find(|(p, _)| p == &normalized) {
        bail!("Edit not allowed for {}: {}", path.display(), reason);
    }
    
    if set.binary.contains(&normalized) && !policy.allow_binary_edit {
        bail!("Edit not allowed for {}: binary file editing is disabled", path.display());
    }
    
    if set.generated.contains(&normalized) && !policy.allow_generated_edits {
        bail!("Edit not allowed for {}: generated file editing is disabled", path.display());
    }
    
    if !set.editable.contains(&normalized) && normalized.exists() {
        bail!("Edit not allowed for {}: file is not in editable set", path.display());
    }
    
    Ok(())
}

pub fn assert_delete_allowed(path: &Path, set: &FileSet, policy: &FilePolicy) -> Result<()> {
    if !policy.allow_delete {
        bail!("Delete operations are not allowed by policy");
    }
    
    assert_edit_allowed(path, set, policy)
}

pub fn assert_rename_allowed(from: &Path, to: &Path, set: &FileSet, policy: &FilePolicy) -> Result<()> {
    if !policy.allow_rename {
        bail!("Rename operations are not allowed by policy");
    }
    
    assert_edit_allowed(from, set, policy)?;
    
    let to_normalized = normalize_path(&policy.repo_root, to)?;
    if is_path_denied(&to_normalized, policy)? {
        bail!("Cannot rename to {}: target path is denied", to.display());
    }
    
    Ok(())
}

pub(crate) fn normalize_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    
    Ok(resolved.canonicalize().unwrap_or(resolved))
}

pub fn get_file_category(path: &Path, set: &FileSet) -> &'static str {
    let normalized = match normalize_path(Path::new("."), path) {
        Ok(p) => p,
        Err(_) => return "unknown",
    };
    
    if set.editable.contains(&normalized) {
        "editable"
    } else if set.readonly.contains(&normalized) {
        "readonly"
    } else if set.generated.contains(&normalized) {
        "generated"
    } else if set.binary.contains(&normalized) {
        "binary"
    } else if set.denied.iter().any(|(p, _)| p == &normalized) {
        "denied"
    } else {
        "unknown"
    }
}

pub fn get_file_stats(set: &FileSet) -> HashMap<&'static str, usize> {
    let mut stats = HashMap::new();
    stats.insert("editable", set.editable.len());
    stats.insert("readonly", set.readonly.len());
    stats.insert("generated", set.generated.len());
    stats.insert("binary", set.binary.len());
    stats.insert("denied", set.denied.len());
    stats
}
