//! #136 Slice D — genuine model-authored pilot runner.
//!
//! This module wires the **real, governed** boundary end to end for a
//! model-authored repair pilot (see `tests/real_model_pilot.rs`):
//!
//!   phase B  — the pre-fix failure is proven by actually running the
//!              pinned validator (`cargo test --offline`) in a throwaway
//!              copy of the staged source; the expected failure is
//!              recorded, never assumed;
//!   phase C  — a chat-completion model endpoint (a loopback gateway or
//!              an offline canned responder) is routed through the normal
//!              `resolve_route` → `dispatch_resolved_route` governed path.
//!              The model sees ONLY the task statement, the repository
//!              source, and the public failure signal. It investigates
//!              and authors its own edits as JSON search-replace
//!              operations; bounded attempts (max [`MAX_ATTEMPTS`]) with
//!              the failing validator output fed back between attempts;
//!   phase D  — the proposed edits are preflighted read-only (repo
//!              relative path, inside the editable set — `src/bucket.rs`
//!              only — dry-run patch against live bytes) BEFORE the
//!              workspace write authority is exercised. The governed
//!              model dispatch itself holds no write authority; the
//!              applied edits are bound to the model's raw response and
//!              to the parsed proposal via [`ModelPatchProvenance`];
//!   phase E  — the pinned validator runs for real; the envelope is
//!              sealed only from measured evidence. A re-sealed claim
//!              conflicting with the trusted pins, or with the model
//!              provenance, is rejected at verification.
//!
//! Honesty boundary: canned responders exist so the offline suite can
//! exercise the machinery deterministically; they are **not** model
//! output, and nothing in this module presents them as such. A genuine
//! run requires the env-gated live path documented in
//! `docs/guides/governed-workflow-evaluation-fixtures.md`.
//!
//! Nothing here weakens any Slice A/B/C gate: the same preflight order,
//! the same workspace/file-policy permit, the same sealing rules apply.

#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use prometheos_lite::harness::edit_protocol::{EditOperation, SearchReplaceEdit};
use prometheos_lite::harness::file_control::{FilePolicy, FileSet};
use prometheos_lite::harness::patch_applier::apply_patch_with_rollback;
use prometheos_lite::workflow::accounting::{
    AccountingContext, AccountingLedger, AttemptAccountingV1, PricingBook, account_routed_failure,
    account_routed_outcome,
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
    FixedTrustedTimeSource, RoutedDispatchFailure, RoutedDispatchOutcome,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteResolution, RouteResolutionRequest, RouteTrust, RouteWorkspaceBinding,
    resolve_route,
};
use prometheos_lite::workflow::scenario::{
    self, LiteTaskRunEvidence, ModelPatchProvenance, ScenarioDescriptor, ScenarioOutcomeClass,
    TaskRepairOrigin, TaskRepairProvenanceKind, TaskRepairRecord, TaskRepairWriteAuthority,
    TaskValidatorProvenance, TaskValidatorRecord,
};
use prometheos_lite::workflow::soma::capability::Locality;
use prometheos_lite::workflow::soma::capability::{
    SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::workspace::{self, GitWorktreeWorkspace, WorkspaceManifestV1};

use super::task_execution::{
    GRAPH_ID, NODE_ID, STEP_ID, TaskExecutionSpec, build_workspace_manifest,
    dispatch_resolved_route_block_on, fixture_root, repair_patch_digest, repair_preflight,
    repo_relative, sha256_hex, stage_source_repository, tree_changed_paths,
};

/// Deterministic facts for this fixture. The single editable file is
/// `src/bucket.rs` (least privilege: the reference repair touches only
/// that file); the solver may read the whole repository but may edit
/// nothing else.
pub const MODEL_PILOT_SPEC: TaskExecutionSpec = TaskExecutionSpec {
    repair_target: "src/bucket.rs",
    repo_identity_url: "https://repo.invalid/prometheos/lite-fixtures/rust-tokenbucket-bugfix-model-001.git",
};

/// Bounded iteration: the model gets at most this many attempts.
pub const MAX_ATTEMPTS: u32 = 3;
/// The one host the operator may authorize for the live pilot's egress,
/// by explicit per-run grant; everything else stays denied.
pub const LIVE_EGRESS_HOST: &str = "openrouter.ai";
/// The effective network allowlist granted to an authorized live run.
/// Anything not listed here is refused.
pub const LIVE_EGRESS_ALLOWLIST: &[&str] = &["openrouter.ai"];
/// Cap on the validator output fed back to the model between attempts.
const FEEDBACK_CAP: usize = 3_000;
/// Timeout for one validator subprocess.
const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(240);

/// Price book for the pilot. The `demo-local` entry matches the Slice
/// B/C example operator book; the `openrouter-via-loopback-gateway`
/// entries price both operator-selected free upstreams at 0/0 because
/// they are listed by OpenRouter at prompt/completion price 0 at the
/// time of pinning. Neither entry is a market price claim; if the
/// upstream pricing ever changes the operator must update the book or
/// the cost is reported Unknown.
pub const PILOT_EXAMPLE_PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "example-operator-price-book",
    "version": "example-2026-11",
    "entries": [
        { "provider": "demo-local", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 },
        { "provider": "openrouter-via-loopback-gateway", "model": "north-mini-code-free", "currency": "USD",
          "promptMinorPer1k": 0, "completionMinorPer1k": 0 },
        { "provider": "openrouter-via-loopback-gateway", "model": "nemotron-ultra-550b-free", "currency": "USD",
          "promptMinorPer1k": 0, "completionMinorPer1k": 0 }
    ]
}"#;

/// One attempt, preserved verbatim whether it failed or succeeded.
#[derive(Debug, Clone)]
pub struct PilotAttempt {
    pub attempt: u32,
    /// sha256 of the raw model content bytes (before any parsing).
    pub model_response_digest: Option<String>,
    /// Canonical digest of the parsed edit operations the model proposed.
    /// Equal to `repair.patch_digest` when the proposal applied.
    pub proposal_digest: Option<String>,
    pub edit_count: usize,
    pub repair_preflight: Option<Result<(), String>>,
    pub applied: bool,
    pub validator_exit_code: Option<i32>,
    pub validator_provenance: Option<TaskValidatorProvenance>,
    /// Typed failure for this attempt, when any.
    pub failure: Option<String>,
    /// The raw model content (bounded by the harness's output cap).
    /// Preserved so intermediate failed attempts are inspectable.
    pub model_response_raw: Option<String>,
    /// The edits parsed from the model's proposal, when parsing succeeded.
    pub proposed_edits: Vec<EditOperation>,
    /// Idempotency key of THIS attempt's routed dispatch (absent when the
    /// dispatch was refused before effect).
    pub idempotency_key: Option<String>,
}

/// The pre-fix proof: an actual validator run against a throwaway copy
/// of the staged source, before any model dispatch. `output_tail` keeps
/// the last bytes of the failing validator log — the same public failure
/// signal a CI job would show.
#[derive(Debug, Clone)]
pub struct PreFixProbe {
    pub command: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub output_digest: Option<String>,
    pub output_bytes: u64,
    pub timed_out: bool,
    pub output_tail: String,
}

/// What the caller's endpoint closure returns per attempt.
pub struct PilotEndpoint {
    pub endpoint: String,
    pub effects: Arc<AtomicU64>,
}

pub struct ModelPilotRun {
    pub attempts: Vec<PilotAttempt>,
    pub dispatch: Option<Result<RoutedDispatchOutcome, RoutedDispatchFailure>>,
    pub accounting: Option<AttemptAccountingV1>,
    pub envelope: Option<LiteTaskRunEvidence>,
    pub seal_error: Option<String>,
    pub classification: ScenarioOutcomeClass,
    pub capability_effects: u64,
    pub patched: bool,
    pub changed_vs_baseline: Vec<String>,
    pub repaired_file_contents: Option<String>,
    pub baseline_digest: Option<String>,
    pub post_run_digest: Option<String>,
    pub pre_fix: Option<PreFixProbe>,
    pub preflight_error: Option<String>,
    pub stage_error: Option<String>,
    pub acquire_error: Option<String>,
    pub staging_digest_ok: bool,
}

/// Files the solver is permitted to know about (the whole repo minus
/// `.git`). Repo-relative, forward slashes, sorted by path.
pub fn solver_visible_files(root: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("repo dir readable")
            .collect::<Result<_, _>>()
            .expect("entries readable");
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().expect("file type");
            if file_type.is_symlink() {
                continue; // refused by the fixture digest anyway
            }
            if file_type.is_dir() {
                if entry.file_name() == ".git" {
                    continue;
                }
                walk(root, &path, out);
            } else if file_type.is_file() {
                let rel = repo_relative(root, &path).expect("repo-relative path");
                let content = std::fs::read_to_string(&path).expect("utf8 fixture file");
                out.push((rel, content));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Build the exact chat messages the solver sees. Exposed so the
/// leakage-guard test can enumerate the solver's entire visible corpus.
pub fn build_solver_messages(
    descriptor: &ScenarioDescriptor,
    files: &[(String, String)],
    failure_signal: &str,
    feedback: Option<&str>,
) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    messages.push(json!({
        "role": "system",
        "content": "You are a precise coding agent in a pinned, disposable workspace. Investigate the repository, find the defect, and reply with ONLY a JSON object of this exact shape: {\"edits\":[{\"file\":\"src/bucket.rs\",\"search\":\"<exact bytes to find>\",\"replace\":\"<exact replacement bytes>\"}]}. \"file\" must be exactly \"src/bucket.rs\" — you may edit NO other file. Output no prose, no markdown fences, no explanations."
    }));
    messages.push(json!({
        "role": "user",
        "content": format!(
            "TASK:\n{statement}\n\nPUBLIC FAILURE SIGNAL (from `cargo test --offline` before any fix):\n{failure_signal}",
            statement = descriptor.task.statement,
        )
    }));
    for (path, content) in files {
        messages.push(json!({
            "role": "user",
            "content": format!("FILE {path} ({} bytes):\n{content}", content.len())
        }));
    }
    if let Some(feedback) = feedback {
        messages.push(json!({
            "role": "user",
            "content": format!("Your previous attempt did not pass validation. Validator output (truncated to {FEEDBACK_CAP} bytes):\n{feedback}\nReply with the corrected JSON edits only, same shape.")
        }));
    }
    messages
}

/// Extract the assistant content string from a governed local-model
/// outcome, exactly as the harness recorded it. The governed result
/// document has the shape
/// `{"success": true, "status": "ok", "output": {"harnessId": …,
///   "capability": …, "content": {…provider document…}}}`
/// where the provider document carries the assistant message at its own
/// `content` (a string in chat-completions mode).
pub fn model_content_from_outcome(outcome: &RoutedDispatchOutcome) -> Option<String> {
    let root: Value = serde_json::from_str(&outcome.outcome.output).ok()?;
    let content = root.get("output")?.get("content")?;
    let message = match content {
        Value::String(s) => return Some(s.clone()),
        Value::Object(_) => content.get("content")?,
        _ => return None,
    };
    match message {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Parse the model's JSON proposal into edit operations, enforcing the
/// single-editable-file boundary at parse time. Returns a typed refusal
/// string on any defect; the caller must not mutate the workspace.
pub fn parse_model_proposal(content: &str) -> Result<Vec<EditOperation>, String> {
    let trimmed = content.trim();
    // Tolerate markdown code fences around the JSON object.
    let candidate = if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest).trim_start();
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    };
    // Fall back to the outermost { ... } span when the model adds prose.
    let candidate = if serde_json::from_str::<Value>(candidate).is_ok() {
        candidate
    } else {
        match (candidate.find('{'), candidate.rfind('}')) {
            (Some(start), Some(end)) if end > start => &candidate[start..=end],
            _ => return Err("model response contains no JSON object".to_string()),
        }
    };
    let value: Value = serde_json::from_str(candidate)
        .map_err(|e| format!("model response is not valid JSON: {e}"))?;
    let edits = value
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| "model response is missing the `edits` array".to_string())?;
    if edits.is_empty() {
        return Err("model response proposed zero edits".to_string());
    }
    if edits.len() > 8 {
        return Err(format!(
            "model response proposed too many edits ({})",
            edits.len()
        ));
    }
    let mut out: Vec<EditOperation> = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        let file = edit
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edit #{index} is missing `file`"))?;
        if file != MODEL_PILOT_SPEC.repair_target {
            return Err(format!(
                "edit #{index} targets {file:?}, outside the authorized editable set ({})",
                MODEL_PILOT_SPEC.repair_target
            ));
        }
        let search = edit
            .get("search")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edit #{index} is missing `search`"))?;
        let replace = edit
            .get("replace")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edit #{index} is missing `replace`"))?;
        if search.is_empty() {
            return Err(format!("edit #{index} has an empty `search` string"));
        }
        if search.len() > 4096 || replace.len() > 4096 {
            return Err(format!("edit #{index} exceeds the 4096-byte edit bound"));
        }
        out.push(EditOperation::SearchReplace(SearchReplaceEdit {
            file: PathBuf::from(file),
            search: search.to_string(),
            replace: replace.to_string(),
            replace_all: None,
            context_lines: None,
        }));
    }
    Ok(out)
}

/// Canonical digest of the parsed proposal (the edit operations list).
/// Binds the applied patch to the model's proposal.
pub fn proposal_digest(edits: &[EditOperation]) -> String {
    repair_patch_digest(edits)
}

fn scripted_repair_grant_model(worktree: &Path) -> (FilePolicy, FileSet) {
    let repair_target = MODEL_PILOT_SPEC.repair_target;
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

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Validator run that also keeps the captured output bytes (bounded),
/// so the pilot can feed the failing output back to the model and the
/// report can quote the failure truthfully.
fn run_validator_capture(
    pins: &scenario::ValidationPin,
    worktree: &Path,
    target_dir: &Path,
) -> (TaskValidatorRecord, Vec<u8>, Vec<u8>) {
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
    const CAP: usize = 100_000;
    fn drain(reader: impl Read) -> (Vec<u8>, u64) {
        let mut buf = Vec::new();
        let mut total = 0u64;
        let mut chunk = [0u8; 8192];
        let mut reader = reader;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    total += n as u64;
                    if buf.len() < CAP {
                        let keep = (CAP - buf.len()).min(n);
                        buf.extend_from_slice(&chunk[..keep]);
                    }
                }
            }
        }
        (buf, total)
    }
    match result {
        Err(_) => (
            TaskValidatorRecord {
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
            Vec::new(),
            Vec::new(),
        ),
        Ok(mut child) => {
            let out_thread = child.stdout.take().map(|r| thread::spawn(move || drain(r)));
            let err_thread = child.stderr.take().map(|r| thread::spawn(move || drain(r)));
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
            (
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
                },
                out_bytes,
                err_bytes,
            )
        }
    }
}

fn pre_fix_probe(descriptor: &ScenarioDescriptor, staged_source: &Path) -> PreFixProbe {
    let probe_root = tempfile::tempdir().expect("pre-fix probe workspace");
    let probe_src = probe_root.path().join("source");
    copy_dir(staged_source, &probe_src).expect("pre-fix probe copy");
    let pins = descriptor.validation.clone().expect("validation pins");
    let target_dir = probe_root.path().join("target");
    let (record, out_bytes, err_bytes) = run_validator_capture(&pins, &probe_src, &target_dir);
    let mut merged = Vec::with_capacity(out_bytes.len() + err_bytes.len());
    merged.extend_from_slice(&out_bytes);
    merged.extend_from_slice(&err_bytes);
    const TAIL: usize = 2_000;
    let tail = merged;
    let start = tail.len().saturating_sub(TAIL);
    // The tail of the captured log can split a UTF-8 boundary; use lossy
    // conversion (the log is ASCII in practice).
    let output_tail = String::from_utf8_lossy(&tail[start..]).into_owned();
    PreFixProbe {
        command: record.command,
        args: record.args,
        exit_code: record.exit_code,
        output_digest: record.output_digest,
        output_bytes: record.output_bytes,
        timed_out: record.timed_out,
        output_tail,
    }
}

fn route_candidate(descriptor: &ScenarioDescriptor) -> RouteCandidateV1 {
    RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: descriptor.route.candidate_id.clone(),
        provider_identity: descriptor.route.provider_identity.clone(),
        provider_locality: match descriptor.route.provider_locality.as_str() {
            "local" => Locality::Local,
            "remote" => Locality::Remote,
            other => panic!("unsupported provider locality in fixture: {other}"),
        },
        model: Some(descriptor.route.model.clone()),
        harness_id: descriptor.harness.id.clone(),
        harness_revision: descriptor.harness.revision.clone(),
        workspace: Some(RouteWorkspaceBinding {
            adapter: workspace::AdapterKind::GitWorktree,
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
    }
}

fn preflight_refusal(
    classification: ScenarioOutcomeClass,
    preflight_error: Option<String>,
    staging_digest_ok: bool,
) -> ModelPilotRun {
    ModelPilotRun {
        attempts: vec![],
        dispatch: None,
        accounting: None,
        envelope: None,
        seal_error: None,
        classification,
        capability_effects: 0,
        patched: false,
        changed_vs_baseline: vec![],
        repaired_file_contents: None,
        baseline_digest: None,
        post_run_digest: None,
        pre_fix: None,
        preflight_error,
        stage_error: None,
        acquire_error: None,
        staging_digest_ok,
    }
}

/// Run the model-authored pilot end to end. `spawn_endpoint` is called
/// once per attempt with the 1-based attempt number and must return a
/// loopback endpoint handle; `effects` is the caller's capability-effect
/// counter for that endpoint (the runner aggregates them). `profile`
/// selects the route profile; `provenance_kind` declares how the model
/// response is observed — `CannedTest` for the offline suite,
/// `OperatorObservedLive` only for the authorized real-model path.
pub fn run_model_pilot(
    descriptor_text: &str,
    price_book: &str,
    dispatch_timeout_ms: u64,
    profile: prometheos_lite::workflow::route_resolution::RouteProfile,
    provenance_kind: TaskRepairProvenanceKind,
    spawn_endpoint: &mut dyn FnMut(u32) -> PilotEndpoint,
) -> ModelPilotRun {
    let spec = MODEL_PILOT_SPEC;
    let repair_target = spec.repair_target;
    let descriptor = ScenarioDescriptor::parse_json(descriptor_text)
        .expect("descriptor must validate before running");

    // ---- preflight: committed fixture bytes are pinned -----------------
    let fixture = fixture_root(&descriptor);
    if let Err(error) =
        scenario::verify_fixture_content(&fixture, &descriptor.repository.content_digest)
    {
        return ModelPilotRun {
            acquire_error: None,
            ..preflight_refusal(
                ScenarioOutcomeClass::SetupError,
                Some(format!("{error:#}")),
                false,
            )
        };
    }
    if descriptor.validation.is_none() {
        return preflight_refusal(
            ScenarioOutcomeClass::InvalidTask,
            Some("descriptor pins no `validation` section".to_string()),
            false,
        );
    }

    // ---- stage the disposable source ------------------------------------
    let staging_root = tempfile::tempdir().expect("staging workspace");
    let source = staging_root.path().join("source");
    let workspaces = staging_root.path().join("workspaces");
    let mut stage_error = None;
    let mut staging_digest_ok = false;
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
    if let Some(error) = stage_error {
        return ModelPilotRun {
            stage_error: Some(error),
            ..preflight_refusal(ScenarioOutcomeClass::SetupError, None, staging_digest_ok)
        };
    }

    // ---- acquire the isolated writable worktree --------------------------
    let workspace_manifest = build_workspace_manifest(&spec, &descriptor);
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
    if let Some(error) = acquire_error {
        return ModelPilotRun {
            acquire_error: Some(error),
            ..preflight_refusal(ScenarioOutcomeClass::SetupError, None, staging_digest_ok)
        };
    }
    let worktree_root = acquired
        .as_ref()
        .map(|a| a.root.clone())
        .expect("acquired worktree root");
    let baseline_digest = scenario::task_run_tree_digest(&worktree_root).ok();

    // ---- phase B: prove the pre-fix failure (real validator run) --------
    let pre_fix = Some(pre_fix_probe(&descriptor, &source));
    let failure_signal = pre_fix
        .as_ref()
        .map(|p| {
            format!(
                "`{} {}` exited with code {} (expected 0). Last of the failing log:\n{}",
                p.command,
                p.args.join(" "),
                p.exit_code.unwrap_or(-1),
                p.output_tail
            )
        })
        .unwrap_or_else(|| "validator did not run".to_string());

    // ---- route setup (shared across attempts) ----------------------------
    let files = solver_visible_files(&worktree_root);
    let locality_allowed: Vec<&str> = match descriptor.route.provider_locality.as_str() {
        "local" => vec!["local"],
        "remote" => vec!["local", "remote"],
        other => panic!("unsupported provider locality in fixture: {other}"),
    };
    let requirements: WorkRequirements = serde_json::from_value(json!({
        "id": "reqs-model-pilot",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": descriptor.requirements.degradation_permitted,
        "forbidSubstitution": descriptor.requirements.forbid_substitution,
        "requiredCapabilities": [],
        "locality": { "allowedExecution": locality_allowed }
    }))
    .expect("requirements parse");
    let authority: AuthorityProfile = match provenance_kind {
        // An externally authorized live run carries an explicit,
        // effective network policy permitting exactly the pinned host;
        // everything else stays denied.
        TaskRepairProvenanceKind::OperatorObservedLive => serde_json::from_value(json!({
            "executionClass": "deterministic",
            "mutation": "none",
            "networkPolicy": { "default": "deny", "allowlist": LIVE_EGRESS_ALLOWLIST }
        }))
        .expect("authority parses"),
        // The offline canned path has no external egress at all.
        TaskRepairProvenanceKind::CannedTest => {
            serde_json::from_str(r#"{"executionClass": "deterministic", "mutation": "none"}"#)
                .expect("authority parses")
        }
    };
    let candidate = route_candidate(&descriptor);
    let request = RouteResolutionRequest {
        route_set_id: descriptor.route.route_set_id.clone(),
        route_set_version: descriptor.route.route_set_version.clone(),
        candidates: vec![candidate],
        pinned_candidate_id: Some(&descriptor.route.pinned_candidate_id),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile,
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
            return ModelPilotRun {
                preflight_error: Some(format!("route resolution refused before effect: {error:#}")),
                baseline_digest,
                pre_fix,
                staging_digest_ok,
                ..preflight_refusal(
                    scenario::classify_pre_effect_refusal(),
                    None,
                    staging_digest_ok,
                )
            };
        }
    };
    resolution.evidence.verify().expect("evidence verifies");
    resolution.snapshot.verify().expect("snapshot verifies");

    let manifest = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed model-authored pilot node",
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

    // ---- attempts (bounded) ----------------------------------------------
    let mut attempts: Vec<PilotAttempt> = Vec::new();
    let mut capability_effects: u64 = 0;
    let mut feedback: Option<String> = None;
    let mut final_dispatch: Option<Result<RoutedDispatchOutcome, RoutedDispatchFailure>> = None;
    let mut final_accounting: Option<AttemptAccountingV1> = None;
    let mut changed_files: Vec<String> = Vec::new();
    let mut file_after_digest = String::new();
    // (attempt, before) of the most recent applied patch.
    let mut applied_state: Option<(u32, String)> = None;

    for attempt_number in 1..=MAX_ATTEMPTS {
        let PilotEndpoint { endpoint, effects } = spawn_endpoint(attempt_number);
        let fixture_workspace = tempfile::tempdir().expect("fixture workspace");
        let permit = super::permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
        let mut capability_registry = CapabilityRegistry::new();
        capability_registry.declare(
            LOCAL_MODEL_INFER_CAPABILITY,
            local_model_capability(
                LocalModelConfig::new(endpoint.clone(), descriptor.route.model.clone())
                    .timeout_ms(dispatch_timeout_ms)
                    .max_output_bytes(256 * 1024),
                fixture_workspace.path().to_path_buf(),
            ),
        );
        let mut runner = NodeRunner::new(capability_registry, permit);

        let messages =
            build_solver_messages(&descriptor, &files, &failure_signal, feedback.as_deref());
        let dispatch = dispatch_resolved_route_block_on(
            prometheos_lite::workflow::route_dispatch::dispatch_resolved_route(
                prometheos_lite::workflow::route_dispatch::RoutedDispatchRequest {
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
                            "model": descriptor.route.model,
                            "endpoint": endpoint,
                            "messages": messages,
                            "maxTokens": 2048,
                            "timeoutMs": dispatch_timeout_ms,
                        }),
                    },
                },
            ),
        );
        capability_effects += effects.load(std::sync::atomic::Ordering::SeqCst);

        let accounting_context = AccountingContext {
            graph_id: GRAPH_ID.to_string(),
            node_id: NODE_ID.to_string(),
            currency: Some("USD".to_string()),
        };
        let accounting = match dispatch.as_ref() {
            Ok(outcome) => {
                let pricing = PricingBook::from_json(price_book).expect("price book parses");
                account_routed_outcome(outcome, &resolution, &accounting_context, Some(&pricing))
                    .ok()
                    .inspect(|record| {
                        record.verify().expect("accounting record verifies");
                    })
            }
            Err(RoutedDispatchFailure::ExecutionFailed(failure)) => Some(account_routed_failure(
                &resolution,
                "execution-failed",
                &accounting_context,
                STEP_ID,
                failure.journal_entry.clone(),
                0,
            )),
            Err(RoutedDispatchFailure::Refused(_)) => None,
        };

        let mut attempt = PilotAttempt {
            attempt: attempt_number,
            model_response_digest: None,
            proposal_digest: None,
            edit_count: 0,
            repair_preflight: None,
            applied: false,
            validator_exit_code: None,
            validator_provenance: None,
            failure: None,
            model_response_raw: None,
            proposed_edits: vec![],
            idempotency_key: None,
        };

        match dispatch.as_ref() {
            Ok(outcome) => {
                let content = model_content_from_outcome(outcome).unwrap_or_default();
                attempt.model_response_raw = Some(content.clone());
                attempt.model_response_digest = Some(sha256_hex(content.as_bytes()));
                attempt.idempotency_key = Some(outcome.attempt.idempotency_key.clone());
                final_accounting = accounting;
                match parse_model_proposal(&content) {
                    Ok(edits) => {
                        attempt.edit_count = edits.len();
                        attempt.proposal_digest = Some(proposal_digest(&edits));
                        attempt.proposed_edits = edits.clone();
                        let preflight = repair_preflight(&edits, &worktree_root, repair_target);
                        attempt.repair_preflight = Some(preflight.clone());
                        match preflight {
                            Ok(()) => {
                                let before =
                                    std::fs::read(worktree_root.join(Path::new(repair_target)))
                                        .map(|b| sha256_hex(&b))
                                        .unwrap_or_default();
                                let (policy, file_set) =
                                    scripted_repair_grant_model(&worktree_root);
                                match dispatch_resolved_route_block_on(apply_patch_with_rollback(
                                    &edits, &file_set, &policy,
                                )) {
                                    Ok((patch_result, _rollback)) => {
                                        if patch_result.applied && patch_result.failures.is_empty()
                                        {
                                            attempt.applied = true;
                                            changed_files = patch_result
                                                .changed_files
                                                .iter()
                                                .map(|p| {
                                                    repo_relative(&worktree_root, p).unwrap_or_else(
                                                        |_| p.to_string_lossy().replace('\\', "/"),
                                                    )
                                                })
                                                .collect();
                                            let after = std::fs::read(
                                                worktree_root.join(Path::new(repair_target)),
                                            )
                                            .map(|b| sha256_hex(&b))
                                            .unwrap_or_default();
                                            applied_state = Some((attempt_number, before.clone()));
                                            file_after_digest = after;
                                            let pins = descriptor.validation.clone().expect("pins");
                                            let target_dir = staging_root
                                                .path()
                                                .join(format!("validator-target-{attempt_number}"));
                                            let (record, out_bytes, err_bytes) =
                                                run_validator_capture(
                                                    &pins,
                                                    &worktree_root,
                                                    &target_dir,
                                                );
                                            attempt.validator_exit_code = record.exit_code;
                                            attempt.validator_provenance = Some(record.provenance);
                                            if record.exit_code == Some(pins.expected_exit_code) {
                                                attempts.push(attempt);
                                                final_dispatch = Some(dispatch);
                                                return finish_run(
                                                    descriptor,
                                                    resolution,
                                                    manifest,
                                                    workspace_manifest,
                                                    worktree_root,
                                                    &source,
                                                    attempts,
                                                    capability_effects,
                                                    final_dispatch,
                                                    final_accounting,
                                                    baseline_digest,
                                                    changed_files,
                                                    before,
                                                    file_after_digest,
                                                    pre_fix,
                                                    attempt_number,
                                                    record,
                                                    staging_digest_ok,
                                                    provenance_kind,
                                                );
                                            }
                                            let mut merged = Vec::with_capacity(
                                                out_bytes.len() + err_bytes.len(),
                                            );
                                            merged.extend_from_slice(&out_bytes);
                                            merged.extend_from_slice(&err_bytes);
                                            let text = String::from_utf8_lossy(&merged);
                                            let start = text.len().saturating_sub(FEEDBACK_CAP);
                                            feedback = Some(text[start..].to_string());
                                        } else {
                                            let detail = patch_result
                                                .failures
                                                .iter()
                                                .map(|f| {
                                                    format!("{}: {}", f.file.display(), f.reason)
                                                })
                                                .collect::<Vec<_>>()
                                                .join("; ");
                                            attempt.failure =
                                                Some(format!("repair applied no edit: {detail}"));
                                            attempts.push(attempt);
                                            final_dispatch = Some(dispatch);
                                            break;
                                        }
                                    }
                                    Err(error) => {
                                        attempt.failure =
                                            Some(format!("repair patch refused: {error:#}"));
                                        attempts.push(attempt);
                                        final_dispatch = Some(dispatch);
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                attempt.failure =
                                    Some(format!("repair preflight refused: {error}"));
                                feedback = Some(format!("patch refused before apply: {error}"));
                            }
                        }
                    }
                    Err(error) => {
                        attempt.failure = Some(format!("model proposal rejected: {error}"));
                        feedback = Some(format!("proposal rejected: {error}"));
                    }
                }
                attempts.push(attempt);
                final_dispatch = Some(dispatch);
            }
            Err(RoutedDispatchFailure::Refused(refused)) => {
                attempt.failure = Some(format!(
                    "governed dispatch refused before any effect: {refused:#}"
                ));
                attempts.push(attempt);
                final_dispatch = Some(dispatch);
                break;
            }
            Err(RoutedDispatchFailure::ExecutionFailed(failure)) => {
                attempt.failure = Some(format!("governed effect failed: {}", failure.source));
                attempts.push(attempt);
                final_dispatch = Some(dispatch);
                break;
            }
        }
    }

    // ---- terminal state: seal a RepairFailed envelope if a patch was
    // applied and validated (failed), else report the no-effect outcome.
    let post_run_digest = scenario::task_run_tree_digest(&worktree_root).ok();
    match applied_state {
        Some((winning_attempt, before)) => {
            let pins = descriptor.validation.clone().expect("pins");
            let target_dir = staging_root.path().join("validator-target-final");
            let (record, _out, _err) = run_validator_capture(&pins, &worktree_root, &target_dir);
            finish_run(
                descriptor,
                resolution,
                manifest,
                workspace_manifest,
                worktree_root,
                &source,
                attempts,
                capability_effects,
                final_dispatch,
                final_accounting,
                baseline_digest,
                changed_files,
                before,
                file_after_digest,
                pre_fix,
                winning_attempt,
                record,
                staging_digest_ok,
                provenance_kind,
            )
        }
        None => ModelPilotRun {
            attempts,
            dispatch: final_dispatch,
            accounting: final_accounting,
            envelope: None,
            seal_error: None,
            classification: ScenarioOutcomeClass::InfrastructureError,
            capability_effects,
            patched: false,
            changed_vs_baseline: tree_changed_paths(&source, &worktree_root),
            repaired_file_contents: std::fs::read_to_string(
                worktree_root.join(Path::new(repair_target)),
            )
            .ok(),
            baseline_digest,
            post_run_digest,
            pre_fix,
            preflight_error: None,
            stage_error: None,
            acquire_error: None,
            staging_digest_ok,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_run(
    descriptor: ScenarioDescriptor,
    resolution: RouteResolution,
    manifest: NodeManifestV1,
    workspace_manifest: WorkspaceManifestV1,
    worktree_root: PathBuf,
    source: &Path,
    attempts: Vec<PilotAttempt>,
    capability_effects: u64,
    final_dispatch: Option<Result<RoutedDispatchOutcome, RoutedDispatchFailure>>,
    final_accounting: Option<AttemptAccountingV1>,
    baseline_digest: Option<String>,
    changed_files: Vec<String>,
    file_before_digest: String,
    file_after_digest: String,
    pre_fix: Option<PreFixProbe>,
    winning_attempt: u32,
    validator_record: TaskValidatorRecord,
    staging_digest_ok: bool,
    provenance_kind: TaskRepairProvenanceKind,
) -> ModelPilotRun {
    let dispatch_classification = scenario::classify_completed_run(
        true,
        final_dispatch.as_ref().expect("dispatched"),
        final_accounting.is_some(),
    );
    let graph = final_accounting.as_ref().map(|record| {
        let mut ledger = AccountingLedger::new();
        ledger.record(record.clone()).expect("record registers");
        ledger.aggregate_graph(GRAPH_ID).expect("graph aggregates")
    });
    let post_run_digest = scenario::task_run_tree_digest(&worktree_root).ok();
    let classification = dispatch_classification;
    let validator_provenance = validator_record.provenance;

    let (envelope, seal_error) = match (
        final_dispatch.as_ref(),
        final_accounting.as_ref(),
        graph.as_ref(),
        baseline_digest.clone(),
        post_run_digest.clone(),
    ) {
        (Some(Ok(outcome)), Some(record), Some(graph), Some(baseline), Some(post)) => {
            let winning = attempts
                .iter()
                .find(|a| a.attempt == winning_attempt)
                .expect("winning attempt preserved");
            let winning_edits = winning.proposed_edits.clone();
            let response_digest = winning
                .model_response_digest
                .clone()
                .expect("winning attempt carries a response digest");
            let raw_response = winning
                .model_response_raw
                .clone()
                .expect("winning attempt carries the raw response");
            let winning_idempotency_key = winning
                .idempotency_key
                .clone()
                .expect("winning attempt carries the routed attempt idempotency key");
            let proposal_digest = winning
                .proposal_digest
                .clone()
                .expect("winning attempt carries a proposal digest");
            let harness_revision = descriptor.harness.revision.clone();
            let scenario_run = scenario::LiteScenarioRunEvidence::sealed(
                descriptor,
                &resolution,
                &manifest,
                outcome,
                record.clone(),
                graph.clone(),
                Some(&workspace_manifest),
                classification,
            );
            match scenario_run {
                Ok(scenario_run) => {
                    let repair_record = TaskRepairRecord {
                        origin: TaskRepairOrigin::ModelAuthored,
                        write_authority: TaskRepairWriteAuthority::ModelAuthoredWorkspaceFilePolicy,
                        action: "search-replace".to_string(),
                        changed_files: changed_files.clone(),
                        file_before_digest,
                        file_after_digest,
                        patch_digest: repair_patch_digest(&winning_edits),
                        applied: true,
                        model_provenance: Some(ModelPatchProvenance {
                            model_response_digest: response_digest,
                            proposal_digest,
                            attempt: winning_attempt,
                            model_identity: outcome
                                .attempt
                                .model
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string()),
                            harness_revision: format!(
                                "{}@{}",
                                outcome.attempt.harness_id, harness_revision
                            ),
                            kind: Some(provenance_kind),
                            raw_response: Some(raw_response),
                            attempt_idempotency_key: Some(winning_idempotency_key),
                        }),
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
        ScenarioOutcomeClass::InfrastructureError
    } else {
        classification
    };
    let _ = validator_provenance;

    ModelPilotRun {
        attempts,
        dispatch: final_dispatch,
        accounting: final_accounting,
        envelope,
        seal_error,
        classification,
        capability_effects,
        patched: true,
        changed_vs_baseline: tree_changed_paths(source, &worktree_root),
        repaired_file_contents: std::fs::read_to_string(
            worktree_root.join(Path::new(MODEL_PILOT_SPEC.repair_target)),
        )
        .ok(),
        baseline_digest,
        post_run_digest,
        pre_fix,
        preflight_error: None,
        stage_error: None,
        acquire_error: None,
        staging_digest_ok,
    }
}
