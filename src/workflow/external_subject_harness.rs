//! Governed external coding-agent compatibility subject (issue #154,
//! Slice J).
//!
//! Delivers the **protocol-bridged** family an external-subject adapter
//! speaking the published **JSON-RPC 2.0** envelope
//! (<https://www.jsonrpc.org/specification>) across a real, separately
//! spawned process boundary. The adapter verifies the subject's identity
//! (`serverInfo.name`), protocol version and advertised capabilities
//! **before** issuing any completion request, refuses the exchange on any
//! mismatch, and tears down the owned process tree on every exit path.
//!
//! What this is:
//!   * a governed adapter through [`HarnessDispatch::call`] →
//!     [`crate::workflow::node_runner::NodeRunner`] → governed capability;
//!   * a JSON-RPC 2.0 envelope with a deliberately small, documented method
//!     set (`initialize`, `completion/complete`) defined by the deterministic
//!     fixture [`crate::bin::external_subject_worker`];
//!   * proof that the governed boundary drives a *separately developed*
//!     process through a standard envelope with validation, bounds,
//!     cancellation and checkpoint semantics intact.
//!
//! What this is NOT:
//!   * a claim of compatibility with Claude Code, Codex, Goose, OpenCode,
//!     Aider, ACP, or any other third party. No live third-party agent was
//!     installed or invoked here (no CLI is available on this host and paid
//!     inference is out of scope). The fixture proves the transport and
//!     governance path only; live external-CLI smoke is NOT RUN.
//!   * a new provider framework or a second runtime.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::workflow::artifact_integrity::sha256_hex;
use crate::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessCheckpointV1, HarnessContinuation,
    HarnessDeclarationV1, HarnessDispatch, HarnessEvent, HarnessFamily, HarnessOutcome,
    HarnessStep,
};
use crate::workflow::node_runner::Capability;
use crate::workflow::redaction::Redactor;
use crate::workflow::soma::capability::{
    Identity, Locality, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
};
use crate::workflow::subprocess_harness::{
    BridgeStdinWriter, SpawnedChild, child_cwd, governed_root, make_pipe, spawn_contained,
    spawn_drain,
};

pub const EXTERNAL_SUBJECT_HARNESS_ID: &str = "lite.harness.external_subject";
pub const EXTERNAL_SUBJECT_COMPLETION_CAPABILITY: &str = "harness.external_subject.completion";
pub const EXTERNAL_SUBJECT_IDENTITY_NAME: &str = "lite-external-subject";
pub const EXTERNAL_SUBJECT_REVISION: &str = "v1";
pub const EXTERNAL_SUBJECT_SCHEMA: &str = "lite.external.subject";
pub const EXTERNAL_SUBJECT_PROTOCOL_VERSION: &str = "2026.1";
/// The exact subject identity the adapter accepts. Anything else refuses
/// before any completion is requested.
pub const EXPECTED_SERVER_NAME: &str = "external-subject-fixture";
pub const EXPECTED_SUBJECT_MODEL: &str = "external-subject-model";
pub const EXTERNAL_SUBJECT_CAPS_ID: &str = "lite.harness.external_subject.caps";

/// Hard bound on a single JSON-RPC message read from the subject.
const MAX_RESPONSE_LINE_BYTES: usize = 1024 * 1024;
/// Hard bound on retained stderr bytes.
const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Aggregate request bound for the two outbound messages.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Cap on how many lines the subject may send per connection (we expect
/// exactly two: one response per request).
const MAX_MESSAGES: usize = 2;
/// Poll quantum for deadline/cancellation while waiting on the subject.
const MESSAGE_POLL: Duration = Duration::from_millis(10);

/// Configuration for the governed external-subject handler.
///
/// All bounds are enforced on every invocation; there is no unbounded mode.
/// Credentials come from the operator-owned configuration, never from task
/// arguments. The executable is exact-match allowlisted — no discovery.
#[derive(Debug, Clone)]
pub struct ExternalSubjectConfig {
    /// Exact executables permitted to be spawned ("exact match, before any
    /// spawn"). The deterministic fixture lives at the path recorded in
    /// `CARGO_BIN_EXE_external_subject_worker`.
    pub allowed_executables: Vec<String>,
    /// Model the subject is expected to attest in its completion response.
    pub expected_model: String,
    /// Default invocation deadline when the step omits `timeoutMs`.
    pub default_timeout_ms: u64,
    /// Hard ceiling on any `timeoutMs`, including the operator default.
    pub max_timeout_ms: u64,
    /// Hard bound on bytes accepted from the subject per message.
    pub max_output_bytes: usize,
    /// Known secret values redacted from the verified result.
    pub known_secrets: Vec<String>,
    /// Cooperative cancellation source; setting it aborts the invocation.
    pub cancel: Arc<AtomicBool>,
}

impl Default for ExternalSubjectConfig {
    fn default() -> Self {
        Self {
            allowed_executables: Vec::new(),
            expected_model: EXPECTED_SUBJECT_MODEL.to_string(),
            default_timeout_ms: 30_000,
            max_timeout_ms: 3_600_000,
            max_output_bytes: MAX_RESPONSE_LINE_BYTES,
            known_secrets: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ExternalSubjectConfig {
    pub fn allowed_executables(mut self, executables: Vec<String>) -> Self {
        self.allowed_executables = executables;
        self
    }

    pub fn expected_model(mut self, model: impl Into<String>) -> Self {
        self.expected_model = model.into();
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

    pub fn known_secrets(mut self, known_secrets: Vec<String>) -> Self {
        self.known_secrets = known_secrets;
        self
    }

    pub fn cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Typed failure classification for one external-subject invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSubjectFailure {
    pub status: String,
    pub message: String,
}

impl ExternalSubjectFailure {
    fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ExternalSubjectFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for ExternalSubjectFailure {}

fn governed_failure(status: &str, error: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "success": false,
        "status": status,
        "output": { "status": status, "error": error.into() },
    }))
    .context("external subject failure serializes")
    .expect("governed failure document serializes")
}

fn governed_success(
    output: Value,
    latency_ms: u64,
    request_id: &str,
    correlation_id: &str,
) -> String {
    serde_json::to_string(&json!({
        "success": true,
        "status": "ok",
        "output": {
            "harnessId": EXTERNAL_SUBJECT_HARNESS_ID,
            "capability": EXTERNAL_SUBJECT_COMPLETION_CAPABILITY,
            "content": output,
            "latencyMs": latency_ms,
            "requestId": request_id,
            "correlationId": correlation_id,
        },
    }))
    .context("external subject success serializes")
    .expect("governed success document serializes")
}

/// Versioned external-subject declaration owned by this implementation.
pub fn external_subject_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: EXTERNAL_SUBJECT_HARNESS_ID.to_string(),
        identity: Identity {
            name: EXTERNAL_SUBJECT_IDENTITY_NAME.to_string(),
            revision: EXTERNAL_SUBJECT_REVISION.to_string(),
        },
        family: HarnessFamily::ProtocolBridged,
        declared_capabilities: vec![EXTERNAL_SUBJECT_COMPLETION_CAPABILITY.to_string()],
        claimed_capabilities: vec![EXTERNAL_SUBJECT_COMPLETION_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: EXTERNAL_SUBJECT_CAPS_ID.to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: EXTERNAL_SUBJECT_IDENTITY_NAME.to_string(),
                revision: EXTERNAL_SUBJECT_REVISION.to_string(),
            },
            declared_at: "2026-11-12T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: Vec::new(),
            effect_classes: vec!["process-exec".to_string()],
            workspace: Some(crate::workflow::soma::profile::Boundaries {
                workspace: Some(crate::workflow::soma::profile::Boundary::Read),
                filesystem: None,
                git: None,
                process: None,
                network: None,
                secret: None,
            }),
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
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

/// The replaceable governed external-subject adapter.
pub struct ExternalSubjectHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for ExternalSubjectHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalSubjectHarness {
    pub fn new() -> Self {
        Self {
            declaration: external_subject_declaration(),
        }
    }
}

#[async_trait]
impl ExecutionHarness for ExternalSubjectHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        if step.step_id.chars().count() > 512 {
            anyhow::bail!(
                "external subject step identifier exceeds the 512 character bound; refusing before dispatch"
            );
        }
        let mut args = step.args.clone();
        if let Some(map) = args.as_object_mut() {
            map.insert(
                "knownSecrets".to_string(),
                serde_json::json!(dispatch.known_secrets()),
            );
            map.insert("correlationId".to_string(), serde_json::json!(step.step_id));
            map.insert("stepId".to_string(), serde_json::json!(step.step_id));
            map.insert(
                "requestId".to_string(),
                serde_json::json!(format!("req-{}", step.step_id)),
            );
        }
        dispatch.ensure_declared(&step.capability)?;
        let run_outcome = dispatch.call(&step.capability, args, &step.step_id)?;
        let output = run_outcome.output.clone();

        let parsed: Value = serde_json::from_str(&output)
            .context("external subject result must be a JSON result")?;
        if parsed.get("success").and_then(Value::as_bool) != Some(true)
            || parsed.get("status").and_then(Value::as_str) != Some("ok")
        {
            let status = parsed
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("external subject invocation did not succeed (status {status})");
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
            harness_id: EXTERNAL_SUBJECT_HARNESS_ID.to_string(),
            output,
            evidence_entry_digest: run_outcome.evidence_entry.entry_digest.clone(),
            events,
            content_digest: None,
        }
        .sealed())
    }

    async fn checkpoint(
        &self,
        outcome: &HarnessOutcome,
        portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        outcome.verify()?;
        Ok(HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: EXTERNAL_SUBJECT_HARNESS_ID.to_string(),
            harness_revision: EXTERNAL_SUBJECT_REVISION.to_string(),
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
        if checkpoint.harness_id != EXTERNAL_SUBJECT_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {EXTERNAL_SUBJECT_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != EXTERNAL_SUBJECT_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({EXTERNAL_SUBJECT_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: EXTERNAL_SUBJECT_HARNESS_ID.to_string(),
            harness_revision: EXTERNAL_SUBJECT_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.external_subject.completion` capability for a
/// runner's registry, bound to the operator-owned configuration.
pub fn external_subject_capability(
    config: ExternalSubjectConfig,
    workspace_root: &Path,
) -> Capability {
    let workspace_root = workspace_root.to_path_buf();
    Capability::deterministic(&["exe", "prompt"], move |args| {
        let config = config.clone();
        run_external_subject_guarded(&config, &workspace_root, args)
    })
}

// ---------------------------------------------------------------------------
// Bounded line reader: newline-delimited JSON-RPC messages from the subject
// ---------------------------------------------------------------------------

enum MessageEvent {
    Line(Vec<u8>),
    Eof,
    ReadFailed,
}

fn line_reader<R: std::io::Read>(
    mut reader_file: R,
    line_cap: usize,
    tx: std::sync::mpsc::SyncSender<MessageEvent>,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut byte = [0u8; 1];
    let mut lines_seen: usize = 0;
    // Hard message-count ceiling: any subject that emits more lines than the
    // protocol requires is refused; we never queue unboundedly and never
    // block on a full channel.
    loop {
        match std::io::Read::read(&mut reader_file, &mut byte) {
            Ok(0) => {
                if !buf.is_empty() {
                    if lines_seen < MAX_MESSAGES {
                        let _ = tx.try_send(MessageEvent::Line(std::mem::take(&mut buf)));
                    } else {
                        let _ = tx.try_send(MessageEvent::ReadFailed);
                    }
                }
                let _ = tx.try_send(MessageEvent::Eof);
                return;
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    let line = std::mem::take(&mut buf);
                    lines_seen += 1;
                    if lines_seen > MAX_MESSAGES {
                        // Flooding: stop reading, flag the overflow, and let
                        // the caller decide via the channel contents.
                        let _ = tx.try_send(MessageEvent::ReadFailed);
                        return;
                    }
                    // Non-blocking send. On a full channel the reader stops
                    // reading and exits; the caller treats the message-count
                    // overflow as a typed refusal. No blocked reader thread,
                    // no unbounded queue.
                    if tx.try_send(MessageEvent::Line(line)).is_err() {
                        return;
                    }
                    continue;
                }
                buf.push(byte[0]);
                if buf.len() > line_cap {
                    let _ = tx.try_send(MessageEvent::ReadFailed);
                    return;
                }
            }
            Err(_) => {
                let _ = tx.try_send(MessageEvent::ReadFailed);
                return;
            }
        }
    }
}

fn next_message(
    rx: &std::sync::mpsc::Receiver<MessageEvent>,
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<MessageEvent, ExternalSubjectFailure> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(ExternalSubjectFailure::new(
                "timeout",
                "external subject did not answer before the invocation deadline",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(ExternalSubjectFailure::new(
                "cancelled",
                "external subject inference cancelled",
            ));
        }
        let wait = (deadline - now).min(MESSAGE_POLL);
        match rx.recv_timeout(wait) {
            Ok(msg) => return Ok(msg),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ExternalSubjectFailure::new(
                    "transport-error",
                    "external subject stdout reader terminated abnormally",
                ));
            }
        }
    }
}

fn read_json_message(
    rx: &std::sync::mpsc::Receiver<MessageEvent>,
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<Value, ExternalSubjectFailure> {
    match next_message(rx, deadline, cancel)? {
        MessageEvent::Line(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            ExternalSubjectFailure::new(
                "invalid-response",
                "external subject response was not valid JSON",
            )
        }),
        MessageEvent::Eof => Err(ExternalSubjectFailure::new(
            "truncated-response",
            "external subject closed stdout before responding",
        )),
        MessageEvent::ReadFailed => Err(ExternalSubjectFailure::new(
            "invalid-response",
            "external subject response exceeded the configured bound",
        )),
    }
}

fn teardown_tree(
    child: &mut SpawnedChild,
    reader_thread: &mut Option<std::thread::JoinHandle<()>>,
    stdin_writer: &mut Option<BoundedStdinWriter>,
    stderr_drain: crate::workflow::subprocess_harness::DrainHandle,
    force: bool,
) {
    // 1) Stop feeding the writer: the writer thread then drops the child's
    //    stdin once it sees the closed channel.
    if let Some(writer) = stdin_writer.take() {
        let (tx, join) = (writer.tx, writer.join);
        drop(tx);
        // 2) Tear down the tree first so the child's read end of our stdin
        //    pipe closes and any blocked write returns with EPIPE.
        if force {
            child.terminate_and_reap();
        } else {
            child.sweep_on_normal();
        }
        // 3) Join the writer; after tree teardown it cannot block.
        if let Some(handle) = join {
            let _ = handle.join();
        }
    } else if force {
        child.terminate_and_reap();
    } else {
        child.sweep_on_normal();
    }
    // 4) Join the stdout reader (exits on EOF, read failure, or message
    //    overflow).
    if let Some(handle) = reader_thread.take() {
        let _ = handle.join();
    }
    let _ = stderr_drain.finish();
}

/// Bounded writer for the subject's stdin. The write itself happens on a
/// dedicated thread fed by a bounded channel (never more queued messages
/// than the protocol uses); the caller waits for the writer's ack with the
/// invocation deadline and the cancellation flag both honoured.
struct BoundedStdinWriter {
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
    ack_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    join: Option<std::thread::JoinHandle<Result<(), String>>>,
}

fn spawn_stdin_writer(writer: BridgeStdinWriter) -> BoundedStdinWriter {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(3);
    let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(3);
    let join = std::thread::spawn(move || -> Result<(), String> {
        let mut writer = writer;
        while let Ok(bytes) = rx.recv() {
            match writer.write_all(&bytes) {
                Ok(()) => {
                    let _ = ack_tx.try_send(Ok(()));
                }
                Err(e) => {
                    let err = e.to_string();
                    let _ = ack_tx.try_send(Err(err.clone()));
                    return Err(err);
                }
            }
        }
        // Channel closed: close the child's stdin explicitly. On Windows the
        // raw handle carries no Drop, so dropping alone would leak it.
        writer.close();
        Ok(())
    });
    BoundedStdinWriter {
        tx,
        ack_rx,
        join: Some(join),
    }
}

fn send_line(
    writer: &BoundedStdinWriter,
    payload: &Value,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<(), ExternalSubjectFailure> {
    let mut wire = serde_json::to_vec(payload).map_err(|_| {
        ExternalSubjectFailure::new(
            "invalid-request",
            "external subject request could not serialize",
        )
    })?;
    wire.push(b'\n');
    // Bounded, fail-closed submission: never block the caller on a full or
    // disconnected bounded queue. The queue holds at most three wire
    // messages; two are queued per invocation, so a full queue here means
    // an unexpected internal state and is refused rather than waited on.
    writer.tx.try_send(wire).map_err(|e| match e {
        std::sync::mpsc::TrySendError::Full(_) => ExternalSubjectFailure::new(
            "stdin-queue-full",
            "stdin writer bounded queue is unexpectedly full; refusing to block",
        ),
        std::sync::mpsc::TrySendError::Disconnected(_) => ExternalSubjectFailure::new(
            "transport-error",
            "stdin writer no longer accepts messages",
        ),
    })?;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(ExternalSubjectFailure::new(
                "timeout",
                "external subject did not consume the request within the deadline",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(ExternalSubjectFailure::new(
                "cancelled",
                "external subject invocation cancelled during write",
            ));
        }
        match writer
            .ack_rx
            .recv_timeout((deadline - now).min(Duration::from_millis(10)))
        {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(_)) => {
                return Err(ExternalSubjectFailure::new(
                    "transport-error",
                    "external subject stdin write failed",
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ExternalSubjectFailure::new(
                    "transport-error",
                    "stdin writer terminated abnormally",
                ));
            }
        }
    }
}

fn jsonrpc_error_message(error: &serde_json::Map<String, Value>) -> String {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
    format!("subject JSON-RPC error {code}")
}

fn redact_json_value(value: &mut Value, redactor: &Redactor) {
    match value {
        Value::String(s) => {
            *s = redactor.redact(s);
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value(item, redactor);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                redact_json_value(v, redactor);
            }
        }
        _ => {}
    }
}

/// The two-phase exchange. `deadline` covers the WHOLE lifecycle (spawn,
/// handshake, completion, teardown).
fn run_external_subject_guarded(
    config: &ExternalSubjectConfig,
    workspace_root: &Path,
    args: &Value,
) -> Result<String> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("external subject arguments must be an object"))?;
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "exe"
                | "argv"
                | "prompt"
                | "timeoutMs"
                | "knownSecrets"
                | "requestId"
                | "correlationId"
                | "stepId"
        ) {
            return Ok(governed_failure(
                "unsupported-argument",
                "external subject argument is not supported",
            ));
        }
    }
    let exe = obj
        .get("exe")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("external subject argument \"exe\" must be a non-empty string")
        })?;
    let argv: Vec<String> = match obj.get("argv") {
        None => Vec::new(),
        Some(Value::Array(arr)) => {
            if arr.len() > 16 {
                return Ok(governed_failure(
                    "invalid-argv",
                    "external subject argv exceeds the argument-count bound",
                ));
            }
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::String(s) if s.len() <= 1024 => out.push(s.clone()),
                    _ => {
                        return Ok(governed_failure(
                            "invalid-argv",
                            "external subject argv must be an array of strings with bounded length entries",
                        ));
                    }
                }
            }
            out
        }
        Some(_) => {
            return Ok(governed_failure(
                "invalid-argv",
                "external subject argv must be an array of strings",
            ));
        }
    };
    let prompt = obj
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("external subject argument \"prompt\" must be a non-empty string")
        })?;
    if prompt.len() > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!("external subject prompt exceeded the budget of {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    let args_bytes: usize = obj.values().map(|v| v.to_string().len()).sum();
    if args_bytes > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!("external subject arguments exceeded the budget of {MAX_REQUEST_BYTES} bytes"),
        ));
    }
    if config.default_timeout_ms == 0
        || config.max_timeout_ms == 0
        || config.default_timeout_ms > config.max_timeout_ms
    {
        return Ok(governed_failure(
            "invalid-config",
            "operator timeout configuration is invalid; refusing",
        ));
    }
    let timeout_ms = match obj.get("timeoutMs") {
        None => config.default_timeout_ms,
        Some(value) => match value.as_u64() {
            Some(t) if t >= 1 && t <= config.max_timeout_ms => t,
            _ => {
                return Ok(governed_failure(
                    "invalid-timeout",
                    "timeoutMs must be a positive integer no greater than the configured ceiling",
                ));
            }
        },
    };
    for (key, value) in obj {
        if matches!(key.as_str(), "requestId" | "correlationId" | "stepId") {
            match value {
                Value::String(s) if !s.is_empty() && s.chars().count() <= 512 => {}
                _ => {
                    return Ok(governed_failure(
                        "invalid-identifier",
                        "correlation identifiers must be non-empty strings of at most 512 characters".to_string(),
                    ));
                }
            }
        }
    }
    let known_secrets: Vec<String> = match obj.get("knownSecrets") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            if items.len() > 32 {
                return Ok(governed_failure(
                    "invalid-secrets",
                    "knownSecrets accepts at most 32 known secret values",
                ));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) if !s.is_empty() && s.len() <= 4096 => out.push(s.clone()),
                    _ => {
                        return Ok(governed_failure(
                            "invalid-secrets",
                            "knownSecrets entries must be non-empty strings of at most 4096 bytes",
                        ));
                    }
                }
            }
            out
        }
        Some(_) => {
            return Ok(governed_failure(
                "invalid-secrets",
                "knownSecrets must be an array of strings",
            ));
        }
    };
    let request_id = obj
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("req-step")
        .to_string();
    let correlation_id = obj
        .get("correlationId")
        .and_then(Value::as_str)
        .unwrap_or("step")
        .to_string();

    // ---- Executable allowlist (exact match, before any spawn). ----
    if !config.allowed_executables.iter().any(|a| a == &exe) {
        return Ok(governed_failure(
            "allowlist-refused",
            "external subject executable is not allowlisted".to_string(),
        ));
    }
    // ---- Workspace binding (no fallback). ----
    let canonical_root = governed_root(workspace_root)?;
    let cwd = child_cwd(&canonical_root, None::<&str>)?;

    let mut redactor = Redactor::new().with_known_secrets(&known_secrets);
    for secret in &config.known_secrets {
        redactor = redactor.with_known_secrets(std::slice::from_ref(secret));
    }

    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);

    // ---- Spawn the subject inside the governed containment. ----
    let (out_r, out_w) = make_pipe()?;
    let (err_r, err_w) = make_pipe()?;
    let (mut child, stdin_writer) = spawn_contained(&exe, &argv, &cwd, out_w, err_w, true)
        .context("external subject spawn failed")?;
    let stdin_writer = spawn_stdin_writer(
        stdin_writer
            .ok_or_else(|| anyhow::anyhow!("external subject spawn returned no stdin channel"))?,
    );
    let mut stdin_writer = Some(stdin_writer);
    let err_drain = spawn_drain(err_r, MAX_STDERR_BYTES);

    let line_cap = config.max_output_bytes.min(MAX_RESPONSE_LINE_BYTES);
    let (tx, rx) = std::sync::mpsc::sync_channel(MAX_MESSAGES + 1);
    let mut reader_thread = Some(std::thread::spawn(move || line_reader(out_r, line_cap, tx)));

    // Failure path helper: tear the tree down before classifying. Every
    // post-spawn error path goes through teardown; there is no bypass.
    macro_rules! refuse {
        ($failure:expr) => {{
            let failure: ExternalSubjectFailure = $failure;
            teardown_tree(
                &mut child,
                &mut reader_thread,
                &mut stdin_writer,
                err_drain,
                true,
            );
            return Ok(governed_failure(&failure.status, failure.message));
        }};
    }

    // ---- Phase 1: initialize (identity / version / capabilities) ----
    let init_id = format!("init-{request_id}");
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": init_id,
        "method": "initialize",
        "params": {
            "protocolVersion": EXTERNAL_SUBJECT_PROTOCOL_VERSION,
            "clientInfo": { "name": "prometheos-lite", "version": "1.6.1" },
            "capabilities": { "completions": {} },
        },
    });
    if let Err(failure) = send_line(
        stdin_writer.as_ref().expect("writer present"),
        &init_request,
        deadline,
        &config.cancel,
    ) {
        refuse!(failure);
    }
    let init_response = match read_json_message(&rx, deadline, &config.cancel) {
        Ok(v) => v,
        Err(failure) => refuse!(failure),
    };
    if init_response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        refuse!(ExternalSubjectFailure::new(
            "handshake-refused",
            "external subject handshake response is not a JSON-RPC 2.0 reply",
        ));
    }
    if init_response.get("id") != Some(&Value::String(init_id.clone())) {
        refuse!(ExternalSubjectFailure::new(
            "invalid-response",
            "external subject handshake response id does not match the request",
        ));
    }
    if let Some(error) = init_response.get("error").and_then(Value::as_object) {
        refuse!(ExternalSubjectFailure::new(
            "handshake-refused",
            format!(
                "external subject initialize failed with {}",
                jsonrpc_error_message(error)
            ),
        ));
    }
    let init_result = match init_response.get("result").cloned() {
        Some(r) => r,
        None => {
            refuse!(ExternalSubjectFailure::new(
                "handshake-refused",
                "external subject initialize response is missing result",
            ));
        }
    };
    if init_result.get("protocolVersion").and_then(Value::as_str)
        != Some(EXTERNAL_SUBJECT_PROTOCOL_VERSION)
    {
        refuse!(ExternalSubjectFailure::new(
            "handshake-refused",
            "external subject reports an unsupported protocol version",
        ));
    }
    if init_result
        .get("serverInfo")
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        != Some(EXPECTED_SERVER_NAME)
    {
        refuse!(ExternalSubjectFailure::new(
            "handshake-refused",
            "external subject identity is not the expected compatibility subject",
        ));
    }
    if init_result
        .get("capabilities")
        .and_then(|c| c.get("completions"))
        .is_none()
    {
        refuse!(ExternalSubjectFailure::new(
            "handshake-refused",
            "external subject does not advertise the completions capability",
        ));
    }

    // ---- Phase 2: completion ----
    let comp_id = format!("comp-{request_id}");
    let completion_request = json!({
        "jsonrpc": "2.0",
        "id": comp_id,
        "method": "completion/complete",
        "params": { "prompt": prompt },
    });
    if let Err(failure) = send_line(
        stdin_writer.as_ref().expect("writer present"),
        &completion_request,
        deadline,
        &config.cancel,
    ) {
        refuse!(failure);
    }
    let completion_response = match read_json_message(&rx, deadline, &config.cancel) {
        Ok(v) => v,
        Err(failure) => refuse!(failure),
    };
    if completion_response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        refuse!(ExternalSubjectFailure::new(
            "invalid-response",
            "external subject completion response is not a JSON-RPC 2.0 reply",
        ));
    }
    if completion_response.get("id") != Some(&Value::String(comp_id.clone())) {
        refuse!(ExternalSubjectFailure::new(
            "invalid-response",
            "external subject completion response id does not match the request",
        ));
    }
    if let Some(error) = completion_response.get("error").and_then(Value::as_object) {
        refuse!(ExternalSubjectFailure::new(
            "external-subject-error",
            format!(
                "external subject completion failed with {}",
                jsonrpc_error_message(error)
            ),
        ));
    }
    let comp_result = match completion_response.get("result").cloned() {
        Some(r) => r,
        None => {
            refuse!(ExternalSubjectFailure::new(
                "invalid-response",
                "external subject completion response is missing result",
            ));
        }
    };
    let model = match comp_result.get("model").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => {
            refuse!(ExternalSubjectFailure::new(
                "invalid-response",
                "external subject completion response is missing model",
            ));
        }
    };
    if model != config.expected_model {
        refuse!(ExternalSubjectFailure::new(
            "model-mismatch",
            "external subject responded with a different model than the configured one",
        ));
    }
    let choice_text = match comp_result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
    {
        Some(t) => t.to_string(),
        None => {
            refuse!(ExternalSubjectFailure::new(
                "invalid-response",
                "external subject completion response is missing content",
            ));
        }
    };
    let usage_raw = comp_result.get("usage").cloned().unwrap_or(Value::Null);
    let usage = json!({
        "promptTokens": usage_raw.get("promptTokens").and_then(Value::as_u64),
        "completionTokens": usage_raw.get("completionTokens").and_then(Value::as_u64),
        "totalTokens": usage_raw.get("totalTokens").and_then(Value::as_u64),
    });
    let latency_ms = started.elapsed().as_millis() as u64;

    // ---- Signal EOF and reap the tree. ----
    if let Some(writer) = stdin_writer.take() {
        let (tx, join) = (writer.tx, writer.join);
        drop(tx); // channel closes: the writer drops the child's stdin
        if let Some(handle) = join {
            let _ = handle.join();
        }
    }
    if let Err(failure) = wait_or_reap(&mut child, &mut reader_thread, &config.cancel, deadline) {
        teardown_tree(&mut child, &mut reader_thread, &mut None, err_drain, true);
        return Ok(governed_failure(&failure.status, failure.message));
    }
    let _ = err_drain.finish();

    // ---- Build the governed result document (redacted). ----
    let mut output = json!({
        "harnessId": EXTERNAL_SUBJECT_HARNESS_ID,
        "capability": EXTERNAL_SUBJECT_COMPLETION_CAPABILITY,
        "model": model,
        "content": choice_text,
        "usage": usage,
        "latencyMs": latency_ms,
        "requestId": request_id,
        "correlationId": correlation_id,
    });
    redact_json_value(&mut output, &redactor);
    let text = governed_success(output, latency_ms, &request_id, &correlation_id);
    if redactor.redact(&text) != text {
        return Ok(governed_failure(
            "secret-bearing-response",
            "external subject result still contains a known secret after redaction; refusing to persist",
        ));
    }
    Ok(text)
}

fn wait_or_reap(
    child: &mut SpawnedChild,
    reader_thread: &mut Option<std::thread::JoinHandle<()>>,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> std::result::Result<(), ExternalSubjectFailure> {
    let outcome = wait_for_exit_helper(child, deadline, cancel);
    if outcome.is_ok() {
        child.sweep_on_normal();
        if let Some(handle) = reader_thread.take() {
            let _ = handle.join();
        }
        Ok(())
    } else {
        child.terminate_and_reap();
        if let Some(handle) = reader_thread.take() {
            let _ = handle.join();
        }
        outcome
    }
}

fn wait_for_exit_helper(
    child: &mut SpawnedChild,
    deadline: Instant,
    cancel: &Arc<AtomicBool>,
) -> std::result::Result<(), ExternalSubjectFailure> {
    loop {
        if child.try_wait().is_some() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(ExternalSubjectFailure::new(
                "timeout",
                "external subject did not exit before the invocation deadline",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(ExternalSubjectFailure::new(
                "cancelled",
                "external subject invocation cancelled",
            ));
        }
        std::thread::sleep(MESSAGE_POLL);
    }
}

// ---------------------------------------------------------------------------
// Bounded stdin writer: a pipe without a reader never hangs the caller
// ---------------------------------------------------------------------------

#[cfg(all(test, unix))]
mod bounded_writer_tests {
    use super::*;

    #[test]
    fn blocked_stdin_write_times_out_and_is_joined() {
        use std::os::unix::io::FromRawFd;
        // A pipe whose read end is never read and never closed until the end
        // of the test fills after 64 KiB; a 1 MiB write then blocks
        // deterministically. The bounded writer must report the deadline,
        // never hang.
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let read_fd = fds[0];
        let write_fd = fds[1];
        let out_w = unsafe { std::fs::File::from_raw_fd(write_fd) };

        let writer = spawn_stdin_writer(BridgeStdinWriter::Unix(out_w));
        let cancel = AtomicBool::new(false);
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        let payload = json!({"jsonrpc": "2.0", "id": "blk-1", "method": "initialize", "params": { "pad": "a".repeat(1_000_000) }});
        let err = super::send_line(&writer, &payload, deadline, &cancel)
            .expect_err("a blocked write must hit the deadline");
        assert_eq!(err.status, "timeout");

        // Close the read end: the blocked write returns EPIPE, the writer
        // thread exits and joins cleanly.
        unsafe { libc::close(read_fd) };
        drop(writer.tx);
        let joined = writer
            .join
            .expect("writer join handle")
            .join()
            .expect("writer thread joins");
        assert!(joined.is_err(), "EPIPE must be reported by the writer");
    }

    #[test]
    fn cancellation_reports_cancelled_before_write_completes() {
        use std::os::unix::io::FromRawFd;
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let read_fd = fds[0];
        let write_fd = fds[1];
        let out_w = unsafe { std::fs::File::from_raw_fd(write_fd) };

        let writer = spawn_stdin_writer(BridgeStdinWriter::Unix(out_w));
        let cancel = Arc::new(AtomicBool::new(false));
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let payload = json!({"jsonrpc": "2.0", "id": "blk-2", "method": "initialize", "params": { "pad": "a".repeat(1_000_000) }});
        let err = std::thread::scope(|scope| {
            let cancel_clone = Arc::clone(&cancel);
            let handle =
                scope.spawn(|| super::send_line(&writer, &payload, deadline, &cancel_clone));
            // Let the writer reach the blocked write, then cancel.
            std::thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::SeqCst);
            handle.join().expect("send_line thread joins")
        });
        assert_eq!(
            err.expect_err("cancelled during a blocked write").status,
            "cancelled"
        );

        // Unblock the pipe: the blocked write returns EPIPE and the writer
        // thread exits. Join it explicitly; a passing cancellation test must
        // not quietly detach the writer it verified against.
        unsafe { libc::close(read_fd) };
        let (tx, join) = (writer.tx, writer.join);
        drop(tx);
        let joined = join
            .expect("writer join handle")
            .join()
            .expect("writer thread joins");
        assert!(joined.is_err(), "EPIPE must be reported by the writer");
    }
}

// ---------------------------------------------------------------------------
// Regression coverage for bounded, fail-closed channel submission
// ---------------------------------------------------------------------------

#[cfg(test)]
mod send_line_fail_closed_tests {
    use super::*;

    fn stub_writer(capacity: usize) -> (BoundedStdinWriter, std::sync::mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(capacity);
        let (_ack_tx, ack_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        (
            BoundedStdinWriter {
                tx,
                ack_rx,
                join: None,
            },
            rx,
        )
    }

    #[test]
    fn send_line_refuses_a_full_bounded_queue() {
        let (writer, _rx) = stub_writer(1);
        // Occupy the only slot and keep the receiver alive; never drain it.
        writer
            .tx
            .try_send(b"occupied".to_vec())
            .expect("seed the bounded queue");
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(1);
        let payload = json!({"jsonrpc": "2.0", "id": "full-1", "method": "initialize"});
        let err = super::send_line(&writer, &payload, deadline, &cancel)
            .expect_err("a full bounded queue must be refused fail-closed");
        assert_eq!(err.status, "stdin-queue-full");
    }

    #[test]
    fn send_line_refuses_a_disconnected_writer() {
        let (writer, rx) = stub_writer(1);
        drop(rx); // simulate the writer thread having terminated
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(1);
        let payload = json!({"jsonrpc": "2.0", "id": "disc-1", "method": "initialize"});
        let err = super::send_line(&writer, &payload, deadline, &cancel)
            .expect_err("a disconnected stdin writer must be refused fail-closed");
        assert_eq!(err.status, "transport-error");
    }
}
