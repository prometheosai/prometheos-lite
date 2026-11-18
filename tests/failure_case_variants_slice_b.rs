//! Issue #156 Slice B — variant replay matrix over a sealed failure case.
//!
//! Deterministic conformance for the replay-across-variants pipeline stage:
//! one sealed failure case, several declared variants (model / harness /
//! context / workflow axes), classifications derived strictly from sealed
//! replay observations. Every observation here is fixture-scripted; nothing
//! demonstrates a live provider, and no Foundry ingestion, ranking,
//! certification or score claim is produced.

use prometheos_lite::workflow::failure_case::{
    FAILURE_CASE_NAMESPACE, FAILURE_CASE_SCHEMA_VERSION, FailureCaseExpected, FailureCaseRedaction,
    FailureCaseSourcePin, FailureSignature, LiteFailureCase, MINIMIZATION_ALGORITHM,
    MinimizedReproduction, ReplaySignature,
};
use prometheos_lite::workflow::failure_case_variants::{
    self, FAILURE_CASE_VARIANT_MATRIX_NAMESPACE, FAILURE_CASE_VARIANT_MATRIX_SCHEMA_VERSION,
    FAILURE_CASE_VARIANT_MATRIX_VERIFICATION_SCOPE, FAILURE_CASE_VARIANT_MATRIX_VERSION,
    FailureCaseVariantAxis, FailureCaseVariantReplayInput, VariantExpectation,
    VariantMatrixVerdict, VariantVerdict,
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
    LiteFailureCase {
        schema_version: FAILURE_CASE_SCHEMA_VERSION.to_string(),
        namespace: FAILURE_CASE_NAMESPACE.to_string(),
        version: "1.0.0".to_string(),
        case_id: format!("case-{}", &h("failed-case")[..32]),
        title: "fixture classified failure".to_string(),
        evaluated_at: "2026-01-01T00:00:00Z".to_string(),
        failure_identity: h("fixture-failure-identity"),
        source: FailureCaseSourcePin {
            scenario_id: "rust-bugfix-arithmetic-001".to_string(),
            scenario_digest: h("scenario"),
            repository_revision: h("revision"),
            repository_fixture: "tests/fixtures/scenarios/rust-bugfix-arithmetic/repo".to_string(),
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
            repair_write_authority: prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy,
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
            minimized_tree_digest: h("minimized-tree"),
            removed_paths: Vec::new(),
            rejected_candidates: Vec::new(),
            steps: Vec::new(),
        },
        reproduction_evidence_digest: h("reproduction"),
        fix_evidence_digest: h("fix"),
        chain_evidence_digest: h("chain"),
        redaction: FailureCaseRedaction {
            scanner: "test-scanner".to_string(),
            known_secrets_registered: 1,
            canary_scanned: true,
            leaked_secrets: 0,
            machine_path_free: true,
        },
        provenance: RunProvenance {
            runner: "test".to_string(),
            runner_revision: "v1".to_string(),
            injected_clock: true,
            fixture_only: true,
            redaction: "none-needed".to_string(),
        },
        verification_scope: FAILURE_CASE_VARIANT_MATRIX_VERIFICATION_SCOPE.to_string(),
        external_evaluation_format: ExternalEvaluationFormat::unsupported(),
        content_digest: None,
    }
}

fn pinned_failure() -> FailureSignature {
    FailureSignature {
        validator_command: "cargo".to_string(),
        validator_args: vec!["test".to_string(), "--offline".to_string()],
        validator_exit_code: 101,
        task_outcome: TaskOutcome::RepairFailed,
        classification: ScenarioOutcomeClass::GovernedProductOutcome,
        failure_class: FailureReplayClass::GovernedValidationFailure,
    }
}

fn observed(sealed: bool, outcome: TaskOutcome, exit_code: i32) -> ReplaySignature {
    ReplaySignature {
        sealed,
        task_outcome: Some(outcome),
        classification: Some(ScenarioOutcomeClass::GovernedProductOutcome),
        failure_class: Some(FailureReplayClass::GovernedValidationFailure),
        validator_command: Some("cargo".to_string()),
        validator_args: vec!["test".to_string(), "--offline".to_string()],
        validator_exit_code: Some(exit_code),
    }
}

fn variant(
    id: &str,
    axis: FailureCaseVariantAxis,
    expectation: VariantExpectation,
    observed: ReplaySignature,
) -> FailureCaseVariantReplayInput {
    FailureCaseVariantReplayInput {
        variant_id: id.to_string(),
        axis,
        observed,
        expectation,
        simulation_fixture: true,
    }
}

fn build(
    inputs: &[FailureCaseVariantReplayInput],
) -> anyhow::Result<prometheos_lite::workflow::failure_case_variants::FailureCaseVariantMatrix> {
    let case = minimal_case();
    let failure = pinned_failure();
    failure_case_variants::build_variant_matrix(&case, &failure, inputs)
}

#[test]
fn reproduce_failure_variant_matrix_conformance_and_digest_round_trip() {
    let inputs = vec![
        variant(
            "variant-model-alt-a",
            FailureCaseVariantAxis::Model,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "variant-harness-alt-a",
            FailureCaseVariantAxis::Harness,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
    ];
    let matrix = build(&inputs).expect("matrix builds");
    assert_eq!(
        matrix.schema_version,
        FAILURE_CASE_VARIANT_MATRIX_SCHEMA_VERSION
    );
    assert_eq!(matrix.namespace, FAILURE_CASE_VARIANT_MATRIX_NAMESPACE);
    assert_eq!(matrix.version, FAILURE_CASE_VARIANT_MATRIX_VERSION);
    assert_eq!(
        matrix.verification_scope,
        FAILURE_CASE_VARIANT_MATRIX_VERIFICATION_SCOPE
    );
    assert!(matrix.simulation_only);
    assert_eq!(matrix.case_id, minimal_case().case_id);
    assert_eq!(matrix.failure_identity, minimal_case().failure_identity);
    assert_eq!(matrix.verdict, VariantMatrixVerdict::AllConforming);
    assert_eq!(matrix.replays.len(), 2);
    assert_eq!(matrix.replays[0].verdict, VariantVerdict::Reproduced);
    assert_eq!(matrix.replays[0].axis, "model");
    assert_eq!(matrix.replays[1].axis, "harness");
    assert!(
        matrix
            .replays
            .iter()
            .all(|replay| replay.simulation_fixture)
    );
    // Expected pins bind the case, not a relabelled failure.
    assert_eq!(matrix.expected.validator_exit_code, 101);
    assert_eq!(matrix.expected.expectation, "reproduce-failure");
    assert_eq!(matrix.expected.task_outcome, "repair-failed");
    // Digest is sealed and re-derives.
    matrix.verify().expect("sealed matrix verifies");
    let round_trip = serde_json::to_string(&matrix).expect("matrix serializes");
    let parsed: prometheos_lite::workflow::failure_case_variants::FailureCaseVariantMatrix =
        serde_json::from_str(&round_trip).expect("matrix deserializes");
    parsed.verify().expect("round-trip matrix verifies");
    assert_eq!(parsed, matrix);
}

#[test]
fn observe_fix_variant_matrix_classification() {
    let inputs = vec![
        variant(
            "variant-context-tight",
            FailureCaseVariantAxis::Context,
            VariantExpectation::ObserveFix,
            observed(true, TaskOutcome::Repaired, 0),
        ),
        variant(
            "variant-workflow-candidate",
            FailureCaseVariantAxis::Workflow,
            VariantExpectation::ObserveFix,
            observed(true, TaskOutcome::Repaired, 0),
        ),
    ];
    let matrix = build(&inputs).expect("matrix builds");
    assert_eq!(matrix.verdict, VariantMatrixVerdict::AllConforming);
    assert_eq!(
        matrix.replays[0].verdict,
        VariantVerdict::RepairedFixObserved
    );
    assert_eq!(matrix.replays[0].expected.expectation, "observe-fix");
    assert_eq!(matrix.replays[0].expected.task_outcome, "repaired");
    assert_eq!(matrix.replays[0].expected.validator_exit_code, 0);
    assert_eq!(matrix.replays[0].axis, "context");
    assert_eq!(matrix.replays[1].axis, "workflow");
    matrix.verify().expect("verifies");
}

#[test]
fn drifted_variant_is_not_confused_with_conforming() {
    let inputs = vec![
        variant(
            "variant-model-pass",
            FailureCaseVariantAxis::Model,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "variant-model-drift",
            FailureCaseVariantAxis::Model,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 42), // wrong exit code
        ),
    ];
    let matrix = build(&inputs).expect("matrix builds");
    assert_eq!(matrix.verdict, VariantMatrixVerdict::Mixed);
    let drifted = matrix
        .replays
        .iter()
        .find(|r| r.variant_id == "variant-model-drift")
        .expect("present");
    assert_eq!(drifted.verdict, VariantVerdict::Drifted);
    matrix.verify().expect("verifies");
}

#[test]
fn unsealed_variant_observation_is_unverifiable() {
    let inputs = vec![variant(
        "variant-harness-unsealed",
        FailureCaseVariantAxis::Harness,
        VariantExpectation::ReproduceFailure,
        observed(false, TaskOutcome::RepairFailed, 101),
    )];
    let matrix = build(&inputs).expect("matrix builds");
    assert_eq!(matrix.verdict, VariantMatrixVerdict::AllUnverifiable);
    assert_eq!(matrix.replays[0].verdict, VariantVerdict::Unverifiable);
}

#[test]
fn duplicate_variant_ids_are_refused() {
    let inputs = vec![
        variant(
            "variant-model-alt-a",
            FailureCaseVariantAxis::Model,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "variant-model-alt-a",
            FailureCaseVariantAxis::Harness,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
    ];
    let err = build(&inputs).expect_err("duplicate ids refused");
    assert!(
        format!("{err:#}").contains("duplicate variant id"),
        "{err:#}"
    );
}

#[test]
fn non_fixture_variant_is_refused() {
    let mut input = variant(
        "variant-model-live",
        FailureCaseVariantAxis::Model,
        VariantExpectation::ReproduceFailure,
        observed(true, TaskOutcome::RepairFailed, 101),
    );
    input.simulation_fixture = false;
    let err = build(&[input]).expect_err("live-provider fixture is refused");
    assert!(format!("{err:#}").contains("fixture-synthetic"), "{err:#}");
}

#[test]
fn empty_variant_set_is_refused() {
    let err = build(&[]).expect_err("empty matrix refused");
    assert!(format!("{err:#}").contains("at least one"), "{err:#}");
}

#[test]
fn observe_fix_with_failure_observation_is_drifted() {
    let inputs = vec![variant(
        "variant-context-fix",
        FailureCaseVariantAxis::Context,
        VariantExpectation::ObserveFix,
        observed(true, TaskOutcome::RepairFailed, 101),
    )];
    let matrix = build(&inputs).expect("matrix builds");
    assert_eq!(matrix.replays[0].verdict, VariantVerdict::Drifted);
    assert_eq!(matrix.verdict, VariantMatrixVerdict::NoneConforming);
}

#[test]
fn matrix_verify_refuses_tampered_replays() {
    let inputs = vec![variant(
        "variant-model-alt-a",
        FailureCaseVariantAxis::Model,
        VariantExpectation::ReproduceFailure,
        observed(true, TaskOutcome::RepairFailed, 101),
    )];
    let mut matrix = build(&inputs).expect("matrix builds");
    matrix.replays[0].variant_id = "variant-model-alt-b".to_string();
    let err = matrix.verify().expect_err("tampered matrix refused");
    assert!(format!("{err:#}").contains("matrixDigest"), "{err:#}");
}

#[test]
fn axis_serializations_are_stable_kebab_case() {
    let inputs = vec![
        variant(
            "v-model",
            FailureCaseVariantAxis::Model,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "v-harness",
            FailureCaseVariantAxis::Harness,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "v-context",
            FailureCaseVariantAxis::Context,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
        variant(
            "v-workflow",
            FailureCaseVariantAxis::Workflow,
            VariantExpectation::ReproduceFailure,
            observed(true, TaskOutcome::RepairFailed, 101),
        ),
    ];
    let matrix = build(&inputs).expect("matrix builds");
    let json = serde_json::to_string(&matrix).expect("serializes");
    for needle in [
        "\"axis\":\"model\"",
        "\"axis\":\"harness\"",
        "\"axis\":\"context\"",
        "\"axis\":\"workflow\"",
    ] {
        assert!(json.contains(needle), "missing {needle}: {json}");
    }
}
