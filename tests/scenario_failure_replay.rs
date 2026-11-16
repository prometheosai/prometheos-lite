//! Issue #136 Slice C — replayable failure evidence: pinned failure →
//! replay → verified repair.
//!
//! Slice B sealed a task-run envelope for a REPAIRED task and for a FAILED
//! task, but nothing bound the two together. This suite binds them into one
//! Lite-internal evidence chain:
//!
//! 1. the committed execute fixture (Slice B, untouched) is run with the
//!    fixture-scripted **wrong** repair; the pinned validator
//!    (`cargo test --offline`) really executes and exits 101, and the run
//!    seals a task-run envelope with `taskOutcome: repair-failed`,
//!    `classification: governed-product-outcome` — the **original failure**;
//! 2. the same fixture is run with the fixture-scripted **correct** repair;
//!    the validator exits 0 and the run seals a `repaired` envelope — the
//!    **replay**;
//! 3. a versioned, Lite-internal failure-replay descriptor (committed
//!    golden: `tests/fixtures/scenarios/rust-bugfix-arithmetic-replay/descriptor.json`)
//!    pins the scenario digest, the baseline pin, the known failed validator
//!    (command/argv/non-zero exit), the failed file-after digest, the failed
//!    post-run tree digest, the replay expectation and the exact authority
//!    both runs must hold;
//! 4. `LiteFailureReplayEvidence` embeds BOTH sealed envelopes, derives a
//!    stable, wall-clock-free `failureIdentity` of the failure, and re-derives
//!    every binding at verification time. The replay may not be an unrelated
//!    success, may not swap validators, may not widen authority, may not
//!    change classification, and may not re-present the failure as a repair.
//!
//! Both runs dispatch through the same real governed boundary as Slice B
//! (`tests/common/task_execution.rs`). The repair is a **scripted
//! fixture**, not model output; nothing here demonstrates model coding
//! ability, and no benchmark score, ranking, certification or Foundry claim
//! is produced. The failure-replay descriptor is a bridge for issue #156,
//! not a completed external evaluation.

use std::path::PathBuf;

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::workflow::failure_replay::{
    self, FailureReplayDescriptor, LiteFailureReplayEvidence,
};
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::route_dispatch::RoutedDispatchFailure;
use prometheos_lite::workflow::scenario::{
    LiteTaskRunEvidence, ScenarioDescriptor, ScenarioOutcomeClass, TaskOutcome, TaskRepairOrigin,
};

mod common;

use common::task_execution::{DispatchInjection, TaskExecutionRun, TaskExecutionSpec, repo_root};

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");
const REPLAY_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-replay/descriptor.json");

/// Stable repository identity for the disposable source repository (same as
/// Slice B). Never dereferenced, never fetched.
const BUGFIX_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";

/// Per-fixture facts for the bugfix execute scenario (same as Slice B).
const EXECUTE_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/lib.rs",
    repo_identity_url: BUGFIX_REPO_IDENTITY_URL,
};

/// Invoke the shared governed task runner for the failure/replay fixture.
fn run_task_execution(descriptor_text: &str, edits: Vec<EditOperation>) -> TaskExecutionRun {
    common::task_execution::run_task_execution(&EXECUTE_SPEC, descriptor_text, edits)
}

/// Same, with a deterministic governed-dispatch fault injected.
fn run_task_execution_with_fault(
    descriptor_text: &str,
    edits: Vec<EditOperation>,
    injection: DispatchInjection,
) -> TaskExecutionRun {
    common::task_execution::run_task_execution_with_fault(
        &EXECUTE_SPEC,
        descriptor_text,
        edits,
        injection,
    )
}

// ---------------------------------------------------------------------------
// Repair actions (fixture-scripted; NEVER model output)
// ---------------------------------------------------------------------------

fn correct_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/lib.rs"),
        search: "a - b".to_string(),
        replace: "a + b".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

fn wrong_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/lib.rs"),
        search: "a - b".to_string(),
        replace: "a - b + 1".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

/// Seal a failure-replay chain from two sealed task-run envelopes.
fn seal_chain(
    original_failure: LiteTaskRunEvidence,
    replay: LiteTaskRunEvidence,
) -> anyhow::Result<LiteFailureReplayEvidence> {
    let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR)
        .expect("the committed replay descriptor must validate");
    LiteFailureReplayEvidence::sealed(descriptor, original_failure, replay)
}

/// Run the pinned failure and the verified replay; seal the chain.
fn run_and_seal_chain() -> LiteFailureReplayEvidence {
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("the pinned failure seals a task-run envelope");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("the replay seals a task-run envelope");
    seal_chain(failure, replay).expect("failure-replay evidence seals")
}

// ---------------------------------------------------------------------------
// Replay descriptor pins
// ---------------------------------------------------------------------------

#[test]
fn replay_descriptor_loads_pins_and_validates() {
    let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR)
        .expect("replay descriptor must validate");
    assert_eq!(descriptor.namespace, "lite.fixtures.failure-replay");
    assert_eq!(descriptor.schema_version, "1.0.0");
    assert_eq!(
        descriptor.replay_id,
        "lite-rust-bugfix-arithmetic-replay-001"
    );
    assert_eq!(descriptor.validate(), Vec::new());
    assert_eq!(
        descriptor.original.scenario_id,
        "lite-rust-bugfix-arithmetic-execute-001"
    );
    assert_eq!(
        descriptor.original.failure_class,
        failure_replay::FailureReplayClass::GovernedValidationFailure
    );
    assert_eq!(
        descriptor.original.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(descriptor.original.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(descriptor.original.validator.exit_code, 101);
    assert_eq!(descriptor.original.validator.command, "cargo");
    assert_eq!(
        descriptor.original.validator.args,
        vec!["test", "--offline"]
    );
    assert_eq!(
        descriptor.original.baseline_content_digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6"
    );
    assert_eq!(
        descriptor.original.failed_file_after_digest,
        "ee0fc1a2f85a324a999e792b0c5d1bd490dc80c6d554500a440af9ce564a2379"
    );
    assert_eq!(
        descriptor.original.failed_post_run_content_digest,
        "099c84f3afec65c339c07ad6e0057990f6400e35f18b3e18f495dc82bd4ecf01"
    );
    assert_eq!(descriptor.replay.task_outcome, TaskOutcome::Repaired);
    assert_eq!(
        descriptor.replay.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(descriptor.replay.expected_exit_code, 0);
    assert_eq!(descriptor.authority.writable_scopes, vec!["repo://fixture"]);
    assert_eq!(
        descriptor.authority.repair_origin,
        TaskRepairOrigin::ScriptedFixture
    );
    assert_eq!(
        descriptor.authority.repair_write_authority,
        prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy
    );
    assert_eq!(descriptor.authority.changed_files, vec!["src/lib.rs"]);
}

#[test]
fn replay_descriptor_scenario_digest_pin_matches_the_execute_descriptor() {
    let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR).expect("validates");
    let execute = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    assert_eq!(
        descriptor.original.scenario_digest,
        execute.scenario_digest().expect("scenario digest"),
        "the golden replay descriptor pins the execute descriptor's canonical scenario digest"
    );
}

// ---------------------------------------------------------------------------
// The original failure, executed for real
// ---------------------------------------------------------------------------

#[test]
fn failing_validation_produces_a_verified_but_failed_envelope() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits());
    assert_eq!(run.acquire_error, None);
    assert_eq!(run.patch_error, None, "wrong patch still applies");
    assert!(run.patched);
    assert_eq!(
        run.capability_effects, 1,
        "the governed dispatch ran exactly once"
    );
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome,
        "a validator that ran and failed is a product outcome, not infrastructure"
    );
    let envelope = run.envelope.as_ref().expect("failure envelope seals");
    envelope.verify().expect("failure envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(envelope.validator.exit_code, Some(101));
    assert_ne!(envelope.validator.exit_code, Some(0));
    assert_eq!(
        envelope.baseline_content_digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6"
    );
    assert_eq!(
        envelope.post_run_content_digest,
        "099c84f3afec65c339c07ad6e0057990f6400e35f18b3e18f495dc82bd4ecf01",
        "the failed run's post-run digest equals the pinned failed value"
    );
    assert_eq!(
        envelope.repair.file_after_digest,
        "ee0fc1a2f85a324a999e792b0c5d1bd490dc80c6d554500a440af9ce564a2379"
    );
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);

    // The failing dispatch completed and is accounted truthfully: the
    // attempt sealed, usage is known (loopback fixture), and cost is known
    // for the loopback provider.
    let accounting = run.accounting.as_ref().expect("accounting recorded");
    accounting.verify().expect("accounting verifies");
    assert_eq!(
        accounting.outcome,
        prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Completed,
        "the dispatch completed; only the task (validator) failed"
    );
    assert!(accounting.attempt_digest.is_some());
    assert_eq!(envelope.scenario_run.graph_accounting.attempts, 1);
    assert_eq!(envelope.scenario_run.graph_accounting.completed, 1);
}

// ---------------------------------------------------------------------------
// The replay → verified repair, and the sealed failure→replay chain
// ---------------------------------------------------------------------------

#[test]
fn failure_replay_chain_seals_and_verifies_end_to_end() {
    let chain = run_and_seal_chain();
    chain.verify().expect("failure-replay evidence verifies");
    assert_eq!(chain.namespace, "lite.evidence.failure-replay");
    assert_eq!(chain.schema_version, "1.0.0");
    assert_eq!(
        chain.evaluated_at,
        chain.original_failure.scenario_run.evaluated_at
    );
    assert_eq!(
        chain
            .original_failure
            .scenario_run
            .descriptor
            .scenario_digest()
            .unwrap(),
        chain
            .replay
            .scenario_run
            .descriptor
            .scenario_digest()
            .unwrap(),
        "failure and replay ran the same pinned scenario"
    );
    assert_eq!(
        chain.original_failure.task_outcome,
        TaskOutcome::RepairFailed
    );
    assert_eq!(chain.replay.task_outcome, TaskOutcome::Repaired);
    assert_eq!(chain.original_failure.validator.exit_code, Some(101));
    assert_eq!(chain.replay.validator.exit_code, Some(0));
    assert_eq!(
        chain.original_failure.baseline_content_digest, chain.replay.baseline_content_digest,
        "failure and replay ran on the same pinned fixture bytes"
    );
    assert_eq!(
        chain.original_failure.repair.changed_files,
        chain.replay.repair.changed_files
    );
    assert_eq!(
        chain.failure_identity,
        failure_replay::derive_failure_identity(&chain.original_failure).expect("identity derives"),
        "the sealed failureIdentity re-derives from the embedded failure"
    );
    assert_eq!(chain.external_evaluation_format.status, "unsupported");
    assert_eq!(
        chain.verification_scope,
        failure_replay::FAILURE_REPLAY_VERIFICATION_SCOPE
    );
}

#[test]
fn failure_identity_is_stable_across_independent_failure_runs() {
    let first = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("first failure seals");
    let second = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("second failure seals");
    let first_identity = failure_replay::derive_failure_identity(&first).expect("derives");
    let second_identity = failure_replay::derive_failure_identity(&second).expect("derives");
    assert_eq!(
        first_identity, second_identity,
        "the same wrong fixture repair reproduces the same failure identity (dedup key)"
    );
}

#[test]
fn failure_identity_excludes_wall_clock_and_differs_from_success() {
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    assert_ne!(
        failure_replay::derive_failure_identity(&failure).expect("derives"),
        failure_replay::derive_failure_identity(&replay).expect("derives"),
        "a repaired run is not the same failure"
    );
}

#[test]
fn sealing_is_deterministic_across_reruns() {
    let first = run_and_seal_chain();
    let second = run_and_seal_chain();
    assert_eq!(first.failure_identity, second.failure_identity);
    assert_eq!(
        first.original_failure.baseline_content_digest,
        second.original_failure.baseline_content_digest
    );
    assert_eq!(
        first.original_failure.post_run_content_digest,
        second.original_failure.post_run_content_digest
    );
    assert_eq!(
        first.replay.post_run_content_digest,
        second.replay.post_run_content_digest
    );
    first.verify().expect("first chain verifies");
    second.verify().expect("second chain verifies");
}

// ---------------------------------------------------------------------------
// A replay is never an unrelated success, and never gains authority
// ---------------------------------------------------------------------------

#[test]
fn replay_from_a_different_scenario_is_rejected_as_unrelated_success() {
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    // An honest, fully verified repaired envelope — but from the DOCUMENTATION
    // fixture, not this bugfix. It may not be presented as this failure's fix.
    let docs_descriptor = include_str!("fixtures/scenarios/rust-docs-sum-execute/descriptor.json");
    let docs_spec = TaskExecutionSpec {
        repair_target: "README.md",
        repo_identity_url: "https://repo.invalid/prometheos/lite-fixtures/rust-docs-sum-execute-001.git",
    };
    let docs_repair = vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("README.md"),
        search: "`sum(a: i64, b: i64) -> i64` returns `a - b`.\n\n## Example\n\n`sum(2, 2) == 0`"
            .to_string(),
        replace: "`sum(a: i64, b: i64) -> i64` returns `a + b`.\n\n## Example\n\n`sum(2, 2) == 4`"
            .to_string(),
        replace_all: Some(false),
        context_lines: None,
    })];
    let unrelated =
        common::task_execution::run_task_execution(&docs_spec, docs_descriptor, docs_repair)
            .envelope
            .expect("docs run seals");
    let err = seal_chain(failure, unrelated).unwrap_err().to_string();
    assert!(
        err.contains("scenario digest")
            || err.contains("scenarioId")
            || err.contains("changedFiles"),
        "{err}"
    );
}

#[test]
fn authority_widening_in_the_replay_descriptor_is_rejected() {
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["authority"]["writableScopes"] = serde_json::json!(["repo://fixture", "repo://other"]);
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    // Widening passes schema validation (scopes are merely required to be
    // non-empty) but fails the binding check: the replay may not hold more
    // authority than the failure did.
    let descriptor = FailureReplayDescriptor::parse_json(&text).expect("still schema-valid");
    let err = LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
        .unwrap_err()
        .to_string();
    assert!(err.contains("writable scopes"), "{err}");
}

#[test]
fn authority_widening_of_changed_files_is_rejected() {
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["authority"]["changedFiles"] = serde_json::json!(["src/lib.rs", "Cargo.toml"]);
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    let descriptor = FailureReplayDescriptor::parse_json(&text).expect("still schema-valid");
    let err = LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
        .unwrap_err()
        .to_string();
    assert!(err.contains("changedFiles"), "{err}");
}

#[test]
fn swapped_validator_between_failure_and_replay_is_rejected() {
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["original"]["validator"]["command"] = serde_json::json!("echo");
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    let descriptor = FailureReplayDescriptor::parse_json(&text).expect("still schema-valid");
    let err = LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
        .unwrap_err()
        .to_string();
    assert!(err.contains("validator.command"), "{err}");
}

// ---------------------------------------------------------------------------
// Negative tests: invalid descriptors and altered evidence
// ---------------------------------------------------------------------------

#[test]
fn zero_exit_pinned_as_the_original_failure_is_rejected() {
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["original"]["validator"]["exitCode"] = serde_json::json!(0);
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let err = FailureReplayDescriptor::parse_json(&text)
        .unwrap_err()
        .to_string();
    assert!(err.contains("non-zero"), "{err}");
}

#[test]
fn changed_failure_classification_in_the_descriptor_is_rejected() {
    // A "setup-error" is not a replayable governed validation failure.
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["original"]["classification"] = serde_json::json!("setup-error");
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let err = FailureReplayDescriptor::parse_json(&text)
        .unwrap_err()
        .to_string();
    assert!(err.contains("governed-product-outcome"), "{err}");

    // And a "repair-failed" claim with a "repaired" taskOutcome is equally
    // incoherent — the failure side may not call itself the repaired side.
    let mut value: serde_json::Value =
        serde_json::from_str(REPLAY_DESCRIPTOR).expect("descriptor json parses");
    value["original"]["taskOutcome"] = serde_json::json!("repaired");
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let err = FailureReplayDescriptor::parse_json(&text)
        .unwrap_err()
        .to_string();
    assert!(err.contains("repair-failed"), "{err}");
}

#[test]
fn unknown_failure_class_fails_closed_at_the_json_layer() {
    let text = REPLAY_DESCRIPTOR.replace("governed-validation-failure", "infrastructure-flake");
    let err = FailureReplayDescriptor::parse_json(&text)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("failure-replay descriptor is not a valid"),
        "{err}"
    );
}

#[test]
fn tampered_scenario_digest_pin_is_rejected() {
    let tampered = REPLAY_DESCRIPTOR.replace(
        "af2d5780917ce9102d6dff0ff8c471866e5a4f0db8f50abf36bce1ab1c886dce",
        "bf2d5780917ce9102d6dff0ff8c471866e5a4f0db8f50abf36bce1ab1c886dce",
    );
    assert_ne!(tampered, REPLAY_DESCRIPTOR);
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    let descriptor = FailureReplayDescriptor::parse_json(&tampered).expect("schema-valid");
    let err = LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
        .unwrap_err()
        .to_string();
    assert!(err.contains("scenarioDigest"), "{err}");
}

#[test]
fn tampered_baseline_pin_is_rejected() {
    let tampered = REPLAY_DESCRIPTOR.replace(
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
        "e4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
    );
    assert_ne!(tampered, REPLAY_DESCRIPTOR);
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let replay = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits())
        .envelope
        .expect("replay seals");
    let descriptor = FailureReplayDescriptor::parse_json(&tampered).expect("schema-valid");
    let err = LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
        .unwrap_err()
        .to_string();
    assert!(err.contains("baselineContentDigest"), "{err}");
}

#[test]
fn tampered_original_failure_envelope_is_rejected() {
    let chain = run_and_seal_chain();
    // Re-present the failure as repaired: flip its sealed task outcome AND
    // reseal its outer digest. The inner envelope's measurements do not
    // support the claim, so the chain fails closed.
    let mut forged = chain;
    forged.original_failure.task_outcome = TaskOutcome::Repaired;
    forged.original_failure.validator.exit_code = Some(0);
    forged.original_failure.content_digest = None;
    forged.original_failure.content_digest = Some(
        forged
            .original_failure
            .compute_digest()
            .expect("outer recompute succeeds"),
    );
    // The sealed-in content digest no longer matches the outer envelope.
    forged.content_digest = None;
    forged.content_digest = Some(forged.compute_digest().expect("recompute succeeds"));
    assert!(
        forged.verify().is_err(),
        "a re-presented failure may not be sealed as the repaired side"
    );
}

#[test]
fn tampered_replay_envelope_is_rejected() {
    let chain = run_and_seal_chain();
    // Pretend the replay touched a different file than the failure did.
    let mut forged = chain;
    forged.replay.repair.changed_files = vec!["Cargo.toml".to_string()];
    forged.replay.content_digest = None;
    forged.replay.content_digest = Some(forged.replay.compute_digest().expect("recompute"));
    forged.content_digest = None;
    forged.content_digest = Some(forged.compute_digest().expect("recompute"));
    assert!(forged.verify().is_err());
}

#[test]
fn stale_envelope_digest_is_rejected() {
    let chain = run_and_seal_chain();
    let mut forged = chain;
    // The failure identity is recorded; drop the content digest (stale).
    forged.content_digest = None;
    assert!(forged.verify().is_err());
}

#[test]
fn forged_success_from_failed_repair_is_rejected_inner() {
    // Take the sealed FAILED task-run envelope, flip the outcome claim and
    // reseal — the pinned expected digests and measurements reject it.
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    let mut forged = failure;
    forged.task_outcome = TaskOutcome::Repaired;
    forged.validator.exit_code = Some(0);
    forged.content_digest = None;
    forged.content_digest = Some(forged.compute_digest().expect("recompute"));
    assert!(
        forged.verify().is_err(),
        "a failed repair may not be re-sealed as a verified repair"
    );
}

// ---------------------------------------------------------------------------
// Preflight refusals and failed dispatch: the original failure is preserved,
// never upgraded to a success
// ---------------------------------------------------------------------------

#[test]
fn replay_with_invalid_revision_produces_no_success_chain() {
    let tampered = EXECUTE_DESCRIPTOR.replace(
        "bfca0ee96857e365026ac07f9a95c514b7e58e8b",
        "bfca0ee96857e365026ac07f9a95c514b7e58e8c",
    );
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let refusal = run_task_execution(&tampered, correct_repair_edits());
    assert_eq!(refusal.classification, ScenarioOutcomeClass::SetupError);
    assert!(refusal.stage_error.is_some());
    assert_eq!(refusal.capability_effects, 0);
    assert!(refusal.envelope.is_none(), "no replay envelope exists");
    // The original failure (computed independently) still verifies on its own.
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    failure.verify().expect("original failure preserved");
    assert_eq!(failure.task_outcome, TaskOutcome::RepairFailed);
}

#[test]
fn replay_with_failed_dispatch_preserves_the_typed_failure_and_no_success() {
    let run = run_task_execution_with_fault(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        DispatchInjection::Http500,
    );
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(
        run.capability_effects, 1,
        "the capability executed and failed"
    );
    match &run.dispatch {
        Some(Err(RoutedDispatchFailure::ExecutionFailed(failure))) => {
            assert!(failure.calls >= 1);
            assert!(failure.journal_entry.is_some());
        }
        other => panic!("expected a typed ExecutionFailed, got {other:?}"),
    }
    let accounting = run.accounting.as_ref().expect("typed failure accounted");
    accounting.verify().expect("failure accounting verifies");
    assert_eq!(
        accounting.outcome,
        prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Failed {
            kind: "execution-failed".to_string()
        }
    );
    assert_eq!(
        accounting.attempt_digest, None,
        "no attempt digest fabricated"
    );
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(run.envelope, None, "a failed dispatch seals no repair");

    // The original failure evidence is preserved exactly as measured; nothing
    // upgrades it to a success.
    let failure = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("failure seals");
    failure.verify().expect("original failure preserved");
    assert_eq!(failure.task_outcome, TaskOutcome::RepairFailed);
}

#[test]
fn replay_with_post_dispatch_refusal_applies_no_repair_and_runs_no_validator() {
    let run = run_task_execution_with_fault(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        DispatchInjection::RefuseStepCapability,
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(run.capability_effects, 0);
    assert!(run.accounting.is_none());
    assert!(!run.patched);
    assert_eq!(run.envelope, None);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn replay_against_an_incompatible_workspace_is_refused() {
    // Forced read-only manifest against the writable-only git worktree:
    // refused before any effect; no replay, no success.
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    let tmp = tempfile::tempdir().expect("temp dir");
    let fixture = common::task_execution::fixture_root(&descriptor);
    let source = tmp.path().join("source");
    let workspaces = tmp.path().join("workspaces");
    common::task_execution::stage_source_repository(&fixture, &source, BUGFIX_REPO_IDENTITY_URL)
        .expect("staging succeeds");
    let mut manifest = common::task_execution::build_workspace_manifest(&EXECUTE_SPEC, &descriptor);
    manifest.mode = prometheos_lite::workflow::workspace::WorkspaceMode::ReadOnly;
    let manifest = manifest.sealed();
    let adapter = prometheos_lite::workflow::workspace::GitWorktreeWorkspace {
        parent_dir: workspaces,
        source_repo: source,
    };
    let refused =
        prometheos_lite::workflow::workspace::WorkspaceAdapter::acquire(&adapter, &manifest);
    match refused {
        Err(error) => {
            let msg = error.to_string();
            assert!(
                msg.contains("writable"),
                "refusal names the writable-only contract: {msg}"
            );
        }
        Ok(_) => panic!("git-worktree must refuse a read-only manifest"),
    }
}

// ---------------------------------------------------------------------------
// Known/unknown accounting is preserved exactly as measured
// ---------------------------------------------------------------------------

#[test]
fn replay_accounting_preserves_known_unknown_values() {
    let failure_run = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits());
    let replay_run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());

    for (label, run) in [("failure", &failure_run), ("replay", &replay_run)] {
        let accounting = run.accounting.as_ref().expect("accounting recorded");
        accounting.verify().expect("accounting verifies");
        // The loopback provider reports usage, the price book is present, and
        // latency is measured. These are KNOWN, not invented; unreported
        // fields would stay Unknown (never zero), per the accounting contract.
        match &accounting.usage {
            prometheos_lite::workflow::accounting::UsageAccounting::Known(usage) => {
                assert_eq!(usage.prompt_tokens, Some(7), "{label} prompt tokens");
                assert_eq!(
                    usage.completion_tokens,
                    Some(4),
                    "{label} completion tokens"
                );
                assert_eq!(usage.total_tokens, Some(11), "{label} total tokens");
            }
            other => panic!("{label} usage must be known for the loopback fixture, got {other:?}"),
        }
        assert!(
            matches!(
                &accounting.cost,
                prometheos_lite::workflow::accounting::CostAccounting::Known { .. }
            ),
            "{label} cost must be known under the pinned price book"
        );
        assert!(
            accounting.latency_ms < 60_000,
            "{label} latency is measured, not invented"
        );

        // Dispatch-level accounting is Completed for both runs: the dispatch
        // succeeded; the task failed only on the validator side.
        assert_eq!(
            accounting.outcome,
            prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Completed,
            "{label} dispatch completed and is accounted as such"
        );

        let graph = accounting_graph(run);
        assert_eq!(graph.attempts, 1, "{label} graph accounts one attempt");
        assert_eq!(
            graph.completed, 1,
            "{label} graph accounts one completed attempt"
        );
    }

    // The replay chain itself carries no invented measurements.
    let chain = run_and_seal_chain();
    chain.verify().expect("chain verifies");
    let failure_accounting = &chain.original_failure.scenario_run.attempt_accounting;
    failure_accounting
        .verify()
        .expect("failure accounting verifies embedded");
    assert!(failure_accounting.attempt_digest.is_some());
    assert!(failure_accounting.journal_digest.is_some());
}

fn accounting_graph(
    run: &TaskExecutionRun,
) -> prometheos_lite::workflow::accounting::GraphAccounting {
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.scenario_run.graph_accounting.clone()
}

// ---------------------------------------------------------------------------
// Exported evidence: no secrets, no machine paths, no Foundry/benchmark claims
// ---------------------------------------------------------------------------

#[test]
fn failure_replay_evidence_carries_no_secret_material_and_no_machine_paths() {
    let chain = run_and_seal_chain();
    let json = failure_replay::failure_replay_json(&chain).expect("serializes");
    assert!(!json.contains(SECRET_CANARY));
    let temp_marker = std::env::temp_dir().to_string_lossy().into_owned();
    let repo_marker = repo_root().to_string_lossy().into_owned();
    for marker in [temp_marker.as_str(), repo_marker.as_str()] {
        assert!(
            !json.contains(marker),
            "machine path leaked into exported evidence: {marker}"
        );
    }
}

#[test]
fn failure_replay_evidence_claims_no_benchmark_success_and_no_foundry_contract() {
    let chain = run_and_seal_chain();
    let json = failure_replay::failure_replay_json(&chain)
        .expect("serializes")
        .to_lowercase();
    for forbidden in ["passrate", "benchmark", "certif", "ranking"] {
        assert!(!json.contains(forbidden), "evidence claims {forbidden:?}");
    }
    assert_eq!(chain.external_evaluation_format.status, "unsupported");
    assert!(
        chain
            .external_evaluation_format
            .contract
            .starts_with("foundry.workflow-evaluation")
    );
    assert_eq!(
        chain.replay.repair.origin,
        TaskRepairOrigin::ScriptedFixture,
        "the replay repair is labelled scripted, never model skill"
    );
    assert!(
        json.contains("\"failureidentity\""),
        "exported evidence carries the stable failure identity"
    );
}

#[test]
fn failure_replay_namespace_and_schema_are_lite_internal() {
    let chain = run_and_seal_chain();
    assert_eq!(chain.namespace, "lite.evidence.failure-replay");
    assert_eq!(chain.schema_version, "1.0.0");
    let json = failure_replay::failure_replay_json(&chain).expect("serializes");
    assert!(!json.contains("lite.fixtures.failure-replay\", \"schemaVersion\": \"2"));
    assert!(json.contains("lite.fixtures.failure-replay"));
}
