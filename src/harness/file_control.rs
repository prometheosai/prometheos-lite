use crate::harness::repo_intelligence::RepoContext;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FileSet {
    pub editable: Vec<PathBuf>,
    pub readonly: Vec<PathBuf>,
    pub generated: Vec<PathBuf>,
    pub artifacts: Vec<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilePolicy {
    pub repo_root: PathBuf,
    pub allowed_write_paths: Vec<PathBuf>,
    pub denied_paths: Vec<PathBuf>,
    pub max_file_size_bytes: u64,
    pub allow_delete: bool,
    pub allow_rename: bool,
    pub allow_generated_edits: bool,
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
            ],
            max_file_size_bytes: 1_000_000,
            allow_delete: false,
            allow_rename: false,
            allow_generated_edits: false,
        }
    }
}
pub fn build_file_set(ctx: &RepoContext, _: &[PathBuf], policy: &FilePolicy) -> Result<FileSet> {
    let mut s = FileSet::default();
    for r in &ctx.ranked_files {
        if fs::metadata(&r.path)
            .map(|m| m.len() > policy.max_file_size_bytes)
            .unwrap_or(false)
        {
            s.readonly.push(r.path.clone())
        } else {
            s.editable.push(r.path.clone())
        }
    }
    Ok(s)
}
pub fn assert_edit_allowed(path: &Path, set: &FileSet, policy: &FilePolicy) -> Result<()> {
    let p = normalize_path(&policy.repo_root, path)?;
    if !p.starts_with(policy.repo_root.canonicalize()?) {
        bail!("outside repo root")
    }
    if p.exists() && !set.editable.contains(&p) {
        bail!("not editable")
    }
    Ok(())
}
pub(crate) fn normalize_path(root: &Path, path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
    .canonicalize()
    .unwrap_or_else(|_| root.join(path)))
}
