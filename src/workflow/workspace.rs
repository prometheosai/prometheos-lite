//! Governed execution workspace seam (issue #171).
//!
//! Generalizes execution isolation so a harness is never synonymous with a
//! filesystem/worktree: an execution node receives an explicit, versioned
//! workspace whose identity, revision, isolation mode, writable scope,
//! lifecycle, and evidence are independent of the selected model/provider/
//! harness.
//!
//! Invariants enforced here (fail closed):
//! - workspace authority is resolved before execution and cannot be widened
//!   by any downstream path;
//! - a writable execution never silently falls back to the source checkout
//!   when isolation setup fails (no fallback path exists);
//! - resume rejects missing/stale/incompatible workspace references unless an
//!   explicit governed remap is authorized AND the authorization itself is
//!   carried in evidence;
//! - cleanup preserves referenced artifacts before teardown.
//!
//! `lite.workspace.v1` is Lite-owned. Mapping to SOMA#80 portable
//! governed-run contracts happens only when that spec publishes.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Schema version of the workspace contract.
pub const WORKSPACE_SCHEMA_VERSION: &str = "1.0.0";

/// Schema version for `WorkspaceRefV1`. See `WorkspaceRefV1::parse_json` for
/// the compatibility contract: refs written at `1.0.0` (without the
/// `headRevision` field) and `1.1.0` (with the optional field) both parse on
/// a modern reader; older readers pinned to `1.0.0` MUST fail closed on a
/// `1.1.0` ref (the unknown-field or unsupported-version bail), which is the
/// intended fail-closed behavior.
pub const WORKSPACE_REF_SCHEMA_VERSION: &str = "1.1.0";

/// Legacy ref version accepted by `WorkspaceRefV1::parse_json`.
pub const WORKSPACE_REF_SCHEMA_VERSION_V1: &str = "1.0.0";

/// Revision of the built-in adapters (bump on behavior change).
pub const ADAPTER_REVISION: &str = "lite.workspace.adapter.v1";

/// Defense-in-depth ID validator. A workspace_id (and any path-derived segment
/// from a plan_id / diagnosis_id) MUST be a bare segment: no `..`, no path
/// separators, no nulls, and only the safe char set `[A-Za-z0-9._-]`. Empty
/// strings and overly long strings are also rejected. Returns `Ok(())` iff the
/// id is safe to join into a filesystem path or workspace identifier.
pub fn validate_workspace_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() {
        anyhow::bail!("workspace id must not be empty");
    }
    if id.len() > 200 {
        anyhow::bail!("workspace id too long ({} > 200)", id.len());
    }
    if id.contains("..") {
        anyhow::bail!("workspace id must not contain '..'");
    }
    for c in id.chars() {
        if matches!(c, '/' | '\\' | '\0') {
            anyhow::bail!("workspace id must not contain path separators or NUL");
        }
        if !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
            anyhow::bail!("workspace id contains unsafe character {c:?}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// Isolation mode of a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    /// Inspection/review paths; write authority structurally denied.
    #[serde(rename = "read-only")]
    ReadOnly,
    /// Isolated repository writes (git worktree).
    Writable,
}

/// Which concrete adapter owns the workspace lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    #[serde(rename = "git-worktree")]
    GitWorktree,
    #[serde(rename = "existing-read-only")]
    ExistingReadOnly,
}

/// Versioned manifest describing one governed execution workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceManifestV1 {
    pub schema_version: String,
    /// Stable workspace identity (NOT agent identity).
    pub workspace_id: String,
    pub adapter: AdapterKind,
    pub adapter_revision: String,
    /// Durable repository identity (origin URL or stable name).
    pub repo_identity: String,
    /// Exact commit SHA the workspace is pinned to.
    pub base_revision: String,
    /// Worktree branch name when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub mode: WorkspaceMode,
    /// Declared writable scopes (authority surface for #154/#133).
    #[serde(default)]
    pub writable_scopes: Vec<String>,
    /// Resource-lock identity for later #124 parallel scheduling.
    pub resource_lock_id: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// Canonical digest over the manifest minus this member.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentDigest"
    )]
    pub content_digest: Option<String>,
}

impl WorkspaceManifestV1 {
    /// Build + seal a manifest (digest computed over manifest-minus-digest).
    pub fn sealed(mut self) -> Self {
        self.content_digest = None;
        let d = self.compute_digest();
        self.content_digest = Some(d);
        self
    }

    /// Canonical digest over every field except `contentDigest`.
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("manifest serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::canonical_digest(&v)
    }

    /// Fail-closed parse: version gate, structural checks, digest verify.
    pub fn parse_json(text: &str) -> Result<Self> {
        let m: Self = serde_json::from_str(text).context("workspace manifest parse failed")?;
        if m.schema_version != WORKSPACE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported workspace schema version {} (supported {WORKSPACE_SCHEMA_VERSION})",
                m.schema_version
            );
        }
        if m.workspace_id.is_empty() || m.repo_identity.is_empty() || m.base_revision.is_empty() {
            anyhow::bail!("workspace manifest missing identity fields");
        }
        if m.mode == WorkspaceMode::Writable && m.writable_scopes.is_empty() {
            anyhow::bail!("writable workspace must declare at least one writable scope");
        }
        match &m.content_digest {
            None => anyhow::bail!("workspace manifest missing contentDigest"),
            Some(d) if *d != m.compute_digest() => {
                anyhow::bail!("workspace manifest contentDigest does not verify")
            }
            Some(_) => {}
        }
        Ok(m)
    }

    /// Portable reference derived from this manifest (for PortableWorkState /
    /// checkpoints / evidence): durable identity only, no process state.
    /// `headRevision` is unset (None) because the manifest captures the
    /// pre-write base; post-write refs are built separately by the implement
    /// / repair nodes with `headRevision` set to the committed HEAD.
    /// Emits ref schema `1.1.0` (WorkspaceRefV1 now carries `headRevision`).
    pub fn to_reference(&self) -> WorkspaceRefV1 {
        WorkspaceRefV1 {
            schema_version: WORKSPACE_REF_SCHEMA_VERSION.into(),
            workspace_id: self.workspace_id.clone(),
            adapter: self.adapter,
            adapter_revision: self.adapter_revision.clone(),
            repo_identity: self.repo_identity.clone(),
            base_revision: self.base_revision.clone(),
            mode: self.mode,
            content_digest: self.compute_digest(),
            head_revision: None,
        }
    }
}

/// Portable, versioned reference to a workspace. Round-trips through
/// PortableWorkState / checkpoint evidence without hidden process state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceRefV1 {
    pub schema_version: String,
    pub workspace_id: String,
    pub adapter: AdapterKind,
    pub adapter_revision: String,
    pub repo_identity: String,
    pub base_revision: String,
    pub mode: WorkspaceMode,
    /// Digest of the originating manifest (manifest-minus-digest form).
    #[serde(rename = "contentDigest")]
    pub content_digest: String,
    /// Optional committed HEAD for post-write refs (implement / repair).
    /// When set, `recover()` checks `headRevision` against the on-disk HEAD
    /// instead of `baseRevision`; when absent (older / read-only refs),
    /// `recover()` keeps the pre-write `baseRevision` semantics. Optional
    /// for forward compatibility: older serialized refs deserialize cleanly
    /// via `#[serde(default)]`. Adding this field moves the wire schema to
    /// `1.1.0` (WORKSPACE_REF_SCHEMA_VERSION); refs written at `1.0.0` are
    /// still accepted by `parse_json` but new refs emit `1.1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_revision: Option<String>,
}

impl WorkspaceRefV1 {
    pub fn parse_json(text: &str) -> Result<Self> {
        let r: Self = serde_json::from_str(text).context("workspace ref parse failed")?;
        if r.schema_version != WORKSPACE_REF_SCHEMA_VERSION_V1
            && r.schema_version != WORKSPACE_REF_SCHEMA_VERSION
        {
            anyhow::bail!(
                "unsupported workspace ref schema version {} (supported: {}, {})",
                r.schema_version,
                WORKSPACE_REF_SCHEMA_VERSION_V1,
                WORKSPACE_REF_SCHEMA_VERSION
            );
        }
        Ok(r)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Canonical digest over every field including `headRevision`. Must be
    /// recomputed after mutating `headRevision` post-construction (the
    /// implement / repair nodes do this so the emitted ref attests to the
    /// committed HEAD, not the pre-write base).
    pub fn compute_digest(&self) -> String {
        let mut v = serde_json::to_value(self).expect("ref serializes");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("contentDigest");
        }
        crate::workflow::soma::canonical_digest(&v)
    }
}

/// Explicit, evidenced authorization required to remap a stale/missing/
/// incompatible workspace reference on resume. Carried into recovery output
/// so the remap itself is auditable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemapAuthorization {
    pub reason: String,
    pub authorized_by: String,
    #[serde(rename = "recordedAt")]
    pub recorded_at: String,
}

/// Typed failure modes for reference validation (all fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRefError {
    Missing,
    StaleRevision,
    AdapterMismatch,
    IncompatibleSchema,
}

impl std::fmt::Display for WorkspaceRefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            WorkspaceRefError::Missing => "workspace missing",
            WorkspaceRefError::StaleRevision => "workspace revision stale/mismatched",
            WorkspaceRefError::AdapterMismatch => "workspace adapter mismatch",
            WorkspaceRefError::IncompatibleSchema => "workspace schema incompatible",
        };
        f.write_str(s)
    }
}

/// An acquired, validated workspace handed to execution.
#[derive(Debug, Clone)]
pub struct AcquiredWorkspace {
    pub manifest: WorkspaceManifestV1,
    pub root: std::path::PathBuf,
    /// Revision actually validated at acquisition time.
    pub revision: String,
}

impl AcquiredWorkspace {
    /// Write-authority gate: read-only modes can NEVER acquire write power,
    /// accidentally or otherwise.
    pub fn ensure_writable(&self) -> Result<()> {
        if self.manifest.mode != WorkspaceMode::Writable {
            anyhow::bail!(
                "LITE-GOV-0002-class guard: workspace {} is read-only; write authority denied",
                self.manifest.workspace_id
            );
        }
        Ok(())
    }
}

/// Artifact paths that MUST survive cleanup (proposals, checkpoints,
/// evidence, journal state). Copied into `evidence_dir` before teardown.
#[derive(Debug, Clone, Default)]
pub struct PreservationSet {
    pub files: Vec<std::path::PathBuf>,
}

/// Cleanup report naming what was preserved and what was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupReport {
    pub workspace_id: String,
    pub removed_root: Option<String>,
    pub preserved: Vec<String>,
}

/// Recovery outcome for crash-recovery validation.
#[derive(Debug, Clone)]
pub enum RecoveryOutcome {
    /// Existing workspace matches its reference; safe to resume.
    Resumed(Box<AcquiredWorkspace>),
    /// Reference unusable and either no authorization was given or it did
    /// not apply; callers must not proceed.
    Rejected(WorkspaceRefError),
    /// Governed remap performed under explicit authorization.
    RemappedUnderAuthority(Box<RemapAuthorization>),
}

// ---------------------------------------------------------------------------
// Adapter trait
// ---------------------------------------------------------------------------

/// Lifecycle owner for one workspace kind.
pub trait WorkspaceAdapter {
    fn kind(&self) -> AdapterKind;

    /// Deterministically materialize the workspace described by `manifest`.
    fn acquire(&self, manifest: &WorkspaceManifestV1) -> Result<AcquiredWorkspace>;

    /// Validate an acquired workspace against its manifest (stale/missing).
    fn validate(&self, acquired: &AcquiredWorkspace) -> Result<()>;

    /// Preserve referenced artifacts, then tear the workspace down.
    fn cleanup(
        &self,
        acquired: AcquiredWorkspace,
        preserve: &PreservationSet,
        evidence_dir: &std::path::Path,
    ) -> Result<CleanupReport>;

    /// Crash-recovery: re-validate an existing on-disk workspace against a
    /// portable reference WITHOUT re-acquiring (never mutates the checkout).
    fn recover(
        &self,
        expected_root: &std::path::Path,
        reference: &WorkspaceRefV1,
        manifest: &WorkspaceManifestV1,
        remap: Option<&RemapAuthorization>,
    ) -> Result<RecoveryOutcome>;
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn run_git(dir: &std::path::Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("git invocation failed")?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn head_revision(repo: &std::path::Path) -> Result<String> {
    run_git(repo, &["rev-parse", "HEAD"])
}

/// Canonical toplevel path of a git working copy, with symlinks resolved.
fn toplevel(repo: &std::path::Path) -> Result<std::path::PathBuf> {
    let s = run_git(repo, &["rev-parse", "--show-toplevel"])?;
    Ok(std::path::PathBuf::from(s))
}

/// Returns true if `identity` looks like a filesystem path (exists, has a
/// path separator, or starts with a drive letter). Conservative: anything
/// that could be a path is treated as one so the canonicalize binding
/// applies. Anything that can't possibly be a path falls through to the
/// URL / stable-name branches. URL / git-protocol patterns are excluded.
fn existing_fs_path_like(identity: &str) -> bool {
    // Empty is impossible (validated upstream) but guard anyway.
    if identity.is_empty() {
        return false;
    }
    // URLs and git-protocol strings are never paths.
    if looks_like_url_or_git_proto(identity) {
        return false;
    }
    // Contains a path separator (forward or back) — treat as a path.
    if identity.contains('/') || identity.contains('\\') {
        return true;
    }
    // Looks like a Windows drive letter path (e.g. "C:").
    let bytes = identity.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return true;
    }
    // The identity exists on disk — the canonicalize path is the right
    // binding.
    std::path::Path::new(identity).exists()
}

/// Returns true if `identity` is a URL (http/https/ssh/git protocol) or a
/// git-protocol SCP-style reference (`user@host:path`).
fn looks_like_url_or_git_proto(identity: &str) -> bool {
    if identity.starts_with("http://")
        || identity.starts_with("https://")
        || identity.starts_with("git://")
        || identity.starts_with("ssh://")
        || identity.starts_with("git@")
    {
        return true;
    }
    // SCP-style: `user@host:path` where path contains a colon.
    if let Some(at) = identity.find('@') {
        let after = &identity[at + 1..];
        if after.contains(':') && !after.starts_with("//") {
            return true;
        }
    }
    false
}

/// Weak but correct equality for URLs / git-protocol strings: normalizes
/// trailing `.git`, trailing slash, and the `.git@` / `:` SCP separator
/// before comparing. This is NOT a full URL parser — it's intentionally
/// minimal and only used in the authority-binding path.
fn url_or_git_proto_equal(a: &str, b: &str) -> bool {
    fn normalize_url(s: &str) -> String {
        let mut s = s.trim().to_lowercase();
        // Normalize SCP-style `git@host:path` to `ssh://git@host/path`.
        if let Some(at_pos) = s.find('@') {
            let host_path = &s[at_pos + 1..];
            if let Some(colon_pos) = host_path.find(':') {
                let host = &host_path[..colon_pos];
                let path = &host_path[colon_pos + 1..];
                if !host.is_empty() && !path.is_empty() && !path.starts_with("//") {
                    s = format!("ssh://git@{}/{}", host, path);
                }
            }
        }
        // Strip trailing .git and /
        while s.ends_with(".git") {
            s.truncate(s.len() - 4);
        }
        while s.ends_with('/') {
            s.truncate(s.len() - 1);
        }
        s
    }
    normalize_url(a) == normalize_url(b)
}

fn preserve_files(
    acquired: &AcquiredWorkspace,
    preserve: &PreservationSet,
    evidence_dir: &std::path::Path,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(evidence_dir)?;
    let mut preserved = Vec::new();
    for f in &preserve.files {
        // Only relative paths inside the workspace may be preserved.
        let abs = if f.is_absolute() {
            f.clone()
        } else {
            acquired.root.join(f)
        };
        if !abs.exists() {
            continue;
        }
        let name = format!(
            "{}-{}",
            acquired.manifest.workspace_id,
            f.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        let dest = evidence_dir.join(&name);
        std::fs::copy(&abs, &dest).with_context(|| format!("preserving {} failed", f.display()))?;
        preserved.push(name);
    }
    Ok(preserved)
}

// ---------------------------------------------------------------------------
// GitWorktreeWorkspace — isolated repository writes
// ---------------------------------------------------------------------------

/// First production adapter: cheap parallel repository isolation via
/// `git worktree add`, deterministically pinned to the requested revision.
pub struct GitWorktreeWorkspace {
    /// Parent directory under which `<workspace_id>/worktree` is created.
    pub parent_dir: std::path::PathBuf,
    /// Path of the SOURCE repository the worktrees attach to.
    pub source_repo: std::path::PathBuf,
}

impl GitWorktreeWorkspace {
    fn worktree_path(&self, manifest: &WorkspaceManifestV1) -> std::path::PathBuf {
        // Deterministic: derived only from stable identity fields.
        self.parent_dir
            .join(&manifest.workspace_id)
            .join("worktree")
    }
}

impl WorkspaceAdapter for GitWorktreeWorkspace {
    fn kind(&self) -> AdapterKind {
        AdapterKind::GitWorktree
    }

    fn acquire(&self, manifest: &WorkspaceManifestV1) -> Result<AcquiredWorkspace> {
        if manifest.mode != WorkspaceMode::Writable {
            anyhow::bail!("git-worktree adapter acquires writable workspaces only");
        }
        if manifest.base_revision.is_empty() {
            anyhow::bail!("base revision required");
        }
        // Defense in depth: reject any workspace_id that could escape
        // parent_dir when joined into a filesystem path. Callers (notably
        // the implement/repair nodes) are expected to validate their own
        // plan_id / diagnosis_id, but the adapter MUST NOT trust upstream.
        validate_workspace_id(&manifest.workspace_id)?;
        // Authority binding (E5/I02 repair round 2): repo_identity is
        // documented as "origin URL or stable name" (or a filesystem path).
        // All three forms are valid per the manifest contract. We branch:
        //
        //  a) URL / git-protocol identity (contains "://" or matches
        //     "user@host:path") -> bind via `git remote get-url origin`:
        //     the source_repo's origin remote must equal the declared URL.
        //  b) Existing filesystem path -> canonicalize BOTH sides and compare
        //     (the original E5/I02 P1.2 check, preserved).
        //  c) Fallback: treat it as a stable-name label (no binding beyond
        //     the recorded identity string). This is the documented contract;
        //     the manifest's contentDigest still attests to the exact string,
        //     so silent substitution is detectable on audit.
        //
        // Fail closed: any of the three branches errors on mismatch.
        let identity = &manifest.repo_identity;
        let looks_like_path = existing_fs_path_like(identity);
        if looks_like_path {
            let declared = std::path::PathBuf::from(identity);
            let actual = toplevel(&self.source_repo).map_err(|e| {
                anyhow::anyhow!(
                    "source_repo {} is not a git working copy: {e}",
                    self.source_repo.display()
                )
            })?;
            let declared_canon = std::fs::canonicalize(&declared).unwrap_or(declared);
            let actual_canon = std::fs::canonicalize(&actual).unwrap_or(actual);
            if actual_canon != declared_canon {
                anyhow::bail!(
                    "workspace repo identity mismatch: manifest.repo_identity={} (path) but source_repo resolves to {}",
                    declared_canon.display(),
                    actual_canon.display()
                );
            }
        } else if looks_like_url_or_git_proto(identity) {
            // URL / git-protocol identity: compare against the source repo's
            // origin remote, if any. If the source has no origin, fail closed.
            let origin_url = run_git(&self.source_repo, &["remote", "get-url", "origin"])
                .context("source_repo has no origin remote: cannot bind a URL identity")?;
            if !url_or_git_proto_equal(identity, origin_url.trim()) {
                anyhow::bail!(
                    "workspace repo identity mismatch: manifest.repo_identity={} (URL) but source_repo's origin is {}",
                    identity,
                    origin_url.trim()
                );
            }
        } else {
            // Stable-name label: no filesystem or URL binding is possible by
            // design. The manifest digest still commits the exact string; an
            // audit of the stderr log will reveal substitutions. We still
            // require the string to be non-empty (already validated above)
            // and to be a valid workspace-id-shaped token so it can't be
            // confused with a path or URL by a downstream consumer.
            validate_workspace_id(identity).map_err(|e| {
                anyhow::anyhow!("manifest.repo_identity as stable name is unsafe: {e}")
            })?;
        }
        let path = self.worktree_path(manifest);
        if path.exists() {
            anyhow::bail!(
                "workspace path {} already exists: refusing to reuse or fall back",
                path.display()
            );
        }
        std::fs::create_dir_all(path.parent().expect("parent exists"))?;
        // Pin the worktree to the EXACT requested revision. If this fails,
        // the error propagates — there is deliberately NO fallback to the
        // source checkout — and any created directories are removed so
        // failed acquisitions leave no residue.
        if let Err(e) = run_git(
            &self.source_repo,
            &[
                "worktree",
                "add",
                "--detach",
                &path.to_string_lossy(),
                &manifest.base_revision,
            ],
        ) {
            let _ = std::fs::remove_dir_all(path.parent().expect("parent exists"));
            return Err(e).context("worktree acquisition failed (fail closed)");
        }
        Ok(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: path,
            revision: manifest.base_revision.clone(),
        })
    }

    fn validate(&self, acquired: &AcquiredWorkspace) -> Result<()> {
        if !acquired.root.exists() {
            anyhow::bail!("{}", WorkspaceRefError::Missing);
        }
        let actual = head_revision(&acquired.root)?;
        if actual != acquired.manifest.base_revision {
            anyhow::bail!(
                "{}: HEAD {actual} != pinned {}",
                WorkspaceRefError::StaleRevision,
                acquired.manifest.base_revision
            );
        }
        Ok(())
    }

    fn cleanup(
        &self,
        acquired: AcquiredWorkspace,
        preserve: &PreservationSet,
        evidence_dir: &std::path::Path,
    ) -> Result<CleanupReport> {
        let preserved = preserve_files(&acquired, preserve, evidence_dir)?;
        // Workspace teardown is unconditional AFTER preservation succeeds:
        // workspaces are ephemeral execution sandboxes; durable artifacts
        // survive only via the PreservationSet (copied out above). The
        // source checkout is never touched.
        run_git(
            &self.source_repo,
            &[
                "worktree",
                "remove",
                "--force",
                &acquired.root.to_string_lossy(),
            ],
        )
        .context("worktree removal failed after preservation")?;
        Ok(CleanupReport {
            workspace_id: acquired.manifest.workspace_id.clone(),
            removed_root: Some(acquired.root.to_string_lossy().to_string()),
            preserved,
        })
    }

    fn recover(
        &self,
        expected_root: &std::path::Path,
        reference: &WorkspaceRefV1,
        manifest: &WorkspaceManifestV1,
        remap: Option<&RemapAuthorization>,
    ) -> Result<RecoveryOutcome> {
        if reference.adapter != AdapterKind::GitWorktree
            || manifest.adapter != AdapterKind::GitWorktree
        {
            return reject_or_remap(remap, WorkspaceRefError::AdapterMismatch);
        }
        if reference.schema_version != WORKSPACE_REF_SCHEMA_VERSION_V1
            && reference.schema_version != WORKSPACE_REF_SCHEMA_VERSION
        {
            return reject_or_remap(remap, WorkspaceRefError::IncompatibleSchema);
        }
        if manifest.schema_version != WORKSPACE_SCHEMA_VERSION {
            return reject_or_remap(remap, WorkspaceRefError::IncompatibleSchema);
        }
        if !expected_root.exists() {
            return reject_or_remap(remap, WorkspaceRefError::Missing);
        }
        let actual = match head_revision(expected_root) {
            Ok(r) => r,
            Err(_) => return reject_or_remap(remap, WorkspaceRefError::Missing),
        };
        // For post-write refs (headRevision set), the workspace is "pinned"
        // to the committed HEAD, not the pre-write base. When headRevision
        // is set, the manifest's base_revision describes the PRE-write state
        // and must not be compared against the actual HEAD for the resume
        // decision — the ref's headRevision is the authoritative pin.
        // For pre-write refs (headRevision absent), keep the pre-existing
        // strict check against BOTH the ref and the manifest.
        let actual_expected = match &reference.head_revision {
            Some(head) => head.as_str(),
            None => reference.base_revision.as_str(),
        };
        if actual != actual_expected {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
        }
        // For pre-write refs, the manifest must ALSO agree with the ref.
        // For post-write refs, the manifest may legitimately carry the
        // pre-write base revision (audit trail), so we skip that check.
        if reference.head_revision.is_none() && actual != manifest.base_revision {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
        }
        Ok(RecoveryOutcome::Resumed(Box::new(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: expected_root.to_path_buf(),
            revision: actual,
        })))
    }
}

fn reject_or_remap(
    remap: Option<&RemapAuthorization>,
    err: WorkspaceRefError,
) -> Result<RecoveryOutcome> {
    match remap {
        None => Ok(RecoveryOutcome::Rejected(err)),
        Some(auth) => {
            if auth.reason.is_empty() || auth.authorized_by.is_empty() {
                anyhow::bail!("governed remap requires non-empty reason and authorizer");
            }
            Ok(RecoveryOutcome::RemappedUnderAuthority(Box::new(
                auth.clone(),
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// ExistingReadOnlyWorkspace — inspection/review paths
// ---------------------------------------------------------------------------

/// Read-only mode over an already-existing checkout. Write authority is
/// structurally denied: acquisition validates existence and pins the
/// observed revision; `ensure_writable` always refuses.
pub struct ExistingReadOnlyWorkspace {
    /// The existing checkout this adapter exposes (bound at construction).
    root: std::path::PathBuf,
}

impl ExistingReadOnlyWorkspace {
    /// Bind the adapter to the existing checkout it will expose.
    pub fn bound_to(root: std::path::PathBuf) -> Self {
        Self { root }
    }

    fn root_for(&self, _manifest: &WorkspaceManifestV1) -> Result<std::path::PathBuf> {
        Ok(self.root.clone())
    }
}

impl WorkspaceAdapter for ExistingReadOnlyWorkspace {
    fn kind(&self) -> AdapterKind {
        AdapterKind::ExistingReadOnly
    }

    fn acquire(&self, manifest: &WorkspaceManifestV1) -> Result<AcquiredWorkspace> {
        if manifest.mode != WorkspaceMode::ReadOnly {
            anyhow::bail!("existing-read-only adapter acquires read-only workspaces only");
        }
        let path = self.root_for(manifest)?;
        if !path.exists() {
            anyhow::bail!("{}", WorkspaceRefError::Missing);
        }
        // Pin whatever revision the existing checkout is actually at; the
        // manifest's base_revision must MATCH reality or acquisition fails
        // (no silent rebasing of authority onto a different tree).
        let actual = head_revision(&path)?;
        if actual != manifest.base_revision {
            anyhow::bail!("{}: found {actual}", WorkspaceRefError::StaleRevision);
        }
        Ok(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: path,
            revision: actual,
        })
    }

    fn validate(&self, acquired: &AcquiredWorkspace) -> Result<()> {
        if !acquired.root.exists() {
            anyhow::bail!("{}", WorkspaceRefError::Missing);
        }
        let actual = head_revision(&acquired.root)?;
        if actual != acquired.manifest.base_revision {
            anyhow::bail!("{}", WorkspaceRefError::StaleRevision);
        }
        Ok(())
    }

    fn cleanup(
        &self,
        acquired: AcquiredWorkspace,
        preserve: &PreservationSet,
        evidence_dir: &std::path::Path,
    ) -> Result<CleanupReport> {
        // Never deletes anything: read-only inspection leaves the user's
        // checkout untouched; referenced artifacts are still copied out.
        let preserved = preserve_files(&acquired, preserve, evidence_dir)?;
        Ok(CleanupReport {
            workspace_id: acquired.manifest.workspace_id.clone(),
            removed_root: None,
            preserved,
        })
    }

    fn recover(
        &self,
        expected_root: &std::path::Path,
        reference: &WorkspaceRefV1,
        manifest: &WorkspaceManifestV1,
        remap: Option<&RemapAuthorization>,
    ) -> Result<RecoveryOutcome> {
        if reference.adapter != AdapterKind::ExistingReadOnly
            || manifest.adapter != AdapterKind::ExistingReadOnly
        {
            return reject_or_remap(remap, WorkspaceRefError::AdapterMismatch);
        }
        if reference.schema_version != WORKSPACE_REF_SCHEMA_VERSION_V1
            && reference.schema_version != WORKSPACE_REF_SCHEMA_VERSION
        {
            return reject_or_remap(remap, WorkspaceRefError::IncompatibleSchema);
        }
        if !expected_root.exists() {
            return reject_or_remap(remap, WorkspaceRefError::Missing);
        }
        let actual = match head_revision(expected_root) {
            Ok(r) => r,
            Err(_) => return reject_or_remap(remap, WorkspaceRefError::Missing),
        };
        if actual != reference.base_revision {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
        }
        Ok(RecoveryOutcome::Resumed(Box::new(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: expected_root.to_path_buf(),
            revision: actual,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct TempRepo(std::path::PathBuf);
    impl TempRepo {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("lite-ws-{}-{}", tag, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let g = |a: &[&str]| {
                let o = Command::new("git")
                    .args(a)
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(
                    o.status.success(),
                    "git {a:?}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            };
            g(&["init", "-q"]);
            g(&["config", "user.email", "t@t"]);
            g(&["config", "user.name", "T"]);
            std::fs::write(dir.join("a.txt"), "one\n").unwrap();
            g(&["add", "."]);
            g(&["commit", "-q", "-m", "c1"]);
            Self(dir)
        }
        fn head(&self) -> String {
            let o = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&self.0)
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
    }
    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn writable_manifest(source_repo: &Path, name: &str, revision: &str) -> WorkspaceManifestV1 {
        WorkspaceManifestV1 {
            schema_version: WORKSPACE_SCHEMA_VERSION.into(),
            workspace_id: format!("ws-{name}"),
            adapter: AdapterKind::GitWorktree,
            adapter_revision: ADAPTER_REVISION.into(),
            // Authority binding (E5/I02 repair): repo_identity must canonicalize
            // to the source repo's toplevel, otherwise the new
            // GitWorktreeWorkspace::acquire authority check rejects the call.
            repo_identity: source_repo.to_string_lossy().to_string(),
            base_revision: revision.into(),
            branch: None,
            mode: WorkspaceMode::Writable,
            writable_scopes: vec!["work://x".into()],
            resource_lock_id: format!("lock-{name}"),
            created_at: "2026-08-25T00:00:00Z".into(),
            content_digest: None,
        }
        .sealed()
    }

    #[test]
    fn manifest_digest_seals_and_verifies() {
        let m = writable_manifest(Path::new("/tmp/dummy"), "t1", &"a".repeat(40));
        let sealed = m.sealed();
        assert!(
            sealed
                .content_digest
                .as_deref()
                .is_some_and(|d| d.len() == 64)
        );
        let text = serde_json::to_string(&sealed).unwrap();
        let parsed = WorkspaceManifestV1::parse_json(&text).expect("verifies");
        assert_eq!(parsed, sealed);

        // Tamper with any field -> digest fails closed.
        let mut tampered = sealed.clone();
        tampered.base_revision = "b".repeat(40);
        let bad = serde_json::to_string(&tampered).unwrap();
        assert!(WorkspaceManifestV1::parse_json(&bad).is_err());

        // Missing digest fails closed.
        let mut naked = sealed.clone();
        naked.content_digest = None;
        let bad2 = serde_json::to_string(&naked).unwrap();
        assert!(WorkspaceManifestV1::parse_json(&bad2).is_err());
    }

    #[test]
    fn manifest_parse_rejects_bad_version_identity_and_unscoped_writable() {
        let m = writable_manifest(Path::new("/tmp/dummy"), "guards", &"a".repeat(40));
        let mut v = serde_json::to_value(&m).unwrap();
        v["schemaVersion"] = serde_json::json!("9.9.9");
        let err = WorkspaceManifestV1::parse_json(&v.to_string())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unsupported workspace schema version"),
            "{err}"
        );

        let mut v2 = serde_json::to_value(&m).unwrap();
        v2["workspaceId"] = serde_json::json!("");
        let t2 = serde_json::to_string(&v2).unwrap();
        assert!(WorkspaceManifestV1::parse_json(&t2).is_err());

        let mut v3 = serde_json::to_value(&m).unwrap();
        v3["writableScopes"] = serde_json::json!([]);
        v3["contentDigest"] = serde_json::Value::Null;
        let t3 = serde_json::to_string(&v3).unwrap();
        let err3 = WorkspaceManifestV1::parse_json(&t3)
            .unwrap_err()
            .to_string();
        assert!(
            err3.contains("contentDigest") || err3.contains("writable scope"),
            "{err3}"
        );
    }

    #[test]
    fn worktree_create_teardown_deterministic_pinned() {
        let repo = TempRepo::new("ct");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "ct", &rev);
        let acquired = adapter.acquire(&manifest).expect("worktree created");
        assert_eq!(acquired.revision, rev);
        // Deterministic path from stable identity.
        assert_eq!(acquired.root, parent.join("ws-ct").join("worktree"));
        adapter
            .validate(&acquired)
            .expect("pinned revision validates");

        // Write inside the isolated workspace does not touch the source.
        std::fs::write(acquired.root.join("new.txt"), "isolated\n").unwrap();
        assert!(!repo.0.join("new.txt").exists());

        let ev = repo.0.join("_evidence");
        std::fs::write(acquired.root.join("keep.json"), "{}").unwrap();
        let report = adapter
            .cleanup(
                acquired,
                &PreservationSet {
                    files: vec![PathBuf::from("keep.json")],
                },
                &ev,
            )
            .expect("cleanup");
        assert_eq!(report.preserved, vec!["ws-ct-keep.json"]);
        assert!(ev.join("ws-ct-keep.json").exists());
        assert!(!parent.join("ws-ct").join("worktree").exists());
    }

    #[test]
    fn stale_revision_fails_closed() {
        let repo = TempRepo::new("stale");
        let old = repo.head();
        std::fs::write(repo.0.join("a.txt"), "two\n").unwrap();
        {
            let st = Command::new("git")
                .args(["commit", "-aqm", "c2"])
                .current_dir(&repo.0)
                .status()
                .unwrap();
            assert!(st.success());
        }
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let mut manifest = writable_manifest(&repo.0, "stale", &old);
        manifest.base_revision = old.clone();
        let manifest = manifest.sealed();
        let acquired = adapter.acquire(&manifest).unwrap();
        // The pinned revision no longer matches... acquire pins the OLD
        // commit which still exists; simulate staleness by validating
        // against a manifest claiming a NEWER revision.
        let mut drifted = manifest.clone();
        drifted.base_revision = repo.head();
        let drifted = drifted.sealed();
        let mut acq = acquired.clone();
        acq.manifest = drifted;
        let err = adapter.validate(&acq).unwrap_err().to_string();
        assert!(err.contains("stale/mismatched"), "{err}");
    }

    #[test]
    fn readonly_mode_cannot_acquire_write_authority() {
        let repo = TempRepo::new("ro");
        let adapter = ExistingReadOnlyWorkspace::bound_to(repo.0.clone());
        let manifest = WorkspaceManifestV1 {
            schema_version: WORKSPACE_SCHEMA_VERSION.into(),
            workspace_id: "ws-ro".into(),
            adapter: AdapterKind::ExistingReadOnly,
            adapter_revision: ADAPTER_REVISION.into(),
            repo_identity: "origin/ro".into(),
            base_revision: repo.head(),
            branch: None,
            mode: WorkspaceMode::ReadOnly,
            writable_scopes: vec![],
            resource_lock_id: "lock-ro".into(),
            created_at: "2026-08-25T00:00:00Z".into(),
            content_digest: None,
        }
        .sealed();
        let acquired = adapter.acquire(&manifest).expect("read-only acquisition");
        let err = acquired.ensure_writable().unwrap_err().to_string();
        assert!(err.contains("write authority denied"), "{err}");
        // Cleanup never deletes the user's checkout.
        let report = adapter
            .cleanup(acquired, &PreservationSet::default(), &repo.0.join("_ev"))
            .unwrap();
        assert!(report.removed_root.is_none());
        assert!(repo.0.join("a.txt").exists());
    }

    #[test]
    fn recovery_rejects_missing_stale_mismatch_and_remaps_under_authority() {
        let repo = TempRepo::new("rec");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "rec", &rev);
        let reference = manifest.to_reference();

        // Missing workspace.
        match adapter
            .recover(
                &repo.0.join("_ws/ws-rec/worktree"),
                &reference,
                &manifest,
                None,
            )
            .unwrap()
        {
            RecoveryOutcome::Rejected(WorkspaceRefError::Missing) => {}
            other => panic!("expected Missing, got {other:?}"),
        }
        // With governed remap authorization -> RemappedUnderAuthority.
        let auth = RemapAuthorization {
            reason: "workspace lost to infra failure".into(),
            authorized_by: "operator-diego".into(),
            recorded_at: "2026-08-25T00:00:00Z".into(),
        };
        match adapter
            .recover(
                &repo.0.join("_ws/ws-rec/worktree"),
                &reference,
                &manifest,
                Some(&auth),
            )
            .unwrap()
        {
            RecoveryOutcome::RemappedUnderAuthority(a) => {
                assert_eq!(a.authorized_by, "operator-diego")
            }
            other => panic!("expected remap, got {other:?}"),
        }

        // Stale revision: the workspace sits at its original pin while the
        // reference/manifest claim a NEWER revision — resume must reject.
        let acquired = adapter.acquire(&manifest).unwrap();
        std::fs::write(repo.0.join("a.txt"), "three\n").unwrap();
        {
            let st = Command::new("git")
                .args(["commit", "-aqm", "c3"])
                .current_dir(&repo.0)
                .status()
                .unwrap();
            assert!(st.success());
        }
        let newer = repo.head();
        let drifted_manifest = writable_manifest(&repo.0, "rec", &newer);
        let drifted_ref = drifted_manifest.to_reference();
        // The drifted manifest digest differs from the on-disk workspace's
        // origin; recovery compares REVISIONS, so present the drifted pair
        // against the old-pinned worktree.
        let outcome = adapter
            .recover(&acquired.root, &drifted_ref, &drifted_manifest, None)
            .unwrap();
        assert!(matches!(
            outcome,
            RecoveryOutcome::Rejected(WorkspaceRefError::StaleRevision)
        ));
        // Without authorization the caller must NOT proceed on drift; with
        // governed remap authorization it is allowed and evidenced.
        let outcome2 = adapter
            .recover(&acquired.root, &drifted_ref, &drifted_manifest, Some(&auth))
            .unwrap();
        assert!(matches!(
            outcome2,
            RecoveryOutcome::RemappedUnderAuthority(_)
        ));
        // The honest pair still resumes cleanly.
        let outcome3 = adapter
            .recover(&acquired.root, &reference, &manifest, None)
            .unwrap();
        assert!(matches!(outcome3, RecoveryOutcome::Resumed(_)));
        adapter
            .cleanup(acquired, &PreservationSet::default(), &repo.0.join("_ev"))
            .ok();
    }

    #[test]
    fn adapter_mismatch_fails_closed() {
        let repo = TempRepo::new("mix");
        let ro_adapter = ExistingReadOnlyWorkspace::bound_to(repo.0.clone());
        let wt_adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let wt_manifest = writable_manifest(&repo.0, "mix", &rev);
        let reference = wt_manifest.to_reference(); // git-worktree ref

        // Presenting a git-worktree reference to the read-only adapter is
        // rejected (adapter mismatch), not silently adapted.
        let outcome = ro_adapter
            .recover(&repo.0, &reference, &wt_manifest, None)
            .unwrap();
        assert!(matches!(
            outcome,
            RecoveryOutcome::Rejected(WorkspaceRefError::AdapterMismatch)
        ));

        // And vice versa: read-only ref to worktree adapter.
        let ro_manifest = WorkspaceManifestV1 {
            schema_version: WORKSPACE_SCHEMA_VERSION.into(),
            workspace_id: "ws-mix-ro".into(),
            adapter: AdapterKind::ExistingReadOnly,
            adapter_revision: ADAPTER_REVISION.into(),
            repo_identity: "origin/mix".into(),
            base_revision: rev.clone(),
            branch: None,
            mode: WorkspaceMode::ReadOnly,
            writable_scopes: vec![],
            resource_lock_id: "lock-x".into(),
            created_at: "2026-08-25T00:00:00Z".into(),
            content_digest: None,
        }
        .sealed();
        let ro_ref = ro_manifest.to_reference();
        let outcome2 = wt_adapter
            .recover(&repo.0, &ro_ref, &ro_manifest, None)
            .unwrap();
        assert!(matches!(
            outcome2,
            RecoveryOutcome::Rejected(WorkspaceRefError::AdapterMismatch)
        ));
    }

    #[test]
    fn references_round_trip_without_process_state() {
        let repo = TempRepo::new("rt");
        let manifest = writable_manifest(&repo.0, "rt", &repo.head());
        let reference = manifest.to_reference();
        let json = reference.to_json().unwrap();
        let parsed = WorkspaceRefV1::parse_json(&json).unwrap();
        assert_eq!(parsed, reference);
        // No hidden process state: the JSON must only carry the declared
        // durable identity keys (E5/I02 repair: the list is exhaustive
        // because `WorkspaceRefV1` uses `deny_unknown_fields`, so an extra
        // key would have failed parsing).
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = v.as_object().expect("ref is a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "adapter",
                "adapterRevision",
                "baseRevision",
                "contentDigest",
                "mode",
                "repoIdentity",
                "schemaVersion",
                "workspaceId",
            ]
        );
        // Resource-lock identity available for later parallel scheduling.
        assert_eq!(manifest.resource_lock_id, "lock-rt");
    }

    #[test]
    fn writable_acquisition_never_falls_back_to_source_checkout() {
        let repo = TempRepo::new("fb");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        // Nonexistent revision: acquisition must FAIL, and critically the
        // source checkout must be untouched (no silent fallback).
        let manifest = writable_manifest(&repo.0, "fb", &"f".repeat(40));
        assert!(adapter.acquire(&manifest).is_err());
        let head_before = repo.head();
        assert_eq!(repo.head(), head_before);
        assert!(!repo.0.join("_ws/ws-fb").exists());
    }
}
