//! Issue #136 Slice B — one genuinely executed repository-task fixture.
//!
//! Slice A pinned the fixture bytes and ran the governed boundary with a
//! loopback model, but it never applied a patch to the pinned repository or
//! validated the repair. This suite executes ONE real, deterministic
//! repository task end to end, in a load-bearing preflight order:
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

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;
use prometheos_lite::workflow::accounting::{
    AccountingContext, AccountingLedger, AttemptAccountingV1, PricingBook, account_routed_outcome,
};
use prometheos_lite::workflow::execution_harness::HarnessRegistry;
use prometheos_lite::workflow::local_model_harness::{
    LOCAL_MODEL_INFER_CAPABILITY, LocalModelConfig, LocalModelHarness, local_model_capability,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::route_dispatch::{
    FixedTrustedTimeSource, RoutedDispatchFailure, RoutedDispatchOutcome, RoutedDispatchRequest,
    dispatch_resolved_route,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteResolutionRequest, RouteTrust, RouteWorkspaceBinding, resolve_route,
};
use prometheos_lite::workflow::scenario::{
    self, LiteTaskRunEvidence, ScenarioDescriptor, ScenarioOutcomeClass, ScenarioWorkspaceAdapter,
    TaskOutcome, TaskRepairOrigin, TaskRepairRecord,
};
use prometheos_lite::workflow::soma;
use prometheos_lite::workflow::soma::capability::Locality;
use prometheos_lite::workflow::soma::capability::{
    SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::workspace::{
    self, AdapterKind, GitWorktreeWorkspace, WorkspaceManifestV1, WorkspaceMode,
};

mod common;

const EXECUTE_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic-execute/descriptor.json");

/// Deterministic commit the disposable source repository is staged with.
/// Must match `repository.revision` in the execute descriptor. Fixed author
/// identity and timestamps make the commit (and therefore the revision pin)
/// reproducible on every platform.
const STAGED_SOURCE_AUTHOR_NAME: &str = "lite-fixture";
const STAGED_SOURCE_AUTHOR_EMAIL: &str = "fixture@example.invalid";
const STAGED_SOURCE_DATE: &str = "1764547200 +0000";
const STAGED_SOURCE_MESSAGE: &str = "lite-fixture pinned source";
/// Stable repository identity for the disposable source repository. A URL
/// identity label bound through the adapter's `git remote get-url origin`
/// branch; never dereferenced, never fetched (the host is an RFC 2606
/// non-resolving name). Carries no machine paths into the evidence chain.
const SOURCE_REPO_IDENTITY_URL: &str =
    "https://repo.invalid/prometheos/lite-fixtures/rust-bugfix-arithmetic-execute-001.git";

const GRAPH_ID: &str = "scenario-task-execution-graph";
const NODE_ID: &str = "fixture-model-node";
const STEP_ID: &str = "fixture-model-step-1";
const MODEL: &str = "tiny-local-model";

const PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "example-operator-price-book",
    "version": "example-2026-11",
    "entries": [
        { "provider": "demo-local", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 }
    ]
}"#;

// ---------------------------------------------------------------------------
// Loopback model fixture (same contract as tests/scenario_fixtures.rs)
// ---------------------------------------------------------------------------

struct LoopbackModel {
    endpoint: String,
    receiver: mpsc::Receiver<()>,
    /// Number of capability invocations observed hitting the loopback
    /// server. This is the fixture's real capability-effect counter: a
    /// governed dispatch through the local-model capability hits exactly
    /// one request per step.
    effects: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

fn spawn_loopback_model(response_body: String) -> LoopbackModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds loopback");
    let addr = listener.local_addr().expect("fixture address");
    let (sender, receiver) = mpsc::channel();
    let effects = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let effects_thread = effects.clone();
    thread::spawn(move || {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => return,
        };
        effects_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = read_request(&stream);
        let body = response_body.as_bytes();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            response_body
        );
        let mut stream = stream;
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = sender.send(());
    });
    LoopbackModel {
        endpoint: format!("http://127.0.0.1:{}/v1", addr.port()),
        receiver,
        effects,
    }
}

fn read_request(mut stream: &TcpStream) -> Option<String> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(5);
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    thread::sleep(Duration::from_millis(50));
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(_) => break,
                    }
                    break;
                }
                if Instant::now() > deadline {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8(buf).ok()
}

fn canned_chat_completion() -> String {
    json!({
        "id": "chatcmpl-scenario-task-execution",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response from loopback local model" },
            "finish_reason": "stop",
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 4, "total_tokens": 11 },
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Disposable deterministic source repository
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_root(descriptor: &ScenarioDescriptor) -> PathBuf {
    descriptor
        .repository
        .fixture
        .split('/')
        .collect::<PathBuf>()
        .iter()
        .fold(repo_root(), |acc, part| acc.join(part))
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

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git invocation failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Stage a disposable source repository from the pinned fixture bytes.
/// Deterministic: fixed identity, fixed author/committer timestamps, no
/// EOL conversion. Returns the commit SHA.
fn stage_source_repository(fixture: &Path, dest: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {dest:?}: {e}"))?;
    copy_tree(fixture, dest);
    git(dest, &["-c", "core.autocrlf=false", "init", "-q"])?;
    // Persist (not just -c) so the later `git worktree add` — issued by the
    // adapter with no -c of its own — cannot convert line endings either.
    git(dest, &["config", "core.autocrlf", "false"])?;
    git(dest, &["config", "commit.gpgsign", "false"])?;
    git(dest, &["config", "user.name", STAGED_SOURCE_AUTHOR_NAME])?;
    git(dest, &["config", "user.email", STAGED_SOURCE_AUTHOR_EMAIL])?;
    git(dest, &["-c", "core.autocrlf=false", "add", "-A"])?;
    let out = Command::new("git")
        .args(["commit", "-q", "-m", STAGED_SOURCE_MESSAGE])
        .current_dir(dest)
        .env("GIT_AUTHOR_NAME", STAGED_SOURCE_AUTHOR_NAME)
        .env("GIT_AUTHOR_EMAIL", STAGED_SOURCE_AUTHOR_EMAIL)
        .env("GIT_COMMITTER_NAME", STAGED_SOURCE_AUTHOR_NAME)
        .env("GIT_COMMITTER_EMAIL", STAGED_SOURCE_AUTHOR_EMAIL)
        .env("GIT_AUTHOR_DATE", STAGED_SOURCE_DATE)
        .env("GIT_COMMITTER_DATE", STAGED_SOURCE_DATE)
        .output()
        .map_err(|e| format!("git commit failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let sha = git(dest, &["rev-parse", "HEAD"])?;
    // Bind the stable URL identity (never fetched); remotes are not part of
    // the commit object, so the revision pin is unaffected.
    git(dest, &["remote", "add", "origin", SOURCE_REPO_IDENTITY_URL])?;
    Ok(sha)
}

/// Relative-path → sha256 map of every regular file in `root`, skipping a
/// top-level `.git` worktree pointer. Used only to report which paths the
/// disposable workspace gained/lost — never for pins.
fn tree_file_map(root: &Path) -> std::collections::BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, map: &mut std::collections::BTreeMap<String, String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_top_level_git = dir == root && entry.file_name() == ".git";
                if is_top_level_git {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => walk(root, &path, map),
                    Ok(t) if t.is_file() => {
                        if let Ok(rel) = path.strip_prefix(root) {
                            let rel = rel
                                .components()
                                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                                .collect::<Vec<_>>()
                                .join("/");
                            if let Ok(bytes) = std::fs::read(&path) {
                                map.insert(rel, sha256_hex(&bytes));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut map = std::collections::BTreeMap::new();
    walk(root, root, &mut map);
    map
}

/// Paths whose content differs (or which exist on only one side) between the
/// staged source and the post-run worktree, relative and forward-slash.
fn tree_changed_paths(before: &Path, after: &Path) -> Vec<String> {
    let a = tree_file_map(before);
    let b = tree_file_map(after);
    let mut changed: Vec<String> = a
        .keys()
        .chain(b.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect();
    changed.sort();
    changed
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

fn repair_patch_digest(edits: &[EditOperation]) -> String {
    let value = serde_json::to_value(edits).expect("edits serialize");
    soma::try_canonical_digest(&value).expect("patch digest")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn repo_relative(root: &Path, path: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| format!("{path:?} escapes {root:?}"))?;
    Ok(rel
        .components()
        .map(|c| match c {
            Component::Normal(name) => Ok(name.to_string_lossy().into_owned()),
            _ => Err(format!("{path:?} has a non-normal component")),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("/"))
}

// ---------------------------------------------------------------------------
// Validator execution (real command; pinned by the descriptor; bounded)
// ---------------------------------------------------------------------------

/// Cap on captured stdout+stderr bytes retained for digesting. Retained
/// bytes feed the receipt digest; `output_bytes` records the total number of
/// bytes consumed by the validator.
const VALIDATOR_OUTPUT_CAP: usize = 1_000_000;
const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(240);

fn drain_capped(mut reader: impl std::io::Read) -> (Vec<u8>, u64) {
    let mut buf = Vec::new();
    let mut total: u64 = 0;
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n as u64;
                if buf.len() < VALIDATOR_OUTPUT_CAP {
                    let keep = (VALIDATOR_OUTPUT_CAP - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..keep]);
                }
            }
        }
    }
    (buf, total)
}

fn run_validator(
    pins: &prometheos_lite::workflow::scenario::ValidationPin,
    worktree: &Path,
    target_dir: &Path,
) -> prometheos_lite::workflow::scenario::TaskValidatorRecord {
    use prometheos_lite::workflow::scenario::{TaskValidatorProvenance, TaskValidatorRecord};
    let started = Instant::now();
    let result = Command::new(&pins.command)
        .args(&pins.args)
        .current_dir(worktree)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    match result {
        Err(_) => TaskValidatorRecord {
            command: pins.command.clone(),
            args: pins.args.clone(),
            expected_exit_code: pins.expected_exit_code,
            assertion: pins.expected_assertion.clone(),
            exit_code: None,
            timed_out: false,
            output_digest: None,
            output_bytes: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            provenance: TaskValidatorProvenance::NotExecuted,
        },
        Ok(mut child) => {
            let out_thread = child
                .stdout
                .take()
                .map(|r| thread::spawn(move || drain_capped(r)));
            let err_thread = child
                .stderr
                .take()
                .map(|r| thread::spawn(move || drain_capped(r)));
            let deadline = Instant::now() + VALIDATOR_TIMEOUT;
            let mut timed_out = false;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => {
                        if Instant::now() > deadline {
                            timed_out = true;
                            let _ = child.kill();
                            let _ = child.wait();
                            break None;
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break None,
                }
            };
            let duration_ms = started.elapsed().as_millis() as u64;
            let (out_bytes, out_total) = out_thread.and_then(|h| h.join().ok()).unwrap_or_default();
            let (err_bytes, err_total) = err_thread.and_then(|h| h.join().ok()).unwrap_or_default();
            let mut digest_input = Vec::with_capacity(out_bytes.len() + err_bytes.len());
            digest_input.extend_from_slice(&out_bytes);
            digest_input.extend_from_slice(&err_bytes);
            TaskValidatorRecord {
                command: pins.command.clone(),
                args: pins.args.clone(),
                expected_exit_code: pins.expected_exit_code,
                assertion: pins.expected_assertion.clone(),
                exit_code: status.and_then(|s| s.code()),
                timed_out,
                output_digest: Some(sha256_hex(&digest_input)),
                output_bytes: out_total + err_total,
                duration_ms,
                provenance: TaskValidatorProvenance::ObservedSubprocess,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Task-execution runner (test adapter around the real governed boundary)
//
// Phase order is load-bearing. NO governed effect (no model dispatch, no
// tool call) happens until every preflight step below has passed:
//   1. the committed fixture bytes are content-verified against the pin;
//   2. the descriptor carries a pinned `validation` surface (without it no
//      task-run success claim could ever seal);
//   3. the disposable source repository is staged deterministically and its
//      staged commit == the pinned revision, with the staged tree digest ==
//      the pin;
//   4. an isolated writable worktree is acquired under the #171 contract and
//      verified (writable-only gate, ensure_writable, adapter validation,
//      revision, baseline tree digest == pin);
//   5. the repair target passes the authorization preflight: every target is
//      repo-relative, inside the editable set, exists, and the patch
//      dry-runs cleanly against the live bytes (read-only, zero writes);
//   6. only then does the governed boundary dispatch the loopback model —
//      with a real capability-effect counter attached;
//   7. the fixture-scripted repair is applied with permission;
//   8. the pinned validator (`cargo test --offline`) runs with its output
//      captured into the receipt.
// ---------------------------------------------------------------------------

struct TaskExecutionRun {
    /// `None` = the governed boundary was never entered (preflight refusal).
    dispatch: Option<Result<RoutedDispatchOutcome, RoutedDispatchFailure>>,
    accounting: Option<AttemptAccountingV1>,
    envelope: Option<LiteTaskRunEvidence>,
    classification: ScenarioOutcomeClass,
    /// Capability invocations observed hitting the loopback server. Zero on
    /// every preflight refusal; exactly one on the positive path.
    capability_effects: u64,
    repaired_lib_rs: Option<String>,
    changed_vs_baseline: Vec<String>,
    patched: bool,
    /// Evidence refused to seal (never a success). Typed reason.
    seal_error: Option<String>,
    /// Pre-effect refusal that happened before anything else (fixture
    /// content pin rejected). Carries the reason.
    preflight_error: Option<String>,
    stage_error: Option<String>,
    acquire_error: Option<String>,
    /// Repair target/policy/dry-run preflight refusal (pre-dispatch).
    repair_preflight_error: Option<String>,
    /// Repair failed to apply after an executed dispatch (infrastructure).
    patch_error: Option<String>,
    staging_digest_ok: bool,
    baseline_digest: Option<String>,
    post_run_digest: Option<String>,
    validator_provenance: Option<prometheos_lite::workflow::scenario::TaskValidatorProvenance>,
}

/// Build the (workspace, file policy, editable set) triple the fixture
/// repair is authorized by. The model dispatch holds NO part of this:
/// authority is the workspace (#171) plus the file policy, never the model.
fn scripted_repair_grant(worktree: &Path) -> (FilePolicy, FileSet) {
    let mut policy = FilePolicy::strict();
    policy.repo_root = worktree.to_path_buf();
    policy.allowed_write_paths = vec![PathBuf::from("src/lib.rs")];
    let file_set = FileSet {
        editable: vec![worktree.join(Path::new("src").join("lib.rs"))],
        readonly: vec![],
        generated: vec![],
        artifacts: vec![],
        denied: vec![],
        binary: vec![],
    };
    (policy, file_set)
}

/// Preflight the requested repair WITHOUT any effect: every target path is
/// repo-relative, inside the editable set, exists, and the whole patch
/// dry-runs cleanly against the live bytes (transaction validation, zero
/// writes). Refuses (returns Err) before anything is authorized.
fn repair_preflight(edits: &[EditOperation], worktree: &Path) -> Result<(), String> {
    use prometheos_lite::harness::file_control::{resolve_repo_path, validate_repo_relative_path};
    use prometheos_lite::harness::patch_applier::dry_run_patch;

    let (policy, file_set) = scripted_repair_grant(worktree);
    for edit in edits {
        let raw: &PathBuf = match edit {
            EditOperation::SearchReplace(x) => &x.file,
            EditOperation::WholeFile(x) => &x.file,
            EditOperation::CreateFile(x) => &x.file,
            EditOperation::DeleteFile(x) => &x.file,
            EditOperation::RenameFile(_) => {
                return Err("rename edits are not permitted for this fixture".to_string());
            }
            EditOperation::UnifiedDiff(_) => {
                return Err("unified-diff edits are not permitted for this fixture".to_string());
            }
        };
        let cleaned =
            validate_repo_relative_path(raw).map_err(|e| format!("repair target refused: {e}"))?;
        let resolved = resolve_repo_path(worktree, &cleaned)
            .map_err(|e| format!("repair target refused: {e}"))?;
        if !file_set.editable.contains(&resolved) {
            return Err(format!(
                "repair target {cleaned:?} is outside the authorized editable set"
            ));
        }
        if !resolved.exists() {
            return Err(format!("repair target {cleaned:?} does not exist"));
        }
    }
    // Transaction validation against the live bytes. Read-only.
    match dispatch_resolved_route_block_on(dry_run_patch(edits, &file_set, &policy)) {
        Ok(result) if result.failures.is_empty() => Ok(()),
        Ok(result) => Err(format!(
            "repair dry-run failed: {}",
            result
                .failures
                .iter()
                .map(|f| format!("{}: {}", f.file.display(), f.reason))
                .collect::<Vec<_>>()
                .join("; ")
        )),
        Err(error) => Err(format!("repair dry-run refused: {error:#}")),
    }
}

fn workspace_adapter(kind: ScenarioWorkspaceAdapter) -> AdapterKind {
    match kind {
        ScenarioWorkspaceAdapter::ExistingReadOnly => AdapterKind::ExistingReadOnly,
        ScenarioWorkspaceAdapter::GitWorktree => AdapterKind::GitWorktree,
    }
}

fn build_workspace_manifest(descriptor: &ScenarioDescriptor) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: format!("scenario-workspace-{}", descriptor.scenario_id),
        adapter: workspace_adapter(descriptor.workspace.adapter),
        adapter_revision: descriptor.workspace.adapter_revision.clone(),
        repo_identity: SOURCE_REPO_IDENTITY_URL.to_string(),
        base_revision: descriptor.repository.revision.clone(),
        branch: None,
        mode: match descriptor.workspace.mode {
            scenario::ScenarioWorkspaceMode::ReadOnly => WorkspaceMode::ReadOnly,
            scenario::ScenarioWorkspaceMode::Writable => WorkspaceMode::Writable,
        },
        writable_scopes: descriptor.workspace.writable_scopes.clone(),
        resource_lock_id: format!("scenario-lock-{}", descriptor.scenario_id),
        created_at: descriptor.route.evaluated_at.clone(),
        content_digest: None,
    }
    .sealed()
}

fn preflight_refusal(
    classification: ScenarioOutcomeClass,
    preflight_error: Option<String>,
    stage_error: Option<String>,
    acquire_error: Option<String>,
    repair_preflight_error: Option<String>,
    staging_digest_ok: bool,
    baseline_digest: Option<String>,
) -> TaskExecutionRun {
    TaskExecutionRun {
        dispatch: None,
        accounting: None,
        envelope: None,
        classification,
        capability_effects: 0,
        repaired_lib_rs: None,
        changed_vs_baseline: vec![],
        patched: false,
        seal_error: None,
        preflight_error,
        stage_error,
        acquire_error,
        repair_preflight_error,
        patch_error: None,
        staging_digest_ok,
        post_run_digest: baseline_digest.clone(),
        baseline_digest,
        validator_provenance: None,
    }
}

/// Execute one task fixture end to end through the governed boundary.
/// `edits` are the fixture-scripted repair operations. The validator is
/// NOT caller-selectable: it comes from the descriptor's pinned
/// `validation` section.
fn run_task_execution(descriptor_text: &str, edits: Vec<EditOperation>) -> TaskExecutionRun {
    let descriptor = ScenarioDescriptor::parse_json(descriptor_text)
        .expect("descriptor must validate before running");

    // ---- PHASE 1: the committed fixture bytes are pinned -----------------
    let fixture = fixture_root(&descriptor);
    if let Err(error) =
        scenario::verify_fixture_content(&fixture, &descriptor.repository.content_digest)
    {
        // Pre-effect refusal: the pinned bytes are not the committed bytes.
        // No staging, no dispatch, no effects of any kind.
        return preflight_refusal(
            ScenarioOutcomeClass::SetupError,
            Some(format!("{error:#}")),
            None,
            None,
            None,
            false,
            None,
        );
    }

    // ---- PHASE 2: the descriptor must pin the validator surface ---------
    // Without it no task-run success claim could ever seal; failing here
    // keeps the governed boundary from running at all for an unverifiable
    // task.
    if descriptor.validation.is_none() {
        return preflight_refusal(
            ScenarioOutcomeClass::InvalidTask,
            Some(
                "descriptor pins no `validation` section (command/argv, expected exit code, expected assertion and expected digests); the task has no definition of done"
                    .to_string(),
            ),
            None,
            None,
            None,
            false,
            None,
        );
    }

    // ---- PHASE 3: stage the disposable deterministic source --------------
    let staging_root = tempfile::tempdir().expect("staging workspace");
    let source = staging_root.path().join("source");
    let workspaces = staging_root.path().join("workspaces");

    let mut stage_error = None;
    let mut staging_digest_ok = false;
    let mut baseline_digest = None;
    match stage_source_repository(&fixture, &source) {
        Ok(staged_sha) => {
            staging_digest_ok = scenario::task_run_tree_digest(&source)
                .map(|digest| digest == descriptor.repository.content_digest)
                .unwrap_or(false);
            if staged_sha != descriptor.repository.revision {
                stage_error = Some(format!(
                    "staged source commit {staged_sha} != descriptor revision pin {}",
                    descriptor.repository.revision
                ));
            } else if !staging_digest_ok {
                stage_error =
                    Some("staged source tree digest != pinned fixture content digest".to_string());
            }
        }
        Err(error) => stage_error = Some(error),
    }
    if stage_error.is_none() {
        baseline_digest = scenario::task_run_tree_digest(&source).ok();
    }
    if stage_error.is_some() {
        // Pre-effect refusal: the staging proof failed. No dispatch happened.
        return preflight_refusal(
            ScenarioOutcomeClass::SetupError,
            None,
            stage_error,
            None,
            None,
            staging_digest_ok,
            baseline_digest,
        );
    }

    // ---- PHASE 4: isolated writable worktree, acquired and verified ------
    // BEFORE any governed effect. The manifest pins the staged source
    // commit; the adapter's writable-only gate, revision check, adapter
    // validation and a baseline tree-digest check against the fixture pin
    // must ALL pass before dispatch is authorized.
    let workspace_manifest = build_workspace_manifest(&descriptor);
    let adapter = GitWorktreeWorkspace {
        parent_dir: workspaces,
        source_repo: source.clone(),
    };
    let mut acquire_error = None;
    let mut acquired = None;
    match workspace::WorkspaceAdapter::acquire(&adapter, &workspace_manifest) {
        Ok(acquired_workspace) => {
            if let Err(error) = acquired_workspace.ensure_writable() {
                acquire_error = Some(format!("ensure_writable refused: {error}"));
            } else if acquired_workspace.revision != workspace_manifest.base_revision {
                acquire_error = Some(format!(
                    "acquired revision {} != pinned {}",
                    acquired_workspace.revision, workspace_manifest.base_revision
                ));
            } else if let Err(error) =
                workspace::WorkspaceAdapter::validate(&adapter, &acquired_workspace)
            {
                acquire_error = Some(format!("workspace validate refused: {error}"));
            } else if let Ok(digest) = scenario::task_run_tree_digest(&acquired_workspace.root) {
                if digest != descriptor.repository.content_digest {
                    acquire_error = Some(format!(
                        "worktree source digest {digest} != pinned fixture {}",
                        descriptor.repository.content_digest
                    ));
                }
            } else {
                acquire_error = Some("worktree source digest unreadable".to_string());
            }
            if acquire_error.is_none() {
                acquired = Some(acquired_workspace);
            }
        }
        Err(error) => acquire_error = Some(format!("{error:#}")),
    }
    if acquire_error.is_some() {
        // Pre-effect refusal: the authorized workspace does not exist. No
        // dispatch, no effects of any kind.
        return preflight_refusal(
            ScenarioOutcomeClass::SetupError,
            None,
            None,
            acquire_error,
            None,
            staging_digest_ok,
            scenario::task_run_tree_digest(&source).ok(),
        );
    }
    let worktree_root = acquired
        .as_ref()
        .map(|a| a.root.clone())
        .expect("acquired worktree root");
    let baseline_digest = scenario::task_run_tree_digest(&worktree_root).ok();
    let baseline_file_digest = std::fs::read(worktree_root.join("src").join("lib.rs"))
        .ok()
        .as_deref()
        .map(sha256_hex);

    // ---- PHASE 5: repair-target authorization preflight (zero writes) ----
    if let Err(error) = repair_preflight(&edits, &worktree_root) {
        return preflight_refusal(
            ScenarioOutcomeClass::SetupError,
            None,
            None,
            None,
            Some(error),
            staging_digest_ok,
            baseline_digest,
        );
    }

    // ---- PHASE 7: governed dispatch (first and only effect) --------------
    let server = spawn_loopback_model(canned_chat_completion());
    let candidate = RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: descriptor.route.candidate_id.clone(),
        provider_identity: descriptor.route.provider_identity.clone(),
        provider_locality: match descriptor.route.provider_locality.as_str() {
            "local" => Locality::Local,
            other => panic!("unsupported provider locality in fixture: {other}"),
        },
        model: Some(descriptor.route.model.clone()),
        harness_id: descriptor.harness.id.clone(),
        harness_revision: descriptor.harness.revision.clone(),
        workspace: Some(RouteWorkspaceBinding {
            adapter: workspace_adapter(descriptor.workspace.adapter),
            revision: descriptor.workspace.adapter_revision.clone(),
        }),
        trust: RouteTrust {
            source: "scenario-fixture-pin".to_string(),
            verified_at: descriptor.route.trust_verified_at.clone(),
            ttl_seconds: descriptor.route.trust_ttl_seconds,
        },
        budget: None,
        priority: descriptor.route.priority,
        experimental: false,
    };

    let requirements: WorkRequirements = serde_json::from_value(json!({
        "id": "reqs-scenario-task-execution",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": descriptor.requirements.degradation_permitted,
        "forbidSubstitution": descriptor.requirements.forbid_substitution,
        "requiredCapabilities": []
    }))
    .expect("requirements parse");
    let authority: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass": "deterministic", "mutation": "none"}"#)
            .expect("authority parses");
    let request = RouteResolutionRequest {
        route_set_id: descriptor.route.route_set_id.clone(),
        route_set_version: descriptor.route.route_set_version.clone(),
        candidates: vec![candidate],
        pinned_candidate_id: Some(&descriptor.route.pinned_candidate_id),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: prometheos_lite::workflow::route_resolution::RouteProfile::Standard,
        evaluated_at: descriptor.route.evaluated_at.clone(),
        workspace: Some(&workspace_manifest),
    };

    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(LocalModelHarness::new()))
        .expect("local model harness registers");

    let resolution = match resolve_route(&request, &harness_registry) {
        Ok(resolution) => resolution,
        Err(error) => {
            // Route resolution refused before any effect. Effects: none.
            return preflight_refusal(
                scenario::classify_pre_effect_refusal(),
                Some(format!("route resolution refused before effect: {error:#}")),
                None,
                None,
                None,
                staging_digest_ok,
                baseline_digest,
            );
        }
    };
    resolution.evidence.verify().expect("evidence verifies");
    resolution.snapshot.verify().expect("snapshot verifies");

    let mut capability_registry = CapabilityRegistry::new();
    let fixture_workspace = tempfile::tempdir().expect("fixture workspace");
    capability_registry.declare(
        LOCAL_MODEL_INFER_CAPABILITY,
        local_model_capability(
            LocalModelConfig::new(server.endpoint.clone(), MODEL).timeout_ms(5_000),
            fixture_workspace.path().to_path_buf(),
        ),
    );
    let permit = common::permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
    let mut runner = NodeRunner::new(capability_registry, permit);

    let manifest = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed scenario task-execution node",
            "inputs": [],
            "outputs": [{ "name": "out", "typeRef": "string" }],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": { "maxAttempts": 1, "retryableClasses": [] }
        })
        .to_string(),
    )
    .expect("manifest parses");
    let restrictions = LocalRestrictions {
        readable_scopes: vec!["repo://fixture".into()],
        writable_scopes: vec![],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 1,
        escalation_target: "human-review".into(),
    };

    let dispatch =
        dispatch_resolved_route_block_on(dispatch_resolved_route(RoutedDispatchRequest {
            resolution: &resolution,
            original_request: &request,
            registry: &harness_registry,
            runner: &mut runner,
            manifest: &manifest,
            local_restrictions: &restrictions,
            known_secrets: vec![SECRET_CANARY.to_string()],
            clock: &FixedTrustedTimeSource::new(&descriptor.route.dispatch_at),
            step: prometheos_lite::workflow::execution_harness::HarnessStep {
                step_id: STEP_ID.to_string(),
                capability: LOCAL_MODEL_INFER_CAPABILITY.to_string(),
                args: json!({
                    "model": MODEL,
                    "endpoint": server.endpoint,
                    "messages": [{ "role": "user", "content": "hi" }],
                    "maxTokens": 64,
                }),
            },
        }));
    // The capability-effect counter is read from the loopback server: it is
    // the number of times a governed capability actually executed.
    let capability_effects = server.effects.load(std::sync::atomic::Ordering::SeqCst);
    let dispatch_wait = match &dispatch {
        Ok(_) | Err(RoutedDispatchFailure::ExecutionFailed(_)) => Duration::from_secs(10),
        Err(RoutedDispatchFailure::Refused(_)) => Duration::ZERO,
    };
    if dispatch_wait.is_zero() {
        drop(server);
    } else {
        let _ = server.receiver.recv_timeout(dispatch_wait);
    }

    let accounting_context = AccountingContext {
        graph_id: GRAPH_ID.to_string(),
        node_id: NODE_ID.to_string(),
        currency: Some("USD".to_string()),
    };
    let accounting = match dispatch.as_ref() {
        Ok(outcome) => {
            let pricing = PricingBook::from_json(PRICE_BOOK).expect("price book parses");
            let record =
                account_routed_outcome(outcome, &resolution, &accounting_context, Some(&pricing))
                    .expect("outcome accounts");
            record.verify().expect("accounting record verifies");
            Some(record)
        }
        Err(RoutedDispatchFailure::ExecutionFailed(failure)) => Some(
            prometheos_lite::workflow::accounting::account_routed_failure(
                &resolution,
                "execution-failed",
                &accounting_context,
                STEP_ID,
                failure.journal_entry.clone(),
                0,
            ),
        ),
        Err(RoutedDispatchFailure::Refused(_)) => None,
    };
    let graph = accounting.as_ref().map(|record| {
        let mut ledger = AccountingLedger::new();
        ledger.record(record.clone()).expect("record registers");
        ledger.aggregate_graph(GRAPH_ID).expect("graph aggregates")
    });

    let dispatch_classification =
        scenario::classify_completed_run(true, &dispatch, accounting.is_some());

    // ---- PHASE 7b: apply the controlled repair (permitted; post-dispatch) --
    let (policy, file_set) = scripted_repair_grant(&worktree_root);
    // The dispatch itself holds NO write authority; the repair write is
    // gated here by the workspace boundary + the file policy + the editable
    // set, not by the model's authority. (The preflight above already
    // validated the same grant against the live bytes; this is the write.)
    let mut patch_error = None;
    let mut patched = false;
    let mut changed_files: Vec<String> = vec![];
    let file_before_digest = baseline_file_digest.clone().unwrap_or_default();
    match dispatch_resolved_route_block_on(apply_patch_with_rollback(&edits, &file_set, &policy)) {
        Ok((patch_result, _rollback)) => {
            if patch_result.applied && patch_result.failures.is_empty() {
                patched = true;
                changed_files = patch_result
                    .changed_files
                    .iter()
                    .map(|p| {
                        repo_relative(&worktree_root, p)
                            .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/"))
                    })
                    .collect();
            } else {
                patch_error = Some(format!(
                    "repair patch not applied after dry-run passed: {}",
                    patch_result
                        .failures
                        .iter()
                        .map(|f| format!("{}: {}", f.file.display(), f.reason))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
        }
        Err(error) => patch_error = Some(format!("repair patch refused: {error:#}")),
    }
    let file_after_digest = std::fs::read(worktree_root.join("src").join("lib.rs"))
        .map(|b| sha256_hex(&b))
        .unwrap_or_default();

    // A repair that cannot be applied after a completed dispatch is an
    // infrastructure fault, never a success: the task produced no effect and
    // no evidence is sealed.
    if patch_error.is_some() {
        return TaskExecutionRun {
            dispatch: Some(dispatch),
            accounting,
            envelope: None,
            classification: ScenarioOutcomeClass::InfrastructureError,
            capability_effects,
            repaired_lib_rs: std::fs::read_to_string(worktree_root.join("src").join("lib.rs")).ok(),
            changed_vs_baseline: tree_changed_paths(&source, &worktree_root),
            patched,
            seal_error: None,
            preflight_error: None,
            stage_error: None,
            acquire_error: None,
            repair_preflight_error: None,
            patch_error,
            staging_digest_ok,
            baseline_digest,
            post_run_digest: scenario::task_run_tree_digest(&worktree_root).ok(),
            validator_provenance: Some(
                prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted,
            ),
        };
    }

    // ---- PHASE 8: the pinned validator (only when a permitted repair was
    // applied) --------------------------------------------------------------
    let pins = descriptor
        .validation
        .clone()
        .expect("validation pins checked at preflight");
    let validator_record = if patched {
        let target_dir = staging_root.path().join("validator-target");
        run_validator(&pins, &worktree_root, &target_dir)
    } else {
        // Defensive: the repair was applied and dry-run-clean, so this is
        // unreachable in practice; if it ever happened, the validator record
        // reports not-executed rather than inventing a result.
        prometheos_lite::workflow::scenario::TaskValidatorRecord {
            command: pins.command.clone(),
            args: pins.args.clone(),
            expected_exit_code: pins.expected_exit_code,
            assertion: pins.expected_assertion.clone(),
            exit_code: None,
            timed_out: false,
            output_digest: None,
            output_bytes: 0,
            duration_ms: 0,
            provenance: prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted,
        }
    };
    let validator_provenance = validator_record.provenance;

    // ---- changed set on disk (guards against out-of-scope effects) -------
    let changed_vs_baseline: Vec<String> = tree_changed_paths(&source, &worktree_root);

    // ---- post-run digest --------------------------------------------------
    let post_run_digest = scenario::task_run_tree_digest(&worktree_root).ok();

    // ---- classification -----------------------------------------------------
    // Nothing above this point may fail without classify-before-seal; the
    // only remaining failure modes are validator-execution outcomes, mapped
    // here. Anything else would have returned before the dispatch.
    let classification = if validator_record.exit_code.is_none() {
        // Validator could not complete (missing tool / killed on timeout).
        ScenarioOutcomeClass::InfrastructureError
    } else {
        dispatch_classification
    };

    // ---- seal the task-run envelope (only with full dispatch evidence) ---
    let (envelope, seal_error) = match (
        dispatch.as_ref(),
        accounting.as_ref(),
        graph.as_ref(),
        baseline_digest.clone(),
        post_run_digest.clone(),
    ) {
        (Ok(outcome), Some(record), Some(graph), Some(baseline), Some(post)) => {
            let scenario_run = prometheos_lite::workflow::scenario::LiteScenarioRunEvidence::sealed(
                descriptor,
                &resolution,
                &manifest,
                outcome,
                record.clone(),
                graph.clone(),
                Some(&workspace_manifest),
                dispatch_classification,
            );
            match scenario_run {
                Ok(scenario_run) => {
                    let repair_record = TaskRepairRecord {
                        origin: TaskRepairOrigin::ScriptedFixture,
                        write_authority: prometheos_lite::workflow::scenario::TaskRepairWriteAuthority::ScriptedFixtureWorkspaceFilePolicy,
                        action: "search-replace".to_string(),
                        changed_files: changed_files.clone(),
                        file_before_digest,
                        file_after_digest,
                        patch_digest: repair_patch_digest(&edits),
                        applied: patched,
                    };
                    match LiteTaskRunEvidence::sealed(
                        scenario_run,
                        repair_record,
                        validator_record,
                        baseline,
                        post,
                        classification,
                    ) {
                        Ok(task_run) => (Some(task_run), None),
                        Err(error) => (None, Some(format!("task-run refused to seal: {error:#}"))),
                    }
                }
                Err(error) => (
                    None,
                    Some(format!("scenario-run refused to seal: {error:#}")),
                ),
            }
        }
        _ => (None, Some("dispatch evidence incomplete".to_string())),
    };

    let classification = if seal_error.is_some() {
        // The run happened but its evidence cannot be truthfully sealed;
        // never a success.
        ScenarioOutcomeClass::InfrastructureError
    } else {
        classification
    };

    // Read the patched source before the staging root is dropped, so tests
    // can assert the on-disk result without keeping the tree alive.
    let repaired_lib_rs = std::fs::read_to_string(worktree_root.join("src").join("lib.rs")).ok();

    TaskExecutionRun {
        dispatch: Some(dispatch),
        accounting,
        envelope,
        classification,
        capability_effects,
        repaired_lib_rs,
        changed_vs_baseline,
        patched,
        seal_error,
        preflight_error: None,
        stage_error: None,
        acquire_error: None,
        repair_preflight_error: None,
        patch_error,
        staging_digest_ok,
        baseline_digest,
        post_run_digest,
        validator_provenance: Some(validator_provenance),
    }
}

fn dispatch_resolved_route_block_on<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

/// The pinned commit the disposable source repository stages. Must match
/// `repository.revision` in the execute descriptor.
const EXECUTE_REVISION: &str = "bfca0ee96857e365026ac07f9a95c514b7e58e8b";

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
    let sha = stage_source_repository(&fixture, &dest).expect("staging succeeds");
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
        .repaired_lib_rs
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
    stage_source_repository(&fixture, &source).expect("staging succeeds");

    // Build the manifest with mode forced to read-only: the git-worktree
    // adapter must refuse to acquire it, before any effect.
    let mut manifest = build_workspace_manifest(&descriptor);
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
    stage_source_repository(&fixture, &dest).expect("staging succeeds");
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
    stage_source_repository(&fixture, &dest).expect("staging succeeds");

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
            .starts_with("foundry.workflow-evaluation")
    );
    assert_eq!(
        envelope.repair.origin,
        TaskRepairOrigin::ScriptedFixture,
        "scripted repair is labelled scripted, never model skill"
    );
}
