//! Shared task-execution fixture runner for the executed-scenario suites
//! (issue #136 Slices B and C).
//!
//! This module is the *test adapter around the real governed boundary*,
//! extracted so the executed fixture suites (Slice B `scenario_task_execution`,
//! Slice C `scenario_documentation_task` and `scenario_failure_replay`) all
//! exercise the same load-bearing preflight order and the same evidence
//! semantics. The caller pins a [`TaskExecutionSpec`] per fixture: which
//! repo-relative file the scripted repair is permitted to touch, and which
//! stable repository identity the disposable source carries. Everything else
//! - the preflight phases, the governed dispatch, the validator receipt, the
//! sealing and every refusal gate - is shared, unmodified code.
//!
//! Phase order (load-bearing). NO governed effect happens until every
//! preflight step below has passed:
//!   1. the committed fixture bytes are content-verified against the pin;
//!   2. the descriptor carries a pinned `validation` surface (without it no
//!      task-run success claim could ever seal);
//!   3. the disposable source repository is staged deterministically and its
//!      staged commit == the pinned revision, with the staged tree digest ==
//!      the pin;
//!   4. an isolated writable worktree is acquired under the #171 contract and
//!      verified (writable-only gate, ensure_writable, adapter validation,
//!      revision, baseline tree digest == pin);
//!   5. the repair target passes the authorization preflight: every target is
//!      repo-relative, inside the editable set, exists, and the whole patch
//!      dry-runs cleanly against the live bytes (read-only, zero writes);
//!   6. only then does the governed boundary dispatch the loopback model -
//!      with a real capability-effect counter attached.
//! Result gate: a refused or failed dispatch NEVER authorizes the repair.
//!
//! The repair is a **scripted fixture**, not model output. Nothing here
//! demonstrates model coding ability; no benchmark score, ranking or Foundry
//! claim is produced.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

use prometheos_lite::harness::edit_protocol::EditOperation;
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
    TaskRepairOrigin, TaskRepairRecord,
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

/// Deterministic commit the disposable source repository is staged with.
/// Fixed author identity and timestamps make the commit (and therefore the
/// revision pin) reproducible on every platform. Each executed fixture pins
/// its own commit SHA in its descriptor.
pub const STAGED_SOURCE_AUTHOR_NAME: &str = "lite-fixture";
pub const STAGED_SOURCE_AUTHOR_EMAIL: &str = "fixture@example.invalid";
pub const STAGED_SOURCE_DATE: &str = "1764547200 +0000";
pub const STAGED_SOURCE_MESSAGE: &str = "lite-fixture pinned source";

pub const GRAPH_ID: &str = "scenario-task-execution-graph";
pub const NODE_ID: &str = "fixture-model-node";
pub const STEP_ID: &str = "fixture-model-step-1";
pub const MODEL: &str = "tiny-local-model";

pub const PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "example-operator-price-book",
    "version": "example-2026-11",
    "entries": [
        { "provider": "demo-local", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 }
    ]
}"#;

/// Per-fixture facts the shared runner cannot know on its own. Everything
/// else is pinned by the descriptor or fixed for every executed fixture.
#[derive(Debug, Clone, Copy)]
pub struct TaskExecutionSpec {
    /// Repo-relative, forward-slash path of the single file the scripted
    /// repair is permitted to touch (e.g. `src/lib.rs`, `README.md`).
    pub repair_target: &'static str,
    /// Stable repository identity URL bound through the adapter's
    /// `git remote get-url origin` branch; never dereferenced, never fetched
    /// (the host is an RFC 2606 non-resolving name). Carries no machine
    /// paths into the evidence chain.
    pub repo_identity_url: &'static str,
}

// ---------------------------------------------------------------------------
// Loopback model fixture
// ---------------------------------------------------------------------------

pub struct LoopbackModel {
    pub endpoint: String,
    pub receiver: mpsc::Receiver<()>,
    /// Number of capability invocations observed hitting the loopback
    /// server. This is the fixture's real capability-effect counter: a
    /// governed dispatch through the local-model capability hits exactly
    /// one request per step.
    pub effects: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

pub fn spawn_loopback_model(response_body: String) -> LoopbackModel {
    spawn_loopback_with_status_line("200 OK", response_body)
}

/// Loopback with a selectable status line, so the fixture can also simulate
/// a failing model endpoint deterministically.
pub fn spawn_loopback_with_status_line(status_line: &str, response_body: String) -> LoopbackModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds loopback");
    let addr = listener.local_addr().expect("fixture address");
    let (sender, receiver) = mpsc::channel();
    let effects = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let effects_thread = effects.clone();
    let status_line = status_line.to_string();
    thread::spawn(move || {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => return,
        };
        effects_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = read_request(&stream);
        let body = response_body.as_bytes();
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

pub fn canned_chat_completion() -> String {
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

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture_root(descriptor: &ScenarioDescriptor) -> PathBuf {
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

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
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
/// EOL conversion. Returns the commit SHA. `repo_identity_url` binds the
/// stable, never-fetched URL identity (see [`TaskExecutionSpec`]).
pub fn stage_source_repository(
    fixture: &Path,
    dest: &Path,
    repo_identity_url: &str,
) -> Result<String, String> {
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
    git(dest, &["remote", "add", "origin", repo_identity_url])?;
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
pub fn tree_changed_paths(before: &Path, after: &Path) -> Vec<String> {
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

pub fn repair_patch_digest(edits: &[EditOperation]) -> String {
    let value = serde_json::to_value(edits).expect("edits serialize");
    soma::try_canonical_digest(&value).expect("patch digest")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn repo_relative(root: &Path, path: &Path) -> Result<String, String> {
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
pub const VALIDATOR_OUTPUT_CAP: usize = 1_000_000;
pub const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(240);

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

pub fn run_validator(
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
// ---------------------------------------------------------------------------

pub struct TaskExecutionRun {
    /// `None` = the governed boundary was never entered (preflight refusal).
    pub dispatch: Option<Result<RoutedDispatchOutcome, RoutedDispatchFailure>>,
    pub accounting: Option<AttemptAccountingV1>,
    pub envelope: Option<LiteTaskRunEvidence>,
    pub classification: ScenarioOutcomeClass,
    /// Capability invocations observed hitting the loopback server. Zero on
    /// every preflight refusal; exactly one on the positive path.
    pub capability_effects: u64,
    /// The repaired target file's content read back after the run.
    pub repaired_file_contents: Option<String>,
    pub changed_vs_baseline: Vec<String>,
    pub patched: bool,
    /// Evidence refused to seal (never a success). Typed reason.
    pub seal_error: Option<String>,
    /// Pre-effect refusal that happened before anything else (fixture
    /// content pin rejected). Carries the reason.
    pub preflight_error: Option<String>,
    pub stage_error: Option<String>,
    pub acquire_error: Option<String>,
    /// Repair target/policy/dry-run preflight refusal (pre-dispatch).
    pub repair_preflight_error: Option<String>,
    /// Repair failed to apply after an executed dispatch (infrastructure).
    pub patch_error: Option<String>,
    pub staging_digest_ok: bool,
    pub baseline_digest: Option<String>,
    pub post_run_digest: Option<String>,
    pub validator_provenance: Option<prometheos_lite::workflow::scenario::TaskValidatorProvenance>,
}

/// Build the (workspace, file policy, editable set) triple the fixture
/// repair is authorized by. The model dispatch holds NO part of this:
/// authority is the workspace (#171) plus the file policy, never the model.
fn scripted_repair_grant(worktree: &Path, repair_target: &str) -> (FilePolicy, FileSet) {
    let mut policy = FilePolicy::strict();
    policy.repo_root = worktree.to_path_buf();
    policy.allowed_write_paths = vec![PathBuf::from(repair_target)];
    let file_set = FileSet {
        editable: vec![worktree.join(Path::new(repair_target))],
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
pub fn repair_preflight(
    edits: &[EditOperation],
    worktree: &Path,
    repair_target: &str,
) -> Result<(), String> {
    use prometheos_lite::harness::file_control::{resolve_repo_path, validate_repo_relative_path};
    use prometheos_lite::harness::patch_applier::dry_run_patch;

    let (policy, file_set) = scripted_repair_grant(worktree, repair_target);
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

pub fn build_workspace_manifest(
    spec: &TaskExecutionSpec,
    descriptor: &ScenarioDescriptor,
) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: format!("scenario-workspace-{}", descriptor.scenario_id),
        adapter: workspace_adapter(descriptor.workspace.adapter),
        adapter_revision: descriptor.workspace.adapter_revision.clone(),
        repo_identity: spec.repo_identity_url.to_string(),
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

#[allow(clippy::too_many_arguments)]
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
        repaired_file_contents: None,
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

/// Test-only dispatch boundary injection. `Normal` is the only production
/// wiring; the other two variants exist purely so the fixture can force a
/// governed post-preflight failure deterministically, without weakening any
/// gate (the refusal fires inside the real dispatch code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchInjection {
    Normal,
    /// The step names a capability the selected harness never declared: the
    /// governed boundary refuses BEFORE any effect (typed refusal).
    RefuseStepCapability,
    /// The loopback model endpoint returns HTTP 500: the capability handler
    /// runs and the boundary fails the effect with a typed, sealed
    /// `ExecutionFailed` failure and a journal entry.
    Http500,
}

/// Execute one task fixture end to end through the governed boundary.
/// `edits` are the fixture-scripted repair operations. The validator is
/// NOT caller-selectable: it comes from the descriptor's pinned
/// `validation` section.
pub fn run_task_execution(
    spec: &TaskExecutionSpec,
    descriptor_text: &str,
    edits: Vec<EditOperation>,
) -> TaskExecutionRun {
    run_task_execution_with_fault(spec, descriptor_text, edits, DispatchInjection::Normal)
}

/// Full runner with an optional deterministic fault injected at the governed
/// dispatch boundary. See [`DispatchInjection`].
pub fn run_task_execution_with_fault(
    spec: &TaskExecutionSpec,
    descriptor_text: &str,
    edits: Vec<EditOperation>,
    injection: DispatchInjection,
) -> TaskExecutionRun {
    let repair_target = spec.repair_target;
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
    match stage_source_repository(&fixture, &source, spec.repo_identity_url) {
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
    let workspace_manifest = build_workspace_manifest(spec, &descriptor);
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
    let baseline_file_digest = std::fs::read(worktree_root.join(Path::new(repair_target)))
        .ok()
        .as_deref()
        .map(sha256_hex);

    // ---- PHASE 5: repair-target authorization preflight (zero writes) ----
    if let Err(error) = repair_preflight(&edits, &worktree_root, repair_target) {
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
    let server = match injection {
        DispatchInjection::Http500 => {
            spawn_loopback_with_status_line("500 Internal Server Error", canned_chat_completion())
        }
        _ => spawn_loopback_model(canned_chat_completion()),
    };
    // The step capability the harness does (not) declare: a mismatch forces
    // a typed, pre-effect refusal inside the governed dispatch itself —
    // before the loopback is ever contacted.
    let step_capability = match injection {
        DispatchInjection::RefuseStepCapability => {
            "harness.local_model.infer-not-declared".to_string()
        }
        _ => LOCAL_MODEL_INFER_CAPABILITY.to_string(),
    };
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
    let permit = crate::common::permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
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
                capability: step_capability,
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

    // ---- RESULT GATE: a refused or failed dispatch NEVER authorizes the
    // scripted repair. The workspace+file-policy permit is necessary, not
    // sufficient. On any Err the run returns with no repair, no validator,
    // no task-outcome claim, and (for a refusal) no accounting at all;
    // for a typed ExecutionFailed failure the typed failure/journal/
    // accounting evidence is preserved exactly as measured.
    match &dispatch {
        Ok(_) => {} // the governed effect succeeded; the repair may proceed
        Err(RoutedDispatchFailure::Refused(refused)) => {
            let refused_reason = format!("{refused:#}");
            return TaskExecutionRun {
                dispatch: Some(dispatch),
                accounting: None,
                envelope: None,
                classification: ScenarioOutcomeClass::SetupError,
                capability_effects,
                repaired_file_contents: std::fs::read_to_string(
                    worktree_root.join(Path::new(repair_target)),
                )
                .ok(),
                changed_vs_baseline: tree_changed_paths(&source, &worktree_root),
                patched: false,
                seal_error: None,
                preflight_error: Some(format!(
                    "governed dispatch refused before any effect: {refused_reason}"
                )),
                stage_error: None,
                acquire_error: None,
                repair_preflight_error: None,
                patch_error: None,
                staging_digest_ok,
                baseline_digest,
                post_run_digest: scenario::task_run_tree_digest(&worktree_root).ok(),
                validator_provenance: Some(
                    prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted,
                ),
            };
        }
        Err(RoutedDispatchFailure::ExecutionFailed(_)) => {
            return TaskExecutionRun {
                dispatch: Some(dispatch),
                accounting,
                envelope: None,
                classification: dispatch_classification,
                capability_effects,
                repaired_file_contents: std::fs::read_to_string(
                    worktree_root.join(Path::new(repair_target)),
                )
                .ok(),
                changed_vs_baseline: tree_changed_paths(&source, &worktree_root),
                patched: false,
                seal_error: None,
                preflight_error: None,
                stage_error: None,
                acquire_error: None,
                repair_preflight_error: None,
                patch_error: None,
                staging_digest_ok,
                baseline_digest,
                post_run_digest: scenario::task_run_tree_digest(&worktree_root).ok(),
                validator_provenance: Some(
                    prometheos_lite::workflow::scenario::TaskValidatorProvenance::NotExecuted,
                ),
            };
        }
    }

    // ---- PHASE 7b: apply the controlled repair (permitted; post-dispatch) --
    let (policy, file_set) = scripted_repair_grant(&worktree_root, repair_target);
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
    let file_after_digest = std::fs::read(worktree_root.join(Path::new(repair_target)))
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
            repaired_file_contents: std::fs::read_to_string(
                worktree_root.join(Path::new(repair_target)),
            )
            .ok(),
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
    let repaired_file_contents =
        std::fs::read_to_string(worktree_root.join(Path::new(repair_target))).ok();

    TaskExecutionRun {
        dispatch: Some(dispatch),
        accounting,
        envelope,
        classification,
        capability_effects,
        repaired_file_contents,
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

pub fn dispatch_resolved_route_block_on<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}
