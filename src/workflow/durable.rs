//! Crash-safe atomic JSON publication.
//!
//! Every mutable durable document (registry, identity, checkpoint, portable
//! work state export) is published through [`atomic_write_json`]: serialize →
//! write temp file → fsync → atomically rename → fsync the containing
//! directory. Every step propagates its error; a failed publication is
//! reported to the caller and never silently swallowed.

use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Write `value` as pretty JSON to `path` atomically and durably.
pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize document for {}", path.display()))?;
    let mut file = std::fs::File::create(&tmp)
        .with_context(|| format!("failed to create temp file {}", tmp.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to fsync temp file {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to atomically rename into {}", path.display()))?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

/// Atomically publish a current-format document that carries an explicit
/// `schema_version` field (injected on the serialized form).
///
/// Every current-format write goes through here so that no durable document
/// can be written without declaring its schema version.
pub fn versioned_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut v = serde_json::to_value(value)
        .with_context(|| format!("failed to serialize document for {}", path.display()))?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "schema_version".to_string(),
            serde_json::Value::String(super::schema::CURRENT_SCHEMA_VERSION.to_string_owned()),
        );
    }
    atomic_write_json(path, &v)
}

/// Write `bytes` to `path` atomically and durably (temp file → fsync → rename →
/// fsync dir). Used for non-JSON artifacts such as raw validation logs that
/// still require crash-safe publication.
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)
        .with_context(|| format!("failed to create temp file {}", tmp.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to fsync temp file {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to atomically rename into {}", path.display()))?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

/// Fsync a directory so a rename inside it is durable.
///
/// On this windows build `std::fs::File::open` cannot open a directory handle
/// (it requires `FILE_FLAG_BACKUP_SEMANTICS`, unsupported by std), so the
/// directory fsync is a best-effort no-op. On platforms where it is testable
/// (posix), a failure to open or fsync the directory is propagated so that an
/// undurable rename is surfaced as an error.
pub fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = dir;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let f = std::fs::File::open(dir)
            .with_context(|| format!("failed to open dir for fsync: {}", dir.display()))?;
        f.sync_all()
            .with_context(|| format!("failed to fsync dir: {}", dir.display()))?;
        Ok(())
    }
}

/// Render `path` as a forward-slash path relative to `repo`, when possible.
///
/// Used for durable document references (`evidence_ref`, `checkpoint_ref`) so
/// they are portable across machines and cannot accidentally escape the repo.
pub fn repo_relative_path(repo: &Path, path: &Path) -> String {
    let repo_abs = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let path_abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    match path_abs.strip_prefix(&repo_abs) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.display().to_string(),
    }
}

/// Normalize a repo-relative reference to a safe absolute path.
///
/// Rejects references that escape the repository (`..`, absolute paths) so a
/// hostile on-disk document cannot direct reads outside the repo.
pub fn resolve_repo_relative(repo: &Path, reference: &str) -> Result<std::path::PathBuf> {
    let p = Path::new(reference);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!("reference escapes repository: {reference}");
    }
    let resolved = repo.join(p);
    let repo_abs = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let resolved_abs = resolved
        .canonicalize()
        .unwrap_or_else(|_| resolved.to_path_buf());
    if !resolved_abs.starts_with(&repo_abs) {
        anyhow::bail!("reference escapes repository: {reference}");
    }
    Ok(resolved)
}

/// Resolve a workflow-scoped reference (a proposal id or similar) to
/// `<repo>/.prometheos/workflow/<id>`, rejecting empty, absolute,
/// current-dir and parent-traversing references so a hostile id can never
/// escape the workflow directory.
pub fn confined_workflow_dir(repo: &Path, id: &str) -> Result<std::path::PathBuf> {
    if id.is_empty() {
        anyhow::bail!("empty workflow reference");
    }
    let p = Path::new(id);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        anyhow::bail!("workflow reference escapes repository: {id}");
    }
    Ok(repo.join(".prometheos").join("workflow").join(p))
}
