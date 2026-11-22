//! Issue #136 Slice B — one genuinely executed repository-task fixture.
//!
//! Slice A pinned the fixture bytes and ran the governed boundary with a
//! loopback model, but it never applied a patch to the pinned repository or
//! validated the repair. This suite executes ONE real, deterministic
//! repository task end to end. The runner itself lives in
//! `tests/common/task_execution.rs` (shared with Slice C's documentation
//! and failure-replay suites); it runs preflight phases in a load-bearing
//! order:
//!
//! 1. the committed fixture is content-verified against its pin BEFORE any
//!    work;
//! 2. the descriptor's pinned `validation` surface (command/argv/expected
//!    exit code/assertion/expected digests) is present and must hold —
//!    without it the task is invalid and is never dispatched;
//! 3. a disposable, deterministic source repository is staged from those
//!    bytes (fixed identity, fixed commit timestamp) and its staged commit
//!    is checked against the descriptor's revision pin;
//! 4. an isolated, writable git worktree is acquired under the existing
//!    governed workspace contract (#171) and verified (writable-only gate,
//!    `ensure_writable`, adapter validation, revision, baseline digest == pin)
//!    BEFORE any governed effect;
//! 5. the repair target passes an authorization preflight (repo-relative
//!    paths, editable-set gate, transaction dry-run — zero writes);
//! 6. only then does the governed boundary dispatch the loopback model —
//!    with a real capability-effect counter attached;
//! 7. a controlled, fixture-scripted repair is applied through the existing
//!    filesystem permission boundary (`FilePolicy` + `FileSet` +
//!    `apply_patch_with_rollback`) — NEVER the committed source fixture —
//!    under the scripted-fixture workspace/file-policy permit, NOT the
//!    model's authority (the model dispatch holds no write authority);
//! 8. the pinned validator (`cargo test --offline`) runs with its output
//!    captured into a digested receipt with explicit provenance;
//! 9. the post-run tree digest, the patch digest and the validator receipt
//!    are sealed into a Lite-internal task-run envelope bound to the route,
//!    attempt, journal and accounting records of the underlying governed run —
//!    and `verify()` re-derives the outcome from the measurements, rejecting
//!    claims they do not support (e.g. a re-sealed failed repair presented
//!    as success).
//!
//! The repair is a **scripted fixture**, not model output. Nothing here
//! demonstrates model coding ability; no benchmark score, ranking or
//! Foundry claim is produced. What is demonstrated: an authorised,
//! content-pinned repair executed under the governed workspace boundary with
//! sealed, re-verifiable task-execution evidence — and refusal classes that
//! never report false success.

use std::path::PathBuf;

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::route_dispatch::RoutedDispatchFailure;
use prometheos_lite::workflow::scenario::{
    self, LiteTaskRunEvidence, ScenarioDescriptor, ScenarioOutcomeClass, ScenarioWorkspaceAdapter,
    TaskOutcome, TaskRepairOrigin,
};
use prometheos_lite::workflow::workspace::{self, GitWorktreeWorkspace, WorkspaceMode};

mod common;

use common::task_execution::{
    DispatchInjection, NODE_ID, TaskExecutionRun, TaskExecutionSpec, build_workspace_manifest,
    dispatch_resolved_route_block_on, fixture_root, repo_root, sha256_hex, stage_source_repository,
};

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");

/// Stable repository identity for the disposable source repository. A URL
/// identity label bound through the adapter's `git remote get-url origin`
/// branch; never dereferenced, never fetched (the host is an RFC 2606
/// non-resolving name). Carries no machine paths into the evidence chain.
const BUGFIX_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";

/// Per-fixture facts for this executed scenario.
const EXECUTE_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/lib.rs",
    repo_identity_url: BUGFIX_REPO_IDENTITY_URL,
};

/// The pinned commit the disposable source repository stages. Must match
/// `repository.revision` in the execute descriptor.
const EXECUTE_REVISION: &str = "bfca0ee96857e365026ac07f9a95c514b7e58e8b";

/// Invoke the shared governed task runner for this fixture.
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

fn inapplicable_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("src/lib.rs"),
        search: "this block does not exist in the fixture".to_string(),
        replace: "replacement".to_string(),
        replace_all: Some(false),
        context_lines: None,
    })]
}

fn out_of_scope_repair_edits() -> Vec<EditOperation> {
    vec![EditOperation::SearchReplace(SearchReplaceEdit {
        file: PathBuf::from("Cargo.toml"),
        search: "fixture-buggy-sum".to_string(),
        replace: "fixture-buggy-sum".to_string(),
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

// ---------------------------------------------------------------------------
// Descriptor pins
// ---------------------------------------------------------------------------

#[test]
fn execute_descriptor_loads_and_pins_the_writable_isolated_workspace() {
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
        .expect("execute descriptor must validate");
    assert_eq!(
        descriptor.scenario_id,
        "lite-rust-bugfix-arithmetic-execute-001"
    );
    assert_eq!(
        descriptor.repository.fixture,
        "tests/fixtures/scenarios/rust-bugfix-arithmetic/repo"
    );
    assert_eq!(
        descriptor.repository.content_digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
        "execute fixture reuses the pinned rust-bugfix-arithmetic bytes"
    );
    assert_eq!(descriptor.repository.revision, EXECUTE_REVISION);
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
    // The Slice A read-only fixtures keep their pins untouched.
    let parsed: serde_json::Value = serde_json::from_str(EXECUTE_DESCRIPTOR).expect("json parses");
    assert_eq!(parsed["workspace"]["mode"], "writable");
    assert_eq!(parsed["workspace"]["adapter"], "git-worktree");
}

#[test]
fn modified_fixture_baseline_pin_is_rejected_before_any_effect() {
    // Flip one hex digit of the pinned content digest: still hex64, still the
    // same shape, but no longer the pinned bytes.
    let tampered = EXECUTE_DESCRIPTOR.replace(
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
        "d3234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
    );
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(&tampered, correct_repair_edits());
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
fn descriptor_pins_the_validator_argv_assertion_and_expected_digests() {
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    let validation = descriptor
        .validation
        .as_ref()
        .expect("execute descriptor pins the validator");
    assert_eq!(validation.schema_version, "1.0.0");
    assert_eq!(validation.command, "cargo");
    assert_eq!(validation.args, vec!["test", "--offline"]);
    assert_eq!(validation.expected_exit_code, 0);
    assert_eq!(
        validation.expected_assertion,
        "pinned-fixture-task: sum(2, 2) == 4 (src/lib.rs); verified by `cargo test --offline` in the isolated workspace"
    );
    assert_eq!(
        validation.expected_file_after_digest,
        "28c2fe0394cb607ead1337d151fefe63b2c15ffd256725e3dbfd31dc9c576667"
    );
    assert_eq!(
        validation.expected_post_run_digest,
        "6335830cfd80a56ab742fe0c67c44a402695c968efb152c7058ad18d30acb6c7"
    );
}

#[test]
fn descriptor_without_validation_pins_is_an_invalid_task_never_dispatched() {
    let mut value: serde_json::Value =
        serde_json::from_str(EXECUTE_DESCRIPTOR).expect("descriptor json parses");
    value
        .as_object_mut()
        .expect("descriptor is an object")
        .remove("validation");
    let text = serde_json::to_string(&value).expect("descriptor reserializes");
    let run = run_task_execution(&text, correct_repair_edits());
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
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    assert_eq!(descriptor.repository.revision, EXECUTE_REVISION);
    let fixture = fixture_root(&descriptor);
    let tmp = tempfile::tempdir().expect("temp dir");
    let dest = tmp.path().join("source");
    let sha = stage_source_repository(&fixture, &dest, BUGFIX_REPO_IDENTITY_URL)
        .expect("staging succeeds");
    assert_eq!(sha, descriptor.repository.revision);
    let digest = scenario::task_run_tree_digest(&dest).expect("digest");
    assert_eq!(digest, descriptor.repository.content_digest);
}

// ---------------------------------------------------------------------------
// Happy path: the controlled repair is applied and validated for real
// ---------------------------------------------------------------------------

#[test]
fn executed_repair_is_applied_validated_and_fully_bound() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
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
        .expect("patched source read back");
    assert!(
        repaired.contains("    a + b\n"),
        "the repair is in the file:\n{repaired}"
    );
    assert!(
        !repaired.contains("    a - b\n"),
        "the faulty expression is gone"
    );

    let envelope = run.envelope.as_ref().expect("task-run envelope seals");
    envelope.verify().expect("task-run envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::Repaired);
    assert_eq!(
        envelope.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(envelope.validator.command, "cargo");
    assert_eq!(envelope.validator.args, vec!["test", "--offline"]);
    assert_eq!(envelope.validator.expected_exit_code, 0);
    assert_eq!(
        envelope.validator.assertion,
        "pinned-fixture-task: sum(2, 2) == 4 (src/lib.rs); verified by `cargo test --offline` in the isolated workspace"
    );
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
        "cargo test output was captured"
    );
    assert_eq!(
        envelope.baseline_content_digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
        "baseline binds the embedded repository pin"
    );
    assert_eq!(
        envelope.post_run_content_digest,
        "6335830cfd80a56ab742fe0c67c44a402695c968efb152c7058ad18d30acb6c7",
        "post-run digest equals the pinned expected value"
    );
    assert_eq!(
        envelope.repair.file_after_digest,
        "28c2fe0394cb607ead1337d151fefe63b2c15ffd256725e3dbfd31dc9c576667"
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
    assert!(envelope.repair.applied);
    assert_eq!(envelope.repair.changed_files, vec!["src/lib.rs"]);
    assert_eq!(envelope.namespace, "lite.evidence.task-run");
    assert_eq!(envelope.schema_version, "1.1.0");
    assert_eq!(
        envelope.verification_scope,
        scenario::TASK_RUN_VERIFICATION_SCOPE
    );
    assert_eq!(
        envelope.evaluated_at, envelope.scenario_run.evaluated_at,
        "task-run instant binds the embedded run"
    );

    // The underlying governed run sealed its own chain too.
    let scenario_run = &envelope.scenario_run;
    scenario_run
        .verify()
        .expect("embedded scenario-run envelope verifies");
    assert_eq!(scenario_run.namespace, "lite.evidence.scenario-run");
    assert_eq!(scenario_run.graph_accounting.attempts, 1);
    assert_eq!(scenario_run.graph_accounting.completed, 1);
    assert_eq!(scenario_run.manifest.node_id, NODE_ID);
    assert_eq!(
        scenario_run.manifest.writable_scopes,
        Vec::<String>::new(),
        "the governed model node holds no write authority"
    );
    assert_eq!(
        scenario_run.attempt_accounting.attempt_digest.as_deref(),
        scenario_run.attempt.content_digest.as_deref()
    );
    assert_eq!(
        scenario_run.attempt_accounting.journal_digest.as_deref(),
        Some(scenario_run.attempt.journal_entry.entry_digest.as_str())
    );

    // The repair is one file; nothing else was written by the repair. (The
    // validator itself may create Cargo.lock — a product of running the
    // validator, not of the repair action.)
    assert_eq!(
        envelope.repair.changed_files,
        vec!["src/lib.rs".to_string()]
    );
    let mut disk_changes = run.changed_vs_baseline.clone();
    disk_changes.sort();
    for changed in &disk_changes {
        assert!(
            changed == "src/lib.rs" || changed == "Cargo.lock",
            "unexpected file changed in the disposable workspace: {changed}"
        );
    }

    // The authoritative committed fixture is untouched by the whole run.
    let fixture =
        fixture_root(&ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates"));
    scenario::verify_fixture_content(
        &fixture,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
    )
    .expect("committed fixture must remain byte-identical");
}

#[test]
fn repaired_envelope_reports_only_authorized_effects() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
    let envelope = run.envelope.as_ref().expect("envelope seals");
    assert_ne!(
        envelope.repair.file_before_digest,
        envelope.repair.file_after_digest
    );
    assert_eq!(
        sha256_hex(
            std::fs::read(
                repo_root().join("tests/fixtures/scenarios/rust-bugfix-arithmetic/repo/src/lib.rs")
            )
            .expect("fixture readable")
            .as_slice()
        ),
        envelope.repair.file_before_digest,
        "the committed fixture file is the pre-image of the repair"
    );
}

#[test]
fn reruns_reproduce_the_stable_canonical_parts() {
    let first_run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
    let second_run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
    assert_eq!(first_run.capability_effects, 1);
    assert_eq!(second_run.capability_effects, 1);
    let first = first_run.envelope.expect("first run seals");
    let second = second_run.envelope.expect("second run seals");
    // Wall-clock-free, injected parts reproduce; the measured dispatch
    // latency inside the attempt does not, and nothing here claims it does.
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
    // The captured stdout/stderr digest and wall-clock receipt are measured
    // per run and are therefore NOT asserted byte-equal, on purpose.
    first.verify().expect("first rerun verifies");
    second.verify().expect("second rerun verifies");
}

// ---------------------------------------------------------------------------
// Negative tests: no false success, ever
// ---------------------------------------------------------------------------

#[test]
fn wrong_workspace_revision_rejects_before_any_effect() {
    // Flip one hex digit of the pinned revision: still a non-empty string of
    // the right shape, but no resolvable commit of the staged source — the
    // staging proof fails and NOTHING runs: no dispatch, no model/tool
    // effect, no accounting, no evidence sealed.
    let tampered =
        EXECUTE_DESCRIPTOR.replace(EXECUTE_REVISION, "bfca0ee96857e365026ac07f9a95c514b7e58e8c");
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(&tampered, correct_repair_edits());
    assert!(run.stage_error.is_some());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(
        run.dispatch.is_none(),
        "the governed boundary must not be entered for a bad revision"
    );
    assert_eq!(run.capability_effects, 0, "no capability may execute");
    assert!(run.accounting.is_none(), "no fabricated usage or cost");
    assert_eq!(run.envelope, None);
    assert!(
        !run.patched,
        "no repair happens when staging proof is refused"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn unresolvable_workspace_revision_never_reaches_the_validator() {
    let tampered =
        EXECUTE_DESCRIPTOR.replace(EXECUTE_REVISION, "0000000000000000000000000000000000000000");
    let run = run_task_execution(&tampered, correct_repair_edits());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.stage_error.is_some());
    assert_eq!(run.capability_effects, 0, "no capability may execute");
    assert!(run.dispatch.is_none());
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn read_only_workspace_adapter_refuses_the_writable_task() {
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    let tmp = tempfile::tempdir().expect("temp dir");
    let source = tmp.path().join("source");
    let workspaces = tmp.path().join("workspaces");
    let fixture = fixture_root(&descriptor);
    stage_source_repository(&fixture, &source, BUGFIX_REPO_IDENTITY_URL).expect("staging succeeds");

    // Build the manifest with mode forced to read-only: the git-worktree
    // adapter must refuse to acquire it, before any effect.
    let mut manifest = build_workspace_manifest(&EXECUTE_SPEC, &descriptor);
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
fn unauthorized_edit_to_cargo_toml_produces_zero_filesystem_effects() {
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    let fixture = fixture_root(&descriptor);
    let tmp = tempfile::tempdir().expect("temp dir");
    let dest = tmp.path().join("source");
    stage_source_repository(&fixture, &dest, BUGFIX_REPO_IDENTITY_URL).expect("staging succeeds");
    let before = std::fs::read(dest.join("Cargo.toml")).expect("readable");

    let policy = {
        let mut p = FilePolicy::strict();
        p.repo_root = dest.clone();
        p.allowed_write_paths = vec![PathBuf::from("src/lib.rs")];
        p
    };
    let file_set = FileSet {
        editable: vec![dest.join("src").join("lib.rs")],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    let result = dispatch_resolved_route_block_on(apply_patch_with_rollback(
        &out_of_scope_repair_edits(),
        &file_set,
        &policy,
    ));
    assert!(
        result.is_err(),
        "an edit outside the editable set must be refused by the permission boundary"
    );
    let after = std::fs::read(dest.join("Cargo.toml")).expect("readable");
    assert_eq!(before, after, "unauthorized edit changed the file");
}

#[test]
fn unauthorized_cargo_toml_edit_is_rejected_through_runner_before_any_effect() {
    // An out-of-scope repair target is refused in the preflight, before the
    // governed boundary is even entered: zero capability effects, zero file
    // effects, no dispatch, no accounting, no sealed evidence.
    let run = run_task_execution(EXECUTE_DESCRIPTOR, out_of_scope_repair_edits());
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
fn path_traversal_edit_is_refused_through_runner_before_any_effect() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, traversal_repair_edits());
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
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
}

#[test]
fn path_traversal_edit_is_refused_and_creates_nothing() {
    let descriptor = ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR).expect("validates");
    let fixture = fixture_root(&descriptor);
    let tmp = tempfile::tempdir().expect("temp dir");
    let dest = tmp.path().join("source");
    stage_source_repository(&fixture, &dest, BUGFIX_REPO_IDENTITY_URL).expect("staging succeeds");

    let policy = {
        let mut p = FilePolicy::strict();
        p.repo_root = dest.clone();
        p.allowed_write_paths = vec![PathBuf::from("src/lib.rs")];
        p
    };
    let file_set = FileSet {
        editable: vec![dest.join("src").join("lib.rs")],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    let result = dispatch_resolved_route_block_on(apply_patch_with_rollback(
        &traversal_repair_edits(),
        &file_set,
        &policy,
    ));
    assert!(result.is_err(), "path traversal must be refused");
    assert!(
        !tmp.path().join("escape.txt").exists(),
        "traversal edit must not create the escaped file"
    );
    assert_eq!(
        scenario::task_run_tree_digest(&dest).expect("digest"),
        descriptor.repository.content_digest,
        "tree digest must be unchanged by a refused edit"
    );
}

#[test]
fn failing_repair_is_a_task_failure_not_an_infrastructure_crash_and_never_a_success() {
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
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(
        envelope.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_ne!(envelope.validator.exit_code, Some(0));
    assert_ne!(
        envelope.baseline_content_digest, envelope.post_run_content_digest,
        "the failing repair did change the tree"
    );
    // The genuine AI-coding claim is never made for this outcome.
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);
}

#[test]
fn validator_tool_missing_is_infrastructure_error_never_a_success() {
    let missing_tool = EXECUTE_DESCRIPTOR.replace(
        "\"command\": \"cargo\"",
        "\"command\": \"lite-no-such-cargo-136\"",
    );
    assert_ne!(missing_tool, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(&missing_tool, correct_repair_edits());
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
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted)
    );
}

#[test]
fn inapplicable_repair_is_setup_error_with_zero_effects() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, inapplicable_repair_edits());
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
    assert_eq!(
        run.baseline_digest,
        Some("d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6".to_string()),
        "post-run digest equals the baseline pin: zero effects"
    );
    assert_eq!(run.baseline_digest, run.post_run_digest);
}

// ---------------------------------------------------------------------------
// Envelope integrity: forged claims of success are rejected
// ---------------------------------------------------------------------------

fn happy_path_envelope() -> LiteTaskRunEvidence {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
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
    let run = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits());
    let envelope = run.envelope.expect("envelope seals");
    // Spliced success: claim repaired while the validator exit code is not 0.
    let forged = reseal(envelope, |e| {
        e.task_outcome = TaskOutcome::Repaired;
    });
    assert!(forged.verify().is_err());
}

#[test]
fn forged_success_from_failed_repair_with_resealed_envelope_is_rejected() {
    // The concrete review case: take a REALLY failed repair record, flip its
    // exit code to 0, claim a repaired outcome, recompute the outer digest —
    // and attempt verification. The pinned expected-outcome digests reject
    // it: the file bytes are not the pinned repaired bytes, and the tree
    // does not match the pinned post-run digest.
    let run = run_task_execution(EXECUTE_DESCRIPTOR, wrong_repair_edits());
    let envelope = run.envelope.expect("envelope seals");
    let forged = reseal(envelope, |e| {
        e.validator.exit_code = Some(0);
        e.task_outcome = TaskOutcome::Repaired;
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(
        err.contains("expectedFileAfterDigest") || err.contains("expectedPostRunDigest"),
        "{err}"
    );
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
fn forged_validator_args_are_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.validator.args = vec!["build".to_string()];
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("validator.args"), "{err}");
}

#[test]
fn forged_expected_exit_code_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.validator.expected_exit_code = 1;
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("expectedExitCode"), "{err}");
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
    // The recorded digest is for non-empty output; claim the capture was
    // empty. The receipt no longer matches its own digest.
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
    // (post == baseline is rejected shape-wise) — plus an unrelated hex:
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
fn tampered_baseline_not_bound_to_repository_pin_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.baseline_content_digest =
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string();
    });
    let err = forged.verify().unwrap_err().to_string();
    assert!(err.contains("contentDigest"), "{err}");
}

#[test]
fn model_dispatch_holds_no_write_authority_and_the_repair_declares_its_own() {
    let run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
    let envelope = run.envelope.expect("envelope seals");
    assert_eq!(
        envelope.scenario_run.manifest.writable_scopes,
        Vec::<String>::new(),
        "the governed model node holds no write authority"
    );
    assert_eq!(
        envelope.repair.write_authority,
        prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy,
        "the scripted fixture repair carries its own bounded workspace/file-policy permit"
    );
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);
}

#[test]
fn tampered_expected_file_after_digest_refuses_to_seal() {
    // The descriptor-embedded pin no longer matches what runs; sealing
    // refuses and the run is infrastructure, never success.
    let tampered = EXECUTE_DESCRIPTOR.replace(
        "28c2fe0394cb607ead1337d151fefe63b2c15ffd256725e3dbfd31dc9c576667",
        "08c2fe0394cb607ead1337d151fefe63b2c15ffd256725e3dbfd31dc9c576667",
    );
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(&tampered, correct_repair_edits());
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
    let tampered = EXECUTE_DESCRIPTOR.replace(
        "6335830cfd80a56ab742fe0c67c44a402695c968efb152c7058ad18d30acb6c7",
        "8335830cfd80a56ab742fe0c67c44a402695c968efb152c7058ad18d30acb6c7",
    );
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(&tampered, correct_repair_edits());
    assert_eq!(run.capability_effects, 1, "dispatch happened once");
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::InfrastructureError
    );
    assert_eq!(run.envelope, None);
    assert!(run.seal_error.is_some());
}

#[test]
fn forged_zero_effect_violation_is_rejected() {
    let envelope = happy_path_envelope();
    // Spliced zero-effect claim: pretend the tree never moved even though
    // the patch was applied.
    let forged = reseal(envelope.clone(), |e| {
        e.post_run_content_digest = e.baseline_content_digest.clone();
    });
    assert!(forged.verify().is_err());
    // And the converse: drop the change set while still claiming the repair.
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
fn malformed_post_run_digest_is_rejected() {
    let envelope = happy_path_envelope();
    let forged = reseal(envelope, |e| {
        e.post_run_content_digest = "not-hex-at-all".to_string();
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

#[test]
fn tampered_repair_origin_is_not_serializable_as_model_work() {
    // The repair origin enum has exactly one variant: scripted-fixture.
    // Deserializing anything else fails closed at the JSON layer.
    let envelope = happy_path_envelope();
    let mut value = serde_json::to_value(&envelope).expect("serializes");
    value["repair"]["origin"] = serde_json::json!("model-generated");
    let result: Result<LiteTaskRunEvidence, _> = serde_json::from_value(value);
    assert!(result.is_err(), "unknown repair origin must fail closed");
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
    assert!(
        envelope
            .external_evaluation_format
            .contract
            .starts_with("foundry.coding-verdict")
    );
    assert_eq!(
        envelope.repair.origin,
        TaskRepairOrigin::ScriptedFixture,
        "scripted repair is labelled scripted, never model skill"
    );
}

// ---------------------------------------------------------------------------
// Dispatch result gate: a failed or refused governed dispatch never
// authorizes the scripted repair (regression coverage)
// ---------------------------------------------------------------------------

fn fixture_lib_rs_bytes() -> Vec<u8> {
    std::fs::read(
        repo_root().join("tests/fixtures/scenarios/rust-bugfix-arithmetic/repo/src/lib.rs"),
    )
    .expect("fixture readable")
}

#[test]
fn governed_refusal_after_preflight_applies_no_repair_and_runs_no_validator() {
    // Case A: every preflight gate passes (fixture, staging, revision,
    // worktree, baseline, repair authorization dry-run), then the governed
    // boundary refuses at the step-eligibility gate — before any effect.
    // The refusal must stop the run: no repair, no validator, no typed
    // success claim, zero capability effects (there were none), and zero
    // filesystem effects (the tree is untouched).
    let run = run_task_execution_with_fault(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        DispatchInjection::RefuseStepCapability,
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(
        run.capability_effects, 0,
        "the refusal happens at the step gate, before any capability executes"
    );
    match &run.dispatch {
        Some(Err(RoutedDispatchFailure::Refused(refused))) => {
            let msg = format!("{refused:#}");
            assert!(msg.contains("step capability"), "{msg}");
        }
        other => panic!("expected a typed refusal, got {other:?}"),
    }
    assert!(
        run.accounting.is_none(),
        "a refusal never produces an accounting record (no attempt, no usage)"
    );
    assert!(
        !run.patched,
        "a refused dispatch never authorizes the repair"
    );
    assert_eq!(
        run.repaired_file_contents.as_deref().map(str::as_bytes),
        Some(fixture_lib_rs_bytes().as_slice()),
        "source bytes unchanged after the refusal"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted),
        "the validator never runs"
    );
    assert_eq!(run.envelope, None, "no task-run success envelope");
    assert_eq!(
        Some("d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6".to_string()),
        run.baseline_digest.as_ref().map(|d| d.to_string()),
        "baseline digest is the pinned fixture digest"
    );
    assert_eq!(
        run.baseline_digest, run.post_run_digest,
        "zero filesystem effects after a refused dispatch"
    );
}

#[test]
fn capability_failure_after_preflight_keeps_typed_failure_and_applies_no_repair() {
    // Case B: every preflight gate passes, the capability executes and
    // fails — the loopback endpoint returns HTTP 500, the harness fails the
    // effect with a typed failure document, and the governed boundary
    // returns ExecutionFailed. The repair must NOT run and the typed
    // failure/journal/accounting evidence must be preserved, never
    // re-manufactured as Completed.
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
        "the capability executed (and failed) — counted truthfully, not zero"
    );
    match &run.dispatch {
        Some(Err(RoutedDispatchFailure::ExecutionFailed(failure))) => {
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
        Some(fixture_lib_rs_bytes().as_slice()),
        "source bytes unchanged after the failed dispatch"
    );
    assert_eq!(
        run.validator_provenance,
        Some(prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted),
        "the validator never runs after an execution failure"
    );
    assert_eq!(run.envelope, None, "no task-run success envelope");
    assert_eq!(
        run.baseline_digest, run.post_run_digest,
        "zero filesystem effects after a failed dispatch"
    );
}

#[test]
fn positive_path_executes_exactly_once_with_repair_and_validator() {
    // Case C, kept and sharpened: exactly one governed capability effect,
    // exactly one authorized scripted repair, the pinned validator runs,
    // and success is sealed only then.
    let run = run_task_execution(EXECUTE_DESCRIPTOR, correct_repair_edits());
    assert_eq!(
        run.capability_effects, 1,
        "exactly one capability execution"
    );
    assert!(run.patched, "the one authorized scripted repair applied");
    let mut changed = run.changed_vs_baseline.clone();
    changed.sort();
    assert!(changed.contains(&"src/lib.rs".to_string()));
    for c in &changed {
        assert!(
            c == "src/lib.rs" || c == "Cargo.lock",
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
