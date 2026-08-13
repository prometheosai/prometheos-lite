//! Durable append-only event journal for evaluation state transitions.
//!
//! Every state transition is recorded as an immutable, atomically-written
//! event before the identity document is flushed. The journal is the source
//! of truth for recovery: replaying it (under an exclusive lock) reconstructs
//! the evaluation state without trusting a possibly-corrupt identity file.
//!
//! Invariants:
//! - Append-only: each event has a monotonic sequence per identity.
//! - Atomic: an event is never partially written (temp file + rename).
//! - Fenced: appends verify ownership (`owner_run_id` + `lease_epoch`) against
//!   the registry under its lock; a stale fence cannot append.
//! - Provenance: every event records the owning run, lease epoch, repository
//!   revision, and evidence reference so recovery can validate them.
//! - A malformed, sequence-colliding, or provenance-mismatched event fails
//!   closed rather than being silently skipped.
//! - No secrets are ever written to events.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::identity::EvaluationState;
use super::registry::FenceToken;
use super::schema::{
    CURRENT_SCHEMA_VERSION, DocumentType, SchemaVersion, validate_version, version_diagnostic,
};

/// An immutable record of one evaluation state transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEvent {
    /// Schema version of this event document.
    pub schema_version: SchemaVersion,
    /// Unique event id (UUID).
    pub event_id: String,
    /// Monotonic sequence for this identity's journal.
    pub sequence: u64,
    /// Run that produced this event.
    pub run_id: String,
    /// Deterministic identity key.
    pub identity_key: String,
    /// RFC3339 timestamp.
    pub timestamp: String,
    /// Previous state (may be the same state for an idempotent re-assert).
    pub from_state: EvaluationState,
    /// New state.
    pub to_state: EvaluationState,
    /// The proposal id, when one exists at this point.
    pub proposal_ref: Option<String>,
    /// Failure classification, for terminal failure events.
    pub failure_classification: Option<String>,
    /// Run id that held ownership of the registry entry when this event was
    /// appended (fencing provenance).
    pub owner_run_id: String,
    /// Lease epoch that protected the append (fencing provenance).
    pub lease_epoch: u64,
    /// Repository revision the transition was made against.
    pub repository_revision: String,
    /// Evidence artifact reference (repo-relative path to the evidence dir or
    /// `evidence.json`), when one exists at this point.
    pub evidence_ref: Option<String>,
    /// Checkpoint file reference (repo-relative path), when written.
    pub checkpoint_ref: Option<String>,
}

/// Directory that holds the event journal for a single identity.
pub fn journal_dir_for(repo: &Path, identity_key: &str) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("journal")
        .join(identity_key)
}

fn event_path(journal_dir: &Path, sequence: u64) -> PathBuf {
    journal_dir.join(format!("{sequence:020}.json"))
}

/// Highest sequence currently persisted, or `None` if the journal is empty.
pub fn last_sequence(journal_dir: &Path) -> Result<Option<u64>> {
    Ok(read_all(journal_dir)?.last().map(|e| e.sequence))
}

/// Enforce that a new event continues the journal tail without forking.
///
/// The durable journal is the authoritative append-only history. Ownership may
/// change across runs (a reclaimed run continues the same identity key), but
/// history may not: every non-empty journal's tail outcome is the only valid
/// `from_state` for the next event, regardless of which run appends it.
///
/// Before an append we also validate, owner-independently:
/// - the tail belongs to this identity key,
/// - the sequence continues monotonically from the tail,
/// - the repository revision is constant across the run,
/// - the lease epoch does not regress.
///
/// Fencing (WHO may append) is enforced separately by the registry lock in
/// `append_event`; this function enforces WHERE they may append (the journal
/// tail).
fn verify_tail_continuity(
    journal_dir: &Path,
    identity_key: &str,
    from_state: EvaluationState,
    owner_run_id: &str,
    repository_revision: &str,
    lease_epoch: u64,
    next_sequence: u64,
) -> Result<()> {
    let events = read_all(journal_dir)?;
    let Some(tail) = events.last() else {
        return Ok(());
    };
    // History is owner-independent: the tail outcome is authoritative for every
    // subsequent append, no matter which run produced it.
    if tail.to_state != from_state {
        bail!(
            "journal tail continuity violation: tail event {} ended at {:?} \
             but new event (owner {}) starts at {:?}",
            tail.sequence,
            tail.to_state,
            owner_run_id,
            from_state
        );
    }
    // The journal is per-identity; the tail must belong to this identity.
    if tail.identity_key != identity_key {
        bail!(
            "journal tail identity mismatch: tail event {} belongs to {} not {}",
            tail.sequence,
            tail.identity_key,
            identity_key
        );
    }
    // Sequences must continue monotonically from the tail.
    if tail.sequence + 1 != next_sequence {
        bail!(
            "journal sequence non-monotonic: tail {} followed by {}",
            tail.sequence,
            next_sequence
        );
    }
    // The repository revision must stay constant across the run.
    if tail.repository_revision != repository_revision {
        bail!(
            "journal repository revision changed mid-run at tail {}: {} -> {}",
            tail.sequence,
            tail.repository_revision,
            repository_revision
        );
    }
    // The lease epoch must not regress across appends.
    if tail.lease_epoch > lease_epoch {
        bail!(
            "journal lease epoch regressed at tail {}: {} -> {}",
            tail.sequence,
            tail.lease_epoch,
            lease_epoch
        );
    }
    Ok(())
}

fn read_all(journal_dir: &Path) -> Result<Vec<JournalEvent>> {
    let mut events: Vec<JournalEvent> = Vec::new();
    if !journal_dir.exists() {
        return Ok(events);
    }
    let expected_key = journal_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(journal_dir)
        .with_context(|| format!("failed to read journal dir {}", journal_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read journal event {}", path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("corrupt journal event {}", path.display()))?;
        let declared = match value.get("schema_version") {
            Some(v) => SchemaVersion::parse(v.as_str().context("schema_version must be a string")?)
                .with_context(|| format!("malformed schema_version in {}", path.display()))?,
            None => super::schema::LEGACY_UNVERSIONED_VERSION,
        };
        let status = validate_version(DocumentType::JournalEvent, declared)?;
        if let super::schema::VersionStatus::Unsupported = status {
            bail!(
                "{}",
                version_diagnostic(DocumentType::JournalEvent, declared).migration_action
            );
        }
        let event: JournalEvent = serde_json::from_value(value)
            .with_context(|| format!("corrupt journal event {}", path.display()))?;
        // The on-disk filename encodes the expected sequence; a mismatch means
        // the journal was reordered or forged — fail closed.
        let filename_seq = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
            .with_context(|| format!("journal filename has no sequence: {}", path.display()))?;
        if filename_seq != event.sequence {
            bail!(
                "journal filename/sequence mismatch for {}: filename {filename_seq}, event {}",
                path.display(),
                event.sequence
            );
        }
        if event.identity_key != expected_key {
            bail!(
                "journal event identity mismatch: directory expects {expected_key}, event has {}",
                event.identity_key
            );
        }
        events.push(event);
    }
    Ok(events)
}

/// Append an event once the sequence is known to be the next expected value.
///
/// The event is written atomically (temp file + fsync + rename + dir fsync).
/// The caller must ensure `next_sequence` equals `last_sequence + 1` to
/// preserve monotonicity (see [`append_event`] and [`append_event_unlocked`]).
fn write_event(journal_dir: &Path, event: &JournalEvent) -> Result<()> {
    let path = event_path(journal_dir, event.sequence);
    if path.exists() {
        bail!(
            "journal sequence {} already exists (collision or replay) for {}",
            event.sequence,
            event.identity_key
        );
    }
    super::durable::atomic_write_json(&path, event)
        .with_context(|| format!("failed to commit event {}", path.display()))
}

/// Append a transition event without registry fencing.
///
/// Used by recovery tests and by [`fenced_finalize`] (which already holds the
/// registry lock and revalidated the fence). Production transitions must use
/// the fenced [`append_event`].
pub fn append_event_unlocked(
    repo: &Path,
    run_id: &str,
    identity_key: &str,
    from_state: EvaluationState,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    failure_classification: Option<String>,
    owner_run_id: &str,
    lease_epoch: u64,
    repository_revision: &str,
    evidence_ref: Option<String>,
) -> Result<u64> {
    super::transition::validate_transition(from_state, to_state)
        .context("refusing to journal an illegal state transition")?;
    let journal_dir = journal_dir_for(repo, identity_key);
    let next_sequence = match last_sequence(&journal_dir)? {
        Some(last) => last + 1,
        None => 0,
    };
    verify_tail_continuity(
        &journal_dir,
        identity_key,
        from_state,
        owner_run_id,
        repository_revision,
        lease_epoch,
        next_sequence,
    )?;
    let checkpoint_ref = super::durable::repo_relative_path(
        repo,
        &super::checkpoint::checkpoint_path_for(repo, identity_key),
    );
    let event = JournalEvent {
        schema_version: CURRENT_SCHEMA_VERSION,
        event_id: uuid::Uuid::new_v4().to_string(),
        sequence: next_sequence,
        run_id: run_id.to_string(),
        identity_key: identity_key.to_string(),
        timestamp: super::identity::now_iso(),
        from_state,
        to_state,
        proposal_ref,
        failure_classification,
        owner_run_id: owner_run_id.to_string(),
        lease_epoch,
        repository_revision: repository_revision.to_string(),
        evidence_ref,
        checkpoint_ref: Some(checkpoint_ref),
    };
    write_event(&journal_dir, &event)?;
    Ok(next_sequence)
}

/// Append a transition event under the registry synchronization boundary.
///
/// The registry lock is acquired, the registry reloaded, and ownership
/// revalidated against `fence` before the next sequence is computed and the
/// event appended. A stale fence (entry owned by another run or a newer
/// epoch) is rejected so a timed-out worker can never append to a journal it
/// no longer owns.
pub fn append_event(
    repo: &Path,
    run_id: &str,
    identity_key: &str,
    from_state: EvaluationState,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    failure_classification: Option<String>,
    repository_revision: &str,
    evidence_ref: Option<String>,
    fence: &FenceToken,
) -> Result<u64> {
    super::transition::validate_transition(from_state, to_state)
        .context("refusing to journal an illegal state transition")?;
    super::registry::with_registry_lock(repo, |registry| {
        let entry = registry.entries.get(identity_key).with_context(|| {
            format!(
                "journal append refused: no registry entry for {identity_key} \
                 (ownership not established)"
            )
        })?;
        if entry.owner_run_id != fence.owner_run_id || entry.lease_epoch != fence.lease_epoch {
            bail!(
                "stale fence: journal append rejected (entry owner={} epoch={}; \
                 caller owner={} epoch={})",
                entry.owner_run_id,
                entry.lease_epoch,
                fence.owner_run_id,
                fence.lease_epoch,
            );
        }
        let journal_dir = journal_dir_for(repo, identity_key);
        let next_sequence = match last_sequence(&journal_dir)? {
            Some(last) => last + 1,
            None => 0,
        };
        verify_tail_continuity(
            &journal_dir,
            identity_key,
            from_state,
            &fence.owner_run_id,
            repository_revision,
            fence.lease_epoch,
            next_sequence,
        )?;
        let checkpoint_ref = super::durable::repo_relative_path(
            repo,
            &super::checkpoint::checkpoint_path_for(repo, identity_key),
        );
        let event = JournalEvent {
            schema_version: CURRENT_SCHEMA_VERSION,
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: next_sequence,
            run_id: run_id.to_string(),
            identity_key: identity_key.to_string(),
            timestamp: super::identity::now_iso(),
            from_state,
            to_state,
            proposal_ref,
            failure_classification,
            owner_run_id: fence.owner_run_id.clone(),
            lease_epoch: fence.lease_epoch,
            repository_revision: repository_revision.to_string(),
            evidence_ref,
            checkpoint_ref: Some(checkpoint_ref),
        };
        write_event(&journal_dir, &event)?;
        Ok(next_sequence)
    })
}

/// Read all journal events for an identity, in sequence order.
pub fn read_journal(repo: &Path, identity_key: &str) -> Result<Vec<JournalEvent>> {
    let events = read_all(&journal_dir_for(repo, identity_key))?;
    for event in &events {
        if event.identity_key != identity_key {
            bail!(
                "journal event identity mismatch: expected {identity_key}, event has {}",
                event.identity_key
            );
        }
    }
    Ok(events)
}

/// Perform a durable, ordered state transition, fail-closed.
///
/// Ordering protocol (durability before visibility):
/// 1. Validate the transition against the transition law.
/// 2. Append the journal event under the registry lock with the caller's
///    fence (authoritative durable record; stale owners are rejected).
/// 3. Flush the identity document to the new state — a failure here
///    propagates to the caller (the run refuses to continue).
/// 4. Write a compact checkpoint — a failure here also propagates.
///
/// The caller must already hold ownership of the identity.
pub fn record_transition(
    repo: &Path,
    identity_path: &Path,
    run_id: &str,
    identity_key: &str,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    failure_classification: Option<String>,
    repository_revision: &str,
    evidence_ref: Option<String>,
    fence: &FenceToken,
) -> Result<u64> {
    let from_state = super::identity::read_identity_state(identity_path).with_context(|| {
        format!(
            "cannot resolve current identity state at {} for transition to {to_state:?}",
            identity_path.display()
        )
    })?;
    let sequence = append_event(
        repo,
        run_id,
        identity_key,
        from_state,
        to_state,
        proposal_ref.clone(),
        failure_classification,
        repository_revision,
        evidence_ref.clone(),
        fence,
    )
    .context("failed to persist journal event")?;
    super::identity::update_identity_state(identity_path, to_state)
        .with_context(|| format!("failed to flush identity state for {to_state:?}"))?;
    let checkpoint = super::checkpoint::build_checkpoint(
        repo,
        identity_key,
        run_id,
        repository_revision,
        to_state,
        sequence,
        proposal_ref,
        evidence_ref,
        &fence.owner_run_id,
        fence.lease_epoch,
    );
    super::checkpoint::write_checkpoint(repo, &checkpoint)
        .context("failed to persist checkpoint for transition")?;
    Ok(sequence)
}

/// Verify journal integrity:
/// - no duplicate sequences, monotonic order, no gaps
/// - every event validates against the replay (checked by recovery)
pub fn validate_journal(journal_dir: &Path) -> Result<()> {
    let events = read_all(journal_dir)?;
    for pair in events.windows(2) {
        if pair[1].sequence != pair[0].sequence + 1 {
            bail!(
                "journal sequence gap: {} then {}",
                pair[0].sequence,
                pair[1].sequence
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_dir() -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        dir.path().to_path_buf()
    }

    fn sample_event(sequence: u64) -> JournalEvent {
        JournalEvent {
            schema_version: super::CURRENT_SCHEMA_VERSION,
            event_id: format!("evt-{sequence}"),
            sequence,
            run_id: "run-1".to_string(),
            identity_key: "key-1".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            from_state: EvaluationState::Created,
            to_state: EvaluationState::PreflightPassed,
            proposal_ref: None,
            failure_classification: None,
            owner_run_id: "run-1".to_string(),
            lease_epoch: 1,
            repository_revision: "abc123".to_string(),
            evidence_ref: None,
            checkpoint_ref: None,
        }
    }

    #[test]
    fn append_events_are_monotonic_and_round_trippable() {
        let repo = repo_dir();
        let seq0 = append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        let seq1 = append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::PreflightPassed,
            EvaluationState::Generating,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        assert_eq!(seq0, 0);
        assert_eq!(seq1, 1);

        let events = read_journal(&repo, "key-1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
        assert_eq!(events[1].to_state, EvaluationState::Generating);
        assert_eq!(events[1].owner_run_id, "run-1");
        assert_eq!(events[1].lease_epoch, 1);
        assert_eq!(events[1].repository_revision, "abc123");
        assert_eq!(
            last_sequence(&journal_dir_for(&repo, "key-1")).unwrap(),
            Some(1)
        );
        validate_journal(&journal_dir_for(&repo, "key-1")).unwrap();
    }

    #[test]
    fn rejects_illegal_transition_before_writing() {
        let repo = repo_dir();
        let err = append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::ReviewGate,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("illegal state transition"));
        assert!(!journal_dir_for(&repo, "key-1").exists());
    }

    #[test]
    fn duplicate_sequence_fails_closed() {
        let repo = repo_dir();
        append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        // Attempt to write an event at the same sequence: must fail.
        let dir = journal_dir_for(&repo, "key-1");
        let duplicate = sample_event(0);
        assert!(write_event(&dir, &duplicate).is_err());
    }

    #[test]
    fn corrupt_event_file_fails_closed() {
        let repo = repo_dir();
        append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::write(dir.join("00000000000000000001.json"), "not json{{").unwrap();
        assert!(read_journal(&repo, "key-1").is_err());
    }

    #[test]
    fn unsupported_journal_version_fails_closed() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut event = sample_event(0);
        event.schema_version = SchemaVersion::new(99, 0, 0);
        let json = serde_json::to_string_pretty(&event).unwrap();
        std::fs::write(dir.join("00000000000000000000.json"), json).unwrap();
        let err = read_journal(&repo, "key-1").unwrap_err();
        assert!(err.to_string().contains("fail closed"));
    }

    #[test]
    fn filename_sequence_mismatch_fails_closed() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        // Event claims sequence 0 but lives at sequence-1 filename.
        let event = sample_event(0);
        std::fs::write(
            dir.join("00000000000000000001.json"),
            serde_json::to_string_pretty(&event).unwrap(),
        )
        .unwrap();
        let err = read_journal(&repo, "key-1").unwrap_err();
        assert!(err.to_string().contains("filename/sequence mismatch"));
    }

    #[test]
    fn identity_mismatch_fails_closed() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        let mut event = sample_event(0);
        event.identity_key = "other-key".to_string();
        std::fs::write(
            dir.join("00000000000000000000.json"),
            serde_json::to_string_pretty(&event).unwrap(),
        )
        .unwrap();
        let err = read_journal(&repo, "key-1").unwrap_err();
        assert!(err.to_string().contains("identity mismatch"));
    }

    #[test]
    fn tail_continuity_detects_same_owner_fork() {
        let repo = repo_dir();
        append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        // Same owner attempts to branch from the original tail (Created) instead
        // of continuing from PreflightPassed — this is a fork and must fail.
        let err = append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Generating,
            EvaluationState::ProposalGenerated,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("tail continuity"));
    }

    #[test]
    fn tail_continuity_rejects_cross_owner_fork() {
        let repo = repo_dir();
        append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-1",
            1,
            "abc123",
            None,
        )
        .unwrap();
        // A different owner may take over the identity, but it may NOT start a
        // fresh chain from Created — the tail is at PreflightPassed. This is a
        // fork and must fail closed regardless of owner.
        let err = append_event_unlocked(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
            "run-2",
            1,
            "abc123",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("tail continuity"));
    }

    #[test]
    fn tail_continuity_allows_valid_cross_owner_continuation() {
        let repo = repo_dir();
        // Owner A: Created -> PreflightPassed -> Generating -> ProposalGenerated.
        for (from, to) in [
            (EvaluationState::Created, EvaluationState::PreflightPassed),
            (
                EvaluationState::PreflightPassed,
                EvaluationState::Generating,
            ),
            (
                EvaluationState::Generating,
                EvaluationState::ProposalGenerated,
            ),
        ] {
            append_event_unlocked(
                &repo, "run-1", "key-1", from, to, None, None, "run-1", 1, "abc123", None,
            )
            .unwrap();
        }
        // Owner B legitimately reclaims the identity (registry entry owned by
        // run-2, epoch 1) and continues the chain with a valid fence.
        super::super::registry::try_reserve(&repo, "key-1", "run-2").unwrap();
        let fence = FenceToken {
            owner_run_id: "run-2".to_string(),
            lease_epoch: 1,
        };
        let seq = append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::ProposalGenerated,
            EvaluationState::Validating,
            None,
            None,
            "abc123",
            None,
            &fence,
        )
        .unwrap();
        assert_eq!(seq, 3);
        let events = read_journal(&repo, "key-1").unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[3].to_state, EvaluationState::Validating);
        assert_eq!(events[3].owner_run_id, "run-2");
    }

    #[test]
    fn sequence_gap_is_detected() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("00000000000000000000.json"),
            serde_json::to_string_pretty(&sample_event(0)).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("00000000000000000002.json"),
            serde_json::to_string_pretty(&sample_event(2)).unwrap(),
        )
        .unwrap();
        assert!(validate_journal(&dir).is_err());
    }
}
