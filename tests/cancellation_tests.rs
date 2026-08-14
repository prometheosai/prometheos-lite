//! Cooperative cancellation tests (issue #114).
//!
//! Cancellation is a distinct control-flow signal, NOT a failure. A cancelled
//! run stops at the next safe point, durably records a same-state `"cancelled"`
//! journal event, stops its heartbeat, fences further writes, and never deletes
//! the proposal/evidence. These tests prove the durable representation and that
//! a later run resumes from the authoritative journal position.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use prometheos_lite::harness::patch_provider::{
    BlockingProposalProvider, CountingProposalProvider, MockProposalMode, MockProposalProvider,
};
use prometheos_lite::workflow::evaluate::{
    CancellationToken, EvaluationConfig, EvaluationState, LeaseConfig, TaskManifest,
    evaluate_with_cancellation, read_journal,
};
use tempfile::TempDir;
use tokio::sync::Barrier;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn temp_repo() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/calc.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();
    std::fs::write(repo.join(".gitignore"), ".prometheos/\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    (dir, repo)
}

#[cfg(windows)]
const OK_VALIDATION: &str = "findstr /L generated src\\generated_patch.rs";
#[cfg(not(windows))]
const OK_VALIDATION: &str = "grep -qF 'generated' src/generated_patch.rs";

fn make_manifest(repo: &Path, goal: &str) -> TaskManifest {
    TaskManifest {
        task_id: goal.to_string(),
        goal: goal.to_string(),
        repo: repo.to_path_buf(),
        allowed_paths: vec!["src/**".to_string()],
        forbidden_paths: vec![],
        allow_dependency_changes: false,
        max_files_changed: None,
        max_lines_changed: None,
        validation_command: Some(OK_VALIDATION.to_string()),
        provider: "mock".to_string(),
        authority: "propose".to_string(),
        min_disk_bytes: 100 * 1024 * 1024,
        evidence_dir: None,
    }
}

fn default_config(manifest: TaskManifest) -> EvaluationConfig {
    EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: LeaseConfig::default(),
    }
}

/// Poll the registry until an entry appears; return its identity key.
async fn wait_for_identity_key(repo: &Path, timeout: Duration) -> String {
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");
    let start = std::time::Instant::now();
    loop {
        if path.exists()
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(reg) =
                serde_json::from_str::<prometheos_lite::workflow::evaluate::ProposalRegistry>(&text)
            && let Some(key) = reg.entries.keys().next()
        {
            return key.clone();
        }
        assert!(
            start.elapsed() < timeout,
            "registry entry never appeared for {}",
            repo.display()
        );
        // Yield to the runtime so a spawned run can make progress (reserve).
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn cancellation_before_generation_is_durable() {
    let (_dir, repo) = temp_repo();
    let manifest = make_manifest(&repo, "cancel-before-generation");

    // Pre-cancel: the first safe point (before the provider is invoked) fires.
    let token = CancellationToken::new();
    token.cancel();
    let result = evaluate_with_cancellation(default_config(manifest), token).await;
    let err = result.expect_err("cancellation must surface as an error");
    assert!(
        err.to_string().contains("cancelled"),
        "distinct cancellation error expected: {err}"
    );

    // The journal must carry a same-state "cancelled" event at PreflightPassed
    // (the state where the run stopped) — a durable, auditable trace.
    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    let events = read_journal(&repo, &key).unwrap();
    let tail = events.last().expect("cancellation must be journaled");
    assert_eq!(tail.to_state, EvaluationState::PreflightPassed);
    assert_eq!(tail.from_state, EvaluationState::PreflightPassed);
    assert_eq!(tail.failure_classification.as_deref(), Some("cancelled"));

    // The reservation is retained (fences writes); its heartbeat stops, so it
    // ages toward stale and a later run may safely resume or reconcile.
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");
    let reg: prometheos_lite::workflow::evaluate::ProposalRegistry =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        reg.entries.contains_key(&key),
        "cancelled run must keep its reservation (fence writes)"
    );
}

#[tokio::test]
async fn cancellation_during_generation_is_uncertain_and_journaled() {
    let (_dir, repo) = temp_repo();
    let manifest = make_manifest(&repo, "cancel-during-generation");

    let inner = Arc::new(prometheos_lite::harness::BlockingProviderInner {
        invocation_count: AtomicUsize::new(0),
        barrier: Barrier::new(2),
    });
    let provider = BlockingProposalProvider {
        inner: inner.clone(),
    };
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(provider),
        route_info: None,
        lease_config: LeaseConfig::default(),
    };
    let token = CancellationToken::new();
    let run_token = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, run_token).await });

    // Wait until the provider is invoked (and blocked on the barrier).
    let start = std::time::Instant::now();
    while inner.invocation_count.load(Ordering::SeqCst) == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "provider was never invoked"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Cancel while generation is in flight.
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(60), handle)
        .await
        .expect("evaluate must stop at the cancellation safe point")
        .expect("task must not panic");
    let err = result.expect_err("cancellation must surface as an error");
    assert!(err.to_string().contains("cancelled"), "{err}");

    // The journal must record a same-state "cancelled" event at Generating.
    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    let events = read_journal(&repo, &key).unwrap();
    let tail = events.last().expect("cancellation must be journaled");
    assert_eq!(tail.to_state, EvaluationState::Generating);
    assert_eq!(tail.failure_classification.as_deref(), Some("cancelled"));
}

#[tokio::test]
async fn evaluate_without_cancellation_completes_normally() {
    // Sanity: a never-cancelled token must behave exactly like `evaluate`.
    let (_dir, repo) = temp_repo();
    let manifest = make_manifest(&repo, "no-cancel");
    let bundle = evaluate_with_cancellation(default_config(manifest), CancellationToken::new())
        .await
        .unwrap();
    assert!(bundle.proposal.is_some());
    assert!(bundle.validation.is_some());
}

// ---------------------------------------------------------------------------
// Blocker 5: validation must be cancellable WHILE IT IS RUNNING.
// ---------------------------------------------------------------------------

#[cfg(windows)]
const LONG_VALIDATION: &str = "ping -n 60 127.0.0.1 >nul";
#[cfg(not(windows))]
const LONG_VALIDATION: &str = "sleep 60";

#[tokio::test]
async fn cancel_during_active_validation() {
    let (_dir, repo) = temp_repo();
    let mut manifest = make_manifest(&repo, "cancel-during-active-validation");
    manifest.validation_command = Some(LONG_VALIDATION.to_string());
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: LeaseConfig::default(),
    };
    let token = CancellationToken::new();
    let run_token = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, run_token).await });

    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    // Wait until validation actually starts (journal reached Validating).
    let start = std::time::Instant::now();
    loop {
        let events = read_journal(&repo, &key).unwrap();
        if events
            .iter()
            .any(|e| e.to_state == EvaluationState::Validating)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "validation never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Cancel while the long validation command is still running.
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(60), handle)
        .await
        .expect("evaluate must stop during cancellation")
        .expect("task must not panic");
    let err = result.expect_err("cancellation during validation must surface as error");
    assert!(err.to_string().contains("cancelled"), "{err}");

    // The journal records a same-state "cancelled" event at Validating (the
    // cancellation happened during validation, not before it).
    let events = read_journal(&repo, &key).unwrap();
    let tail = events.last().expect("cancellation must be journaled");
    assert_eq!(tail.to_state, EvaluationState::Validating);
    assert_eq!(tail.failure_classification.as_deref(), Some("cancelled"));
}

#[tokio::test]
async fn cancel_during_resumed_validation() {
    let (_dir, repo) = temp_repo();
    // A short lease so a cancelled run's entry goes stale quickly and the
    // second run can reclaim it without waiting out a long timeout.
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    assert!(short.validate().is_ok());

    // First run: cancel before validation so a durable proposal exists and the
    // journal stops before ValidationComplete.
    let mut m1 = make_manifest(&repo, "resume-cancel-validation");
    m1.validation_command = Some(LONG_VALIDATION.to_string());
    let c1 = EvaluationConfig {
        manifest: m1,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short.clone(),
    };
    let t1 = CancellationToken::new();
    let rt1 = t1.clone();
    let h1 = tokio::spawn(async move { evaluate_with_cancellation(c1, rt1).await });
    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    let start = std::time::Instant::now();
    loop {
        let events = read_journal(&repo, &key).unwrap();
        if events
            .iter()
            .any(|e| e.to_state == EvaluationState::ProposalGenerated)
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    t1.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(60), h1).await;

    // Second run: resumes validation and we cancel DURING the resumed validation.
    let mut m2 = make_manifest(&repo, "resume-cancel-validation");
    m2.validation_command = Some(LONG_VALIDATION.to_string());
    let c2 = EvaluationConfig {
        manifest: m2,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short.clone(),
    };
    let t2 = CancellationToken::new();
    let rt2 = t2.clone();
    let h2 = tokio::spawn(async move { evaluate_with_cancellation(c2, rt2).await });
    let start2 = std::time::Instant::now();
    loop {
        let events = read_journal(&repo, &key).unwrap();
        let resumed = events.iter().any(|e| {
            e.to_state == EvaluationState::Validating
                && e.failure_classification.as_deref() != Some("cancelled")
        });
        if resumed {
            break;
        }
        assert!(
            start2.elapsed() < Duration::from_secs(30),
            "resumed validation never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    t2.cancel();
    let result = tokio::time::timeout(Duration::from_secs(60), h2)
        .await
        .expect("resumed evaluate must stop")
        .expect("task must not panic");
    let err = result.expect_err("resumed validation cancellation must surface as error");
    assert!(err.to_string().contains("cancelled"), "{err}");
    let events = read_journal(&repo, &key).unwrap();
    let tail = events.last().expect("cancellation must be journaled");
    assert_eq!(tail.to_state, EvaluationState::Validating);
    assert_eq!(tail.failure_classification.as_deref(), Some("cancelled"));
}

// ---------------------------------------------------------------------------
// Blocker 6: pre-finalization cancellation boundary. Cancellation must be
// honoured AFTER validation/integrity are durable but BEFORE the terminal
// event is published, and recovery must resume finalization without repeating
// generation or validation.
// ---------------------------------------------------------------------------

/// A validation command that always succeeds (exits 0). Used so the run
/// reliably reaches `ValidationComplete`, which the tests observe via the
/// journal to time cancellation and to prove validation is not re-run on
/// resume. The command's first token must resolve via `where`/`which` so the
/// preflight availability check passes.
fn marker_validation_cmd(_repo: &Path) -> String {
    // A validation command that always succeeds (exits 0) so the run reliably
    // reaches `ValidationComplete`, which the tests observe via the journal.
    // No inner quotes: `validation_shell_async` already wraps the command in
    // `cmd /C "..."` (Windows) / `sh -c "..."` (Unix), and an inner quoted
    // `cmd /C "..."` is mangled by tokio's argument quoting. The first token
    // (`cmd`/`echo`) resolves via `where`/`which`, so the preflight
    // availability check passes.
    #[cfg(windows)]
    let cmd = "cmd /C echo marker> _validation_ran.txt";
    #[cfg(not(windows))]
    let cmd = "echo marker > _validation_ran.txt";
    cmd.to_string()
}

enum CancelTrigger {
    JournalValidationComplete,
    JournalIntegrityVerified,
}

/// Run an evaluation, cancel at a pre-finalization safe point, then run again to
/// resume and finalize. Returns (generation count, validation marker lines,
/// terminal event count).
async fn late_cancel_then_resume(
    repo: &Path,
    goal: &str,
    validation_cmd: &str,
    trigger: CancelTrigger,
) -> (usize, usize, usize) {
    // Short lease: a cancelled run's entry goes stale quickly so the resume can
    // reclaim it promptly (the cancelled owner is gone but its registry entry
    // still looks live until it ages out).
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    assert!(short.validate().is_ok());

    let provider_count = Arc::new(AtomicUsize::new(0));
    let mut manifest = make_manifest(repo, goal);
    manifest.validation_command = Some(validation_cmd.to_string());
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(CountingProposalProvider::new(provider_count.clone())),
        route_info: None,
        lease_config: short.clone(),
    };
    let token = CancellationToken::new();
    let run_token = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, run_token).await });

    let key = wait_for_identity_key(repo, Duration::from_secs(30)).await;
    match &trigger {
        CancelTrigger::JournalValidationComplete => {
            let start = std::time::Instant::now();
            loop {
                let events = read_journal(repo, &key).unwrap();
                if events
                    .iter()
                    .any(|e| e.to_state == EvaluationState::ValidationComplete)
                {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "validation complete never reached"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        CancelTrigger::JournalIntegrityVerified => {
            let start = std::time::Instant::now();
            loop {
                let events = read_journal(repo, &key).unwrap();
                if events
                    .iter()
                    .any(|e| e.to_state == EvaluationState::IntegrityVerified)
                {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "integrity verified never reached"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    token.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(60), handle).await;

    // Resume: must complete exactly once, never re-invoking generation or
    // re-running validation.
    let mut manifest2 = make_manifest(repo, goal);
    manifest2.validation_command = Some(validation_cmd.to_string());
    let config2 = EvaluationConfig {
        manifest: manifest2,
        provider: Box::new(CountingProposalProvider::new(provider_count.clone())),
        route_info: None,
        lease_config: short.clone(),
    };
    let bundle = evaluate_with_cancellation(config2, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        bundle.proposal.is_some(),
        "resumed run must complete with a proposal"
    );

    let events = read_journal(repo, &key).unwrap();
    let terminal = events.iter().filter(|e| e.to_state.is_terminal()).count();
    // Validation runs are recorded as `ValidationComplete` journal events. The
    // journal persists across both runs for the same proposal, so the count is
    // the total number of validation executions; a correct resume must not
    // re-run validation (exactly 1).
    let validation_runs = events
        .iter()
        .filter(|e| e.to_state == EvaluationState::ValidationComplete)
        .count();
    (
        provider_count.load(Ordering::SeqCst),
        validation_runs,
        terminal,
    )
}

#[tokio::test]
async fn cancel_after_validation_complete_is_resumable() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, val, term) = late_cancel_then_resume(
        &repo,
        "late-vc",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(gen_count, 1, "generation must not repeat");
    assert_eq!(term, 1, "exactly one terminal event must be published");
    assert_eq!(val, 1, "validation must run once and not repeat on resume");
}

#[tokio::test]
async fn cancel_after_integrity_verified_is_resumable() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, val, term) = late_cancel_then_resume(
        &repo,
        "late-iv",
        &cmd,
        CancelTrigger::JournalIntegrityVerified,
    )
    .await;
    assert_eq!(gen_count, 1, "generation must not repeat");
    assert_eq!(term, 1, "exactly one terminal event must be published");
    assert_eq!(val, 1, "validation must run once and not repeat on resume");
}

#[tokio::test]
async fn late_cancel_does_not_repeat_generation() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, _, _) = late_cancel_then_resume(
        &repo,
        "late-noregen",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(gen_count, 1, "generation must not repeat on resume");
}

#[tokio::test]
async fn late_cancel_does_not_repeat_validation() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (_, val, _) = late_cancel_then_resume(
        &repo,
        "late-noval",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(val, 1, "validation must not repeat on resume");
}

#[tokio::test]
async fn late_cancel_produces_single_terminal_event_after_resume() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (_, _, term) = late_cancel_then_resume(
        &repo,
        "late-oneterm",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(term, 1, "exactly one terminal event must be published");
}
