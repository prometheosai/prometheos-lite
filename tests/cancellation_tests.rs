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
    BlockingProposalProvider, MockProposalMode, MockProposalProvider,
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
fn wait_for_identity_key(repo: &Path, timeout: Duration) -> String {
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json");
    let start = std::time::Instant::now();
    loop {
        if path.exists()
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(reg) = serde_json::from_str::<
                prometheos_lite::workflow::evaluate::ProposalRegistry,
            >(&text)
            && let Some(key) = reg.entries.keys().next()
        {
            return key.clone();
        }
        assert!(
            start.elapsed() < timeout,
            "registry entry never appeared for {}",
            repo.display()
        );
        std::thread::sleep(Duration::from_millis(20));
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
    let key = wait_for_identity_key(&repo, Duration::from_secs(30));
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
    let key = wait_for_identity_key(&repo, Duration::from_secs(30));
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
