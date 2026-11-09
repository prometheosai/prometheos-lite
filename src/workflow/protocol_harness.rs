//! Governed protocol-bridged execution adapter (issue #154, slice G).
//!
//! [`ProtocolBridgeHarness`] is the third real execution adapter conforming
//! to [`ExecutionHarness`]: it exchanges a single versioned, strict JSON
//! request/response envelope with an independent allowlisted worker process
//! over stdin/stdout. It is NOT a declaration-only entry, NOT a scripted
//! in-memory replay, and NOT a claim that any external agent protocol
//! (Claude Code, Codex, ACP, …) is supported — interoperability with
//! third-party protocols is explicitly NOT claimed.
//!
//! Effect discipline (no bypass around #117/#118):
//!
//! - The adapter never spawns a process itself. Its `run` invokes the
//!   declared `harness.protocol.bridge` capability through
//!   [`HarnessDispatch::call`], the only sanctioned effect path, so every
//!   bridge execution passes the NodeRunner nine-gate pipeline.
//! - The capability handler runs the worker through the SAME governed
//!   subprocess containment as slice E (shared `Containment` primitive —
//!   Windows Job Object with suspended launch / assign-before-resume,
//!   restricted inherited handles; Unix session/process-group containment):
//!   there is no second unmanaged launcher. The handler adds only the
//!   stdin/stdout protocol exchange: one bounded request write, bounded
//!   stdout/stderr drains, deadline, cancellation, and tree termination +
//!   reaping on timeout, cancel, budget breach, or normal completion.
//! - Strict envelope validation: schema/version gates, exact echo of
//!   `requestId` / `correlationId` / `stepId` / `harnessId` /
//!   `harnessRevision` / `capability` / `requestDigest`, rejection of
//!   malformed JSON, missing fields, duplicated responses, and any
//!   `toolRequest` a worker attempts to smuggle (rejected, never executed).
//!   Capability is not authority: a protocol message never invokes a tool
//!   except through a governed dispatch, which this bridge does not do.
//! - Budgets are enforced on every invocation: serialized arguments,
//!   serialized request envelope, stdout+stderr bytes actually written,
//!   and response bytes. Exceeding a budget is a typed refusal.
//! - Structured redaction: the merged known-secret list (registration-time
//!   plus request-scoped) redacts the verified `output` value's string
//!   leaves before serialization, so a secret-bearing worker response
//!   cannot leak into durable evidence. A residual secret after
//!   structural redaction refuses instead of persisting.
//!
//! Portable continuity: the adapter owns no durable work state and no
//! hidden session state. `checkpoint`/`resume` bind the harness identity +
//! revision to the digest of the canonical
//! [`PortableWorkState`](crate::workflow::portable_state::PortableWorkState).
//! Live-process resume is not claimed.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

use crate::workflow::artifact_integrity::sha256_hex;
use crate::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessCheckpointV1, HarnessContinuation,
    HarnessDeclarationV1, HarnessDispatch, HarnessEvent, HarnessFamily, HarnessOutcome,
    HarnessStep,
};
use crate::workflow::node_runner::Capability;
use crate::workflow::redaction::Redactor;
use crate::workflow::soma::capability::{
    AdapterFamily, Identity, Locality, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
};
use crate::workflow::soma::profile::Boundaries;
use crate::workflow::subprocess_harness::{
    DrainHandle, DrainState, SpawnedChild, child_cwd, governed_root, make_pipe, spawn_contained,
    spawn_drain,
};
use crate::workflow::workspace::ADAPTER_REVISION;

/// Implementation-owned harness id. Distinct from the slice-A conformance seam
/// `lite.harness.protocol` (a synthetic `ScriptedProtocolHarness` fixture,
/// not a production adapter).
pub const PROTOCOL_HARNESS_ID: &str = "lite.harness.protocol.bridge";
/// The single declared capability (registry key + declared capability).
pub const PROTOCOL_BRIDGE_CAPABILITY: &str = "harness.protocol.bridge";
/// Identity name/revision of the implementation.
pub const PROTOCOL_IDENTITY_NAME: &str = "lite-protocol-bridge";
pub const PROTOCOL_REVISION: &str = "v1";

/// Protocol envelope schema identifier.
pub const PROTOCOL_SCHEMA: &str = "lite.protocol.bridge";
/// Protocol envelope version. Unsupported versions refuse before any spawn.
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// Poll interval for the child wait loop. Mirrors the subprocess harness.
const POLL: Duration = Duration::from_millis(5);

/// Configuration for the governed protocol-bridge handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode.
#[derive(Clone)]
pub struct ProtocolBridgeConfig {
    /// Exact-match executable allowlist. A requested `exe` must equal one
    /// of these strings; path separators are matched literally.
    pub allowed_executables: Vec<String>,
    /// Default invocation deadline when the step omits `timeoutMs`.
    pub default_timeout_ms: u64,
    /// Hard ceiling for any invocation deadline (1 h).
    pub max_timeout_ms: u64,
    /// Hard budget, in bytes, of stdout+stderr the worker may write during
    /// one invocation. Exceeding it kills the tree and refuses.
    pub max_output_bytes: usize,
    /// Bytes of serialized arguments accepted per invocation.
    pub max_args_bytes: usize,
    /// Bytes of the serialized protocol request envelope accepted per
    /// invocation. Checked before spawn.
    pub max_request_bytes: usize,
    /// Bytes of the protocol response on stdout accepted. Checked before
    /// parsing; exceeding it refuses as an oversized message.
    pub max_response_bytes: usize,
    /// Known secret values, redacted verbatim from the verified response.
    pub known_secrets: Vec<String>,
    /// Cooperative cancellation source; setting it aborts the worker tree.
    pub cancel: Arc<AtomicBool>,
}

impl Default for ProtocolBridgeConfig {
    fn default() -> Self {
        Self {
            allowed_executables: Vec::new(),
            default_timeout_ms: 30_000,
            max_timeout_ms: 3_600_000,
            max_output_bytes: 1024 * 1024,
            max_args_bytes: 64 * 1024,
            max_request_bytes: 64 * 1024,
            max_response_bytes: 64 * 1024,
            known_secrets: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ProtocolBridgeConfig {
    pub fn allowed_executables(mut self, executables: Vec<String>) -> Self {
        self.allowed_executables = executables;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn max_request_bytes(mut self, max_request_bytes: usize) -> Self {
        self.max_request_bytes = max_request_bytes;
        self
    }

    pub fn max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    pub fn known_secrets(mut self, known_secrets: Vec<String>) -> Self {
        self.known_secrets = known_secrets;
        self
    }

    pub fn cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = cancel;
        self
    }
}

/// The implementation-owned, sealed declaration for the protocol bridge.
/// Shared by [`ProtocolBridgeHarness`] and the canonical catalog inventory
/// so the two cannot diverge.
pub fn protocol_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: PROTOCOL_HARNESS_ID.to_string(),
        identity: Identity {
            name: PROTOCOL_IDENTITY_NAME.to_string(),
            revision: PROTOCOL_REVISION.to_string(),
        },
        family: HarnessFamily::ProtocolBridged,
        declared_capabilities: vec![PROTOCOL_BRIDGE_CAPABILITY.to_string()],
        claimed_capabilities: vec![PROTOCOL_BRIDGE_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: "lite.harness.protocol.bridge.caps".to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: PROTOCOL_IDENTITY_NAME.to_string(),
                revision: PROTOCOL_REVISION.to_string(),
            },
            declared_at: "2026-11-08T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: vec![
                AdapterFamily {
                    family: "git-worktree".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
                AdapterFamily {
                    family: "existing-read-only".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
            ],
            effect_classes: vec!["process-exec".to_string()],
            workspace: Some(Boundaries {
                workspace: Some(crate::workflow::soma::profile::Boundary::Write),
                filesystem: None,
                git: None,
                process: None,
                network: None,
                secret: None,
            }),
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
            // The bridge exchanges one request/response per invocation;
            // nothing is streamed incrementally.
            event_streaming: Some(false),
            read_projection: Some(false),
            locality: Some(Locality::Local),
            privacy_classes: Vec::new(),
            max_structured_input: None,
            max_structured_output: None,
            budget_ceilings: None,
            evidence_attachment: Some(true),
            validation: Some(true),
            approval_escalation: Some(false),
        },
        content_digest: None,
    }
    .sealed()
}

/// The governed protocol-bridged adapter.
///
/// The adapter itself never spawns a process: `run` invokes the declared
/// capability through the governed dispatch. The protocol handler
/// ([`protocol_bridge_capability`]) is registered by the caller in the
/// [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
/// reviewed governance permit that authorizes it.
pub struct ProtocolBridgeHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for ProtocolBridgeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolBridgeHarness {
    pub fn new() -> Self {
        Self {
            declaration: protocol_declaration(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for ProtocolBridgeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The ONLY effect path: `call` refuses capabilities outside this
        // harness's declaration before any worker process can exist. The
        // governed request's known secrets and the correlation identities
        // ride along so the handler can build and verify the protocol
        // envelope and redact authoritatively.
        let mut args = step.args.clone();
        if let Some(map) = args.as_object_mut() {
            map.insert(
                "knownSecrets".to_string(),
                serde_json::json!(dispatch.known_secrets()),
            );
            // Deterministic correlation binding: `step.step_id` is the
            // idempotency key and the evidence correlation; `requestId`
            // is the per-invocation protocol identity bound to it.
            map.insert("correlationId".to_string(), serde_json::json!(step.step_id));
            map.insert("stepId".to_string(), serde_json::json!(step.step_id));
            map.insert(
                "requestId".to_string(),
                serde_json::json!(format!("req-{}", step.step_id)),
            );
        }
        let run_outcome = dispatch.call(&step.capability, args, &step.step_id)?;
        let output = run_outcome.output.clone();

        // The handler result is machine-readable JSON; a document that does
        // not attest `ok` is not evidence of a successful bridge exchange.
        let parsed: serde_json::Value = serde_json::from_str(&output)
            .context("protocol bridge result must be a JSON result document")?;
        if parsed.get("success").and_then(serde_json::Value::as_bool) != Some(true)
            || parsed.get("status").and_then(serde_json::Value::as_str) != Some("ok")
        {
            anyhow::bail!("protocol bridge exchange did not succeed: {output}");
        }

        let events = vec![
            HarnessEvent {
                sequence: 0,
                correlation_id: step.step_id.clone(),
                kind: "started".to_string(),
                detail_digest: sha256_hex(format!("{}:started", step.step_id).as_bytes()),
            },
            HarnessEvent {
                sequence: 1,
                correlation_id: step.step_id.clone(),
                kind: "completed".to_string(),
                detail_digest: sha256_hex(format!("{}:completed", step.step_id).as_bytes()),
            },
        ];
        Ok(HarnessOutcome {
            harness_id: PROTOCOL_HARNESS_ID.to_string(),
            output,
            evidence_entry_digest: run_outcome.evidence_entry.entry_digest.clone(),
            events,
            content_digest: None,
        }
        .sealed())
    }

    async fn checkpoint(
        &self,
        _outcome: &HarnessOutcome,
        portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        Ok(HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: PROTOCOL_HARNESS_ID.to_string(),
            harness_revision: PROTOCOL_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: None,
            content_digest: None,
        }
        .sealed())
    }

    async fn resume(
        &self,
        checkpoint: &HarnessCheckpointV1,
        portable_state_digest: &str,
    ) -> Result<HarnessContinuation> {
        checkpoint.verify()?;
        if checkpoint.harness_id != PROTOCOL_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {PROTOCOL_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != PROTOCOL_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({PROTOCOL_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: PROTOCOL_HARNESS_ID.to_string(),
            harness_revision: PROTOCOL_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.protocol.bridge` capability for a runner's
/// registry, bound to an acquired workspace root.
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently bounded and validated. `workspace_root` must be the
/// GovernedWorkspace root for the work — the worker cwd is always this root
/// or a validated sub-directory; there is no fallback.
pub fn protocol_bridge_capability(
    config: ProtocolBridgeConfig,
    workspace_root: PathBuf,
) -> Capability {
    Capability::deterministic(&["exe", "argv", "input"], move |args| {
        run_protocol_bridge_guarded(&config, &workspace_root, args)
    })
}

// ---------------------------------------------------------------------------
// Protocol envelope (versioned, strict, single request/response)
// ---------------------------------------------------------------------------

/// Typed worker failure inside a protocol `error` response.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeFailure {
    #[serde(rename = "class")]
    pub failure_class: String,
    pub message: String,
}

/// Versioned protocol request sent to the worker on stdin.
///
/// Serialization is deterministic (struct field order) and `requestDigest`
/// is the existing canonical digest over the envelope minus itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeRequest {
    pub schema: String,
    pub protocol_version: String,
    pub request_id: String,
    pub correlation_id: String,
    pub step_id: String,
    pub harness_id: String,
    pub harness_revision: String,
    pub capability: String,
    pub input: serde_json::Value,
    pub request_digest: String,
}

impl BridgeRequest {
    /// Canonical digest over the envelope minus `requestDigest`.
    fn digest_without_digest(&self) -> std::result::Result<String, String> {
        let mut value =
            serde_json::to_value(self).map_err(|e| format!("request serializes: {e}"))?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("requestDigest");
        }
        crate::workflow::soma::try_canonical_digest(&value)
            .map_err(|e| format!("canonical digest failed: {e}"))
    }

    /// Seal a request by computing its `requestDigest`.
    pub fn sealed(mut self) -> std::result::Result<Self, String> {
        self.request_digest = String::new();
        let digest = self.digest_without_digest()?;
        self.request_digest = digest;
        Ok(self)
    }

    /// Fail-closed verification of a parsed request's digest and version.
    fn verify_digest(&self) -> Result<()> {
        if self.schema != PROTOCOL_SCHEMA {
            anyhow::bail!(
                "unsupported protocol schema {:?} (supported {PROTOCOL_SCHEMA:?})",
                self.schema
            );
        }
        if self.protocol_version != PROTOCOL_VERSION {
            anyhow::bail!(
                "unsupported protocol version {:?} (supported {PROTOCOL_VERSION:?})",
                self.protocol_version
            );
        }
        let expected = self
            .digest_without_digest()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if self.request_digest != expected {
            anyhow::bail!("protocol request digest does not verify");
        }
        Ok(())
    }
}

/// Versioned protocol response read from the worker's stdout.
///
/// `deny_unknown_fields` rejects smuggled envelope fields; `toolRequest`
/// is accepted structurally only so the handler can reject it with an
/// explicit governed-dispatch error instead of a parse error.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeResponse {
    pub schema: String,
    pub protocol_version: String,
    pub request_id: String,
    pub correlation_id: String,
    pub step_id: String,
    pub harness_id: String,
    pub harness_revision: String,
    pub capability: String,
    pub status: String,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub failure: Option<BridgeFailure>,
    #[serde(default)]
    pub tool_request: Option<serde_json::Value>,
    #[serde(default)]
    pub request_digest: String,
}

// ---------------------------------------------------------------------------
// Guarded handler (the capability body)
// ---------------------------------------------------------------------------

/// Recursively redact every string leaf of a JSON value with `redactor`.
/// Mirrors the native harness's structure-aware redaction: quote/backslash
/// sequences inside secrets cannot damage JSON grammar.
fn redact_json_value(value: &mut serde_json::Value, redactor: &Redactor) {
    match value {
        serde_json::Value::String(s) => {
            *s = redactor.redact(s);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_value(item, redactor);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                redact_json_value(v, redactor);
            }
        }
        _ => {}
    }
}

fn parse_secret_array(obj: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<String>> {
    let Some(v) = obj.get("knownSecrets") else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or_else(|| {
        anyhow::anyhow!("protocol bridge argument \"knownSecrets\" must be an array of strings")
    })?;
    if arr.len() > 256 {
        anyhow::bail!("protocol bridge argument \"knownSecrets\" exceeds the entry limit");
    }
    let mut out = Vec::with_capacity(arr.len());
    let mut total = 0usize;
    for item in arr {
        let s = item.as_str().ok_or_else(|| {
            anyhow::anyhow!("protocol bridge argument \"knownSecrets\" must be an array of strings")
        })?;
        total += s.len();
        if total > 128 * 1024 {
            anyhow::bail!("protocol bridge argument \"knownSecrets\" exceeds the size limit");
        }
        out.push(s.to_string());
    }
    Ok(out)
}

fn parse_id(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<String> {
    let s = obj
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("protocol bridge argument \"{key}\" must be a non-empty string")
        })?;
    if s.len() > 512 {
        anyhow::bail!("protocol bridge argument \"{key}\" exceeds the length limit");
    }
    Ok(s.to_string())
}

/// Single quiesce-and-join cleanup for EVERY terminal path.
///
/// Invariant (descendant-pipe fix): owned-tree teardown runs BEFORE the
/// potentially blocking writer join, so a descendant that inherited the
/// stdin read end can never keep the writer blocked after the direct child
/// exits. Callers pass `force_terminate` for live-child kills
/// (cancel/timeout/budget) and `sweep_on_normal` semantics for the
/// direct-child-exited path; both close every stdin read end inside the
/// owned tree (Unix process group SIGKILL / Windows job close) before the
/// join. Drains finish last (EOF after tree death). The writer result, when
/// present, is recorded in `writer_result`; drain states are returned for
/// the success path and discarded on refusal paths.
fn quiesce_and_join(
    child: &mut SpawnedChild,
    force_terminate: bool,
    writer_handle: &mut Option<std::thread::JoinHandle<Result<()>>>,
    writer_result: &mut Option<Result<()>>,
    stdout_drain: DrainHandle,
    stderr_drain: DrainHandle,
) -> (DrainState, DrainState) {
    // 1. Owned-tree teardown FIRST: terminates/sweeps every descendant and
    //    closes every stdin read end, unblocking a blocked writer.
    if force_terminate {
        child.terminate_and_reap();
    } else {
        child.sweep_on_normal();
    }
    // 2. Writer join SECOND: now bounded by tree death, never indefinite.
    if let Some(handle) = writer_handle.take() {
        *writer_result = Some(match handle.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("stdin writer thread panicked")),
        });
    }
    // 3. Drain finish LAST: EOF is guaranteed-measurable once every writer
    //    died with the contained tree.
    let stdout_state = stdout_drain.finish();
    let stderr_state = stderr_drain.finish();
    (stdout_state, stderr_state)
}

/// One governed protocol-bridge invocation behind gates 1-4; gates 6-9
/// seal its JSON result document.
fn run_protocol_bridge_guarded(
    config: &ProtocolBridgeConfig,
    workspace_root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    // ---- Argument validation (fail closed, no coercion). ----
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("protocol bridge arguments must be an object"))?;
    let total_args_bytes: usize = obj.values().map(|v| v.to_string().len()).sum();
    if total_args_bytes > config.max_args_bytes {
        anyhow::bail!(
            "protocol bridge arguments exceed the configured limit of {} bytes",
            config.max_args_bytes
        );
    }
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "exe"
                | "argv"
                | "input"
                | "cwd"
                | "timeoutMs"
                | "knownSecrets"
                | "requestId"
                | "correlationId"
                | "stepId"
        ) {
            anyhow::bail!("protocol bridge argument {key:?} is not supported");
        }
    }
    let exe = obj
        .get("exe")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("protocol bridge argument \"exe\" must be a non-empty string")
        })?;
    let argv: Vec<String> = obj
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| {
            anyhow::anyhow!("protocol bridge argument \"argv\" must be an array of strings")
        })?;
    if argv.len() != obj["argv"].as_array().map(Vec::len).unwrap_or(0) {
        anyhow::bail!("protocol bridge argument \"argv\" must be an array of strings");
    }
    let Some(input) = obj.get("input").cloned() else {
        anyhow::bail!("protocol bridge argument \"input\" is required");
    };
    let request_id = parse_id(obj, "requestId")?;
    let correlation_id = parse_id(obj, "correlationId")?;
    let step_id = parse_id(obj, "stepId")?;
    let cwd = obj
        .get("cwd")
        .map(|v| {
            v.as_str()
                .ok_or_else(|| anyhow::anyhow!("protocol bridge argument \"cwd\" must be a string"))
        })
        .transpose()?;
    let request_known_secrets = parse_secret_array(obj)?;
    let timeout_ms = obj
        .get("timeoutMs")
        .map(|v| {
            v.as_u64()
                .filter(|ms| *ms >= 1 && *ms <= config.max_timeout_ms)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "protocol bridge argument \"timeoutMs\" must be a positive number of milliseconds up to {}",
                        config.max_timeout_ms
                    )
                })
        })
        .transpose()?
        .unwrap_or(config.default_timeout_ms);

    // ---- Executable allowlist (exact match, before any spawn). ----
    if !config.allowed_executables.iter().any(|a| a == exe) {
        anyhow::bail!("protocol bridge executable {exe:?} is not allowlisted");
    }

    // ---- Workspace binding (no fallback). ----
    let canonical_root = governed_root(workspace_root)?;
    let cwd = child_cwd(&canonical_root, cwd)?;

    // ---- Build and seal the protocol request (before any spawn). ----
    let request = BridgeRequest {
        schema: PROTOCOL_SCHEMA.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        request_id: request_id.clone(),
        correlation_id: correlation_id.clone(),
        step_id: step_id.clone(),
        harness_id: PROTOCOL_HARNESS_ID.to_string(),
        harness_revision: PROTOCOL_REVISION.to_string(),
        capability: PROTOCOL_BRIDGE_CAPABILITY.to_string(),
        input,
        request_digest: String::new(),
    }
    .sealed()
    .map_err(|e| anyhow::anyhow!("protocol request seal failed: {e}"))?;
    request.verify_digest()?;
    let request_text = serde_json::to_string(&request).context("protocol request serializes")?;
    if request_text.len() > config.max_request_bytes {
        anyhow::bail!(
            "protocol request exceeds the configured limit of {} bytes",
            config.max_request_bytes
        );
    }

    // ---- Deadline covers the WHOLE lifecycle (spawn, write, wait), so an
    // uncooperative worker cannot consume unbounded time before supervision
    // starts. The stdin write below is supervised by this deadline. ----
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);

    // ---- Pipes + contained spawn with protocol stdin. Reuses the slice-E
    // containment primitive; only the stdin/stdout exchange is new. ----
    let (out_r, out_w) = make_pipe()?;
    let (err_r, err_w) = make_pipe()?;
    let (mut child, stdin_writer) = spawn_contained(
        exe, &argv, &cwd, out_w, err_w, /*want_stdin_pipe=*/ true,
    )
    .context("protocol worker spawn failed")?;
    let mut stdin_writer = stdin_writer
        .ok_or_else(|| anyhow::anyhow!("protocol worker spawn returned no stdin channel"))?;

    let stdout_drain = spawn_drain(out_r, config.max_output_bytes);
    let stderr_drain = spawn_drain(err_r, config.max_output_bytes);

    // ---- Single supervised request write (EOF framing), on ONE managed
    // writer thread, joined on EVERY exit path. A blocking write is
    // unavoidable for bounded pipes, so supervision is done by containing
    // it: the deadline/cancel/budget loop below fires while the writer is
    // blocked, terminates the owned tree (closing every stdin read end, so
    // the blocked write returns with EPIPE), and joins the writer. No
    // detached thread, no leaked handle, no descendant survives. ----
    let mut wire = request_text.into_bytes();
    wire.push(b'\n');
    let writer_handle = std::thread::spawn(move || {
        let result = stdin_writer.write_all(&wire);
        stdin_writer.close();
        result
    });
    let mut writer_handle = Some(writer_handle);
    let mut writer_result: Option<Result<()>> = None;

    // ---- Deadline / cancellation / budget loop (started before spawn). ----
    let status = loop {
        if let Some(status) = child.try_wait() {
            break status;
        }
        if config.cancel.load(Ordering::Relaxed) {
            quiesce_and_join(
                &mut child,
                true,
                &mut writer_handle,
                &mut writer_result,
                stdout_drain,
                stderr_drain,
            );
            anyhow::bail!("protocol bridge cancelled");
        }
        if Instant::now() >= deadline {
            quiesce_and_join(
                &mut child,
                true,
                &mut writer_handle,
                &mut writer_result,
                stdout_drain,
                stderr_drain,
            );
            anyhow::bail!("protocol bridge timed out after {timeout_ms}ms");
        }
        if stdout_drain.total() + stderr_drain.total() > config.max_output_bytes as u64 {
            quiesce_and_join(
                &mut child,
                true,
                &mut writer_handle,
                &mut writer_result,
                stdout_drain,
                stderr_drain,
            );
            anyhow::bail!(
                "protocol bridge output exceeded the configured budget of {} bytes",
                config.max_output_bytes
            );
        }
        // Writer finished early (broken pipe: worker closed/exited the
        // stdin side while still running). Fail promptly with the tree
        // reaped; a successful writer simply means the request is sent
        // and the child will respond (or hang until the deadline).
        if writer_handle
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(false)
        {
            let handle = writer_handle.take().expect("writer handle present");
            writer_result = Some(match handle.join() {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("stdin writer thread panicked")),
            });
            if let Some(Err(_)) = writer_result {
                quiesce_and_join(
                    &mut child,
                    true,
                    &mut writer_handle,
                    &mut writer_result,
                    stdout_drain,
                    stderr_drain,
                );
                anyhow::bail!("protocol request write to worker stdin failed");
            }
        }
        std::thread::sleep(POLL);
    };

    // The direct worker exited. Sweep/kill every owned descendant FIRST
    // (closing every stdin read end, including an inherited one a
    // descendant may still hold), THEN join the writer. A broken pipe
    // here means the worker exited before consuming the request — a
    // typed failure with the child already reaped above. The drain
    // states returned by the helper are the capture completion
    // evidence for the success path.
    let (stdout_state, stderr_state) = quiesce_and_join(
        &mut child,
        false,
        &mut writer_handle,
        &mut writer_result,
        stdout_drain,
        stderr_drain,
    );
    if let Some(Err(write_error)) = writer_result {
        return Err(anyhow::anyhow!(
            "protocol request write to worker stdin failed: {write_error}"
        ));
    }

    // The direct worker exited: verify capture completion before
    // sealing evidence.
    if !stdout_state.done || !stderr_state.done {
        anyhow::bail!("protocol bridge output capture did not complete");
    }
    if stdout_state.total + stderr_state.total > config.max_output_bytes as u64 {
        anyhow::bail!(
            "protocol bridge output exceeded the configured budget of {} bytes",
            config.max_output_bytes
        );
    }
    if status != 0 {
        anyhow::bail!("protocol worker exited with code {status}");
    }

    // ---- Parse and verify the single protocol response. ----
    let stdout_text = String::from_utf8_lossy(&stdout_state.retained);
    if stdout_state.total > config.max_response_bytes as u64 {
        anyhow::bail!(
            "protocol response exceeds the configured limit of {} bytes",
            config.max_response_bytes
        );
    }
    let trimmed = stdout_text.trim();
    if trimmed.is_empty() {
        anyhow::bail!("protocol worker returned an empty response");
    }
    // `from_str` rejects trailing data, so a duplicated/second response
    // document refuses as malformed — never silently picks one.
    let response: BridgeResponse = serde_json::from_str(trimmed)
        .context("protocol response is not a valid response envelope")?;

    if response.schema != PROTOCOL_SCHEMA {
        anyhow::bail!(
            "unsupported protocol schema {:?} (supported {PROTOCOL_SCHEMA:?})",
            response.schema
        );
    }
    if response.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!(
            "unsupported protocol version {:?} (supported {PROTOCOL_VERSION:?})",
            response.protocol_version
        );
    }
    if response.request_id != request.request_id {
        anyhow::bail!("protocol response requestId does not match the request");
    }
    if response.correlation_id != request.correlation_id {
        anyhow::bail!("protocol response correlationId does not match the request");
    }
    if response.step_id != request.step_id {
        anyhow::bail!("protocol response stepId does not match the request");
    }
    if response.harness_id != PROTOCOL_HARNESS_ID {
        anyhow::bail!(
            "protocol response harnessId {:?} does not match {PROTOCOL_HARNESS_ID:?}",
            response.harness_id
        );
    }
    if response.harness_revision != PROTOCOL_REVISION {
        anyhow::bail!(
            "protocol response harnessRevision {:?} does not match {PROTOCOL_REVISION:?}",
            response.harness_revision
        );
    }
    if response.capability != PROTOCOL_BRIDGE_CAPABILITY {
        anyhow::bail!(
            "protocol response capability {:?} is not a declared bridge capability",
            response.capability
        );
    }
    if response.request_digest != request.request_digest {
        anyhow::bail!("protocol response requestDigest does not match the request");
    }
    // A tool request is never routed through a governed dispatch by this
    // bridge. It refuses outright — the request cannot bypass NodeRunner.
    if response.tool_request.is_some() {
        anyhow::bail!(
            "protocol worker tool request refused: tool execution requires governed dispatch and is never performed by the bridge"
        );
    }

    // Normalized redaction BEFORE sealing: verify structure, then redact
    // the verified output's string leaves so secrets never persist.
    let mut secrets = config.known_secrets.clone();
    for secret in &request_known_secrets {
        if !secrets.contains(secret) {
            secrets.push(secret.clone());
        }
    }
    let redactor = Redactor::with_known_secrets(Redactor::new(), &secrets);
    let mut redacted_output = match response.status.as_str() {
        "ok" => response
            .output
            .clone()
            .ok_or_else(|| anyhow::anyhow!("protocol ok response is missing output"))?,
        "error" => {
            let failure = response
                .failure
                .clone()
                .ok_or_else(|| anyhow::anyhow!("protocol error response is missing failure"))?;
            if failure.failure_class.trim().is_empty() || failure.message.trim().is_empty() {
                anyhow::bail!("protocol error response has an empty failure classification");
            }
            anyhow::bail!(
                "protocol worker error ({}): {}",
                failure.failure_class,
                failure.message
            );
        }
        other => anyhow::bail!("protocol response status {other:?} is not supported"),
    };
    redact_json_value(&mut redacted_output, &redactor);

    let result = serde_json::json!({
        "success": true,
        "status": "ok",
        "output": redacted_output,
    });
    let text = serde_json::to_string(&result).context("protocol bridge result serializes")?;
    if text.len() > config.max_output_bytes {
        anyhow::bail!(
            "protocol bridge result exceeds the configured budget of {} bytes",
            config.max_output_bytes
        );
    }
    // Defense in depth: verify — never rewrite — that no known secret or
    // credential-shaped residue survives in the persisted document.
    if redactor.redact(&text) != text {
        anyhow::bail!(
            "protocol bridge result still contains a known secret after redaction; refusing to persist"
        );
    }
    Ok(text)
}
