//! Durable state journal, migration, and recovery — integration tests.
//!
//! Exercises `recover_evaluation` and `read_journal` against the committed
//! on-disk fixture trees under `tests/fixtures/workflow-state/`:
//!
//! - `interrupted-boundaries`: a run killed mid-pipeline.
//! - `unsupported-future`: hostile future-schema documents.
//! - `current-v1`: fully versioned current-schema tree, including a complete
//!   `EvidenceBundle`.
//! - `legacy-v0`: unversioned (`0.0.0`) tree that must migrate to current.
//! - `corrupted`: deliberately corrupt registry / identity / evidence fixtures.
//!
//! The journal is the source of truth; recovery must reconstruct the exact
//! position where the run stopped and fail closed on corrupt data.

use std::path::{Path, PathBuf};

use prometheos_lite::workflow::evaluate::{
    EvaluationState, EvidenceBundle, ExecutionIdentity, read_journal, recover_evaluation,
};

fn fixture_dir(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("tests")
        .join("fixtures")
        .join("workflow-state")
        .join(name)
}

/// Absolute path to one committed fixture document.
fn fixture_file(name: &str, rel: &str) -> PathBuf {
    fixture_dir(name).join(rel)
}

/// Copy `<fixture>/prometheos` into `<temp_repo>/.prometheos` and return the
/// temporary repo root.
fn stage_fixture(name: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let src = fixture_dir(name).join("prometheos");
    let dst = tmp.path().join(".prometheos");
    copy_tree(&src, &dst);
    tmp
}

fn copy_tree(src: &Path, dst: &Path) {
    if src.is_dir() {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &dst.join(entry.file_name()));
        }
    } else {
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(src, dst).unwrap();
    }
}

#[test]
fn interrupted_run_recovers_to_proposal_generated() {
    let tmp = stage_fixture("interrupted-boundaries");
    let repo = tmp.path();
    let key = "checkpoint-key-abc";

    let events = read_journal(repo, key).unwrap();
    assert_eq!(events.len(), 3);
    // Loaded events carry the new fencing/provenance fields.
    assert_eq!(events[2].owner_run_id, "fixture-run");
    assert_eq!(events[2].lease_epoch, 1);
    assert_eq!(events[2].repository_revision, "fixture-head");

    let recovered = recover_evaluation(repo, key, None, None).unwrap().unwrap();
    assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
    assert_eq!(recovered.last_journal_sequence, 2);
    assert_eq!(
        recovered.proposal_ref.as_deref(),
        Some("fixture-proposal-1")
    );
}

#[test]
fn unsupported_future_schema_fails_closed() {
    let tmp = stage_fixture("unsupported-future");
    let repo = tmp.path();
    let key = "future-key";

    // The journal event itself must be rejected before replay.
    let err = read_journal(repo, key).unwrap_err();
    assert!(err.to_string().contains("fail closed"));

    // Recovery must also fail closed, not silently proceed.
    let err = recover_evaluation(repo, key, None, None).unwrap_err();
    assert!(err.to_string().contains("fail closed"));
}

#[test]
fn missing_durable_state_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        recover_evaluation(tmp.path(), "never-seen-key", None, None)
            .unwrap()
            .is_none()
    );
}

/// The committed current-v1 tree must recover cleanly, and its static
/// `EvidenceBundle` must parse as the current schema type (`typed_validate`
/// performs exactly this parse).
#[test]
fn current_fixture_matrix_validates() {
    let tmp = stage_fixture("current-v1");
    let repo = tmp.path();

    let recovered = recover_evaluation(repo, "current-key", None, None)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
    assert_eq!(recovered.last_journal_sequence, 2);
    assert_eq!(
        recovered.proposal_ref.as_deref(),
        Some("current-proposal-1")
    );

    let text = std::fs::read_to_string(fixture_file(
        "current-v1",
        "prometheos/evidence/current-evidence/evidence.json",
    ))
    .unwrap();
    let bundle: EvidenceBundle = serde_json::from_str(&text).unwrap();
    assert_eq!(bundle.schema_version, "1.0.0");
    assert_eq!(bundle.run_id, "current-run");
    assert_eq!(
        bundle.proposal.as_ref().map(|p| p.id.as_str()),
        Some("current-proposal-1")
    );
}

/// The committed legacy-v0 tree is unversioned (`0.0.0`). Recovery migrates
/// the mutable documents (journal, checkpoint, registry, identity) in place
/// and must succeed end-to-end.
#[test]
fn legacy_fixture_matrix_migrates() {
    let tmp = stage_fixture("legacy-v0");
    let repo = tmp.path();

    let recovered = recover_evaluation(repo, "legacy-key", None, None)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.state, EvaluationState::ProposalGenerated);
    assert_eq!(recovered.last_journal_sequence, 2);
    assert_eq!(recovered.proposal_ref.as_deref(), Some("legacy-proposal-1"));

    // The static legacy evidence bundle declares no schema version, so it is
    // detected as legacy. Simulate the in-memory migration `migrate_document`
    // performs for immutable evidence (inject the current version, never
    // rewrite the file) and require the result to validate as current.
    let text = std::fs::read_to_string(fixture_file(
        "legacy-v0",
        "prometheos/evidence/legacy-evidence/evidence.json",
    ))
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        value.get("schema_version").is_none(),
        "legacy evidence must be unversioned"
    );
    value["schema_version"] = serde_json::Value::String("1.0.0".to_string());
    let bundle: EvidenceBundle = serde_json::from_value(value).unwrap();
    assert_eq!(bundle.run_id, "legacy-run");
    assert_eq!(
        bundle.proposal.as_ref().map(|p| p.id.as_str()),
        Some("legacy-proposal-1")
    );
}

/// Evidence is immutable: recovery must never rewrite the on-disk legacy
/// evidence bundle (mutable documents migrate in place; evidence is validated
/// in memory only).
#[test]
fn legacy_evidence_is_not_rewritten() {
    let tmp = stage_fixture("legacy-v0");
    let repo = tmp.path();
    let staged_evidence = tmp
        .path()
        .join(".prometheos")
        .join("evidence")
        .join("legacy-evidence")
        .join("evidence.json");
    let before = std::fs::read(&staged_evidence).unwrap();

    recover_evaluation(repo, "legacy-key", None, None)
        .unwrap()
        .unwrap();

    let after = std::fs::read(&staged_evidence).unwrap();
    assert_eq!(before, after, "legacy evidence must not be rewritten");
}

/// The committed corrupt proposal registry is structurally incomplete (the
/// required `entries` map is missing). Recovery migrates the registry up front
/// and must fail closed.
#[test]
fn corrupted_registry_fixture_fails_closed() {
    let tmp = stage_fixture("corrupted");
    let err = recover_evaluation(tmp.path(), "corrupt-key", None, None).unwrap_err();
    assert!(
        err.to_string().contains("ProposalRegistry"),
        "expected registry failure, got: {err}"
    );
}

/// The committed corrupt identity has a malformed required field (`state` is
/// not a valid `EvaluationState`); it must not parse as the current type.
#[test]
fn corrupted_identity_fixture_fails_closed() {
    let text = std::fs::read_to_string(fixture_file(
        "corrupted",
        "prometheos/evidence/corrupt-evidence/execution_identity.json",
    ))
    .unwrap();
    let err = serde_json::from_str::<ExecutionIdentity>(&text).unwrap_err();
    assert!(err.is_data());
}

/// The committed corrupt evidence bundle is structurally incomplete (required
/// fields missing); it must not parse as the current type.
#[test]
fn corrupted_evidence_fixture_fails_closed() {
    let text = std::fs::read_to_string(fixture_file(
        "corrupted",
        "prometheos/evidence/corrupt-evidence/evidence.json",
    ))
    .unwrap();
    let err = serde_json::from_str::<EvidenceBundle>(&text).unwrap_err();
    assert!(err.is_data());
}
