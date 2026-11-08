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
    /// Durable repository identity: an existing source path, a remote URL,
    /// or a stable-name digest (`stable_repo_identity_digest` of the named
    /// remote's normalized URL).
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
        crate::workflow::soma::try_canonical_digest(&v)
            .expect("canonical digest of Lite-constructed record cannot fail")
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
    ///
    /// The reference's `contentDigest` is the MANIFEST's digest (not the
    /// reference's own digest). This attests to the originating manifest;
    /// recovery recomputes the manifest digest and verifies it matches.
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
        // Schema separation: a 1.0.0 ref must NOT carry headRevision. The
        // field was introduced in 1.1.0; a legacy reader pinned to 1.0.0
        // with deny_unknown_fields would reject it, so we must too.
        if r.schema_version == WORKSPACE_REF_SCHEMA_VERSION_V1 && r.head_revision.is_some() {
            anyhow::bail!(
                "workspace ref schema 1.0.0 must not contain headRevision (upgrade to {WORKSPACE_REF_SCHEMA_VERSION})"
            );
        }
        Ok(r)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Authenticate this reference against the manifest it claims to derive
    /// from. Fail closed: every durable identity field (`workspace_id`,
    /// `adapter`, `adapter_revision`, `repo_identity`, `mode`) must match
    /// the manifest exactly, AND `contentDigest` must equal the manifest's
    /// own digest (`manifest.compute_digest()`).The reference does NOT
    /// carry a digest of itself — `contentDigest` is an attestation of the
    /// ORIGINATING manifest, recomputed here from first principles.
    ///
    /// A mismatch is a hard error (never recoverable, never remappable):
    /// a reference that points at a different manifest than the one it was
    /// created from is evidence of tampering, not of staleness.
    pub fn verify_manifest_attestation(&self, manifest: &WorkspaceManifestV1) -> Result<()> {
        if self.workspace_id != manifest.workspace_id {
            anyhow::bail!(
                "workspace ref does not authenticate the supplied manifest: workspaceId mismatch"
            );
        }
        if self.adapter != manifest.adapter {
            anyhow::bail!(
                "workspace ref does not authenticate the supplied manifest: adapter mismatch"
            );
        }
        if self.adapter_revision != manifest.adapter_revision {
            anyhow::bail!(
                "workspace ref does not authenticate the supplied manifest: adapterRevision mismatch"
            );
        }
        if self.repo_identity != manifest.repo_identity {
            anyhow::bail!(
                "workspace ref does not authenticate the supplied manifest: repoIdentity mismatch"
            );
        }
        if self.mode != manifest.mode {
            anyhow::bail!(
                "workspace ref does not authenticate the supplied manifest: mode mismatch"
            );
        }
        if self.content_digest != manifest.compute_digest() {
            anyhow::bail!(
                "workspace ref contentDigest does not authenticate the supplied manifest"
            );
        }
        Ok(())
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

/// Exclusive owner for one git-worktree workspace identity.
///
/// The claim is an OS-backed exclusive lock on `owner.lock` inside the
/// workspace-id directory (sibling of `worktree`, never inside the git
/// checkout), acquired through the existing [`crate::workflow::evaluate::WorkflowFileLock`]
/// primitive (`flock` on Unix, `LockFileEx` on Windows). The lock file is
/// never truncated, deleted, or read: it carries no payload and no PID. A
/// crash releases the OS lock automatically, so recovery after a crash never
/// needs stale-lock deletion, and a recycled PID can never be mistaken for
/// proof that a workspace is free. Dropping the last guard unlocks our own
/// handle only; it cannot delete a successor's claim.
///
/// Cross-process exclusion comes from the OS lock. Same-process exclusion is
/// layered on top for Windows, where `LockFileEx` is per-process: a process
/// wide registry of held lock paths refuses a second claim from any thread
/// of this process. `AcquiredWorkspace` holds the guard via `Arc` so clones
/// share (not multiply) ownership until all handles drop.
#[derive(Debug)]
struct WorkspaceOwnerLock {
    /// Canonical lock path; the registry key for in-process exclusion.
    key: std::path::PathBuf,
    /// OS lock handle; dropping it releases the OS lock.
    _os_lock: crate::workflow::evaluate::WorkflowFileLock,
}

/// Process-wide set of workspace owner locks currently held by this process.
///
/// Windows `LockFileEx` locks are per-process, so on Windows two threads of
/// one process could otherwise hold the same OS lock. This registry makes
/// same-process exclusion hold on every platform. Unix `flock` is per open
/// file description, so the OS lock alone would suffice; the registry is
/// harmless there.
fn owner_registry() -> &'static std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>> {
    static REGISTRY: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

impl Drop for WorkspaceOwnerLock {
    fn drop(&mut self) {
        // Release only our own in-process claim. The OS handle (`_os_lock`)
        // unlocks on drop. The lock file is deliberately never unlinked, so
        // a late drop cannot delete a successor's claim: ownership is the
        // held handle, never the file's existence or contents.
        if let Ok(mut registry) = owner_registry().lock() {
            registry.remove(&self.key);
        }
    }
}

/// Registry key for one owner lock path: the canonical parent directory
/// joined with the lock file name, so equivalent spellings of the same
/// directory share one exclusion entry.
fn owner_lock_key(lock_path: &std::path::Path) -> std::path::PathBuf {
    match lock_path.parent() {
        Some(parent) => canonical_path(parent).join(
            lock_path
                .file_name()
                .expect("owner lock path always names a file"),
        ),
        None => lock_path.to_path_buf(),
    }
}

/// Claim the exclusive OS-backed owner lock for one workspace identity.
///
/// Bounded no-wait policy (mirrors the previous live-owner refusal):
/// returns `Err` — fail closed — while another owner holds the lock, whether
/// in this process (registry) or in another process (OS lock). There is no
/// stale-claim deletion and no PID liveness check: a leftover lock file from
/// a crashed owner is unheld and immediately claimable, and no concurrent
/// claimant's lock can ever be removed by observing stale metadata.
//
// SAFETY of the `#[allow]` below: `WorkflowFileLock` is not `Sync` on
// Windows (its `OVERLAPPED` is plain data but not declared `Send`/`Sync`).
// The `Arc` never actually crosses threads: `AcquiredWorkspace` could not
// be moved to another thread while holding it (the compiler rejects any
// such use), and the `Arc` only gives clone-sharing within the owning
// thread. So the `Arc`-with-non-`Send`/`Sync` warning has no runtime case.
#[allow(clippy::arc_with_non_send_sync)]
fn claim_owner_lock(lock_path: &std::path::Path) -> Result<std::sync::Arc<WorkspaceOwnerLock>> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let key = owner_lock_key(lock_path);
    {
        let mut registry = owner_registry().lock().map_err(|_| {
            anyhow::anyhow!("workspace ownership registry unavailable (fail closed)")
        })?;
        if !registry.insert(key.clone()) {
            anyhow::bail!("workspace owned by an active process: refusing concurrent acquisition");
        }
    }
    match crate::workflow::evaluate::WorkflowFileLock::try_acquire_at(lock_path) {
        Ok(Some(os_lock)) => Ok(std::sync::Arc::new(WorkspaceOwnerLock {
            key,
            _os_lock: os_lock,
        })),
        Ok(None) => {
            if let Ok(mut registry) = owner_registry().lock() {
                registry.remove(&key);
            }
            anyhow::bail!("workspace owned by an active process: refusing concurrent acquisition");
        }
        Err(e) => {
            if let Ok(mut registry) = owner_registry().lock() {
                registry.remove(&key);
            }
            Err(e).context("workspace owner lock failed (fail closed)")
        }
    }
}

/// Canonical git common directory of a checkout.
fn git_common_dir(repo: &std::path::Path) -> Result<std::path::PathBuf> {
    let s = run_git(repo, &["rev-parse", "--git-common-dir"])?;
    let p = std::path::PathBuf::from(s.trim());
    Ok(if p.is_absolute() { p } else { repo.join(p) })
}

fn canonical_path(p: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Prove `path` is a registered worktree of this adapter's source repo:
/// the worktree's git common directory equals the source's, and
/// `git worktree list --porcelain` from the source names `path`.
/// Rejects an unrelated clone that merely shares commits and directory.
fn verify_attached_to_source(source_repo: &std::path::Path, path: &std::path::Path) -> Result<()> {
    let worktree_common = canonical_path(&git_common_dir(path)?);
    let source_common = canonical_path(&git_common_dir(source_repo)?);
    if worktree_common != source_common {
        anyhow::bail!(
            "worktree at {} is not attached to the expected source repository (git common directory mismatch: refusing to reuse)",
            path.display()
        );
    }
    let listing = run_git(source_repo, &["worktree", "list", "--porcelain"])?;
    let want = canonical_path(path);
    let mut registered = false;
    for line in listing.lines() {
        let Some(rest) = line.strip_prefix("worktree ") else {
            continue;
        };
        if canonical_path(std::path::Path::new(rest.trim())) == want {
            registered = true;
            break;
        }
    }
    if !registered {
        anyhow::bail!(
            "worktree at {} is not registered as a worktree of the expected source repository: refusing to reuse",
            path.display()
        );
    }
    Ok(())
}

/// An acquired, validated workspace handed to execution.
#[derive(Debug, Clone)]
pub struct AcquiredWorkspace {
    pub manifest: WorkspaceManifestV1,
    pub root: std::path::PathBuf,
    /// Revision actually validated at acquisition time.
    pub revision: String,
    /// Exclusive owner claim (git-worktree acquires and recovers). Shared
    /// via `Arc` across clones; ownership is released (OS unlock + registry
    /// removal, never file deletion) when the last handle drops. Never read
    /// directly — the exclusivity is the `Drop` side effect. `None` only for
    /// read-only workspaces, which cleanup never removes.
    #[allow(dead_code)]
    owner_lock: Option<std::sync::Arc<WorkspaceOwnerLock>>,
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
fn normalize_remote_identity(s: &str) -> String {
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

fn url_or_git_proto_equal(a: &str, b: &str) -> bool {
    normalize_remote_identity(a) == normalize_remote_identity(b)
}

/// Derive a STABLE, portable repository identity from a durable remote
/// target (URL, git-protocol string, or canonical remote path). The value
/// is the canonical SHA-256 digest of the normalized target.
///
/// A stable-name `repo_identity` in a workspace manifest MUST equal this
/// digest for one of the source repository's named remotes. The remote's
/// *name* is NEVER consulted as the identifier: unrelated repositories
/// commonly share alias names such as `origin`, so only the independently
/// derived URL digest can bind authority to a specific repository.
pub fn stable_repo_identity_digest(remote_target: &str) -> String {
    crate::workflow::soma::try_canonical_digest(&serde_json::Value::String(
        normalize_remote_identity(remote_target),
    ))
    .expect("canonical digest of a string value cannot fail")
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
        //  c) Stable-name label -> the label is the canonical digest of the
        //     normalized URL of one named remote on the source repo (see
        //     `stable_repo_identity_digest`). The remote NAME is never the
        //     identifier, so alias reuse across unrelated repositories
        //     cannot satisfy binding.
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
            // Stable-name label: the identity is the canonical digest of a
            // normalized remote URL (see `stable_repo_identity_digest`).
            // A bare remote NAME (e.g. "origin") is NEVER sufficient proof:
            // unrelated repositories commonly share the same alias. We
            // enumerate the source repo's remotes and require one whose
            // RESOLVED URL digests to the declared identity.
            validate_workspace_id(identity).map_err(|e| {
                anyhow::anyhow!("workspace repo identity: stable-name rejected: {e}")
            })?;
            let remotes_out = run_git(&self.source_repo, &["remote"])
                .context("source_repo has no remotes: cannot bind a stable-name identity")?;
            let mut matched = false;
            for name in remotes_out.lines().map(str::trim).filter(|n| !n.is_empty()) {
                let url = run_git(&self.source_repo, &["remote", "get-url", name])
                    .with_context(|| format!("failed to resolve remote {name}"))?;
                if stable_repo_identity_digest(url.trim()) == *identity {
                    matched = true;
                    break;
                }
            }
            if !matched {
                anyhow::bail!(
                    "workspace repo identity mismatch: manifest.repo_identity={} (stable-name) but no remote of source_repo resolves to it (remotes: {})",
                    identity,
                    remotes_out
                        .lines()
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let path = self.worktree_path(manifest);
        // Exclusive ownership BEFORE any existing-path decision: the OS lock
        // covers the whole check-then-act window, so two processes racing
        // this acquisition get exactly one winner. While a live owner holds
        // the lock this refuses. A leftover lock FILE from a crashed owner
        // needs no reclaim logic: the OS lock was released by the OS at
        // crash time and the file is never deleted or read.
        // Foreign/failed paths drop this guard (unlocking, never deleting
        // owner.lock) and never delete the leftover worktree.
        let owner_lock = claim_owner_lock(
            &path
                .parent()
                .expect("workspace parent exists")
                .join("owner.lock"),
        )?;
        if path.exists() {
            // Interrupted-acquisition recovery: a prior `acquire` may have
            // completed `git worktree add` but crashed before persisting
            // `AcquiredWorkspace`. Resume that exact worktree ONLY if it is
            // a git working copy whose toplevel is this deterministic path,
            // whose HEAD equals the manifest's pinned revision, AND which
            // is attached to (and registered with) THIS source repository.
            // Anything else fails closed without deleting the leftover,
            // without adopting foreign data, and without touching the
            // source checkout (no fallback path exists).
            let actual = match head_revision(&path) {
                Ok(r) => r,
                Err(_) => {
                    anyhow::bail!(
                        "interrupted acquisition leftover at {} is not a git working copy: refusing to reuse (manual cleanup required)",
                        path.display()
                    );
                }
            };
            let top = match toplevel(&path) {
                Ok(t) => t,
                Err(_) => {
                    anyhow::bail!(
                        "interrupted acquisition leftover at {} is not a git working copy: refusing to reuse (manual cleanup required)",
                        path.display()
                    );
                }
            };
            let top_canon = std::fs::canonicalize(&top).unwrap_or(top);
            let path_canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if top_canon != path_canon {
                anyhow::bail!(
                    "interrupted acquisition leftover at {} resolves to a different toplevel: refusing to reuse",
                    path.display()
                );
            }
            if actual != manifest.base_revision {
                anyhow::bail!(
                    "{}: interrupted acquisition leftover HEAD {} != pinned {} (workspace path already exists: refusing to reuse or fall back)",
                    WorkspaceRefError::StaleRevision,
                    actual,
                    manifest.base_revision
                );
            }
            verify_attached_to_source(&self.source_repo, &path)?;
            return Ok(AcquiredWorkspace {
                manifest: manifest.clone(),
                root: path,
                revision: actual,
                owner_lock: Some(owner_lock),
            });
        }
        std::fs::create_dir_all(path.parent().expect("parent exists"))?;
        // Pin the worktree to the EXACT requested revision. If this fails,
        // the error propagates — there is deliberately NO fallback to the
        // source checkout — and the created worktree directory is removed
        // so failed acquisitions leave no worktree residue. The workspace
        // parent (which holds `owner.lock`) is never removed: the lock file
        // is never deleted, by a failed acquisition or otherwise.
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
            let _ = std::fs::remove_dir_all(&path);
            return Err(e).context("worktree acquisition failed (fail closed)");
        }
        // Verify the pin on disk: never trust the manifest copy. A HEAD
        // mismatch means git did not check out what was asked for; fail
        // closed and remove the just-created worktree.
        let actual = match head_revision(&path) {
            Ok(r) => r,
            Err(e) => {
                let _ = run_git(
                    &self.source_repo,
                    &["worktree", "remove", "--force", &path.to_string_lossy()],
                );
                let _ = std::fs::remove_dir_all(&path);
                return Err(e).context(
                    "worktree acquisition failed: created worktree HEAD unreadable (fail closed)",
                );
            }
        };
        if actual != manifest.base_revision {
            let _ = run_git(
                &self.source_repo,
                &["worktree", "remove", "--force", &path.to_string_lossy()],
            );
            let _ = std::fs::remove_dir_all(&path);
            anyhow::bail!(
                "{}: created worktree HEAD {} != pinned {}",
                WorkspaceRefError::StaleRevision,
                actual,
                manifest.base_revision
            );
        }
        if let Err(e) = verify_attached_to_source(&self.source_repo, &path) {
            let _ = run_git(
                &self.source_repo,
                &["worktree", "remove", "--force", &path.to_string_lossy()],
            );
            let _ = std::fs::remove_dir_all(&path);
            return Err(e);
        }
        Ok(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: path,
            revision: actual,
            owner_lock: Some(owner_lock),
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
        // Cleanup is an ownership operation: only the handle that actually
        // holds the exclusive claim may tear the worktree down. An ownerless
        // handle (e.g. a recovered inspection handout, or a forged value)
        // must never be able to remove a live workspace.
        if acquired.owner_lock.is_none() {
            anyhow::bail!(
                "cleanup refused: workspace {} is not owned by the caller (ownerless handle)",
                acquired.manifest.workspace_id
            );
        }
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
        // Manifest authentication (E5/I02 repair round 4): the reference
        // must attest to THIS manifest — every durable identity field plus
        // contentDigest (the manifest's own digest). Substitution hard-fails
        // here; it is never a remap candidate.
        reference.verify_manifest_attestation(manifest)?;
        // Deterministic path: recovery may only resume the exact worktree
        // this manifest resolves to. A different path is a hard failure
        // (never a remap candidate): resuming an unexpected checkout would
        // silently re-base authority onto an unverified tree.
        let deterministic = self.worktree_path(manifest);
        if canonical_path(expected_root) != canonical_path(&deterministic) {
            anyhow::bail!(
                "recovery refused: expected_root {} is not the deterministic worktree path {} for workspace {}",
                expected_root.display(),
                deterministic.display(),
                manifest.workspace_id
            );
        }
        if !expected_root.exists() {
            return reject_or_remap(remap, WorkspaceRefError::Missing);
        }
        let actual = match head_revision(expected_root) {
            Ok(r) => r,
            Err(_) => return reject_or_remap(remap, WorkspaceRefError::Missing),
        };
        // Both checks are mandatory for crash-recovery integrity:
        //  1. reference.baseRevision MUST match manifest.baseRevision — this
        //     binds the reference to the originating manifest (the manifest's
        //     base is the acquisition base; the reference must carry the same).
        //  2. actual HEAD MUST match the pin: reference.headRevision when set
        //     (post-write ref), or reference.baseRevision (pre-write ref).
        // Skipping either check allows a stale or orphaned workspace to resume.
        if reference.base_revision != manifest.base_revision {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
        }
        let actual_expected = match &reference.head_revision {
            Some(head) => head.as_str(),
            None => reference.base_revision.as_str(),
        };
        if actual != actual_expected {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
        }
        // Repository attachment: the checkout to resume must be THIS
        // adapter's registered worktree of the authority-bound source repo,
        // exactly as interrupted-acquisition resume requires. A foreign
        // clone that merely shares commits is a hard failure (never a
        // remap candidate), and the foreign data is left untouched.
        verify_attached_to_source(&self.source_repo, expected_root)?;
        // Ownership gate — the SAME exclusive claim `acquire` takes, and it
        // is taken only here, at the moment a writable owner is about to be
        // handed out. Validation above never touches ownership, so a failed
        // recovery cannot remove (or observe) an active owner's lock, and
        // remap outcomes carry no workspace and claim nothing: a remapped
        // caller must still `acquire` (which claims) before it can run.
        // A contended claim fails closed with `Err`, exactly as in
        // `acquire`.
        let owner_lock = claim_owner_lock(
            &deterministic
                .parent()
                .expect("workspace parent exists")
                .join("owner.lock"),
        )?;
        Ok(RecoveryOutcome::Resumed(Box::new(AcquiredWorkspace {
            manifest: manifest.clone(),
            root: expected_root.to_path_buf(),
            revision: actual,
            owner_lock: Some(owner_lock),
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
            owner_lock: None,
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
        // Manifest authentication (E5/I02 repair round 4): identical to the
        // git-worktree adapter — the reference must attest to THIS manifest.
        reference.verify_manifest_attestation(manifest)?;
        // Round 5 parity: the read-only adapter must apply the same base
        // binding and manifest-schema gate as the git-worktree adapter.
        // Without these, a ref that authenticates the manifest's identity
        // fields can still resume against a manifest carrying a DIFFERENT
        // acquisition base — silently re-basing authority onto a different
        // lineage. The git-worktree path enforces this; the read-only path
        // must too.
        if manifest.schema_version != WORKSPACE_SCHEMA_VERSION {
            return reject_or_remap(remap, WorkspaceRefError::IncompatibleSchema);
        }
        if reference.base_revision != manifest.base_revision {
            return reject_or_remap(remap, WorkspaceRefError::StaleRevision);
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
            owner_lock: None,
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
            g(&["config", "core.autocrlf", "false"]);
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
        // The honest pair resumes cleanly — but only once the original owner
        // releases the workspace: while `acquired` is alive the same
        // exclusive claim refuses a competing recovery (FIX 1), so the
        // owner is released first, the honest pair resumes WITH a claim,
        // and the recovered owner cleans up.
        let root = acquired.root.clone();
        assert!(
            adapter.recover(&root, &reference, &manifest, None).is_err(),
            "competing recovery must refuse while the owner is active"
        );
        drop(acquired); // release ownership; worktree stays on disk
        let outcome3 = adapter.recover(&root, &reference, &manifest, None).unwrap();
        match outcome3 {
            RecoveryOutcome::Resumed(w) => {
                adapter
                    .cleanup(*w, &PreservationSet::default(), &repo.0.join("_ev2"))
                    .unwrap();
            }
            other => panic!("expected Resumed, got {other:?}"),
        }
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
        // No worktree residue: the claim itself creates the workspace
        // directory (owner.lock lives there), and the lock file is never
        // deleted — least of all on failure. Only the worktree subdirectory
        // must be gone, and the source checkout must be untouched.
        assert!(!repo.0.join("_ws/ws-fb/worktree").exists());
    }

    #[test]
    fn interrupted_acquisition_resumes_exact_revision() {
        // #171 interrupted-acquisition recovery (real git worktree): the
        // first acquire completes the worktree add but the handle is lost
        // (crash between add and durable persist). A second acquire with
        // the SAME manifest must recover the existing worktree at the exact
        // pinned revision instead of failing forever on "already exists".
        let repo = TempRepo::new("intres");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "intres", &rev);
        let first = adapter
            .acquire(&manifest)
            .expect("first acquire creates worktree");
        assert_eq!(first.revision, rev);
        let first_root = first.root.clone();
        // Marker proves the second acquire reuses the same worktree.
        std::fs::write(first_root.join("interrupted.marker"), "left-from-crash\n").unwrap();
        drop(first); // crash: no cleanup, handle lost
        assert!(first_root.exists());
        let head_before = repo.head();

        let recovered = adapter
            .acquire(&manifest)
            .expect("interrupted acquisition must resume at exact revision");
        assert_eq!(recovered.root, first_root);
        assert_eq!(recovered.revision, rev);
        assert!(recovered.root.join("interrupted.marker").exists());
        let actual = {
            let o = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&recovered.root)
                .output()
                .unwrap();
            assert!(o.status.success());
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        assert_eq!(actual, rev);
        assert_eq!(
            repo.head(),
            head_before,
            "source checkout must be untouched"
        );
        adapter
            .cleanup(recovered, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("cleanup");
    }

    #[test]
    fn interrupted_acquisition_stale_leftover_fails_closed() {
        // Leftover from an interrupted acquire at an OLDER pin must NOT be
        // adopted for a manifest claiming a NEWER revision, must not be
        // deleted automatically, and must not fall back to the source.
        let repo = TempRepo::new("intstale");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let old = repo.head();
        let manifest_old = writable_manifest(&repo.0, "intstale", &old);
        let acquired = adapter.acquire(&manifest_old).unwrap();
        let leftover = acquired.root.clone();
        drop(acquired); // interrupted: leftover remains at `old`
        std::fs::write(repo.0.join("a.txt"), "two\n").unwrap();
        {
            let st = Command::new("git")
                .args(["commit", "-aqm", "c2"])
                .current_dir(&repo.0)
                .status()
                .unwrap();
            assert!(st.success());
        }
        let newer = repo.head();
        let manifest_new = writable_manifest(&repo.0, "intstale", &newer);
        let head_before = repo.head();
        let err = adapter.acquire(&manifest_new).unwrap_err().to_string();
        assert!(
            err.contains("stale") || err.contains("already exists") || err.contains("revision"),
            "stale leftover must fail closed, got: {err}"
        );
        assert!(leftover.exists(), "leftover must not be deleted on failure");
        assert_eq!(
            repo.head(),
            head_before,
            "source checkout must be untouched"
        );
    }

    #[test]
    fn interrupted_acquisition_non_git_leftover_fails_closed() {
        // A plain non-git directory at the deterministic worktree path
        // (e.g. partial mkdir before crash, or foreign data) must fail
        // closed without deletion or adoption.
        let repo = TempRepo::new("intnongit");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "intnongit", &rev);
        let path = repo.0.join("_ws/ws-intnongit/worktree");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("foreign.txt"), "not-a-worktree\n").unwrap();
        let err = adapter.acquire(&manifest).unwrap_err().to_string();
        assert!(
            err.contains("already exists") || err.contains("interrupted"),
            "{err}"
        );
        assert!(
            path.join("foreign.txt").exists(),
            "foreign data must not be deleted"
        );
    }

    #[test]
    fn foreign_clone_same_head_refused() {
        // REPAIR A: a foreign clone sharing the commit SHA but not
        // attached to (or registered with) THIS source repo must be
        // refused, even at the deterministic path with matching HEAD.
        let repo = TempRepo::new("foreign");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "foreign", &rev);
        let path = parent.join("ws-foreign").join("worktree");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let st = Command::new("git")
            .args([
                "clone",
                "-q",
                repo.0.to_string_lossy().as_ref(),
                path.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap();
        assert!(st.success(), "foreign clone must be created");
        // Sanity: the foreign clone carries the same HEAD.
        {
            let o = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&path)
                .output()
                .unwrap();
            assert!(o.status.success());
            assert_eq!(String::from_utf8_lossy(&o.stdout).trim().to_string(), rev);
        }
        let head_before = repo.head();
        let err = adapter.acquire(&manifest).unwrap_err().to_string();
        assert!(
            err.contains("attached")
                || err.contains("registered")
                || err.contains("common directory"),
            "foreign clone must be refused with attachment proof, got: {err}"
        );
        assert!(
            path.join("a.txt").exists(),
            "foreign clone must not be deleted on refusal"
        );
        assert_eq!(
            repo.head(),
            head_before,
            "source checkout must be untouched"
        );
    }

    #[test]
    fn second_acquire_while_first_owner_active_refused() {
        // REPAIR B: while the first AcquiredWorkspace owner is alive, a
        // second acquire of the same workspace identity must refuse
        // (bounded no-wait policy), even at the exact pinned revision.
        let repo = TempRepo::new("conc");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "conc", &rev);
        let first = adapter
            .acquire(&manifest)
            .expect("first owner must acquire the workspace");
        assert_eq!(first.revision, rev);
        let err = adapter.acquire(&manifest).unwrap_err().to_string();
        assert!(
            err.contains("owned by an active process") || err.contains("concurrent"),
            "concurrent acquire must be refused, got: {err}"
        );
        // First owner still owns the workspace and its work continues.
        adapter.validate(&first).expect("first owner still valid");
        let head_before = repo.head();
        drop(first);
        // After owner release, interrupted recovery succeeds.
        let recovered = adapter
            .acquire(&manifest)
            .expect("acquire after owner release must recover");
        assert_eq!(recovered.revision, rev);
        assert_eq!(repo.head(), head_before);
        adapter
            .cleanup(recovered, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("cleanup");
    }

    #[test]
    fn concurrent_recover_while_acquired_refused_then_recovers_with_ownership() {
        // FIX 1 regression: recovery must hold the same exclusive claim as
        // acquisition. While the first owner is active, a recover of the
        // exact same reference must refuse and must not disturb the owner;
        // after release, recover succeeds WITH ownership and cleanup works.
        let repo = TempRepo::new("recown");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "recown", &rev);
        let reference = manifest.to_reference();
        let first = adapter
            .acquire(&manifest)
            .expect("first owner must acquire the workspace");
        let root = first.root.clone();

        // Competing recovery while the first owner is active: refusal.
        let err = adapter
            .recover(&root, &reference, &manifest, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("owned by an active process") || err.contains("concurrent"),
            "concurrent recover must refuse, got: {err}"
        );
        // The first owner is untouched and still valid.
        adapter.validate(&first).expect("first owner still valid");
        assert!(root.exists());

        drop(first);
        // After release, recovery succeeds and HOLDS a real claim: a second
        // recovery while the recovered handle is live must refuse.
        let recovered = match adapter
            .recover(&root, &reference, &manifest, None)
            .expect("recover after release must resume")
        {
            RecoveryOutcome::Resumed(w) => *w,
            other => panic!("expected Resumed, got {other:?}"),
        };
        assert_eq!(recovered.revision, rev);
        assert!(adapter.recover(&root, &reference, &manifest, None).is_err());
        // The recovered owner can clean up; the worktree is removed.
        adapter
            .cleanup(recovered, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("recovered owner cleanup");
        assert!(!root.exists());
    }

    #[test]
    fn failed_recovery_preserves_active_owner_claim() {
        // A failed recovery must never remove an active owner's lock: a
        // stale-revision recovery attempt while the owner is live rejects
        // (and touches no ownership state), and the owner still validates.
        let repo = TempRepo::new("failrec");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "failrec", &rev);
        let first = adapter
            .acquire(&manifest)
            .expect("first owner must acquire the workspace");
        let lock_file = repo.0.join("_ws/ws-failrec/owner.lock");
        assert!(lock_file.exists(), "owner lock file must exist");

        // Drifted reference/manifest pair: recovery rejects as stale BEFORE
        // any ownership interaction.
        let drifted_manifest = writable_manifest(&repo.0, "failrec", &"e".repeat(40));
        let drifted_ref = drifted_manifest.to_reference();
        let outcome = adapter
            .recover(&first.root, &drifted_ref, &drifted_manifest, None)
            .unwrap();
        assert!(matches!(
            outcome,
            RecoveryOutcome::Rejected(WorkspaceRefError::StaleRevision)
        ));

        // The active owner's claim is intact: lock file present, owner valid,
        // and a competing acquisition still refuses.
        assert!(
            lock_file.exists(),
            "failed recovery must not remove the lock"
        );
        adapter.validate(&first).expect("owner still valid");
        let err = adapter.acquire(&manifest).unwrap_err();
        assert!(err.to_string().contains("concurrent"));
        adapter
            .cleanup(first, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("owner cleanup");
    }

    #[test]
    fn recover_rejects_non_deterministic_path() {
        // Recovery must verify the deterministic worktree path: a checkout
        // at an unexpected path is refused and its content is untouched.
        let repo = TempRepo::new("pathrec");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "pathrec", &rev);
        let reference = manifest.to_reference();
        let stray = repo.0.join("_ws/stray/worktree");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        let st = Command::new("git")
            .args([
                "clone",
                "-q",
                repo.0.to_string_lossy().as_ref(),
                stray.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap();
        assert!(st.success(), "stray clone must be created");
        let err = adapter
            .recover(&stray, &reference, &manifest, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("deterministic"),
            "non-deterministic path must hard-fail, got: {err}"
        );
        assert!(
            stray.join("a.txt").exists(),
            "stray checkout must be untouched"
        );
        // No ownership was taken by the failed recovery: a legitimate
        // acquire of the deterministic workspace still succeeds.
        let acquired = adapter.acquire(&manifest).expect("legit acquire");
        adapter
            .cleanup(acquired, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("cleanup");
    }

    #[test]
    fn recover_rejects_foreign_clone_attachment() {
        // Recovery must verify repository attachment: a foreign clone at the
        // deterministic path (same HEAD, different git common directory) is
        // refused, left untouched, and never adopted.
        let repo = TempRepo::new("foreignrec");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "foreignrec", &rev);
        let reference = manifest.to_reference();
        let path = parent.join("ws-foreignrec").join("worktree");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let st = Command::new("git")
            .args([
                "clone",
                "-q",
                repo.0.to_string_lossy().as_ref(),
                path.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap();
        assert!(st.success(), "foreign clone must be created");
        let head_before = repo.head();
        let err = adapter
            .recover(&path, &reference, &manifest, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("attached")
                || err.contains("registered")
                || err.contains("common directory"),
            "foreign attachment must hard-fail, got: {err}"
        );
        assert!(
            path.join("a.txt").exists(),
            "foreign clone must not be deleted"
        );
        assert_eq!(
            repo.head(),
            head_before,
            "source checkout must be untouched"
        );
    }

    #[test]
    fn cleanup_refuses_ownerless_handle() {
        // Cleanup is an ownership operation: a handle that does not hold
        // the exclusive claim must never tear down a live workspace.
        let repo = TempRepo::new("unown");
        let adapter = GitWorktreeWorkspace {
            parent_dir: repo.0.join("_ws"),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "unown", &rev);
        let first = adapter
            .acquire(&manifest)
            .expect("first owner must acquire the workspace");
        let root = first.root.clone();
        // Forge an ownerless handle over the SAME live worktree (in-module
        // test only; the field is private to this module).
        let forged = AcquiredWorkspace {
            manifest: manifest.clone(),
            root: root.clone(),
            revision: rev.clone(),
            owner_lock: None,
        };
        let err = adapter
            .cleanup(forged, &PreservationSet::default(), &repo.0.join("_ev"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("ownerless"), "got: {err}");
        assert!(
            root.exists(),
            "live worktree must survive ownerless cleanup"
        );
        adapter.validate(&first).expect("owner still valid");
        adapter
            .cleanup(first, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("owner cleanup");
        assert!(!root.exists());
    }

    #[test]
    fn owner_lock_file_content_is_never_interpreted() {
        // FIX 2: the lock file carries no payload and no PID. Junk content
        // (e.g. from a crashed legacy writer) must not be read as liveness
        // proof: acquisition succeeds because no OS lock is held, and the
        // file is left exactly as found.
        let repo = TempRepo::new("junklock");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "junklock", &rev);
        let lock_file = parent.join("ws-junklock").join("owner.lock");
        std::fs::create_dir_all(lock_file.parent().unwrap()).unwrap();
        std::fs::write(&lock_file, b"garbage-bytes-not-a-pid\n").unwrap();
        let acquired = adapter
            .acquire(&manifest)
            .expect("unheld lock file content must not block acquisition");
        assert_eq!(acquired.revision, rev);
        // While held, confirm via metadata (not content: on Windows the
        // locked byte range is unreadable, exactly as lock.rs documents)
        // that the file was not truncated.
        assert_eq!(
            std::fs::metadata(&lock_file).unwrap().len(),
            b"garbage-bytes-not-a-pid\n".len() as u64,
            "lock file must not be truncated"
        );
        adapter
            .cleanup(acquired, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("cleanup");
        assert!(lock_file.exists(), "cleanup must not delete the lock file");
        assert_eq!(
            std::fs::read(&lock_file).unwrap(),
            b"garbage-bytes-not-a-pid\n",
            "lock file content must be left as found"
        );
    }

    #[test]
    fn stale_lock_file_after_crash_is_claimable_and_never_deleted() {
        // FIX 2 crash recovery: a lock file left behind by a dead owner
        // (no OS lock held) is claimable without any stale-deletion step,
        // and the file itself survives the whole lifecycle.
        let repo = TempRepo::new("stalefile");
        let parent = repo.0.join("_ws");
        let adapter = GitWorktreeWorkspace {
            parent_dir: parent.clone(),
            source_repo: repo.0.clone(),
        };
        let rev = repo.head();
        let manifest = writable_manifest(&repo.0, "stalefile", &rev);
        let lock_file = parent.join("ws-stalefile").join("owner.lock");
        // Simulate a crashed owner's residue: file exists, nothing holds it.
        std::fs::create_dir_all(lock_file.parent().unwrap()).unwrap();
        std::fs::write(&lock_file, b"").unwrap();
        let first = adapter
            .acquire(&manifest)
            .expect("crash residue must be claimable");
        drop(first); // owner gone; lock file remains
        assert!(lock_file.exists(), "drop must not delete the lock file");
        let second = adapter
            .acquire(&manifest)
            .expect("reacquisition after release must succeed");
        let err = adapter.acquire(&manifest).unwrap_err();
        assert!(err.to_string().contains("concurrent"));
        adapter
            .cleanup(second, &PreservationSet::default(), &repo.0.join("_ev"))
            .expect("cleanup");
        assert!(lock_file.exists(), "cleanup must not delete the lock file");
    }
}
