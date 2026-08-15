//! Concurrency and single-winner tests (issue #114).
//!
//! Exactly one worker may take ownership of a stale registry entry. In-process
//! exclusion is only provable on Unix (flock is per open file description), so
//! the binding test spawns separate processes on every platform.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use prometheos_lite::workflow::evaluate::{
    LeaseConfig, OwnershipObservation, ProposalRegistry, ProposalState, RegistryEntry,
    TakeoverResult, try_take_ownership_cas,
};

const CHILD_ENV: &str = "PROMETHEOS_TAKEOVER_CHILD";
const REPO_ENV: &str = "PROMETHEOS_TAKEOVER_REPO";
const KEY_ENV: &str = "PROMETHEOS_TAKEOVER_KEY";
const OWNER_ENV: &str = "PROMETHEOS_TAKEOVER_OWNER";
const OUT_ENV: &str = "PROMETHEOS_TAKEOVER_OUT";

fn registry_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json")
}

/// Seed a stale `Reserved` entry so any worker is eligible to take it over.
fn seed_stale_entry(repo: &Path, key: &str) {
    let entry = RegistryEntry {
        state: ProposalState::Reserved,
        proposal_id: None,
        owner_run_id: "stale-owner".to_string(),
        lease_epoch: 1,
        reserved_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
        heartbeat_at: "2020-01-01T00:00:00Z".to_string(),
        evidence_dir: None,
    };
    let reg = ProposalRegistry {
        entries: [(key.to_string(), entry)].into_iter().collect(),
    };
    let path = registry_path(repo);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&reg).unwrap()).unwrap();
}

/// Observation that matches the seeded stale entry, as a contender would record
/// after observing it. The CAS takeover race is decided against this.
fn seeded_observation() -> OwnershipObservation {
    OwnershipObservation {
        owner_run_id: "stale-owner".to_string(),
        lease_epoch: 1,
        state: ProposalState::Reserved,
    }
}

fn takeover_child_command(repo: &Path, key: &str, owner: &str, out: &Path) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg("--exact").arg("child_takeover_routine");
    cmd.arg("--test-threads=1").arg("--nocapture");
    cmd.env(CHILD_ENV, "1");
    cmd.env(REPO_ENV, repo);
    cmd.env(KEY_ENV, key);
    cmd.env(OWNER_ENV, owner);
    cmd.env(OUT_ENV, out);
    cmd
}

fn wait_for_result(path: &Path, timeout: Duration) -> String {
    let start = std::time::Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < timeout,
            "child never wrote a result for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn exactly_one_process_wins_the_takeover() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-takeover";
    seed_stale_entry(&repo, key);

    let out_a = dir.path().join("out-a.txt");
    let out_b = dir.path().join("out-b.txt");
    let mut a = takeover_child_command(&repo, key, "worker-a", &out_a)
        .spawn()
        .unwrap();
    let mut b = takeover_child_command(&repo, key, "worker-b", &out_b)
        .spawn()
        .unwrap();
    let (sa, sb) = (a.wait().unwrap(), b.wait().unwrap());
    assert!(sa.success() && sb.success(), "children must exit cleanly");

    let (ra, rb) = (
        wait_for_result(&out_a, Duration::from_secs(60)),
        wait_for_result(&out_b, Duration::from_secs(60)),
    );
    let winners = [ra.as_str(), rb.as_str()]
        .into_iter()
        .filter(|r| *r == "won")
        .count();
    assert_eq!(
        winners, 1,
        "exactly one worker may win the takeover: {ra} / {rb}"
    );

    // The winning owner must be durably recorded with a bumped epoch.
    let text = std::fs::read_to_string(registry_path(&repo)).unwrap();
    let reg: ProposalRegistry = serde_json::from_str(&text).unwrap();
    let entry = &reg.entries[key];
    assert_eq!(entry.lease_epoch, 2, "epoch must increment exactly once");
    assert!(
        entry.owner_run_id == "worker-a" || entry.owner_run_id == "worker-b",
        "winner must be recorded: {}",
        entry.owner_run_id
    );
}

#[cfg(unix)]
#[test]
fn exactly_one_thread_wins_the_takeover() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let key = "key-threads";
    seed_stale_entry(&repo, key);

    let repo = Arc::new(repo);
    let lease = Arc::new(LeaseConfig::default());
    let mut handles = Vec::new();
    for i in 0..8 {
        let repo = repo.clone();
        let lease = lease.clone();
        handles.push(std::thread::spawn(move || {
            try_take_ownership_cas(
                &repo,
                key,
                &format!("thread-{i}"),
                &lease,
                Some(&seeded_observation()),
            )
            .unwrap()
        }));
    }
    let results: Vec<TakeoverResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let taken = results
        .iter()
        .filter(|r| matches!(r, TakeoverResult::Taken(_)))
        .count();
    assert_eq!(taken, 1, "exactly one thread may win the takeover");
}

/// Child routine: one-shot takeover attempt of the seeded stale entry.
/// Records "won" or "lost" into the result file. No-op when not spawned.
#[test]
fn child_takeover_routine() {
    if std::env::var(CHILD_ENV)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    let repo = PathBuf::from(std::env::var(REPO_ENV).unwrap());
    let key = std::env::var(KEY_ENV).unwrap();
    let owner = std::env::var(OWNER_ENV).unwrap();
    let out = PathBuf::from(std::env::var(OUT_ENV).unwrap());
    let result = match try_take_ownership_cas(
        &repo,
        &key,
        &owner,
        &LeaseConfig::default(),
        Some(&seeded_observation()),
    )
    .unwrap()
    {
        TakeoverResult::Taken(_) => "won",
        _ => "lost",
    };
    std::fs::write(out, result).unwrap();
}
