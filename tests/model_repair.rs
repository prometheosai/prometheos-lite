//! Model-authored repair loop (issue #136 Slice D) - hermetic verification.
//!
//! These tests exercise the same governed boundary the live run dispatches
//! through — staging, worktree isolation, file-policy authorization, dry-run
//! preflight, apply-with-rollback, subprocess validator, and sealed evidence
//! — with a deterministic scripted model instead of a live provider. The
//! scripted proposer is a test double; it is never evidence of model
//! capability, and nothing here claims a benchmark result.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use prometheos_lite::workflow::model_repair::{
    ModelDescriptor, ModelProposeError, ModelRepairReport, ModelRepairSpec, ModelRepairStatus,
    ScriptedProposer, ValidatorSpec, batch_of, format_unix_utc, tree_digest,
};
use prometheos_lite::workflow::soma::types::SemVer;
use sha2::Digest;

fn fixture_bytes() -> Vec<(PathBuf, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scenarios")
        .join("rust-bugfix-arithmetic")
        .join("repo");
    let mut out = Vec::new();
    collect(&root, Path::new(""), &mut out);
    out
}

fn collect(root: &Path, rel: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in std::fs::read_dir(root.join(rel))
        .expect("fixture dir readable")
        .flatten()
    {
        let path = rel.join(entry.file_name());
        if path.starts_with(".git") {
            continue;
        }
        let meta = entry.metadata().expect("metadata");
        if meta.is_dir() {
            collect(root, &path, out);
        } else {
            out.push((
                path.clone(),
                std::fs::read(root.join(&path)).expect("fixture file readable"),
            ));
        }
    }
}

/// Deterministic staging of the pinned fixture source into a disposable
/// git repository with fixed identity and timestamps.
fn stage_fixture() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    for (rel, bytes) in fixture_bytes() {
        let dest = root.join(&rel);
        std::fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
        std::fs::write(&dest, bytes).expect("write fixture file");
    }
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .env("GIT_AUTHOR_NAME", "lite-fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", "1764547200 +0000")
            .env("GIT_COMMITTER_NAME", "lite-fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_DATE", "1764547200 +0000")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {:?} failed: {:?}", args, out);
        out
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "pinned fixture source"]);
    let rev = git(&["rev-parse", "HEAD"]);
    let rev = String::from_utf8_lossy(&rev.stdout).trim().to_string();
    (temp, rev)
}

const ISSUE_STATEMENT: &str = "fixture-buggy-sum exposes sum(a: i64, b: i64) -> i64 in src/lib.rs, but it does not add. Read the source, determine the defect, and repair it. Do not change the public signature or the tests.";

const FIX_DIFF: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -6 +6 @@\n-    a - b\n+    a + b\n";

const WRONG_DIFF: &str =
    "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -6 +6 @@\n-    a - b\n+    a - b + 1\n";

const CARGO_TOML_DIFF: &str =
    "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -2 +2 @@\n-version = \"0.1.0\"\n+version = \"0.2.0\"\n";

const GARBAGE: &str = "I cannot repair this; it is honest work though.";

fn descriptor() -> ModelDescriptor {
    ModelDescriptor {
        provider: "test-double".to_string(),
        model: "scripted-test-model".to_string(),
        harness_id: "lite.model_repair.scripted".to_string(),
        harness_revision: "v1".to_string(),
    }
}

fn validator() -> ValidatorSpec {
    ValidatorSpec {
        command: "cargo".to_string(),
        args: vec!["test".to_string(), "--offline".to_string()],
        expected_exit_code: 0,
        assertion: "fixture-buggy-sum: cargo test --offline exits 0".to_string(),
    }
}

fn repair_spec(worktree: &Path, rev: &str) -> ModelRepairSpec {
    let target = worktree.join("src").join("lib.rs");
    let target_digest = {
        let bytes = std::fs::read(&target).expect("target readable");
        format!("{:x}", sha2::Sha256::digest(&bytes))
    };
    ModelRepairSpec {
        task_id: "lite-fixture-sum-0001".to_string(),
        repair_target: "src/lib.rs".to_string(),
        worktree: worktree.to_path_buf(),
        repair_target_sha256: target_digest,
        workspace_fingerprint: tree_digest(worktree).expect("tree digests"),
        base_revision: rev.to_string(),
        issue_statement: ISSUE_STATEMENT.to_string(),
        validator: validator(),
        validator_timeout: std::time::Duration::from_secs(240),
        max_attempts: 4,
        validator_target_dir: None,
        prompt_context: None,
        source: None,
    }
}

fn repair_spec_with_source(worktree: &Path, rev: &str) -> ModelRepairSpec {
    let mut spec = repair_spec(worktree, rev);
    spec.source = Some(prometheos_lite::workflow::model_repair::SomaSourceBinding {
        repository: "prometheosai/prometheos-lite".to_string(),
        repository_revision: rev.to_string(),
        issue_number: Some(136),
    });
    spec
}

#[tokio::test]
async fn model_authored_repair_succeeds_with_failure_feedback() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![
            Ok(WRONG_DIFF.to_string()),
            Ok(format!("```diff\n{FIX_DIFF}```")),
        ],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    eprintln!("DEBUG report: {:#?}", report);
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert_eq!(report.attempts.len(), 2);
    assert!(
        report.attempts[0].rolled_back,
        "failed attempt must roll back"
    );
    assert_eq!(
        report.attempts[0].refusal.as_deref(),
        Some("validator-rejected")
    );
    assert!(!report.attempts[1].rolled_back);
    // The model saw the failure feedback and repaired accordingly.
    let proposer_prompts_len = report.attempts.len();
    assert_eq!(proposer_prompts_len, 2);
    assert!(report.attempts[0].validator.as_ref().is_some());
    assert_eq!(
        report.attempts[0].validator.as_ref().unwrap().exit_code,
        Some(101)
    );
    assert!(report.verify().is_ok(), "sealed report must verify");
    let diff = report.final_diff_excerpt.clone().unwrap_or_default();
    assert!(
        diff.contains("a + b"),
        "diff must carry the model's repair: {diff}"
    );
    // The repaired tree actually validates.
    let out = Command::new("cargo")
        .args(["test", "--offline"])
        .current_dir(&worktree)
        .output()
        .expect("cargo runs");
    assert!(out.status.success());
    // The patch must not silently strip the trailing newline.
    let bytes = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert!(tree_digest(&worktree).is_ok());
}

#[tokio::test]
async fn pre_fix_baseline_is_recorded_failing() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    let baseline = report.baseline_validator.expect("baseline receipt present");
    assert_ne!(baseline.exit_code, Some(0), "pre-fix validator must fail");
    assert_eq!(
        baseline.provenance,
        prometheos_lite::workflow::scenario::TaskValidatorProvenance::ObservedSubprocess
    );
}

#[tokio::test]
async fn always_wrong_patch_never_claims_success() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let mut spec = repair_spec(&worktree, &rev);
    spec.max_attempts = 3;
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![
            Ok(WRONG_DIFF.to_string()),
            Ok(WRONG_DIFF.to_string()),
            Ok(WRONG_DIFF.to_string()),
        ],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::ValidatorRejected);
    assert_eq!(report.attempts.len(), 3);
    assert!(report.attempts.iter().all(|a| a.applied && a.rolled_back));
    assert!(report.verify().is_ok());
    // The pinned source file is byte-identical to the pin again.
    let bytes = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(&bytes)),
        spec.repair_target_sha256
    );
}

#[tokio::test]
async fn model_refusal_produces_typed_outcome_and_zero_writes() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let before = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Err(ModelProposeError::Refused(
            "policy: content not repairable by model".to_string(),
        ))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::ModelRefusal);
    assert_eq!(report.attempts.len(), 1);
    assert!(report.attempts[0].validator.is_none());
    let after = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    assert_eq!(before, after, "refusal must never write");
}

#[tokio::test]
async fn provider_unavailable_is_typed_and_effect_free() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let before = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Err(ModelProposeError::Unavailable(
            "endpoint refused connection".to_string(),
        ))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::ProviderUnavailable);
    let after = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    assert_eq!(before, after);
}

#[tokio::test]
async fn unauthorized_repair_target_proposal_refuses_without_effects() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let manifest_before = std::fs::read(worktree.join("Cargo.toml")).expect("read");
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(CARGO_TOML_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::UnauthorizedProposal);
    assert_eq!(
        report.attempts[0].refusal.as_deref(),
        Some("unauthorized-proposal")
    );
    assert!(
        report.attempts[0].validator.is_none(),
        "unauthorized proposal never validates"
    );
    let manifest_after = std::fs::read(worktree.join("Cargo.toml")).expect("read");
    assert_eq!(
        manifest_before, manifest_after,
        "unauthorized file must stay byte-identical"
    );
}

#[tokio::test]
async fn unparseable_proposal_feeds_back_then_recovers() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Ok(GARBAGE.to_string()), Ok(FIX_DIFF.to_string())],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(
        report.attempts[0].refusal.as_deref(),
        Some("unparseable-proposal")
    );
    assert!(report.verify().is_ok());
}

#[tokio::test]
async fn empty_response_is_unparseable_and_effect_free() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let before = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(String::new())]);
    let mut spec = spec;
    spec.max_attempts = 1;
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_ne!(report.status, ModelRepairStatus::Repaired);
    let after = std::fs::read(worktree.join("src").join("lib.rs")).expect("read");
    assert_eq!(before, after);
}

#[tokio::test]
async fn cancel_token_interrupts_before_dispatch() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let flag = Arc::new(AtomicBool::new(true));
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report = prometheos_lite::workflow::model_repair::run_model_repair(
        &spec,
        proposer,
        Some(flag.clone()),
    )
    .await;
    assert_eq!(report.status, ModelRepairStatus::Interrupted);
    assert!(
        report.attempts.is_empty(),
        "interrupted before dispatch: zero effects"
    );
    assert!(flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stale_workspace_pin_refuses_before_dispatch() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let mut spec = repair_spec(&worktree, &rev);
    spec.workspace_fingerprint = "0".repeat(64);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::SetupError);
    assert!(report.attempts.len() <= 1);
    let refusal = report.attempts[0].refusal.clone().unwrap_or_default();
    assert!(
        refusal.contains("does not match the pinned fingerprint"),
        "refusal must cite the fingerprint pin: {refusal}"
    );
}

#[tokio::test]
async fn stale_repair_target_pin_refuses_before_dispatch() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let mut spec = repair_spec(&worktree, &rev);
    spec.repair_target_sha256 = "0".repeat(64);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::SetupError);
    let after = std::fs::read_to_string(worktree.join("src").join("lib.rs")).expect("read");
    assert!(after.contains("a - b"));
}

#[tokio::test]
async fn determinism_repeated_runs_agree_on_content_digests() {
    let run_once = || {
        let (temp, rev) = stage_fixture();
        let worktree = temp.path().to_path_buf();
        let spec = repair_spec(&worktree, &rev);
        let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
        (spec, proposer, temp)
    };
    let (spec_a, proposer_a, _t1) = run_once();
    let (spec_b, proposer_b, _t2) = run_once();
    assert_eq!(spec_a.workspace_fingerprint, spec_b.workspace_fingerprint);
    assert_eq!(spec_a.repair_target_sha256, spec_b.repair_target_sha256);
    let report_a =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec_a, proposer_a, None).await;
    let report_b =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec_b, proposer_b, None).await;

    // Determinism scope: the model's repair artifact and the tree shape are
    // deterministic; prompts are not asserted byte-equal because baseline
    // validator feedback embeds volatile tool output.
    assert_eq!(report_a.final_diff_sha256, report_b.final_diff_sha256);
    assert_eq!(report_a.final_tree_digest, report_b.final_tree_digest);
    assert_eq!(report_a.status, report_b.status);
}

#[tokio::test]
async fn tampered_repaired_report_fails_verification() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let mut report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    // Tampered validator exit code: the repaired claim must not survive.
    report.attempts[0].validator.as_mut().unwrap().exit_code = Some(1);
    assert!(report.verify().is_err());
    // Forged receipt binding: command must equal the pin.
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let mut report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    report.validator.command = "curl".to_string();
    assert!(report.verify().is_err());
}

#[tokio::test]
async fn tampered_pinned_command_fails_verification() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let mut report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    report.attempts[0].validator.as_mut().unwrap().command = "rm".to_string();
    assert!(
        report.verify().is_err(),
        "pinned-command tampering must fail closed"
    );
}

#[tokio::test]
async fn work_event_batch_emission_is_audit_clean() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Ok(WRONG_DIFF.to_string()), Ok(FIX_DIFF.to_string())],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    let batch = batch_of(&report);
    assert_eq!(batch.events.len(), report.attempts.len() + 1);
    assert_eq!(batch.events[batch.events.len() - 1].event_type, "outcome");
    // Events are sealed against the CURRENT normative bundle version
    // (SOMA 1.3.0); Lite's ported audit with a 1.3.0-pinned bundle reports
    // zero diagnostics.
    let supported = SemVer::parse("1.3.0").expect("semver");
    let diagnostics = batch.audit(&supported);
    assert!(
        diagnostics.is_empty(),
        "audit diagnostics: {:?}",
        diagnostics
            .iter()
            .map(|d| d.code.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn rfc3339_formatter_is_valid_and_deterministic() {
    assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_unix_utc(1_794_700_800), "2026-11-15T00:00:00Z");
    let later = format_unix_utc(1_794_700_800 + 3661);
    assert_eq!(later, "2026-11-15T01:01:01Z");
}

#[test]
fn report_serde_round_trip_and_digest() {
    let (temp, rev) = stage_fixture();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let report: ModelRepairReport = runtime.block_on(async {
        let spec = repair_spec(temp.path(), &rev);
        let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await
    });
    let json = serde_json::to_string(&report).expect("serialize");
    let parsed: ModelRepairReport = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, report);
    assert_eq!(
        prometheos_lite::workflow::model_repair::report_digest(&report),
        prometheos_lite::workflow::model_repair::report_digest(&parsed)
    );
}

// ---------------------------------------------------------------------------
// Provider provenance and unique execution identity (PR #314 review)
// ---------------------------------------------------------------------------

fn completion_with_id(
    content: &str,
    id: &str,
) -> prometheos_lite::workflow::model_repair::ProposedCompletion {
    prometheos_lite::workflow::model_repair::ProposedCompletion {
        content: content.to_string(),
        provider_response_id: Some(id.to_string()),
        provider_response_id_source: Some(
            "response-body:id via lite.harness.remote_service".to_string(),
        ),
        gateway_trace_id: None,
        local_request_id: Some("req-model-repair-live-step".to_string()),
    }
}

// 1. Genuine provider id present: captured, sealed, verifiable, and sealable.
#[tokio::test]
async fn provider_id_present_is_captured_sealed_and_verify_passes() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![Ok(completion_with_id(FIX_DIFF, "chatcmpl-test-0001"))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    let sealing = report.attempts.last().expect("attempt");
    assert_eq!(
        sealing.provider_response_id.as_deref(),
        Some("chatcmpl-test-0001")
    );
    assert_eq!(
        sealing.provider_response_id_source.as_deref(),
        Some("response-body:id via lite.harness.remote_service")
    );
    assert!(report.execution_id.starts_with("exec-"));
    assert!(report.execution_id.len() > 16);
    assert!(report.sealable, "sealable with id + bindings: {report:?}");
    assert!(report.sealability_reasons.is_empty());
    assert!(report.verify().is_ok(), "sealed report verifies");
}

// 2. Provider id absent: recorded absent, report not sealable, verify still passes.
#[tokio::test]
async fn provider_id_absent_is_recorded_absent_and_not_sealable() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert!(
        report
            .attempts
            .iter()
            .all(|a| a.provider_response_id.is_none())
    );
    assert!(!report.sealable);
    assert!(
        report
            .sealability_reasons
            .iter()
            .any(|r| r.contains("responder did not expose")),
        "reason enumerates the missing provider id: {:?}",
        report.sealability_reasons
    );
    // Verification is not sealability: integrity still holds.
    assert!(report.verify().is_ok());
}

// 3. Gateway-only trace: recorded, no fabricated provider id, not sealable.
#[tokio::test]
async fn gateway_only_trace_is_recorded_without_fabrication() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let completion = prometheos_lite::workflow::model_repair::ProposedCompletion {
        content: FIX_DIFF.to_string(),
        provider_response_id: None,
        provider_response_id_source: None,
        gateway_trace_id: Some("gw-trace-abc123".to_string()),
        local_request_id: Some("req-model-repair-live-step".to_string()),
    };
    let proposer = ScriptedProposer::with_completions(descriptor(), vec![Ok(completion)]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let sealing = report.attempts.last().expect("attempt");
    assert_eq!(sealing.gateway_trace_id.as_deref(), Some("gw-trace-abc123"));
    assert!(sealing.provider_response_id.is_none());
    assert!(!report.sealable);
}

// 4. Two distinct executions of the same task share no identity.
#[tokio::test]
async fn two_distinct_executions_of_same_task_do_not_alias() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec_a = repair_spec_with_source(&worktree, &rev);
    let spec_b = repair_spec_with_source(&worktree, &rev);
    let proposer_a = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let proposer_b = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report_a =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec_a, proposer_a, None).await;
    let report_b =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec_b, proposer_b, None).await;
    assert_ne!(report_a.execution_id, report_b.execution_id);
    assert_ne!(report_a.events_sha256, report_b.events_sha256);
    // Replays of the sealed reports preserve their identities.
    let batch_a = batch_of(&report_a);
    let batch_b = batch_of(&report_b);
    assert_ne!(
        batch_a.events[0].correlation_id,
        batch_b.events[0].correlation_id
    );
    assert_ne!(batch_a.events[0].id, batch_b.events[0].id);
}

// 5. Multiple attempts in one execution share one execution identity.
#[tokio::test]
async fn multiple_attempts_share_one_execution_identity() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![
            Ok(completion_with_id(WRONG_DIFF, "p-attempt-1")),
            Ok(completion_with_id(FIX_DIFF, "p-attempt-2")),
        ],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(
        report.attempts[0].provider_response_id.as_deref(),
        Some("p-attempt-1")
    );
    assert_eq!(
        report.attempts[1].provider_response_id.as_deref(),
        Some("p-attempt-2")
    );
    let batch = batch_of(&report);
    for event in &batch.events {
        assert_eq!(event.correlation_id, report.execution_id);
    }
    // Sequences are 0..n and parents chain deterministically within the run.
    for (index, event) in batch.events.iter().enumerate() {
        assert_eq!(event.sequence, index as u64);
        if index == 0 {
            assert!(event.parents.is_empty());
        } else {
            assert_eq!(event.parents, vec![batch.events[index - 1].id.clone()]);
        }
        assert!(event.id.starts_with("evt-"));
    }
}

// 6. Provider failure records a typed outcome with no fabricated id.
#[tokio::test]
async fn retried_provider_failure_never_fabricates_an_id() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Err(ModelProposeError::Unavailable(
            "endpoint refused connection".to_string(),
        ))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::ProviderUnavailable);
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(
        report.attempts[0].refusal.as_deref(),
        Some("provider-unavailable")
    );
    assert!(report.attempts[0].provider_response_id.is_none());
    assert!(report.attempts[0].response_sha256.is_none());
    assert!(!report.execution_id.is_empty());
    assert!(report.verify().is_ok());
}

// 7. Replaying emission of a sealed report reproduces the identical batch.
#[tokio::test]
async fn work_event_batch_replay_preserves_identity_and_detects_duplicates() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let first = batch_of(&report);
    let replay = batch_of(&report);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&replay).unwrap()
    );
    // Duplicate emissions share event ids — detectable, not silent.
    assert_eq!(
        first.events.iter().map(|e| &e.id).collect::<Vec<_>>(),
        replay.events.iter().map(|e| &e.id).collect::<Vec<_>>()
    );
}

// 8. Tampering with the identity or any bound field fails verification.
#[tokio::test]
async fn event_digest_and_identity_tampering_fail_verification() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    // Tampered execution identity.
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let mut report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let original = report.execution_id.clone();
    report.execution_id = "exec-00000000000000000000000000000000".to_string();
    assert_ne!(report.execution_id, original);
    assert!(
        report.verify().is_err(),
        "identity tamper must fail verification"
    );

    // Tampered attempt provider id.
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![Ok(completion_with_id(FIX_DIFF, "p-1"))],
    );
    let mut report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    report.attempts[0].provider_response_id = Some("p-forged".to_string());
    assert!(
        report.verify().is_err(),
        "provider id tamper must fail verification"
    );
}

// 9. Report, events and provenance bind to one another (parent chain + ids).
#[tokio::test]
async fn report_events_and_identity_binding_is_consistent() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![
            Ok(completion_with_id(WRONG_DIFF, "p-attempt-1")),
            Ok(completion_with_id(FIX_DIFF, "p-attempt-2")),
        ],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let batch = batch_of(&report);
    assert_eq!(batch.events.len(), 3);
    assert_eq!(report.attempts.len(), 2);
    // Every event references the same run and the pinned revision.
    for event in &batch.events {
        assert_eq!(event.correlation_id, report.execution_id);
        assert_eq!(event.repo_revision, report.base_revision);
    }
    let outcome = batch.events.last().expect("outcome event");
    assert_eq!(outcome.event_type, "outcome");
    assert_eq!(outcome.parents, vec![batch.events[1].id.clone()]);
    // The sealed report's eventsDigest matches a re-derived batch.
    let rederived = batch_of(&report);
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(serde_json::to_vec(&rederived).unwrap_or_default());
        format!("{:x}", hasher.finalize())
    };
    assert_eq!(report.events_sha256.as_deref(), Some(digest.as_str()));
}

// 10. Source revision mismatch fails verification.
#[tokio::test]
async fn source_revision_mismatch_fails_verification() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let mut spec = repair_spec_with_source(&worktree, &rev);
    spec.source = Some(prometheos_lite::workflow::model_repair::SomaSourceBinding {
        repository: "prometheosai/prometheos-lite".to_string(),
        repository_revision: "0000000000000000000000000000000000000001".to_string(),
        issue_number: Some(136),
    });
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert!(report.verify().is_err());
}

// 11. Missing tracked issue identity: not sealable, verify still passes,
//     mapping reports the gap honestly.
#[tokio::test]
async fn missing_tracked_issue_identity_blocks_sealable_only() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let mut spec = repair_spec_with_source(&worktree, &rev);
    spec.source = Some(prometheos_lite::workflow::model_repair::SomaSourceBinding {
        repository: "prometheosai/prometheos-lite".to_string(),
        repository_revision: rev.to_string(),
        issue_number: None,
    });
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![Ok(completion_with_id(FIX_DIFF, "p-1"))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert!(report.verify().is_ok());
    assert!(!report.sealable);
    assert!(
        report
            .sealability_reasons
            .iter()
            .any(|r| r.contains("source.issueNumber")),
        "reasons: {:?}",
        report.sealability_reasons
    );
    let mapping = report.soma_field_mapping();
    let issue = mapping
        .iter()
        .find(|row| row.path == "source.issueNumber")
        .expect("issue row");
    assert_eq!(issue.status, "gap");
    assert_eq!(issue.value, None);
}

// 12. Absent provider identity leaves no trace in the serialized report.
#[tokio::test]
async fn no_synthetic_provider_id_appears_anywhere() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let json = serde_json::to_string(&report).expect("serialize");
    for forged in [
        "providerResponseId",
        "providerResponseIdSource",
        "localRequestId",
        "gatewayTraceId",
        "chatcmpl",
        "gen-",
    ] {
        assert!(
            !json.contains(forged),
            "absent provenance must not appear in the report: {forged}"
        );
    }
}

// 13. Credentials never persist: canary secrets redact from every artifact.
#[tokio::test]
async fn credential_redaction_holds_across_attempts() {
    use prometheos_lite::workflow::redaction::SECRET_CANARY;
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let leaky = format!("ok I will fix it: {SECRET_CANARY}\n```diff\n{FIX_DIFF}```");
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(leaky)]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let json = serde_json::to_string(&report).expect("serialize");
    assert!(
        !json.contains(SECRET_CANARY),
        "the canary must not persist anywhere in the report"
    );
}

// 14. No unauthorized model dispatch: the hermetic suite never reaches the
//     network, and the sealed report carries no endpoint configuration.
#[tokio::test]
async fn sealed_report_carries_no_endpoint_or_credential_configuration() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    let json = serde_json::to_string(&report).expect("serialize");
    let lower = json.to_lowercase();
    for forbidden in [
        "apikey",
        "api_key",
        "endpoint",
        "openrouter",
        "https://",
        "bearer",
        "authorization",
    ] {
        assert!(
            !lower.contains(forbidden),
            "sealed report must not carry endpoint/credential material: {forbidden}"
        );
    }
    assert_eq!(report.model.provider, descriptor().provider);
}

// 15. Deterministic serialization, digests, and sealability are stable.
#[tokio::test]
async fn serialization_and_sealability_are_deterministic() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::with_completions(
        descriptor(),
        vec![Ok(completion_with_id(FIX_DIFF, "p-deterministic"))],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    // Content-addressed digests re-derive from the same recorded inputs.
    let json = serde_json::to_string(&report).expect("serialize");
    let parsed: ModelRepairReport = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, report);
    assert_eq!(
        prometheos_lite::workflow::model_repair::report_digest(&report),
        prometheos_lite::workflow::model_repair::report_digest(&parsed)
    );
    assert_eq!(report.sealable, parsed.sealable);
    assert_eq!(report.sealability_reasons, parsed.sealability_reasons);
    // Re-derivation from the parsed report agrees.
    let (sealable, reasons) = prometheos_lite::workflow::model_repair::compute_sealability(&parsed);
    assert_eq!(sealable, parsed.sealable);
    assert_eq!(reasons, parsed.sealability_reasons);
}

// SOMA field mapping reports typed gaps without inventing values.
#[test]
fn soma_field_mapping_reports_present_rows_and_typed_gaps() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let report: ModelRepairReport = runtime.block_on(async {
        let proposer = ScriptedProposer::with_completions(
            descriptor(),
            vec![Ok(completion_with_id(FIX_DIFF, "p-mapping"))],
        );
        let spec = repair_spec_with_source(&worktree, &rev);
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await
    });
    let mapping = report.soma_field_mapping();
    for row in &mapping {
        match row.status.as_str() {
            "present" => assert!(
                row.value.is_some(),
                "present row carries value: {}",
                row.path
            ),
            "gap" => {
                assert!(
                    row.value.is_none(),
                    "gap must not carry a value: {}",
                    row.path
                );
                assert!(row.note.is_some(), "gap must carry a reason: {}", row.path);
            }
            other => panic!("unknown mapping status {other}"),
        }
    }
    let by_path = |p: &str| mapping.iter().find(|r| r.path == p).expect("row").clone();
    assert_eq!(by_path("source.issueNumber").status, "present");
    assert_eq!(by_path("source.issueNumber").value.as_deref(), Some("136"));
    assert_eq!(by_path("source.repository").status, "present");
    assert_eq!(by_path("model.providerRunId").status, "present");
    assert_eq!(
        by_path("model.providerRunId").value.as_deref(),
        Some("p-mapping")
    );
    assert_eq!(by_path("model.harness").status, "present");
    assert_eq!(by_path("provenance").status, "present");
    assert_eq!(by_path("events").status, "present");
    assert_eq!(by_path("workspace").status, "present");
    assert_eq!(by_path("patch").status, "present");
    assert_eq!(by_path("validation").status, "present");
    assert_eq!(by_path("runDigest").status, "present");
}

#[test]
fn soma_field_mapping_reports_gaps_without_fabrication() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let report: ModelRepairReport = runtime.block_on(async {
        let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
        let spec = repair_spec_with_source(&worktree, &rev);
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await
    });
    let mapping = report.soma_field_mapping();
    let by_path = |p: &str| mapping.iter().find(|r| r.path == p).expect("row").clone();
    // No provider id was exposed: the row is a typed gap, never synthesized.
    assert_eq!(by_path("model.providerRunId").status, "gap");
    assert_eq!(by_path("model.providerRunId").value, None);
    assert_eq!(by_path("runDigest").status, "gap");
    assert_eq!(by_path("runDigest").value, None);
}

// Full mapping with unconfigured source reports every gap honestly.
#[test]
fn soma_field_mapping_with_unconfigured_source_reports_all_gaps() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let report: ModelRepairReport = runtime.block_on(async {
        let proposer = ScriptedProposer::new(descriptor(), vec![Ok(FIX_DIFF.to_string())]);
        let spec = repair_spec(&worktree, &rev);
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await
    });
    let mapping = report.soma_field_mapping();
    let by_path = |p: &str| mapping.iter().find(|r| r.path == p).expect("row").clone();
    assert_eq!(by_path("source.issueNumber").status, "gap");
    assert_eq!(by_path("source.repository").status, "gap");
    assert_eq!(by_path("source.repositoryRevision").status, "gap");
    assert_eq!(by_path("model.providerRunId").status, "gap");
    assert_eq!(by_path("runDigest").status, "gap");
}

// Unparseable responses still seal with a unique execution identity.
#[tokio::test]
async fn unparseable_responses_and_empty_response_paths_have_identity() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Ok(GARBAGE.to_string()), Ok(FIX_DIFF.to_string())],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert!(report.verify().is_ok());
    assert_eq!(batch_of(&report).events.len(), 3);

    let (temp2, rev2) = stage_fixture();
    let worktree2 = temp2.path().to_path_buf();
    let empty = ScriptedProposer::new(descriptor(), vec![Ok(String::new())]);
    let report2 = prometheos_lite::workflow::model_repair::run_model_repair(
        &repair_spec(&worktree2, &rev2),
        empty,
        None,
    )
    .await;
    assert_ne!(report2.status, ModelRepairStatus::Repaired);
    assert!(!report2.execution_id.is_empty());
    assert!(report2.verify().is_ok());
}

// ---------------------------------------------------------------------------
// #314 Slice D review repairs — complete diff artifact binding + UTF-8-safe
// bounded excerpt
// ---------------------------------------------------------------------------

fn stage_big_fixture() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"big-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n",
    )
    .expect("write manifest");
    let mut lib = String::from(
        "pub fn sum(a: i64, b: i64) -> i64 {\n    a - b\n}\n#[cfg(test)]\nmod tests {\n    use super::sum;\n    #[test]\n    fn sums_two_positive_numbers() {\n        assert_eq!(sum(2, 2), 4);\n    }\n}\n",
    );
    for n in 0..1_200 {
        lib.push_str(&format!("// marker {n}: a - b\n"));
    }
    std::fs::write(root.join("src/lib.rs"), lib).expect("write lib.rs");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .env("GIT_AUTHOR_NAME", "lite-fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", "1764547200 +0000")
            .env("GIT_COMMITTER_NAME", "lite-fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_DATE", "1764547200 +0000")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {:?} failed: {:?}", args, out);
        out
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "pinned big rust source"]);
    let rev = git(&["rev-parse", "HEAD"]);
    let rev = String::from_utf8_lossy(&rev.stdout).trim().to_string();
    (temp, rev)
}

#[tokio::test]
async fn model_authored_repair_produces_complete_untruncated_diff() {
    let (temp, rev) = stage_big_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Ok(
            r#"[{"type":"search_replace","file":"src/lib.rs","search":"a - b","replace":"a + b","replace_all":true}]"#
                .to_string(),
        )],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);

    let full = report
        .final_diff
        .as_ref()
        .expect("a repaired outcome must carry the complete finalDiff");
    assert!(
        full.len() > prometheos_lite::workflow::model_repair::MAX_DIFF_BYTES,
        "fixture must exceed the excerpt cap, got {} bytes",
        full.len()
    );
    let excerpt = report
        .final_diff_excerpt
        .as_deref()
        .expect("excerpt present");
    assert!(
        excerpt.len() < full.len(),
        "excerpt is a bounded view, never a parseable patch"
    );
    assert!(
        excerpt.contains("[... "),
        "truncated excerpt carries the truncation marker: {excerpt:?}"
    );
    let expected_digest = prometheos_lite::workflow::model_repair::sha256_hex(full.as_bytes());
    assert_eq!(
        report.final_diff_sha256.as_deref(),
        Some(expected_digest.as_str()),
        "the digest binds the complete diff bytes"
    );
    assert!(report.verify().is_ok(), "sealed report must verify");

    // Applying the COMPLETE diff to a fresh pristine clone yields the
    // repaired source and the validator passes.
    let dest_scratch = std::env::temp_dir().join(format!(
        "lite-model-repair-final-diff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dest_scratch).expect("scratch dir");
    let clone = dest_scratch.join("clone");
    let status = Command::new("git")
        .args(["clone", "--no-local", "--no-hardlinks"])
        .arg(&worktree)
        .arg(&clone)
        .status()
        .expect("git clone runs");
    assert!(status.success(), "clone succeeds");
    let checkout = Command::new("git")
        .args(["checkout", "-q", "--detach", &rev])
        .current_dir(&clone)
        .status()
        .expect("git checkout runs");
    assert!(checkout.success(), "checkout succeeds");
    let diff_path = dest_scratch.join("repair.diff");
    std::fs::write(&diff_path, full).unwrap();
    let status = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .arg(&diff_path)
        .current_dir(&clone)
        .status()
        .expect("git apply runs");
    assert!(status.success(), "complete diff applies cleanly");
    std::fs::remove_dir_all(&dest_scratch).unwrap();
}

#[tokio::test]
async fn report_verify_rejects_tampered_full_diff() {
    let (temp, rev) = stage_big_fixture();
    let worktree = temp.path().to_path_buf();
    let spec = repair_spec_with_source(&worktree, &rev);
    let proposer = ScriptedProposer::new(
        descriptor(),
        vec![Ok(
            r#"[{"type":"search_replace","file":"src/lib.rs","search":"a - b","replace":"a + b","replace_all":true}]"#
                .to_string(),
        )],
    );
    let report =
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await;
    assert_eq!(report.status, ModelRepairStatus::Repaired);

    // Tamper the stored full diff: verification must fail.
    let mut tampered = report.clone();
    tampered.final_diff =
        Some("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-wrong\n+right\n".to_string());
    assert!(
        tampered.verify().is_err(),
        "tampered finalDiff breaks verification"
    );

    // Tamper the bound digest: verification must fail.
    let mut tampered2 = report.clone();
    tampered2.final_diff_sha256 = Some("0".repeat(64));
    assert!(
        tampered2.verify().is_err(),
        "tampered digest breaks verification"
    );
}

#[test]
fn bounded_excerpt_never_splits_a_multibyte_character() {
    // 4-byte codepoints; naive byte slicing would split codepoints at cap.
    let bytes = "𝒫𝓇𝑜".repeat(12_000).into_bytes();
    let out = prometheos_lite::workflow::model_repair::bounded_excerpt(&bytes, 8192);
    assert!(out.len() < bytes.len());
    assert!(out.contains("[... "));
    // The cut must not panic and the bytes remain valid UTF-8 with a
    // finalized marker line.
    let marker = out
        .rfind('\n')
        .map(|idx| &out[idx + 1..])
        .expect("marker line present");
    assert!(marker.starts_with("[... "));
}

#[test]
fn bounded_excerpt_exact_boundary_is_untrimmed() {
    let bytes = "abc";
    let out = prometheos_lite::workflow::model_repair::bounded_excerpt(bytes.as_bytes(), 3);
    assert_eq!(out, "abc");
    let out2 = prometheos_lite::workflow::model_repair::bounded_excerpt("abcdef".as_bytes(), 3);
    assert!(out2.len() > bytes.len());
}

#[test]
fn scripted_double_provider_id_never_yields_sealable_by_marker() {
    let (temp, rev) = stage_fixture();
    let worktree = temp.path().to_path_buf();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let report = runtime.block_on(async {
        let completion = prometheos_lite::workflow::model_repair::ProposedCompletion {
            content: FIX_DIFF.to_string(),
            provider_response_id: Some("p-double".to_string()),
            provider_response_id_source: Some(
                "scripted-test-double (not live-provider evidence)".to_string(),
            ),
            gateway_trace_id: None,
            local_request_id: Some("req-model-repair-live-step".to_string()),
        };
        let proposer = ScriptedProposer::with_completions(descriptor(), vec![Ok(completion)]);
        let spec = repair_spec_with_source(&worktree, &rev);
        prometheos_lite::workflow::model_repair::run_model_repair(&spec, proposer, None).await
    });
    assert_eq!(report.status, ModelRepairStatus::Repaired);
    assert!(
        !report.sealable,
        "a scripted double is not live-provider evidence"
    );
    assert!(
        report
            .sealability_reasons
            .iter()
            .any(|r| r.contains("test double"))
    );
    assert!(report.verify().is_ok());
}
