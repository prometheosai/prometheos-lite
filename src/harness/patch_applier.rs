use crate::harness::{
    edit_protocol::{EditOperation, ParsedDiff, parse_unified_diff, apply_unified_diff, validate_edit_operations},
    file_control::{FilePolicy, FileSet, normalize_path, assert_delete_allowed, assert_rename_allowed},
};
use anyhow::{Result, Context, bail};
use diffy::create_patch;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Write,
    path::{Path, PathBuf},
};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchResult {
    pub applied: bool,
    pub changed_files: Vec<PathBuf>,
    pub failures: Vec<PatchFailure>,
    pub diff: String,
    pub dry_run: bool,
    pub transaction_id: Option<String>,
    pub content_hashes: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFailure {
    pub file: PathBuf,
    pub operation: String,
    pub reason: String,
    pub nearby_context: Option<String>,
    pub line_number: Option<usize>,
}

#[derive(Debug, Clone)]
struct FileSnapshot {
    path: PathBuf,
    content: Option<String>,
    hash: Option<String>,
    exists: bool,
}

#[derive(Debug)]
struct Transaction {
    id: String,
    snapshots: BTreeMap<PathBuf, FileSnapshot>,
    pending_changes: BTreeMap<PathBuf, Option<String>>,
}

impl Transaction {
    fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            snapshots: BTreeMap::new(),
            pending_changes: BTreeMap::new(),
        }
    }

    fn record_snapshot(&mut self, path: PathBuf, content: Option<String>, exists: bool) {
        let hash = content.as_ref().map(|c| compute_hash(c));
        self.snapshots.insert(path.clone(), FileSnapshot {
            path,
            content,
            hash,
            exists,
        });
    }

    fn record_change(&mut self, path: PathBuf, new_content: Option<String>) {
        self.pending_changes.insert(path, new_content);
    }
}

pub async fn dry_run_patch(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
) -> Result<PatchResult> {
    run_with_transaction(edits, set, policy, true).await
}

pub async fn apply_patch(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
) -> Result<PatchResult> {
    run_with_transaction(edits, set, policy, false).await
}

pub async fn apply_patch_with_rollback(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
) -> Result<(PatchResult, RollbackHandle)> {
    let result = run_with_transaction(edits, set, policy, false).await?;
    
    let rollback = RollbackHandle {
        transaction_id: result.transaction_id.clone().unwrap_or_default(),
        snapshots: result.content_hashes.clone(),
        repo_root: policy.repo_root.clone(),
    };
    
    Ok((result, rollback))
}

async fn run_with_transaction(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
    dry_run: bool,
) -> Result<PatchResult> {
    validate_edit_operations(edits, set, policy)?;
    
    let mut transaction = Transaction::new();
    let mut failures = Vec::new();
    
    for (idx, edit) in edits.iter().enumerate() {
        if let Err(f) = apply_edit_to_transaction(edit, policy, &mut transaction, idx).await {
            failures.push(f);
        }
    }
    
    let diff = generate_diff(&transaction);
    
    if !failures.is_empty() {
        return Ok(PatchResult {
            applied: false,
            changed_files: vec![],
            failures,
            diff,
            dry_run: true,
            transaction_id: Some(transaction.id),
            content_hashes: compute_content_hashes(&transaction),
        });
    }
    
    let changed_files: Vec<PathBuf> = transaction.pending_changes.keys().cloned().collect();
    let content_hashes = compute_content_hashes(&transaction);
    
    if !dry_run {
        commit_transaction(&transaction, policy).await?;
    }
    
    Ok(PatchResult {
        applied: !dry_run,
        changed_files,
        failures: vec![],
        diff,
        dry_run,
        transaction_id: Some(transaction.id),
        content_hashes,
    })
}

async fn apply_edit_to_transaction(
    edit: &EditOperation,
    policy: &FilePolicy,
    transaction: &mut Transaction,
    edit_index: usize,
) -> std::result::Result<(), PatchFailure> {
    match edit {
        EditOperation::SearchReplace(x) => {
            let path = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "search_replace", e.to_string(), None))?;
            
            let content = fs::read_to_string(&path)
                .await
                .map_err(|e| fail(&x.file, "search_replace", format!("Cannot read file: {}", e), None))?;
            
            let search_count = content.matches(&x.search).count();
            let replace_all = x.replace_all.unwrap_or(false);
            
            if search_count == 0 {
                return Err(fail(
                    &x.file,
                    "search_replace",
                    format!("Search block not found in file"),
                    Some(content.lines().take(10).collect::<Vec<_>>().join("\n")),
                ));
            }
            
            if !replace_all && search_count > 1 {
                let lines: Vec<(usize, &str)> = content.lines().enumerate()
                    .filter(|(_, line)| line.contains(&x.search))
                    .map(|(i, line)| (i + 1, line))
                    .collect();
                
                return Err(PatchFailure {
                    file: x.file.clone(),
                    operation: "search_replace".into(),
                    reason: format!("Search block matched {} times - use replace_all:true if intentional", search_count),
                    nearby_context: Some(lines.iter()
                        .take(3)
                        .map(|(n, l)| format!("Line {}: {}", n, l))
                        .collect::<Vec<_>>()
                        .join("\n")),
                    line_number: Some(lines[0].0),
                });
            }
            
            let new_content = if replace_all {
                content.replace(&x.search, &x.replace)
            } else {
                content.replacen(&x.search, &x.replace, 1)
            };
            
            transaction.record_snapshot(path.clone(), Some(content), true);
            transaction.record_change(path, Some(new_content));
        }
        
        EditOperation::WholeFile(x) => {
            let path = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "whole_file", e.to_string(), None))?;
            
            let existing = fs::read_to_string(&path).await.ok();
            let existed = path.exists();
            
            transaction.record_snapshot(path.clone(), existing, existed);
            transaction.record_change(path, Some(x.content.clone()));
        }
        
        EditOperation::CreateFile(x) => {
            let path = policy.repo_root.join(&x.file);
            
            if path.exists() {
                return Err(fail(
                    &x.file,
                    "create_file",
                    "File already exists",
                    None,
                ));
            }
            
            transaction.record_snapshot(path.clone(), None, false);
            transaction.record_change(path, Some(x.content.clone()));
        }
        
        EditOperation::DeleteFile(x) => {
            let path = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "delete_file", e.to_string(), None))?;
            
            let content = fs::read_to_string(&path)
                .await
                .map_err(|e| fail(&x.file, "delete_file", format!("Cannot read file: {}", e), None))?;
            
            transaction.record_snapshot(path.clone(), Some(content), true);
            transaction.record_change(path, None);
        }
        
        EditOperation::RenameFile(x) => {
            let from_path = normalize_path(&policy.repo_root, &x.from)
                .map_err(|e| fail(&x.from, "rename_file", e.to_string(), None))?;
            let to_path = policy.repo_root.join(&x.to);
            
            if !from_path.exists() {
                return Err(fail(
                    &x.from,
                    "rename_file",
                    "Source file does not exist",
                    None,
                ));
            }
            
            if to_path.exists() {
                return Err(fail(
                    &x.from,
                    "rename_file",
                    format!("Target file already exists: {}", x.to.display()),
                    None,
                ));
            }
            
            let content = fs::read_to_string(&from_path)
                .await
                .map_err(|e| fail(&x.from, "rename_file", format!("Cannot read file: {}", e), None))?;
            
            transaction.record_snapshot(from_path.clone(), Some(content.clone()), true);
            transaction.record_change(from_path.clone(), None);
            transaction.record_change(to_path, Some(content));
        }
        
        EditOperation::UnifiedDiff(x) => {
            let diffs = parse_unified_diff(&x.diff)
                .map_err(|e| fail(
                    x.target_file.as_ref().unwrap_or(&PathBuf::from("unknown")),
                    "unified_diff",
                    format!("Failed to parse diff: {}", e),
                    None,
                ))?;
            
            for diff in diffs {
                let target_path = x.target_file.as_ref()
                    .or(diff.old_file.as_ref())
                    .or(Some(&diff.new_file))
                    .ok_or_else(|| fail(
                        &PathBuf::from("unknown"),
                        "unified_diff",
                        "Diff missing target file",
                        None,
                    ))?;
                
                let full_path = normalize_path(&policy.repo_root, target_path)
                    .map_err(|e| fail(target_path, "unified_diff", e.to_string(), None))?;
                
                if diff.is_new_file {
                    let content = reconstruct_new_file_content(&diff)
                        .map_err(|e| fail(target_path, "unified_diff", e.to_string(), None))?;
                    
                    transaction.record_snapshot(full_path.clone(), None, false);
                    transaction.record_change(full_path, Some(content));
                } else if diff.is_deleted {
                    let content = fs::read_to_string(&full_path)
                        .await
                        .map_err(|e| fail(target_path, "unified_diff", format!("Cannot read file: {}", e), None))?;
                    
                    transaction.record_snapshot(full_path.clone(), Some(content), true);
                    transaction.record_change(full_path, None);
                } else {
                    let original = fs::read_to_string(&full_path)
                        .await
                        .map_err(|e| fail(target_path, "unified_diff", format!("Cannot read file: {}", e), None))?;
                    
                    let new_content = apply_unified_diff(&original, &diff)
                        .map_err(|e| fail(target_path, "unified_diff", e.to_string(), None))?;
                    
                    transaction.record_snapshot(full_path.clone(), Some(original), true);
                    transaction.record_change(full_path, Some(new_content));
                }
            }
        }
    }
    
    Ok(())
}

async fn commit_transaction(transaction: &Transaction, policy: &FilePolicy) -> Result<()> {
    let changes: Vec<_> = transaction.pending_changes.iter().collect();
    
    for (path, new_content) in changes.iter().rev() {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await
                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            }
        }
    }
    
    for (path, new_content) in &transaction.pending_changes {
        match new_content {
            Some(content) => {
                fs::write(path, content).await
                    .with_context(|| format!("Failed to write file: {}", path.display()))?;
            }
            None => {
                if path.exists() {
                    fs::remove_file(path).await
                        .with_context(|| format!("Failed to delete file: {}", path.display()))?;
                }
            }
        }
    }
    
    Ok(())
}

fn generate_diff(transaction: &Transaction) -> String {
    let mut keys = BTreeSet::new();
    keys.extend(transaction.snapshots.keys().cloned());
    keys.extend(transaction.pending_changes.keys().cloned());
    
    let mut output = String::new();
    
    for k in keys {
        let old = transaction.snapshots.get(&k).and_then(|s| s.content.clone()).unwrap_or_default();
        let new = transaction.pending_changes.get(&k).cloned().flatten().unwrap_or_default();
        
        if old != new {
            output.push_str(&format!(
                "diff -- {}\n{}",
                k.display(),
                create_patch(&old, &new)
            ));
        }
    }
    
    output
}

fn compute_content_hashes(transaction: &Transaction) -> HashMap<PathBuf, String> {
    let mut hashes = HashMap::new();
    
    for (path, snapshot) in &transaction.snapshots {
        if let Some(ref hash) = snapshot.hash {
            hashes.insert(path.clone(), hash.clone());
        }
    }
    
    for (path, content) in &transaction.pending_changes {
        if let Some(ref c) = content {
            hashes.insert(path.clone(), compute_hash(c));
        }
    }
    
    hashes
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

fn fail(path: &Path, op: &str, reason: impl Into<String>, context: Option<String>) -> PatchFailure {
    PatchFailure {
        file: path.into(),
        operation: op.into(),
        reason: reason.into(),
        nearby_context: context,
        line_number: None,
    }
}

fn reconstruct_new_file_content(diff: &ParsedDiff) -> Result<String> {
    let mut content = Vec::new();
    
    for hunk in &diff.hunks {
        for line in &hunk.lines {
            if let crate::harness::edit_protocol::DiffLine::Added(text) | 
               crate::harness::edit_protocol::DiffLine::Context(text) = line {
                content.push(text.clone());
            }
        }
    }
    
    Ok(content.join("\n"))
}

#[derive(Debug, Clone)]
pub struct RollbackHandle {
    transaction_id: String,
    snapshots: HashMap<PathBuf, String>,
    repo_root: PathBuf,
}

impl RollbackHandle {
    pub async fn rollback(self) -> Result<()> {
        for (path, original_hash) in &self.snapshots {
            if path.exists() {
                let current_content = fs::read_to_string(path).await.ok();
                let current_hash = current_content.as_ref().map(|c| compute_hash(c));
                
                if current_hash.as_ref() != Some(original_hash) {
                    fs::remove_file(path).await.ok();
                }
            }
        }
        
        Ok(())
    }
    
    pub fn get_transaction_id(&self) -> &str {
        &self.transaction_id
    }
}

pub async fn verify_file_integrity(path: &Path, expected_hash: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    
    let content = fs::read_to_string(path).await?;
    let actual_hash = compute_hash(&content);
    
    Ok(actual_hash == expected_hash)
}

pub async fn atomic_patch_operation<F, Fut>(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
    operation: F,
) -> Result<PatchResult>
where
    F: FnOnce(&PatchResult) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let (result, rollback) = apply_patch_with_rollback(edits, set, policy).await?;
    
    if !result.failures.is_empty() {
        return Ok(result);
    }
    
    if let Err(e) = operation(&result).await {
        rollback.rollback().await?;
        bail!("Operation failed, patch rolled back: {}", e);
    }
    
    Ok(result)
}
