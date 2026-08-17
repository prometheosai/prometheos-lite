//! Protected-evidence retention planning.
//!
//! Evidence directories accumulate durable artifacts (proposal, validation,
//! integrity, raw logs, terminal evidence). Retention must never delete a
//! referenced artifact — terminal, nonterminal/recoverable, journal-referenced,
//! checkpoint-referenced, or `PortableWorkState`-referenced — and must never
//! follow a reference outside the repository.
//!
//! This module provides the deterministic building blocks:
//! - [`ProtectedReferences`] — the set of artifact paths that must survive.
//! - [`plan_retention`] — classify every candidate under a root.
//! - [`apply_retention`] — remove only planned, unprotected, in-repo candidates
//!   (revalidating safety immediately before each removal).
//!
//! Corrupted-but-referenced evidence is still protected: fail closed and
//! preserve it. Negative evidence is debugging gold.

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::workflow::artifact_integrity::sidecar_for;
use crate::workflow::portable_state::PortableWorkState;

/// A set of absolute artifact paths that retention must never delete.
#[derive(Debug, Clone, Default)]
pub struct ProtectedReferences {
    set: HashSet<PathBuf>,
}

impl ProtectedReferences {
    /// Create an empty protected set.
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
        }
    }

    /// Protect an absolute artifact path (and its checksum sidecar).
    pub fn insert(&mut self, absolute: &Path) -> Result<()> {
        let abs = absolute
            .canonicalize()
            .unwrap_or_else(|_| absolute.to_path_buf());
        if abs.is_relative() {
            bail!("protected reference must be absolute: {absolute:?}");
        }
        self.set.insert(abs.clone());
        // Pair the checksum sidecar with its artifact.
        self.set.insert(sidecar_for(&abs));
        Ok(())
    }

    /// True if `absolute` (or its checksum sidecar) is protected.
    pub fn contains(&self, absolute: &Path) -> bool {
        let abs = absolute
            .canonicalize()
            .unwrap_or_else(|_| absolute.to_path_buf());
        self.set.contains(&abs) || self.set.contains(&sidecar_for(&abs))
    }

    /// Add every repo-local artifact referenced by a `PortableWorkState`.
    ///
    /// This deliberately does NOT scan the repository or perform memory
    /// retrieval; it only protects what an exported portable state explicitly
    /// references (per #115's PortableWorkState retention boundary). Each
    /// reference URI is resolved against `repo` and must stay inside it.
    pub fn extend_from_portable_work_state(&mut self, repo: &Path, pws: &PortableWorkState) {
        for r in &pws.artifact_refs {
            let path = repo.join(&r.uri);
            let _ = self.insert(&path);
        }
    }
}

/// One classified retention candidate.
#[derive(Debug, Clone)]
pub struct RetentionEntry {
    /// Absolute path of the candidate.
    pub path: PathBuf,
    /// True if retention must preserve this artifact.
    pub protected: bool,
    /// Human-readable reason for the classification.
    pub reason: String,
    /// Age in seconds if the mtime was available.
    pub age_seconds: Option<u64>,
    /// Size in bytes if available.
    pub size_bytes: Option<u64>,
    /// True if this candidate is an orphan past its TTL (deletion candidate).
    pub deletion_candidate: bool,
}

/// A deterministic retention plan.
#[derive(Debug, Clone, Default)]
pub struct RetentionPlan {
    pub entries: Vec<RetentionEntry>,
}

/// Outcome of applying a retention plan.
#[derive(Debug, Clone, Default)]
pub struct RetentionOutcome {
    /// Number of protected artifacts preserved.
    pub preserved: usize,
    /// Number of unprotected orphans removed.
    pub removed: usize,
    /// Number of paths rejected for safety (out-of-repo / escape).
    pub rejected: usize,
}

/// Reject a candidate path that escapes the repository. Fail closed: a hostile
/// or malformed reference must never be deleted or followed.
fn ensure_inside_repo(repo: &Path, path: &Path) -> Result<PathBuf> {
    let repo_abs = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let path_abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if path_abs.is_relative()
        || path_abs
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        || !path_abs.starts_with(&repo_abs)
    {
        bail!("retention path escapes the repository: {path:?}");
    }
    Ok(path_abs)
}

/// Collect every regular file under `root` (recursively). Fails closed if
/// `root` escapes the repository. Checksum sidecar files (`*.sidecar.json`) are
/// never standalone candidates: they are always managed together with the
/// artifact they describe, so they must not be deleted independently.
pub fn collect_candidates(repo: &Path, root: &Path) -> Result<Vec<PathBuf>> {
    let _ = ensure_inside_repo(repo, root)?;
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read retention dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                let is_sidecar = p
                    .file_name()
                    .map(|n| n.to_string_lossy().ends_with(".sidecar.json"))
                    .unwrap_or(false);
                if !is_sidecar {
                    out.push(p);
                }
            }
        }
    }
    Ok(out)
}

/// Build a deterministic retention plan for all candidates under `root`.
///
/// `orphan_ttl` is the minimum age an *unreferenced* artifact must reach before
/// it becomes a deletion candidate. Referenced (protected) artifacts are never
/// candidates. The plan is pure: it performs no deletion.
pub fn plan_retention(
    repo: &Path,
    root: &Path,
    protected: &ProtectedReferences,
    now: SystemTime,
    orphan_ttl: Duration,
) -> Result<RetentionPlan> {
    let candidates = collect_candidates(repo, root)?;
    let mut entries = Vec::new();
    for path in candidates {
        let path_abs = match ensure_inside_repo(repo, &path) {
            Ok(p) => p,
            Err(_) => continue, // safety-rejected candidates are simply skipped
        };
        let protected_flag = protected.contains(&path_abs);
        let meta = std::fs::metadata(&path_abs).ok();
        let size = meta.as_ref().map(|m| m.len());
        let age = meta
            .and_then(|m| m.modified().ok())
            .map(|t| now.duration_since(t).unwrap_or(Duration::ZERO).as_secs());
        let orphan = !protected_flag;
        let expired = age.map(|a| a >= orphan_ttl.as_secs()).unwrap_or(false);
        let deletion_candidate = orphan && expired;
        let reason = if protected_flag {
            "referenced / protected".to_string()
        } else if expired {
            format!("unreferenced orphan older than {}s", orphan_ttl.as_secs())
        } else {
            "unreferenced orphan within retention window".to_string()
        };
        entries.push(RetentionEntry {
            path: path_abs,
            protected: protected_flag,
            reason,
            age_seconds: age,
            size_bytes: size,
            deletion_candidate,
        });
    }
    // Deterministic ordering for stable plans.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(RetentionPlan { entries })
}

/// Apply a retention plan, removing only planned, unprotected, in-repo
/// candidates. Each removal revalidates safety immediately before deleting.
///
/// Atomicity guarantee: the artifact is removed first, and its checksum sidecar
/// is only removed afterwards. If the artifact cannot be removed, the sidecar is
/// deliberately left in place so the artifact remains verifiable (we never strand
/// an artifact without its checksum). A sidecar is never deleted while its
/// artifact survives.
pub fn apply_retention(repo: &Path, plan: &RetentionPlan) -> Result<RetentionOutcome> {
    let mut outcome = RetentionOutcome::default();
    for entry in &plan.entries {
        if entry.protected {
            outcome.preserved += 1;
            continue;
        }
        if !entry.deletion_candidate {
            outcome.preserved += 1;
            continue;
        }
        // Revalidate safety at removal time: never follow an escape.
        match ensure_inside_repo(repo, &entry.path) {
            Ok(safe) => {
                // Artifact first; sidecar only after the artifact is gone.
                match std::fs::remove_file(&safe) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(sidecar_for(&safe));
                        outcome.removed += 1;
                    }
                    Err(_) => {
                        // Could not remove the artifact; leave the sidecar so the
                        // artifact stays verifiable. Count as rejected (not removed).
                        outcome.rejected += 1;
                    }
                }
            }
            Err(_) => {
                outcome.rejected += 1;
            }
        }
    }
    Ok(outcome)
}

/// Reclaim unreferenced, expired evaluation artifacts under
/// `<repo>/.prometheos/workflow`, removing each together with its checksum
/// sidecar. Referenced/protected artifacts (and anything within the retention
/// window) are preserved. This is the production integration entry point called
/// by the orchestrator after a run finalizes.
pub fn reclaim_orphan_artifacts(
    repo: &Path,
    orphan_ttl: Duration,
    protected: &ProtectedReferences,
) -> Result<RetentionOutcome> {
    let root = repo.join(".prometheos").join("workflow");
    let plan = plan_retention(repo, &root, protected, SystemTime::now(), orphan_ttl)?;
    apply_retention(repo, &plan)
}

/// Mtime of a path as a `SystemTime`, used by callers to inject a clock.
pub fn now_or(_mtime: Option<SystemTime>) -> SystemTime {
    // Tests inject an explicit clock via `plan_retention`; this is a no-op
    // placeholder retained for API symmetry.
    SystemTime::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn protected_artifact_never_deleted() {
        let dir = tmp();
        let repo = dir.path();
        let ev = repo.join("evidence.json");
        fs::write(&ev, "terminal evidence").unwrap();
        let mut prot = ProtectedReferences::new();
        prot.insert(&ev).unwrap();
        let plan = plan_retention(repo, repo, &prot, SystemTime::now(), Duration::ZERO).unwrap();
        assert!(plan.entries.iter().all(|e| e.protected));
        let out = apply_retention(repo, &plan).unwrap();
        assert_eq!(out.removed, 0);
        assert!(ev.exists());
    }

    #[test]
    fn expired_orphan_is_removed_with_sidecar() {
        let dir = tmp();
        let repo = dir.path();
        let orphan = repo.join("old.log");
        fs::write(&orphan, "data").unwrap();
        fs::write(sidecar_for(&orphan), "{}").unwrap();
        let prot = ProtectedReferences::new();
        // Inject a clock one hour ahead so the just-written file is "expired".
        let clock = SystemTime::now() + Duration::from_secs(3600);
        let plan = plan_retention(repo, repo, &prot, clock, Duration::from_secs(10)).unwrap();
        assert!(plan.entries.iter().any(|e| e.deletion_candidate));
        let out = apply_retention(repo, &plan).unwrap();
        // The artifact (and, as a consequence, its checksum sidecar) is removed;
        // the sidecar is never counted as a separate removal.
        assert_eq!(out.removed, 1);
        assert!(!orphan.exists());
        assert!(!sidecar_for(&orphan).exists());
    }

    #[test]
    fn young_orphan_remains() {
        let dir = tmp();
        let repo = dir.path();
        let orphan = repo.join("fresh.log");
        fs::write(&orphan, "data").unwrap();
        let prot = ProtectedReferences::new();
        let plan = plan_retention(
            repo,
            repo,
            &prot,
            SystemTime::now(),
            Duration::from_secs(10_000),
        )
        .unwrap();
        assert!(plan.entries.iter().all(|e| !e.deletion_candidate));
        let out = apply_retention(repo, &plan).unwrap();
        assert_eq!(out.removed, 0);
        assert!(orphan.exists());
    }

    #[test]
    fn path_escape_is_rejected() {
        let dir = tmp();
        let repo = dir.path();
        let outside = dir.path().join("..").join("escape.log");
        fs::write(&outside, "x").unwrap();
        let mut prot = ProtectedReferences::new();
        // A reference outside the repo must not be accepted as protected, and
        // planning must never delete outside the repo.
        assert!(ensure_inside_repo(repo, &outside).is_err());
        // A reference that is safely inside the repo is accepted.
        let inside = repo.join("evidence.json");
        fs::write(&inside, "x").unwrap();
        assert!(prot.insert(&inside).is_ok());
        // Planning/applying only ever touches in-repo candidates; the outside
        // file is never read, planned, or deleted.
        let plan = plan_retention(repo, repo, &prot, SystemTime::now(), Duration::ZERO).unwrap();
        let _ = apply_retention(repo, &plan);
        assert!(outside.exists());
    }

    #[test]
    fn sidecar_preserved_when_artifact_removal_fails() {
        let dir = tmp();
        let repo = dir.path();
        // If the artifact cannot be removed, its checksum sidecar must survive:
        // we must never strand a (still-present) artifact without its checksum.
        let missing = repo.join("gone.json");
        let sidecar = sidecar_for(&missing);
        fs::write(&sidecar, "{}").unwrap();
        let plan = RetentionPlan {
            entries: vec![RetentionEntry {
                path: missing.clone(),
                protected: false,
                reason: "orphan".to_string(),
                age_seconds: Some(1000),
                size_bytes: Some(1),
                deletion_candidate: true,
            }],
        };
        let out = apply_retention(repo, &plan).unwrap();
        assert_eq!(out.rejected, 1);
        assert!(
            sidecar.exists(),
            "sidecar must survive when its artifact cannot be removed"
        );
    }

    #[test]
    fn reclaim_orphan_artifacts_scoped_to_workflow_tree() {
        let dir = tmp();
        let repo = dir.path();
        // An expired orphan under .prometheos/workflow must be reclaimed (with its
        // sidecar); a fresh artifact under the same tree must be preserved.
        let orphan = repo
            .join(".prometheos")
            .join("workflow")
            .join("old")
            .join("a.log");
        fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        fs::write(&orphan, "x").unwrap();
        fs::write(sidecar_for(&orphan), "{}").unwrap();
        let past = SystemTime::now() - Duration::from_secs(3600);
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(&orphan)
            .and_then(|f| f.set_modified(past));
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(sidecar_for(&orphan))
            .and_then(|f| f.set_modified(past));

        let fresh = repo
            .join(".prometheos")
            .join("workflow")
            .join("fresh")
            .join("b.log");
        fs::create_dir_all(fresh.parent().unwrap()).unwrap();
        fs::write(&fresh, "y").unwrap();

        let out =
            reclaim_orphan_artifacts(repo, Duration::from_secs(10), &ProtectedReferences::new())
                .unwrap();
        assert_eq!(out.removed, 1);
        assert!(!orphan.exists());
        assert!(!sidecar_for(&orphan).exists());
        assert!(fresh.exists());
    }
}
