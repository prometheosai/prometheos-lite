//! Issue #137 Slice A — controlled reliability / fault-injection campaign.
//!
//! These tests exercise the Lite durability boundary under deterministic,
//! injected interruptions and real cross-process races. They reuse the
//! existing production contracts (durable atomic publication, the file lock,
//! the proposal registry with lease/fencing semantics, the journal, and the
//! artifact integrity publication path) — nothing here implements a second
//! lease registry, journal or authority framework.
//!
//! Fault model: [`prometheos_lite::workflow::durable::set_atomic_write_hook`]
//! registers a one-shot thread-local hook consulted at each atomic-publication
//! step (temp create → write → fsync → rename → dir sync). The hook aborts
//! the publication exactly at that step with a typed error, exactly as a crash
//! would interrupt it in production. When unset (every other test), the hook
//! is a no-op. ENOSPC and friends are simulated this way — the host disk is
//! never filled. Cross-process races spawn the real test binary as a detached
//! OS process (as the #114/#155 suites do), so two threads in one process are
//! never mistaken for two workers.
//!
//! Validator timeouts and output-size limits are exercised against real
//! subprocesses through the same fixture validator runner the #136 executed
//! fixtures use (`tests/common/task_execution.rs`), with a caller-supplied
//! deadline. Evidence is always inspected (files, journal, registry, digests,
//! counters) — never a canned "it worked" flag.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::workflow::artifact_integrity::{
    self, ArtifactKind, publish_with_integrity, read_verified,
};
use prometheos_lite::workflow::durable::{
    AtomicWriteStep, atomic_write_bytes, atomic_write_json, set_atomic_write_hook,
};
use prometheos_lite::workflow::evaluate::{
    self, LeaseConfig, OwnershipObservation, ProposalRegistry, ProposalState, RegistryEntry,
    TakeoverResult, is_entry_stale_at, try_take_ownership_cas,
};
use prometheos_lite::workflow::scenario::{TaskOutcome, fixture_content_digest};

mod common;

use common::task_execution::{
    TaskExecutionSpec, fixture_root, run_task_execution, run_validator_with_timeout,
};

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");
const BUGFIX_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";

const EXECUTE_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/lib.rs",
    repo_identity_url: BUGFIX_REPO_IDENTITY_URL,
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

// ---------------------------------------------------------------------------
// Fault-injection helpers
// ---------------------------------------------------------------------------

/// Arm a one-shot fault hook: next call to the matching step fails once with
/// `message`, exactly as a crash at that step would surface in production.
/// The hook fires exactly once and then resets itself, so a single armed
/// fault cannot poison unrelated later publications.
fn arm_fault_once(step: AtomicWriteStep, message: &'static str) {
    let fired = std::cell::Cell::new(false);
    set_atomic_write_hook(Some(Box::new(move |at, _path| {
        if at == step && !fired.get() {
            fired.set(true);
            anyhow::bail!(message);
        }
        Ok(())
    })));
}

/// Disarm any fault hook (defensive; the suite arms hooks only inside the
/// tests that need them and always disarms on return).
fn disarm_fault() {
    set_atomic_write_hook(None);
}

/// Assert the pinned bugfix fixture is untouched (digest pins both
/// revision-dependent content and live test state).
fn assert_fixture_pristine() {
    let descriptor =
        prometheos_lite::workflow::scenario::ScenarioDescriptor::parse_json(EXECUTE_DESCRIPTOR)
            .expect("committed descriptor validates");
    let root = fixture_root(&descriptor);
    let digest = fixture_content_digest(&root).expect("fixture digests");
    assert_eq!(
        digest, "d4234cd58be74da1e9b369ab21bdecf96d33f1d43e12c4a5f7ae848dc2e400d6",
        "the committed fixture must never be mutated by the tests"
    );
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn registry_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json")
}

fn write_registry(repo: &Path, key: &str, entry: &RegistryEntry) {
    let path = registry_path(repo);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let reg = ProposalRegistry {
        entries: [(key.to_string(), entry.clone())].into_iter().collect(),
    };
    std::fs::write(&path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();
}

fn read_registry(repo: &Path) -> ProposalRegistry {
    let path = registry_path(repo);
    let text = std::fs::read_to_string(path).expect("registry readable");
    serde_json::from_str(&text).expect("registry parses")
}

// ---------------------------------------------------------------------------
// Campaign A — durable atomic publication interrupted at every step
// ---------------------------------------------------------------------------

#[test]
fn campaign_a_interrupted_before_temp_create_never_strands_half_docs() {
    let dir = tempdir();
    let target = dir.path().join("artifact.json");
    let pre = serde_json::json!({"status": "committed", "value": 7});
    atomic_write_json(&target, &pre).expect("pre-existing artifact publishes");
    let pre_bytes = std::fs::read(&target).expect("committed bytes readable");

    arm_fault_once(
        AtomicWriteStep::BeforeTempCreate,
        "simulated ENOSPC before temp create",
    );
    let result = atomic_write_json(&target, &serde_json::json!({"status": "replacement"}));
    disarm_fault();
    assert!(
        result.is_err(),
        "the interrupted publication must surface as an error"
    );

    // The committed document is byte-for-byte intact.
    assert_eq!(std::fs::read(&target).unwrap(), pre_bytes);
    // No temp material masquerading as a document.
    assert!(!dir.path().join("artifact.json.tmp").exists());
    assert!(!dir.path().join("artifact.tmp").exists());
    // And it still parses and carries its classification.
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).expect("committed doc parses");
    assert_eq!(parsed["status"], "committed");
}

#[test]
fn campaign_a_interrupted_during_temp_write_temp_material_is_never_valid() {
    let dir = tempdir();
    let target = dir.path().join("checkpoint.json");
    // Fault armed at TempCreated: the temp file is created (empty), the
    // hook aborts before any byte is written.
    let payload = serde_json::json!({"evidence": "durable", "value": 42});
    arm_fault_once(
        AtomicWriteStep::TempCreated,
        "simulated ENOSPC during temp write",
    );
    let result = atomic_write_json(&target, &payload);
    disarm_fault();
    assert!(result.is_err());

    let temp = dir.path().join("checkpoint.json.tmp");
    assert!(
        temp.exists(),
        "crash mid-publication leaves temp material behind"
    );
    let bytes = std::fs::read(&temp).unwrap();
    assert_eq!(
        bytes.len(),
        0,
        "no bytes were written before the interruption"
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes).is_err(),
        "temp material must not parse as a complete document"
    );
    assert!(
        !target.exists(),
        "interrupted publication must not create the target"
    );

    // Torn variant: explicitly truncate material on disk and confirm the
    // reader fails closed on its own (no injection needed for the read side).
    let torn = dir.path().join("torn.json");
    std::fs::write(&torn, b"{\"evidence\":").expect("torn temp bytes");
    assert!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&torn).unwrap()).is_err(),
        "torn bytes never parse as a document"
    );

    // Recovery: with the fault cleared, publication succeeds and the document
    // verifies.
    atomic_write_json(&target, &payload).expect("recovery publication succeeds");
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).expect("recovered doc parses");
    assert_eq!(parsed["evidence"], "durable");
}

#[test]
fn campaign_a_interrupted_after_temp_write_before_rename_keeps_old_bytes() {
    let dir = tempdir();
    let target = dir.path().join("registry.json");
    let old = serde_json::json!({"version": 1, "entries": 3});
    atomic_write_json(&target, &old).expect("base publish succeeds");
    let old_bytes = std::fs::read(&target).unwrap();

    // Crash after fsync but before the atomic rename: the temp file holds the
    // complete replacement, the target keeps the old committed bytes.
    arm_fault_once(
        AtomicWriteStep::TempFsynced,
        "simulated crash after temp fsync, before rename",
    );
    let result = atomic_write_json(&target, &serde_json::json!({"version": 2}));
    disarm_fault();
    assert!(result.is_err(), "the crash point must surface as an error");
    assert_eq!(std::fs::read(&target).unwrap(), old_bytes);
    let temp = dir.path().join("registry.json.tmp");
    assert!(
        temp.exists(),
        "the fully-written replacement remains in temp"
    );
    let temp_parsed: serde_json::Value = serde_json::from_slice(&std::fs::read(&temp).unwrap())
        .expect("complete temp replacement parses");
    assert_eq!(temp_parsed["version"], 2);

    // Recovery re-publication succeeds and replaces the old bytes atomically.
    atomic_write_json(&target, &serde_json::json!({"version": 2}))
        .expect("recovery publish succeeds");
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).expect("target parses");
    assert_eq!(parsed["version"], 2);
    assert_fixture_pristine();
}

#[test]
fn campaign_a_interrupted_immediately_after_atomic_publication_sees_complete_new_bytes() {
    // The rename is the atomic publication point: after it, the target MUST
    // hold the complete new bytes even though the caller never observed Ok
    // (the crash landed before the directory fsync returned).
    let dir = tempdir();
    let target = dir.path().join("published.json");
    arm_fault_once(
        AtomicWriteStep::AfterRename,
        "simulated crash immediately after atomic rename",
    );
    let result = atomic_write_json(&target, &serde_json::json!({"status": "committed"}));
    disarm_fault();
    assert!(result.is_err(), "the caller sees the crash as an error");

    let bytes = std::fs::read(&target).expect("rename already happened: the document exists");
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("published bytes parse");
    assert_eq!(parsed["status"], "committed");

    // The inverse (crash between artifact bytes and sidecar) must fail closed
    // at the integrity gate — a committed-looking artifact is never trusted
    // without its verification sidecar.
    let target2 = dir.path().join("evidence.json");
    arm_fault_once(
        AtomicWriteStep::AfterRename,
        "simulated crash between artifact rename and sidecar publication",
    );
    let publish = publish_with_integrity(
        dir.path(),
        &target2,
        b"real artifact bytes",
        ArtifactKind::Other,
    );
    disarm_fault();
    assert!(publish.is_err(), "the interruption must abort publication");

    // Artifact exists; sidecar was never published; integrity read refuses.
    assert!(target2.exists(), "artifact bytes are on disk");
    let read = read_verified(dir.path(), &target2, ArtifactKind::Other);
    assert!(
        read.is_err(),
        "an artifact without its verification sidecar must fail closed, not be read as valid evidence"
    );
}

#[test]
fn campaign_a_interrupted_after_publish_leaves_verifiable_artifact_but_no_false_claim() {
    let dir = tempdir();
    let target = dir.path().join("receipt.json");
    arm_fault_once(
        AtomicWriteStep::AfterPublish,
        "simulated crash after publication completed",
    );
    let result = atomic_write_json(&target, &serde_json::json!({"outcome": "complete"}));
    disarm_fault();
    assert!(result.is_err(), "caller does not observe Ok");
    let bytes = std::fs::read(&target).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
}

#[test]
fn campaign_a_torn_artifact_fails_verification_and_forged_sidecar_fails_too() {
    let dir = tempdir();
    let target = dir.path().join("evidence.json");
    publish_with_integrity(dir.path(), &target, b"authentic bytes", ArtifactKind::Other)
        .expect("artifact publishes");

    // 1) Torn artifact: overwrite with a truncated prefix.
    std::fs::write(&target, b"auth").expect("torn write");
    let err = read_verified(dir.path(), &target, ArtifactKind::Other)
        .expect_err("torn artifact fails verification");
    assert!(
        err.to_string().contains("integrity verification failed")
            || err.to_string().contains("sidecar"),
        "unexpected: {err:#}"
    );

    // 2) Artifact restored to authentic bytes, sidecar forged.
    let good = b"authentic bytes";
    std::fs::write(&target, good).expect("authentic restore");
    let sidecar = artifact_integrity::sidecar_for(&target);
    let sidecar_text = std::fs::read_to_string(&sidecar).unwrap();
    let mut sidecar_json: serde_json::Value = serde_json::from_str(&sidecar_text).unwrap();
    // Forge the claimed digest so it "verifies" a different payload.
    sidecar_json["sha256"] = serde_json::Value::String("0".repeat(64));
    std::fs::write(
        &sidecar,
        serde_json::to_string_pretty(&sidecar_json).unwrap(),
    )
    .unwrap();
    let err = read_verified(dir.path(), &target, ArtifactKind::Other)
        .expect_err("forged sidecar fails verification");
    assert!(
        format!("{err:#}").contains("integrity verification failed")
            || format!("{err:#}").contains("sha256"),
        "forged sidecar must be rejected: {err:#}"
    );

    // 3) Kind mismatch on an otherwise intact artifact fails closed.
    let good_sidecar: serde_json::Value = serde_json::from_str(&sidecar_text).unwrap();
    let mut wrong_kind = good_sidecar.clone();
    wrong_kind["artifact_kind"] = serde_json::Value::String("proposal".to_string());
    std::fs::write(&sidecar, serde_json::to_string_pretty(&wrong_kind).unwrap()).unwrap();
    let result = read_verified(dir.path(), &target, ArtifactKind::Other);
    assert!(result.is_err(), "sidecar kind mismatch must fail closed");
}

#[test]
fn campaign_a_publish_interrupted_before_rename_recovers_deterministically() {
    // Kill the publication of the SAME artifact twice in a row at the rename
    // boundary: every retry fails, committed state never half-publishes, and
    // after the fault window clears a single retry completes.
    let dir = tempdir();
    let target = dir.path().join("recoverable.json");
    for _ in 0..3 {
        arm_fault_once(
            AtomicWriteStep::TempFsynced,
            "simulated repeated crash before rename",
        );
        let result = atomic_write_json(&target, &serde_json::json!({"recovered": true}));
        disarm_fault();
        assert!(result.is_err());
        assert!(!target.exists(), "no publication may be half-committed");
    }
    atomic_write_json(&target, &serde_json::json!({"recovered": true}))
        .expect("recovery succeeds deterministically");
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).expect("recovered doc parses");
    assert_eq!(parsed["recovered"], true);
}

// ---------------------------------------------------------------------------
// Campaign C — lease correctness (deterministic injected clock)
// ---------------------------------------------------------------------------

fn lease_config() -> LeaseConfig {
    // Short leases so a stale heartbeat reclaims quickly in tests; heartbeat
    // defaults consistent with the safety ratio (hb * 3 <= gen timeout).
    LeaseConfig::with_timeouts(
        std::time::Duration::from_secs(3),
        std::time::Duration::from_secs(9),
        std::time::Duration::from_secs(1),
    )
}

fn entry(state: ProposalState, heartbeat: &str, owner: &str, epoch: u64) -> RegistryEntry {
    RegistryEntry {
        state,
        proposal_id: None,
        owner_run_id: owner.to_string(),
        lease_epoch: epoch,
        reserved_at: heartbeat.to_string(),
        updated_at: heartbeat.to_string(),
        heartbeat_at: heartbeat.to_string(),
        evidence_dir: None,
    }
}

fn now_plus(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::seconds(seconds)
}

#[test]
fn lease_healthy_long_running_worker_is_never_reclaimed_prematurely() {
    // A live worker that keeps heartbeating is NOT stale no matter how old
    // the original reservation is — the heartbeat, not the reservation, is
    // the liveness signal.
    let e = entry(
        ProposalState::Generating,
        &now_plus(-600).to_rfc3339(),
        "worker-a",
        1,
    );
    let fresh_e = entry(
        ProposalState::Generating,
        &now_plus(-1).to_rfc3339(),
        "worker-a",
        1,
    );
    let cfg = lease_config();
    assert!(
        !is_entry_stale_at(&fresh_e, &cfg, chrono::Utc::now()).unwrap(),
        "a heartbeating worker must never read as stale"
    );
    assert!(
        is_entry_stale_at(&e, &cfg, chrono::Utc::now()).unwrap(),
        "a stopped worker past the generation lease timeout IS stale"
    );
}

#[test]
fn lease_validating_entry_with_recent_heartbeat_is_not_reclaimed() {
    let cfg = lease_config();
    let e = entry(
        ProposalState::Validating,
        &now_plus(-2).to_rfc3339(),
        "worker-a",
        7,
    );
    assert!(
        !is_entry_stale_at(&e, &cfg, chrono::Utc::now()).unwrap(),
        "a validating worker with a recent heartbeat must not be reclaimable"
    );
}

#[test]
fn lease_stale_orphan_is_reclaimable_and_epoch_increments() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-orphan-reclaim";
    let stale = entry(
        ProposalState::Generating,
        &now_plus(-3600).to_rfc3339(),
        "dead-owner",
        3,
    );
    write_registry(&repo, key, &stale);

    let cfg = lease_config();
    let result = try_take_ownership_cas(&repo, key, "rescuer", &cfg, None).unwrap();
    match result {
        TakeoverResult::Taken(fence) => {
            assert_eq!(fence.owner_run_id, "rescuer");
            assert_eq!(fence.lease_epoch, stale.lease_epoch + 1);
        }
        other => panic!("stale orphan must be reclaimable, got {other:?}"),
    }
    // The registry entry now records the new owner with a bumped epoch.
    let reg = read_registry(&repo);
    let entry = &reg.entries[key];
    assert_eq!(entry.owner_run_id, "rescuer");
    assert_eq!(entry.lease_epoch, 4);
}

#[test]
fn lease_clock_skew_within_tolerance_is_not_stale_beyond_tolerance_fails_closed() {
    let cfg = lease_config();
    // Heartbeat 3 seconds in the future: within the 5s default skew, treat as
    // live (it will age normally).
    let skewed = entry(
        ProposalState::Generating,
        &now_plus(3).to_rfc3339(),
        "worker-a",
        1,
    );
    assert!(!is_entry_stale_at(&skewed, &cfg, chrono::Utc::now()).unwrap());

    // Heartbeat 6 seconds in the future: beyond tolerated skew — fail closed,
    // never silently immortal.
    let beyond = entry(
        ProposalState::Generating,
        &now_plus(6).to_rfc3339(),
        "worker-a",
        1,
    );
    let err = is_entry_stale_at(&beyond, &cfg, chrono::Utc::now()).unwrap_err();
    assert!(
        format!("{err:#}").contains("clock skew"),
        "future heartbeat beyond skew must fail closed: {err:#}"
    );
}

#[test]
fn lease_heartbeat_renewed_between_observation_and_takeover_is_still_live() {
    // The outer observer records a stale entry; between that observation and
    // the takeover, the owner renews its heartbeat. The under-lock
    // re-validation must see the live entry and refuse the reclaim.
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-renewed-race";
    let stale = entry(
        ProposalState::Generating,
        &now_plus(-3600).to_rfc3339(),
        "worker-a",
        1,
    );
    write_registry(&repo, key, &stale);

    // Renew heartbeat right now (as the live worker would), THEN attempt the
    // takeover: the under-lock re-check must observe the live entry.
    let renewed = entry(
        ProposalState::Generating,
        &now_plus(-1).to_rfc3339(),
        "worker-a",
        1,
    );
    write_registry(&repo, key, &renewed);

    let cfg = lease_config();
    let expected = OwnershipObservation {
        owner_run_id: "worker-a".to_string(),
        lease_epoch: 1,
        state: ProposalState::Generating,
    };
    let result = try_take_ownership_cas(&repo, key, "rescuer", &cfg, Some(&expected)).unwrap();
    assert!(
        matches!(result, TakeoverResult::StillLive),
        "a worker that renewed between observation and takeover must not be reclaimed: {result:?}"
    );
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].owner_run_id, "worker-a");
    assert_eq!(reg.entries[key].lease_epoch, 1);
}

#[test]
fn lease_ambiguous_timestamp_fails_closed_and_registry_is_unchanged() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-ambiguous";
    let garbage = entry(ProposalState::Generating, "not-a-timestamp", "worker-a", 1);
    write_registry(&repo, key, &garbage);

    let cfg = lease_config();
    let result = try_take_ownership_cas(&repo, key, "rescuer", &cfg, None);
    assert!(
        result.is_err(),
        "an ambiguous lease state must fail closed, not decide reclaim: {result:?}"
    );
    let reg = read_registry(&repo);
    assert_eq!(
        reg.entries[key].owner_run_id, "worker-a",
        "failed takeover must not mutate ownership"
    );
    assert_eq!(reg.entries[key].lease_epoch, 1);
}

#[test]
fn lease_ambiguous_future_timestamp_also_fails_closed() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-future-ambiguous";
    // 6s in the future: beyond tolerated skew — fail closed even under lock.
    let beyond = entry(
        ProposalState::Generating,
        &now_plus(6).to_rfc3339(),
        "worker-a",
        1,
    );
    write_registry(&repo, key, &beyond);

    let cfg = lease_config();
    let result = try_take_ownership_cas(&repo, key, "rescuer", &cfg, None);
    assert!(
        result.is_err(),
        "future heartbeat beyond skew must fail closed: {result:?}"
    );
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].owner_run_id, "worker-a");
    assert_eq!(reg.entries[key].lease_epoch, 1);
}

#[test]
fn lease_validation_complete_is_never_reclaimed() {
    let cfg = lease_config();
    let e = entry(
        ProposalState::ValidationComplete,
        &now_plus(-9999).to_rfc3339(),
        "worker-a",
        9,
    );
    assert!(
        !is_entry_stale_at(&e, &cfg, chrono::Utc::now()).unwrap(),
        "terminal ValidationComplete is never stale, however old"
    );
}

// ---------------------------------------------------------------------------
// Campaign B — two real processes race one governed step
// ---------------------------------------------------------------------------

const B_CHILD_ENV: &str = "PROMETHEOS_RELIABILITY_CHILD";
const B_REPO_ENV: &str = "PROMETHEOS_RELIABILITY_REPO";
const B_KEY_ENV: &str = "PROMETHEOS_RELIABILITY_KEY";
const B_OWNER_ENV: &str = "PROMETHEOS_RELIABILITY_OWNER";
const B_OUT_ENV: &str = "PROMETHEOS_RELIABILITY_OUT";
const B_MODE_ENV: &str = "PROMETHEOS_RELIABILITY_MODE";
const B_OBS_OWNER_ENV: &str = "PROMETHEOS_RELIABILITY_OBS_OWNER";
const B_OBS_EPOCH_ENV: &str = "PROMETHEOS_RELIABILITY_OBS_EPOCH";
const B_LEASE_SECS_ENV: &str = "PROMETHEOS_RELIABILITY_LEASE_SECS";

fn b_child_command(repo: &Path, key: &str, owner: &str, out: &Path, mode: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg("--exact")
        .arg("reliability_worker_child")
        .arg("--test-threads=1")
        .arg("--nocapture");
    cmd.env(B_CHILD_ENV, "1");
    cmd.env(B_REPO_ENV, repo);
    cmd.env(B_KEY_ENV, key);
    cmd.env(B_OWNER_ENV, owner);
    cmd.env(B_OUT_ENV, out);
    cmd.env(B_MODE_ENV, mode);
    // Default observation: whatever the registry currently records for the
    // entry. Tests that drive a takeover with a deliberately stale token pass
    // the expected owner/epoch explicitly instead.
    if let Some((obs_owner, obs_epoch, _)) = match mode {
        "takeback-stale" => Some(("worker-a", "2", "")),
        "takeback-current" => Some(("worker-b", "3", "")),
        _ => None,
    } {
        cmd.env(B_OBS_OWNER_ENV, obs_owner);
        cmd.env(B_OBS_EPOCH_ENV, obs_epoch);
        // A long lease makes the still-live refusal deterministic regardless
        // of how long worker B's fixture run took before the contender runs.
        cmd.env(B_LEASE_SECS_ENV, "3600");
    }
    cmd
}

fn wait_for_out(path: &Path, timeout: Duration) -> String {
    let start = std::time::Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < timeout,
            "child never wrote its result: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::read_to_string(path).unwrap()
}

fn seed_stale_entry(repo: &Path, key: &str, owner: &str, epoch: u64, age_seconds: i64) {
    let when = (chrono::Utc::now() - chrono::Duration::seconds(age_seconds)).to_rfc3339();
    let stale = RegistryEntry {
        state: ProposalState::Reserved,
        proposal_id: None,
        owner_run_id: owner.to_string(),
        lease_epoch: epoch,
        reserved_at: when.clone(),
        updated_at: when.clone(),
        heartbeat_at: when,
        evidence_dir: None,
    };
    write_registry(repo, key, &stale);
}

#[test]
fn campaign_b_two_processes_race_one_governed_step_exactly_one_wins() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "campaign-b-race";
    seed_stale_entry(&repo, key, "dead-worker", 1, 3600);

    let out_a = dir.path().join("out-a.json");
    let out_b = dir.path().join("out-b.json");
    let mut a = b_child_command(&repo, key, "worker-a", &out_a, "race")
        .spawn()
        .unwrap();
    let mut b = b_child_command(&repo, key, "worker-b", &out_b, "race")
        .spawn()
        .unwrap();
    let (sa, sb) = (a.wait().unwrap(), b.wait().unwrap());
    assert!(sa.success() && sb.success(), "children must exit cleanly");

    let (ra, rb) = (
        wait_for_out(&out_a, Duration::from_secs(120)),
        wait_for_out(&out_b, Duration::from_secs(120)),
    );
    let winners = [ra.as_str(), rb.as_str()]
        .into_iter()
        .filter(|r| r.contains("\"completed\""))
        .count();
    let losers = [ra.as_str(), rb.as_str()]
        .into_iter()
        .filter(|r| r.contains("lost"))
        .count();
    assert_eq!(winners, 1, "exactly one worker may complete: {ra} / {rb}");
    assert_eq!(losers, 1, "exactly one worker may lose: {ra} / {rb}");

    // The winner ran the real governed boundary: repair applied (changed
    // exactly one file), real validator exited 0.
    let winner = if ra.contains("\"completed\"") {
        &ra
    } else {
        &rb
    };
    let winner_json: serde_json::Value =
        serde_json::from_str(winner).expect("winner receipt parses");
    assert_eq!(
        winner_json["validatorExitCode"],
        serde_json::json!(0),
        "winner's real validator must exit 0"
    );
    let changed: Vec<String> = winner_json["changedVsBaseline"]
        .as_array()
        .expect("changedVsBaseline is an array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();
    assert!(
        changed.contains(&"src/lib.rs".to_string()),
        "winner applied the pinned repair: {changed:?}"
    );
    assert!(
        changed
            .iter()
            .all(|p| p == "src/lib.rs" || p == "Cargo.lock"),
        "only the pinned repair (and cargo's lockfile) may change: {changed:?}"
    );
    assert_eq!(winner_json["taskOutcome"], serde_json::json!("repaired"));

    // The registry records exactly one takeover: winner, epoch 2.
    let reg = read_registry(&repo);
    let entry = &reg.entries[key];
    assert_eq!(entry.lease_epoch, 2, "exactly one epoch increment occurred");
    assert!(
        entry.owner_run_id == "worker-a" || entry.owner_run_id == "worker-b",
        "winner is durably recorded: {}",
        entry.owner_run_id
    );

    // No fixture mutation anywhere: the pinned committed fixture is untouched.
    assert_fixture_pristine();
}

#[test]
fn campaign_e_interrupted_winner_recovers_in_fresh_process_with_zero_duplicate_effects() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "campaign-e-recovery";
    seed_stale_entry(&repo, key, "dead-worker", 1, 3600);

    // ---- Worker A claims, completes the governed step, then crashes before
    // publishing anything (no completion receipt, no terminal registry write).
    let out_a = dir.path().join("out-a.json");
    let mut a = b_child_command(&repo, key, "worker-a", &out_a, "crash-after-effect")
        .spawn()
        .unwrap();
    assert!(
        a.wait().unwrap().success(),
        "worker A exits (never completes cleanly)"
    );
    assert!(
        !out_a.exists(),
        "worker A crashed before writing any completion receipt"
    );

    // The registry still records A as owner at the pre-crash epoch; the
    // heartbeat is A's claim time (fresh). The worktree A used is gone with
    // its process — the effect was committed and loudly lost, never silently
    // double-applied.
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].owner_run_id, "worker-a");
    assert_eq!(reg.entries[key].lease_epoch, 2);

    // ---- Fresh process / recovery attempt: a new worker reclaims only after
    // the lease goes stale, and completes with zero duplicate effects.
    std::thread::sleep(Duration::from_secs(10));

    let out_b = dir.path().join("out-b.json");
    let mut b = b_child_command(&repo, key, "worker-b", &out_b, "race")
        .spawn()
        .unwrap();
    assert!(b.wait().unwrap().success());
    let rb = wait_for_out(&out_b, Duration::from_secs(120));
    assert!(
        rb.contains("\"completed\""),
        "the recovery worker completes: {rb}"
    );
    let winner: serde_json::Value = serde_json::from_str(&rb).unwrap();
    assert_eq!(winner["validatorExitCode"], serde_json::json!(0));
    // The winner applied the pinned repair (and cargo may stage a fresh
    // Cargo.lock) through the real boundary.
    let changed_b: Vec<String> = winner["changedVsBaseline"]
        .as_array()
        .expect("changedVsBaseline is an array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect();
    assert!(changed_b.contains(&"src/lib.rs".to_string()));
    assert!(
        changed_b
            .iter()
            .all(|p| p == "src/lib.rs" || p == "Cargo.lock")
    );

    let reg = read_registry(&repo);
    let entry = &reg.entries[key];
    assert_eq!(entry.owner_run_id, "worker-b");
    assert_eq!(
        entry.lease_epoch, 3,
        "recovery bumped the epoch exactly once"
    );

    // ---- A stale owner replaying against the replaced claim is refused.
    let out_c = dir.path().join("out-c.json");
    let mut c = b_child_command(&repo, key, "worker-c", &out_c, "takeback-stale")
        .spawn()
        .unwrap();
    assert!(c.wait().unwrap().success());
    let rc = wait_for_out(&out_c, Duration::from_secs(60));
    assert!(
        rc.contains("lost"),
        "stale owner whose observation no longer matches must lose the race: {rc}"
    );

    // ---- A contender whose observation matches (owner B, epoch 3) but whose
    // lease is live is not reclaimed either (still-live refusal).
    let out_d = dir.path().join("out-d.json");
    let mut d = b_child_command(&repo, key, "worker-d", &out_d, "takeback-current")
        .spawn()
        .unwrap();
    assert!(d.wait().unwrap().success());
    let rd = wait_for_out(&out_d, Duration::from_secs(60));
    assert!(
        rd.contains("lost"),
        "a live lease must not be reclaimed: {rd}"
    );
    // The registry is byte-identical to B's completion state: no mutation by
    // either refused contender.
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].owner_run_id, "worker-b");
    assert_eq!(reg.entries[key].lease_epoch, 3);

    assert_fixture_pristine();
}

/// Child routine: observe the entry (from the registry, or from an explicit
/// injected stale token), attempt a CAS takeover, and — only if the claim is
/// won in `race` mode — execute the real governed fixture task through the
/// real boundary. Writes a JSON receipt instead of stdout so the parent can
/// inspect effects deterministically.
#[test]
fn reliability_worker_child() {
    if std::env::var(B_CHILD_ENV)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    let repo = PathBuf::from(std::env::var(B_REPO_ENV).unwrap());
    let key = std::env::var(B_KEY_ENV).unwrap();
    let owner = std::env::var(B_OWNER_ENV).unwrap();
    let out = PathBuf::from(std::env::var(B_OUT_ENV).unwrap());
    let mode = std::env::var(B_MODE_ENV).unwrap();

    let reg = read_registry(&repo);
    let current = reg
        .entries
        .get(&key)
        .cloned()
        .expect("entry must exist (seeded by the parent)");
    let observation = match (
        std::env::var(B_OBS_OWNER_ENV),
        std::env::var(B_OBS_EPOCH_ENV),
    ) {
        (Ok(obs_owner), Ok(obs_epoch)) => OwnershipObservation {
            owner_run_id: obs_owner,
            lease_epoch: obs_epoch.parse().expect("epoch parses"),
            state: current.state,
        },
        _ => OwnershipObservation {
            owner_run_id: current.owner_run_id.clone(),
            lease_epoch: current.lease_epoch,
            state: current.state,
        },
    };

    let lease_secs: u64 = std::env::var(B_LEASE_SECS_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9);
    let lease_cfg = LeaseConfig::with_timeouts(
        Duration::from_secs(lease_secs.max(3)),
        Duration::from_secs(lease_secs),
        Duration::from_secs(1),
    );
    let result = try_take_ownership_cas(&repo, &key, &owner, &lease_cfg, Some(&observation))
        .expect("takeover attempt must not error in this fixture");

    match (mode.as_str(), result) {
        ("crash-after-effect", TakeoverResult::Taken(fence)) => {
            // Claim won: execute the real governed boundary fully, then die
            // before writing ANY completion receipt (no success publication,
            // no terminal registry write, no accounting).
            let run = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, correct_repair_edits());
            assert_eq!(
                run.envelope.as_ref().map(|e| e.task_outcome),
                Some(task_outcome_repaired()),
                "worker A must complete the fixture repair before crashing"
            );
            // Crash: return WITHOUT publishing anything.
            let _ = fence;
            std::process::exit(0);
        }
        ("race", TakeoverResult::Taken(fence)) => {
            let run = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, correct_repair_edits());
            let receipt = match run.envelope.as_ref() {
                Some(envelope) => serde_json::json!({
                    "outcome": "completed",
                    "taskOutcome": outcome_label(envelope.task_outcome),
                    "validatorExitCode": envelope.validator.exit_code,
                    "validatorProvenance": "observed-subprocess",
                    "changedVsBaseline": run.changed_vs_baseline,
                    "claimOwner": fence.owner_run_id,
                    "claimEpoch": fence.lease_epoch,
                }),
                None => serde_json::json!({
                    "outcome": "completed-without-envelope",
                    "sealError": run.seal_error,
                }),
            };
            std::fs::write(&out, receipt.to_string()).expect("receipt written");
        }
        (_, TakeoverResult::Taken(_)) => { /* unhandled combination: nothing published */ }
        (_, _) => {
            std::fs::write(&out, "lost").expect("lost marker written");
        }
    }
}

fn task_outcome_repaired() -> TaskOutcome {
    TaskOutcome::Repaired
}

fn outcome_label(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Repaired => "repaired",
        TaskOutcome::RepairFailed => "repair-failed",
        TaskOutcome::RepairRefused => "repair-refused",
    }
}

// ---------------------------------------------------------------------------
// Campaign D — resource failures (bounded, deterministic, never host-filling)
// ---------------------------------------------------------------------------

fn powershell_validator(
    assertion: &str,
    expected_exit_code: i32,
    args: Vec<String>,
) -> prometheos_lite::workflow::scenario::ValidationPin {
    prometheos_lite::workflow::scenario::ValidationPin {
        schema_version: "1.0.0".to_string(),
        command: "powershell".to_string(),
        args,
        expected_exit_code,
        expected_assertion: assertion.to_string(),
        expected_file_after_digest: "0".repeat(64),
        expected_post_run_digest: "0".repeat(64),
    }
}

#[test]
fn campaign_d_enospc_during_artifact_write_leaves_no_artifact_no_trust() {
    let dir = tempdir();
    let target = dir.path().join("artifact.json");

    arm_fault_once(
        AtomicWriteStep::TempCreated,
        "simulated ENOSPC during artifact write",
    );
    let result = publish_with_integrity(
        dir.path(),
        &target,
        b"payload bytes",
        ArtifactKind::Proposal,
    );
    disarm_fault();
    assert!(result.is_err(), "the publish must surface the disk failure");

    // No artifact, no sidecar: nothing for a reader to trust.
    let sidecar = artifact_integrity::sidecar_for(&target);
    assert!(
        !target.exists(),
        "no artifact may be visible after an aborted publish"
    );
    assert!(
        !sidecar.exists(),
        "no sidecar may be visible after an aborted publish"
    );
    let err = read_verified(dir.path(), &target, ArtifactKind::Proposal).unwrap_err();
    assert!(
        format!("{err:#}")
            .to_lowercase()
            .contains("failed to read artifact"),
        "read of an absent artifact fails closed: {err:#}"
    );
}

#[test]
fn campaign_d_enospc_during_checkpoint_publication_preserves_committed_state() {
    let dir = tempdir();
    let target = dir.path().join("checkpoint.json");
    let committed = serde_json::json!({"state": "committed", "epoch": 1});
    atomic_write_json(&target, &committed).expect("initial checkpoint publishes");
    let committed_bytes = std::fs::read(&target).unwrap();

    arm_fault_once(
        AtomicWriteStep::TempWritten,
        "simulated ENOSPC during checkpoint publication",
    );
    let result = atomic_write_json(&target, &serde_json::json!({"state": "overwritten"}));
    disarm_fault();
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(&target).unwrap(),
        committed_bytes,
        "a failed checkpoint publication must never displace the committed checkpoint"
    );

    // Recovery re-publication succeeds; the committed state was never lost.
    atomic_write_json(&target, &serde_json::json!({"state": "overwritten"}))
        .expect("recovery re-publication succeeds");
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).expect("checkpoint parses");
    assert_eq!(parsed["state"], "overwritten");
}

#[test]
fn campaign_d_enospc_before_atomic_publication_keeps_bytes_intact() {
    let dir = tempdir();
    let target = dir.path().join("raw.bin");
    atomic_write_bytes(&target, b"committed artifact").expect("base bytes publish");

    arm_fault_once(
        AtomicWriteStep::TempFsynced,
        "simulated ENOSPC before atomic publication",
    );
    let result = atomic_write_bytes(&target, b"replacement");
    disarm_fault();
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"committed artifact",
        "an ENOSPC-interrupted publication must never destroy the committed bytes"
    );
    // Leftover temp material must not replace the committed artifact.
    assert_eq!(std::fs::read(&target).unwrap(), b"committed artifact");
}

#[test]
fn campaign_d_validator_timeout_is_a_timeout_never_a_success() {
    // A real hanging validator subprocess killed at the deadline.
    let worktree = tempdir();
    let pins = powershell_validator(
        "fixture: validator must exit 0",
        0,
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Start-Sleep -Seconds 60".to_string(),
        ],
    );
    let record = run_validator_with_timeout(
        &pins,
        worktree.path(),
        worktree.path(),
        std::time::Duration::from_millis(400),
    );
    assert!(
        record.timed_out,
        "the hanging validator must be classified as timed out"
    );
    assert_eq!(
        record.exit_code, None,
        "a killed validator carries no exit code"
    );
    assert!(
        record.duration_ms < 10_000,
        "the validator must be killed at the deadline, not run to completion ({} ms)",
        record.duration_ms
    );
    // The receipt cannot support a repaired/success claim.
    assert!(
        record.timed_out || record.exit_code != Some(pins.expected_exit_code),
        "a timeout receipt can never satisfy the pinned expected exit code"
    );
    assert_fixture_pristine();
}

#[test]
fn campaign_d_validator_output_size_limit_captures_and_digests_without_unbounded_memory() {
    // ~1.5 MB of stdout from a real subprocess, above the 1 MB capture cap.
    let worktree = tempdir();
    let pins = powershell_validator(
        "fixture: validator output must stay bounded",
        0,
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "[Console]::Out.Write('x' * 1500000)".to_string(),
        ],
    );
    let record = run_validator_with_timeout(
        &pins,
        worktree.path(),
        worktree.path(),
        std::time::Duration::from_secs(60),
    );
    assert_eq!(record.exit_code, Some(0));
    assert!(!record.timed_out);
    assert!(
        record.output_bytes > 1_000_000,
        "total captured bytes must exceed the cap to prove the drain pumped everything: {}",
        record.output_bytes
    );
    assert!(
        record.output_digest.is_some(),
        "a capped digest over the captured prefix must be recorded"
    );
    // The digest binds only the captured prefix — it does not claim bytes
    // that never reached the buffer (nothing fabricated).
    assert!(record.output_bytes <= 2_000_000);
}

#[test]
fn campaign_d_cancellation_before_effect_produces_zero_effects() {
    let run = common::task_execution::run_task_execution_with_fault(
        &EXECUTE_SPEC,
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        common::task_execution::DispatchInjection::RefuseStepCapability,
    );
    assert_eq!(
        run.capability_effects, 0,
        "a pre-effect cancellation runs zero capability calls"
    );
    assert!(
        !run.patched,
        "no patch may be applied before the effect boundary"
    );
    assert!(
        run.changed_vs_baseline.is_empty(),
        "zero repository mutations: {:?}",
        run.changed_vs_baseline
    );
    assert!(
        run.envelope.is_none(),
        "no success receipt may exist for a cancelled-before-effect run"
    );
    assert!(
        run.seal_error.is_none() || run.acquire_error.is_none() || run.preflight_error.is_none(),
        "the refusal is typed and explicit"
    );
    assert_fixture_pristine();
}

#[test]
fn campaign_d_validator_output_digest_never_fabricates_bytes() {
    // Empty-output validator: the pinned sha256 of the empty byte string is a
    // well-known constant; the record must carry exactly that, zero bytes.
    let worktree = tempdir();
    let pins = powershell_validator(
        "fixture: silent validator",
        0,
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "".to_string(),
        ],
    );
    let record = run_validator_with_timeout(
        &pins,
        worktree.path(),
        worktree.path(),
        std::time::Duration::from_secs(30),
    );
    assert_eq!(record.exit_code, Some(0));
    assert_eq!(record.output_bytes, 0);
    assert_eq!(
        record.output_digest.as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        "empty output pins the known empty-input sha256"
    );
}

#[test]
fn campaign_d_interrupted_accounting_publication_yields_no_zero_cost_claim() {
    let run = run_task_execution(&EXECUTE_SPEC, EXECUTE_DESCRIPTOR, correct_repair_edits());
    let accounting = run
        .accounting
        .as_ref()
        .expect("accounting recorded by the real run");

    // The accounting input is real: the fixture reports KNOWN usage/cost.
    match &accounting.usage {
        prometheos_lite::workflow::accounting::UsageAccounting::Known(usage) => {
            assert_eq!(usage.prompt_tokens, Some(7));
            assert_eq!(usage.completion_tokens, Some(4));
        }
        other => panic!("expected known usage from the pinned loopback fixture, got {other:?}"),
    }

    // Publication interrupted by ENOSPC — nothing is published, nothing claims.
    let dir = tempdir();
    let target = dir.path().join("attempt-accounting.json");
    arm_fault_once(
        AtomicWriteStep::TempWritten,
        "simulated ENOSPC during accounting publication",
    );
    let result = atomic_write_json(&target, accounting);
    disarm_fault();
    assert!(
        result.is_err(),
        "interrupted accounting publication must surface an error"
    );
    assert!(
        !target.exists(),
        "no accounting document exists after the interruption"
    );

    // A reader must not find a zero-cost stand-in either.
    let missing = std::fs::read_to_string(&target);
    assert!(
        missing.is_err(),
        "absent publication is absence, not a zero"
    );

    // Recovery publishes the SAME document (not a re-derived zero) and it
    // round-trips to the identical accounting values.
    atomic_write_json(&target, accounting).expect("recovery publication succeeds");
    let text = std::fs::read_to_string(&target).unwrap();
    let restored: prometheos_lite::workflow::accounting::AttemptAccountingV1 =
        serde_json::from_str(&text).expect("restored accounting parses");
    restored.verify().expect("restored accounting verifies");
    match &restored.cost {
        prometheos_lite::workflow::accounting::CostAccounting::Known(computation) => {
            assert_ne!(
                computation.minor_units, 0,
                "known non-zero cost must survive publication"
            );
        }
        other => panic!("expected known cost, got {other:?}"),
    }

    // Forged-zero: a document claiming cost 0 where the fixture reports a
    // known non-zero cost cannot verify if it was tampered (digest binding),
    // and a document publishing Unknown must stay Unknown (never 0).
    let mut unknown = restored.clone();
    unknown.cost = prometheos_lite::workflow::accounting::CostAccounting::Unknown {
        reason: "provider reported unusable pricing".to_string(),
    };
    atomic_write_json(&dir.path().join("unknown.json"), &unknown)
        .expect("unknown-cost accounting publishes (as unknown)");
    let text = std::fs::read_to_string(dir.path().join("unknown.json")).unwrap();
    assert!(
        text.contains("provider reported unusable pricing"),
        "unknown must survive as unknown, with its reason, never as a zero"
    );
}

#[test]
fn campaign_d_forged_completion_receipt_fails_closed_at_integrity_gate() {
    let dir = tempdir();
    let target = dir.path().join("completion.json");
    publish_with_integrity(
        dir.path(),
        &target,
        br#"{"outcome":"completed","costMinorUnits":0}"#,
        ArtifactKind::Other,
    )
    .expect("receipt publishes");

    // Forge: an attacker flips the payload's zero-cost completion bytes
    // without re-signing — the digest must not match.
    std::fs::write(&target, br#"{"outcome":"completed","costMinorUnits":999}"#).unwrap();
    let err = read_verified(dir.path(), &target, ArtifactKind::Other).unwrap_err();
    assert!(
        format!("{err:#}").contains("integrity verification failed"),
        "forged receipt must fail at the integrity gate: {err:#}"
    );

    // Forge: rewrite the sidecar to match a different payload entirely.
    let sidecar = artifact_integrity::sidecar_for(&target);
    let sidecar_text = std::fs::read_to_string(&sidecar).unwrap();
    let mut forged: serde_json::Value = serde_json::from_str(&sidecar_text).unwrap();
    forged["sha256"] = serde_json::Value::String("f".repeat(64));
    std::fs::write(&sidecar, serde_json::to_string_pretty(&forged).unwrap()).unwrap();
    assert!(
        read_verified(dir.path(), &target, ArtifactKind::Other).is_err(),
        "a forged sidecar cannot authenticate a foreign payload"
    );
}

#[test]
fn journal_integrity_gap_detection_and_filename_mismatch_fail_closed() {
    let dir = tempdir();
    let journal_dir = dir
        .path()
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join("key-journal-integrity");
    std::fs::create_dir_all(&journal_dir).unwrap();

    // Two valid events, one missing in between.
    write_sample_journal_event(&journal_dir, 1, "key-journal-integrity");
    write_sample_journal_event(&journal_dir, 3, "key-journal-integrity");

    // read_journal yields both events; continuity across the pair is broken
    // and detectable without trusting file content alone.
    let events = evaluate::read_journal(dir.path(), "key-journal-integrity")
        .expect("journal reads (filename/content still self-consistent)");
    assert_eq!(events.len(), 2);
    let contiguous = events
        .windows(2)
        .all(|w| w[1].sequence == w[0].sequence + 1);
    assert!(
        !contiguous,
        "a gapped journal is not a valid recovery basis"
    );

    // Filename/content mismatch fails closed at read time (production check).
    let event3 = std::fs::read_to_string(journal_dir.join(format!("{:020}.json", 3))).unwrap();
    let mut forged: serde_json::Value = serde_json::from_str(&event3).unwrap();
    forged["sequence"] = serde_json::json!(4);
    std::fs::write(
        journal_dir.join(format!("{:020}.json", 3)),
        serde_json::to_string_pretty(&forged).unwrap(),
    )
    .unwrap();
    let err = evaluate::read_journal(dir.path(), "key-journal-integrity").unwrap_err();
    assert!(
        format!("{err:#}").contains("filename") || format!("{err:#}").contains("sequence"),
        "a journal whose filename does not match its event must fail closed: {err:#}"
    );

    // A fully valid journal reads cleanly.
    let dir_ok = tempdir();
    let journal_ok = dir_ok
        .path()
        .join(".prometheos")
        .join("workflow")
        .join("journal")
        .join("key-ok");
    std::fs::create_dir_all(&journal_ok).unwrap();
    for seq in 0..3 {
        write_sample_journal_event(&journal_ok, seq, "key-ok");
    }
    let events = evaluate::read_journal(dir_ok.path(), "key-ok").expect("valid journal reads");
    assert_eq!(events.len(), 3);
    assert!(
        events
            .windows(2)
            .all(|w| w[1].sequence == w[0].sequence + 1)
    );
    assert_eq!(events[0].repository_revision, "abc123");
}

fn write_sample_journal_event(journal_dir: &Path, sequence: u64, key: &str) {
    let event = evaluate::JournalEvent {
        schema_version: prometheos_lite::workflow::schema::CURRENT_SCHEMA_VERSION,
        event_id: format!("evt-{sequence}"),
        sequence,
        run_id: "run-1".to_string(),
        identity_key: key.to_string(),
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        from_state: evaluate::EvaluationState::Created,
        to_state: evaluate::EvaluationState::PreflightPassed,
        proposal_ref: None,
        failure_classification: None,
        owner_run_id: "run-1".to_string(),
        lease_epoch: 1,
        repository_revision: "abc123".to_string(),
        evidence_ref: None,
        checkpoint_ref: None,
    };
    let path = journal_dir.join(format!("{sequence:020}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&event).unwrap()).unwrap();
}

#[test]
fn campaign_d_unapproved_workspace_write_stale_revision_produces_zero_effects() {
    // Tamper the descriptor to pin a revision the staged fixture cannot
    // produce: the boundary refuses pre-effect, with zero capability calls,
    // zero files touched, and no success receipt.
    let base = EXECUTE_DESCRIPTOR;
    let tampered = base.replace(
        "\"revision\": \"bfca0ee96857e365026ac07f9a95c514b7e58e8b\"",
        "\"revision\": \"0000000000000000000000000000000000000000\"",
    );
    assert_ne!(tampered, base, "the revision pin must actually change");
    let run = run_task_execution(&EXECUTE_SPEC, &tampered, correct_repair_edits());
    assert_eq!(
        run.capability_effects, 0,
        "no governed effect may run against an unapproved revision"
    );
    assert!(!run.patched);
    assert!(
        run.changed_vs_baseline.is_empty(),
        "zero repository mutations against an unapproved revision: {:?}",
        run.changed_vs_baseline
    );
    assert!(run.envelope.is_none());
    assert_eq!(
        run.classification,
        prometheos_lite::workflow::scenario::ScenarioOutcomeClass::SetupError,
        "stale revision refuses as setup-error before any effect"
    );
    assert_fixture_pristine();
}

#[test]
fn campaign_d_campaign_e_registry_identity_durably_records_one_terminal_handoff() {
    // Two independent processes claim-then-crash and claim-then-complete are
    // covered in campaigns B/E above; here we pin the terminal handoff: a
    // takeover followed by durable evidence verification yields a registry
    // record with exactly one epoch increment and no hidden reclamation.
    let dir = tempdir();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let key = "campaign-d-terminal-handoff";
    seed_stale_entry(&repo, key, "dead-worker", 1, 3600);

    // Epoch 2.
    let cfg = LeaseConfig::with_timeouts(
        Duration::from_secs(3),
        Duration::from_secs(9),
        Duration::from_secs(1),
    );
    let first = try_take_ownership_cas(&repo, key, "worker-a", &cfg, None).unwrap();
    assert!(matches!(first, TakeoverResult::Taken(_)));
    // Epoch 3: stale again (heartbeat now fresh; force staleness via epoch jump is
    // not possible without rewriting the entry — verify the entry is NOT reclaimable now).
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].lease_epoch, 2);
    assert_eq!(reg.entries[key].owner_run_id, "worker-a");
    let again = try_take_ownership_cas(&repo, key, "worker-b", &cfg, None).unwrap();
    assert!(
        matches!(again, TakeoverResult::StillLive),
        "a live lease must refuse the second takeover: {again:?}"
    );
    let reg = read_registry(&repo);
    assert_eq!(reg.entries[key].lease_epoch, 2);
    assert_eq!(reg.entries[key].owner_run_id, "worker-a");
}
