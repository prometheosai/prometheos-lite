//! Issue #136 Slice B — one genuinely executed repository-task fixture.
//!
//! Slice A pinned the fixture bytes and ran the governed boundary with a
//! loopback model, but it never applied a patch to the pinned repository or
//! validated the repair. This suite executes ONE real, deterministic
//! repository task end to end:
//!
//! 1. the committed fixture is content-verified against its pin BEFORE work;
//! 2. a disposable, deterministic source repository is staged from those
//!    bytes (fixed identity, fixed commit timestamp) and its commit is
//!    checked against the descriptor's revision pin;
//! 3. an isolated, writable git worktree is acquired under the existing
//!    governed workspace contract (#171) and the authority is verified;
//! 4. a controlled, fixture-scripted repair is applied through the existing
//!    filesystem permission boundary (`FilePolicy` + `FileSet` +
//!    `apply_patch_with_rollback`) — NEVER the committed source fixture;
//! 5. the real validator (`cargo test --offline`) runs with a pinned command
//!    and its exit status is captured;
//! 6. the post-run tree digest, the patch digest and the validator result
//!    are sealed into a Lite-internal task-run envelope bound to the route,
//!    attempt, journal and accounting records of the underlying governed run.
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

const VALIDATOR_COMMAND: &str = "cargo test --offline";
const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(240);

// ---------------------------------------------------------------------------
// Loopback model fixture (same contract as tests/scenario_fixtures.rs)
// ---------------------------------------------------------------------------

struct LoopbackModel {
    endpoint: String,
    receiver: mpsc::Receiver<()>,
}

fn spawn_loopback_model(response_body: String) -> LoopbackModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds loopback");
    let addr = listener.local_addr().expect("fixture address");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => return,
        };
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
// Validator execution (real command; pinned; bounded)
// ---------------------------------------------------------------------------

struct ValidatorSpec {
    program: String,
    args: Vec<String>,
    command_display: String,
}

impl ValidatorSpec {
    fn pinned_cargo_test() -> Self {
        Self {
            program: "cargo".to_string(),
            args: vec!["test".to_string(), "--offline".to_string()],
            command_display: VALIDATOR_COMMAND.to_string(),
        }
    }

    fn missing_tool() -> Self {
        Self {
            program: "lite-no-such-cargo-136".to_string(),
            args: vec!["test".to_string()],
            command_display: "lite-no-such-cargo-136 test".to_string(),
        }
    }
}

fn run_validator(
    spec: &ValidatorSpec,
    worktree: &Path,
    target_dir: &Path,
) -> prometheos_lite::workflow::scenario::TaskValidatorRecord {
    let result = Command::new(&spec.program)
        .args(&spec.args)
        .current_dir(worktree)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_NET_OFFLINE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match result {
        Err(_) => prometheos_lite::workflow::scenario::TaskValidatorRecord {
            command: spec.command_display.clone(),
            exit_code: None,
            timed_out: false,
        },
        Ok(mut child) => {
            let deadline = Instant::now() + VALIDATOR_TIMEOUT;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return prometheos_lite::workflow::scenario::TaskValidatorRecord {
                            command: spec.command_display.clone(),
                            exit_code: status.code(),
                            timed_out: false,
                        };
                    }
                    Ok(None) => {
                        if Instant::now() > deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return prometheos_lite::workflow::scenario::TaskValidatorRecord {
                                command: spec.command_display.clone(),
                                exit_code: None,
                                timed_out: true,
                            };
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => {
                        return prometheos_lite::workflow::scenario::TaskValidatorRecord {
                            command: spec.command_display.clone(),
                            exit_code: None,
                            timed_out: false,
                        };
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Task-execution runner (test adapter around the real governed boundary)
// ---------------------------------------------------------------------------

struct TaskExecutionRun {
    dispatch: Result<RoutedDispatchOutcome, RoutedDispatchFailure>,
    accounting: Option<AttemptAccountingV1>,
    envelope: Option<LiteTaskRunEvidence>,
    classification: ScenarioOutcomeClass,
    repaired_lib_rs: Option<String>,
    changed_vs_baseline: Vec<String>,
    patched: bool,
    /// Pre-effect refusal that happened before any staged work (e.g. the
    /// fixture content did not match the pin). Carries the reason.
    preflight_error: Option<String>,
    stage_error: Option<String>,
    acquire_error: Option<String>,
    patch_error: Option<String>,
    staging_digest_ok: bool,
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

/// Execute one task fixture end to end through the governed boundary.
/// `edits` are the fixture-scripted repair operations; `validator` selects
/// the validator command (pinned `cargo test --offline` in the happy path).
fn run_task_execution(
    descriptor_text: &str,
    edits: Vec<EditOperation>,
    validator: ValidatorSpec,
) -> TaskExecutionRun {
    let descriptor = ScenarioDescriptor::parse_json(descriptor_text)
        .expect("descriptor must validate before running");
    let fixture = fixture_root(&descriptor);
    if let Err(error) =
        scenario::verify_fixture_content(&fixture, &descriptor.repository.content_digest)
    {
        // Pre-effect refusal: the pinned bytes are not the committed bytes.
        // No staging, no dispatch, no effects of any kind.
        return TaskExecutionRun {
            dispatch: Err(RoutedDispatchFailure::Refused(anyhow::anyhow!(
                "fixture content pin rejected: {error:#}"
            ))),
            accounting: None,
            envelope: None,
            classification: ScenarioOutcomeClass::SetupError,
            repaired_lib_rs: None,
            changed_vs_baseline: vec![],
            patched: false,
            preflight_error: Some(format!("{error:#}")),
            stage_error: None,
            acquire_error: None,
            patch_error: None,
            staging_digest_ok: false,
        };
    }

    let staging_root = tempfile::tempdir().expect("staging workspace");
    let source = staging_root.path().join("source");
    let workspaces = staging_root.path().join("workspaces");

    let mut stage_error = None;
    let mut staging_digest_ok = false;
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
            }
        }
        Err(error) => stage_error = Some(error),
    }

    // ---- governed dispatch through the real boundary (loopback model) ----
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
    let workspace_manifest = build_workspace_manifest(&descriptor);
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
            return TaskExecutionRun {
                dispatch: Err(RoutedDispatchFailure::Refused(error)),
                accounting: None,
                envelope: None,
                classification: scenario::classify_pre_effect_refusal(),
                repaired_lib_rs: None,
                changed_vs_baseline: vec![],
                patched: false,
                preflight_error: None,
                stage_error,
                acquire_error: None,
                patch_error: None,
                staging_digest_ok,
            };
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

    // ---- workspace acquire under the #171 contract (pre-effect gate) ----
    let adapter = GitWorktreeWorkspace {
        parent_dir: workspaces,
        source_repo: source.clone(),
    };
    let mut acquire_error = None;
    let mut acquired = None;
    if stage_error.is_none() {
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
                } else if let Ok(digest) = scenario::task_run_tree_digest(&acquired_workspace.root)
                {
                    if digest != descriptor.repository.content_digest {
                        acquire_error = Some(format!(
                            "worktree source digest {digest} != pinned fixture {}",
                            descriptor.repository.content_digest
                        ));
                    }
                } else {
                    acquire_error = Some("worktree source digest unreadable".to_string());
                }
                if acquire_error.is_some() {
                    acquired = None;
                } else {
                    acquired = Some(acquired_workspace);
                }
            }
            Err(error) => acquire_error = Some(format!("{error:#}")),
        }
    }

    // ---- baseline digest (pre-effect) -------------------------------------
    let digest_root = acquired
        .as_ref()
        .map(|a| a.root.clone())
        .unwrap_or_else(|| source.clone());
    let lib_rs = digest_root.join("src").join("lib.rs");
    let lib_rs_bytes = std::fs::read(&lib_rs).ok();
    let baseline_file_digest = lib_rs_bytes.as_deref().map(sha256_hex);
    let baseline_digest = scenario::task_run_tree_digest(&digest_root).ok();

    // ---- the controlled repair action (fixture-scripted; permitted) ------
    let mut patch_error = None;
    let mut patched = false;
    let mut changed_files: Vec<String> = vec![];
    let mut file_before_digest = String::new();
    let mut file_after_digest = String::new();
    let mut worktree_root = source.clone();
    if let Some(a) = &acquired {
        worktree_root = a.root.clone();
        file_before_digest = baseline_file_digest.clone().unwrap_or_default();
        let mut policy = FilePolicy::strict();
        policy.repo_root = a.root.clone();
        policy.allowed_write_paths = vec![PathBuf::from("src/lib.rs")];
        // The dispatch itself holds NO write authority; the repair write is
        // gated here by the workspace boundary + the file policy + the
        // editable set, not by the model's authority.
        let file_set = FileSet {
            editable: vec![a.root.join(Path::new("src").join("lib.rs"))],
            readonly: vec![],
            generated: vec![],
            artifacts: vec![],
            denied: vec![],
            binary: vec![],
        };
        match dispatch_resolved_route_block_on(apply_patch_with_rollback(
            &edits, &file_set, &policy,
        )) {
            Ok((patch_result, _rollback)) => {
                if patch_result.applied && patch_result.failures.is_empty() {
                    patched = true;
                    changed_files = patch_result
                        .changed_files
                        .iter()
                        .map(|p| {
                            repo_relative(&a.root, p)
                                .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/"))
                        })
                        .collect();
                } else {
                    patch_error = Some(format!(
                        "repair patch not applied: {}",
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
        file_after_digest = std::fs::read(digest_root.join("src").join("lib.rs"))
            .map(|b| sha256_hex(&b))
            .unwrap_or_default();
    } else if let Some(pre) = baseline_file_digest {
        // Refused before the worktree existed or before any write: the tree
        // is untouched, so the repair record carries the unchanged digest.
        file_before_digest = pre.clone();
        file_after_digest = pre;
    }

    // ---- the real validator (only when a permitted repair was applied) ---
    let validator_record = if patched {
        let target_dir = staging_root.path().join("validator-target");
        run_validator(&validator, &worktree_root, &target_dir)
    } else {
        prometheos_lite::workflow::scenario::TaskValidatorRecord {
            command: validator.command_display.clone(),
            exit_code: None,
            timed_out: false,
        }
    };

    // ---- changed set on disk (guards against out-of-scope effects) -------
    let changed_vs_baseline: Vec<String> = match &acquired {
        Some(a) => tree_changed_paths(&source, &a.root),
        None => vec![],
    };

    // ---- post-run digest --------------------------------------------------
    let post_run_digest = match &acquired {
        Some(a) => scenario::task_run_tree_digest(&a.root).ok(),
        None => baseline_digest.clone(),
    };

    // ---- classification -----------------------------------------------------
    let classification =
        if stage_error.is_some() || acquire_error.is_some() || patch_error.is_some() {
            // Pre-effect refusal or a repair that could not be applied: no
            // validator ran, no product outcome was reached.
            ScenarioOutcomeClass::SetupError
        } else if validator_record.exit_code.is_none() {
            // Validator could not complete (missing tool / killed on timeout).
            ScenarioOutcomeClass::InfrastructureError
        } else {
            dispatch_classification
        };

    // ---- seal the task-run envelope (only with full dispatch evidence) ---
    let envelope = match (
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
                        Ok(task_run) => Some(task_run),
                        Err(error) => {
                            panic!("happy-path task-run envelope failed to seal: {error:#}")
                        }
                    }
                }
                Err(error) => panic!("scenario-run envelope failed to seal: {error:#}"),
            }
        }
        _ => None,
    };

    // Read the patched source before the staging root is dropped, so tests
    // can assert the on-disk result without keeping the tree alive.
    let repaired_lib_rs = match &acquired {
        Some(a) => std::fs::read_to_string(a.root.join("src").join("lib.rs")).ok(),
        None => None,
    };

    TaskExecutionRun {
        dispatch,
        accounting,
        envelope,
        classification,
        repaired_lib_rs,
        changed_vs_baseline,
        patched,
        preflight_error: None,
        stage_error,
        acquire_error,
        patch_error,
        staging_digest_ok,
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
    let run = run_task_execution(
        &tampered,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.preflight_error.is_some());
    assert!(!run.patched);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
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
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
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
    let dispatch = run.dispatch.as_ref().expect("dispatch sealed");
    assert_eq!(dispatch.attempt.outcome_kind, "completed");
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
    assert_eq!(envelope.validator.command, VALIDATOR_COMMAND);
    assert_eq!(envelope.validator.exit_code, Some(0));
    assert!(!envelope.validator.timed_out);
    assert_ne!(
        envelope.baseline_content_digest, envelope.post_run_content_digest,
        "post-run digest must reflect the repair"
    );
    assert_eq!(envelope.repair.origin, TaskRepairOrigin::ScriptedFixture);
    assert!(envelope.repair.applied);
    assert_eq!(envelope.repair.changed_files, vec!["src/lib.rs"]);
    assert_eq!(envelope.namespace, "lite.evidence.task-run");
    assert_eq!(envelope.schema_version, "1.0.0");
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
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
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
    let first = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    let second = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    let first = first.envelope.expect("first run seals");
    let second = second.envelope.expect("second run seals");
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
    assert_eq!(first.validator.exit_code, second.validator.exit_code);
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
    // worktree acquisition must fail closed, before any effect.
    let tampered =
        EXECUTE_DESCRIPTOR.replace(EXECUTE_REVISION, "bfca0ee96857e365026ac07f9a95c514b7e58e8c");
    assert_ne!(tampered, EXECUTE_DESCRIPTOR);
    let run = run_task_execution(
        &tampered,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    assert!(run.stage_error.is_some() || run.acquire_error.is_some());
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(
        !run.patched,
        "no repair happens when the workspace is refused"
    );
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    let envelope = run
        .envelope
        .as_ref()
        .expect("envelope still seals with typed markers");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairRefused);
    assert_eq!(envelope.validator.exit_code, None);
    assert_eq!(
        envelope.baseline_content_digest, envelope.post_run_content_digest,
        "a pre-effect refusal must leave the tree digest at the baseline"
    );
    envelope.verify().expect("refusal envelope verifies");
}

#[test]
fn unresolvable_workspace_revision_never_reaches_the_validator() {
    let tampered =
        EXECUTE_DESCRIPTOR.replace(EXECUTE_REVISION, "0000000000000000000000000000000000000000");
    let run = run_task_execution(
        &tampered,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert!(run.acquire_error.is_some() || run.stage_error.is_some());
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
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        wrong_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    assert_eq!(run.acquire_error, None);
    assert_eq!(run.patch_error, None, "wrong patch still applies");
    assert!(run.patched);
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
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::missing_tool(),
    );
    assert!(run.patched, "the permitted patch was applied");
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
}

#[test]
fn inapplicable_repair_is_setup_error_with_zero_effects() {
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        inapplicable_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    assert!(!run.patched);
    assert!(
        run.patch_error.is_some(),
        "search block absent must not apply"
    );
    assert_eq!(run.classification, ScenarioOutcomeClass::SetupError);
    assert_eq!(run.changed_vs_baseline, Vec::<String>::new());
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.task_outcome, TaskOutcome::RepairRefused);
    assert_eq!(
        envelope.baseline_content_digest,
        envelope.post_run_content_digest
    );
}

// ---------------------------------------------------------------------------
// Envelope integrity: forged claims of success are rejected
// ---------------------------------------------------------------------------

fn happy_path_envelope() -> LiteTaskRunEvidence {
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        correct_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
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
    let run = run_task_execution(
        EXECUTE_DESCRIPTOR,
        wrong_repair_edits(),
        ValidatorSpec::pinned_cargo_test(),
    );
    let envelope = run.envelope.expect("envelope seals");
    // Spliced success: claim repaired while the validator exit code is not 0.
    let forged = reseal(envelope, |e| {
        e.task_outcome = TaskOutcome::Repaired;
    });
    assert!(forged.verify().is_err());
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
