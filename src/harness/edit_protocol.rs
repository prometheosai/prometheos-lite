use crate::harness::file_control::{FilePolicy, FileSet, assert_edit_allowed, normalize_path};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    pub replace_all: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnifiedDiffEdit {
    pub diff: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_file: Option<PathBuf>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDiff {
    pub old_file: Option<PathBuf>,
    pub new_file: PathBuf,
    pub hunks: Vec<DiffHunk>,
    pub is_new_file: bool,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
    pub header: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

pub fn parse_edit_response(raw: &str) -> Result<Vec<EditOperation>> {
    let t = raw.trim();

    if let Ok(v) = serde_json::from_str::<Vec<EditOperation>>(t) {
        return Ok(v);
    }
    if let Ok(v) = serde_json::from_str::<EditOperation>(t) {
        return Ok(vec![v]);
    }

    if t.starts_with("---") || t.contains("diff --git") || t.contains("@@") {
        let edits = parse_unified_diff_text(t)?;
        if !edits.is_empty() {
            return Ok(edits);
        }
    }

    bail!("unknown edit protocol: input is neither valid JSON nor recognized diff format")
}

fn parse_unified_diff_text(diff_text: &str) -> Result<Vec<EditOperation>> {
    let mut edits = Vec::new();
    let diffs = parse_unified_diff(diff_text)?;

    for diff in diffs {
        if diff.is_new_file {
            let content = reconstruct_new_file_content(&diff)?;
            edits.push(EditOperation::CreateFile(CreateFileEdit {
                file: diff.new_file,
                content,
                executable: None,
            }));
        } else if diff.is_deleted {
            edits.push(EditOperation::DeleteFile(DeleteFileEdit {
                file: diff.old_file.unwrap_or(diff.new_file.clone()),
            }));
        } else {
            edits.push(EditOperation::UnifiedDiff(UnifiedDiffEdit {
                diff: render_hunks(&diff),
                target_file: Some(diff.new_file),
            }));
        }
    }

    Ok(edits)
}

pub fn parse_unified_diff(diff_text: &str) -> Result<Vec<ParsedDiff>> {
    let mut diffs = Vec::new();
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].starts_with("diff --git") || lines[i].starts_with("---") {
            let (diff, consumed) = parse_single_diff(&lines[i..])
                .with_context(|| format!("Failed to parse diff at line {}", i + 1))?;
            diffs.push(diff);
            i += consumed;
        } else {
            i += 1;
        }
    }

    Ok(diffs)
}

fn parse_single_diff(lines: &[&str]) -> Result<(ParsedDiff, usize)> {
    let mut i = 0;
    let mut old_file: Option<PathBuf> = None;
    let mut new_file: PathBuf = PathBuf::new();
    let mut is_new_file = false;
    let mut is_deleted = false;
    let mut hunks = Vec::new();

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("diff --git") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                old_file = Some(PathBuf::from(&parts[2][2..]));
                new_file = PathBuf::from(&parts[3][2..]);
            }
            i += 1;
            continue;
        }

        if line.starts_with("---") {
            let content = &line[4..];
            if content == "/dev/null" {
                is_new_file = true;
            } else if content.starts_with('"') && content.ends_with('"') {
                old_file = Some(PathBuf::from(&content[1..content.len() - 1]));
            } else {
                let path = content.split_whitespace().next().unwrap_or("");
                if path.starts_with("a/") || path.starts_with("b/") {
                    old_file = Some(PathBuf::from(&path[2..]));
                } else {
                    old_file = Some(PathBuf::from(path));
                }
            }
            i += 1;
            continue;
        }

        if line.starts_with("+++") {
            let content = &line[4..];
            if content == "/dev/null" {
                is_deleted = true;
            } else if content.starts_with('"') && content.ends_with('"') {
                new_file = PathBuf::from(&content[1..content.len() - 1]);
            } else {
                let path = content.split_whitespace().next().unwrap_or("");
                if path.starts_with("a/") || path.starts_with("b/") {
                    new_file = PathBuf::from(&path[2..]);
                } else {
                    new_file = PathBuf::from(path);
                }
            }
            i += 1;
            continue;
        }

        if line.starts_with("@@") {
            let (hunk, consumed) = parse_hunk(&lines[i..])?;
            hunks.push(hunk);
            i += consumed;
            continue;
        }

        if line.starts_with("diff --git") && i > 0 {
            break;
        }

        i += 1;
    }

    if new_file.as_os_str().is_empty() && old_file.is_some() {
        new_file = old_file.clone().unwrap();
    }

    let diff = ParsedDiff {
        old_file,
        new_file,
        hunks,
        is_new_file,
        is_deleted,
    };

    Ok((diff, i))
}

fn parse_hunk(lines: &[&str]) -> Result<(DiffHunk, usize)> {
    let header = lines[0];
    let header_re = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();

    let caps = header_re
        .captures(header)
        .ok_or_else(|| anyhow::anyhow!("Invalid hunk header: {}", header))?;

    let old_start: usize = caps[1].parse()?;
    let old_lines: usize = caps
        .get(2)
        .map(|m| m.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    let new_start: usize = caps[3].parse()?;
    let new_lines: usize = caps
        .get(4)
        .map(|m| m.as_str().parse().unwrap_or(0))
        .unwrap_or(0);

    let mut diff_lines = Vec::new();
    let mut i = 1;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("@@") || line.starts_with("diff --git") {
            break;
        }

        if line.starts_with('+') {
            diff_lines.push(DiffLine::Added(line[1..].to_string()));
        } else if line.starts_with('-') && !line.starts_with("---") {
            diff_lines.push(DiffLine::Removed(line[1..].to_string()));
        } else if line.starts_with(' ') || line.is_empty() {
            let content = if line.is_empty() { "" } else { &line[1..] };
            diff_lines.push(DiffLine::Context(content.to_string()));
        } else if line.starts_with("\\") {
            // "\ No newline at end of file" - metadata line, skip
        }

        i += 1;
    }

    let hunk = DiffHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines: diff_lines,
        header: header.to_string(),
    };

    Ok((hunk, i))
}

fn reconstruct_new_file_content(diff: &ParsedDiff) -> Result<String> {
    let mut content = Vec::new();

    for hunk in &diff.hunks {
        for line in &hunk.lines {
            if let DiffLine::Added(text) | DiffLine::Context(text) = line {
                content.push(text.clone());
            }
        }
    }

    Ok(content.join("\n"))
}

fn render_hunks(diff: &ParsedDiff) -> String {
    let mut output = String::new();

    for hunk in &diff.hunks {
        output.push_str(&hunk.header);
        output.push('\n');

        for line in &hunk.lines {
            match line {
                DiffLine::Context(text) => {
                    output.push(' ');
                    output.push_str(text);
                    output.push('\n');
                }
                DiffLine::Added(text) => {
                    output.push('+');
                    output.push_str(text);
                    output.push('\n');
                }
                DiffLine::Removed(text) => {
                    output.push('-');
                    output.push_str(text);
                    output.push('\n');
                }
            }
        }
    }

    output
}

/// Apply a unified diff to original content
///
/// This function correctly handles context lines - they are verified but not modified.
/// Only Removed lines are deleted and Added lines are inserted.
pub fn apply_unified_diff(original: &str, diff: &ParsedDiff) -> Result<String> {
    let original_lines: Vec<&str> = original.lines().collect();
    let mut result: Vec<String> = original_lines.iter().map(|s| s.to_string()).collect();
    let mut offset: i64 = 0;

    for (hunk_idx, hunk) in diff.hunks.iter().enumerate() {
        let insert_pos = (hunk.old_start as i64 - 1 + offset) as usize;

        if insert_pos > result.len() {
            bail!(
                "Hunk {} offset out of bounds: trying to insert at line {}, but file only has {} lines",
                hunk_idx + 1,
                insert_pos + 1,
                result.len()
            );
        }

        // Verify context lines match before applying
        let mut verify_pos = insert_pos;
        for line in &hunk.lines {
            if let DiffLine::Context(expected) = line {
                if verify_pos < result.len() {
                    let actual = &result[verify_pos];
                    // Allow fuzzy matching - context might have slight differences
                    if actual.trim() != expected.trim() {
                        bail!(
                            "Hunk {} context mismatch at line {}: expected '{}', found '{}'",
                            hunk_idx + 1,
                            verify_pos + 1,
                            expected,
                            actual
                        );
                    }
                }
                verify_pos += 1;
            }
        }

        // Collect operations: (position, operation, content)
        // We process in reverse order to maintain correct indices
        let mut operations: Vec<(usize, char, Option<String>)> = Vec::new();
        let mut line_idx = insert_pos;

        for line in &hunk.lines {
            match line {
                DiffLine::Context(_) => {
                    // Context lines: keep as-is, just advance
                    line_idx += 1;
                }
                DiffLine::Removed(_) => {
                    // Mark for removal
                    operations.push((line_idx, '-', None));
                }
                DiffLine::Added(text) => {
                    // Mark for insertion
                    operations.push((line_idx, '+', Some(text.clone())));
                    // For added lines, we don't advance line_idx since we're inserting
                }
            }
        }

        // Sort operations: first by position (descending), then removals before insertions
        operations.sort_by(|a, b| {
            let pos_cmp = b.0.cmp(&a.0); // Reverse position order
            if pos_cmp != std::cmp::Ordering::Equal {
                return pos_cmp;
            }
            // At same position, removals ('-') come before insertions ('+')
            b.1.cmp(&a.1)
        });

        // Apply operations
        for (pos, op, content) in operations {
            match op {
                '-' => {
                    // Remove line
                    if pos < result.len() {
                        result.remove(pos);
                    }
                }
                '+' => {
                    // Insert line
                    if let Some(text) = content {
                        result.insert(pos, text);
                    }
                }
                _ => {}
            }
        }

        // Update offset for next hunk
        offset += (hunk.new_lines as i64) - (hunk.old_lines as i64);
    }

    Ok(result.join("\n"))
}

pub fn validate_edit_operations(
    edits: &[EditOperation],
    set: &FileSet,
    policy: &FilePolicy,
) -> Result<()> {
    let mut seen_files: HashMap<PathBuf, usize> = HashMap::new();

    for (idx, e) in edits.iter().enumerate() {
        match e {
            EditOperation::SearchReplace(x) => {
                if x.search == x.replace {
                    bail!("Edit {} has identical search and replace content", idx);
                }
                if x.search.is_empty() && x.replace.is_empty() {
                    bail!("Edit {} has empty search and replace", idx);
                }
                assert_edit_allowed(&x.file, set, policy)?;
                *seen_files.entry(x.file.clone()).or_insert(0) += 1;
            }
            EditOperation::WholeFile(x) => {
                assert_edit_allowed(&x.file, set, policy)?;
                if x.content.is_empty() {
                    bail!(
                        "Whole file edit for {} would result in empty file",
                        x.file.display()
                    );
                }
                *seen_files.entry(x.file.clone()).or_insert(0) += 1;
            }
            EditOperation::CreateFile(x) => {
                let normalized = normalize_path(&policy.repo_root, &x.file)?;
                if normalized.exists() {
                    bail!("Cannot create {}: file already exists", x.file.display());
                }
                if is_path_denied(&normalized, policy)? {
                    bail!("Cannot create {}: path is denied", x.file.display());
                }
                *seen_files.entry(x.file.clone()).or_insert(0) += 1;
            }
            EditOperation::DeleteFile(x) => {
                if !policy.allow_delete {
                    bail!("Delete operations are disabled by policy");
                }
                assert_edit_allowed(&x.file, set, policy)?;
                *seen_files.entry(x.file.clone()).or_insert(0) += 1;
            }
            EditOperation::RenameFile(x) => {
                if !policy.allow_rename {
                    bail!("Rename operations are disabled by policy");
                }
                assert_edit_allowed(&x.from, set, policy)?;

                let to_normalized = normalize_path(&policy.repo_root, &x.to)?;
                if is_path_denied(&to_normalized, policy)? {
                    bail!("Cannot rename to {}: target path is denied", x.to.display());
                }
            }
            EditOperation::UnifiedDiff(x) => {
                if !x.diff.contains("@@") {
                    bail!("Invalid unified diff: missing hunk markers (@@)");
                }

                let parsed = parse_unified_diff(&x.diff)?;
                if parsed.is_empty() {
                    bail!("Invalid unified diff: no hunks found");
                }

                for diff in &parsed {
                    if !diff.is_new_file {
                        let target = x
                            .target_file
                            .as_ref()
                            .or(diff.old_file.as_ref())
                            .or(Some(&diff.new_file))
                            .ok_or_else(|| anyhow::anyhow!("Diff missing target file"))?;
                        assert_edit_allowed(target, set, policy)?;
                    }
                }
            }
        }
    }

    for (file, count) in seen_files {
        if count > 1 {
            bail!(
                "File {} is modified by {} different edits - consider consolidating",
                file.display(),
                count
            );
        }
    }

    Ok(())
}

fn is_path_denied(path: &Path, policy: &FilePolicy) -> Result<bool> {
    let canonical = path.canonicalize()?;
    let repo_root = policy.repo_root.canonicalize()?;

    if !canonical.starts_with(&repo_root) {
        return Ok(true);
    }

    let relative = canonical
        .strip_prefix(&repo_root)
        .map_err(|_| anyhow::anyhow!("Failed to get relative path"))?;

    for denied in &policy.denied_paths {
        if relative.starts_with(denied) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn get_edit_summary(edits: &[EditOperation]) -> String {
    let mut creates = 0;
    let mut modifications = 0;
    let mut deletions = 0;
    let mut renames = 0;

    for edit in edits {
        match edit {
            EditOperation::CreateFile(_) => creates += 1,
            EditOperation::DeleteFile(_) => deletions += 1,
            EditOperation::RenameFile(_) => renames += 1,
            _ => modifications += 1,
        }
    }

    let mut parts = Vec::new();
    if creates > 0 {
        parts.push(format!("{} create(s)", creates));
    }
    if modifications > 0 {
        parts.push(format!("{} modification(s)", modifications));
    }
    if deletions > 0 {
        parts.push(format!("{} deletion(s)", deletions));
    }
    if renames > 0 {
        parts.push(format!("{} rename(s)", renames));
    }

    if parts.is_empty() {
        "No changes".to_string()
    } else {
        parts.join(", ")
    }
}

pub fn merge_edits(edits: Vec<EditOperation>) -> Vec<EditOperation> {
    let mut by_file: HashMap<PathBuf, Vec<EditOperation>> = HashMap::new();

    for edit in edits {
        let file = match &edit {
            EditOperation::SearchReplace(x) => x.file.clone(),
            EditOperation::WholeFile(x) => x.file.clone(),
            EditOperation::CreateFile(x) => x.file.clone(),
            EditOperation::DeleteFile(x) => x.file.clone(),
            EditOperation::RenameFile(x) => x.from.clone(),
            EditOperation::UnifiedDiff(x) => x.target_file.clone().unwrap_or_default(),
        };

        by_file.entry(file).or_default().push(edit);
    }

    let mut merged = Vec::new();

    for (file, file_edits) in by_file {
        if file_edits.len() == 1 {
            merged.push(file_edits.into_iter().next().unwrap());
        } else {
            let whole_file = file_edits
                .iter()
                .find(|e| matches!(e, EditOperation::WholeFile(_)));

            if let Some(wf) = whole_file {
                merged.push(wf.clone());
            } else {
                let create = file_edits
                    .iter()
                    .find(|e| matches!(e, EditOperation::CreateFile(_)));
                if let Some(c) = create {
                    merged.push(c.clone());
                } else {
                    merged.extend(file_edits);
                }
            }
        }
    }

    merged
}
