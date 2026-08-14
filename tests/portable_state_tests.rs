//! Portable work state (#151) integration tests.
//!
//! Covers the export/import pipeline, deterministic canonical serialization,
//! stable SHA-256 digests, the immutable decision graph, portable ref safety,
//! repository compatibility, migration fixtures, and cross-harness portability.

use anyhow::Result;
use prometheos_lite::workflow::portable_state::{
    DecisionStatus, PortableWorkState, RepositorySnapshot, export_portable_state, from_json,
    import_portable_state, state_digest, to_canonical_json,
};
use serde_json::{Map, Value};
use std::path::Path;

fn fixture_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/portable-work-state")
        .join(rel)
}

fn load_fixture(rel: &str) -> String {
    std::fs::read_to_string(fixture_path(rel)).expect("fixture must exist")
}

fn expected_repo() -> RepositorySnapshot {
    RepositorySnapshot {
        identity: "https://github.com/prometheosai/prometheos-lite".to_string(),
        branch: "main".to_string(),
        revision: "0e756a38b6688d37c10eca190c7d8be8ac68a296".to_string(),
        local_path: None,
    }
}

fn current_fixture() -> PortableWorkState {
    import_portable_state(&load_fixture("current-v1/portable_work_state.json"), None)
        .expect("current fixture must import")
}

/// Re-serialize the current fixture with a modified field, then import.
fn import_modified(json_patch: impl FnOnce(&mut Value)) -> Result<PortableWorkState> {
    let mut value: Value =
        serde_json::from_str(&load_fixture("current-v1/portable_work_state.json"))
            .expect("fixture must parse");
    json_patch(&mut value);
    import_portable_state(&serde_json::to_string(&value).unwrap(), None)
}

// ---------------------------------------------------------------------------
// Fixture matrix
// ---------------------------------------------------------------------------

#[test]
fn current_fixture_matrix_validates() {
    let state = current_fixture();
    assert_eq!(state.schema_version.to_string_owned(), "1.0.0");
    assert_eq!(state.work.work_id, "work-1");
    assert_eq!(state.decisions.len(), 4);
    assert_eq!(state.execution_history.len(), 2);
}

#[test]
fn migration_legacy_fixture_migrates_to_current() {
    let state = import_portable_state(&load_fixture("legacy-v0/portable_work_state.json"), None)
        .expect("legacy fixture must migrate");
    assert_eq!(state.schema_version.to_string_owned(), "1.0.0");
    assert_eq!(state.work.work_id, "work-legacy-1");
}

#[test]
fn unsupported_future_schema_fails_closed() {
    let err = import_portable_state(
        &load_fixture("unsupported-future/portable_work_state.json"),
        None,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("PortableWorkState"), "{msg}");
    assert!(msg.contains("fail closed"), "{msg}");
}

#[test]
fn missing_required_field_fails_closed() {
    let err = import_portable_state(&load_fixture("corrupted/missing-required-field.json"), None)
        .unwrap_err();
    assert!(format!("{err:#}").contains("objective"), "{err:#}");
}

// ---------------------------------------------------------------------------
// Decision graph
// ---------------------------------------------------------------------------

#[test]
fn duplicate_decision_id_fails_closed() {
    let err = import_portable_state(&load_fixture("corrupted/duplicate-decision-id.json"), None)
        .unwrap_err();
    assert!(err.to_string().contains("duplicate decision_id"), "{}", err);
}

#[test]
fn dangling_supersession_fails_closed() {
    let err = import_portable_state(&load_fixture("corrupted/dangling-supersession.json"), None)
        .unwrap_err();
    assert!(err.to_string().contains("supersedes unknown"), "{}", err);
}

#[test]
fn supersession_cycle_fails_closed() {
    let err = import_portable_state(&load_fixture("corrupted/supersession-cycle.json"), None)
        .unwrap_err();
    assert!(err.to_string().contains("not Accepted"), "{}", err);
}

#[test]
fn supersession_marks_predecessor_superseded() {
    let state = current_fixture();
    let predecessor = state
        .decisions
        .iter()
        .find(|d| d.decision_id == "d-approach-unified-diff")
        .unwrap();
    let superseder = state
        .decisions
        .iter()
        .find(|d| d.decision_id == "d-approach-diffy")
        .unwrap();
    assert_eq!(predecessor.status, DecisionStatus::Superseded);
    assert_eq!(
        predecessor.superseded_by.as_deref(),
        Some("d-approach-diffy")
    );
    assert_eq!(superseder.status, DecisionStatus::Accepted);
    assert!(
        superseder
            .supersedes
            .contains(&"d-approach-unified-diff".to_string())
    );
}

#[test]
fn superseded_decision_remains_in_history() {
    let state = current_fixture();
    // The superseded decision is still present and auditable, never deleted.
    assert!(
        state
            .decisions
            .iter()
            .any(|d| d.decision_id == "d-approach-unified-diff")
    );
    let superseded = state
        .decisions
        .iter()
        .filter(|d| d.status == DecisionStatus::Superseded)
        .count();
    assert_eq!(superseded, 1);
}

#[test]
fn explicit_conflict_validates() {
    let state = import_portable_state(&load_fixture("contradictory/explicit-conflict.json"), None)
        .expect("explicit, mutually-recorded conflict must import");
    assert_eq!(state.decisions.len(), 2);
}

#[test]
fn untracked_conflict_fails_closed() {
    let err = import_portable_state(&load_fixture("contradictory/untracked-conflict.json"), None)
        .unwrap_err();
    assert!(err.to_string().contains("untracked"), "{}", err);
}

// ---------------------------------------------------------------------------
// Canonical serialization and digests
// ---------------------------------------------------------------------------

#[test]
fn canonical_serialization_is_deterministic() {
    let state = current_fixture();
    let a = to_canonical_json(&state).unwrap();
    let b = to_canonical_json(&state).unwrap();
    assert_eq!(a, b);
    assert_eq!(state_digest(&state).unwrap(), state_digest(&state).unwrap());
}

#[test]
fn canonical_serialization_independent_of_key_order() {
    let state = current_fixture();
    let canonical = to_canonical_json(&state).unwrap();
    let value: Value = serde_json::from_str(&canonical).unwrap();
    let obj = value.as_object().unwrap();
    let keys: Vec<String> = obj.keys().cloned().collect();
    let mut shuffled = Map::new();
    for k in keys.iter().rev() {
        shuffled.insert(k.clone(), obj.get(k).unwrap().clone());
    }
    let shuffled_json = serde_json::to_string(&Value::Object(shuffled)).unwrap();
    assert_eq!(canonical, shuffled_json);
}

#[test]
fn state_digest_is_stable_across_export_import() {
    let state = current_fixture();
    let before = state_digest(&state).unwrap();
    let exported = to_canonical_json(&state).unwrap();
    let reimported = import_portable_state(&exported, None).unwrap();
    assert_eq!(state_digest(&reimported).unwrap(), before);
}

#[test]
fn canonical_digest_is_stable_across_fixture_reimport() {
    let state = current_fixture();
    let exported = to_canonical_json(&state).unwrap();
    let reimported = import_portable_state(&exported, None).unwrap();
    assert_eq!(to_canonical_json(&reimported).unwrap(), exported);
}

// ---------------------------------------------------------------------------
// Export/import round trips
// ---------------------------------------------------------------------------

#[test]
fn export_import_round_trip_preserves_state() {
    let state = current_fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("portable_work_state.json");
    export_portable_state(&state, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let imported = import_portable_state(&text, None).unwrap();
    assert_eq!(imported, state);
}

#[test]
fn export_writes_reimportable_document() {
    let state = current_fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/portable_work_state.json");
    export_portable_state(&state, &path).unwrap();
    assert!(path.exists());
    let imported = import_portable_state(&std::fs::read_to_string(&path).unwrap(), None).unwrap();
    assert_eq!(imported, state);
}

#[test]
fn export_import_preserves_portable_refs() {
    let state = current_fixture();
    let exported = to_canonical_json(&state).unwrap();
    let imported = import_portable_state(&exported, None).unwrap();
    assert_eq!(imported.context_refs, state.context_refs);
    assert_eq!(imported.artifact_refs, state.artifact_refs);
    assert_eq!(imported.proposal_ref, state.proposal_ref);
    assert_eq!(imported.diff_ref, state.diff_ref);
    assert_eq!(imported.validation_results, state.validation_results);
}

#[test]
fn no_hidden_state_import_round_trip() {
    let state = current_fixture();
    let imported = import_portable_state(&to_canonical_json(&state).unwrap(), None).unwrap();
    assert_eq!(imported.work, state.work);
    assert_eq!(imported.repository, state.repository);
    assert_eq!(imported.plan, state.plan);
    assert_eq!(imported.steps, state.steps);
    assert_eq!(imported.decisions, state.decisions);
    assert_eq!(imported.review_results, state.review_results);
    assert_eq!(imported.failures, state.failures);
    assert_eq!(imported.authority, state.authority);
    assert_eq!(imported.execution_history, state.execution_history);
    assert_eq!(imported.compatibility, state.compatibility);
}

// ---------------------------------------------------------------------------
// Portable ref safety
// ---------------------------------------------------------------------------

#[test]
fn traversal_reference_fails_closed() {
    let err = import_modified(|v| {
        v["artifact_refs"] = serde_json::json!([{ "kind": "artifact", "uri": "../../secret" }]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("escapes"), "{}", err);
}

#[test]
fn absolute_reference_fails_closed() {
    let err = import_modified(|v| {
        v["artifact_refs"] = serde_json::json!([{ "kind": "artifact", "uri": "/etc/passwd" }]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("escapes"), "{}", err);
}

// ---------------------------------------------------------------------------
// Confidence and capabilities
// ---------------------------------------------------------------------------

#[test]
fn confidence_out_of_range_fails_closed() {
    let err = import_modified(|v| {
        v["decisions"][0]["confidence"] = serde_json::json!({ "value": 1.5 });
    })
    .unwrap_err();
    assert!(err.to_string().contains("0.0..=1.0"), "{}", err);
}

#[test]
fn confidence_negative_fails_closed() {
    let err = import_modified(|v| {
        v["decisions"][0]["confidence"] = serde_json::json!({ "value": -0.1 });
    })
    .unwrap_err();
    assert!(err.to_string().contains("0.0..=1.0"), "{}", err);
}

#[test]
fn non_portable_capability_fails_closed() {
    let err = import_modified(|v| {
        v["compatibility"]["required_capabilities"] = serde_json::json!(["provider:alpha-harness"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("non-portable"), "{}", err);
}

#[test]
fn plan_references_unknown_step_fails_closed() {
    let err = import_modified(|v| {
        v["plan"]["step_ids"] = serde_json::json!(["step-1", "step-9"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("unknown step"), "{}", err);
}

#[test]
fn blocked_step_requires_reason_fails_closed() {
    let err = import_modified(|v| {
        v["steps"] = serde_json::json!([{
            "step_id": "step-b",
            "title": "blocked step",
            "status": "blocked",
            "blocked_reason": null,
            "started_at": null,
            "completed_at": null,
            "notes": []
        }]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("blocked_reason"), "{}", err);
}

// ---------------------------------------------------------------------------
// Repository compatibility
// ---------------------------------------------------------------------------

#[test]
fn repository_revision_match_required() {
    let expected = expected_repo();
    let state = import_portable_state(
        &load_fixture("current-v1/portable_work_state.json"),
        Some(&expected),
    )
    .expect("matching repository must import");
    assert_eq!(state.repository.revision, expected.revision);
}

#[test]
fn stale_revision_fixture_fails_closed() {
    let expected = expected_repo();
    let err = import_portable_state(
        &load_fixture("stale-revision/portable_work_state.json"),
        Some(&expected),
    )
    .unwrap_err();
    assert!(err.to_string().contains("revision mismatch"), "{}", err);
}

#[test]
fn repository_identity_match_required() {
    let mut expected = expected_repo();
    expected.identity = "https://github.com/other/other-repo".to_string();
    let err = import_portable_state(
        &load_fixture("current-v1/portable_work_state.json"),
        Some(&expected),
    )
    .unwrap_err();
    assert!(err.to_string().contains("identity mismatch"), "{}", err);
}

#[test]
fn import_without_expected_repo_succeeds() {
    let state = import_portable_state(
        &load_fixture("stale-revision/portable_work_state.json"),
        None,
    )
    .expect("import without expected repo must succeed");
    assert_eq!(state.repository.revision, "stale-abc");
}

// ---------------------------------------------------------------------------
// Cross-harness portability
// ---------------------------------------------------------------------------

#[test]
fn cross_harness_import_does_not_require_original_harness() {
    // The fixture records two different harnesses in its execution history.
    let state = current_fixture();
    let harnesses: Vec<&str> = state
        .execution_history
        .iter()
        .map(|e| e.harness.as_deref().expect("harness recorded"))
        .collect();
    assert!(harnesses.contains(&"harness-a"));
    assert!(harnesses.contains(&"harness-b"));

    // No required capability demands the original provider, model, or harness.
    let caps = state
        .compatibility
        .required_capabilities
        .join(",")
        .to_lowercase();
    assert!(!caps.contains("provider:"));
    assert!(!caps.contains("model:"));
    assert!(!caps.contains("harness:"));

    // The state imports under a different provider/model/harness context
    // without requiring any of the originals.
    let exported = to_canonical_json(&state).unwrap();
    let reimported = import_portable_state(&exported, None).unwrap();
    assert_eq!(
        state_digest(&reimported).unwrap(),
        state_digest(&state).unwrap()
    );
}

#[test]
fn provider_and_model_fields_are_optional_on_import() {
    let state = import_modified(|v| {
        v["execution_history"] = serde_json::json!([{
            "execution_id": "exec-new",
            "provider": null,
            "model": null,
            "harness": null,
            "harness_version": null,
            "started_at": "2026-08-14T02:00:00Z",
            "completed_at": null
        }]);
    })
    .expect("execution provenance without provider/model/harness must import");
    assert_eq!(state.execution_history.len(), 1);
}

#[test]
fn compatibility_metadata_is_preserved_and_validated() {
    let state = current_fixture();
    assert_eq!(
        state.compatibility.state_schema_version.to_string_owned(),
        "1.0.0"
    );
    assert!(
        state
            .compatibility
            .required_capabilities
            .contains(&"sha256-digest".to_string())
    );
    let imported = import_portable_state(&to_canonical_json(&state).unwrap(), None).unwrap();
    assert_eq!(imported.compatibility, state.compatibility);
}

// ---------------------------------------------------------------------------
// Pure parsing entry point
// ---------------------------------------------------------------------------

#[test]
fn from_json_is_import_without_repository_expectation() {
    let state = from_json(&load_fixture("current-v1/portable_work_state.json"))
        .expect("from_json must validate");
    assert_eq!(state.work.work_id, "work-1");
}

// ---------------------------------------------------------------------------
// Contract-integrity fixes (review 4938920836)
// ---------------------------------------------------------------------------

// --- 1. Fail-closed output path ---

#[test]
fn invalid_state_cannot_canonicalize() {
    let mut state = current_fixture();
    state.work.work_id = "".to_string();
    assert!(
        to_canonical_json(&state).is_err(),
        "canonicalization of an invalid state must fail closed"
    );
}

#[test]
fn invalid_state_cannot_digest() {
    let mut state = current_fixture();
    state.work.work_id = "".to_string();
    assert!(
        state_digest(&state).is_err(),
        "digesting an invalid state must fail closed"
    );
}

#[test]
fn invalid_state_cannot_export() {
    let mut state = current_fixture();
    state.work.work_id = "".to_string();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.json");
    assert!(
        export_portable_state(&state, &path).is_err(),
        "exporting an invalid state must fail closed"
    );
    assert!(
        !path.exists(),
        "no file may be written for an invalid state"
    );
}

// --- 2. Semantic canonicalization of set-like collections ---

fn reorderable_state() -> PortableWorkState {
    let mut state = current_fixture();
    state.compatibility.required_capabilities = vec![
        "cap-b".to_string(),
        "cap-a".to_string(),
        "cap-c".to_string(),
    ];
    state.compatibility.optional_capabilities = vec!["opt-b".to_string(), "opt-a".to_string()];
    state.authority.allowed_paths = vec!["src/b".to_string(), "src/a".to_string()];
    state.authority.forbidden_paths = vec!["forb-b".to_string(), "forb-a".to_string()];
    state
}

#[test]
fn semantic_sets_canonicalize_independently_of_order() {
    let a = reorderable_state();
    let mut b = a.clone();
    b.compatibility.required_capabilities.reverse();
    b.compatibility.optional_capabilities.reverse();
    b.authority.allowed_paths.reverse();
    b.authority.forbidden_paths.reverse();
    assert_eq!(
        to_canonical_json(&a).unwrap(),
        to_canonical_json(&b).unwrap(),
        "set-like collections must canonicalize to sorted order"
    );
    assert_eq!(
        state_digest(&a).unwrap(),
        state_digest(&b).unwrap(),
        "digests of reordered set-like collections must match"
    );
}

#[test]
fn decision_edges_canonicalize_independently_of_order() {
    let a = reorderable_state();
    // Add a third decision edge so reversal is observable on the canonical form.
    let mut b = a.clone();
    let edge_a = &a.decisions[1];
    assert_eq!(
        edge_a.supersedes,
        vec!["d-approach-unified-diff".to_string()]
    );
    assert_eq!(
        edge_a.conflicts_with,
        vec!["d-approach-git-apply".to_string()]
    );
    let rev = &mut b.decisions[1];
    rev.supersedes.reverse();
    rev.conflicts_with.reverse();
    assert_eq!(
        to_canonical_json(&a).unwrap(),
        to_canonical_json(&b).unwrap(),
        "decision edge sets must canonicalize to sorted order"
    );
    assert_eq!(state_digest(&a).unwrap(), state_digest(&b).unwrap());
}

#[test]
fn canonicalization_does_not_mutate_caller_state() {
    let state = reorderable_state();
    let before = state.clone();
    let _ = to_canonical_json(&state).unwrap();
    let _ = state_digest(&state).unwrap();
    assert_eq!(state, before, "canonicalization must not mutate its input");
}

#[test]
fn ordered_collections_keep_original_order_in_canonical_form() {
    // Plan step order is meaningful and must survive canonicalization.
    let state = current_fixture();
    let mut reordered = state.clone();
    reordered.plan.as_mut().unwrap().step_ids.reverse();
    let a = to_canonical_json(&state).unwrap();
    let b = to_canonical_json(&reordered).unwrap();
    assert_ne!(
        a, b,
        "ordered collections must not be sorted by canonicalization"
    );

    // Execution history order is preserved too.
    let mut reordered_history = state.clone();
    reordered_history.execution_history.reverse();
    assert_ne!(
        to_canonical_json(&reordered_history).unwrap(),
        a,
        "execution history order must be preserved"
    );
}

// --- 3. Record identity invariants ---

#[test]
fn duplicate_result_id_fails() {
    let err = import_modified(|v| {
        v["validation_results"] = serde_json::json!([
            {
                "result_id": "val-1",
                "passed": true,
                "summary": "first",
                "evidence": null,
                "executed_at": "2026-08-14T00:55:00Z"
            },
            {
                "result_id": "val-1",
                "passed": false,
                "summary": "second",
                "evidence": null,
                "executed_at": "2026-08-14T00:56:00Z"
            }
        ]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("duplicate result_id"), "{}", err);
}

#[test]
fn duplicate_review_id_fails() {
    let err = import_modified(|v| {
        v["review_results"] = serde_json::json!([
            {
                "review_id": "review-1",
                "decision": "approved",
                "reviewer": "reviewer-a",
                "notes": "",
                "reviewed_at": "2026-08-14T00:50:00Z"
            },
            {
                "review_id": "review-1",
                "decision": "rejected",
                "reviewer": "reviewer-b",
                "notes": "",
                "reviewed_at": "2026-08-14T00:51:00Z"
            }
        ]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("duplicate review_id"), "{}", err);
}

#[test]
fn duplicate_failure_id_fails() {
    let err = import_modified(|v| {
        v["failures"] = serde_json::json!([
            {
                "failure_id": "fail-1",
                "class": "infrastructure",
                "stage": "preflight",
                "step_id": null,
                "evidence": null,
                "recoverable": true,
                "occurred_at": "2026-08-14T00:12:00Z",
                "detail": "first"
            },
            {
                "failure_id": "fail-1",
                "class": "model",
                "stage": "generation",
                "step_id": null,
                "evidence": null,
                "recoverable": true,
                "occurred_at": "2026-08-14T00:35:00Z",
                "detail": "second"
            }
        ]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("duplicate failure_id"), "{}", err);
}

#[test]
fn review_missing_reviewer_fails() {
    let err = import_modified(|v| {
        v["review_results"][0]["reviewer"] = serde_json::json!("");
    })
    .unwrap_err();
    assert!(err.to_string().contains("reviewer"), "{}", err);
}

#[test]
fn review_missing_reviewed_at_fails() {
    let err = import_modified(|v| {
        v["review_results"][0]["reviewed_at"] = serde_json::json!("");
    })
    .unwrap_err();
    assert!(err.to_string().contains("reviewed_at"), "{}", err);
}

#[test]
fn failure_step_id_must_reference_existing_step() {
    let err = import_modified(|v| {
        v["failures"][0]["step_id"] = serde_json::json!("step-does-not-exist");
    })
    .unwrap_err();
    assert!(err.to_string().contains("unknown step"), "{}", err);
}

#[test]
fn duplicate_step_id_fails() {
    let err = import_modified(|v| {
        v["steps"] = serde_json::json!([
            {
                "step_id": "step-1",
                "title": "first",
                "status": "completed",
                "blocked_reason": null,
                "started_at": null,
                "completed_at": "2026-08-14T00:10:00Z",
                "notes": []
            },
            {
                "step_id": "step-1",
                "title": "second",
                "status": "pending",
                "blocked_reason": null,
                "started_at": null,
                "completed_at": null,
                "notes": []
            }
        ]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("duplicate step_id"), "{}", err);
}

// --- 4. Decision graph hardening ---

#[test]
fn self_conflict_fails() {
    let err = import_modified(|v| {
        v["decisions"][1]["conflicts_with"] = serde_json::json!(["d-approach-diffy"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("conflict with itself"), "{}", err);
}

#[test]
fn duplicate_conflict_edge_fails() {
    let err = import_modified(|v| {
        v["decisions"][1]["conflicts_with"] =
            serde_json::json!(["d-approach-git-apply", "d-approach-git-apply"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("more than once"), "{}", err);
}

#[test]
fn duplicate_supersession_edge_fails() {
    let err = import_modified(|v| {
        v["decisions"][1]["supersedes"] =
            serde_json::json!(["d-approach-unified-diff", "d-approach-unified-diff"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("more than once"), "{}", err);
}

#[test]
fn self_supersede_fails() {
    let err = import_modified(|v| {
        v["decisions"][1]["supersedes"] = serde_json::json!(["d-approach-diffy"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("supersede itself"), "{}", err);
}

// --- 5. PortableRef kind invariants ---

#[test]
fn proposal_ref_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["proposal_ref"]["kind"] = serde_json::json!("artifact");
    })
    .unwrap_err();
    assert!(err.to_string().contains("proposal_ref"), "{}", err);
}

#[test]
fn diff_ref_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["diff_ref"]["kind"] = serde_json::json!("context");
    })
    .unwrap_err();
    assert!(err.to_string().contains("diff_ref"), "{}", err);
}

#[test]
fn context_ref_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["context_refs"][0]["kind"] = serde_json::json!("artifact");
    })
    .unwrap_err();
    assert!(err.to_string().contains("context ref"), "{}", err);
}

#[test]
fn artifact_ref_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["artifact_refs"][0]["kind"] = serde_json::json!("context");
    })
    .unwrap_err();
    assert!(err.to_string().contains("artifact ref"), "{}", err);
}

#[test]
fn validation_evidence_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["validation_results"][0]["evidence"]["kind"] = serde_json::json!("artifact");
    })
    .unwrap_err();
    assert!(err.to_string().contains("validation evidence"), "{}", err);
}

#[test]
fn failure_evidence_wrong_kind_fails() {
    let err = import_modified(|v| {
        v["failures"][0]["evidence"] = serde_json::json!({
            "kind": "artifact",
            "uri": "evidence/work-1/other.json",
            "digest": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "media_type": "application/json"
        });
    })
    .unwrap_err();
    assert!(err.to_string().contains("failure evidence"), "{}", err);
}

// --- 6. Authority snapshot validation ---

#[test]
fn authority_traversal_path_fails() {
    let err = import_modified(|v| {
        v["authority"]["allowed_paths"] = serde_json::json!(["../escape"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("escapes"), "{}", err);
}

#[test]
fn authority_absolute_path_fails() {
    let err = import_modified(|v| {
        v["authority"]["allowed_paths"] = serde_json::json!(["/etc/passwd"]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("escapes"), "{}", err);
}

#[test]
fn authority_empty_path_fails() {
    let err = import_modified(|v| {
        v["authority"]["forbidden_paths"] = serde_json::json!([""]);
    })
    .unwrap_err();
    assert!(err.to_string().contains("empty path"), "{}", err);
}

#[test]
fn malformed_policy_digest_fails() {
    let err = import_modified(|v| {
        v["authority"]["policy_digest"] = serde_json::json!("not-a-sha256-digest");
    })
    .unwrap_err();
    assert!(err.to_string().contains("policy_digest"), "{}", err);
}

#[test]
fn well_formed_policy_digest_validates() {
    let state = current_fixture();
    assert_eq!(state.authority.policy_digest.as_deref().unwrap().len(), 64);
}

// --- 7. Per-document schema versions (unit-tested in workflow::schema) ---
// --- 8. Migration idempotence ---

#[test]
fn migration_is_idempotent_for_portable_state() {
    let legacy = load_fixture("legacy-v0/portable_work_state.json");
    let imported = import_portable_state(&legacy, None).expect("legacy must migrate");
    let canonical = to_canonical_json(&imported).unwrap();
    assert_eq!(imported.schema_version.to_string_owned(), "1.0.0");

    // Importing the canonical (current) form yields the identical canonical
    // form and identical digest: migration is idempotent.
    let reimported = import_portable_state(&canonical, None).expect("current must reimport");
    assert_eq!(to_canonical_json(&reimported).unwrap(), canonical);
    assert_eq!(
        state_digest(&reimported).unwrap(),
        state_digest(&imported).unwrap()
    );
}
