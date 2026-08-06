//! Recovery of evaluation state from durable artifacts.
//!
//! When a run is restarted (crash, kill, reboot), the durable state journal
//! and checkpoints let us reconstruct where the evaluation stopped. The
//! journal is the source of truth; checkpoints are a fast path. Recovery
//! never trusts a possibly-corrupt identity file on its own.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use super::checkpoint::{checkpoint_path_for, read_checkpoint};
use super::identity::EvaluationState;
use super::journal::{journal_dir_for, read_journal, validate_journal};
use super::migration::migrate_document;
use super::schema::DocumentType;

/// The reconstructed state of an evaluation after recovery.
#[derive(Debug, Clone)]
pub struct RecoveredEvaluation {
    /// Replayed evaluation state.
    pub state: EvaluationState,
    /// Highest journal sequence replayed.
    pub last_journal_sequence: u64,
    /// Proposal reference, if the proposal was generated.
    pub proposal_ref: Option<String>,
    /// Evidence directory reference, if known.
    pub evidence_ref: Option<String>,
    /// True if the journal was missing but a checkpoint was present.
    pub recovered_from_checkpoint: bool,
}

/// Gate a resume on the recovered durable state.
///
/// Fails closed when the journal records a terminal outcome (the run already
/// finished and must not be resumed), when the recovered proposal reference
/// disagrees with the proposal being resumed, or when a recorded evidence
/// reference is missing. Reading all fields here also makes the recovered
/// structure's contract explicit.
pub fn ensure_resumable(
    resumed_from: EvaluationState,
    proposal_id: &str,
    recovered: &RecoveredEvaluation,
) -> Result<()> {
    if recovered.state.is_terminal() {
        bail!(
            "durable journal records terminal state {:?} at sequence {}; refusing to resume",
            recovered.state,
            recovered.last_journal_sequence
        );
    }
    if let Some(ref journal_proposal) = recovered.proposal_ref
        && journal_proposal != proposal_id
    {
        bail!("journal proposal {journal_proposal} does not match resumed proposal {proposal_id}");
    }
    if let Some(ref evidence_ref) = recovered.evidence_ref {
        let evidence_path = Path::new(evidence_ref);
        if !evidence_path.exists() {
            bail!(
                "recovered evidence reference {} is missing",
                evidence_path.display()
            );
        }
    }
    if recovered.recovered_from_checkpoint && resumed_from == EvaluationState::Created {
        // A checkpoint-only legacy recovery should never claim a run has not
        // started while the journal is empty and the checkpoint exists.
        bail!("checkpoint-only recovery conflicts with fresh resume from Created");
    }
    Ok(())
}

/// Recover the durable state of an evaluation for `identity_key`.
///
/// Returns `None` when no journal or checkpoint exists yet (the run has not
/// reached any durable transition). Order of operations:
/// 1. Migrate any legacy durable documents to the current schema.
/// 2. Validate the journal (monotonic, no gaps, no unsupported versions).
/// 3. Replay the journal to reconstruct the evaluation state.
/// 4. Cross-check against the latest checkpoint; fail if the checkpoint is
///    ahead of the journal (snapshot-ahead corruption).
///
/// Does not read or trust `execution_identity.json` for state reconstruction.
pub fn recover_evaluation(repo: &Path, identity_key: &str) -> Result<Option<RecoveredEvaluation>> {
    // Step 1: migrate legacy durable documents (journal events, checkpoint,
    // registry, and the identity/evidence referenced by the checkpoint).
    let journal_dir = journal_dir_for(repo, identity_key);
    let checkpoint_path = checkpoint_path_for(repo, identity_key);

    if journal_dir.exists() {
        for path in list_json(&journal_dir)? {
            migrate_document(&path, DocumentType::JournalEvent)?;
        }
    }
    if checkpoint_path.exists() {
        migrate_document(&checkpoint_path, DocumentType::EvaluationCheckpoint)?;
    }
    let registry = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");
    if registry.exists() {
        migrate_document(&registry, DocumentType::ProposalRegistry)?;
    }
    if let Some(cp) = read_checkpoint(repo, identity_key)?
        && let Some(evidence_ref) = cp.evidence_ref
    {
        let identity_path = PathBuf::from(evidence_ref);
        if identity_path.exists() {
            migrate_document(&identity_path, DocumentType::ExecutionIdentity)?;
        }
        if let Some(parent) = identity_path.parent() {
            let evidence = parent.join("evidence.json");
            if evidence.exists() {
                migrate_document(&evidence, DocumentType::EvidenceBundle)?;
            }
        }
    }

    // Step 2 + 3: validate and replay the journal.
    let events = read_journal(repo, identity_key)?;
    if events.is_empty() {
        // No journal yet: this run never reached a durable transition beyond
        // the identity file. Reconstruct from the checkpoint, if any.
        if let Some(cp) = read_checkpoint(repo, identity_key)? {
            return Ok(Some(RecoveredEvaluation {
                state: cp.state,
                last_journal_sequence: cp.last_journal_sequence,
                proposal_ref: cp.proposal_ref,
                evidence_ref: cp.evidence_ref,
                recovered_from_checkpoint: true,
            }));
        }
        return Ok(None);
    }
    validate_journal(&journal_dir)?;

    let mut state = events[0].from_state;
    let mut last_seq = events[0].sequence;
    let mut proposal_ref: Option<String> = None;
    for event in &events {
        super::transition::validate_transition(state, event.to_state).with_context(|| {
            format!(
                "journal replay hit illegal transition at sequence {}: {:?} -> {:?}",
                event.sequence, state, event.to_state
            )
        })?;
        state = event.to_state;
        last_seq = event.sequence;
        if event.proposal_ref.is_some() {
            proposal_ref = event.proposal_ref.clone();
        }
    }

    // Step 4: checkpoint cross-check. A checkpoint ahead of the journal means
    // the journal was truncated or lost — fail closed.
    if let Some(cp) = read_checkpoint(repo, identity_key)? {
        if cp.last_journal_sequence > last_seq {
            bail!(
                "checkpoint sequence {} is ahead of journal sequence {} for {}",
                cp.last_journal_sequence,
                last_seq,
                identity_key
            );
        }
        if !cp.state.is_terminal() {
            state = cp.state;
            proposal_ref = cp.proposal_ref.or(proposal_ref);
            last_seq = last_seq.max(cp.last_journal_sequence);
        }
    }

    Ok(Some(RecoveredEvaluation {
        state,
        last_journal_sequence: last_seq,
        proposal_ref,
        evidence_ref: None,
        recovered_from_checkpoint: false,
    }))
}

fn list_json(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::evaluate::journal::{append_event, journal_dir_for};

    fn repo_dir() -> PathBuf {
        tempfile::tempdir().unwrap().path().to_path_buf()
    }

    #[test]
    fn no_durable_state_returns_none() {
        let repo = repo_dir();
        assert!(recover_evaluation(&repo, "key-1").unwrap().is_none());
    }

    #[test]
    fn replay_reconstructs_state_from_journal() {
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
        append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::PreflightPassed,
            EvaluationState::Generating,
            None,
            None,
        )
        .unwrap();
        let recovered = recover_evaluation(&repo, "key-1").unwrap().unwrap();
        assert_eq!(recovered.state, EvaluationState::Generating);
        assert_eq!(recovered.last_journal_sequence, 1);
        assert!(!recovered.recovered_from_checkpoint);
        assert!(recovered.proposal_ref.is_none());
    }

    #[test]
    fn replay_carries_proposal_reference() {
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
        append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::PreflightPassed,
            EvaluationState::Generating,
            None,
            None,
        )
        .unwrap();
        append_event(
            &repo,
            "run-1",
            "key-1",
            EvaluationState::Generating,
            EvaluationState::ProposalGenerated,
            Some("proposal-9".to_string()),
            None,
        )
        .unwrap();
        let recovered = recover_evaluation(&repo, "key-1").unwrap().unwrap();
        assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
        assert_eq!(recovered.proposal_ref.as_deref(), Some("proposal-9"));
    }

    #[test]
    fn illegal_journal_sequence_fails_closed() {
        let repo = repo_dir();
        // Write an illegal jump (Created -> ReviewGate) by hand, bypassing the
        // transition law, to simulate a corrupted/forged journal.
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        let event = serde_json::json!({
            "schema_version": "1.0.0",
            "event_id": "evt-0",
            "sequence": 0,
            "run_id": "run-1",
            "identity_key": "key-1",
            "timestamp": "2026-01-01T00:00:00Z",
            "from_state": "created",
            "to_state": "review_gate",
            "proposal_ref": null,
            "checkpoint_ref": null,
            "failure_classification": null,
        });
        std::fs::write(
            dir.join("00000000000000000000.json"),
            serde_json::to_string_pretty(&event).unwrap(),
        )
        .unwrap();
        let err = recover_evaluation(&repo, "key-1").unwrap_err();
        assert!(err.to_string().contains("illegal transition"));
    }

    #[test]
    fn snapshot_ahead_of_journal_fails_closed() {
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
        // Checkpoint claims sequence 7, but the journal only reaches 0.
        let cp = crate::workflow::evaluate::checkpoint::build_checkpoint(
            &repo,
            "key-1",
            "run-1",
            "abc",
            EvaluationState::Validating,
            7,
            Some("proposal-1".to_string()),
            None,
            "run-1",
            0,
        );
        crate::workflow::evaluate::checkpoint::write_checkpoint(&repo, &cp).unwrap();
        let err = recover_evaluation(&repo, "key-1").unwrap_err();
        assert!(err.to_string().contains("ahead of journal"));
    }

    #[test]
    fn checkpoint_only_recovery_returns_state() {
        let repo = repo_dir();
        let cp = crate::workflow::evaluate::checkpoint::build_checkpoint(
            &repo,
            "key-1",
            "run-1",
            "abc",
            EvaluationState::IntegrityVerified,
            3,
            Some("proposal-1".to_string()),
            None,
            "run-1",
            0,
        );
        crate::workflow::evaluate::checkpoint::write_checkpoint(&repo, &cp).unwrap();
        let recovered = recover_evaluation(&repo, "key-1").unwrap().unwrap();
        assert!(recovered.recovered_from_checkpoint);
        assert_eq!(recovered.state, EvaluationState::IntegrityVerified);
        assert_eq!(recovered.last_journal_sequence, 3);
    }
}
