//! Issue #156 Slice A — Lite-native classified failure → minimized,
//! redacted, versioned evaluation case, with the fix linked back to the
//! failure that motivated it.
//!
//! Pipeline under test (positive end-to-end):
//!
//! ```text
//! PINNED ORIGINAL SOURCE (fixture d4234cd…)
//!         |
//!         v
//! REAL FAILURE   (scripted wrong repair; `cargo test --offline` exits 101)
//!         |
//!         v
//! CLASSIFIED + EVIDENCE-BOUND   (sealed task-run envelope; class governed-product-outcome)
//!         |
//!         v
//! CONTROLLED MINIMIZATION        (remove incidental files one at a time; every
//!                                 accepted removal replayed through the real
//!                                 governed boundary)
//!         |
//!         v
//! SAME FAILURE REPRODUCED        (same signature: cargo / [test --offline] / 101 / repair-failed)
//!         |
//!         v
//! REDACTED LITE CASE             (export scan refuses any registered secret,
//!                                 the canary, or machine-path material)
//!         |
//!         v
//! AUTHORIZED CORRECTION          (scripted correct repair, applied with permission)
//!         |
//!         v
//! REAL VALIDATOR PASSES          (ObservedSubprocess, exit 0)
//!         |
//!         v
//! FIX LINKED TO ORIGINAL FAILURE (case binds reproduction/fix/chain digests)
//! ```
//!
//! Everything here is Lite-internal, fixture-scripted and deterministic. The
//! repair is a **scripted fixture**, never model output; nothing here
//! demonstrates model coding ability, and no benchmark score, ranking,
//! certification, Foundry ingestion or Foundry evaluation claim is produced.
//! `externalEvaluationFormat.status` stays `unsupported` on every record.
//!
//! The minimization proof is real: the committed bugfix fixture is enriched
//! with incidental files, a pinned reduced-tree descriptor is derived per
//! candidate (fresh fixture digest + staged commit), and each candidate is
//! replayed through the same governed boundary used by #136 Slices B/C.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::workflow::accounting::{CostAccounting, UsageAccounting};
use prometheos_lite::workflow::failure_case::{
    self, FAILURE_CASE_NAMESPACE, FAILURE_CASE_SCANNER_ID, FAILURE_CASE_SCHEMA_VERSION,
    FAILURE_CASE_VERIFICATION_SCOPE, FailureSignature, LiteFailureCase, MINIMIZATION_ALGORITHM,
    MinimizationOracle, MinimizedReproduction, ReplaySignature,
};
use prometheos_lite::workflow::failure_replay::{
    FailureReplayClass, FailureReplayDescriptor, LiteFailureReplayEvidence,
};
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::scenario::{
    self, ScenarioOutcomeClass, TaskOutcome, TaskRepairOrigin, fixture_content_digest,
};

mod common;

use common::task_execution::{
    TaskExecutionSpec, fixture_root, repo_root, run_task_execution, stage_source_repository,
};

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");
const REPLAY_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-replay/descriptor.json");
const DOCS_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-docs-sum-execute/descriptor.json");
const BUGFIX_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";
const DOCS_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-docs-sum-execute-001.git";

const EXECUTE_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/lib.rs",
    repo_identity_url: BUGFIX_REPO_IDENTITY_URL,
};

const DOCS_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "README.md",
    repo_identity_url: DOCS_REPO_IDENTITY_URL,
};

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

fn docs_correct_repair_edits() -> Vec<EditOperation> {
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

fn known_secrets() -> Vec<String> {
    vec![SECRET_CANARY.to_string()]
}

// ---------------------------------------------------------------------------
// Shared chain (built once per test process)
// ---------------------------------------------------------------------------

static SHARED_CHAIN: OnceLock<LiteFailureReplayEvidence> = OnceLock::new();

fn shared_chain() -> &'static LiteFailureReplayEvidence {
    SHARED_CHAIN.get_or_init(|| {
        let failure = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, wrong_repair_edits())
            .envelope
            .expect("the pinned failure seals a task-run envelope");
        let replay = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, correct_repair_edits())
            .envelope
            .expect("the replay seals a task-run envelope");
        let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR)
            .expect("the committed replay descriptor must validate");
        LiteFailureReplayEvidence::sealed(descriptor, failure, replay)
            .expect("failure-replay evidence seals")
    })
}

fn base_minimization() -> MinimizedReproduction {
    let committed = fixture_root(
        &prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
            .expect("committed descriptor validates"),
    );
    let digest = fixture_content_digest(&committed).expect("committed fixture digests");
    MinimizedReproduction {
        algorithm: MINIMIZATION_ALGORITHM.to_string(),
        required_paths: vec!["Cargo.toml".to_string(), "src/lib.rs".to_string()],
        original_tree_digest: digest.clone(),
        minimized_tree_digest: digest,
        removed_paths: Vec::new(),
        rejected_candidates: Vec::new(),
        steps: Vec::new(),
    }
}

fn base_case() -> LiteFailureCase {
    LiteFailureCase::sealed(shared_chain(), base_minimization(), &[])
        .expect("the base failure case seals")
}

/// Clone a sealed case with one mutation and re-seal its digest. This
/// mirrors what an attacker hand-crafting an export would do; `verify` /
/// `verify_against_chain` must still refuse it.
fn redigest(mut case: LiteFailureCase) -> LiteFailureCase {
    case.content_digest = None;
    case.content_digest = Some(case.compute_digest().expect("case re-serializes"));
    case
}

// ---------------------------------------------------------------------------
// Enriched fixture + oracle (real replays against reduced trees)
// ---------------------------------------------------------------------------

static ENRICH_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique_scratch(tag: &str) -> PathBuf {
    let id = ENRICH_COUNTER.fetch_add(1, Ordering::SeqCst);
    repo_root().join(format!(
        "target/failure-case-min/{}-{id}-{tag}",
        std::process::id()
    ))
}

fn copy_tree(source: &Path, dest: &Path) {
    for entry in std::fs::read_dir(source).expect("fixture dir readable") {
        let entry = entry.expect("entry readable");
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            std::fs::create_dir_all(&to).expect("dest dir");
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("fixture file copies");
        }
    }
}

/// An enriched copy of the pinned fixture: the committed bytes plus
/// incidental material that does not participate in the pinned failure.
struct EnrichedFixture {
    root: PathBuf,
    rel: String,
}

impl Drop for EnrichedFixture {
    fn drop(&mut self) {
        if let Some(parent) = self.root.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

fn build_enriched_fixture() -> EnrichedFixture {
    let scratch = unique_scratch("enriched");
    let root = scratch.join("repo");
    std::fs::create_dir_all(root.join("docs/nested")).expect("docs dir");
    std::fs::create_dir_all(root.join("tests")).expect("tests dir");
    let committed = fixture_root(
        &prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
            .expect("committed descriptor validates"),
    );
    copy_tree(&committed, &root);
    std::fs::write(
        root.join("README.md"),
        "# rust-bugfix-arithmetic\n\nDescribes the pinned evaluation fixture. Incidental to the pinned task.\n",
    )
    .expect("README written");
    std::fs::write(
        root.join("docs/notes.txt"),
        "incidental design notes; not part of the pinned task\n",
    )
    .expect("notes written");
    std::fs::write(
        root.join("docs/nested/deep.txt"),
        "deeply nested incidental notes\n",
    )
    .expect("deep notes written");
    std::fs::write(
        root.join("tests/extra_probe.rs"),
        "#[test]\nfn sum_is_addition() {\n    assert_eq!(fixture_buggy_sum::sum(2, 2), 4);\n}\n",
    )
    .expect("extra probe written");
    let rel = scratch
        .strip_prefix(repo_root())
        .expect("scratch lives under the repo root")
        .join("repo")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    EnrichedFixture { root, rel }
}

fn repo_relative_forward_slash(root: &Path) -> String {
    root.strip_prefix(repo_root())
        .expect("tree lives under the repo root")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

struct BugfixOracle {
    descriptor_base: &'static str,
    spec: &'static TaskExecutionSpec,
    edits: fn() -> Vec<EditOperation>,
}

impl MinimizationOracle for BugfixOracle {
    fn replay_reduced_tree(
        &mut self,
        tree_root: &Path,
        _step_index: usize,
    ) -> anyhow::Result<ReplaySignature> {
        let digest = fixture_content_digest(tree_root)?;
        let rel = repo_relative_forward_slash(tree_root);
        let scratch = unique_scratch("stage");
        let staged_sha = stage_source_repository(tree_root, &scratch, self.spec.repo_identity_url)
            .map_err(anyhow::Error::msg)?;
        let descriptor = failure_case::derive_reduced_descriptor(
            self.descriptor_base,
            &rel,
            &digest,
            &staged_sha,
        )?;
        let run = run_task_execution(self.spec, &descriptor, (self.edits)());
        let _ = std::fs::remove_dir_all(&scratch);
        let observed = match run.envelope {
            Some(envelope) => ReplaySignature {
                sealed: true,
                task_outcome: Some(envelope.task_outcome),
                classification: Some(envelope.classification),
                // The distinct failure class is *derived*: a sealed
                // governed-product-outcome envelope of outcome repair-failed
                // with the repair applied and a non-zero validator exit is by
                // definition the governed-validation-failure.
                failure_class: (envelope.task_outcome == TaskOutcome::RepairFailed
                    && envelope.classification == ScenarioOutcomeClass::GovernedProductOutcome
                    && envelope.validator.exit_code.is_some_and(|code| code != 0)
                    && envelope.repair.applied)
                    .then_some(FailureReplayClass::GovernedValidationFailure),
                validator_command: Some(envelope.validator.command.clone()),
                validator_args: envelope.validator.args.clone(),
                validator_exit_code: envelope.validator.exit_code,
            },
            None => ReplaySignature {
                sealed: false,
                task_outcome: None,
                classification: Some(run.classification),
                failure_class: None,
                validator_command: None,
                validator_args: Vec::new(),
                validator_exit_code: None,
            },
        };
        Ok(observed)
    }
}

fn bugfix_wrong_oracle() -> BugfixOracle {
    BugfixOracle {
        descriptor_base: EXECUTE_DESCRIPTOR,
        spec: &EXECUTE_SPEC,
        edits: wrong_repair_edits,
    }
}

// ---------------------------------------------------------------------------
// Positive end-to-end: pinned failure -> minimized case -> linked fix
// ---------------------------------------------------------------------------

#[test]
fn positive_end_to_end_pinned_failure_to_linked_fix() {
    let chain = shared_chain();

    // ---- The original failure is real, classified and evidence-bound -------
    let failure = &chain.original_failure;
    assert_eq!(failure.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(
        failure.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(failure.validator.command, "cargo");
    assert_eq!(
        failure.validator.args,
        vec!["test".to_string(), "--offline".to_string()]
    );
    assert_eq!(failure.validator.exit_code, Some(101));
    assert_eq!(failure.repair.origin, TaskRepairOrigin::ScriptedFixture);
    assert!(failure.repair.applied);
    assert_eq!(
        chain.descriptor.original.failure_class,
        FailureReplayClass::GovernedValidationFailure
    );

    // Baseline / revision / fixture pins come from the pinned descriptor.
    let descriptor = &failure.scenario_run.descriptor;
    assert_eq!(
        descriptor.repository.revision,
        "bfca0ee96857e365026ac07f9a95c514b7e58e8b"
    );
    assert_eq!(
        descriptor.repository.content_digest,
        "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6"
    );

    // ---- The fix is the same task under the same authority -----------------
    let repaired = &chain.replay;
    assert_eq!(repaired.task_outcome, TaskOutcome::Repaired);
    assert_eq!(
        repaired.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(repaired.validator.exit_code, Some(0));
    assert_eq!(
        repaired.validator.provenance,
        prometheos_lite::workflow::scenario::TaskValidatorProvenance::ObservedSubprocess
    );
    assert_eq!(
        repaired.repair.changed_files,
        vec!["src/lib.rs".to_string()]
    );

    // ---- Controlled minimization over an enriched copy ---------------------
    let signature = FailureSignature::from_chain(chain).expect("signature derives");
    let enriched = build_enriched_fixture();
    let committed_digest = fixture_content_digest(&fixture_root(
        &prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
            .expect("committed descriptor validates"),
    ))
    .expect("committed fixture digests");
    let enriched_digest = fixture_content_digest(&enriched.root).expect("enriched fixture digests");
    assert_ne!(
        enriched_digest, committed_digest,
        "the enriched copy must differ"
    );

    let required = vec!["Cargo.toml".to_string(), "src/lib.rs".to_string()];
    let minimization = failure_case::minimize_fixture_tree(
        &enriched.root,
        &required,
        &signature,
        &mut bugfix_wrong_oracle(),
    )
    .expect("minimization runs against the real boundary");

    // Every accepted removal was a real replay that reproduced the pinned
    // failure signature (cargo test --offline / exit 101); every accepted
    // step records the validator's observed exit code.
    for path in [
        "README.md",
        "docs/notes.txt",
        "docs/nested/deep.txt",
        "tests/extra_probe.rs",
    ] {
        assert!(
            minimization
                .removed_paths
                .iter()
                .any(|removed| removed == path),
            "incidental {path:?} must be removed, got {:?}",
            minimization.removed_paths
        );
    }
    assert_eq!(minimization.removed_paths.len(), 4);
    assert!(minimization.rejected_candidates.is_empty());
    assert_eq!(minimization.steps.len(), 4);
    for step in &minimization.steps {
        assert!(step.accepted, "step {:?} accepted", step.path);
        assert_eq!(
            step.validator_exit_code,
            Some(101),
            "step {:?} replayed the real failure",
            step.path
        );
        assert!(
            failure_case::collect_fixture_files(&enriched.root)
                .expect("reduced tree lists")
                .iter()
                .all(|f| f != &step.path),
            "accepted removal physically absent from the reduced tree"
        );
    }
    // The reduced tree is the committed fixture byte-for-byte, by digest.
    assert_eq!(minimization.original_tree_digest, enriched_digest);
    assert_eq!(minimization.minimized_tree_digest, committed_digest);
    assert!(
        !enriched.root.join("docs").exists(),
        "pruned directories stay pruned"
    );
    assert!(!enriched.root.join("tests").exists());
    assert!(!enriched.root.join("README.md").exists());

    // ---- Redacted Lite case ------------------------------------------------
    let case = LiteFailureCase::sealed(chain, minimization.clone(), &known_secrets())
        .expect("the failure case seals");
    case.verify().expect("the case verifies");
    case.verify_against_chain(chain)
        .expect("the case binds to the chain");

    // The case binds both halves of the fix to the original failure identity.
    assert_eq!(case.failure_identity, chain.failure_identity);
    assert_eq!(
        case.reproduction_evidence_digest,
        chain
            .original_failure
            .content_digest
            .as_deref()
            .expect("failure digest")
    );
    assert_eq!(
        case.fix_evidence_digest,
        chain.replay.content_digest.as_deref().expect("fix digest")
    );
    assert_eq!(
        case.chain_evidence_digest,
        chain.content_digest.as_deref().expect("chain digest")
    );
    assert_eq!(
        case.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    assert_eq!(case.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(
        case.failure_class,
        FailureReplayClass::GovernedValidationFailure
    );
    assert_eq!(case.validator.exit_code, 101);
    assert_eq!(case.expected_result.task_outcome, TaskOutcome::Repaired);
    assert_eq!(case.expected_result.validator_exit_code, 0);
    assert_eq!(
        case.source.repository_revision,
        "bfca0ee96857e365026ac07f9a95c514b7e58e8b"
    );
    assert_eq!(case.source.harness_id, "lite.harness.local_model");
    assert_eq!(case.source.provider_identity, "demo-local");
    assert_eq!(case.source.model, "tiny-local-model");
    assert_eq!(
        case.source.repository_fixture,
        "tests/fixtures/scenarios/rust-bugfix-arithmetic/repo"
    );
    assert_eq!(case.source.repository_baseline_digest, committed_digest);
    assert!(!case.controlled_dimensions.is_empty());
    assert_eq!(case.redaction.known_secrets_registered, 1);
    assert_eq!(case.redaction.leaked_secrets, 0);
    assert!(case.redaction.machine_path_free);
    assert_eq!(case.redaction.scanner, FAILURE_CASE_SCANNER_ID);
    assert_eq!(case.external_evaluation_format.status, "unsupported");
    assert_eq!(case.provenance.runner, "lite-failure-case-exporter");
    assert_eq!(case.schema_version, FAILURE_CASE_SCHEMA_VERSION);
    assert_eq!(case.namespace, FAILURE_CASE_NAMESPACE);
    assert_eq!(case.verification_scope, FAILURE_CASE_VERIFICATION_SCOPE);

    // Export: no secret, no canary, no machine path, no machine marker.
    let json = failure_case::case_export_json(&case, &known_secrets()).expect("export scans clean");
    assert!(!json.contains(SECRET_CANARY));
    let temp_marker = std::env::temp_dir().to_string_lossy().into_owned();
    let repo_marker = repo_root().to_string_lossy().into_owned();
    for marker in [temp_marker.as_str(), repo_marker.as_str()] {
        assert!(
            !json.contains(marker),
            "machine path leaked into the case export: {marker}"
        );
    }

    // ---- Deterministic re-export: the same failure re-exports byte-for-byte,
    // arming no second effect ----------------------------------------------
    let case_again = LiteFailureCase::sealed(chain, minimization, &[]).expect("re-export seals");
    assert_eq!(case.content_digest, case_again.content_digest);
    assert_eq!(
        serde_json::to_string_pretty(&case).expect("serializes"),
        serde_json::to_string_pretty(&case_again).expect("serializes")
    );
}

#[test]
fn minimization_rejects_a_reduction_when_the_failure_no_longer_reproduces() {
    let chain = shared_chain();
    let signature = FailureSignature::from_chain(chain).expect("signature derives");
    let enriched = build_enriched_fixture();

    // An artificially narrow required set: only Cargo.toml is pinned.
    // src/lib.rs is a candidate; removing it makes the repair patch
    // inapplicable, so the pinned failure no longer reproduces (the run
    // seals a different outcome/class or refuses) — the candidate must be
    // rejected and the file restored byte-for-byte.
    let original_lib_bytes =
        std::fs::read(enriched.root.join("src/lib.rs")).expect("lib.rs readable");
    let minimization = failure_case::minimize_fixture_tree(
        &enriched.root,
        &["Cargo.toml".to_string()],
        &signature,
        &mut bugfix_wrong_oracle(),
    )
    .expect("minimization runs");

    let rejection = minimization
        .rejected_candidates
        .iter()
        .find(|rejection| rejection.path == "src/lib.rs")
        .expect("src/lib.rs removal must be rejected");
    assert!(!rejection.reason.trim().is_empty());
    assert!(
        !rejection
            .reason
            .contains(repo_root().to_string_lossy().as_ref()),
        "rejection reasons must not leak machine paths: {}",
        rejection.reason
    );
    // Restored byte-for-byte.
    assert_eq!(
        std::fs::read(enriched.root.join("src/lib.rs")).expect("lib.rs restored"),
        original_lib_bytes
    );
    // The rejected step is recorded with accepted=false; lib.rs is not removed.
    assert!(
        minimization
            .steps
            .iter()
            .any(|step| step.path == "src/lib.rs" && !step.accepted)
    );
    assert!(!minimization.removed_paths.iter().any(|p| p == "src/lib.rs"));
    // Incidental files still minimized away.
    for path in [
        "README.md",
        "docs/notes.txt",
        "docs/nested/deep.txt",
        "tests/extra_probe.rs",
    ] {
        assert!(
            minimization
                .removed_paths
                .iter()
                .any(|removed| removed == path)
        );
    }
}

#[test]
fn minimization_refuses_an_invalid_required_set() {
    let signature = FailureSignature::from_chain(shared_chain()).expect("signature derives");
    let enriched = build_enriched_fixture();

    // Required path that does not exist.
    let result = failure_case::minimize_fixture_tree(
        &enriched.root,
        &["does/not/exist.txt".to_string()],
        &signature,
        &mut bugfix_wrong_oracle(),
    );
    assert!(result.is_err());

    // Duplicate required paths.
    let result = failure_case::minimize_fixture_tree(
        &enriched.root,
        &["Cargo.toml".to_string(), "Cargo.toml".to_string()],
        &signature,
        &mut bugfix_wrong_oracle(),
    );
    assert!(result.is_err());

    // Traversal in a required path.
    let result = failure_case::minimize_fixture_tree(
        &enriched.root,
        &["../outside.rs".to_string()],
        &signature,
        &mut bugfix_wrong_oracle(),
    );
    assert!(result.is_err());
}

#[test]
fn derive_reduced_descriptor_rewrites_only_repository_pins() {
    let digest = "a".repeat(64);
    let revision = "b".repeat(40);
    let rendered = failure_case::derive_reduced_descriptor(
        EXECUTE_DESCRIPTOR,
        "target/example/repo",
        &digest,
        &revision,
    )
    .expect("derived descriptor validates");
    let original: serde_json::Value =
        serde_json::from_str(EXECUTE_DESCRIPTOR).expect("descriptor parses");
    let derived: serde_json::Value = serde_json::from_str(&rendered).expect("derived parses");
    assert_eq!(derived["repository"]["fixture"], "target/example/repo");
    assert_eq!(derived["repository"]["contentDigest"], digest);
    assert_eq!(derived["repository"]["revision"], revision);
    // Everything else is carried over verbatim.
    for key in [
        "task",
        "expected",
        "requirements",
        "policy",
        "workspace",
        "harness",
        "route",
        "dimensions",
        "validation",
        "scenarioId",
        "title",
    ] {
        assert_eq!(
            original[key], derived[key],
            "pin {key:?} must be carried over verbatim"
        );
    }

    // Rejected: traversal, absolute-ish path, uppercase digest, empty revision.
    assert!(
        failure_case::derive_reduced_descriptor(
            EXECUTE_DESCRIPTOR,
            "../escape",
            &digest,
            &revision
        )
        .is_err()
    );
    assert!(
        failure_case::derive_reduced_descriptor(EXECUTE_DESCRIPTOR, "C:/abs", &digest, &revision)
            .is_err()
    );
    assert!(
        failure_case::derive_reduced_descriptor(
            EXECUTE_DESCRIPTOR,
            "target/x",
            &"A".repeat(64),
            &revision
        )
        .is_err()
    );
    assert!(
        failure_case::derive_reduced_descriptor(EXECUTE_DESCRIPTOR, "target/x", &digest, "")
            .is_err()
    );
    assert!(
        failure_case::derive_reduced_descriptor(
            EXECUTE_DESCRIPTOR,
            "target/x",
            &digest,
            &"G".repeat(40)
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// Negative matrix: every failure mode refuses to become a case
// ---------------------------------------------------------------------------

#[test]
fn changed_original_failure_class_is_refused() {
    // Case-level: classification drift out of governed-product-outcome.
    let mut case = base_case();
    case.classification = ScenarioOutcomeClass::SetupError;
    assert!(
        redigest(case).verify().is_err(),
        "setup-error is not a governed failure"
    );

    // Case-level: a "failure" that claims the repair succeeded is not a failure.
    let mut case = base_case();
    case.task_outcome = TaskOutcome::Repaired;
    assert!(redigest(case).verify().is_err());

    // Chain-level: any post-seal tamper of the original failure's classification
    // is caught before the case can be sealed.
    let chain = shared_chain().clone();
    let mut tampered = chain;
    tampered.original_failure.classification = ScenarioOutcomeClass::InfrastructureError;
    assert!(tampered.verify().is_err());
    assert!(LiteFailureCase::sealed(&tampered, base_minimization(), &[]).is_err());
}

#[test]
fn minimized_case_that_no_longer_reproduces_is_rejected() {
    // Sealed case claiming to have dropped a required pin.
    let mut minimization = base_minimization();
    minimization.removed_paths = vec!["src/lib.rs".to_string()];
    minimization.steps = vec![prometheos_lite::workflow::failure_case::MinimizationStep {
        path: "src/lib.rs".to_string(),
        accepted: true,
        resulting_tree_digest: "c".repeat(64),
        validator_exit_code: Some(101),
    }];
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());

    // A stale minimized surface (does not equal the pinned baseline) is
    // refused at seal time.
    let mut minimization = base_minimization();
    minimization.minimized_tree_digest = "f".repeat(64);
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());

    // Steps and removedPaths disagree.
    let mut minimization = base_minimization();
    minimization.removed_paths = vec!["README.md".to_string()];
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());

    // The minimized digest is not a digest at all.
    let mut minimization = base_minimization();
    minimization.minimized_tree_digest = "not-a-digest".to_string();
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());

    // A rejected candidate recorded as accepted (or vice versa) fails validation.
    let mut minimization = base_minimization();
    minimization.rejected_candidates = vec![
        prometheos_lite::workflow::failure_case::MinimizationRejection {
            path: "README.md".to_string(),
            reason: "pinned failure signature drifted".to_string(),
        },
    ];
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());
}

#[test]
fn altered_validator_command_or_arguments_is_refused() {
    let case = base_case();

    // Swapped executable.
    let mut evil = case.clone();
    evil.validator.command = "cargotest".to_string();
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Dropped argument.
    let mut evil = case.clone();
    evil.validator.args = vec!["test".to_string()];
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Reordered arguments (a different command line entirely).
    let mut evil = case.clone();
    evil.validator.args = vec!["--offline".to_string(), "test".to_string()];
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Zero-exit failure pin is refused outright.
    let mut evil = case.clone();
    evil.validator.exit_code = 0;
    assert!(redigest(evil).verify().is_err());

    // Chain-level: a tampered validation pin never reaches a case.
    let mut tampered_chain = shared_chain().clone();
    tampered_chain.descriptor.original.validator.args = vec!["check".to_string()];
    assert!(LiteFailureCase::sealed(&tampered_chain, base_minimization(), &[]).is_err());
}

#[test]
fn stale_repository_revision_is_refused() {
    let mut evil = base_case();
    evil.source.repository_revision = "0".repeat(40);
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    let mut evil = base_case();
    evil.source.repository_revision = "../escape".to_string();
    assert!(redigest(evil).verify().is_err());
}

#[test]
fn tampered_baseline_digest_is_refused() {
    let mut evil = base_case();
    evil.source.repository_baseline_digest = "e".repeat(64);
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // A stale minimization baseline is refused at seal time too.
    let mut minimization = base_minimization();
    minimization.original_tree_digest = "f".repeat(64);
    assert!(LiteFailureCase::sealed(shared_chain(), minimization, &[]).is_err());
}

#[test]
fn changed_effective_authority_is_refused() {
    // Widened writable scope.
    let mut evil = base_case();
    evil.authority
        .writable_scopes
        .push("repo://elsewhere".to_string());
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Widened changed-files surface.
    let mut evil = base_case();
    evil.authority
        .changed_files
        .push("tests/extra_probe.rs".to_string());
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Narrowed changed-files surface is equally inauthentic.
    let mut evil = base_case();
    evil.authority.changed_files = vec!["Cargo.toml".to_string()];
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    // Authority origin drift.
    let mut evil = base_case();
    evil.authority.repair_origin =
        prometheos_lite::workflow::scenario::TaskRepairOrigin::ModelAuthored;
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());
}

#[test]
fn unauthorized_writable_scope_in_the_chain_is_refused() {
    let mut tampered_chain = shared_chain().clone();
    tampered_chain
        .descriptor
        .authority
        .writable_scopes
        .push("repo://elsewhere".to_string());
    assert!(tampered_chain.verify().is_err());
    assert!(LiteFailureCase::sealed(&tampered_chain, base_minimization(), &[]).is_err());
}

#[test]
fn missing_original_failure_evidence_is_refused() {
    let mut evil = base_case();
    evil.reproduction_evidence_digest = "0".repeat(64);
    assert!(redigest(evil).verify_against_chain(shared_chain()).is_err());

    let mut evil = base_case();
    evil.reproduction_evidence_digest = "not-a-digest".to_string();
    assert!(redigest(evil).verify().is_err());

    // Chain whose embedded failure envelope is corrupt cannot export a case.
    let mut tampered_chain = shared_chain().clone();
    tampered_chain.original_failure.validator.exit_code = Some(1);
    assert!(LiteFailureCase::sealed(&tampered_chain, base_minimization(), &[]).is_err());
}

#[test]
fn a_secret_surviving_redaction_refuses_the_export() {
    // No known secret may appear: sealing with the canary registered and a
    // clean case passes and records the registration.
    let case = LiteFailureCase::sealed(shared_chain(), base_minimization(), &known_secrets())
        .expect("seals with the canary registered");
    assert_eq!(case.redaction.known_secrets_registered, 1);
    let json = failure_case::case_export_json(&case, &known_secrets()).expect("export scans clean");
    assert!(!json.contains(SECRET_CANARY));

    // Attacker-in-the-middle: a case whose bytes carry the canary never
    // exports, even with a recomputed contentDigest.
    let mut evil = case.clone();
    evil.title = format!("shrink the canary {SECRET_CANARY} please");
    let evil = redigest(evil);
    assert!(failure_case::case_export_json(&evil, &known_secrets()).is_err());

    // Leaked-secret accounting in the redaction record is fail-closed too.
    let mut evil = base_case();
    evil.redaction.leaked_secrets = 1;
    assert!(redigest(evil).verify().is_err());

    // The canary scan flag is mandatory.
    let mut evil = base_case();
    evil.redaction.canary_scanned = false;
    assert!(redigest(evil).verify().is_err());

    // Machine-path material is refused by the export scan.
    let mut evil = base_case();
    evil.title = "pinned C:\\Users\\operator\\secret-repo".to_string();
    let evil = redigest(evil);
    assert!(failure_case::case_export_json(&evil, &[]).is_err());

    // ... and by the scan record itself.
    let mut evil = base_case();
    evil.redaction.machine_path_free = false;
    assert!(redigest(evil).verify().is_err());
}

#[test]
fn malformed_evaluation_case_is_refused() {
    // Not JSON at all.
    assert!(LiteFailureCase::parse_json("{").is_err());
    // Unknown top-level field (deny_unknown_fields).
    let case = base_case();
    let mut value = serde_json::to_value(&case).expect("serializes");
    value["surprise"] = serde_json::json!(true);
    assert!(LiteFailureCase::parse_json(&value.to_string()).is_err());
    // Missing contentDigest.
    let json = serde_json::to_string_pretty(&case).expect("serializes");
    let mut value: serde_json::Value = serde_json::from_str(&json).expect("parses");
    value
        .as_object_mut()
        .expect("object")
        .remove("contentDigest");
    assert!(LiteFailureCase::parse_json(&value.to_string()).is_err());
    // Wrong namespace.
    let mut evil = case.clone();
    evil.namespace = "lite.fixtures.failure-case".to_string();
    assert!(redigest(evil).verify().is_err());
    // Wrong schema version.
    let mut evil = case.clone();
    evil.schema_version = "2.0.0".to_string();
    assert!(redigest(evil).verify().is_err());
    // Wrong verification scope.
    let mut evil = case.clone();
    evil.verification_scope = "proves benchmark standing".to_string();
    assert!(redigest(evil).verify().is_err());
    // Foundry boundary flip.
    let mut evil = case.clone();
    evil.external_evaluation_format.status = "supported".to_string();
    assert!(redigest(evil).verify().is_err());
}

// ---------------------------------------------------------------------------
// Provenance, dedup and fix linkage
// ---------------------------------------------------------------------------

#[test]
fn same_signature_incompatible_provenance_is_kept_separate() {
    // Two cases with the SAME failure identity but incompatible pins must
    // NOT merge. Simulate a forged case claiming the identity with a
    // different pinned revision (identity is derived from the chain and
    // never re-deriveable from the case, so the case simply claims it).
    let a = base_case();
    let mut b = a.clone();
    b.source.repository_revision = "f".repeat(40);
    b.case_id = a.case_id.clone(); // still claims the same identity-derived id
    let b = redigest(b);

    let report = failure_case::dedup_cases(&[a.clone(), b]).expect("dedup runs");
    assert_eq!(report.accounted(), 2);
    assert_eq!(report.groups.len(), 2, "incompatible pins never merge");
    assert!(
        !report.conflicts.is_empty(),
        "same identity with incompatible pins is reported as a conflict"
    );

    // A genuinely equivalent case DOES merge, retaining both occurrences'
    // provenance; nothing is dropped.
    let c1 = a.clone();
    let c2 = redigest(a.clone());
    let report = failure_case::dedup_cases(&[a.clone(), c1, c2]).expect("dedup runs");
    assert_eq!(report.accounted(), 3);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].occurrences.len(), 3);
    assert_eq!(
        report.groups[0].representative_case_digest,
        report.groups[0].occurrences[0].case_digest
    );
    // The representative is the lexicographically smallest digest (stable,
    // wall-clock-free).
    assert!(
        report.groups[0]
            .occurrences
            .windows(2)
            .all(|w| w[0].case_digest <= w[1].case_digest)
    );
}

#[test]
fn deduplication_never_loses_an_occurrence() {
    let case = base_case();
    let duplicates = vec![case.clone(), case.clone(), case.clone(), case.clone()];
    let report = failure_case::dedup_cases(&duplicates).expect("dedup runs");
    assert_eq!(report.accounted(), 4, "every input occurrence is preserved");
    assert_eq!(report.occurrences_total, 4);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].occurrences.len(), 4);
    for occurrence in &report.groups[0].occurrences {
        assert_eq!(occurrence.chain_evidence_digest, case.chain_evidence_digest);
        assert_eq!(occurrence.runner, case.provenance.runner);
    }

    // Empty input: clean, empty report.
    let report = failure_case::dedup_cases(&[]).expect("dedup of nothing is defined");
    assert_eq!(report.accounted(), 0);
    assert!(report.groups.is_empty());
}

#[test]
fn a_correction_that_fails_the_original_regression_yields_no_fix() {
    // The "fix" run applies the wrong repair again: the pinned validator
    // fails, so the chain seal refuses and no case can be built on it.
    let failure = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("the pinned failure seals");
    let failed_fix = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("the failing fix seals a (failed) envelope");
    assert_eq!(failed_fix.task_outcome, TaskOutcome::RepairFailed);
    assert_eq!(failed_fix.validator.exit_code, Some(101));
    let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR)
        .expect("the replay descriptor validates");
    assert!(
        LiteFailureReplayEvidence::sealed(descriptor, failure, failed_fix).is_err(),
        "a regression-failing correction must not seal as a fix"
    );
}

#[test]
fn incorrect_post_fix_source_digest_is_refused() {
    let mut chain = shared_chain().clone();
    chain.replay.post_run_content_digest = "f".repeat(64);
    assert!(chain.verify().is_err(), "post-fix digest drift is caught");

    let mut chain = shared_chain().clone();
    chain.replay.repair.file_after_digest = "1".repeat(64);
    assert!(chain.verify().is_err());

    let mut chain = shared_chain().clone();
    chain.original_failure.baseline_content_digest = "2".repeat(64);
    assert!(chain.verify().is_err());
}

#[test]
fn fabricated_validator_success_is_refused() {
    // Replay claims repaired but the validator exited 101: sealing refuses.
    let mut chain = shared_chain().clone();
    chain.replay.validator.exit_code = Some(101);
    assert!(chain.verify().is_err());

    // Replay claims repaired but the validator never executed.
    let mut chain = shared_chain().clone();
    chain.replay.validator.provenance =
        prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted;
    chain.replay.validator.exit_code = None;
    assert!(chain.verify().is_err());

    // Replay claims the wrong (but zero) exit code vs the pin.
    let mut chain = shared_chain().clone();
    chain.replay.validator.exit_code = Some(1);
    assert!(chain.verify().is_err());
}

#[test]
fn unknown_cost_or_usage_is_never_represented_as_zero() {
    let chain = shared_chain();
    for (label, run) in [
        ("failure", &chain.original_failure),
        ("replay", &chain.replay),
    ] {
        let accounting = &run.scenario_run.attempt_accounting;
        accounting.verify().expect("accounting verifies embedded");
        // The loopback fixture reports usage/pricing: KNOWN, never Unknown.
        match &accounting.usage {
            UsageAccounting::Known(usage) => {
                assert_eq!(usage.prompt_tokens, Some(7), "{label}");
                assert_eq!(usage.completion_tokens, Some(4), "{label}");
                assert_eq!(usage.total_tokens, Some(11), "{label}");
            }
            UsageAccounting::Unknown { reason } => {
                panic!("{label} usage must be KNOWN for the pinned loopback fixture: {reason}")
            }
        }
        match &accounting.cost {
            CostAccounting::Known(computation) => {
                assert!(
                    computation.minor_units < 1000,
                    "{label} known under the example price book"
                );
            }
            CostAccounting::Unknown { reason } => {
                panic!("{label} cost must be KNOWN under the pinned price book: {reason}")
            }
        }
    }

    // Zero-substituting an Unknown (or a known) anywhere in the embedded
    // accounting breaks the envelope digest: the chain no longer verifies,
    // so no case can be exported from it.
    let mut tampered = shared_chain().clone();
    tampered
        .original_failure
        .scenario_run
        .attempt_accounting
        .usage = UsageAccounting::Unknown {
        reason: "provider produced nothing".to_string(),
    };
    assert!(
        tampered.verify().is_err(),
        "accounting rewritten to Unknown (let alone zero) is detected by re-verification"
    );
    assert!(LiteFailureCase::sealed(&tampered, base_minimization(), &[]).is_err());

    // The exported case carries NO cost/usage fields at all, so there is
    // nothing in it that could read as a fabricated zero measurement.
    let case = base_case();
    let json = failure_case::case_export_json(&case, &[]).expect("export scans clean");
    for forbidden in [
        "minorUnits",
        "promptTokens",
        "completionTokens",
        "totalTokens",
        "passrate",
    ] {
        assert!(
            !json.contains(forbidden),
            "the exported case carries no cost/usage measurement: found {forbidden:?}"
        );
    }
}

#[test]
fn an_unrelated_successful_task_is_not_fix_evidence_for_this_failure() {
    let failure = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, wrong_repair_edits())
        .envelope
        .expect("the pinned failure seals");
    let docs_success = run_task_execution(&DOCS_SPEC, DOCS_DESCRIPTOR, docs_correct_repair_edits())
        .envelope
        .expect("the docs task repairs and seals");
    assert_eq!(docs_success.task_outcome, TaskOutcome::Repaired);

    // The unrelated success cannot stand in as the fix: the chain seal
    // refuses (scenario digest mismatch).
    let descriptor = FailureReplayDescriptor::parse_json(REPLAY_DESCRIPTOR)
        .expect("the replay descriptor validates");
    assert!(
        LiteFailureReplayEvidence::sealed(descriptor, failure, docs_success).is_err(),
        "an unrelated successful task is not a fix for this failure"
    );
}

#[test]
fn a_replayed_action_never_duplicates_an_already_completed_effect() {
    let case = base_case();

    // The exported case is a CLOSED artifact: no key in it can carry an
    // effect to apply, perform, dispatch or execute. Inspect every key in
    // the serialized JSON tree, not just the top level.
    let value: serde_json::Value =
        serde_json::from_str(&failure_case::case_export_json(&case, &[]).expect("export"))
            .expect("export parses");
    let mut keys = std::collections::BTreeSet::new();
    fn collect_keys(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    out.insert(key.to_string());
                    collect_keys(inner, out);
                }
            }
            serde_json::Value::Array(items) => {
                for inner in items {
                    collect_keys(inner, out);
                }
            }
            _ => {}
        }
    }
    collect_keys(&value, &mut keys);
    for forbidden in [
        "apply", "execute", "dispatch", "perform", "mutate", "invoke", "command", "argv",
    ] {
        // NB: `command`/`argv`-like keys do not exist; `validator.command`
        // exists but is a bare executable NAME bound as evidence, not an
        // action. Guard the exact action-shaped keys instead.
        if forbidden == "command" || forbidden == "argv" {
            continue;
        }
        assert!(
            !keys.iter().any(|key| key.eq_ignore_ascii_case(forbidden)),
            "the case export must not carry an action-shaped key: {forbidden:?}"
        );
    }

    // The fix runs against a FRESH staged source inside the chain; the
    // replay's effect landed exactly once, in its own worktree.
    let repaired = &shared_chain().replay;
    assert_eq!(
        repaired.repair.changed_files,
        vec!["src/lib.rs".to_string()]
    );
    assert!(repaired.repair.applied);

    // Deterministic re-export (verified in the positive test) combined with
    // dedup: recording the same failure N times yields ONE group of N
    // occurrences — never N separately-recorded "fixes".
    let clones = vec![case.clone(), case.clone(), case];
    let report = failure_case::dedup_cases(&clones).expect("dedup runs");
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].occurrences.len(), 3);
}

#[test]
fn the_exported_case_claims_no_foundry_and_no_benchmark() {
    let case = base_case();
    let json = failure_case::case_export_json(&case, &[])
        .expect("export scans clean")
        .to_lowercase();
    for forbidden in [
        "passrate",
        "ranking",
        "certif",
        "leaderboard",
        "benchmark",
        "comparability",
    ] {
        assert!(!json.contains(forbidden), "case claims {forbidden:?}");
    }
    assert_eq!(case.external_evaluation_format.status, "unsupported");
    assert_eq!(
        case.external_evaluation_format.contract,
        "foundry.workflow-evaluation (unpublished)"
    );
    // Provenance is pinned fixture-only with an injected clock.
    assert!(case.provenance.fixture_only && case.provenance.injected_clock);
    // The failure class is the governed validation failure, never a model signal.
    assert_eq!(
        case.failure_class,
        FailureReplayClass::GovernedValidationFailure
    );
    // The replay repair is labelled scripted, never model skill.
    assert_eq!(
        shared_chain().replay.repair.origin,
        TaskRepairOrigin::ScriptedFixture
    );
}
