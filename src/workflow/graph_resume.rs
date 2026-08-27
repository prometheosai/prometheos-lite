//! Resumable graph execution + whole-run cancellation (issue #123).
//!
//! Built on the durable state law in [`super::graph_state`]: [`resume_run`] imports
//! a checkpoint (which already fails closed on digest/tamper/version/graph-revision)
//! and THEN enforces two resume-specific gates — stale REPOSITORY revision and
//! unauthorized replay of completed nodes. [`cancel_run`] reconciles in-flight
//! attempts, records a terminal outcome, and clears the frontier so nothing is
//! reopened or lost; subsequent resume refuses with a typed contradiction.

use anyhow::{Context as _, Result};

use super::graph_state::{
    GraphManifestV1, GraphRunStateV1, ImplChangeV1, OutcomeCategory, RunTerminationV1,
};

/// Resume-specific typed contradictions (beyond [`super::graph_state::GraphRunError`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeError {
    /// Run pinned a different repository revision than the current checkout.
    StaleRepoRevision,
    /// Resume would re-execute a node that already completed, without authorization.
    ReplayRequiresAuthorization,
    /// Run has already been terminated; no further routing allowed.
    RunAlreadyTerminated,
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ResumeError::StaleRepoRevision => "checkpoint pinned a different repository revision",
            ResumeError::ReplayRequiresAuthorization => {
                "resume would replay a completed node without replay authorization"
            }
            ResumeError::RunAlreadyTerminated => "graph run already terminated",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ResumeError {}

/// Authorization to replay already-completed nodes during resume.
#[derive(Debug, Clone, Default)]
pub struct ReplayAuthorization {
    pub reason: String,
    pub authorized_by: String,
}

/// Outcome of a successful resume.
#[derive(Debug)]
pub struct ResumedRun {
    pub state: GraphRunStateV1,
    /// Count of interrupted (in-flight) attempts safely closed.
    pub reconciled_attempts: usize,
    /// True when replay authorization was consumed (a completed node re-enters).
    pub replay_authorized: bool,
}

/// Resume a run from a durable checkpoint.
pub fn resume_run(
    checkpoint_text: &str,
    manifest: &GraphManifestV1,
    expected_portable_digest: &str,
    repo_revision_now: &str,
    replay: Option<ReplayAuthorization>,
) -> Result<ResumedRun> {
    let mut state =
        GraphRunStateV1::import_checkpoint(checkpoint_text, manifest, expected_portable_digest)
            .context("checkpoint import failed")?;

    if state.termination.is_some() {
        return Err(ResumeError::RunAlreadyTerminated.into());
    }
    // Gate: stale repository revision (separate from the graph-manifest revision
    // gate enforced inside import_checkpoint).
    if state.repo_revision != repo_revision_now {
        return Err(ResumeError::StaleRepoRevision.into());
    }

    // Gate: unauthorized replay of already-completed nodes. A node on the
    // frontier that already holds a Completed attempt would be re-executed;
    // require explicit authorization (recorded in the evidence trail).
    let frontier_replays_completed = state.frontier.iter().any(|n| node_has_completed(&state, n));
    let replay_authorized = if frontier_replays_completed {
        match replay {
            Some(auth)
                if !auth.reason.trim().is_empty() && !auth.authorized_by.trim().is_empty() =>
            {
                // Authorization accepted. Record the authorization truthfully:
                // real canonical digests of the authorization record and the
                // resumed frontier (no fabricated zero digests, no free text in
                // the timestamp field). The returned state's digest is resealed
                // below so any caller-side save-after-resume round-trips.
                let auth_digest = crate::workflow::soma::canonical_digest(&serde_json::json!({
                    "kind": "replay-authorization",
                    "authorizedBy": auth.authorized_by,
                    "reason": auth.reason,
                }));
                let frontier_digest = crate::workflow::soma::canonical_digest(&serde_json::json!({
                    "runId": state.run_id,
                    "frontier": state.frontier,
                }));
                state
                    .evidence_refs
                    .push(super::memory_contracts::EvidenceReferenceV1 {
                        id: format!("replay-auth:{}", auth.authorized_by),
                        event_digest: auth_digest,
                        artifact_digest: frontier_digest,
                        artifact_kind: "replay-authorization".into(),
                        produced_by: auth.authorized_by.clone(),
                        produced_at: Some("resume-replay".into()),
                    });
                true
            }
            _ => return Err(ResumeError::ReplayRequiresAuthorization.into()),
        }
    } else {
        false
    };

    let reconciled_attempts = reconcile_interrupted(&mut state);
    // The evidence push above mutated the durable state; reseal unconditionally
    // so the returned checkpoint always round-trips (zero-reconciled/authorized
    // replay must not leave a stale content_digest that fails re-import).
    state.content_digest = Some(state.compute_digest());
    Ok(ResumedRun {
        state,
        reconciled_attempts,
        replay_authorized,
    })
}

/// True when `node` already holds a *finished* `Completed` attempt (a genuinely
/// completed node that would be re-executed on resume). An in-flight attempt
/// (no `completed_at`) is NOT "completed" and is reconciled instead.
fn node_has_completed(state: &GraphRunStateV1, node: &str) -> bool {
    state
        .node_attempts
        .get(node)
        .map(|ats| {
            ats.iter()
                .any(|a| a.outcome == OutcomeCategory::Completed && a.completed_at.is_some())
        })
        .unwrap_or(false)
}

/// Close in-flight attempts (started but never completed) as `Cancelled`,
/// preserving their evidence (started_at + result_digest intact; only
/// completed_at + outcome set). Returns the number of attempts closed. Frontier
/// membership is unchanged — the node remains eligible under normal caps.
pub fn reconcile_interrupted(state: &mut GraphRunStateV1) -> usize {
    let mut closed = 0usize;
    for attempts in state.node_attempts.values_mut() {
        for a in attempts.iter_mut() {
            if a.completed_at.is_none() {
                a.completed_at = Some("interrupted-reconciled".to_string());
                a.outcome = OutcomeCategory::Cancelled;
                closed += 1;
            }
        }
    }
    if closed > 0 {
        state.content_digest = Some(state.compute_digest());
    }
    closed
}

/// Whole-run cancellation: reconcile in-flight attempts, record a terminal
/// outcome, and clear the frontier. Decisions/attempts/evidence are preserved
/// verbatim — nothing reopened, nothing lost. Subsequent resume refuses via
/// [`ResumeError::RunAlreadyTerminated`].
pub fn cancel_run(state: &mut GraphRunStateV1, reason: &str, recorded_at: &str) -> Result<()> {
    if state.termination.is_some() {
        return Err(ResumeError::RunAlreadyTerminated.into());
    }
    reconcile_interrupted(state);
    state.termination = Some(RunTerminationV1 {
        kind: "cancelled".to_string(),
        reason: reason.to_string(),
        recorded_at: recorded_at.to_string(),
    });
    state.frontier.clear();
    state.content_digest = Some(state.compute_digest());
    Ok(())
}

/// Record a provider/model/harness swap with mandatory policy-evidence digest
/// (compatibility + policy evidence per acceptance). Missing/invalid evidence
/// refuses.
pub fn record_implementation_change(
    state: &mut GraphRunStateV1,
    kind: &str,
    from_id: &str,
    to_id: &str,
    policy_evidence_digest: &str,
    recorded_at: &str,
) -> Result<()> {
    if !matches!(kind, "provider" | "model" | "harness") {
        anyhow::bail!("implementation change kind must be provider|model|harness");
    }
    if !is_64hex(policy_evidence_digest) {
        anyhow::bail!("implementation change requires a 64-hex policy evidence digest");
    }
    if to_id.trim().is_empty() {
        anyhow::bail!("implementation change requires a destination id");
    }
    state.implementation_changes.push(ImplChangeV1 {
        kind: kind.to_string(),
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        policy_evidence_digest: policy_evidence_digest.to_string(),
        recorded_at: recorded_at.to_string(),
    });
    state.content_digest = Some(state.compute_digest());
    Ok(())
}

fn is_64hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::graph_state::{
        EdgeKind, GraphEdgeV1, GraphNodeV1, NodeAttemptRecordV1, OutcomeCategory,
    };

    fn manifest() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: super::super::graph_state::GRAPH_SCHEMA_VERSION.into(),
            graph_id: "g".into(),
            version: "1.0.0".into(),
            nodes: vec![
                GraphNodeV1 {
                    node_id: "a".into(),
                    capability: "c.a".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
                GraphNodeV1 {
                    node_id: "b".into(),
                    capability: "c.b".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
                GraphNodeV1 {
                    node_id: "done".into(),
                    capability: "c.done".into(),
                    purpose: None,
                    resources: Vec::new(),
                    join: None,
                },
            ],
            edges: vec![
                GraphEdgeV1 {
                    from: "a".into(),
                    to: "b".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
                GraphEdgeV1 {
                    from: "b".into(),
                    to: "done".into(),
                    kind: EdgeKind::Sequence,
                    condition_label: None,
                },
            ],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["done".into()],
            shared_state_keys: vec!["plan".into()],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    fn completed_attempt() -> NodeAttemptRecordV1 {
        NodeAttemptRecordV1 {
            attempt: 1,
            started_at: "t0".into(),
            completed_at: Some("t1".into()),
            outcome: OutcomeCategory::Completed,
            result_digest: "d".repeat(64),
        }
    }

    #[test]
    fn restart_resumes_from_durable_checkpoint() {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        run.apply_node_completion("a", completed_attempt(), "t1")
            .unwrap();
        run.record_route_decision(
            super::super::graph_state::RouteDecisionV1 {
                recorded_at: "t2".into(),
                from_node: "a".into(),
                to_node: "b".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        )
        .unwrap();
        let ckpt = run.export_checkpoint().unwrap();
        let resumed = resume_run(&ckpt, &m, "pd", "rev-1", None).expect("resumes");
        assert_eq!(resumed.state.frontier, vec!["b".to_string()]);
        assert_eq!(resumed.reconciled_attempts, 0);
    }

    #[test]
    fn completed_nodes_not_rerun_without_authorization() {
        let m = manifest();
        // Route a -> b -> back to a (cycle) so a completed node re-enters frontier.
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        run.apply_node_completion("a", completed_attempt(), "t1")
            .unwrap();
        run.record_route_decision(
            super::super::graph_state::RouteDecisionV1 {
                recorded_at: "t2".into(),
                from_node: "a".into(),
                to_node: "b".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        )
        .unwrap();
        run.apply_node_completion("b", completed_attempt(), "t3")
            .unwrap();
        // Route b -> a creates a cycle: "a" already completed, re-enters frontier.
        run.record_route_decision(
            super::super::graph_state::RouteDecisionV1 {
                recorded_at: "t4".into(),
                from_node: "b".into(),
                to_node: "a".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        )
        .unwrap();
        assert!(run.frontier.contains(&"a".to_string()));
        let ckpt = run.export_checkpoint().unwrap();

        // No authorization -> refuses.
        let err = resume_run(&ckpt, &m, "pd", "rev-1", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("replay"), "{err}");

        // With authorization -> allowed and recorded.
        let resumed = resume_run(
            &ckpt,
            &m,
            "pd",
            "rev-1",
            Some(ReplayAuthorization {
                reason: "demonstrate replay of completed node a".into(),
                authorized_by: "operator-1".into(),
            }),
        )
        .expect("authorized replay");
        assert!(resumed.replay_authorized);
        assert!(
            resumed
                .state
                .evidence_refs
                .iter()
                .any(|e| e.id.contains("operator-1"))
        );
    }

    #[test]
    fn interrupted_attempt_reconciled_as_cancelled() {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        // In-flight (no completed_at) attempt on frontier node "a".
        run.node_attempts.insert(
            "a".into(),
            vec![NodeAttemptRecordV1 {
                attempt: 1,
                started_at: "t0".into(),
                completed_at: None,
                outcome: OutcomeCategory::Completed, // naively set; reconcile fixes
                result_digest: "d".repeat(64),
            }],
        );
        run.content_digest = Some(run.compute_digest());
        let ckpt = run.export_checkpoint().unwrap();
        let resumed = resume_run(&ckpt, &m, "pd", "rev-1", None).expect("resumes");
        assert_eq!(resumed.reconciled_attempts, 1);
        let a = &resumed.state.node_attempts["a"][0];
        assert_eq!(a.outcome, OutcomeCategory::Cancelled);
        assert!(a.completed_at.is_some());
        // Evidence preserved: started_at and result_digest unchanged.
        assert_eq!(a.started_at, "t0");
        assert_eq!(a.result_digest, "d".repeat(64));
    }

    #[test]
    fn cancellation_records_terminal_outcome_and_blocks_resume() {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        run.apply_node_completion("a", completed_attempt(), "t1")
            .unwrap();
        let ckpt = run.export_checkpoint().unwrap();

        let mut resumed = resume_run(&ckpt, &m, "pd", "rev-1", None).unwrap().state;
        cancel_run(&mut resumed, "user requested", "t9").expect("cancel");
        assert!(resumed.frontier.is_empty());
        assert_eq!(resumed.termination.as_ref().unwrap().kind, "cancelled");
        // Evidence preserved across cancellation.
        assert_eq!(
            resumed.node_attempts["a"][0].outcome,
            OutcomeCategory::Completed
        );

        // Re-import the cancelled checkpoint and attempt resume -> refused.
        let cancelled_ckpt = resumed.export_checkpoint().unwrap();
        let err = resume_run(&cancelled_ckpt, &m, "pd", "rev-1", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("terminated"), "{err}");
    }

    #[test]
    fn stale_repo_revision_rejected() {
        let m = manifest();
        let run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let ckpt = run.export_checkpoint().unwrap();
        let err = resume_run(&ckpt, &m, "pd", "rev-2", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("repository revision"), "{err}");
    }

    #[test]
    fn implementation_change_requires_policy_evidence() {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        // Missing evidence digest -> refuses.
        let err =
            record_implementation_change(&mut run, "model", "old-model", "new-model", "", "t0")
                .unwrap_err()
                .to_string();
        assert!(err.contains("policy evidence"), "{err}");
        // Invalid kind -> refuses.
        assert!(
            record_implementation_change(&mut run, "oracle", "x", "y", &"e".repeat(64), "t0")
                .is_err()
        );
        // Valid -> recorded.
        record_implementation_change(
            &mut run,
            "model",
            "old-model",
            "new-model",
            &"e".repeat(64),
            "t0",
        )
        .expect("records change");
        assert_eq!(run.implementation_changes.len(), 1);
        assert_eq!(run.implementation_changes[0].to_id, "new-model");
    }

    #[test]
    fn resume_authorized_replay_roundtrips() {
        // The gate-caught defect: evidence push during authorized resume reseals
        // the chain; resumed checkpoint round-trips.
        let m = manifest();
        let mut run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        run.apply_node_completion("a", completed_attempt(), "t1")
            .unwrap();
        run.record_route_decision(
            super::super::graph_state::RouteDecisionV1 {
                recorded_at: "t2".into(),
                from_node: "a".into(),
                to_node: "b".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        )
        .unwrap();
        run.apply_node_completion("b", completed_attempt(), "t3")
            .unwrap();
        let _ = run.record_route_decision(
            super::super::graph_state::RouteDecisionV1 {
                recorded_at: "t4".into(),
                from_node: "b".into(),
                to_node: "a".into(),
                condition_label: None,
                basis_result_digest: "d".repeat(64),
            },
            &m,
        );
        let ckpt = run.export_checkpoint().unwrap();
        let resumed = resume_run(
            &ckpt,
            &m,
            "pd",
            "rev-1",
            Some(ReplayAuthorization {
                reason: "replay".into(),
                authorized_by: "op".into(),
            }),
        )
        .expect("authorized replay");
        // Round-trip: export + re-import must pass digest verification.
        let rt = resumed.state.export_checkpoint().unwrap();
        let re = resume_run(
            &rt,
            &m,
            "pd",
            "rev-1",
            Some(ReplayAuthorization {
                reason: "replay".into(),
                authorized_by: "op".into(),
            }),
        );
        assert!(re.is_ok());
    }

    #[test]
    fn legacy_checkpoint_without_termination_or_changes_imports() {
        // A pre-#123 sealed checkpoint had NO termination / implementationChanges
        // members; both are additive (skip_serializing_if), so the digest stays
        // byte-identical and import still succeeds.
        let m = manifest();
        let run = GraphRunStateV1::open("r", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let ckpt = run.export_checkpoint().unwrap();
        assert!(!ckpt.contains("termination"));
        assert!(!ckpt.contains("implementationChanges"));
        let resumed = resume_run(&ckpt, &m, "pd", "rev-1", None).expect("legacy imports");
        assert!(resumed.state.termination.is_none());
        assert!(resumed.state.implementation_changes.is_empty());
    }
}
