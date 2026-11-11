//! Real governed local-model inference harness (issue #154, Slice H).
//!
//! Delivers the **local-model** production harness family
//! ([`HarnessFamily::LocalModel`]) through the single sanctioned effect path:
//! [`HarnessDispatch::call`] → [`crate::workflow::node_runner::NodeRunner`] →
//! governed capability. The adapter speaks an OpenAI-compatible chat
//! completions protocol to a configured local inference endpoint and never
//! invokes the declared capability outside its declaration.
//!
//! Network boundary (Slice H): default to local-only operation — the endpoint
//! must be loopback-bound, no proxy bypass, no redirect escape, bounded response
//! bytes, configured deadline, cooperative cancellation, redaction before
//! persistence, and no secret-bearing response acceptance. Capability
//! declarations never grant authority; unsupported combinations refuse before
//! any effect.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
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
use crate::workflow::soma::profile::{Boundaries, Boundary};

pub const LOCAL_MODEL_HARNESS_ID: &str = "lite.harness.local_model";
pub const LOCAL_MODEL_INFER_CAPABILITY: &str = "harness.local_model.infer";
pub const LOCAL_MODEL_IDENTITY_NAME: &str = "lite-local-model";
pub const LOCAL_MODEL_REVISION: &str = "v1";
pub const LOCAL_MODEL_SCHEMA: &str = "lite.model.infer";
pub const LOCAL_MODEL_VERSION: &str = "1.0.0";
pub const LOCAL_MODEL_CAPS_ID: &str = "lite.harness.local_model.caps";

/// Configuration for the governed local-model handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode.
#[derive(Debug, Clone)]
pub struct LocalModelConfig {
    /// Local inference endpoint (e.g. `http://127.0.0.1:8080/v1`). Must be
    /// loopback-bound; non-loopback endpoints are refused before any network
    /// effect.
    pub endpoint: String,
    /// Model identifier the endpoint must serve (provenance evidence).
    pub model: String,
    /// Default invocation deadline when the step omits `timeoutMs`.
    pub timeout_ms: u64,
    /// Hard budget, in bytes, of the local response body accepted per
    /// invocation.
    pub max_output_bytes: usize,
    /// Known secret values, redacted verbatim from the verified result.
    pub known_secrets: Vec<String>,
    /// Cooperative cancellation source; setting it aborts the inference.
    pub cancel: Arc<AtomicBool>,
}

impl Default for LocalModelConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8080/v1".to_string(),
            model: "local-model-default".to_string(),
            timeout_ms: 30_000,
            max_output_bytes: 256 * 1024,
            known_secrets: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl LocalModelConfig {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            ..Self::default()
        }
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
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

/// Typed failure classification for one governed local-model inference.
///
/// The classification is the typed failure surface the harness outcome
/// contract requires; it never widens authority, it only names the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalModelFailure {
    status: String,
    message: String,
}

impl LocalModelFailure {
    fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LocalModelFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for LocalModelFailure {}

fn governed_failure(status: &str, error: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "success": false,
        "status": status,
        "output": { "status": status, "error": error.into() },
    }))
    .context("local model failure serializes")
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
            "harnessId": LOCAL_MODEL_HARNESS_ID,
            "capability": LOCAL_MODEL_INFER_CAPABILITY,
            "content": output,
            "latencyMs": latency_ms,
            "requestId": request_id,
            "correlationId": correlation_id,
        },
    }))
    .context("local model success serializes")
    .expect("governed success document serializes")
}

/// Versioned local-model declaration owned by this implementation.
pub fn local_model_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
        identity: Identity {
            name: LOCAL_MODEL_IDENTITY_NAME.to_string(),
            revision: LOCAL_MODEL_REVISION.to_string(),
        },
        family: HarnessFamily::LocalModel,
        declared_capabilities: vec![LOCAL_MODEL_INFER_CAPABILITY.to_string()],
        claimed_capabilities: vec![LOCAL_MODEL_INFER_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: LOCAL_MODEL_CAPS_ID.to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: LOCAL_MODEL_IDENTITY_NAME.to_string(),
                revision: LOCAL_MODEL_REVISION.to_string(),
            },
            declared_at: "2026-11-09T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: Vec::new(),
            effect_classes: vec!["network-io".to_string()],
            workspace: Some(Boundaries {
                workspace: Some(Boundary::Read),
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

/// The replaceable governed local-model adapter.
pub struct LocalModelHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for LocalModelHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalModelHarness {
    pub fn new() -> Self {
        Self {
            declaration: local_model_declaration(),
        }
    }
}

#[async_trait]
impl ExecutionHarness for LocalModelHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The ONLY effect path: `call` refuses capabilities outside this
        // harness's declaration before any network effect can occur. The
        // governed request's known secrets and the correlation identities
        // ride along so the handler can build and verify the inference
        // evidence.
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
        // Validate the step capability is within this harness's declaration.
        // `dispatch.call` will enforce this, but we check early for clarity.
        dispatch.ensure_declared(&step.capability)?;
        let run_outcome = dispatch.call(&step.capability, args, &step.step_id)?;
        let output = run_outcome.output.clone();

        // The handler result is a machine-readable JSON result; a document
        // that does not attest `ok` is not evidence of a successful inference.
        let parsed: Value =
            serde_json::from_str(&output).context("local model result must be a JSON result")?;
        if parsed.get("success").and_then(Value::as_bool) != Some(true)
            || parsed.get("status").and_then(Value::as_str) != Some("ok")
        {
            anyhow::bail!("local model inference did not succeed: {output}");
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
            harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
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
            harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
            harness_revision: LOCAL_MODEL_REVISION.to_string(),
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
        if checkpoint.harness_id != LOCAL_MODEL_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {LOCAL_MODEL_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != LOCAL_MODEL_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({LOCAL_MODEL_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
            harness_revision: LOCAL_MODEL_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.local_model.infer` capability for a runner's
/// registry, bound to an acquired workspace root.
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently bounded and validated. `workspace_root` must be the
/// GovernedWorkspace root for the work — the worker cwd is always this root
/// or a validated sub-directory; there is no fallback.
pub fn local_model_capability(config: LocalModelConfig, workspace_root: PathBuf) -> Capability {
    Capability::deterministic(
        &["model", "endpoint", "messages", "maxTokens"],
        move |args| {
            let config = config.clone();
            let workspace_root = workspace_root.clone();
            run_local_model_guarded(&config, &workspace_root, args)
        },
    )
}

// ---------------------------------------------------------------------------
// Network boundary (loopback-only, deadline, bytes, cancellation, redaction)
// ---------------------------------------------------------------------------

/// Endpoint host must be loopback-only by default (operator allowlist is the
/// boundary; no environment-driven endpoint substitution happens here — the
/// endpoint arrives only through the governed args).
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "127.0.0.1" || h == "localhost" || h == "::1"
}

/// Split `scheme://host:port[/path]` into (host, port).
fn endpoint_host_port(endpoint: &str) -> std::result::Result<(String, u16), String> {
    let rest = endpoint
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let (authority, _path) = match rest.split_once('/') {
        Some((a, p)) => (a, p),
        None => (rest, ""),
    };
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        // IPv6 literal: [::1]:8080
        let end = stripped
            .find(']')
            .ok_or("local model endpoint has a malformed IPv6 host")?;
        let host = stripped[..end].to_string();
        let port: u16 = if let Some((p, _)) = stripped[end + 1..].split_once(':') {
            p.parse().unwrap_or(80)
        } else {
            80
        };
        (host, port)
    } else {
        let (host, port_str) = authority.rsplit_once(':').unwrap_or((authority, "80"));
        (host.to_string(), port_str.parse().unwrap_or(80))
    };
    Ok((host, port))
}

/// Find the first occurrence of `needle` in `haystack` (byte-wise).
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|i| &haystack[*i..*i + needle.len()] == needle)
}

fn local_model_io_failure(error: std::io::Error) -> LocalModelFailure {
    let status = if error.kind() == std::io::ErrorKind::TimedOut {
        "timeout"
    } else {
        "transport-error"
    };
    LocalModelFailure::new(
        status,
        format!("local model inference transport failed: {error}"),
    )
}

/// One bounded OpenAI-compatible chat-completions exchange over the local
/// transport: loopback-only destination, configured deadline, bounded response
/// bytes, cooperative cancellation.
fn local_http_infer(
    endpoint: &str,
    body: &str,
    timeout: Duration,
    max_response_bytes: usize,
    cancel: &AtomicBool,
) -> std::result::Result<String, LocalModelFailure> {
    let (host, port) =
        endpoint_host_port(endpoint).map_err(|e| LocalModelFailure::new("invalid-endpoint", e))?;
    if !is_loopback_host(&host) {
        return Err(LocalModelFailure::new(
            "nonlocal-endpoint",
            format!(
                "local model endpoint {endpoint:?} is not loopback-only; refusing the network effect"
            ),
        ));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(LocalModelFailure::new(
            "cancelled",
            "local model inference cancelled before the network effect",
        ));
    }
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|e| {
        let status = if e.kind() == std::io::ErrorKind::TimedOut {
            "timeout"
        } else {
            "connection-refused"
        };
        LocalModelFailure::new(
            status,
            format!("local model inference connect to {endpoint} failed: {e}"),
        )
    })?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| {
        LocalModelFailure::new(
            "timeout",
            format!("local model read timeout could not be applied: {e}"),
        )
    })?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| {
        LocalModelFailure::new(
            "timeout",
            format!("local model write timeout could not be applied: {e}"),
        )
    })?;
    let head = format!(
        "POST /chat/completions HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccept: application/json\r\nConnection: close\r\nUser-Agent: prometheos-lite\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(local_model_io_failure)?;
    stream
        .write_all(body.as_bytes())
        .map_err(local_model_io_failure)?;
    stream.flush().map_err(local_model_io_failure)?;
    read_local_model_response(&mut stream, timeout, max_response_bytes)
}

/// Read one bounded HTTP/1.1 response: headers + content-length body, with
/// the configured deadline and response budget enforced.
fn read_local_model_response(
    stream: &mut TcpStream,
    timeout: Duration,
    max_response_bytes: usize,
) -> std::result::Result<String, LocalModelFailure> {
    let deadline = Instant::now() + timeout;
    let mut response: Vec<u8> = Vec::new();
    let mut header_end: Option<usize> = None;
    let mut content_length: Option<usize> = None;
    let mut received: usize = 0;
    let mut buf = [0u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(LocalModelFailure::new(
                "timeout",
                format!("local model inference timed out"),
            ));
        }
        if let (Some(he), Some(cl)) = (header_end, content_length) {
            if received >= he + cl {
                break;
            }
        }
        let n = stream.read(&mut buf).map_err(local_model_io_failure)?;
        if n == 0 {
            return Err(LocalModelFailure::new(
                "truncated-response",
                "local model response ended before the declared body arrived",
            ));
        }
        response.extend_from_slice(&buf[..n]);
        received += n;
        if header_end.is_none() {
            if let Some(pos) = find_bytes(&response, b"\r\n\r\n") {
                let first_crlf = find_bytes(&response, b"\r\n").ok_or_else(|| {
                    LocalModelFailure::new(
                        "invalid-response",
                        "local model response is missing a status line",
                    )
                })?;
                let status_line = String::from_utf8_lossy(&response[..first_crlf]).into_owned();
                let status_code = status_line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| {
                        LocalModelFailure::new(
                            "invalid-response",
                            "local model response has a malformed status line",
                        )
                    })?;
                if !(200..300).contains(&status_code) {
                    return Err(LocalModelFailure::new(
                        "unexpected-http-response",
                        format!("local model endpoint returned HTTP {status_code}"),
                    ));
                }
                header_end = Some(pos + 4);
                let head = String::from_utf8_lossy(&response[..pos]).into_owned();
                for line in head.split('\n') {
                    let line = line.trim_end_matches('\r');
                    if let Some((key, value)) = line.split_once(':') {
                        if key.trim().eq_ignore_ascii_case("content-length") {
                            if let Ok(parsed) = value.trim().parse::<usize>() {
                                content_length = Some(parsed);
                            }
                        }
                    }
                }
            }
        }
    }
    let header_end = header_end.ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response is missing a header terminator",
        )
    })?;
    let content_length = content_length.ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response is missing content-length",
        )
    })?;
    if received < header_end + content_length {
        return Err(LocalModelFailure::new(
            "truncated-response",
            "local model response body was truncated",
        ));
    }
    let body = &response[header_end..header_end + content_length];
    if body.len() > max_response_bytes {
        return Err(LocalModelFailure::new(
            "oversized-response",
            format!(
                "local model response exceeded the configured budget of {max_response_bytes} bytes"
            ),
        ));
    }
    Ok(String::from_utf8_lossy(body).into_owned())
}

/// Recursively redact every string leaf of a JSON value with `redactor`.
/// Mirrors the native harness's structure-aware redaction: quote/backslash
/// sequences inside secrets cannot damage JSON grammar.
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

/// Bounded identifier argument (correlation/request/step identity).
fn parse_bounded_id(obj: &serde_json::Map<String, Value>, key: &str) -> anyhow::Result<String> {
    let s = obj
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("local model argument {key:?} must be a non-empty string")
        })?;
    if s.len() > 512 {
        anyhow::bail!("local model argument {key:?} exceeds the length limit");
    }
    Ok(s.to_string())
}

/// Build the governed success document from one verified local response.
fn build_local_model_success(
    response: &str,
    model: &str,
    json_mode: bool,
    latency_ms: u64,
    request_id: &str,
    correlation_id: &str,
) -> std::result::Result<Value, LocalModelFailure> {
    let parsed: Value = serde_json::from_str(response).map_err(|_| {
        LocalModelFailure::new(
            "invalid-json",
            format!("local model response was not valid JSON: {response}"),
        )
    })?;
    if parsed.get("model").and_then(Value::as_str) != Some(model) {
        return Err(LocalModelFailure::new(
            "model-mismatch",
            format!(
                "local model responded with model {:?}, expected {model:?}",
                parsed.get("model").and_then(Value::as_str)
            ),
        ));
    }
    let choices = parsed
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            LocalModelFailure::new(
                "invalid-response",
                "local model response is missing the choices array",
            )
        })?;
    let choice = choices.get(0).ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response is missing choices[0]",
        )
    })?;
    let message = choice.get("message").ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response choice is missing message",
        )
    })?;
    let content_str = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LocalModelFailure::new(
                "invalid-response",
                "local model response is missing message content",
            )
        })?;
    let content = if json_mode {
        serde_json::from_str::<Value>(content_str).map_err(|_| {
            LocalModelFailure::new(
                "invalid-structured-output",
                format!("local model structured output was not valid JSON: {content_str}"),
            )
        })?
    } else {
        json!(content_str)
    };
    let usage_raw = parsed.get("usage").cloned().unwrap_or(Value::Null);
    let usage = json!({
        "promptTokens": usage_raw.get("prompt_tokens").and_then(Value::as_u64),
        "completionTokens": usage_raw.get("completion_tokens").and_then(Value::as_u64),
        "totalTokens": usage_raw.get("total_tokens").and_then(Value::as_u64),
    });
    Ok(json!({
        "harnessId": LOCAL_MODEL_HARNESS_ID,
        "capability": LOCAL_MODEL_INFER_CAPABILITY,
        "model": model,
        "content": content,
        "usage": usage,
        "latencyMs": latency_ms,
        "requestId": request_id,
        "correlationId": correlation_id,
    }))
}

/// Governed local-model inference: OpenAI-compatible chat completions over a
/// loopback-only endpoint, bounded by deadline, bytes, cancellation, redaction
/// and fail-closed failure classification.
fn run_local_model_guarded(
    config: &LocalModelConfig,
    _workspace_root: &PathBuf,
    args: &Value,
) -> Result<String> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("local model arguments must be an object"))?;
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "model"
                | "endpoint"
                | "messages"
                | "maxTokens"
                | "responseFormat"
                | "timeoutMs"
                | "knownSecrets"
                | "requestId"
                | "correlationId"
                | "stepId"
        ) {
            return Ok(governed_failure(
                "unsupported-argument",
                format!("local model argument {key:?} is not supported"),
            ));
        }
    }
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("local model argument \"model\" must be a non-empty string")
        })?;
    if model.chars().count() > 256
        || !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Ok(governed_failure(
            "invalid-model",
            format!("local model identifier {model:?} is not supported"),
        ));
    }
    if model != config.model {
        return Ok(governed_failure(
            "model-mismatch",
            format!(
                "local model {model:?} does not match the configured model {:?}",
                config.model
            ),
        ));
    }
    let endpoint = obj
        .get("endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("local model argument \"endpoint\" must be a non-empty string")
        })?;
    if endpoint != config.endpoint {
        return Ok(governed_failure(
            "endpoint-mismatch",
            format!(
                "local model endpoint {endpoint:?} is not the configured allowlisted endpoint {:?}",
                config.endpoint
            ),
        ));
    }
    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("local model argument \"messages\" must be an array"))?;
    for message in messages {
        let message_object = message
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("local model messages must be objects"))?;
        let role = message_object
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(role, "system" | "user" | "assistant" | "tool") {
            return Ok(governed_failure(
                "invalid-arguments",
                format!("local model message role {role:?} is not supported"),
            ));
        }
        let content = message_object
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        if content.len() > 8192 {
            return Ok(governed_failure(
                "invalid-arguments",
                "local model message content exceeds the bounded size".to_string(),
            ));
        }
    }
    let max_tokens = obj
        .get("maxTokens")
        .and_then(Value::as_u64)
        .filter(|t| *t >= 1 && *t <= 8192)
        .unwrap_or(1024);
    let timeout_ms = obj
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .filter(|ms| *ms >= 1)
        .unwrap_or(config.timeout_ms);
    let timeout = Duration::from_millis(timeout_ms);
    let mut secrets = config.known_secrets.clone();
    if let Some(arr) = obj.get("knownSecrets").and_then(Value::as_array) {
        for item in arr {
            if let Some(secret) = item.as_str() {
                if !secret.is_empty() && !secrets.iter().any(|e| e == secret) {
                    secrets.push(secret.to_string());
                }
            }
        }
    }
    let request_id = parse_bounded_id(obj, "requestId").context("local model request id")?;
    let correlation_id =
        parse_bounded_id(obj, "correlationId").context("local model correlation id")?;

    let started = Instant::now();
    let mut body_value = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
    });
    if let Some(response_format) = obj.get("responseFormat") {
        if let serde_json::Value::Object(map) = &mut body_value {
            map.insert("response_format".to_string(), response_format.clone());
        }
    }
    let body_text = serde_json::to_string(&body_value).context("local model request serializes")?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let json_mode = obj
        .get("responseFormat")
        .and_then(Value::as_object)
        .and_then(|m| m.get("type"))
        .and_then(Value::as_str)
        .map(|t| t == "json" || t == "json_object")
        .unwrap_or(false);
    let outcome = local_http_infer(
        &endpoint,
        &body_text,
        timeout,
        config.max_output_bytes,
        &config.cancel,
    );
    let result_value = match outcome {
        Ok(response) => build_local_model_success(
            &response,
            &model,
            json_mode,
            latency_ms,
            &request_id,
            &correlation_id,
        )?,
        Err(failure) => return Ok(governed_failure(failure.status.as_str(), failure.message)),
    };
    let mut result_value = result_value;
    let redactor = Redactor::new().with_known_secrets(&secrets);
    redact_json_value(&mut result_value, &redactor);
    let text = governed_success(result_value, latency_ms, &request_id, &correlation_id);
    if redactor.redact(&text) != text {
        return Ok(governed_failure(
            "secret-bearing-response",
            "local model result still contains a known secret after redaction; refusing to persist",
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_model_harness_declares_local_family_and_capability() {
        let harness = LocalModelHarness::new();
        assert_eq!(harness.declaration().harness_id, LOCAL_MODEL_HARNESS_ID);
        assert_eq!(harness.declaration().family, HarnessFamily::LocalModel);
        assert_eq!(
            harness.declaration().declared_capabilities,
            vec![LOCAL_MODEL_INFER_CAPABILITY.to_string()]
        );
    }
}
