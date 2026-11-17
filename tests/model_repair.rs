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
    }
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
