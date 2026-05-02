use crate::harness::{
    edit_protocol::{EditOperation, validate_edit_operations},
    file_control::{FilePolicy, FileSet, normalize_path},
};
use anyhow::Result;
use diffy::create_patch;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFailure {
    pub file: PathBuf,
    pub operation: String,
    pub reason: String,
    pub nearby_context: Option<String>,
}
pub async fn dry_run_patch(
    e: &[EditOperation],
    s: &FileSet,
    p: &FilePolicy,
) -> Result<PatchResult> {
    run(e, s, p, true).await
}
pub async fn apply_patch(e: &[EditOperation], s: &FileSet, p: &FilePolicy) -> Result<PatchResult> {
    run(e, s, p, false).await
}
async fn run(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
    dry: bool,
) -> Result<PatchResult> {
    validate_edit_operations(edits, set, policy)?;
    let mut orig: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut next: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut failures = Vec::new();
    for e in edits {
        if let Err(f) = mem(e, policy, &mut orig, &mut next).await {
            failures.push(f)
        }
    }
    let diff = diff(&orig, &next);
    if !failures.is_empty() {
        return Ok(PatchResult {
            applied: false,
            changed_files: vec![],
            failures,
            diff,
            dry_run: dry,
        });
    }
    let changed = next.keys().cloned().collect::<Vec<_>>();
    if !dry {
        for (p, c) in &next {
            match c {
                Some(s) => fs::write(p, s).await?,
                None => {
                    if p.exists() {
                        fs::remove_file(p).await?
                    }
                }
            }
        }
    }
    Ok(PatchResult {
        applied: !dry,
        changed_files: changed,
        failures: vec![],
        diff,
        dry_run: dry,
    })
}
async fn mem(
    e: &EditOperation,
    policy: &FilePolicy,
    o: &mut BTreeMap<PathBuf, Option<String>>,
    n: &mut BTreeMap<PathBuf, Option<String>>,
) -> std::result::Result<(), PatchFailure> {
    match e {
        EditOperation::SearchReplace(x) => {
            let p = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "search_replace", e.to_string()))?;
            let c = fs::read_to_string(&p)
                .await
                .map_err(|e| fail(&x.file, "search_replace", e.to_string()))?;
            let count = c.matches(&x.search).count();
            if count != 1 {
                return Err(fail(
                    &x.file,
                    "search_replace",
                    format!("search block matched {count} times"),
                ));
            }
            o.insert(p.clone(), Some(c.clone()));
            n.insert(p, Some(c.replacen(&x.search, &x.replace, 1)));
        }
        EditOperation::WholeFile(x) => {
            let p = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "whole_file", e.to_string()))?;
            o.insert(p.clone(), fs::read_to_string(&p).await.ok());
            n.insert(p, Some(x.content.clone()));
        }
        EditOperation::CreateFile(x) => {
            let p = policy.repo_root.join(&x.file);
            o.insert(p.clone(), None);
            n.insert(p, Some(x.content.clone()));
        }
        EditOperation::DeleteFile(x) => {
            let p = normalize_path(&policy.repo_root, &x.file)
                .map_err(|e| fail(&x.file, "delete", e.to_string()))?;
            o.insert(p.clone(), fs::read_to_string(&p).await.ok());
            n.insert(p, None);
        }
        EditOperation::RenameFile(_) | EditOperation::UnifiedDiff(_) => {
            return Err(fail(
                Path::new("<edit>"),
                "unsupported",
                "operation not supported by atomic applier",
            ));
        }
    }
    Ok(())
}
fn diff(o: &BTreeMap<PathBuf, Option<String>>, n: &BTreeMap<PathBuf, Option<String>>) -> String {
    let mut keys = BTreeSet::new();
    keys.extend(o.keys().cloned());
    keys.extend(n.keys().cloned());
    let mut out = String::new();
    for k in keys {
        let a = o.get(&k).and_then(Clone::clone).unwrap_or_default();
        let b = n.get(&k).and_then(Clone::clone).unwrap_or_default();
        if a != b {
            out.push_str(&format!(
                "diff -- {}\n{}",
                k.display(),
                create_patch(&a, &b)
            ))
        }
    }
    out
}
fn fail(p: &Path, op: &str, r: impl Into<String>) -> PatchFailure {
    PatchFailure {
        file: p.into(),
        operation: op.into(),
        reason: r.into(),
        nearby_context: None,
    }
}
