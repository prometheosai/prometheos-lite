//! Durable state journal, migration, and recovery — integration tests.
//!
//! Exercises `recover_evaluation` against on-disk fixture trees representing
//! real interruption boundaries (a run killed mid-pipeline) and hostile
//! inputs (unsupported future schema). The journal is the source of truth;
//! recovery must reconstruct the exact position where the run stopped.

use std::path::{Path, PathBuf};

use prometheos_lite::workflow::evaluate::{EvaluationState, read_journal, recover_evaluation};

fn fixture_dir(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("tests")
        .join("fixtures")
        .join("workflow-state")
        .join(name)
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

    let recovered = recover_evaluation(repo, key).unwrap().unwrap();
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
    let err = recover_evaluation(repo, key).unwrap_err();
    assert!(err.to_string().contains("fail closed"));
}

#[test]
fn missing_durable_state_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        recover_evaluation(tmp.path(), "never-seen-key")
            .unwrap()
            .is_none()
    );
}
