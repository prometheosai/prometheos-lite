//! Issue #156 / #136 — export roundtrip conformance (Foundry ingestion
//! preconditions, not a Foundry contract).
//!
//! The smallest contract an external evaluator would consume from Lite is
//! the pinned failure-case bytes themselves: a consumer must be able to
//! (1) accept the bytes, (2) re-derive the sealed content digest, (3)
//! re-parse and re-verify the case with zero drift, and (4) reject
//! tampered or foreign bytes fail-closed. These tests pin exactly that
//! boundary on Lite's already-ratified `lite.evaluation.failure-case`
//! document — nothing here promotes a format, flips any `unsupported`
//! marker, or makes any Foundry or evaluation claim.
//!
//! NOTE: the ingestion contract itself is an owner/Foundry ratification
//! decision (Foundry #150 / SOMA #146), not a Lite-local decision. This
//! suite pins the byte-stability preconditions that any such contract
//! must rely on.
//!
//! Deterministic, synthetic fixture only; no provider, no network.

use prometheos_lite::workflow::failure_case::{
    FAILURE_CASE_NAMESPACE, FAILURE_CASE_SCANNER_ID, FAILURE_CASE_SCHEMA_VERSION,
    FAILURE_CASE_VERIFICATION_SCOPE, FailureCaseExpected, FailureCaseRedaction,
    FailureCaseSourcePin, LiteFailureCase, MINIMIZATION_ALGORITHM, MinimizedReproduction,
};
use prometheos_lite::workflow::failure_replay::{
    FailureAuthorityPin, FailureReplayClass, FailureValidatorPin,
};
use prometheos_lite::workflow::scenario::{
    ExternalEvaluationFormat, RunProvenance, ScenarioOutcomeClass, TaskOutcome, TaskRepairOrigin,
};
use sha2::Digest;

fn h(input: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(input.as_bytes()))
}

fn minimal_case() -> LiteFailureCase {
    let identity = h("roundtrip-fixture-identity");
    let mut case = LiteFailureCase {
        schema_version: FAILURE_CASE_SCHEMA_VERSION.to_string(),
        namespace: FAILURE_CASE_NAMESPACE.to_string(),
        version: "1.0.0".to_string(),
        case_id: format!("lite-failure-case-{}", &identity[..16]),
        title: "roundtrip conformance fixture".to_string(),
        evaluated_at: "2026-01-01T00:00:00Z".to_string(),
        failure_identity: identity,
        source: FailureCaseSourcePin {
            scenario_id: "rust-bugfix-arithmetic-001".to_string(),
            scenario_digest: h("scenario"),
            repository_revision: h("revision"),
            repository_fixture: "tests/fixtures/scenarios/rust-bugfix-arithmetic/repo"
                .to_string(),
            repository_baseline_digest: h("baseline"),
            task_statement_digest: h("task"),
            harness_id: "lite.fixture.scripted".to_string(),
            harness_revision: "v1".to_string(),
            provider_identity: "scripted-fixture".to_string(),
            model: "scripted-fixture".to_string(),
        },
        classification: ScenarioOutcomeClass::GovernedProductOutcome,
        task_outcome: TaskOutcome::RepairFailed,
        failure_class: FailureReplayClass::GovernedValidationFailure,
        validator: FailureValidatorPin {
            command: "cargo".to_string(),
            args: vec!["test".to_string(), "--offline".to_string()],
            exit_code: 101,
        },
        authority: FailureAuthorityPin {
            writable_scopes: vec!["src/lib.rs".to_string()],
            repair_origin: TaskRepairOrigin::ScriptedFixture,
            repair_write_authority:
                prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy,
            changed_files: vec!["src/lib.rs".to_string()],
        },
        controlled_dimensions: vec!["harness".to_string()],
        expected_result: FailureCaseExpected {
            task_outcome: TaskOutcome::Repaired,
            classification: ScenarioOutcomeClass::GovernedProductOutcome,
            validator_exit_code: 0,
            expected_assertion: "cargo test --offline exits 0".to_string(),
        },
        minimized_reproduction: MinimizedReproduction {
            algorithm: MINIMIZATION_ALGORITHM.to_string(),
            required_paths: vec!["src/lib.rs".to_string()],
            original_tree_digest: h("original-tree"),
            minimized_tree_digest: h("baseline"),
            removed_paths: Vec::new(),
            rejected_candidates: Vec::new(),
            steps: Vec::new(),
        },
        reproduction_evidence_digest: h("reproduction"),
        fix_evidence_digest: h("fix"),
        chain_evidence_digest: h("chain"),
        redaction: FailureCaseRedaction {
            scanner: FAILURE_CASE_SCANNER_ID.to_string(),
            known_secrets_registered: 0,
            canary_scanned: true,
            leaked_secrets: 0,
            machine_path_free: true,
        },
        provenance: RunProvenance {
            runner: "roundtrip-conformance".to_string(),
            runner_revision: "v1".to_string(),
            injected_clock: true,
            fixture_only: true,
            redaction: "none-needed".to_string(),
        },
        verification_scope: FAILURE_CASE_VERIFICATION_SCOPE.to_string(),
        external_evaluation_format: ExternalEvaluationFormat::unsupported(),
        content_digest: None,
    };
    case.content_digest = Some(case.compute_digest().expect("digest computes"));
    case.verify().expect("case verifies");
    case
}

#[test]
fn exported_bytes_reparse_and_reverify_with_zero_drift() {
    let case = minimal_case();
    let exported = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("export succeeds");
    let reparsed = LiteFailureCase::parse_json(&exported).expect("re-parse succeeds");
    assert_eq!(
        reparsed, case,
        "roundtrip must preserve every field exactly"
    );
    assert_eq!(
        reparsed.content_digest, case.content_digest,
        "reparsed contentDigest must be identical"
    );
}

#[test]
fn exports_are_byte_stable_across_serializations() {
    let case = minimal_case();
    let first = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("first export succeeds");
    let second = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("second export succeeds");
    assert_eq!(
        first, second,
        "identical input must produce byte-identical output"
    );
}

#[test]
fn tampered_bytes_fail_closed_at_reverify() {
    let case = minimal_case();
    let exported = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("export succeeds");
    // Flip one hex character inside the pinned validator args.
    let tampered = exported.replacen("\"--offline\"", "\"--onffline\"", 1);
    assert_ne!(tampered, exported);
    let parsed = LiteFailureCase::parse_json(&tampered);
    assert!(
        parsed.is_err(),
        "tampered bytes must fail closed at reverify"
    );
}

#[test]
fn foreign_digest_and_case_id_mismatch_are_refused() {
    let case = minimal_case();
    let exported = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("export succeeds");
    // A case with a case_id that does not derive from failure_identity is rejected.
    let mutated = exported.replacen("lite-failure-case-", "lite-failure-case-zz", 1);
    assert!(LiteFailureCase::parse_json(&mutated).is_err());
}

#[test]
fn external_evaluation_format_is_never_silently_present() {
    let case = minimal_case();
    let exported = prometheos_lite::workflow::failure_case::case_export_json(&case, &[])
        .expect("export succeeds");
    // The unsupported marker must be structurally present and typed.
    assert!(exported.contains("\"externalEvaluationFormat\""));
    assert!(exported.contains("\"status\": \"unsupported\""));
    let parsed = LiteFailureCase::parse_json(&exported).expect("parse");
    assert_eq!(parsed.external_evaluation_format.status, "unsupported");
}
