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

use crate::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use crate::harness::patch_provider::{GenerateRequest, PatchProvider, PatchProviderContext};

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
    /// Optional validation command recorded at generation time; used when a later
    /// `dry-run`/`apply` does not supply its own.
    #[serde(default)]
    pub validation_command: Option<String>,
    /// Provider provenance for proposals generated through a `PatchProvider`.
    #[serde(default)]
    pub provider_provenance: Option<ProviderProvenance>,
    /// Validation command actually used during the isolated dry-run (if any).
    #[serde(default)]
    pub dry_run_validation: Option<String>,
    /// Validation command actually used during apply (if any).
    #[serde(default)]
    pub apply_validation: Option<String>,
    /// Checkpoint branch created before apply (recovery evidence).
    #[serde(default)]
    pub checkpoint_ref: Option<String>,
    /// Rollback outcome after a failed apply validation: "clean", "rolled_back",
    /// or "rollback_failed".
    #[serde(default)]
    pub rollback_status: Option<String>,
}

/// Provider provenance recorded with a governed proposal.
///
/// This answers *which* provider produced the patch, under *what* route/model, and
/// binds the patch to its inputs and scope. It is deliberately free of any secret:
/// no API key, authorization header, or token is ever persisted here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProvenance {
    /// Provider implementation name (e.g. "mock", "llm", "deterministic").
    pub implementation: String,
    /// Configured model name, when known (not a secret).
    pub model: Option<String>,
    /// Configured route/endpoint (e.g. base URL), when known (not a secret).
    pub route: Option<String>,
    /// Generation timestamp (RFC3339).
    pub generated_at: String,
    /// Work/trace identifier when available.
    pub work_id: Option<String>,
    /// Digest of the prompt/input (goal + requirements), not raw configuration.
    pub input_digest: String,
    /// Internally computed patch hash (mirrors `ProposalArtifact.patch_hash`).
    pub patch_hash: String,
    /// Base commit SHA the proposal was validated against.
    pub base_sha: String,
    /// Digest of the scope contract the proposal was validated against.
    pub scope_digest: String,
}

/// Route/identity metadata used only to populate non-secret `ProviderProvenance`.
#[derive(Debug, Clone, Default)]
pub struct ProviderRouteInfo {
    /// Configured model name (not a secret).
    pub model: Option<String>,
    /// Configured route/endpoint (e.g. base URL), not a secret.
    pub route: Option<String>,
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
///
/// This is the stable entry point used by the `workflow propose` CLI command and by
/// tests. Provider-generated proposals should call [`generate_proposal`] (which routes
/// through this same gating path) so that no parallel proposal path is introduced.
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
    propose_with_meta(
        repo,
        goal,
        authority,
        patch,
        allowed_paths,
        forbidden_paths,
        allow_dependency_changes,
        max_files_changed,
        max_lines_changed,
        None,
        None,
    )
}

/// Internal proposal constructor that accepts optional validation command and
/// provider provenance. All gating lives here so the public `propose` and the
/// provider-backed `generate_proposal` share identical behavior.
fn propose_with_meta(
    repo: &Path,
    goal: &str,
    authority: AuthorityLevel,
    patch: &str,
    allowed_paths: &[String],
    forbidden_paths: &[String],
    allow_dependency_changes: bool,
    max_files_changed: Option<usize>,
    max_lines_changed: Option<usize>,
    validation_command: Option<String>,
    provider_provenance: Option<ProviderProvenance>,
) -> Result<String> {
    if !is_git_repo(repo) {
        bail!("not a git repository: {}", repo.display());
    }
    reject_unsupported_patch(patch)?;
    require_unified_diff(patch)?;
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
        validation_command,
        provider_provenance,
        dry_run_validation: None,
        apply_validation: None,
        checkpoint_ref: None,
        rollback_status: None,
    };
    save_proposal(repo, &proposal)?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Provider-backed proposal generation (#78)
//
// A `PatchProvider` generates candidate edits. Those edits are rendered into a
// unified diff and treated as *hostile* input (absolute/traversal rejection,
// scope, unsupported-form, budget). The patch is then routed through
// `propose_with_meta`, so every #77 gate (integrity, dry-run, approval-hash,
// base-SHA, checkpoint, rollback) applies unchanged. No parallel proposal path
// and no model invocation lives here.
// ---------------------------------------------------------------------------

/// Scope contract for a provider-backed generation request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateScope {
    pub allowed_paths: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allow_dependency_changes: bool,
    pub max_files_changed: Option<usize>,
    pub max_lines_changed: Option<usize>,
}

/// Result of a provider-backed generation request.
#[derive(Debug, Clone)]
pub struct GenerateResult {
    pub id: String,
    pub patch: String,
    pub patch_hash: String,
}

/// Run `git` and return stdout, tolerating exit code 1 (which `git diff
/// --no-index` uses to signal "files differ"). Other non-zero exits fail.
fn run_git_capture(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("failed to execute git")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() || output.status.code() == Some(1) {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim())
    }
}

/// True if `path` is absolute, a Windows drive path, a UNC path, or contains
/// `..` traversal. Detected on *every* platform so a Linux CI runner still
/// rejects Windows-shaped hostile paths (`C:\...`, `\\server\share`).
fn is_hostile_path(path: &str) -> bool {
    if path.is_empty() || path == "/dev/null" {
        return false;
    }
    // Absolute on the current platform (handles Windows drive + UNC on Windows,
    // and `/...` on Unix).
    if Path::new(path).is_absolute() {
        return true;
    }
    // Unix absolute.
    if path.starts_with('/') {
        return true;
    }
    // Windows drive letter on any platform, e.g. `C:\foo` or `C:foo`.
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }
    // UNC / double-slash escapes, e.g. `\\server\share` or `//server/share`.
    if path.starts_with("\\\\") || path.starts_with("//") {
        return true;
    }
    // Parent-directory traversal.
    if path.split('/').any(|c| c == "..") {
        return true;
    }
    false
}

/// Reject any patch path that escapes the repository: absolute paths (unix
/// `/...`, Windows drive/UNC) or `..` traversal. `/dev/null` is always allowed.
fn validate_patch_paths(repo: &Path, patch: &str) -> Result<()> {
    for line in patch.lines() {
        let path = match line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"))
        {
            Some(rest) => rest.trim(),
            None => continue,
        };
        if is_hostile_path(path) {
            bail!("rejected patch path is absolute or escapes the repo: {path}");
        }
        if path != "/dev/null" && !repo.join(path).starts_with(repo) {
            bail!("rejected patch path escapes the repo: {path}");
        }
    }
    Ok(())
}

/// Normalize a provider-supplied file path to a repo-relative path, rejecting
/// anything absolute, a Windows drive/UNC path, or traversing outside the
/// repository. First hostile-input gate applied to provider output.
fn sanitize_repo_relative(file: &Path) -> Result<PathBuf> {
    if is_hostile_path(&file.to_string_lossy()) {
        bail!(
            "rejected provider file path is absolute or escapes the repo: {}",
            file.display()
        );
    }
    if file.as_os_str().is_empty() {
        bail!("rejected empty provider file path");
    }
    Ok(file.to_path_buf())
}

/// Reject a patch that is not a real unified diff (plain text or otherwise
/// malformed) *before* any proposal artifact is created.
fn require_unified_diff(patch: &str) -> Result<()> {
    let has_hunk = patch.contains("@@");
    let has_header = patch.contains("--- ") || patch.contains("+++ ");
    if !(has_hunk && has_header) {
        bail!("rejected patch is not a unified diff (missing hunk/header markers)");
    }
    Ok(())
}

/// Sanitize a provider route/endpoint for provenance: keep only
/// `scheme://host[:port]`, stripping any userinfo (the secret-bearing part),
/// path, query, and fragment. Returns `None` if the URL has no scheme/host.
pub fn sanitize_provider_route(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    // Drop any userinfo (e.g. `sk-abc@`) before the host.
    let after_userinfo = match rest.rfind('@') {
        Some(idx) => &rest[idx + 1..],
        None => rest,
    };
    // Keep only authority; drop /path, ?query, #fragment.
    let authority = after_userinfo
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_userinfo);
    if authority.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme, authority))
}

/// Structured, content-free record of an edit-rendering rejection.
///
/// Persisted under `.prometheos/diagnostics/` so render-time rejections become
/// observability evidence instead of being lost to stderr. No raw model response
/// is required: the parsed operation and the render failure are sufficient. This
/// intentionally does not reinterpret the model's intent (e.g. a `search_replace`
/// on a missing file is recorded and rejected, never silently rewritten into a
/// `create_file`).
#[derive(Debug, serde::Serialize)]
struct RenderRejectionRecord {
    stage: &'static str,
    outcome: &'static str,
    rejection_reason: &'static str,
    operation: &'static str,
    path: String,
    proposal_generated: bool,
    repository_mutated: bool,
}

/// Result of attempting to read a render target file, classified so that only a
/// genuine absence is reported as "missing". Other read failures (permission,
/// directory, invalid UTF-8) are preserved separately rather than mislabeled.
enum TargetRead {
    Present(String),
    Missing,
    Unreadable(std::io::Error),
}

/// Read a render target file from `repo`, mapping `NotFound` to
/// [`TargetRead::Missing`] and any other I/O error to [`TargetRead::Unreadable`]
/// so callers can classify the rejection reason accurately.
fn read_target_file(repo: &Path, rel: &Path) -> TargetRead {
    match std::fs::read_to_string(repo.join(rel)) {
        Ok(s) => TargetRead::Present(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TargetRead::Missing,
        Err(e) => TargetRead::Unreadable(e),
    }
}

/// Persist a render-time rejection record under `<base>/.prometheos/diagnostics/`.
///
/// `base` is the target repository being rendered (or its workflow context), so
/// diagnostics are scoped to the work they describe rather than the caller's
/// current directory. The file name incorporates the operation, path, and
/// rejection reason, plus a unique id, so distinct or repeated rejections are each
/// captured as separate evidence instead of overwriting one another. All failures
/// are swallowed: diagnostics are best-effort observability, never a reason to fail
/// the surrounding workflow.
fn persist_render_rejection(base: &Path, record: &RenderRejectionRecord) {
    let dir = base.join(".prometheos").join("diagnostics");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stem = hash_str(&format!(
        "render:{}:{}:{}",
        record.operation, record.path, record.rejection_reason
    ));
    let id = format!("{}-{}", stem, uuid::Uuid::new_v4());
    if let Ok(json) = serde_json::to_string_pretty(record) {
        let _ = std::fs::write(dir.join(format!("{}.json", id)), json);
    }
}

/// Render a single provider edit into a unified-diff fragment. The edit's file
/// path is validated as hostile input first.
///
/// Semantic rejections (e.g. `search_replace` against a missing file, `create_file`
/// against an existing file, search text not found/ambiguous) persist a structured
/// [`RenderRejectionRecord`] before returning an error, so the governed path's
/// rejection is captured as evidence rather than lost to stderr.
fn render_single_edit(repo: &Path, edit: &EditOperation) -> Result<String> {
    match edit {
        EditOperation::UnifiedDiff(u) => {
            validate_patch_paths(repo, &u.diff)?;
            Ok(u.diff.clone())
        }
        EditOperation::CreateFile(c) => {
            let rel = sanitize_repo_relative(&c.file)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if repo.join(&rel).exists() {
                persist_render_rejection(
                    repo,
                    &RenderRejectionRecord {
                        stage: "edit_rendering",
                        outcome: "rejected",
                        rejection_reason: "create_target_exists",
                        operation: "create_file",
                        path: rel_str.clone(),
                        proposal_generated: false,
                        repository_mutated: false,
                    },
                );
                bail!("provider targets existing file for creation: {}", rel_str);
            }
            render_two_sides(&rel_str, None, Some(&c.content))
        }
        EditOperation::DeleteFile(d) => {
            let rel = sanitize_repo_relative(&d.file)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            match read_target_file(repo, &rel) {
                TargetRead::Present(old) => render_two_sides(&rel_str, Some(&old), None),
                TargetRead::Missing => {
                    persist_render_rejection(
                        repo,
                        &RenderRejectionRecord {
                            stage: "edit_rendering",
                            outcome: "rejected",
                            rejection_reason: "delete_target_missing",
                            operation: "delete_file",
                            path: rel_str.clone(),
                            proposal_generated: false,
                            repository_mutated: false,
                        },
                    );
                    bail!("provider targets missing file for deletion: {}", rel_str)
                }
                TargetRead::Unreadable(e) => {
                    persist_render_rejection(
                        repo,
                        &RenderRejectionRecord {
                            stage: "edit_rendering",
                            outcome: "rejected",
                            rejection_reason: "delete_target_unreadable",
                            operation: "delete_file",
                            path: rel_str.clone(),
                            proposal_generated: false,
                            repository_mutated: false,
                        },
                    );
                    bail!("provider could not read target {}: {}", rel_str, e)
                }
            }
        }
        EditOperation::WholeFile(w) => {
            let rel = sanitize_repo_relative(&w.file)?;
            let old = std::fs::read_to_string(repo.join(&rel)).unwrap_or_default();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            render_two_sides(&rel_str, Some(&old), Some(&w.content))
        }
        EditOperation::SearchReplace(sr) => {
            let rel = sanitize_repo_relative(&sr.file)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let old = match read_target_file(repo, &rel) {
                TargetRead::Present(o) => o,
                TargetRead::Missing => {
                    persist_render_rejection(
                        repo,
                        &RenderRejectionRecord {
                            stage: "edit_rendering",
                            outcome: "rejected",
                            rejection_reason: "search_replace_target_missing",
                            operation: "search_replace",
                            path: rel_str.clone(),
                            proposal_generated: false,
                            repository_mutated: false,
                        },
                    );
                    bail!("provider targets missing file: {}", rel_str)
                }
                TargetRead::Unreadable(e) => {
                    persist_render_rejection(
                        repo,
                        &RenderRejectionRecord {
                            stage: "edit_rendering",
                            outcome: "rejected",
                            rejection_reason: "search_replace_target_unreadable",
                            operation: "search_replace",
                            path: rel_str.clone(),
                            proposal_generated: false,
                            repository_mutated: false,
                        },
                    );
                    bail!("provider could not read target {}: {}", rel_str, e)
                }
            };
            if sr.replace_all != Some(true) && old.matches(&sr.search).count() > 1 {
                persist_render_rejection(
                    repo,
                    &RenderRejectionRecord {
                        stage: "edit_rendering",
                        outcome: "rejected",
                        rejection_reason: "search_text_ambiguous",
                        operation: "search_replace",
                        path: rel_str.clone(),
                        proposal_generated: false,
                        repository_mutated: false,
                    },
                );
                bail!(
                    "provider search text matched multiple locations: {}",
                    rel_str
                );
            }
            match apply_search_replace(&old, sr) {
                Ok(new) => render_two_sides(&rel_str, Some(&old), Some(&new)),
                Err(_) => {
                    persist_render_rejection(
                        repo,
                        &RenderRejectionRecord {
                            stage: "edit_rendering",
                            outcome: "rejected",
                            rejection_reason: "search_text_not_found",
                            operation: "search_replace",
                            path: rel_str,
                            proposal_generated: false,
                            repository_mutated: false,
                        },
                    );
                    bail!("provider search text not found: {}", ellipsize(&sr.search))
                }
            }
        }
        EditOperation::RenameFile(_) => {
            bail!("rename edits are not supported by the governed proposal path")
        }
    }
}

/// Run `git diff --no-index` between two optional sides (`/dev/null` when `None`)
/// and normalize the header paths to repo-relative `a/<rel>` / `b/<rel>`.
fn render_two_sides(rel_str: &str, old: Option<&str>, new: Option<&str>) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!("prometheos-render-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).context("failed to create temp dir for diff")?;
    let result = (|| -> Result<String> {
        let old_arg = match old {
            Some(text) => {
                let p = tmp.join("o").join(rel_str);
                std::fs::create_dir_all(p.parent().unwrap())
                    .context("failed to create temp old file")?;
                std::fs::write(&p, text).context("failed to write temp old file")?;
                format!("o/{rel_str}")
            }
            None => "/dev/null".to_string(),
        };
        let new_arg = match new {
            Some(text) => {
                let p = tmp.join("n").join(rel_str);
                std::fs::create_dir_all(p.parent().unwrap())
                    .context("failed to create temp new file")?;
                std::fs::write(&p, text).context("failed to write temp new file")?;
                format!("n/{rel_str}")
            }
            None => "/dev/null".to_string(),
        };
        let raw = run_git_capture(
            &tmp,
            &["diff", "--no-index", "--no-color", &old_arg, &new_arg],
        )?;
        Ok(rewrite_diff_header(&raw))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Normalize `git diff --no-index` headers (`a/o/<rel>` / `b/n/<rel>`) to
/// `a/<rel>` / `b/<rel>` and force forward slashes.
fn rewrite_diff_header(raw: &str) -> String {
    raw.replace('\\', "/")
        .replace("a/o/", "a/")
        .replace("b/o/", "b/")
        .replace("a/n/", "a/")
        .replace("b/n/", "b/")
}

/// Apply a provider `SearchReplace` edit to file text. An empty search prepends
/// the replacement; otherwise the first occurrence must match.
fn apply_search_replace(old: &str, sr: &SearchReplaceEdit) -> Result<String> {
    if sr.search.is_empty() {
        if sr.replace.is_empty() {
            return Ok(old.to_string());
        }
        let prefix = if sr.replace.ends_with('\n') {
            sr.replace.clone()
        } else {
            format!("{}\n", sr.replace)
        };
        return Ok(format!("{prefix}{old}"));
    }
    if sr.replace_all == Some(true) {
        if !old.contains(&sr.search) {
            bail!("provider search text not found: {}", ellipsize(&sr.search));
        }
        return Ok(old.replace(&sr.search, &sr.replace));
    }
    match old.split_once(&sr.search) {
        Some((before, after)) => Ok(format!("{before}{}{after}", sr.replace)),
        None => bail!("provider search text not found: {}", ellipsize(&sr.search)),
    }
}

fn ellipsize(s: &str) -> String {
    let t = s.trim();
    if t.len() > 40 {
        format!("{}…", &t[..40])
    } else {
        t.to_string()
    }
}

/// Render provider edits into a single unified diff, treating the result as
/// hostile input (empty-patch rejection handled here).
fn render_edits_to_patch(repo: &Path, edits: &[EditOperation]) -> Result<String> {
    if edits.is_empty() {
        bail!("provider produced no edits");
    }
    let mut out = String::new();
    for edit in edits {
        let frag = render_single_edit(repo, edit)?;
        if frag.trim().is_empty() {
            continue;
        }
        out.push_str(&frag);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        bail!("provider produced an empty patch");
    }
    Ok(out)
}

/// Generate a governed proposal through a `PatchProvider`.
///
/// The provider's candidate edits are rendered into a unified diff and treated
/// as hostile input. The patch is then routed through [`propose_with_meta`], so
/// every #77 gate (integrity, dry-run, approval-hash, base-SHA, checkpoint,
/// rollback) applies unchanged. No model is invoked directly from this module.
pub async fn generate_proposal(
    repo: &Path,
    goal: &str,
    authority: AuthorityLevel,
    provider: &dyn PatchProvider,
    context: PatchProviderContext,
    scope: &GenerateScope,
    route_info: Option<ProviderRouteInfo>,
    validation_command: Option<String>,
) -> Result<GenerateResult> {
    if authority == AuthorityLevel::Review {
        bail!("review authority cannot generate a source-modifying patch");
    }

    let response = provider
        .generate(GenerateRequest {
            context: context.clone(),
            preferred_strategies: vec![],
        })
        .await?;

    let candidate = response
        .candidates
        .into_iter()
        .find(|c| !c.edits.is_empty())
        .ok_or_else(|| anyhow::anyhow!("provider produced no usable candidate"))?;

    // Never trust provider-supplied metadata; derive the patch from the edits.
    let patch = render_edits_to_patch(repo, &candidate.edits)?;

    // Hostile-input gate on the rendered patch (absolute paths, traversal).
    validate_patch_paths(repo, &patch)?;
    // Reject plain text / malformed patches before any artifact is created.
    require_unified_diff(&patch)?;

    let patch_hash = hash_str(&patch);

    // Build non-secret provenance. The patch hash, base SHA and scope digest are
    // all derived internally; no key, token, or header is ever persisted.
    let input_digest = hash_str(&format!(
        "{}\n{}",
        context.task,
        context.requirements.join("\n")
    ));
    let scope_digest = hash_str(&serde_json::to_string(&scope).unwrap_or_default());
    let provenance = ProviderProvenance {
        implementation: provider.name().to_string(),
        model: route_info.as_ref().and_then(|r| r.model.clone()),
        // Persist only a sanitized scheme://host[:port]; never raw credentials.
        route: route_info
            .as_ref()
            .and_then(|r| r.route.as_ref())
            .and_then(|u| sanitize_provider_route(u)),
        generated_at: now_iso(),
        work_id: None,
        input_digest,
        patch_hash: patch_hash.clone(),
        base_sha: run_git(repo, &["rev-parse", "HEAD"])?.trim().to_string(),
        scope_digest,
    };

    let id = propose_with_meta(
        repo,
        goal,
        authority,
        &patch,
        &scope.allowed_paths,
        &scope.forbidden_paths,
        scope.allow_dependency_changes,
        scope.max_files_changed,
        scope.max_lines_changed,
        validation_command,
        Some(provenance),
    )?;

    Ok(GenerateResult {
        id,
        patch,
        patch_hash,
    })
}

/// Platform-aware shell selection for running a user-supplied validation command.
///
/// On Windows the command runs through `cmd /C`; elsewhere through `sh -c`. The
/// command string is always passed as a single shell expression (one argument to
/// the shell flag), never split on whitespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShellSpec {
    program: &'static str,
    flag: &'static str,
}

/// Return the shell the current platform uses to evaluate a validation command.
fn validation_shell_spec() -> ShellSpec {
    #[cfg(windows)]
    {
        ShellSpec {
            program: "cmd",
            flag: "/C",
        }
    }
    #[cfg(not(windows))]
    {
        ShellSpec {
            program: "sh",
            flag: "-c",
        }
    }
}

/// Build a [`Command`] that runs `command` as one platform-appropriate shell
/// expression. The caller sets the working directory and reads status/output.
fn validation_shell(command: &str) -> Command {
    let spec = validation_shell_spec();
    let mut cmd = Command::new(spec.program);
    cmd.arg(spec.flag).arg(command);
    cmd
}

/// Run an isolated dry-run in a detached Git worktree: validate with `git apply --check`,
/// apply, optionally run a validation command, then remove the worktree.
pub fn dry_run(repo: &Path, id: &str, validation: Option<&str>) -> Result<bool> {
    let mut proposal = load_proposal(repo, id)?;
    if !proposal.authority.can_dry_run() {
        bail!("authority '{}' cannot run a dry-run", proposal.authority);
    }
    let validation = validation.or(proposal.validation_command.as_deref());
    proposal.dry_run_validation = validation.map(|s| s.to_string());
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
            let status = validation_shell(cmd)
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
/// Validation commands run through a platform-aware shell (`sh -c` on Unix,
/// `cmd /C` on Windows) with the CLI process's OS permissions. They are NOT
/// sandboxed (no process/network/secrets isolation). Remote or model-generated
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

    let validation = validation.or(proposal.validation_command.as_deref());
    proposal.apply_validation = validation.map(|s| s.to_string());

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
    proposal.checkpoint_ref = Some(checkpoint_branch.clone());

    let patch_file = std::env::temp_dir().join(format!("prometheos-apply-{id}.patch"));
    std::fs::write(&patch_file, &proposal.patch).context("failed to write patch file")?;

    let apply_result: Result<()> = (|| {
        run_git(repo, &["apply", patch_file.to_str().unwrap()])
            .context("patch application failed")?;
        if let Some(cmd) = validation {
            let status = validation_shell(cmd)
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
            proposal.rollback_status = Some("rolled_back".to_string());
            save_proposal(repo, &proposal)?;
            match rollback_result {
                Ok(_) => {
                    return Err(
                        apply_error.context("apply failed; working tree successfully rolled back")
                    );
                }
                Err(rollback_error) => {
                    proposal.rollback_status = Some("rollback_failed".to_string());
                    save_proposal(repo, &proposal)?;
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
        proposal.rollback_status = Some("disabled".to_string());
        save_proposal(repo, &proposal)?;
        return Err(
            apply_error.context("apply failed; working tree left modified (rollback disabled)")
        );
    }

    // Checkpoint branch is intentionally preserved as recovery evidence.
    proposal.applied = Some(true);
    proposal.rollback_status = Some("clean".to_string());
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
            "pub fn add(a: i32, b: i32) -> i32 { a.add(b) }",
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

    // Platform-aware validation commands used by the workflow gating tests. The
    // workflow runs the user-supplied command through a platform shell (`sh -c`
    // on Unix, `cmd /C` on Windows — see #98), so the command itself must be
    // expressed in a way the target shell understands. Paths use the platform
    // separator (findstr on Windows does not accept a forward-slash file path),
    // and the search tokens are space-free to avoid nested-quote parsing.
    #[cfg(windows)]
    const OK_VALIDATION: &str = "findstr /L a.add(b) src\\calc.rs";
    #[cfg(not(windows))]
    const OK_VALIDATION: &str = "grep -qF 'a.add(b)' src/calc.rs";

    #[cfg(windows)]
    const FAIL_VALIDATION: &str = "findstr /L NOPE src\\calc.rs";
    #[cfg(not(windows))]
    const FAIL_VALIDATION: &str = "grep -qF NOPE src/calc.rs";

    #[cfg(windows)]
    const STAR_VALIDATION: &str = "findstr /L a.mul(b) src\\calc.rs";
    #[cfg(not(windows))]
    const STAR_VALIDATION: &str = "grep -qF 'a.mul(b)' src/calc.rs";

    // Corrupt the patched file (so reverse-apply can no longer match) and then
    // exit non-zero, to exercise rollback-failure reporting.
    #[cfg(windows)]
    const CORRUPT_VALIDATION: &str = "echo corrupted > src\\calc.rs & exit /b 1";
    #[cfg(not(windows))]
    const CORRUPT_VALIDATION: &str = "printf 'corrupted\\n' > src/calc.rs; false";

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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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
        assert!(dry_run(&repo, &id, Some(FAIL_VALIDATION)).is_err());
        assert!(approve(&repo, &id, &phash(&good_patch()), "op").is_err());
    }

    #[test]
    fn apply_rejects_after_head_changed() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        assert!(approve(&repo, &id, "deadbeef", "op").is_err());
    }

    #[test]
    fn stored_patch_tamper_after_approval_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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
            "pub fn add(a: i32, b: i32) -> i32 { a.mul(b) }",
        ));
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn stored_metadata_tamper_rejected() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Untracked user change (not under .prometheos).
        std::fs::write(repo.join("scratch.rs"), "fn s() {}\n").unwrap();
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_err());
    }

    #[test]
    fn prometheos_metadata_not_dirty() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Only .prometheos/ is untracked; apply must not treat it as dirty.
        assert!(apply(&repo, &id, &phash(&good_patch()), None, true).is_ok());
    }

    #[test]
    fn validation_failure_rolls_back_and_preserves_checkpoint() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Apply a different patch whose validation (matching the patched line) fails.
        let bad_patch = patch_for(
            "src/calc.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a - b }",
            "pub fn add(a: i32, b: i32) -> i32 { a.mul(b) }",
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
        dry_run(&repo, &bad_id, Some(STAR_VALIDATION)).unwrap();
        approve(&repo, &bad_id, &phash(&bad_patch), "op").unwrap();
        assert!(
            apply(
                &repo,
                &bad_id,
                &phash(&bad_patch),
                Some(OK_VALIDATION),
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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // Validation command fails (to trigger rollback) but mutates the changed line
        // so that reverse-apply can no longer match, forcing a rollback failure.
        let result = apply(
            &repo,
            &id,
            &phash(&good_patch()),
            Some(CORRUPT_VALIDATION),
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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
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

    // --- #98: platform-aware validation shell ---

    #[test]
    fn validation_shell_spec_is_platform_aware() {
        let spec = validation_shell_spec();
        #[cfg(windows)]
        assert_eq!(
            spec,
            ShellSpec {
                program: "cmd",
                flag: "/C"
            }
        );
        #[cfg(not(windows))]
        assert_eq!(
            spec,
            ShellSpec {
                program: "sh",
                flag: "-c"
            }
        );
    }

    #[test]
    fn validation_command_runs_as_single_shell_expression() {
        // If the command were split into multiple argv entries, a Unix `sh -c`
        // would run only the first token and not emit both words. On Windows the
        // same expression runs under `cmd /C`. Both prove it reaches the shell as
        // one expression rather than being tokenized by the caller.
        let out = validation_shell("echo one two")
            .output()
            .expect("platform shell must be available");
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("one two"),
            "validation command not run as one expression; stdout={stdout:?}"
        );
    }

    #[test]
    fn validation_runs_in_worktree_directory() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        // The repository still holds the *unpatched* src/calc.rs ("a - b") during a
        // dry-run; only the detached worktree has the patched line ("a.add(b)"). So a
        // validation that matches the patched line can only succeed if the command
        // ran with the worktree as its working directory.
        assert!(dry_run(&repo, &id, Some(OK_VALIDATION)).is_ok());
    }

    #[test]
    fn apply_validation_runs_in_repository_directory() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        dry_run(&repo, &id, Some(OK_VALIDATION)).unwrap();
        approve(&repo, &id, &phash(&good_patch()), "op").unwrap();
        // The validation writes a sentinel file into its working directory. If that
        // directory is the user's repository (as required), the sentinel appears
        // there after apply. `validation_shell` already supplies the platform
        // shell wrapper, so the raw command below is shell-internal only.
        #[cfg(windows)]
        let sentinel = "echo sentinel > PROMETHEOS_CWD_MARKER";
        #[cfg(not(windows))]
        let sentinel = "echo sentinel > PROMETHEOS_CWD_MARKER";
        apply(&repo, &id, &phash(&good_patch()), Some(sentinel), true).unwrap();
        let marker = std::fs::read_to_string(repo.join("PROMETHEOS_CWD_MARKER")).unwrap();
        assert!(
            marker.contains("sentinel"),
            "apply validation did not run in the repository directory; marker={marker:?}"
        );
    }

    #[test]
    fn validation_failure_is_diagnosable() {
        let (_d, repo, _base) = temp_repo();
        let id = propose_ok(&repo, AuthorityLevel::Assist);
        let err = dry_run(&repo, &id, Some(FAIL_VALIDATION)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("validation command failed"),
            "expected diagnosable validation failure, got: {msg}"
        );
    }
}

#[cfg(test)]
mod render_diagnostics_tests {
    use super::render_single_edit;
    use crate::harness::edit_protocol::{
        CreateFileEdit, DeleteFileEdit, EditOperation, SearchReplaceEdit,
    };
    use std::path::PathBuf;

    fn temp_repo_with_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let path = repo.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
        (dir, repo)
    }

    /// Return the structured JSON diagnostics files written for `repo`, or empty
    /// if no `.prometheos/diagnostics` directory exists yet.
    fn diagnostics_files(repo: &std::path::Path) -> Vec<PathBuf> {
        let dir = repo.join(".prometheos").join("diagnostics");
        match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Assert that exactly one rejection diagnostics file was persisted for `repo`
    /// and that it carries the expected structured fields.
    fn assert_rejection_persisted(
        repo: &std::path::Path,
        reason: &str,
        operation: &str,
        path: &str,
    ) {
        let files = diagnostics_files(repo);
        assert_eq!(
            files.len(),
            1,
            "expected exactly one diagnostics file, found {files:?}"
        );
        let content = std::fs::read_to_string(&files[0]).unwrap();
        assert!(
            content.contains("\"stage\": \"edit_rendering\""),
            "{content}"
        );
        assert!(
            content.contains(&format!("\"rejection_reason\": \"{reason}\"")),
            "{content}"
        );
        assert!(
            content.contains(&format!("\"operation\": \"{operation}\"")),
            "{content}"
        );
        assert!(
            content.contains(&format!("\"path\": \"{path}\"")),
            "{content}"
        );
        assert!(
            content.contains("\"proposal_generated\": false"),
            "{content}"
        );
        assert!(
            content.contains("\"repository_mutated\": false"),
            "{content}"
        );
    }

    #[test]
    fn search_replace_missing_file_persists_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn f() {}\n");
        let edit = EditOperation::SearchReplace(SearchReplaceEdit {
            file: "src/missing.rs".into(),
            search: "fn f".into(),
            replace: "fn g".into(),
            replace_all: None,
            context_lines: None,
        });
        let err = render_single_edit(&repo, &edit).unwrap_err();
        assert!(
            err.to_string().contains("provider targets missing file"),
            "unexpected error: {err}"
        );
        // Diagnostics must be scoped to the target repo, never the checkout.
        assert!(!std::path::Path::new(".prometheos/diagnostics").exists());
        assert_rejection_persisted(
            &repo,
            "search_replace_target_missing",
            "search_replace",
            "src/missing.rs",
        );
    }

    #[test]
    fn create_file_existing_target_persists_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn f() {}\n");
        let edit = EditOperation::CreateFile(CreateFileEdit {
            file: "src/existing.rs".into(),
            content: "pub fn g() {}\n".into(),
            executable: None,
        });
        let err = render_single_edit(&repo, &edit).unwrap_err();
        assert!(
            err.to_string()
                .contains("provider targets existing file for creation"),
            "unexpected error: {err}"
        );
        assert_rejection_persisted(
            &repo,
            "create_target_exists",
            "create_file",
            "src/existing.rs",
        );
    }

    #[test]
    fn delete_missing_file_persists_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn f() {}\n");
        let edit = EditOperation::DeleteFile(DeleteFileEdit {
            file: "src/missing.rs".into(),
        });
        let err = render_single_edit(&repo, &edit).unwrap_err();
        assert!(
            err.to_string()
                .contains("provider targets missing file for deletion"),
            "unexpected error: {err}"
        );
        assert_rejection_persisted(
            &repo,
            "delete_target_missing",
            "delete_file",
            "src/missing.rs",
        );
    }

    #[test]
    fn search_text_not_found_persists_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn f() {}\n");
        let edit = EditOperation::SearchReplace(SearchReplaceEdit {
            file: "src/existing.rs".into(),
            search: "nonexistent_token".into(),
            replace: "x".into(),
            replace_all: None,
            context_lines: None,
        });
        let err = render_single_edit(&repo, &edit).unwrap_err();
        assert!(
            err.to_string().contains("provider search text not found"),
            "unexpected error: {err}"
        );
        assert_rejection_persisted(
            &repo,
            "search_text_not_found",
            "search_replace",
            "src/existing.rs",
        );
    }

    #[test]
    fn search_text_ambiguous_persists_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn a() {}\npub fn a() {}\n");
        let edit = EditOperation::SearchReplace(SearchReplaceEdit {
            file: "src/existing.rs".into(),
            search: "pub fn a() {}".into(),
            replace: "pub fn b() {}".into(),
            replace_all: None,
            context_lines: None,
        });
        let err = render_single_edit(&repo, &edit).unwrap_err();
        assert!(
            err.to_string().contains("matched multiple locations"),
            "unexpected error: {err}"
        );
        assert_rejection_persisted(
            &repo,
            "search_text_ambiguous",
            "search_replace",
            "src/existing.rs",
        );
    }

    #[test]
    fn create_file_renders_without_leaked_temp_prefix() {
        // Regression test for the leaked `a/n/` / `b/n/` temp path segments
        // that `git diff --no-index` emits for create-file operations. The
        // renderer must normalize create-file headers to repo-relative
        // `a/<rel>` / `b/<rel>` and never leak the temporary `o/` or `n/`
        // prefixes used inside the OS temp directory.
        let raw = "\
diff --git a/n/tests/foo.rs b/n/tests/foo.rs\n\
new file mode 100644\n\
index 0000000..7f869ab\n\
--- /dev/null\n\
+++ b/n/tests/foo.rs\n\
@@ -0,0 +1,2 @@\n\
+use foo;\n\
+";
        let out = crate::workflow::rewrite_diff_header(raw);
        assert!(
            out.contains("--- /dev/null"),
            "create-file old side must be /dev/null, got:\n{out}"
        );
        assert!(
            out.contains("+++ b/tests/foo.rs"),
            "create-file new side must be b/tests/foo.rs, got:\n{out}"
        );
        for bad in ["a/n/", "b/n/", "a/o/", "b/o/"] {
            assert!(
                !out.contains(bad),
                "leaked temp prefix {bad} in rendered header:\n{out}"
            );
        }
    }

    #[test]
    fn valid_search_replace_renders_without_rejection() {
        let (_d, repo) = temp_repo_with_file("src/existing.rs", "pub fn f() {}\n");
        let edit = EditOperation::SearchReplace(SearchReplaceEdit {
            file: "src/existing.rs".into(),
            search: "pub fn f".into(),
            replace: "pub fn g".into(),
            replace_all: None,
            context_lines: None,
        });
        assert!(render_single_edit(&repo, &edit).is_ok());
        assert!(
            diagnostics_files(&repo).is_empty(),
            "a valid edit must not write any diagnostics"
        );
    }
}
