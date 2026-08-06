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
//! - A malformed or sequence-colliding event fails closed rather than being
//!   silently skipped.
//! - No secrets are ever written to events.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::identity::EvaluationState;
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
    /// Checkpoint file reference (relative path), when written.
    pub checkpoint_ref: Option<String>,
    /// Failure classification, for terminal failure events.
    pub failure_classification: Option<String>,
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

fn read_all(journal_dir: &Path) -> Result<Vec<JournalEvent>> {
    let mut events: Vec<JournalEvent> = Vec::new();
    if !journal_dir.exists() {
        return Ok(events);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(journal_dir)
        .with_context(|| format!("failed to read journal dir {}", journal_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read journal event {}", path.display()))?;
        let event: JournalEvent = serde_json::from_str(&text)
            .with_context(|| format!("corrupt journal event {}", path.display()))?;
        let status = validate_version(DocumentType::JournalEvent, event.schema_version)?;
        if let super::schema::VersionStatus::Unsupported = status {
            bail!(
                "{}",
                version_diagnostic(DocumentType::JournalEvent, event.schema_version)
                    .migration_action
            );
        }
        events.push(event);
    }
    Ok(events)
}

/// Append an event once the sequence is known to be the next expected value.
///
/// The event is written to a temp file, fsynced, then renamed into place.
/// The caller must ensure `next_sequence` equals `last_sequence + 1` to
/// preserve monotonicity (see [`append_event`]).
fn write_event(journal_dir: &Path, event: &JournalEvent) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(journal_dir)
        .with_context(|| format!("failed to create journal dir {}", journal_dir.display()))?;
    let path = event_path(journal_dir, event.sequence);
    if path.exists() {
        bail!(
            "journal sequence {} already exists (collision or replay) for {}",
            event.sequence,
            event.identity_key
        );
    }
    let tmp = journal_dir.join(format!("{}.tmp", event.sequence));
    let json = serde_json::to_string_pretty(event).context("failed to serialize journal event")?;
    let mut file = std::fs::File::create(&tmp)
        .with_context(|| format!("failed to create event temp {}", tmp.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("failed to write event temp {}", tmp.display()))?;
    // fsync the temp file so a crash cannot leave a truncated final event.
    file.sync_all()
        .with_context(|| format!("failed to fsync event temp {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to commit event {}", path.display()))?;
    sync_dir(journal_dir);
    Ok(())
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Append a transition event to the identity's journal.
///
/// Returns the sequence number of the appended event. Fails if `from_state`
/// does not legally lead to `to_state`, or if the journal is inconsistent.
pub fn append_event(
    repo: &Path,
    run_id: &str,
    identity_key: &str,
    from_state: EvaluationState,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    failure_classification: Option<String>,
) -> Result<u64> {
    super::transition::validate_transition(from_state, to_state)
        .context("refusing to journal an illegal state transition")?;
    let journal_dir = journal_dir_for(repo, identity_key);
    let next_sequence = match last_sequence(&journal_dir)? {
        Some(last) => last + 1,
        None => 0,
    };
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
        checkpoint_ref: None,
        failure_classification,
    };
    write_event(&journal_dir, &event)?;
    Ok(next_sequence)
}

/// Read all journal events for an identity, in sequence order.
pub fn read_journal(repo: &Path, identity_key: &str) -> Result<Vec<JournalEvent>> {
    read_all(&journal_dir_for(repo, identity_key))
}

/// Perform a durable, ordered state transition.
///
/// Ordering protocol (durability before visibility):
/// 1. Validate the transition against the transition law.
/// 2. Append the journal event (fsynced file + dir sync) — this is the
///    authoritative, durable record.
/// 3. Flush the identity document to the new state (derived, best-effort).
/// 4. Write a compact checkpoint (best-effort fast path for recovery).
///
/// The caller must already hold ownership of the identity.
pub fn record_transition(
    repo: &Path,
    identity_path: &Path,
    run_id: &str,
    identity_key: &str,
    to_state: EvaluationState,
    proposal_ref: Option<String>,
    owner_run_id: &str,
    lease_epoch: u64,
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
        None,
    )
    .context("failed to persist journal event")?;
    super::identity::update_identity_state(identity_path, to_state);
    let checkpoint = super::checkpoint::build_checkpoint(
        repo,
        identity_key,
        run_id,
        &repo_pin(repo),
        to_state,
        sequence,
        proposal_ref,
        Some(identity_path.to_string_lossy().into_owned()),
        owner_run_id,
        lease_epoch,
    );
    let _ = super::checkpoint::write_checkpoint(repo, &checkpoint);
    Ok(sequence)
}

fn repo_pin(repo: &Path) -> String {
    super::integrity::git_rev_parse_head(repo).unwrap_or_else(|_| "unknown".to_string())
}

/// Verify journal integrity:
/// - no duplicate sequences, monotonic order
/// - the newest existing event (if any) is followed (optional)
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
            checkpoint_ref: None,
            failure_classification: None,
        }
    }

    #[test]
    fn append_events_are_monotonic_and_round_trippable() {
        let repo = repo_dir();
        let seq0 = append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
            None,
        )
        .unwrap();
        let seq1 = append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::PreflightPassed,
            EvaluationState::Generating,
            None,
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
        assert_eq!(
            last_sequence(&journal_dir_for(&repo, "key-1")).unwrap(),
            Some(1)
        );
        validate_journal(&journal_dir_for(&repo, "key-1")).unwrap();
    }

    #[test]
    fn rejects_illegal_transition_before_writing() {
        let repo = repo_dir();
        let err = append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::ReviewGate,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("illegal state transition"));
        assert!(!journal_dir_for(&repo, "key-1").exists());
    }

    #[test]
    fn duplicate_sequence_fails_closed() {
        let repo = repo_dir();
        append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
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
        append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Created,
            EvaluationState::PreflightPassed,
            None,
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
