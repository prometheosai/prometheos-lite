//! Approval-controlled patch workflow.
//!
//! Implements the safe path from repository understanding to a verified patch:
//!
//! ```text
//! review -> propose -> isolated dry-run -> explicit approval -> checkpoint -> apply -> validate -> report
//! ```
//!
//! This module is intentionally self-contained. It drives Git directly (worktrees for
//! dry-run, `git apply` for application, checkpoint branches + reverse-apply for rollback)
//! and enforces a [`ScopeContract`] before any write to the user's tree.
//!
//! No model or provider is required here: the proposed patch is supplied by the caller
//! (or generated upstream by a `PatchProvider`). The safety value is the gating, not the
//! generation.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityLevel {
    /// Read and report only.
    Review,
    /// Generate plan + patch artifact. No application.
    Propose,
    /// Dry-run and apply only after explicit approval.
    Assist,
    /// Bounded execution under approved policy.
    Execute,
}

impl AuthorityLevel {
    /// Only `Assist`/`Execute` may apply patches to the user's tree.
    pub fn can_apply(self) -> bool {
        matches!(self, AuthorityLevel::Assist | AuthorityLevel::Execute)
    }

    /// `Propose`/`Assist`/`Execute` may run an isolated dry-run. `Review` may not.
    pub fn can_dry_run(self) -> bool {
        matches!(
            self,
            AuthorityLevel::Propose | AuthorityLevel::Assist | AuthorityLevel::Execute
        )
    }
}

impl FromStr for AuthorityLevel {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "review" => Ok(AuthorityLevel::Review),
            "propose" => Ok(AuthorityLevel::Propose),
            "assist" => Ok(AuthorityLevel::Assist),
            "execute" => Ok(AuthorityLevel::Execute),
            other => {
                bail!("unknown authority level: {other} (expected review|propose|assist|execute)")
            }
        }
    }
}

impl std::fmt::Display for AuthorityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AuthorityLevel::Review => "review",
            AuthorityLevel::Propose => "propose",
            AuthorityLevel::Assist => "assist",
            AuthorityLevel::Execute => "execute",
        };
        f.write_str(s)
    }
}

/// Locked scope for a workflow run. Immutable once proposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeContract {
    pub goal: String,
    pub authority: AuthorityLevel,
    /// Allowed path prefixes (repo-relative). Empty means "any path under the repo".
    pub allowed_paths: Vec<String>,
    /// Forbidden path prefixes. Always block.
    pub forbidden_paths: Vec<String>,
    /// Whether dependency-manifest changes (Cargo.toml, package.json, ...) are permitted.
    pub allow_dependency_changes: bool,
    /// Optional maximum number of changed files.
    pub max_files_changed: Option<usize>,
    /// Optional maximum total changed lines.
    pub max_lines_changed: Option<usize>,
}

/// Explicit approval record. The patch hash must match the approved proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approver: String,
    pub approved_at: String,
    pub patch_hash: String,
}

/// Persisted proposal artifact. Lives at `<repo>/.prometheos/workflow/<id>/proposal.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalArtifact {
    pub id: String,
    pub repo: String,
    pub base_sha: String,
    pub goal: String,
    pub authority: AuthorityLevel,
    pub scope: ScopeContract,
    /// Full unified-diff text.
    pub patch: String,
    pub patch_hash: String,
    pub changed_files: Vec<String>,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub approved: Option<ApprovalRecord>,
    pub dry_run_passed: Option<bool>,
    pub applied: Option<bool>,
}

/// Run `git` in `repo` and return stdout, failing on non-zero exit.
fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .context("failed to execute git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim())
    }
}

fn hash_str(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Dependency-manifest filenames whose change requires explicit scope permission.
fn is_dependency_file(path: &str) -> bool {
    matches!(
        path.trim_start_matches("./").rsplit('/').next(),
        Some("Cargo.toml")
            | Some("Cargo.lock")
            | Some("package.json")
            | Some("package-lock.json")
            | Some("yarn.lock")
            | Some("pnpm-lock.yaml")
            | Some("pom.xml")
            | Some("build.gradle")
            | Some("requirements.txt")
            | Some("poetry.lock")
            | Some("go.mod")
            | Some("go.sum")
    )
}

/// Parse changed file paths and added/removed line counts from a unified diff.
fn analyze_diff(patch: &str) -> (Vec<String>, usize, usize) {
    let mut files = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let f = rest.trim().to_string();
            if !f.is_empty() && f != "/dev/null" && !files.contains(&f) {
                files.push(f.clone());
            }
        } else if let Some(rest) = line.strip_prefix("--- a/") {
            let f = rest.trim().to_string();
            if !f.is_empty() && f != "/dev/null" && !files.contains(&f) {
                files.push(f.clone());
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    (files, added, removed)
}

fn path_matches_filter(path: &str, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let clean = path.trim_start_matches("./");
    // Normalize glob-ish filters: "src/**" or "src/*" collapse to the "src" base.
    let base = filter
        .trim_end_matches("/**")
        .trim_end_matches("**")
        .trim_end_matches('*')
        .trim_end_matches('/');
    clean == base || clean.starts_with(&format!("{base}/"))
}

fn scope_violations(scope: &ScopeContract, files: &[String]) -> Vec<String> {
    let mut violations = Vec::new();
    for f in files {
        for forbidden in &scope.forbidden_paths {
            if path_matches_filter(f, forbidden) {
                violations.push(format!("forbidden path changed: {f} (matches {forbidden})"));
            }
        }
        if !scope.allow_dependency_changes && is_dependency_file(f) {
            violations.push(format!("dependency file changed without permission: {f}"));
        }
        if !scope.allowed_paths.is_empty() {
            let allowed = scope
                .allowed_paths
                .iter()
                .any(|a| path_matches_filter(f, a));
            if !allowed {
                violations.push(format!("path outside approved scope: {f}"));
            }
        }
    }
    violations
}

/// Recompute the patch hash and diff metadata from the stored patch and ensure they
/// match the recorded values. This catches accidental corruption and straightforward
/// artifact tampering. It is NOT a cryptographic guarantee against a privileged local
/// attacker, who can rewrite the whole artifact; remote/team approval will eventually
/// need signed or server-held records.
fn verify_proposal_integrity(proposal: &ProposalArtifact) -> Result<(Vec<String>, usize, usize)> {
    let actual_hash = hash_str(&proposal.patch);
    if actual_hash != proposal.patch_hash {
        bail!("proposal integrity failure: patch content does not match stored hash");
    }
    let (files, added, removed) = analyze_diff(&proposal.patch);
    if files != proposal.changed_files
        || added != proposal.added_lines
        || removed != proposal.removed_lines
    {
        bail!("proposal integrity failure: patch metadata does not match patch content");
    }
    Ok((files, added, removed))
}

/// Reject patch forms the narrow parser does not fully model (binary, renames, mode-only).
/// New/deleted text files (handled via `/dev/null` headers) are allowed.
fn reject_unsupported_patch(patch: &str) -> Result<()> {
    let markers = [
        "GIT binary patch",
        "Binary files",
        "rename from",
        "rename to",
        "similarity index",
        "dissimilarity index",
        "old mode",
        "new mode",
    ];
    for m in markers {
        if patch.contains(m) {
            bail!(
                "unsupported patch form rejected (contains '{m}'); only unified text diffs are supported"
            );
        }
    }
    Ok(())
}

/// True if a local branch reference exists. `run_git` treats a nonzero exit as an error,
/// so this is the non-panicking way to test existence.
fn git_ref_exists(repo: &Path, branch: &str) -> bool {
    run_git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

fn workflow_dir(repo: &Path, id: &str) -> PathBuf {
    repo.join(".prometheos").join("workflow").join(id)
}

fn load_proposal(repo: &Path, id: &str) -> Result<ProposalArtifact> {
    let path = workflow_dir(repo, id).join("proposal.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read proposal {id} at {}", path.display()))?;
    serde_json::from_str(&text).context("failed to parse proposal artifact")
}

fn save_proposal(repo: &Path, proposal: &ProposalArtifact) -> Result<()> {
    let dir = workflow_dir(repo, &proposal.id);
    std::fs::create_dir_all(&dir).context("failed to create workflow dir")?;
    let text = serde_json::to_string_pretty(proposal).context("failed to serialize proposal")?;
    std::fs::write(dir.join("proposal.json"), text).context("failed to write proposal")?;
    Ok(())
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC3339-ish; precision to seconds is enough for an approval record.
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| secs.to_string())
}

/// Propose a patch. Validates scope immediately and persists a proposal artifact.
/// Returns the workflow id.
pub fn propose(
    repo: &Path,
    goal: &str,
    authority: AuthorityLevel,
    patch: &str,
    allowed_paths: &[String],
    forbidden_paths: &[String],
    allow_dependency_changes: bool,
    max_files_changed: Option<usize>,
    max_lines_changed: Option<usize>,
) -> Result<String> {
    if !is_git_repo(repo) {
        bail!("not a git repository: {}", repo.display());
    }
    reject_unsupported_patch(patch)?;
    let base_sha = run_git(repo, &["rev-parse", "HEAD"])?.trim().to_string();
    let patch_hash = hash_str(patch);
    let (changed_files, added, removed) = analyze_diff(patch);

    let scope = ScopeContract {
        goal: goal.to_string(),
        authority,
        allowed_paths: allowed_paths.to_vec(),
        forbidden_paths: forbidden_paths.to_vec(),
        allow_dependency_changes,
        max_files_changed,
        max_lines_changed,
    };

    let mut violations = scope_violations(&scope, &changed_files);
    if let Some(max) = max_files_changed
        && changed_files.len() > max
    {
        violations.push(format!(
            "changed {} files exceeds budget of {max}",
            changed_files.len()
        ));
    }
    if let Some(max) = max_lines_changed
        && added + removed > max
    {
        violations.push(format!(
            "changed {} lines exceeds budget of {max}",
            added + removed
        ));
    }
    if !violations.is_empty() {
        bail!("scope check failed:\n- {}", violations.join("\n- "));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let proposal = ProposalArtifact {
        id: id.clone(),
        repo: repo.display().to_string(),
        base_sha,
        goal: goal.to_string(),
        authority,
        scope,
        patch: patch.to_string(),
        patch_hash,
        changed_files,
        added_lines: added,
        removed_lines: removed,
        approved: None,
        dry_run_passed: None,
        applied: None,
    };
    save_proposal(repo, &proposal)?;
    Ok(id)
}

/// Run an isolated dry-run in a detached Git worktree: validate with `git apply --check`,
/// apply, optionally run a validation command, then remove the worktree.
pub fn dry_run(repo: &Path, id: &str, validation: Option<&str>) -> Result<bool> {
    let mut proposal = load_proposal(repo, id)?;
    if !proposal.authority.can_dry_run() {
        bail!("authority '{}' cannot run a dry-run", proposal.authority);
    }
    let wt_root = std::env::temp_dir().join(format!("prometheos-dry-run-{id}"));
    // Clean any stale state for this path, then prune orphaned worktree registrations.
    let _ = run_git(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = run_git(repo, &["worktree", "prune"]);

    let patch_file = std::env::temp_dir().join(format!("prometheos-patch-{id}.patch"));
    std::fs::write(&patch_file, &proposal.patch).context("failed to write patch file")?;

    // Create a detached worktree at base sha. `git worktree add` creates the dir itself,
    // so we must not pre-create it.
    run_git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            wt_root.to_str().unwrap(),
            &proposal.base_sha,
        ],
    )
    .context("failed to create dry-run worktree")?;

    let result: Result<bool> = (|| {
        run_git(
            &wt_root,
            &["apply", "--check", patch_file.to_str().unwrap()],
        )
        .context("patch does not apply cleanly (--check failed)")?;
        run_git(&wt_root, &["apply", patch_file.to_str().unwrap()])
            .context("patch application failed in dry-run")?;

        if let Some(cmd) = validation {
            let status = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(&wt_root)
                .status()
                .context("failed to run validation command")?;
            if !status.success() {
                bail!("validation command failed in dry-run");
            }
        }
        Ok(true)
    })();

    let _ = run_git(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    match result {
        Ok(_) => {
            proposal.dry_run_passed = Some(true);
            save_proposal(repo, &proposal)?;
            Ok(true)
        }
        Err(e) => {
            proposal.dry_run_passed = Some(false);
            save_proposal(repo, &proposal)?;
            Err(e)
        }
    }
}

/// Record explicit approval. The supplied patch hash must match the proposal, the
/// proposal must pass an isolated dry-run first, and artifact integrity is verified.
pub fn approve(repo: &Path, id: &str, patch_hash: &str, approver: &str) -> Result<()> {
    let mut proposal = load_proposal(repo, id)?;
    verify_proposal_integrity(&proposal)?;
    if proposal.dry_run_passed != Some(true) {
        bail!("approval blocked: proposal has not passed an isolated dry-run");
    }
    if proposal.patch_hash != patch_hash {
        bail!(
            "approval patch hash mismatch: approved={patch_hash} proposal={}",
            proposal.patch_hash
        );
    }
    proposal.approved = Some(ApprovalRecord {
        approver: approver.to_string(),
        approved_at: now_iso(),
        patch_hash: patch_hash.to_string(),
    });
    save_proposal(repo, &proposal)?;
    Ok(())
}

/// Apply the approved patch to the user's tree.
///
/// Enforces, in order: artifact integrity, a successful isolated dry-run, single-use,
/// current HEAD == proposal `base_sha`, a non-existent checkpoint branch, scope against
/// the *actual* patch, and a clean working tree (ignoring this workflow's own
/// `.prometheos/` metadata). On validation failure it rolls back by reverse-applying and
/// reports honestly; the checkpoint branch is preserved as recovery evidence.
///
/// Validation commands run through `sh -c` with the CLI process's OS permissions. They
/// are NOT sandboxed (no process/network/secrets isolation). Remote or model-generated
/// validation commands must never be executed automatically; future policy needs
/// allowlisted command templates.
pub fn apply(
    repo: &Path,
    id: &str,
    patch_hash: &str,
    validation: Option<&str>,
    rollback_on_failure: bool,
) -> Result<()> {
    let mut proposal = load_proposal(repo, id)?;

    let (actual_files, _, _) = verify_proposal_integrity(&proposal)?;

    if proposal.dry_run_passed != Some(true) {
        bail!("apply blocked: proposal has not passed an isolated dry-run");
    }

    if proposal.applied == Some(true) {
        bail!("apply blocked: this proposal has already been applied");
    }

    if !proposal.authority.can_apply() {
        bail!(
            "authority '{}' cannot apply patches; require assist/execute",
            proposal.authority
        );
    }
    let approval = proposal
        .approved
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("apply blocked: no approval recorded"))?;
    if approval.patch_hash != patch_hash || proposal.patch_hash != patch_hash {
        bail!("apply blocked: approval/patch hash mismatch");
    }

    // Require the repository to be at the same commit the proposal was validated against.
    let current_head = run_git(repo, &["rev-parse", "HEAD"])?.trim().to_string();
    if current_head != proposal.base_sha {
        bail!(
            "apply blocked: repository HEAD changed since proposal; expected {}, found {}. \
             Create a new proposal or re-run against the current base.",
            proposal.base_sha,
            current_head
        );
    }

    // Scope is enforced against the actual patch, not the stored file list.
    let violations = scope_violations(&proposal.scope, &actual_files);
    if !violations.is_empty() {
        bail!(
            "apply blocked: scope violation\n- {}",
            violations.join("\n- ")
        );
    }

    // Refuse to touch a dirty working tree (do not silently stash user work).
    // The workflow's own `.prometheos/` metadata directory is ignored.
    let status = run_git(repo, &["status", "--porcelain"])?;
    let has_user_changes = status.lines().any(|line| {
        let path = line.get(3..).unwrap_or("").trim();
        !path.starts_with(".prometheos/")
    });
    if has_user_changes {
        bail!("apply blocked: repository has uncommitted changes; stash or commit first");
    }

    // Preserve a checkpoint branch at the pre-apply HEAD as recovery evidence.
    let checkpoint_branch = format!("prometheos/checkpoint-{id}");
    if git_ref_exists(repo, &checkpoint_branch) {
        bail!("apply blocked: checkpoint branch already exists: {checkpoint_branch}");
    }
    run_git(repo, &["branch", &checkpoint_branch, &current_head])
        .context("failed to create checkpoint branch")?;

    let patch_file = std::env::temp_dir().join(format!("prometheos-apply-{id}.patch"));
    std::fs::write(&patch_file, &proposal.patch).context("failed to write patch file")?;

    let apply_result: Result<()> = (|| {
        run_git(repo, &["apply", patch_file.to_str().unwrap()])
            .context("patch application failed")?;
        if let Some(cmd) = validation {
            let status = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(repo)
                .status()
                .context("failed to run validation command")?;
            if !status.success() {
                bail!("validation command failed after apply");
            }
        }
        Ok(())
    })();

    if let Err(apply_error) = apply_result {
        if rollback_on_failure {
            let rollback_result = run_git(repo, &["apply", "-R", patch_file.to_str().unwrap()]);
            proposal.applied = Some(false);
            save_proposal(repo, &proposal)?;
            match rollback_result {
                Ok(_) => {
                    return Err(
                        apply_error.context("apply failed; working tree successfully rolled back")
                    );
                }
                Err(rollback_error) => {
                    return Err(anyhow::anyhow!(
                        "apply failed and rollback also failed.\n\
                         Apply error: {apply_error:#}\n\
                         Rollback error: {rollback_error:#}\n\
                         Checkpoint preserved at {checkpoint_branch}"
                    ));
                }
            }
        }
        proposal.applied = Some(false);
        save_proposal(repo, &proposal)?;
        return Err(
            apply_error.context("apply failed; working tree left modified (rollback disabled)")
        );
    }

    // Checkpoint branch is intentionally preserved as recovery evidence.
    proposal.applied = Some(true);
    save_proposal(repo, &proposal)?;
    Ok(())
}

/// Print a JSON report of the proposal.
pub fn report(repo: &Path, id: &str) -> Result<String> {
    let proposal = load_proposal(repo, id)?;
    serde_json::to_string_pretty(&proposal).context("failed to serialize report")
}

/// True if `repo` is a Git repository.
pub fn is_git_repo(repo: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Build a temp git repo with one committed file (a boundary bug to fix).
    fn temp_repo() -> (TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("src/calc.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
        )
        .unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let base = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (dir, repo, base)
    }

    fn patch_for(file: &str, old: &str, new: &str) -> String {
        format!("--- a/{file}\n+++ b/{file}\n@@ -1 +1 @@\n-{old}\n+{new}\n")
    }

    fn good_patch() -> String {
        patch_for(
            "src/calc.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a - b }",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
    }

    fn phash(patch: &str) -> String {
        hash_str(patch)
    }

    fn propose_ok(repo: &Path, authority: AuthorityLevel) -> String {
        propose(
            repo,
            "fix",
            authority,
            &good_patch(),
            &["src/**".to_string()],
            &[],
            false,
            None,
            None,
        )
        .unwrap()
    }

    // --- existing structural tests ---

    #[test]
    fn parses_diff_metadata() {
        let patch = "\
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = 1;
+    let x = 2;
     println!(\"{x}\");
 }
";
        let (files, added, removed) = analyze_diff(patch);
        assert_eq!(files, vec!["src/foo.rs".to_string()]);
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }

    #[test]
    fn detects_forbidden_and_dependency_paths() {
        let scope = ScopeContract {
            goal: "g".into(),
            authority: AuthorityLevel::Assist,
            allowed_paths: vec![],
            forbidden_paths: vec!["secrets/".into()],
            allow_dependency_changes: false,
            max_files_changed: None,
            max_lines_changed: None,
        };
        let bad = scope_violations(&scope, &["secrets/key".into(), "Cargo.toml".into()]);
        assert_eq!(bad.len(), 2);
    }

    #[test]
    fn allows_open_scope() {
        let scope = ScopeContract {
            goal: "g".into(),
            authority: AuthorityLevel::Assist,
            allowed_paths: vec![],
            forbidden_paths: vec![],
            allow_dependency_changes: true,
            max_files_changed: None,
            max_lines_changed: None,
        };
        assert!(scope_violations(&scope, &["src/foo.rs".into()]).is_empty());
    }

    // --- gate tests ---

    #[test]
    fn apply_rejects_without_approval() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn approve_rejects_without_dry_run() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        // No dry-run ran, so approval must be refused.
        assert!(approve(&repo, &id, &phash(&good_patch()), "op").is_err());
    }

    #[test]
    fn apply_rejects_without_dry_run() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        // The public API cannot reach "approved but no passing dry-run", so craft the
        // persisted artifact to exercise the apply-time dry-run gate directly.
        let mut proposal = load_proposal(&repo, &id).unwrap();
        proposal.approved = Some(ApprovalRecord {
            approver: "op".into(),
            approved_at: now_iso(),
            patch_hash: phash(&good_patch()),
        });
        proposal.dry_run_passed = None;
        save_proposal(&repo, &proposal).unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn apply_rejects_after_failed_dry_run() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        // Validation fails during dry-run -> dry_run_passed == false.
        assert!(dry_run(&repo, &id, Some("grep -q 'NOPE' src/calc.rs")).is_err());
        assert!(approve(&repo, &id, &phash(&good_patch()), "op").is_err());
    }

    #[test]
    fn apply_rejects_after_head_changed() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Move HEAD away from the validated base.
        std::fs::write(repo.join("unrelated.rs"), "fn u() {}\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "move head"]);
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn approval_wrong_hash_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        assert!(approve(&repo, &id, "deadbeef", "op").is_err());
    }

    #[test]
    fn stored_patch_tamper_after_approval_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Edit the stored patch but leave the recorded hash unchanged -> integrity fails.
        let path = repo
            .join(".prometheos")
            .join("workflow")
            .join(&id)
            .join("proposal.json");
        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc["patch"] = serde_json::Value::String(patch_for(
            "src/calc.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a - b }",
            "pub fn add(a: i32, b: i32) -> i32 { a * b }",
        ));
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn stored_metadata_tamper_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        let path = repo
            .join(".prometheos")
            .join("workflow")
            .join(&id)
            .join("proposal.json");
        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc["changed_files"] = serde_json::Value::Array(vec![]);
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn path_outside_allowed_scope_rejected() {
        let (_d, repo, _base) = temp_repo();
        let patch = patch_for("other/x.rs", "old", "new");
        let res = propose(
            &repo,
            "fix",
            AuthorityLevel::Assist,
            &patch,
            &["src/**".to_string()],
            &[],
            false,
            None,
            None,
        );
        assert!(res.is_err());
    }

    #[test]
    fn forbidden_overrides_allowed_parent() {
        let (_d, repo, _base) = temp_repo();
        let patch = patch_for("src/secrets/k.rs", "old", "new");
        let res = propose(
            &repo,
            "fix",
            AuthorityLevel::Assist,
            &patch,
            &["src/**".to_string()],
            &["src/secrets/".to_string()],
            false,
            None,
            None,
        );
        assert!(res.is_err());
    }

    #[test]
    fn dependency_rejected_unless_allowed() {
        let (_d, repo, _base) = temp_repo();
        let patch = patch_for("Cargo.toml", "version = \"0.1\"", "version = \"0.2\"");
        assert!(
            propose(
                &repo,
                "fix",
                AuthorityLevel::Assist,
                &patch,
                &[],
                &[],
                false,
                None,
                None
            )
            .is_err()
        );
        // Allowed: proposal succeeds.
        assert!(
            propose(
                &repo,
                "fix",
                AuthorityLevel::Assist,
                &patch,
                &[],
                &[],
                true,
                None,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn file_budget_enforced() {
        let (_d, repo, _base) = temp_repo();
        // Zero files allowed but the patch touches one.
        assert!(
            propose(
                &repo,
                "fix",
                AuthorityLevel::Assist,
                &good_patch(),
                &["src/**".to_string()],
                &[],
                false,
                Some(0),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn line_budget_enforced() {
        let (_d, repo, _base) = temp_repo();
        assert!(
            propose(
                &repo,
                "fix",
                AuthorityLevel::Assist,
                &good_patch(),
                &["src/**".to_string()],
                &[],
                false,
                None,
                Some(0),
            )
            .is_err()
        );
    }

    #[test]
    fn dirty_tree_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Untracked user change (not under .prometheos).
        std::fs::write(repo.join("scratch.rs"), "fn s() {}\n").unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn prometheos_metadata_not_dirty() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Only .prometheos/ is untracked; apply must not treat it as dirty.
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_ok());
    }

    #[test]
    fn validation_failure_rolls_back_and_preserves_checkpoint() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Apply a different patch whose validation (grep 'a + b') fails.
        let bad_patch = patch_for(
            "src/calc.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a - b }",
            "pub fn add(a: i32, b: i32) -> i32 { a * b }",
        );
        let bad_id = propose(
            &repo,
            "change",
            AuthorityLevel::Assist,
            &bad_patch,
            &["src/**".to_string()],
            &[],
            false,
            None,
            None,
        )
        .unwrap();
        // Dry-run validation passes (the patch produces `a * b`); apply validation fails.
        dry_run(&repo, &bad_id, Some("grep -q 'a \\* b' src/calc.rs")).unwrap();
        approve(&repo, &bad_id, &phash(&bad_patch), "op").unwrap();
        assert!(
            apply(
                &repo,
                &bad_id,
                &phash(&bad_patch),
                Some("grep -q 'a + b' src/calc.rs"),
                true
            )
            .is_err()
        );
        // Tree reverted to original buggy form.
        let content = std::fs::read_to_string(repo.join("src/calc.rs")).unwrap();
        assert!(
            content.contains("a - b"),
            "tree was not rolled back: {content}"
        );
        // Checkpoint branch preserved as recovery evidence.
        assert!(git_ref_exists(
            &repo,
            &format!("prometheos/checkpoint-{bad_id}")
        ));
    }

    #[test]
    fn rollback_failure_reported_and_checkpoint_preserved() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Validation command fails (to trigger rollback) but mutates the changed line
        // so that reverse-apply can no longer match, forcing a rollback failure.
        let result = apply(
            &repo,
            &id,
            &phash(&good_patch()),
            Some("sed -i 's/a + b/a + b EXTRA/' src/calc.rs; false"),
            true,
        );
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("rollback also failed"),
            "expected honest rollback-failure report, got: {msg}"
        );
        // Checkpoint must remain when rollback fails (do not delete recovery data).
        assert!(git_ref_exists(
            &repo,
            &format!("prometheos/checkpoint-{id}")
        ));
    }

    #[test]
    fn reapply_already_applied_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_ok());
        // Second application of the same proposal is refused.
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
        // And the checkpoint branch remains.
        assert!(git_ref_exists(
            &repo,
            &format!("prometheos/checkpoint-{id}")
        ));
    }

    #[test]
    fn propose_authority_cannot_apply() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Propose);
        dry_run(&repo, &id, Some("grep -q 'a + b' src/calc.rs")).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Propose authority cannot apply.
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn unsupported_patch_rejected() {
        let (_d, repo, _base) = temp_repo();
        let binary = "--- a/img.png\n+++ b/img.png\nGIT binary patch\nliteral 0\nH4sIAAAAAAAA\n";
        assert!(
            propose(
                &repo,
                "fix",
                AuthorityLevel::Assist,
                binary,
                &["src/**".to_string()],
                &[],
                false,
                None,
                None,
            )
            .is_err()
        );
    }
}
