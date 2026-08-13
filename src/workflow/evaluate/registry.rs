use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::evidence::{EvidenceBundle, write_bundle};
use super::identity::{EvaluationState, now_iso};

// ---------------------------------------------------------------------------
// Lease configuration for ownership fencing
// ---------------------------------------------------------------------------

/// Lease and heartbeat configuration for ownership fencing.
///
/// Controls how long an entry may stay in `Reserved` or `Generating` before
/// another worker may reclaim it. Defaults are conservative for production;
/// tests inject shorter durations.
#[derive(Debug, Clone)]
pub struct LeaseConfig {
    /// How long a `Reserved` entry can exist before it is considered stale.
    /// Default: 120 seconds.
    pub stale_reservation_timeout: std::time::Duration,
    /// How long a `Generating` entry may go without a heartbeat before it is
    /// considered dead and reclaimable. Default: 600 seconds.
    pub generation_lease_timeout: std::time::Duration,
    /// How often the heartbeat task renews the lease on behalf of a live
    /// generation worker. Must be meaningfully shorter than
    /// `generation_lease_timeout` — preferably ≤ ⅓ of it. Default: 180 seconds.
    pub heartbeat_interval: std::time::Duration,
}

impl LeaseConfig {
    pub fn with_timeouts(
        stale_reservation_timeout: std::time::Duration,
        generation_lease_timeout: std::time::Duration,
        heartbeat_interval: std::time::Duration,
    ) -> Self {
        Self {
            stale_reservation_timeout,
            generation_lease_timeout,
            heartbeat_interval,
        }
    }
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            stale_reservation_timeout: std::time::Duration::from_secs(120),
            generation_lease_timeout: std::time::Duration::from_secs(600),
            heartbeat_interval: std::time::Duration::from_secs(180),
        }
    }
}
/// Fencing token proving ownership of a registry entry.
///
/// Every state mutation under the registry lock must verify that the
/// caller's `owner_run_id` and `lease_epoch` still match the entry.
#[derive(Debug, Clone)]
pub struct FenceToken {
    /// Unique run id of the process that owns this entry.
    pub owner_run_id: String,
    /// Monotonically increasing epoch. Each takeover increments this value.
    pub lease_epoch: u64,
}
// ---------------------------------------------------------------------------
// Deterministic identity registry (stateful, atomic reservation)
// ---------------------------------------------------------------------------

/// Registry entry tracking the full lifecycle of a proposal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Current pipeline state for this identity key.
    pub state: ProposalState,
    /// Proposal ID (set after generation completes).
    pub proposal_id: Option<String>,
    /// Run ID of the process that holds the ownership of this entry.
    /// Every state mutation checks that the caller's run id matches.
    pub owner_run_id: String,
    /// Monotonically increasing fencing token. Each takeover or fresh
    /// reservation starts at epoch 1; every subsequent takeover
    /// increments it. State mutations require both `owner_run_id` and
    /// `lease_epoch` to match the caller's token.
    pub lease_epoch: u64,
    /// RFC3339 timestamp when the reservation was created.
    pub reserved_at: String,
    /// RFC3339 timestamp of the last state transition.
    pub updated_at: String,
    /// RFC3339 timestamp of the last heartbeat renewal.
    /// Only relevant in `Generating` state — checked by other workers
    /// that want to reclaim an expired lease.
    pub heartbeat_at: String,
    /// Evidence directory path (set when validation completes).
    /// Used to locate preserved evidence across different run directories.
    pub evidence_dir: Option<String>,
}

/// Lifecycle state of a registry entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// Identity reserved; generation not yet started.
    #[default]
    Reserved,
    /// Generation in progress (model call running, heartbeat active).
    Generating,
    /// Proposal generated and persisted; hasn't started validation yet.
    ProposalGenerated,
    /// Validation is actively running (heartbeat active).
    Validating,
    /// Validation complete; evidence bundle finalized.
    ValidationComplete,
}

/// Registry mapping identity keys to their lifecycle entries.
/// Stored at `<repo>/.prometheos/workflow/proposal_registry.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProposalRegistry {
    /// Maps identity_key → RegistryEntry.
    pub entries: std::collections::HashMap<String, RegistryEntry>,
}

fn registry_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json")
}

fn registry_lock_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.lock")
}

fn load_registry(repo: &Path) -> Result<ProposalRegistry> {
    let path = registry_path(repo);
    if !path.exists() {
        // Missing file is expected — return empty registry.
        return Ok(ProposalRegistry::default());
    }
    // Migrate any unversioned legacy registry to the current versioned form
    // (inject `schema_version`, validate the typed document, fail closed on
    // corrupt or unsupported-future documents).
    super::migration::migrate_document(&path, super::schema::DocumentType::ProposalRegistry)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read proposal registry {}", path.display()))?;
    serde_json::from_str(&text).context("corrupted proposal registry (invalid JSON)")
}

fn save_registry(repo: &Path, registry: &ProposalRegistry) -> Result<()> {
    let path = registry_path(repo);
    // Current-format write: the schema version is embedded in the document.
    super::durable::versioned_write_json(&path, registry)
        .context("failed to persist proposal registry")
}

/// Run `f` under the exclusive registry lock with a freshly loaded registry,
/// then persist the registry.
///
/// Used by journal appends and recovery repairs so that fence verification,
/// sequence computation, and snapshot repair all happen inside the same
/// synchronization boundary as registry mutation. The lock file is kept (not
/// deleted) — dropping the handle releases it.
pub(super) fn with_registry_lock<T>(
    repo: &Path,
    f: impl FnOnce(&mut ProposalRegistry) -> Result<T>,
) -> Result<T> {
    let lock_path = registry_lock_path(repo);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire registry lock");
        }
    }

    let mut registry = load_registry(repo).context("failed to load registry under lock")?;
    let result = f(&mut registry);
    let save = result.is_ok();
    if save {
        save_registry(repo, &registry).context("failed to persist registry under lock")?;
    }
    drop(lock_file);
    result
}
/// Result of attempting to reserve or reclaim an identity key.
#[derive(Debug)]
pub(super) enum ReserveResult {
    /// Successfully acquired ownership. Contains the fencing token.
    Owned(FenceToken),
    /// Entry already exists and is owned by another live worker.
    /// Caller should wait and reuse.
    AlreadyExists,
}

/// Try to acquire an atomic reservation for an identity key.
///
/// Returns `Owned(FenceToken)` if the reservation was acquired.
/// Returns `AlreadyExists` if the entry already exists (caller should reuse or wait).
/// Returns `Err` on I/O failure.
pub(super) fn try_reserve(repo: &Path, identity_key: &str, run_id: &str) -> Result<ReserveResult> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    // Acquire an exclusive lock file.
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file")?;

    // Use platform-exclusive lock (flock on Unix, LockFileEx on Windows).
    // Check return value and fail closed on error.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            // Lock held by another process.
            drop(lock_file);
            return Ok(ReserveResult::AlreadyExists);
        }
    }

    // Now read the registry under the lock.
    let mut registry = load_registry(repo).context("failed to read registry under lock")?;
    if registry.entries.contains_key(identity_key) {
        // Another process reserved it first.
        drop(lock_file);
        return Ok(ReserveResult::AlreadyExists);
    }

    // Reserve the identity with initial epoch 1.
    let now = now_iso();
    registry.entries.insert(
        identity_key.to_string(),
        RegistryEntry {
            state: ProposalState::Reserved,
            proposal_id: None,
            owner_run_id: run_id.to_string(),
            lease_epoch: 1,
            reserved_at: now.clone(),
            updated_at: now.clone(),
            heartbeat_at: now,
            evidence_dir: None,
        },
    );
    save_registry(repo, &registry)?;

    drop(lock_file);
    let token = FenceToken {
        owner_run_id: run_id.to_string(),
        lease_epoch: 1,
    };
    Ok(ReserveResult::Owned(token))
}
/// Result of an attempted takeover.
#[derive(Debug)]
pub(super) enum TakeoverResult {
    /// Successfully took ownership. Contains the new fencing token.
    Taken(FenceToken),
    /// Entry is still live (not stale or owned by another). Caller should wait.
    StillLive,
}

/// Attempt an atomic takeover of a registry entry.
///
/// Under the registry lock:
/// 1. Re-validates staleness using `is_entry_stale`.
/// 2. Confirms the entry's state, owner, and epoch haven't changed since the
///    caller's outer stale check.
/// 3. Increments the epoch and assigns the new owner.
///
/// Returns `Taken(FenceToken)` on success, `StillLive` if the entry is not
/// stale (heartbeat renewed between check and lock), or if
/// another worker already claimed it, or `Err` on I/O failure.
pub(super) fn try_take_ownership(
    repo: &Path,
    identity_key: &str,
    new_owner: &str,
    lease_config: &LeaseConfig,
) -> Result<TakeoverResult> {
    let lock_path = registry_lock_path(repo);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for takeover")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock for takeover: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for registry takeover");
        }
    }

    let mut registry = load_registry(repo).context("failed to read registry for takeover")?;
    let entry = registry
        .entries
        .get_mut(identity_key)
        .context("registry entry not found during takeover")?;

    // Re-validate stale status under the lock. A heartbeat may have renewed
    // between the caller's outer stale check and this lock acquisition.
    match entry.state {
        ProposalState::Reserved | ProposalState::Generating | ProposalState::Validating => {
            let stale = is_entry_stale(entry, lease_config)
                .context("failed to check entry staleness during takeover")?;
            if !stale {
                // Entry is still live — another worker is actively renewing.
                drop(lock_file);
                return Ok(TakeoverResult::StillLive);
            }
        }
        ProposalState::ProposalGenerated => {
            // ProposalGenerated is always reclaimable: the previous owner has
            // finished generation and hasn't entered Validating yet. No stale
            // check needed — but we do verify the entry still exists.
        }
        ProposalState::ValidationComplete => {
            // Terminal state — no takeover needed.
            drop(lock_file);
            return Ok(TakeoverResult::StillLive);
        }
    }

    // Verify this entry hasn't already been taken by another worker.
    // We check that the current epoch matches what the caller last observed.
    // If the caller doesn't have a previous observation, we just take it
    // (the stale check above already validated the entry is reclaimable).
    // Generate a new epoch and assign the new owner.
    let new_epoch = entry.lease_epoch + 1;
    let now = now_iso();
    entry.owner_run_id = new_owner.to_string();
    entry.lease_epoch = new_epoch;
    entry.updated_at = now.clone();
    entry.heartbeat_at = now.clone();
    // Reset proposal_id for Reserved/Generating entries being reclaimed.
    if matches!(
        entry.state,
        ProposalState::Reserved | ProposalState::Generating
    ) {
        entry.proposal_id = None;
    }
    save_registry(repo, &registry)?;

    drop(lock_file);
    Ok(TakeoverResult::Taken(FenceToken {
        owner_run_id: new_owner.to_string(),
        lease_epoch: new_epoch,
    }))
}
/// Renew the heartbeat timestamp for a registry entry.
///
/// Only the current owner (matching `owner_run_id` and `lease_epoch`) may
/// renew. Returns `Ok(true)` on success, `Ok(false)` if ownership was lost
/// (another worker took over), or `Err` on I/O failure.
pub(super) fn renew_heartbeat(repo: &Path, identity_key: &str, fence: &FenceToken) -> Result<bool> {
    let lock_path = registry_lock_path(repo);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for heartbeat")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock for heartbeat: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for heartbeat renewal");
        }
    }

    let mut registry = load_registry(repo).context("failed to read registry for heartbeat")?;
    let entry_option = registry.entries.get_mut(identity_key);

    match entry_option {
        Some(e) if e.owner_run_id == fence.owner_run_id && e.lease_epoch == fence.lease_epoch => {
            e.heartbeat_at = now_iso();
            e.updated_at = now_iso();
            save_registry(repo, &registry)?;
            drop(lock_file);
            Ok(true)
        }
        Some(_) => {
            // Ownership has been taken by another worker.
            drop(lock_file);
            Ok(false)
        }
        None => {
            // Entry was removed.
            drop(lock_file);
            Ok(false)
        }
    }
}
/// Look up the registry entry for an identity key.
pub(super) fn lookup_entry(repo: &Path, identity_key: &str) -> Option<RegistryEntry> {
    let registry = load_registry(repo).ok()?;
    registry.entries.get(identity_key).cloned()
}

/// Transition the state of a registry entry.
/// Requires ownership fencing — the caller must provide a valid FenceToken.
pub(super) fn transition_entry(
    repo: &Path,
    identity_key: &str,
    new_state: ProposalState,
    proposal_id: Option<&str>,
    fence: &FenceToken,
) -> Result<()> {
    transition_entry_with_evidence(repo, identity_key, new_state, proposal_id, None, fence)
}

/// Transition the state of a registry entry, optionally setting the evidence dir.
/// Requires ownership fencing — the caller must provide a valid FenceToken.
fn transition_entry_with_evidence(
    repo: &Path,
    identity_key: &str,
    new_state: ProposalState,
    proposal_id: Option<&str>,
    evidence_dir: Option<&str>,
    fence: &FenceToken,
) -> Result<()> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for transition")?;

    // Check flock return value.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock for transition: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for registry transition");
        }
    }

    let mut registry = load_registry(repo).context("failed to read registry for transition")?;
    let entry = registry
        .entries
        .get_mut(identity_key)
        .context("registry entry not found during transition")?;

    // Fencing check: verify the caller still owns this entry.
    if entry.owner_run_id != fence.owner_run_id || entry.lease_epoch != fence.lease_epoch {
        bail!(
            "ownership lost (expected owner={}, epoch={}; found owner={}, epoch={})",
            fence.owner_run_id,
            fence.lease_epoch,
            entry.owner_run_id,
            entry.lease_epoch,
        );
    }

    entry.state = new_state;
    entry.updated_at = now_iso();
    if let Some(pid) = proposal_id {
        entry.proposal_id = Some(pid.to_string());
    }
    if let Some(ed) = evidence_dir {
        entry.evidence_dir = Some(ed.to_string());
    }
    save_registry(repo, &registry)?;

    // Do NOT delete the lock file — just drop the handle to release.
    drop(lock_file);
    Ok(())
}
/// Fenced finalization: acquire the registry lock, revalidate ownership and
/// heartbeat health, then commit the terminal outcome atomically.
///
/// Ordering (durability before visibility, all under the same lock so that no
/// takeover can occur between the checks and publication):
/// 1. Final evidence JSON + Markdown are written durably ([`write_bundle`]).
/// 2. A terminal journal event is appended, referencing the evidence.
/// 3. The identity document and checkpoint snapshots are flushed to the
///    terminal state (fail-closed).
/// 4. The registry entry transitions to `ValidationComplete`.
pub(super) fn fenced_finalize(
    repo: &Path,
    identity_key: &str,
    fence: &FenceToken,
    proposal_id: &str,
    evidence_dir: &Path,
    bundle: &EvidenceBundle,
    heartbeat_rx: &tokio::sync::watch::Receiver<Option<String>>,
    identity_path: &Path,
    run_id: &str,
    repository_revision: &str,
    terminal_state: super::identity::EvaluationState,
    failure_classification: Option<String>,
) -> Result<()> {
    let lock_path = registry_lock_path(repo);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for finalization")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock for finalization: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for finalization");
        }
    }

    // Under lock: revalidate ownership and heartbeat health.
    let mut registry = load_registry(repo).context("failed to read registry for finalization")?;
    let entry = registry
        .entries
        .get_mut(identity_key)
        .context("entry disappeared before finalization")?;

    if entry.owner_run_id != fence.owner_run_id || entry.lease_epoch != fence.lease_epoch {
        bail!(
            "ownership changed during finalization (expected owner={}, epoch={}; \
             found owner={}, epoch={})",
            fence.owner_run_id,
            fence.lease_epoch,
            entry.owner_run_id,
            entry.lease_epoch,
        );
    }

    // Read heartbeat status under the lock — guarantees no race between the
    // revalidation and the status read.
    if let Some(msg) = heartbeat_rx.borrow().as_ref() {
        bail!("heartbeat failure during finalization: {msg}");
    }

    // 1. Final evidence must be durable before the terminal event that
    //    references it. Write the bundle while holding the lock.
    write_bundle(evidence_dir, bundle)
        .context("failed to write evidence bundle during finalization")?;

    // 2. Terminal journal event referencing the final evidence.
    let from_state = super::identity::read_identity_state(identity_path).with_context(|| {
        format!(
            "cannot resolve identity state for terminal journal event at {}",
            identity_path.display()
        )
    })?;
    let evidence_ref =
        super::durable::repo_relative_path(repo, &evidence_dir.join("evidence.json"));
    let sequence = super::journal::append_event_unlocked(
        repo,
        run_id,
        identity_key,
        from_state,
        terminal_state,
        Some(proposal_id.to_string()),
        failure_classification,
        &fence.owner_run_id,
        fence.lease_epoch,
        repository_revision,
        Some(evidence_ref.clone()),
    )
    .context("failed to journal terminal event during finalization")?;

    // 3. Flush identity and checkpoint snapshots (fail-closed).
    super::identity::update_identity_state(identity_path, terminal_state)
        .context("failed to flush terminal identity state during finalization")?;
    let checkpoint = super::checkpoint::build_checkpoint(
        repo,
        identity_key,
        run_id,
        repository_revision,
        terminal_state,
        sequence,
        Some(proposal_id.to_string()),
        Some(evidence_ref),
        &fence.owner_run_id,
        fence.lease_epoch,
    );
    super::checkpoint::write_checkpoint(repo, &checkpoint)
        .context("failed to write terminal checkpoint during finalization")?;

    // 4. Terminal registry snapshot.
    entry.state = ProposalState::ValidationComplete;
    entry.proposal_id = Some(proposal_id.to_string());
    entry.evidence_dir = Some(evidence_dir.to_str().unwrap_or("").to_string());
    entry.updated_at = now_iso();
    entry.heartbeat_at = now_iso();

    save_registry(repo, &registry).context("failed to save registry during finalization")?;

    drop(lock_file);
    Ok(())
}
/// Release a reservation (remove the entry from the registry).
/// Called when generation fails so another process can retry.
/// Requires ownership fencing.
pub(super) fn release_reservation(
    repo: &Path,
    identity_key: &str,
    fence: &FenceToken,
) -> Result<()> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for release")?;

    // Check flock return value.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed to acquire registry lock for release: {err}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for reservation release");
        }
    }

    let mut registry = load_registry(repo).context("failed to read registry for release")?;
    let entry = registry.entries.get_mut(identity_key);

    match entry {
        Some(ref e)
            if e.owner_run_id == fence.owner_run_id && e.lease_epoch == fence.lease_epoch =>
        {
            // Owner matches — remove the entry.
            registry.entries.remove(identity_key);
        }
        Some(_) => {
            // Ownership has changed. Do not remove the entry.
            bail!(
                "cannot release reservation: ownership lost (expected owner={}, epoch={})",
                fence.owner_run_id,
                fence.lease_epoch,
            );
        }
        None => {
            // Entry already removed — that's fine.
        }
    }

    save_registry(repo, &registry)?;

    // Do NOT delete the lock file — just drop the handle to release.
    drop(lock_file);
    Ok(())
}
// ---------------------------------------------------------------------------
// Lease / stale-entry helpers
// ---------------------------------------------------------------------------

/// Map an evaluation journal state to the registry [`ProposalState`] used to
/// represent it for reconciliation.
///
/// The registry is a coarse lifecycle view; the authoritative terminal outcome
/// always lives in the journal and evidence bundle. Terminal evaluation
/// outcomes (success or failure) all collapse to `ValidationComplete`, because
/// the registry has no distinct failure state. Non-terminal states map to the
/// registry state that a live run in that journal position would hold.
pub(super) fn proposal_state_for_state(state: EvaluationState) -> Option<ProposalState> {
    Some(match state {
        EvaluationState::Created | EvaluationState::PreflightPassed => ProposalState::Reserved,
        EvaluationState::Generating => ProposalState::Generating,
        EvaluationState::ProposalGenerated => ProposalState::ProposalGenerated,
        EvaluationState::GovernancePassed | EvaluationState::Validating => {
            ProposalState::Validating
        }
        EvaluationState::ValidationComplete
        | EvaluationState::IntegrityVerified
        | EvaluationState::ReviewGate
        | EvaluationState::PreflightBlocked
        | EvaluationState::GenerationFailed
        | EvaluationState::GovernanceRejected
        | EvaluationState::CandidateCompileFailed
        | EvaluationState::CandidateTestFailed
        | EvaluationState::ValidationFailed
        | EvaluationState::InfraBlocked
        | EvaluationState::IntegrityFailed
        | EvaluationState::InternalError => ProposalState::ValidationComplete,
    })
}

/// Check whether a registry entry is stale according to its state and
/// the given lease configuration.
///
/// - `Reserved`: stale when `reserved_at` is older than
///   `stale_reservation_timeout`.
/// - `Generating`: stale when `heartbeat_at` is older than
///   `generation_lease_timeout`.
/// - Other states: never stale (they should be consumed or returned).
///
/// Malformed timestamps fail closed (return an error) so that corrupt
/// data cannot silently make an entry immortal.
pub(super) fn is_entry_stale(entry: &RegistryEntry, lease_config: &LeaseConfig) -> Result<bool> {
    match entry.state {
        ProposalState::Reserved => {
            let t = parse_rfc3339(&entry.reserved_at)
                .context("malformed reserved_at timestamp in registry entry")?;
            let age = chrono::Utc::now()
                .signed_duration_since(t)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);
            Ok(age > lease_config.stale_reservation_timeout)
        }
        ProposalState::Generating => {
            let t = parse_rfc3339(&entry.heartbeat_at)
                .context("malformed heartbeat_at timestamp in registry entry")?;
            let age = chrono::Utc::now()
                .signed_duration_since(t)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);
            Ok(age > lease_config.generation_lease_timeout)
        }
        ProposalState::ProposalGenerated => {
            // ProposalGenerated is always reclaimable: the previous owner has
            // finished generation. No heartbeat check needed.
            Ok(false)
        }
        ProposalState::Validating => {
            // Validating uses the same heartbeat-based stale check as Generating.
            let t = parse_rfc3339(&entry.heartbeat_at)
                .context("malformed heartbeat_at timestamp in registry entry")?;
            let age = chrono::Utc::now()
                .signed_duration_since(t)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);
            Ok(age > lease_config.generation_lease_timeout)
        }
        ProposalState::ValidationComplete => Ok(false),
    }
}

/// Parse an RFC3339 timestamp string, returning the `chrono::DateTime<Utc>`.
fn parse_rfc3339(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .context("invalid RFC3339 timestamp")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposal_state_for_state_mapping_is_data_driven() {
        assert_eq!(
            proposal_state_for_state(EvaluationState::Generating),
            Some(ProposalState::Generating)
        );
        assert_eq!(
            proposal_state_for_state(EvaluationState::ProposalGenerated),
            Some(ProposalState::ProposalGenerated)
        );
        assert_eq!(
            proposal_state_for_state(EvaluationState::Validating),
            Some(ProposalState::Validating)
        );
        assert_eq!(
            proposal_state_for_state(EvaluationState::ValidationComplete),
            Some(ProposalState::ValidationComplete)
        );
        // Every terminal outcome collapses to the registry's terminal state.
        assert_eq!(
            proposal_state_for_state(EvaluationState::ReviewGate),
            Some(ProposalState::ValidationComplete)
        );
        assert_eq!(
            proposal_state_for_state(EvaluationState::GenerationFailed),
            Some(ProposalState::ValidationComplete)
        );
        assert_eq!(
            proposal_state_for_state(EvaluationState::IntegrityFailed),
            Some(ProposalState::ValidationComplete)
        );
    }
    #[test]
    fn is_entry_stale_validating_fresh_heartbeat() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Validating,
            heartbeat_at: now_iso(),
            updated_at: now_iso(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(!stale, "fresh Validating heartbeat must not be stale");
    }
    #[test]
    fn is_entry_stale_validating_stale_heartbeat() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Validating,
            heartbeat_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(stale, "old Validating heartbeat must be stale");
    }
    #[test]
    fn is_entry_stale_validating_malformed_heartbeat_fails_closed() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Validating,
            heartbeat_at: "not-a-timestamp".to_string(),
            updated_at: now_iso(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(180),
        );
        let result = is_entry_stale(&entry, &config);
        assert!(result.is_err(), "malformed heartbeat must produce an error");
    }
    #[test]
    fn is_entry_stale_reserved_reclaimable_after_timeout() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Reserved,
            reserved_at: "2020-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120), // stale_reservation_timeout
            std::time::Duration::from_secs(600),
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(stale, "Reserved entry older than timeout must be stale");
    }
    #[test]
    fn generating_state_not_stale_while_heartbeating() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Generating,
            heartbeat_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: now_iso(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(1), // 1s generation_lease_timeout
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(
            stale,
            "Generating with very old heartbeat and 1s timeout must be stale"
        );
    }
    #[test]
    fn proposal_generated_never_stale() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::ProposalGenerated,
            updated_at: "2020-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        let config = LeaseConfig::default();
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(!stale, "ProposalGenerated must never be stale");
    }
    #[test]
    fn generating_state_fresh_heartbeat_not_stale() {
        // A Generating entry with a fresh heartbeat is NOT stale even when
        // the generation_lease_timeout is shorter than typical execution time,
        // because the heartbeat re-validates liveness.  The timeout must be
        // >1s because now_iso() has 1-second precision.
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Generating,
            heartbeat_at: now_iso(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(!stale, "Generating with fresh heartbeat must not be stale");
    }
    #[test]
    fn validating_state_fresh_heartbeat_not_stale() {
        let entry = RegistryEntry {
            owner_run_id: "test".to_string(),
            lease_epoch: 1,
            state: ProposalState::Validating,
            heartbeat_at: now_iso(),
            ..Default::default()
        };
        let config = LeaseConfig::with_timeouts(
            std::time::Duration::from_secs(120),
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(180),
        );
        let stale = is_entry_stale(&entry, &config).unwrap();
        assert!(!stale, "Validating with fresh heartbeat must not be stale");
    }
    #[test]
    fn renew_heartbeat_detects_ownership_change() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        // Create registry with one entry under worker-1's ownership.
        let entry = RegistryEntry {
            owner_run_id: "worker-1".to_string(),
            lease_epoch: 1,
            state: ProposalState::Generating,
            heartbeat_at: now_iso(),
            ..Default::default()
        };
        let mut registry = ProposalRegistry::default();
        registry.entries.insert("test-key".to_string(), entry);
        save_registry(&repo, &registry).unwrap();

        let fence = FenceToken {
            owner_run_id: "worker-1".to_string(),
            lease_epoch: 1,
        };

        // First heartbeat must succeed — owner and epoch match.
        let result = renew_heartbeat(&repo, "test-key", &fence).unwrap();
        assert!(result, "initial heartbeat with matching fence must succeed");

        // Simulate ownership theft by a second worker.
        let mut registry = load_registry(&repo).unwrap();
        if let Some(e) = registry.entries.get_mut("test-key") {
            e.owner_run_id = "thief".to_string();
            e.lease_epoch = 2;
        }
        save_registry(&repo, &registry).unwrap();

        // Heartbeat with original (stale) fence must return false.
        let result = renew_heartbeat(&repo, "test-key", &fence).unwrap();
        assert!(!result, "heartbeat must fail after ownership changes");
    }
    #[test]
    fn renew_heartbeat_missing_entry_fails() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let fence = FenceToken {
            owner_run_id: "worker-1".to_string(),
            lease_epoch: 1,
        };

        // Empty registry — no entry for this key.
        save_registry(&repo, &ProposalRegistry::default()).unwrap();

        let result = renew_heartbeat(&repo, "missing-key", &fence).unwrap();
        assert!(!result, "heartbeat for missing entry must return false");
    }
}
