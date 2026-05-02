use crate::harness::file_control::{FilePolicy, FileSet, assert_edit_allowed, normalize_path};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditOperation {
    SearchReplace(SearchReplaceEdit),
    UnifiedDiff(UnifiedDiffEdit),
    WholeFile(WholeFileEdit),
    CreateFile(CreateFileEdit),
    DeleteFile(DeleteFileEdit),
    RenameFile(RenameFileEdit),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchReplaceEdit {
    pub file: PathBuf,
    pub search: String,
    pub replace: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnifiedDiffEdit {
    pub diff: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WholeFileEdit {
    pub file: PathBuf,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateFileEdit {
    pub file: PathBuf,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteFileEdit {
    pub file: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameFileEdit {
    pub from: PathBuf,
    pub to: PathBuf,
}
pub fn parse_edit_response(raw: &str) -> Result<Vec<EditOperation>> {
    let t = raw.trim();
    if let Ok(v) = serde_json::from_str::<Vec<EditOperation>>(t) {
        return Ok(v);
    }
    if let Ok(v) = serde_json::from_str::<EditOperation>(t) {
        return Ok(vec![v]);
    }
    bail!("unknown edit protocol")
}
pub fn validate_edit_operations(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
) -> Result<()> {
    for e in edits {
        match e {
            EditOperation::SearchReplace(x) => assert_edit_allowed(&x.file, set, policy)?,
            EditOperation::WholeFile(x) => assert_edit_allowed(&x.file, set, policy)?,
            EditOperation::CreateFile(x) => {
                if normalize_path(&policy.repo_root, &x.file)?.exists() {
                    bail!("exists")
                }
            }
            EditOperation::DeleteFile(x) => {
                if !policy.allow_delete {
                    bail!("delete disabled")
                }
                assert_edit_allowed(&x.file, set, policy)?
            }
            EditOperation::RenameFile(x) => {
                if !policy.allow_rename {
                    bail!("rename disabled")
                }
                assert_edit_allowed(&x.from, set, policy)?
            }
            EditOperation::UnifiedDiff(x) => {
                if !x.diff.contains("@@") {
                    bail!("malformed diff")
                }
            }
        }
    }
    Ok(())
}
