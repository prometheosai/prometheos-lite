//! Issue #136 Slice C — executed documentation task fixture.
//!
//! Slice B proved the executed-repair path on a Rust **bugfix**. This suite
//! proves it on a second, distinct executed task type: a **documentation**
//! repair. The fixture crate `fixture-docs-sum` has a CORRECT implementation
//! (`sum(a, b) == a + b`); its README documents the wrong behaviour
//! (`a - b`, `sum(2, 2) == 0`), and the pinned validator is the doc-content
//! check `tests/docs.rs` executed as `cargo test --offline --test docs`.
//!
//! Validation is content-level and reviewable: the validator asserts the
//! README states `a + b` and `sum(2, 2) == 4` and no longer states the wrong
//! claims. It is **not** reported as code-test evidence of anything else,
//! and nothing here claims model coding ability. The repair is a
//! fixture-scripted `search-replace` edit (README.md only, the sole editable
//! file) applied with permission under the same governed boundary and
//! preflight order as Slice B (`tests/common/task_execution.rs`).
//!
//! The committed fixture is never mutated in place. Every refusal class
//! that never reports false success is pinned by test, exactly as in Slice B.

use std::path::PathBuf;

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::scenario::{
    self, LiteTaskRunEvidence, ScenarioDescriptor, ScenarioOutcomeClass, ScenarioWorkspaceAdapter,
    TaskOutcome, TaskRepairOrigin,
};
use prometheos_lite::workflow::workspace::{self, GitWorktreeWorkspace, WorkspaceMode};

mod common;

use common::task_execution::{
    DispatchInjection, TaskExecutionRun, TaskExecutionSpec, build_workspace_manifest, fixture_root,
    repo_root, sha256_hex, stage_source_repository,
};

const DOCS_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-docs-sum-execute/descriptor.json");

/// Stable repository identity for the disposable source repository. A URL
/// identity label bound through the adapter's `git remote get-url origin`
/// branch; never dereferenced, never fetched (the host is an RFC 2606
/// non-resolving name). Carries no machine paths into the evidence chain.
const DOCS_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-docs-sum-execute-001.git";

/// Per-fixture facts for this executed scenario.
const DOCS_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "README.md",
    repo_identity_url: DOCS_REPO_IDENTITY_URL,
};

/// The pinned commit the disposable source repository stages. Must match
/// `repository.revision` in the docs descriptor.
const DOCS_REVISION: &str = "e5a9c0a3f56af865ef7221e1b911df4d3f54895d";

/// Invoke the shared governed task runner for this fixture.
fn run_docs_task(descriptor_text: &str, edits: Vec<EditOperation>) -> TaskExecutionRun {
    common::task_execution::run_task_execution(&DOCS_SPEC, descriptor_text, edits)
}

/// Same, with a deterministic governed-dispatch fault injected.
fn run_docs_task_with_fault(
    descriptor_text: &str,
    edits: Vec<EditOperation>,
    injection: DispatchInjection,
) -> TaskExecutionRun {
    common::task_execution::run_task_execution_with_fault(
        &DOCS_SPEC,
        descriptor_text,
        edits,
        injection,
    )
}

// ---------------------------------------------------------------------------
// Repair actions (fixture-scripted; NEVER model output)
// ---------------------------------------------------------------------------

/// The intended repair: README now documents the true behaviour.
fn correct_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("README.md"),
        search: "`sum(a: i64, b: i64) -> i64` returns `a - b`.\n\n## Example\n\n`sum(2, 2) == 0`"
            .to_string(),
        replace: "`sum(a: i64, b: i64) -> i64` returns `a + b`.\n\n## Example\n\n`sum(2, 2) == 4`"
            .to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

/// A repair that applies cleanly but documents different-but-still-wrong
/// behaviour; the pinned doc-check validator must fail it.
fn wrong_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("README.md"),
        search: "`sum(a: i64, b: i64) -> i64` returns `a - b`.\n\n## Example\n\n`sum(2, 2) == 0`"
            .to_string(),
        replace: "`sum(a: i64, b: i64) -> i64` returns `a * b`.\n\n## Example\n\n`sum(2, 2) == 4`"
            .to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

fn inapplicable_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("README.md"),
        search: "this block does not exist in the fixture README".to_string(),
        replace: "replacement".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

/// The repair targets the (correct) source instead of the README: outside
/// the authorized editable set, refused in the preflight before any effect.
fn out_of_scope_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/lib.rs"),
        search: "a + b".to_string(),
        replace: "a - b".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

fn traversal_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("../escape.txt"),
        search: "x".to_string(),
        replace: "y".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

fn fixture_readme_bytes() -> Vec<u8> {
    std::fs::read(repo_root().join("tests/fixtures/scenarios/rust-docs-sum-execute/repo/README.md"))
        .expect("fixture readable")
}

// ---------------------------------------------------------------------------
// Descriptor pins
// ---------------------------------------------------------------------------

#[test]
fn docs_descriptor_loads_as_a_documentation_kind_with_writable_isolated_workspace() {
    let descriptor =
        ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("docs descriptor must validate");
    assert_eq!(descriptor.scenario_id, "lite-rust-docs-sum-execute-001");
    assert_eq!(descriptor.kind, scenario::ScenarioKind::Documentation);
    assert_eq!(
        descriptor.repository.fixture,
        "tests/fixtures/scenarios/rust-docs-sum-execute/repo"
    );
    assert_eq!(
        descriptor.repository.content_digest,
        "53bb96aad9c2b738507d9f1e06e8c648b3f27816d55fc2dd4efef06c28292a5b",
        "docs fixture content digest is pinned"
    );
    assert_eq!(descriptor.repository.revision, DOCS_REVISION);
    assert_eq!(
        descriptor.workspace.adapter,
        ScenarioWorkspaceAdapter::GitWorktree
    );
    assert_eq!(
        descriptor.workspace.mode,
        scenario::ScenarioWorkspaceMode::Writable
    );
    assert_eq!(descriptor.workspace.writable_scopes, vec!["repo://fixture"]);
    assert_eq!(descriptor.policy.writable_scopes, vec!["repo://fixture"]);
    assert_eq!(
        descriptor.workspace.adapter_revision,
        "lite.workspace.adapter.v1"
    );
    let parsed: serde_json::Value = serde_json::from_str(DOCS_DESCRIPTOR).expect("json parses");
    assert_eq!(parsed["kind"], "documentation");
    assert_eq!(parsed["workspace"]["mode"], "writable");
    assert_eq!(parsed["workspace"]["adapter"], "git-worktree");
}

#[test]
fn docs_descriptor_pins_the_doc_content_validator() {
    let descriptor = ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("validates");
    let validation = descriptor
        .validation
        .as_ref()
        .expect("docs descriptor pins the validator");
    assert_eq!(validation.schema_version, "1.0.0");
    assert_eq!(validation.command, "cargo");
    assert_eq!(validation.args, vec!["test", "--offline", "--test", "docs"]);
    assert_eq!(validation.expected_exit_code, 0);
    assert_eq!(
        validation.expected_assertion,
        "pinned-fixture-task: README.md documents sum(a, b) as a + b and `sum(2, 2) == 4` (and no longer claims a - b or `sum(2, 2) == 0`); verified by `cargo test --offline --test docs` in the isolated workspace"
    );
    assert_eq!(
        validation.expected_file_after_digest,
        "a3f6e7a7649967fc28e55956d88b8643f779fe65192f30e95c806964d1124db4"
    );
    assert_eq!(
        validation.expected_post_run_digest,
        "a6509a10da942186fef8f6d86bbfb001a5137a89f346ef01170f282647e9b120"
    );
}

#[test]
fn modified_fixture_baseline_pin_is_rejected_before_any_effect() {
    let tampered = DOCS_DESCRIPTOR.replace(
        "53bb96aad9c2b738507d9f1e06e8c648b3f27816d55fc2dd4efef06c28292a5b",
        "43bb96aad9c2b738507d9f1e06e8c648b3f27816d55fc2dd4efef06c28292a5b",
    );
    assert_ne!(tampered, DOCS_DESCRIPTOR);
    let run = run_docs_task(&tampered, correct_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.preflight_error.is_some());
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered"
    );
    assert_eq!(run.capability_effects, 0);
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(run.envelope, None);
}

#[test]
fn descriptor_without_validation_pins_is_an_invalid_task_never_dispatched() {
    let mut value: serde_json::Value =
        serde_json::from_str(DOCS_DESCRIPTOR).expect("descriptor json parses");
    value
        .as_object_mut()
        .expect("descriptor is an object")
        .remove("validation");
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let run = run_docs_task(&text, correct_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::InvalidTask);
    assert!(run.preflight_error.is_some());
    assert!(
        run.dispatch.is_none(),
        "no dispatch for an unverifiable task"
    );
    assert_eq!(run.capability_effects, 0);
    assert!(run.accounting.is_none());
    assert!(!run.patched);
    assert_eq!(run.envelope, None);
}

#[test]
fn staged_source_commit_matches_the_descriptor_revision_pin() {
    let descriptor = ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("validates");
    assert_eq!(descriptor.repository.revision, DOCS_REVISION);
    let fixture = fixture_root(&descriptor);
    let tmp = tempfile::tempdir().expect("temp dir");
    let dest = tmp.path().join("source");
    let sha =
        stage_source_repository(&fixture, &dest, DOCS_REPO_IDENTITY_URL).expect("staging succeeds");
    assert_eq!(sha, descriptor.repository.revision);
    let digest = scenario::task_run_tree_digest(&dest).expect("digest");
    assert_eq!(digest, descriptor.repository.content_digest);
}

// ---------------------------------------------------------------------------
// Happy path: the controlled docs repair is applied and validated for real
// ---------------------------------------------------------------------------

#[test]
fn executed_docs_repair_is_applied_validated_and_fully_bound() {
    let run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    assert_eq!(run.stage_error, None, "staging must succeed");
    assert!(
        run.staging_digest_ok,
        "staged source must digest to the pin"
    );
    assert_eq!(run.acquire_error, None, "workspace acquire must succeed");
    assert_eq!(run.patch_error, None, "permitted patch must apply");
    assert!(run.patched);
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    let dispatch = run
        .dispatch
        .as_ref()
        .expect("the governed boundary was entered")
        .as_ref()
        .expect("dispatch completed");
    assert_eq!(dispatch.attempt.outcome_kind, "completed");
    assert_eq!(
        run.capability_effects, 1,
        "the positive case executes exactly once"
    );
    run.accounting
        .as_ref()
        .expect("accounting recorded")
        .verify()
        .expect("accounting record verifies");
    let repaired = run
        .repaired_file_contents
        .as_deref()
        .expect("patched README read back");
    assert!(
        repaired.contains("returns `a + b`"),
        "the repair is in the README:\n{repaired}"
    );
    assert!(
        repaired.contains("sum(2, 2) == 4"),
        "the README states the true example"
    );
    assert!(
        !repaired.contains("returns `a - b`"),
        "the wrong claim is gone"
    );
    assert!(
        !repaired.contains("sum(2, 2) == 0"),
        "the wrong example is gone"
    );

    let envelope = run.envelope.as_ref().expect("task-run envelope seals");
    envelope.verify().expect("task-run envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::Repaired);
    assert_eq!(
        envelope.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(envelope.validator.command, "cargo");
    assert_eq!(
        envelope.validator.args,
        vec!["test", "--offline", "--test", "docs"]
    );
    assert_eq!(envelope.validator.expected_exit_code, 0);
    assert_eq!(
        envelope.validator.provenance,
        prometheos_lite::workflow::scenario::TaskValidatorProvenance::ObservedSubprocess
    );
    assert_eq!(envelope.validator.exit_code, Some(0));
    assert!(!envelope.validator.timed_out);
    assert_eq!(
        envelope.validator.output_digest.as_ref().map(String::len),
        Some(64)
    );
    assert!(
        envelope.validator.output_bytes > 0,
        "doc-check validator output was captured"
    );
    assert_eq!(
        envelope.baseline_content_digest,
        "53bb96aad9c2b738507d9f1e06e8c648b3f27816d55fc2dd4efef06c28292a5b",
        "baseline binds the embedded repository pin"
    );
    assert_eq!(
        envelope.post_run_content_digest,
        "a6509a10da942186fef8f6d86bbfb001a5137a89f346ef01170f282647e9b120",
        "post-run digest equals the pinned expected value"
    );
    assert_eq!(
        envelope.repair.file_after_digest,
        "a3f6e7a7649967fc28e55956d88b8643f779fe65192f30e95c806964d1124db4"
    );
    assert_ne!(
        envelope.baseline_content_digest, envelope.post_run_content_digest,
        "post-run digest must reflect the repair"
    );
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);
    assert_eq!(
        envelope.repair.write_authority,
        prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy,
        "the scripted repair carries its own bounded permit, not the model's authority"
    );
    assert_eq!(envelope.repair.changed_files, vec!["README.md".to_string()]);
    assert_eq!(envelope.namespace, "lite.evidence.task-run");
    assert_eq!(envelope.schema_version, "1.1.0");

    // The governed model node holds no write authority.
    assert_eq!(
        envelope.scenario_run.manifest.writable_scopes,
        Vec::<String>::new(),
        "the governed model node holds no write authority"
    );

    // The repair is one file; nothing else was written by the repair. (The
    // validator itself may create Cargo.lock — a product of running the
    // validator, not of the repair action.)
    let mut disk_changes = run.changed_vs_baseline.clone();
    disk_changes.sort();
    for changed in &disk_changes {
        assert!(
            changed == "README.md" || changed == "Cargo.lock",
            "unexpected file changed in the disposable workspace: {changed}"
        );
    }

    // The authoritative committed fixture is untouched by the whole run.
    let fixture =
        fixture_root(&ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("validates"));
    scenario::verify_fixture_content(
        &fixture,
        "53bb96aad9c2b738507d9f1e06e8c648b3f27816d55fc2dd4efef06c28292a5b",
    )
    .expect("committed fixture must remain byte-identical");
}

#[test]
fn repaired_envelope_reports_only_authorized_effects() {
    let run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    let envelope = run.envelope.as_ref().expect("envelope seals");
    assert_ne!(
        envelope.repair.file_before_digest,
        envelope.repair.file_after_digest
    );
    assert_eq!(
        sha256_hex(fixture_readme_bytes().as_slice()),
        envelope.repair.file_before_digest,
        "the committed README is the pre-image of the repair"
    );
}

#[test]
fn reruns_reproduce_the_stable_canonical_parts() {
    let first_run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    let second_run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    assert_eq!(first_run.capability_effects, 1);
    assert_eq!(second_run.capability_effects, 1);
    let first = first_run.envelope.expect("first run seals");
    let second = second_run.envelope.expect("second run seals");
    assert_eq!(
        first.scenario_run.descriptor.scenario_digest().unwrap(),
        second.scenario_run.descriptor.scenario_digest().unwrap()
    );
    assert_eq!(
        first.baseline_content_digest,
        second.baseline_content_digest
    );
    assert_eq!(
        first.post_run_content_digest,
        second.post_run_content_digest
    );
    assert_eq!(first.repair.patch_digest, second.repair.patch_digest);
    assert_eq!(
        first.repair.file_before_digest,
        second.repair.file_before_digest
    );
    assert_eq!(
        first.repair.file_after_digest,
        second.repair.file_after_digest
    );
    assert_eq!(first.validator.command, second.validator.command);
    assert_eq!(first.validator.args, second.validator.args);
    assert_eq!(
        first.validator.expected_exit_code,
        second.validator.expected_exit_code
    );
    assert_eq!(first.validator.assertion, second.validator.assertion);
    assert_eq!(first.validator.exit_code, second.validator.exit_code);
    assert_eq!(first.validator.provenance, second.validator.provenance);
    first.verify().expect("first rerun verifies");
    second.verify().expect("second rerun verifies");
}

// ---------------------------------------------------------------------------
// Failure classification: wrong docs repair is a task failure, never a success
// ---------------------------------------------------------------------------

#[test]
fn wrong_docs_repair_is_a_task_failure_not_a_success() {
    let run = run_docs_task(DOCS_DESCRIPTOR, wrong_repair_edits());
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
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(
        envelope.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(envelope.validator.exit_code, Some(101));
    assert_ne!(envelope.validator.exit_code, Some(0));
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);
}

#[test]
fn validator_tool_missing_is_infrastructure_error_never_a_success() {
    let missing_tool = DOCS_DESCRIPTOR.replace(
        "\"command\": \"cargo\"",
        "\"command\": \"lite-no-such-cargo-136\"",
    );
    assert_ne!(missing_tool, DOCS_DESCRIPTOR);
    let run = run_docs_task(&missing_tool, correct_repair_edits());
    assert!(run.patched, "the permitted patch was applied");
    assert_eq!(
        run.capability_effects, 1,
        "the governed dispatch ran exactly once"
    );
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::InfrastructureError,
        "a validator that cannot run classifies as infrastructure, not success"
    );
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(envelope.validator.exit_code, None);
    assert!(!envelope.validator.timed_out);
    assert_eq!(
        envelope.validator.provenance,
        prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted
    );
}

#[test]
fn inapplicable_repair_is_setup_error_with_zero_effects() {
    let run = run_docs_task(DOCS_DESCRIPTOR, inapplicable_repair_edits());
    assert!(!run.patched);
    assert!(
        run.repair_preflight_error.is_some(),
        "search block absent must refuse in the preflight, before dispatch"
    );
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered for an inapplicable repair"
    );
    assert_eq!(run.capability_effects, 0, "no capability may execute");
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(run.envelope, None);
    assert_eq!(run.baseline_digest, run.post_run_digest);
}

// ---------------------------------------------------------------------------
// Negative tests: unsafe edits, wrong revision, incompatible workspace
// ---------------------------------------------------------------------------

#[test]
fn unauthorized_edit_to_src_lib_rs_is_rejected_through_runner_before_any_effect() {
    // The README is the only editable file; the repair must not be able to
    // rewrite the (correct) source. Refused in the preflight, before the
    // governed boundary is even entered.
    let run = run_docs_task(DOCS_DESCRIPTOR, out_of_scope_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    let reason = run
        .repair_preflight_error
        .as_deref()
        .expect("preflight refuses the out-of-scope target");
    assert!(
        reason.contains("outside the authorized editable set"),
        "{reason}"
    );
    assert!(!run.patched);
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered"
    );
    assert_eq!(run.capability_effects, 0);
    assert!(run.accounting.is_none());
    assert_eq!(run.envelope, None);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn unauthorized_edit_to_src_lib_rs_produces_zero_filesystem_effects() {
    let descriptor = ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("validates");
    let fixture = fixture_root(&descriptor);
    let tmp = tempfile::tempdir().expect("temp dir");
    let dest = tmp.path().join("source");
    stage_source_repository(&fixture, &dest, DOCS_REPO_IDENTITY_URL).expect("staging succeeds");
    let before = std::fs::read(dest.join("src").join("lib.rs")).expect("readable");

    let policy = {
        let mut p = FilePolicy::strict();
        p.repo_root = dest.clone();
        p.allowed_write_paths = vec![PathBuf::from("README.md")];
        p
    };
    let file_set = FileSet {
        editable: vec![dest.join("README.md")],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    let result = common::task_execution::dispatch_resolved_route_block_on(
        apply_patch_with_rollback(&out_of_scope_repair_edits(), &file_set, &policy),
    );
    assert!(
        result.is_err(),
        "an edit outside the editable set must be refused by the permission boundary"
    );
    let after = std::fs::read(dest.join("src").join("lib.rs")).expect("readable");
    assert_eq!(before, after, "unauthorized edit changed the source");
    assert_eq!(
        scenario::task_run_tree_digest(&dest).expect("digest"),
        descriptor.repository.content_digest,
        "tree digest must be unchanged by a refused edit"
    );
}

#[test]
fn path_traversal_edit_is_refused_through_runner_before_any_effect() {
    let run = run_docs_task(DOCS_DESCRIPTOR, traversal_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    let reason = run
        .repair_preflight_error
        .as_deref()
        .expect("preflight refuses the traversal target");
    assert!(
        reason.contains("traversal") || reason.contains("relative"),
        "{reason}"
    );
    assert!(!run.patched);
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered"
    );
    assert_eq!(run.capability_effects, 0);
    assert!(run.accounting.is_none());
    assert_eq!(run.envelope, None);
}

#[test]
fn wrong_workspace_revision_rejects_before_any_effect() {
    let tampered =
        DOCS_DESCRIPTOR.replace(DOCS_REVISION, "e5a9c0a3f56af865ef7221e1b911df4d3f54895c");
    assert_ne!(tampered, DOCS_DESCRIPTOR);
    let run = run_docs_task(&tampered, correct_repair_edits());
    assert!(run.stage_error.is_some());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered for a bad revision"
    );
    assert_eq!(run.capability_effects, 0);
    assert!(run.accounting.is_none());
    assert_eq!(run.envelope, None);
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn unresolvable_workspace_revision_never_reaches_the_validator() {
    let tampered =
        DOCS_DESCRIPTOR.replace(DOCS_REVISION, "0000000000000000000000000000000000000000");
    let run = run_docs_task(&tampered, correct_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.stage_error.is_some());
    assert_eq!(run.capability_effects, 0);
    assert!(run.dispatch.is_none());
    assert!(!run.patched);
}

#[test]
fn read_only_workspace_adapter_refuses_the_writable_docs_task() {
    let descriptor = ScenarioDescriptor::parse_json(DOCS_DESCRIPTOR).expect("validates");
    let tmp = tempfile::tempdir().expect("temp dir");
    let source = tmp.path().join("source");
    let workspaces = tmp.path().join("workspaces");
    let fixture = fixture_root(&descriptor);
    stage_source_repository(&fixture, &source, DOCS_REPO_IDENTITY_URL).expect("staging succeeds");

    let mut manifest = build_workspace_manifest(&DOCS_SPEC, &descriptor);
    manifest.mode = WorkspaceMode::ReadOnly;
    let manifest = manifest.sealed();
    let adapter = GitWorktreeWorkspace {
        parent_dir: workspaces.clone(),
        source_repo: source,
    };
    let refused = workspace::WorkspaceAdapter::acquire(&adapter, &manifest);
    match refused {
        Err(error) => {
            let err = error.to_string();
            assert!(
                err.contains("writable"),
                "refusal names the writable-only contract: {err}"
            );
        }
        Ok(_) => panic!("git-worktree must refuse a read-only manifest"),
    }
}

#[test]
fn tampered_expected_file_after_digest_refuses_to_seal() {
    let tampered = DOCS_DESCRIPTOR.replace(
        "a3f6e7a7649967fc28e55956d88b8643f779fe65192f30e95c806964d1124db4",
        "b3f6e7a7649967fc28e55956d88b8643f779fe65192f30e95c806964d1124db4",
    );
    assert_ne!(tampered, DOCS_DESCRIPTOR);
    let run = run_docs_task(&tampered, correct_repair_edits());
    assert_eq!(run.capability_effects, 1, "dispatch happened once");
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::InfrastructureError
    );
    assert_eq!(run.envelope, None);
    assert!(run.seal_error.is_some());
}

#[test]
fn tampered_expected_post_run_digest_refuses_to_seal() {
    let tampered = DOCS_DESCRIPTOR.replace(
        "a6509a10da942186fef8f6d86bbfb001a5137a89f346ef01170f282647e9b120",
        "b6509a10da942186fef8f6d86bbfb001a5137a89f346ef01170f282647e9b120",
    );
    assert_ne!(tampered, DOCS_DESCRIPTOR);
    let run = run_docs_task(&tampered, correct_repair_edits());
    assert_eq!(run.capability_effects, 1, "dispatch happened once");
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::InfrastructureError
    );
    assert_eq!(run.envelope, None);
    assert!(run.seal_error.is_some());
}

// ---------------------------------------------------------------------------
// Forged claims of success are rejected
// ---------------------------------------------------------------------------

fn happy_path_envelope() -> LiteTaskRunEvidence {
    let run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    run.envelope.expect("happy path seals")
}

fn reseal(
    mut envelope: LiteTaskRunEvidence,
    f: impl FnOnce(&mut LiteTaskRunEvidence),
) -> LiteTaskRunEvidence {
    f(&mut envelope);
    envelope.content_digest = None;
    envelope.content_digest = Some(envelope.compute_digest().expect("recompute digest"));
    envelope
}

#[test]
fn forged_repaired_claim_with_failing_validator_is_rejected() {
    let run = run_docs_task(DOCS_DESCRIPTOR, wrong_repair_edits());
    let envelope = run.envelope.expect("envelope seals");
    // Spliced success: claim repaired while the validator exit code is not 0.
    let forged = reseal(envelope, |e| {
        e.task_outcome = TaskOutcome::Repaired;
    });
    assert!(forged.verify().is_err());
}

#[test]
fn forged_validator_command_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.validator.command = "echo".to_string();
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("validator.command"), "{err}");
}

#[test]
fn altered_result_receipt_without_provenance_cannot_claim_success() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.validator.provenance =
            prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted;
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("ObservedSubprocess"), "{err}");
}

#[test]
fn altered_result_receipt_zero_bytes_needs_the_empty_digest() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.validator.output_bytes = 0;
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("well-known empty sha256"), "{err}");
}

#[test]
fn unrelated_post_run_digest_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.post_run_content_digest = e.baseline_content_digest.clone();
    });
    assert!(forged.verify().is_err());
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.post_run_content_digest =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("expectedPostRunDigest"), "{err}");
}

#[test]
fn forged_zero_effect_violation_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope.clone(), |e| {
        e.post_run_content_digest = e.baseline_content_digest.clone();
    });
    assert!(forged.verify().is_err());
    let forged_no_change = reseal(envelope, |e| {
        e.repair.applied = false;
        e.repair.changed_files = vec![];
    });
    assert!(forged_no_change.verify().is_err());
}

#[test]
fn unsupported_task_run_schema_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.schema_version = "2.0.0".to_string();
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("unsupported task-run schema version"), "{err}");
}

#[test]
fn flipped_external_format_marker_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.external_evaluation_format.status = "supported".to_string();
    });
    assert!(forged.verify().is_err());
}

#[test]
fn tampered_embedded_attempt_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.scenario_run.attempt.model = Some("some-other-model".to_string());
    });
    assert!(forged.verify().is_err());
}

// ---------------------------------------------------------------------------
// Exported evidence: no secrets, no paths, no Foundry/benchmark claims
// ---------------------------------------------------------------------------

#[test]
fn task_run_evidence_carries_no_secret_material_and_no_machine_paths() {
    let envelope = happy_path_envelope();
    let json = scenario::task_run_json(&envelope).expect("serializes");
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
fn task_run_evidence_claims_no_benchmark_success_and_no_foundry_contract() {
    let envelope = happy_path_envelope();
    let json = scenario::task_run_json(&envelope)
        .expect("serializes")
        .to_lowercase();
    for forbidden in ["passrate", "benchmark", "certif", "ranking"] {
        assert!(!json.contains(forbidden), "evidence claims {forbidden:?}");
    }
    assert_eq!(envelope.external_evaluation_format.status, "unsupported");
    assert_eq!(
        envelope.repair.origin,
        TaskRepairOrigin::ScriptedFixture,
        "scripted repair is labelled scripted, never model skill"
    );
}

// ---------------------------------------------------------------------------
// Dispatch result gate: a failed or refused governed dispatch never
// authorizes the repair
// ---------------------------------------------------------------------------

#[test]
fn governed_refusal_after_preflight_applies_no_repair_and_runs_no_validator() {
    let run = run_docs_task_with_fault(
        DOCS_DESCRIPTOR,
        correct_repair_edits(),
        DispatchInjection::RefuseStepCapability,
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(
        run.capability_effects, 0,
        "the refusal happens at the step gate, before any capability executes"
    );
    match &run.dispatch {
        Some(Err(prometheos_lite::workflow::route_dispatch::RoutedDispatchFailure::Refused(
            refused,
        ))) => {
            let msg = format!("{refused:#}");
            assert!(msg.contains("step capability"), "{msg}");
        }
        other => panic!("expected a typed refusal, got {other:?}"),
    }
    assert!(
        run.accounting.is_none(),
        "a refusal never produces an accounting record"
    );
    assert!(
        !run.patched,
        "a refused dispatch never authorizes the repair"
    );
    assert_eq!(
        run.repaired_file_contents.as_deref().map(str::as_bytes),
        Some(fixture_readme_bytes().as_slice()),
        "README bytes unchanged after the refusal"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted),
        "the validator never runs"
    );
    assert_eq!(run.envelope, None);
    assert_eq!(run.baseline_digest, run.post_run_digest);
}

#[test]
fn capability_failure_after_preflight_keeps_typed_failure_and_applies_no_repair() {
    let run = run_docs_task_with_fault(
        DOCS_DESCRIPTOR,
        correct_repair_edits(),
        DispatchInjection::Http500,
    );
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(
        run.capability_effects, 1,
        "the capability executed (and failed) — counted truthfully, not zero"
    );
    match &run.dispatch {
        Some(Err(
            prometheos_lite::workflow::route_dispatch::RoutedDispatchFailure::ExecutionFailed(
                failure,
            ),
        )) => {
            assert!(failure.calls >= 1);
            assert!(
                failure.journal_entry.is_some(),
                "typed failure journal entry is preserved"
            );
        }
        other => panic!("expected a typed ExecutionFailed, got {other:?}"),
    }
    let accounting = run.accounting.as_ref().expect("typed failure accounted");
    accounting.verify().expect("failure accounting verifies");
    assert_eq!(
        accounting.outcome,
        prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Failed {
            kind: "execution-failed".to_string()
        },
        "typed failure evidence preserved, never a fabricated Completed"
    );
    assert_eq!(
        accounting.attempt_digest, None,
        "no attempt digest fabricated"
    );
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(
        run.repaired_file_contents.as_deref().map(str::as_bytes),
        Some(fixture_readme_bytes().as_slice()),
        "README bytes unchanged after the failed dispatch"
    );
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted),
        "the validator never runs after an execution failure"
    );
    assert_eq!(run.envelope, None, "no task-run success envelope");
    assert_eq!(run.baseline_digest, run.post_run_digest);
}

#[test]
fn positive_path_executes_exactly_once_with_repair_and_validator() {
    let run = run_docs_task(DOCS_DESCRIPTOR, correct_repair_edits());
    assert_eq!(
        run.capability_effects, 1,
        "exactly one capability execution"
    );
    assert!(run.patched, "the one authorized scripted repair applied");
    let mut changed = run.changed_vs_baseline.clone();
    changed.sort();
    assert!(changed.contains(&"README.md".to_string()));
    for c in &changed {
        assert!(
            c == "README.md" || c == "Cargo.lock",
            "unexpected changed file: {c}"
        );
    }
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::ObservedSubprocess),
        "the pinned validator executed"
    );
    let envelope = run.envelope.expect("task-run envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::Repaired);
    assert_eq!(envelope.validator.exit_code, Some(0));
}
