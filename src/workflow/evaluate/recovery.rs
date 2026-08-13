//! Recovery of evaluation state from durable artifacts.
//!
//! When a run is restarted (crash, kill, reboot), the durable state journal
//! and checkpoints let us reconstruct where the evaluation stopped. The
//! journal is the authoritative source of truth; a checkpoint is trusted only
//! as far as it agrees with the journal. Recovery never trusts a possibly
//! corrupt identity file on its own, never rolls back to an older checkpoint,
//! and repairs lagging snapshots only under the exclusive registry lock.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use super::checkpoint::{build_checkpoint, checkpoint_path_for, read_checkpoint, write_checkpoint};
use super::identity::{EvaluationState, ExecutionIdentity, GovernanceScopeSnapshot};
use super::journal::{journal_dir_for, read_journal, validate_journal};
use super::migration::migrate_document;
use super::registry::FenceToken;
use super::schema::DocumentType;

/// The reconstructed state of an evaluation after recovery.
#[derive(Debug, Clone)]
pub struct RecoveredEvaluation {
    /// Replayed evaluation state (journal-authoritative).
    pub state: EvaluationState,
    /// Highest journal sequence replayed.
    pub last_journal_sequence: u64,
    /// Proposal reference, if the proposal was generated.
    pub proposal_ref: Option<String>,
    /// Evidence reference from the newest event that recorded one
    /// (repo-relative path to the evidence dir or `evidence.json`).
    pub evidence_ref: Option<String>,
    /// True if the state was reconstructed from a checkpoint instead of the
    /// journal. Always `false` in the current format: a checkpoint without a
    /// journal fails closed.
    pub recovered_from_checkpoint: bool,
}

/// Gate a resume on the recovered durable state.
///
/// Fails closed when the journal records a terminal outcome (the run already
/// finished and must not be resumed) or when the recovered proposal reference
/// disagrees with the proposal being resumed.
pub fn ensure_resumable(
    _resumed_from: EvaluationState,
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
    Ok(())
}

/// Resolve a repo-relative `evidence_ref` to its evidence directory.
pub(super) fn resolve_evidence_dir(repo: &Path, evidence_ref: &str) -> Option<PathBuf> {
    let resolved = super::durable::resolve_repo_relative(repo, evidence_ref).ok()?;
    if resolved.file_name().and_then(|n| n.to_str()) == Some("evidence.json") {
        resolved.parent().map(|d| d.to_path_buf())
    } else {
        Some(resolved)
    }
}

/// Repair the lagging identity/checkpoint snapshots to match the authoritative
/// journal position. Runs only under the exclusive registry lock and only when
/// a fence (ownership) is provided.
fn repair_snapshots_under_lock(
    repo: &Path,
    identity_key: &str,
    run_id: &str,
    repository_revision: &str,
    state: EvaluationState,
    last_seq: u64,
    proposal_ref: Option<String>,
    evidence_dir: &Path,
    fence: &FenceToken,
) -> Result<()> {
    super::registry::with_registry_lock(repo, |_registry| {
        // Repair the identity document (best-effort derived view; a failure
        // here fails closed rather than leaving a stale snapshot). A missing
        // identity snapshot is created so the entry is fully reconstructable.
        let identity_path = evidence_dir.join("execution_identity.json");
        if identity_path.exists() {
            super::identity::update_identity_state(&identity_path, state)
                .context("failed to repair lagging identity snapshot")?;
        } else {
            let identity = ExecutionIdentity {
                run_id: run_id.to_string(),
                task_id: run_id.to_string(),
                repo: repo.display().to_string(),
                repo_pin: repository_revision.to_string(),
                model: "unknown".to_string(),
                provider: "unknown".to_string(),
                governance_scope: GovernanceScopeSnapshot {
                    allowed_paths: vec![],
                    forbidden_paths: vec![],
                    allow_dependency_changes: false,
                    max_files_changed: None,
                    max_lines_changed: None,
                    authority: "propose".to_string(),
                    validation_command: None,
                },
                created_at: super::identity::now_iso(),
                state,
            };
            super::identity::persist_execution_identity(evidence_dir, &identity)
                .context("failed to create missing identity snapshot")?;
        }
        let evidence_ref =
            super::durable::repo_relative_path(repo, &evidence_dir.join("evidence.json"));
        let checkpoint = build_checkpoint(
            repo,
            identity_key,
            run_id,
            repository_revision,
            state,
            last_seq,
            proposal_ref,
            Some(evidence_ref),
            &fence.owner_run_id,
            fence.lease_epoch,
        );
        write_checkpoint(repo, &checkpoint).context("failed to create or repair checkpoint")?;
        Ok(())
    })
}

/// Recover the durable state of an evaluation for `identity_key`.
///
/// Returns `None` when no journal exists yet (the run has not reached any
/// durable transition). When `fence` is provided the caller owns the entry and
/// lagging snapshots are repaired under the exclusive registry lock; a
/// read-only probe (no repair) uses `fence == None`.
///
/// Replay is authoritative:
/// - A first event anchors `from_state`; each subsequent event's `from_state`
///   must equal the previously reconstructed state.
/// - Event filename sequence must equal the event's sequence; the identity key
///   must match.
/// - The repository revision must stay constant across the run (`expected_revision`
///   is compared against the durable revision, when supplied).
/// - A checkpoint ahead of the journal fails closed (snapshot-ahead).
/// - A checkpoint at the journal end but disagreeing on state/reference fails closed.
/// - A lagging checkpoint is ignored for replay (the journal wins) and repaired
///   under the lock when a fence is provided.
/// - A terminal state requires a durable evidence reference that exists.
pub fn recover_evaluation(
    repo: &Path,
    identity_key: &str,
    expected_revision: Option<&str>,
    fence: Option<&FenceToken>,
) -> Result<Option<RecoveredEvaluation>> {
    // ------------------------------------------------------------------
    // 1. Migrate any legacy durable documents.
    // ------------------------------------------------------------------
    let journal_dir = journal_dir_for(repo, identity_key);
    let checkpoint_path = checkpoint_path_for(repo, identity_key);
    let registry_path = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");

    if journal_dir.exists() {
        for path in list_json(&journal_dir)? {
            migrate_document(&path, DocumentType::JournalEvent)?;
        }
    }
    if checkpoint_path.exists() {
        migrate_document(&checkpoint_path, DocumentType::EvaluationCheckpoint)?;
    }
    if registry_path.exists() {
        migrate_document(&registry_path, DocumentType::ProposalRegistry)?;
    }

    // A legacy checkpoint may still point at the (old) identity path; use it
    // only to discover the evidence directory for migration.
    let mut evidence_dir: Option<PathBuf> = None;
    if let Some(cp) = read_checkpoint(repo, identity_key)?
        && let Some(ref er) = cp.evidence_ref
        && let Some(dir) = resolve_evidence_dir(repo, er)
    {
        evidence_dir = Some(dir);
    }

    // ------------------------------------------------------------------
    // 2 + 3. Validate and replay the journal (authoritative).
    // ------------------------------------------------------------------
    let events = read_journal(repo, identity_key)?;
    if events.is_empty() {
        // No journal: either this run never reached a durable transition, or a
        // checkpoint exists without its journal (fail closed unless the
        // checkpoint is legacy and the caller explicitly migrates it).
        if read_checkpoint(repo, identity_key)?.is_some() {
            bail!(
                "checkpoint present without journal for {identity_key}: \
                 refusing to reconstruct from a checkpoint alone"
            );
        }
        return Ok(None);
    }
    validate_journal(&journal_dir)?;

    let mut state = events[0].from_state;
    let mut last_seq = events[0].sequence;
    let mut proposal_ref: Option<String> = None;
    let mut evidence_ref: Option<String> = None;
    let mut revision: Option<String> = None;

    for (index, event) in events.iter().enumerate() {
        if index > 0 && event.from_state != state {
            bail!(
                "journal replay from_state mismatch at sequence {}: \
                 previous state was {state:?}, event records {:?}",
                event.sequence,
                event.from_state,
            );
        }
        super::transition::validate_transition(state, event.to_state).with_context(|| {
            format!(
                "journal replay hit illegal transition at sequence {}: {:?} -> {:?}",
                event.sequence, state, event.to_state
            )
        })?;
        if let Some(prev) = &revision {
            if *prev != event.repository_revision {
                bail!(
                    "repository revision changed mid-run at sequence {}: \
                     {prev} -> {}",
                    event.sequence,
                    event.repository_revision
                );
            }
        } else {
            revision = Some(event.repository_revision.clone());
        }
        state = event.to_state;
        last_seq = event.sequence;
        if event.proposal_ref.is_some() {
            proposal_ref = event.proposal_ref.clone();
        }
        if event.evidence_ref.is_some() {
            evidence_ref = event.evidence_ref.clone();
        }
    }

    // Derive the evidence directory from the journal when not already known.
    if evidence_dir.is_none()
        && let Some(ref er) = evidence_ref
    {
        evidence_dir = resolve_evidence_dir(repo, er);
    }

    // Migrate derived documents: identity in place, evidence validated in
    // memory (never blindly rewritten).
    let repair_identity_path = evidence_dir
        .as_ref()
        .map(|d| d.join("execution_identity.json"));
    if let Some(p) = &repair_identity_path
        && p.exists()
    {
        migrate_document(p, DocumentType::ExecutionIdentity)?;
    }
    if let Some(dir) = &evidence_dir {
        let evidence = dir.join("evidence.json");
        if evidence.exists() {
            migrate_document(&evidence, DocumentType::EvidenceBundle)?;
        }
    }

    // 4. Repository revision gate: the caller supplies the current revision the
    //    run is being resumed against; it must match the run's durable revision.
    if let Some(rev) = expected_revision
        && let Some(durable) = revision.as_deref()
        && durable != rev
    {
        bail!(
            "repository revision mismatch: durable run revision is {durable}, \
             current revision is {rev}"
        );
    }

    // 5. Checkpoint cross-check.
    let existing_cp = read_checkpoint(repo, identity_key)?;
    let lagging = if let Some(ref cp) = existing_cp {
        if cp.last_journal_sequence > last_seq {
            bail!(
                "checkpoint sequence {} is ahead of journal sequence {} for {} \
                 (snap-forged corruption)",
                cp.last_journal_sequence,
                last_seq,
                identity_key
            );
        }
        if cp.last_journal_sequence == last_seq {
            if cp.state != state {
                bail!(
                    "checkpoint state {:?} disagrees with journal state {state:?} \
                     at sequence {last_seq}",
                    cp.state
                );
            }
            if let (Some(cp_proposal), Some(journal_proposal)) =
                (cp.proposal_ref.as_ref(), proposal_ref.as_ref())
                && cp_proposal != journal_proposal
            {
                bail!(
                    "checkpoint proposal {cp_proposal} disagrees with journal proposal \
                     {journal_proposal} at sequence {last_seq}"
                );
            }
        }
        // Sequence < journal: journal wins; the checkpoint lags and is repaired
        // under the lock below.
        cp.last_journal_sequence < last_seq
    } else {
        false
    };
    // A missing checkpoint is also repaired (created) under the lock below.
    let checkpoint_missing = existing_cp.is_none();

    // 6. Terminal evidence requirement: a terminal state must reference
    //    durable evidence that exists and validates.
    if state.is_terminal() {
        let dir = evidence_dir
            .as_ref()
            .context("terminal journal state has no evidence reference")?;
        let evidence = dir.join("evidence.json");
        if !evidence.exists() {
            bail!(
                "terminal state {state:?} requires durable evidence {} which is missing",
                evidence.display()
            );
        }
        migrate_document(&evidence, DocumentType::EvidenceBundle)?;
    }

    // 7. Repair lagging (or missing) identity/checkpoint snapshots under the
    //    exclusive lock, only when the caller owns the entry.
    if let (Some(fence), Some(dir)) = (fence, evidence_dir.as_ref())
        && (lagging || checkpoint_missing)
    {
        let run_id = events.last().map(|e| e.run_id.clone()).unwrap_or_default();
        repair_snapshots_under_lock(
            repo,
            identity_key,
            &run_id,
            revision.as_deref().unwrap_or("unknown"),
            state,
            last_seq,
            proposal_ref.clone(),
            dir,
            fence,
        )?;
    }

    Ok(Some(RecoveredEvaluation {
        state,
        last_journal_sequence: last_seq,
        proposal_ref,
        evidence_ref,
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
    use crate::workflow::evaluate::journal::{append_event_unlocked, journal_dir_for};

    fn repo_dir() -> PathBuf {
        tempfile::tempdir().unwrap().path().to_path_buf()
    }

    #[test]
    fn no_durable_state_returns_none() {
        let repo = repo_dir();
        assert!(
            recover_evaluation(&repo, "key-1", None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn replay_reconstructs_state_from_journal() {
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
        append_event_unlocked(
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
        let recovered = recover_evaluation(&repo, "key-1", None, None)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.state, EvaluationState::Generating);
        assert_eq!(recovered.last_journal_sequence, 1);
        assert!(!recovered.recovered_from_checkpoint);
        assert!(recovered.proposal_ref.is_none());
    }

    #[test]
    fn replay_carries_proposal_and_evidence_reference() {
        let repo = repo_dir();
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
                &repo,
                "run-1",
                "key-1",
                from,
                to,
                if to == EvaluationState::ProposalGenerated {
                    Some("proposal-9".to_string())
                } else {
                    None
                },
                None,
                "run-1",
                1,
                "abc123",
                if to == EvaluationState::ProposalGenerated {
                    Some("prometheos/evidence/run-1".to_string())
                } else {
                    None
                },
            )
            .unwrap();
        }
        let recovered = recover_evaluation(&repo, "key-1", None, None)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
        assert_eq!(recovered.proposal_ref.as_deref(), Some("proposal-9"));
        assert_eq!(
            recovered.evidence_ref.as_deref(),
            Some("prometheos/evidence/run-1")
        );
    }

    #[test]
    fn from_state_mismatch_fails_closed() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        let ev0 = serde_json::json!({
            "schema_version": "1.0.0", "event_id": "evt-0", "sequence": 0,
            "run_id": "run-1", "identity_key": "key-1", "timestamp": "2026-01-01T00:00:00Z",
            "from_state": "created", "to_state": "preflight_passed", "proposal_ref": null,
            "failure_classification": null, "owner_run_id": "run-1", "lease_epoch": 1,
            "repository_revision": "abc", "evidence_ref": null, "checkpoint_ref": null
        });
        let ev1 = serde_json::json!({
            "schema_version": "1.0.0", "event_id": "evt-1", "sequence": 1,
            "run_id": "run-1", "identity_key": "key-1", "timestamp": "2026-01-01T00:00:01Z",
            // from_state is wrong: should be preflight_passed.
            "from_state": "created", "to_state": "generating", "proposal_ref": null,
            "failure_classification": null, "owner_run_id": "run-1", "lease_epoch": 1,
            "repository_revision": "abc", "evidence_ref": null, "checkpoint_ref": null
        });
        std::fs::write(
            dir.join("00000000000000000000.json"),
            serde_json::to_string_pretty(&ev0).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("00000000000000000001.json"),
            serde_json::to_string_pretty(&ev1).unwrap(),
        )
        .unwrap();
        let err = recover_evaluation(&repo, "key-1", None, None).unwrap_err();
        assert!(err.to_string().contains("from_state"));
    }

    #[test]
    fn illegal_journal_sequence_fails_closed() {
        let repo = repo_dir();
        let dir = journal_dir_for(&repo, "key-1");
        std::fs::create_dir_all(&dir).unwrap();
        let event = serde_json::json!({
            "schema_version": "1.0.0", "event_id": "evt-0", "sequence": 0,
            "run_id": "run-1", "identity_key": "key-1", "timestamp": "2026-01-01T00:00:00Z",
            "from_state": "created", "to_state": "review_gate", "proposal_ref": null,
            "failure_classification": null, "owner_run_id": "run-1", "lease_epoch": 1,
            "repository_revision": "abc123", "evidence_ref": null, "checkpoint_ref": null
        });
        std::fs::write(
            dir.join("00000000000000000000.json"),
            serde_json::to_string_pretty(&event).unwrap(),
        )
        .unwrap();
        let err = recover_evaluation(&repo, "key-1", None, None).unwrap_err();
        assert!(err.to_string().contains("illegal transition"));
    }

    #[test]
    fn snapshot_ahead_of_journal_fails_closed() {
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
        let cp = build_checkpoint(
            &repo,
            "key-1",
            "run-1",
            "abc",
            EvaluationState::Validating,
            7,
            Some("proposal-1".to_string()),
            Some("evidence/run-1".to_string()),
            "run-1",
            0,
        );
        write_checkpoint(&repo, &cp).unwrap();
        let err = recover_evaluation(&repo, "key-1", None, None).unwrap_err();
        assert!(err.to_string().contains("ahead of journal"));
    }

    #[test]
    fn lagging_checkpoint_cannot_roll_back_journal() {
        let repo = repo_dir();
        // Journal reaches ProposalGenerated (seq 2); checkpoint lags at seq 1.
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
        // Lagging checkpoint claims Generating at seq 1.
        let cp = build_checkpoint(
            &repo,
            "key-1",
            "run-1",
            "abc123",
            EvaluationState::Generating,
            1,
            None,
            None,
            "run-1",
            0,
        );
        write_checkpoint(&repo, &cp).unwrap();
        let recovered = recover_evaluation(&repo, "key-1", None, None)
            .unwrap()
            .unwrap();
        // The journal wins: recovery stays at ProposalGenerated, NOT Generating.
        assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
        assert_eq!(recovered.last_journal_sequence, 2);
    }

    #[test]
    fn checkpoint_only_recovery_fails_closed() {
        let repo = repo_dir();
        let cp = build_checkpoint(
            &repo,
            "key-1",
            "run-1",
            "abc",
            EvaluationState::IntegrityVerified,
            3,
            Some("proposal-1".to_string()),
            Some("evidence/run-1".to_string()),
            "run-1",
            0,
        );
        write_checkpoint(&repo, &cp).unwrap();
        let err = recover_evaluation(&repo, "key-1", None, None).unwrap_err();
        assert!(err.to_string().contains("without journal"));
    }

    #[test]
    fn repository_revision_mismatch_fails_closed() {
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
            "abc-123",
            None,
        )
        .unwrap();
        let err = recover_evaluation(&repo, "key-1", Some("different-rev"), None).unwrap_err();
        assert!(err.to_string().contains("revision mismatch"));
    }

    #[test]
    fn missing_checkpoint_is_created_on_recovery() {
        let repo = repo_dir();
        let evidence_dir = repo.join(".prometheos").join("evidence").join("run-1");
        std::fs::create_dir_all(&evidence_dir).unwrap();
        // A portable repo-relative evidence reference (the evidence directory
        // itself exists; no evidence file is required for the checkpoint
        // creation repair under test).
        let evidence_ref = ".prometheos/evidence/run-1".to_string();

        // A valid journal ending in a non-terminal, evidence-bearing state.
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
            (
                EvaluationState::ProposalGenerated,
                EvaluationState::GovernancePassed,
            ),
            (
                EvaluationState::GovernancePassed,
                EvaluationState::Validating,
            ),
            (
                EvaluationState::Validating,
                EvaluationState::ValidationComplete,
            ),
        ] {
            let is_last = to == EvaluationState::ValidationComplete;
            append_event_unlocked(
                &repo,
                "run-1",
                "key-1",
                from,
                to,
                if to == EvaluationState::ProposalGenerated {
                    Some("proposal-9".to_string())
                } else {
                    None
                },
                None,
                "run-1",
                1,
                "abc123",
                if is_last {
                    Some(evidence_ref.clone())
                } else {
                    None
                },
            )
            .unwrap();
        }

        // Persist a matching identity so the repair path can update it.
        let identity = crate::workflow::evaluate::identity::ExecutionIdentity {
            run_id: "run-1".to_string(),
            task_id: "task-1".to_string(),
            repo: repo.display().to_string(),
            repo_pin: "abc123".to_string(),
            model: "mock".to_string(),
            provider: "mock".to_string(),
            governance_scope: crate::workflow::evaluate::identity::GovernanceScopeSnapshot {
                allowed_paths: vec![],
                forbidden_paths: vec![],
                allow_dependency_changes: false,
                max_files_changed: None,
                max_lines_changed: None,
                authority: "propose".to_string(),
                validation_command: None,
            },
            created_at: crate::workflow::evaluate::identity::now_iso(),
            state: EvaluationState::ValidationComplete,
        };
        crate::workflow::evaluate::identity::persist_execution_identity(&evidence_dir, &identity)
            .unwrap();

        // No checkpoint exists yet.
        assert!(
            !crate::workflow::evaluate::checkpoint::checkpoint_path_for(&repo, "key-1").exists()
        );

        // Recover with a fence so the missing checkpoint is created under lock.
        let fence = crate::workflow::evaluate::registry::FenceToken {
            owner_run_id: "run-1".to_string(),
            lease_epoch: 1,
        };
        let recovered = recover_evaluation(&repo, "key-1", None, Some(&fence))
            .unwrap()
            .unwrap();
        assert_eq!(recovered.state, EvaluationState::ValidationComplete);
        assert_eq!(recovered.last_journal_sequence, 5);

        // The missing checkpoint must now exist and match the journal.
        let cp = crate::workflow::evaluate::checkpoint::read_checkpoint(&repo, "key-1")
            .unwrap()
            .unwrap();
        assert_eq!(cp.state, EvaluationState::ValidationComplete);
        assert_eq!(cp.last_journal_sequence, 5);
    }

    #[test]
    fn terminal_event_without_evidence_fails_closed() {
        let repo = repo_dir();
        // A full chain ending in the terminal ReviewGate, but the terminal event
        // carries no evidence_ref and no evidence.json exists. Recovery must
        // fail closed rather than reconstruct a terminal state with no evidence.
        let chain = [
            (EvaluationState::Created, EvaluationState::PreflightPassed),
            (
                EvaluationState::PreflightPassed,
                EvaluationState::Generating,
            ),
            (
                EvaluationState::Generating,
                EvaluationState::ProposalGenerated,
            ),
            (
                EvaluationState::ProposalGenerated,
                EvaluationState::Validating,
            ),
            (
                EvaluationState::Validating,
                EvaluationState::ValidationComplete,
            ),
            (
                EvaluationState::ValidationComplete,
                EvaluationState::IntegrityVerified,
            ),
            (
                EvaluationState::IntegrityVerified,
                EvaluationState::ReviewGate,
            ),
        ];
        for (from, to) in chain {
            append_event_unlocked(
                &repo, "run-1", "key-1", from, to, None, None, "run-1", 1, "abc", None,
            )
            .unwrap();
        }
        let err = recover_evaluation(&repo, "key-1", None, None).unwrap_err();
        assert!(err.to_string().contains("evidence"));
    }
}
