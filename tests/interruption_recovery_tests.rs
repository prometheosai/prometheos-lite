//! Interruption-recovery disposition tests (issue #114).
//!
//! Pure durable-state tests: no git repository, no provider. Each test builds a
//! journal of durable state transitions (the authoritative source) plus an
//! optional registry entry, then asserts the exact disposition the pipeline
//! would derive. These complement the integration tests in
//! `workflow_evaluate_tests.rs` by covering the full disposition matrix.

use std::path::{Path, PathBuf};

use prometheos_lite::workflow::evaluate::{
    EvaluationState, LeaseConfig, ProposalState, RecoveredEvaluation, RecoveryDisposition,
    RegistryEntry, determine_recovery_disposition, recover_evaluation,
};

fn repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    (dir, repo)
}

fn write_event(
    repo: &Path,
    identity_key: &str,
    sequence: u64,
    from: EvaluationState,
    to: EvaluationState,
    proposal_ref: Option<&str>,
    classification: Option<&str>,
) {
    let dir = repo
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join(identity_key);
    std::fs::create_dir_all(&dir).unwrap();
    let event = serde_json::json!({
        "schema_version": "1.0.0",
        "event_id": format!("event-{sequence}"),
        "sequence": sequence,
        "run_id": "run-1",
        "identity_key": identity_key,
        "timestamp": "2026-01-01T00:00:00Z",
        "from_state": serde_json::to_value(from).unwrap(),
        "to_state": serde_json::to_value(to).unwrap(),
        "proposal_ref": proposal_ref,
        "failure_classification": classification,
        "owner_run_id": "run-1",
        "lease_epoch": 1,
        "repository_revision": "abc123",
        "evidence_ref": null,
        "checkpoint_ref": null,
    });
    std::fs::write(
        dir.join(format!("{sequence:020}.json")),
        serde_json::to_string_pretty(&event).unwrap(),
    )
    .unwrap();
}

fn journal_up_to(repo: &Path, key: &str, target: EvaluationState) -> u64 {
    let mut seq = 0;
    write_event(
        repo,
        key,
        seq,
        EvaluationState::Created,
        EvaluationState::PreflightPassed,
        None,
        None,
    );
    seq += 1;
    write_event(
        repo,
        key,
        seq,
        EvaluationState::PreflightPassed,
        EvaluationState::Generating,
        None,
        None,
    );
    seq += 1;
    write_event(
        repo,
        key,
        seq,
        EvaluationState::Generating,
        EvaluationState::ProposalGenerated,
        Some("proposal-1"),
        None,
    );
    seq += 1;
    match target {
        EvaluationState::ProposalGenerated => {}
        _ => {
            write_event(
                repo,
                key,
                seq,
                EvaluationState::ProposalGenerated,
                EvaluationState::Validating,
                Some("proposal-1"),
                None,
            );
            seq += 1;
        }
    }
    seq - 1
}

fn stale_entry(state: ProposalState) -> RegistryEntry {
    RegistryEntry {
        state,
        owner_run_id: "run-1".to_string(),
        lease_epoch: 1,
        heartbeat_at: "2020-01-01T00:00:00Z".to_string(),
        ..Default::default()
    }
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

// ---------------------------------------------------------------------------
// Journal recovery (recover_evaluation)
// ---------------------------------------------------------------------------

#[test]
fn interrupted_during_validation_recovers_to_validating() {
    let (_dir, repo) = repo();
    let key = "key-1";
    journal_up_to(&repo, key, EvaluationState::Validating);

    let recovered = recover_evaluation(&repo, key, None, None)
        .unwrap()
        .expect("journal must exist");
    assert_eq!(recovered.state, EvaluationState::Validating);
    assert_eq!(recovered.proposal_ref.as_deref(), Some("proposal-1"));
    assert_eq!(recovered.last_journal_sequence, 3);
    assert!(!recovered.recovered_from_checkpoint);
}

#[test]
fn interrupted_during_generation_recovers_to_generating() {
    let (_dir, repo) = repo();
    let key = "key-1";
    journal_up_to(&repo, key, EvaluationState::ProposalGenerated);
    // Journal ends at ProposalGenerated; recovery reports exactly that.
    let recovered = recover_evaluation(&repo, key, None, None)
        .unwrap()
        .expect("journal must exist");
    assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
}

#[test]
fn cancelled_event_is_preserved_in_recovery() {
    let (_dir, repo) = repo();
    let key = "key-1";
    journal_up_to(&repo, key, EvaluationState::Validating);
    // A cooperative cancellation is a same-state event with classification.
    let seq = 4;
    write_event(
        &repo,
        key,
        seq,
        EvaluationState::Validating,
        EvaluationState::Validating,
        Some("proposal-1"),
        Some("cancelled"),
    );
    let recovered = recover_evaluation(&repo, key, None, None)
        .unwrap()
        .expect("journal must exist");
    assert_eq!(recovered.state, EvaluationState::Validating);
    assert_eq!(
        recovered.last_failure_classification.as_deref(),
        Some("cancelled")
    );
    assert_eq!(recovered.last_journal_sequence, seq);
}

// ---------------------------------------------------------------------------
// Disposition matrix (determine_recovery_disposition)
// ---------------------------------------------------------------------------

fn recovered(state: EvaluationState, classification: Option<&str>) -> RecoveredEvaluation {
    RecoveredEvaluation {
        state,
        last_journal_sequence: 1,
        proposal_ref: if matches!(
            state,
            EvaluationState::ProposalGenerated
                | EvaluationState::GovernancePassed
                | EvaluationState::Validating
        ) {
            Some("proposal-1".to_string())
        } else {
            None
        },
        evidence_ref: None,
        last_failure_classification: classification.map(|s| s.to_string()),
        recovered_from_checkpoint: false,
    }
}

#[test]
fn terminal_journal_returns_preserved_evidence() {
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::ReviewGate, None),
        Some(&stale_entry(ProposalState::Validating)),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::ReturnTerminalEvidence);
}

#[test]
fn generating_interruption_fails_closed_regardless_of_owner() {
    // With no entry and with a stale entry: outcome is unknown either way.
    for entry in [None, Some(stale_entry(ProposalState::Generating))] {
        let d = determine_recovery_disposition(
            &recovered(EvaluationState::Generating, None),
            entry.as_ref(),
            &LeaseConfig::default(),
            now(),
        );
        assert_eq!(d, RecoveryDisposition::GenerationOutcomeUnknown);
    }
}

#[test]
fn cancelled_generating_is_not_resumable() {
    // A cancellation that raced generation has an unknown outcome too.
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::Generating, Some("cancelled")),
        Some(&stale_entry(ProposalState::Generating)),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::GenerationOutcomeUnknown);
}

#[test]
fn cancelled_proposal_generated_resumes_from_proposal() {
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::ProposalGenerated, Some("cancelled")),
        Some(&stale_entry(ProposalState::ProposalGenerated)),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::ResumeFromProposal);
}

#[test]
fn cancelled_validating_resumes_validation() {
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::Validating, Some("cancelled")),
        Some(&stale_entry(ProposalState::Validating)),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::ResumeValidation);
}

#[test]
fn governance_passed_resumes_validation_after_crash() {
    // Latent bug fix: a crash at GovernancePassed must not fall into the
    // terminal branch — it resumes validation.
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::GovernancePassed, None),
        Some(&stale_entry(ProposalState::Validating)),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::ResumeValidation);
}

#[test]
fn live_owner_is_never_reclaimed() {
    // Fresh heartbeat on ProposalGenerated blocks reclaim even though the
    // journal says ProposalGenerated: binding acceptance for #114.
    let mut entry = stale_entry(ProposalState::ProposalGenerated);
    entry.heartbeat_at = "2026-01-01T00:00:00Z".to_string();
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::ProposalGenerated, None),
        Some(&entry),
        &LeaseConfig::default(),
        now(),
    );
    assert_eq!(d, RecoveryDisposition::WaitForLiveOwner);
}

#[test]
fn malformed_registry_timestamp_fails_closed() {
    let mut entry = stale_entry(ProposalState::Generating);
    entry.heartbeat_at = "not-a-timestamp".to_string();
    let d = determine_recovery_disposition(
        &recovered(EvaluationState::ProposalGenerated, None),
        Some(&entry),
        &LeaseConfig::default(),
        now(),
    );
    match d {
        RecoveryDisposition::FailClosed(msg) => assert!(msg.contains("liveness")),
        other => panic!("expected FailClosed, got {other:?}"),
    }
}
