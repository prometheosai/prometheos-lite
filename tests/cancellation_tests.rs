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

use chrono::Utc;
use prometheos_lite::harness::patch_provider::{
    BlockingProposalProvider, CountingProposalProvider, MockProposalMode, MockProposalProvider,
};
use prometheos_lite::workflow::evaluate::{
    CancellationToken, EvaluationConfig, EvaluationState, LeaseConfig, ProposalRegistry,
    ProposalState, RegistryEntry, TakeoverResult, TaskManifest, evaluate_with_cancellation,
    is_entry_stale_at, read_journal, try_take_ownership,
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
///
/// Cancellation is made deterministic with a test-only barrier: the first run
/// parks at the chosen late safe point until the test cancels the token and
/// releases it. This removes the race between a test observing durable progress
/// and cancelling a run that might otherwise race ahead and publish a terminal
/// event before the cancel arrives.
async fn late_cancel_then_resume(
    repo: &Path,
    goal: &str,
    validation_cmd: &str,
    trigger: CancelTrigger,
) -> (usize, usize, usize, usize) {
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

    // Install a rendezvous barrier on the token so the first run parks at the
    // late safe point until the test cancels and releases it.
    let hold = Arc::new(Barrier::new(2));
    let token = CancellationToken::with_park_barrier(hold.clone());
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
            // Parked at the ValidationComplete safe point. Cancel, then release
            // the hold so the run proceeds into the cancellation path.
            token.cancel();
            hold.wait().await;
        }
        CancelTrigger::JournalIntegrityVerified => {
            // Release the ValidationComplete park first (no cancel yet), let the
            // run run integrity, then park at the IntegrityVerified safe point.
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
            hold.wait().await;
            let start2 = std::time::Instant::now();
            loop {
                let events = read_journal(repo, &key).unwrap();
                if events
                    .iter()
                    .any(|e| e.to_state == EvaluationState::IntegrityVerified)
                {
                    break;
                }
                assert!(
                    start2.elapsed() < Duration::from_secs(30),
                    "integrity verified never reached"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Parked at the IntegrityVerified safe point. Cancel, then release.
            token.cancel();
            hold.wait().await;
        }
    }
    let result = tokio::time::timeout(Duration::from_secs(60), handle)
        .await
        .expect("first run must stop at the cancellation safe point")
        .expect("first run must not panic");
    let err = result.expect_err("cancellation must surface as a distinct error");
    assert!(
        err.to_string().contains("cancelled"),
        "distinct cancellation error expected: {err}"
    );

    // The journal tail must be exactly the nonterminal same-state cancellation.
    let events = read_journal(repo, &key).unwrap();
    let tail = events.last().expect("cancellation must be journaled");
    let expected = match &trigger {
        CancelTrigger::JournalValidationComplete => EvaluationState::ValidationComplete,
        CancelTrigger::JournalIntegrityVerified => EvaluationState::IntegrityVerified,
    };
    assert_eq!(
        tail.to_state, expected,
        "journal tail state must match the safe point"
    );
    assert_eq!(
        tail.from_state, expected,
        "cancellation is a same-state event"
    );
    assert_eq!(
        tail.failure_classification.as_deref(),
        Some("cancelled"),
        "journal tail must be the nonterminal same-state cancellation"
    );
    // No terminal event has been published before the resume.
    let terminal_before = events.iter().filter(|e| e.to_state.is_terminal()).count();
    assert_eq!(terminal_before, 0, "no terminal event before resume");

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
        .filter(|e| {
            e.to_state == EvaluationState::ValidationComplete
                && e.failure_classification.as_deref() != Some("cancelled")
        })
        .count();
    // Integrity verifications are recorded as `IntegrityVerified` journal events;
    // count only durable (non-cancelled) completions. `ResumeFinalization` must
    // reuse the durable integrity artifact, not re-run integrity.
    let integrity_runs = events
        .iter()
        .filter(|e| {
            e.to_state == EvaluationState::IntegrityVerified
                && e.failure_classification.as_deref() != Some("cancelled")
        })
        .count();
    (
        provider_count.load(Ordering::SeqCst),
        validation_runs,
        terminal,
        integrity_runs,
    )
}

#[tokio::test]
async fn cancel_after_validation_complete_is_resumable() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, val, term, integrity) = late_cancel_then_resume(
        &repo,
        "late-vc",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(gen_count, 1, "generation must not repeat");
    assert_eq!(term, 1, "exactly one terminal event must be published");
    assert_eq!(val, 1, "validation must run once and not repeat on resume");
    assert_eq!(integrity, 1, "integrity must run exactly once on resume");
}

#[tokio::test]
async fn cancel_after_integrity_verified_is_resumable() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, val, term, integrity) = late_cancel_then_resume(
        &repo,
        "late-iv",
        &cmd,
        CancelTrigger::JournalIntegrityVerified,
    )
    .await;
    assert_eq!(gen_count, 1, "generation must not repeat");
    assert_eq!(term, 1, "exactly one terminal event must be published");
    assert_eq!(val, 1, "validation must run once and not repeat on resume");
    // ResumeFinalization must reuse the durable integrity artifact (count stays 1),
    // never recompute it.
    assert_eq!(integrity, 1, "integrity must not be re-run on resume");
}

#[tokio::test]
async fn late_cancel_does_not_repeat_generation() {
    let (_dir, repo) = temp_repo();
    let cmd = marker_validation_cmd(&repo);
    let (gen_count, _, _, _) = late_cancel_then_resume(
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
    let (_, val, _, _) = late_cancel_then_resume(
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
    let (_, _, term, _) = late_cancel_then_resume(
        &repo,
        "late-oneterm",
        &cmd,
        CancelTrigger::JournalValidationComplete,
    )
    .await;
    assert_eq!(term, 1, "exactly one terminal event must be published");
}

// ---------------------------------------------------------------------------
// Blocker re-check: the heartbeat must actually STOP when a run is cancelled
// before generation. A detached, still-renewing heartbeat would keep a dead
// owner's reservation immortal and prevent any later reclaim.
// ---------------------------------------------------------------------------

fn read_registry_entry(repo: &Path) -> RegistryEntry {
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");
    let reg: ProposalRegistry =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    reg.entries
        .into_values()
        .next()
        .expect("registry entry must exist")
}

#[tokio::test]
async fn cancel_before_generation_stops_heartbeat() {
    let (_dir, repo) = temp_repo();
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(50),
    );
    assert!(short.validate().is_ok());

    let mut manifest = make_manifest(&repo, "cancel-before-gen-heartbeat");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short.clone(),
    };
    // Cancel before the run starts so it deterministically bails at the
    // before-generation safe point, leaving a Reserved (reclaimable) entry and
    // a heartbeat that must stop once the run exits.
    let token = CancellationToken::new();
    token.cancel();
    let run_token = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, run_token).await });

    let _key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    let hb0 = read_registry_entry(&repo).heartbeat_at;
    let result = tokio::time::timeout(Duration::from_secs(60), handle)
        .await
        .expect("run must stop at the cancellation safe point")
        .expect("run must not panic");
    let err = result.expect_err("cancellation must surface as an error");
    assert!(err.to_string().contains("cancelled"), "{err}");

    let hb1 = read_registry_entry(&repo).heartbeat_at;
    assert!(
        hb1 >= hb0,
        "heartbeat should advance while the run is live (hb0={hb0:?} hb1={hb1:?})"
    );
    // Wait several heartbeat intervals past the cancelled run; the dead owner
    // must NOT keep renewing.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let hb2 = read_registry_entry(&repo).heartbeat_at;
    assert_eq!(
        hb1, hb2,
        "heartbeat must stop advancing after cancel-before-generation"
    );

    // Prove it becomes stale at an injected deterministic time: a timestamp
    // after the frozen heartbeat plus the stale reservation timeout is stale.
    let entry = read_registry_entry(&repo);
    assert_eq!(entry.state, ProposalState::Reserved);
    let injected_now = Utc::now()
        + chrono::Duration::from_std(short.stale_reservation_timeout + Duration::from_secs(5))
            .unwrap();
    assert!(
        is_entry_stale_at(&entry, &short, injected_now).unwrap(),
        "cancelled Reserved entry must be stale at the injected time"
    );
}

#[tokio::test]
async fn cancelled_reserved_entry_eventually_becomes_reclaimable() {
    let (_dir, repo) = temp_repo();
    // Short stale reservation timeout so the frozen heartbeat ages out quickly
    // after cancellation, proving the entry becomes reclaimable. Heartbeat*3
    // (150ms) <= stale timeout (200ms) keeps the lease valid.
    let short = LeaseConfig::with_timeouts(
        Duration::from_millis(200),
        Duration::from_secs(2),
        Duration::from_millis(50),
    );
    assert!(short.validate().is_ok());

    let mut manifest = make_manifest(&repo, "cancel-before-gen-reclaim");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short.clone(),
    };
    // Cancel before the run starts so it bails at the before-generation safe
    // point deterministically, leaving a Reserved (reclaimable) entry.
    let token = CancellationToken::new();
    token.cancel();
    let run_token = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, run_token).await });
    let _ = tokio::time::timeout(Duration::from_secs(60), handle).await;

    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    // Wait for the frozen heartbeat to age past the stale threshold.
    tokio::time::sleep(Duration::from_millis(400)).await;
    match try_take_ownership(&repo, &key, "reclaimer", &short).unwrap() {
        TakeoverResult::Taken(_) => {}
        other => panic!("cancelled Reserved entry must be reclaimable, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Blocker re-check (issue #114, restart-path): a second crash during late
// finalization must be recoverable. Three operations reuse the same durable
// proposal: generation and validation run exactly once, and exactly one
// terminal event is ever published.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_crash_recovery_produces_one_terminal() {
    let (_dir, repo) = temp_repo();
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    assert!(short.validate().is_ok());

    let provider_count = Arc::new(AtomicUsize::new(0));

    // --- RUN A: generate + validate, cancel at the ValidationComplete safe point.
    let a_hold = Arc::new(Barrier::new(2));
    let ta = CancellationToken::with_park_barrier(a_hold.clone());
    let mut ma = make_manifest(&repo, "second-crash");
    ma.validation_command = Some(marker_validation_cmd(&repo));
    let ca = EvaluationConfig {
        manifest: ma,
        provider: Box::new(CountingProposalProvider::new(provider_count.clone())),
        route_info: None,
        lease_config: short.clone(),
    };
    let rta = ta.clone();
    let ha = tokio::spawn(async move { evaluate_with_cancellation(ca, rta).await });
    let key = wait_for_identity_key(&repo, Duration::from_secs(30)).await;
    let start = std::time::Instant::now();
    loop {
        let events = read_journal(&repo, &key).unwrap();
        if events
            .iter()
            .any(|e| e.to_state == EvaluationState::ValidationComplete)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "A: VC never reached"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    ta.cancel();
    a_hold.wait().await;
    let ra = tokio::time::timeout(Duration::from_secs(60), ha)
        .await
        .expect("A must stop")
        .expect("A must not panic");
    assert!(ra.is_err() && ra.unwrap_err().to_string().contains("cancelled"));

    let events_a = read_journal(&repo, &key).unwrap();
    let proposal_id = events_a
        .iter()
        .find(|e| e.to_state == EvaluationState::ValidationComplete)
        .and_then(|e| e.proposal_ref.clone())
        .expect("A must record a proposal reference");
    assert!(!proposal_id.is_empty());
    let term_a = events_a.iter().filter(|e| e.to_state.is_terminal()).count();
    assert_eq!(term_a, 0, "no terminal before the resume");

    // Let A's entry age out so B can reclaim it deterministically.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // --- RUN B: reclaim A, resume, run integrity, then CRASH before finalization.
    let b_hold = Arc::new(Barrier::new(2));
    let rb = CancellationToken::with_park_barrier(b_hold.clone());
    let mut mb = make_manifest(&repo, "second-crash");
    mb.validation_command = Some(marker_validation_cmd(&repo));
    let cb = EvaluationConfig {
        manifest: mb,
        provider: Box::new(CountingProposalProvider::new(provider_count.clone())),
        route_info: None,
        lease_config: short.clone(),
    };
    let rtb = rb.clone();
    let hb = tokio::spawn(async move { evaluate_with_cancellation(cb, rtb).await });
    let start_b = std::time::Instant::now();
    loop {
        let events = read_journal(&repo, &key).unwrap();
        if events
            .iter()
            .any(|e| e.to_state == EvaluationState::IntegrityVerified)
        {
            break;
        }
        assert!(
            start_b.elapsed() < Duration::from_secs(30),
            "B: IntegrityVerified never reached"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Simulate a crash (process killed) while parked at the pre-finalization
    // hold. The barrier is never released, so B stays parked until aborted.
    hb.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Let B's entry age out so C can reclaim it deterministically.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // --- RUN C: reclaim B, publish the terminal outcome exactly once.
    let mut mc = make_manifest(&repo, "second-crash");
    mc.validation_command = Some(marker_validation_cmd(&repo));
    let cc = EvaluationConfig {
        manifest: mc,
        provider: Box::new(CountingProposalProvider::new(provider_count.clone())),
        route_info: None,
        lease_config: short.clone(),
    };
    let bundle_c = evaluate_with_cancellation(cc, CancellationToken::new())
        .await
        .expect("C must finalize");
    assert!(
        bundle_c.proposal.is_some(),
        "C must complete with the original proposal"
    );

    let events = read_journal(&repo, &key).unwrap();
    let terminal = events.iter().filter(|e| e.to_state.is_terminal()).count();
    let validation_runs = events
        .iter()
        .filter(|e| {
            e.to_state == EvaluationState::ValidationComplete
                && e.failure_classification.as_deref() != Some("cancelled")
        })
        .count();
    let integrity_runs = events
        .iter()
        .filter(|e| {
            e.to_state == EvaluationState::IntegrityVerified
                && e.failure_classification.as_deref() != Some("cancelled")
        })
        .count();

    assert_eq!(
        provider_count.load(Ordering::SeqCst),
        1,
        "generation runs exactly once"
    );
    assert_eq!(validation_runs, 1, "validation runs exactly once");
    assert_eq!(integrity_runs, 1, "integrity runs exactly once");
    assert_eq!(terminal, 1, "exactly one terminal event is published");
    assert_eq!(
        bundle_c.proposal.as_ref().unwrap().id,
        proposal_id,
        "the original proposal id is preserved across the second crash"
    );
}

// ---------------------------------------------------------------------------
// Durable evidence-chain proof (issue #114, restart-path final bug).
//
// After recovering from `ValidationComplete`, run B must write its new
// `integrity.json` into the SAME directory the `IntegrityVerified` journal event
// references (the authoritative source evidence directory), so the artifact and
// its reference are consistent. `ResumeFinalization` must then consume the
// durable integrity artifact instead of recomputing it, and any missing or
// corrupt durable evidence must fail closed rather than healing itself.
// ---------------------------------------------------------------------------

/// Run A to `ValidationComplete`, then cancel at the safe point. Returns the
/// identity key and A's durable evidence directory (resolved from the journal).
async fn run_a_cancel_at_validation_complete(repo: &Path) -> (String, PathBuf) {
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let hold = Arc::new(Barrier::new(2));
    let token = CancellationToken::with_park_barrier(hold.clone());
    let rt = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, rt).await });
    let key = wait_for_identity_key(repo, Duration::from_secs(30)).await;
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
            "A: VC never reached"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    token.cancel();
    hold.wait().await;
    let _ = tokio::time::timeout(Duration::from_secs(60), handle).await;
    let events = read_journal(repo, &key).unwrap();
    let ref_str = events
        .iter()
        .rev()
        .find_map(|e| e.evidence_ref.clone())
        .expect("A must record an evidence_ref");
    (key, repo.join(&ref_str))
}

/// Reclaim A and resume finalization: run integrity, park at the
/// `IntegrityVerified` safe point, then abort (simulated crash) before
/// finalization. The durable `integrity.json` is persisted into the source
/// evidence directory by this point.
async fn run_b_resume_to_integrity_then_crash(repo: &Path, key: &str) {
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let hold = Arc::new(Barrier::new(2));
    let token = CancellationToken::with_park_barrier(hold.clone());
    let rt = token.clone();
    let handle = tokio::spawn(async move { evaluate_with_cancellation(config, rt).await });
    let start = std::time::Instant::now();
    loop {
        let events = read_journal(repo, key).unwrap();
        if events
            .iter()
            .any(|e| e.to_state == EvaluationState::IntegrityVerified)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "B: IntegrityVerified never reached"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Crash before finalization: abort while parked at the safe point.
    handle.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Rewrite every journal event so its `evidence_ref` is null, simulating a
/// recovered state with no durable evidence reference.
fn null_out_evidence_refs(repo: &Path, key: &str) {
    let dir = repo
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join(key);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("json") {
            let text = std::fs::read_to_string(&p).unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
            v["evidence_ref"] = serde_json::Value::Null;
            std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
    }
}

#[tokio::test]
async fn integrity_verified_evidence_ref_resolves_and_contains_artifacts() {
    let (_dir, repo) = temp_repo();
    let (key, a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    // Let A's entry age out so B can reclaim it deterministically.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    run_b_resume_to_integrity_then_crash(&repo, &key).await;

    // The `IntegrityVerified` journal event must reference a directory that
    // actually contains BOTH the durable validation and integrity artifacts.
    let events = read_journal(&repo, &key).unwrap();
    let iv_event = events
        .iter()
        .find(|e| e.to_state == EvaluationState::IntegrityVerified)
        .expect("IntegrityVerified must be journaled");
    let ref_str = iv_event
        .evidence_ref
        .as_ref()
        .expect("IntegrityVerified must carry an evidence_ref");
    let dir = repo.join(ref_str);
    assert!(dir.exists(), "IntegrityVerified evidence_ref must resolve");
    assert!(
        dir.join("validation.json").exists(),
        "resolved directory must contain validation.json"
    );
    assert!(
        dir.join("integrity.json").exists(),
        "resolved directory must contain integrity.json (written by B into the source dir)"
    );
    // The source directory is A's original durable evidence directory.
    assert_eq!(
        dir, a_dir,
        "IntegrityVerified must reference the source evidence dir"
    );
}

#[tokio::test]
async fn resume_finalization_reuses_durable_integrity() {
    let (_dir, repo) = temp_repo();
    let (key, _a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    run_b_resume_to_integrity_then_crash(&repo, &key).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // C resumes as `ResumeFinalization`: it must consume the durable integrity
    // artifact WITHOUT re-running integrity, and finalize exactly once.
    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(&repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let bundle = evaluate_with_cancellation(config, CancellationToken::new())
        .await
        .expect("C must finalize from the durable IntegrityVerified state");
    assert!(bundle.proposal.is_some());

    let events = read_journal(&repo, &key).unwrap();
    let integrity_runs = events
        .iter()
        .filter(|e| {
            e.to_state == EvaluationState::IntegrityVerified
                && e.failure_classification.as_deref() != Some("cancelled")
        })
        .count();
    assert_eq!(integrity_runs, 1, "integrity must not be re-run on resume");
    assert_eq!(
        events.iter().filter(|e| e.to_state.is_terminal()).count(),
        1,
        "exactly one terminal event after resume"
    );
}

#[tokio::test]
async fn missing_integrity_json_after_integrity_verified_fails_closed() {
    let (_dir, repo) = temp_repo();
    let (key, a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    run_b_resume_to_integrity_then_crash(&repo, &key).await;
    assert!(
        a_dir.join("integrity.json").exists(),
        "B must persist integrity into the source dir"
    );
    // Simulate loss of the durable integrity artifact.
    std::fs::remove_file(a_dir.join("integrity.json")).unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(&repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let result = evaluate_with_cancellation(config, CancellationToken::new()).await;
    assert!(
        result.is_err(),
        "ResumeFinalization must fail closed when the durable integrity artifact is missing: {result:?}"
    );
}

#[tokio::test]
async fn corrupt_integrity_json_after_integrity_verified_fails_closed() {
    let (_dir, repo) = temp_repo();
    let (key, a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    run_b_resume_to_integrity_then_crash(&repo, &key).await;
    assert!(a_dir.join("integrity.json").exists());
    std::fs::write(a_dir.join("integrity.json"), "this is not valid json {").unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(&repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let result = evaluate_with_cancellation(config, CancellationToken::new()).await;
    assert!(
        result.is_err(),
        "ResumeFinalization must fail closed when the durable integrity artifact is corrupt: {result:?}"
    );
}

#[tokio::test]
async fn unresolvable_evidence_ref_fails_closed() {
    let (_dir, repo) = temp_repo();
    let (_key, a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    // Delete the entire source evidence directory so the durable reference cannot
    // be resolved.
    std::fs::remove_dir_all(&a_dir).unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(&repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let result = evaluate_with_cancellation(config, CancellationToken::new()).await;
    assert!(
        result.is_err(),
        "late finalization must fail closed when the evidence_ref is unresolvable: {result:?}"
    );
}

#[tokio::test]
async fn missing_evidence_ref_fails_closed() {
    let (_dir, repo) = temp_repo();
    let (key, _a_dir) = run_a_cancel_at_validation_complete(&repo).await;
    // Null out every journal evidence_ref so recovery yields none.
    null_out_evidence_refs(&repo, &key);
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let short = LeaseConfig::with_timeouts(
        Duration::from_secs(2),
        Duration::from_secs(2),
        Duration::from_millis(100),
    );
    let mut manifest = make_manifest(&repo, "setup-vc");
    manifest.validation_command = Some(marker_validation_cmd(&repo));
    let config = EvaluationConfig {
        manifest,
        provider: Box::new(MockProposalProvider::with_mode(MockProposalMode::Safe)),
        route_info: None,
        lease_config: short,
    };
    let result = evaluate_with_cancellation(config, CancellationToken::new()).await;
    assert!(
        result.is_err(),
        "late finalization must fail closed when no durable evidence_ref exists: {result:?}"
    );
}
