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
            // Never reflect the raw output (it may carry attacker-influenced
            // or secret-adjacent text); the typed status is sufficient.
            let status = parsed
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("local model inference did not succeed (status {status})");
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
// Network boundary (strict endpoint contract, one end-to-end deadline,
// bounded reads during IO, cooperative cancellation, redaction)
// ---------------------------------------------------------------------------

/// Hard cap on response headers, enforced DURING the read, not afterward.
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Short IO poll quantum so the deadline and cancellation take effect
/// promptly even while the socket is blocked in a blocking read/write.
const IO_POLL: Duration = Duration::from_millis(50);

/// Aggregate request budget: maximum number of messages accepted in one
/// inference request.
const MAX_MESSAGES: usize = 32;
/// Aggregate request budget: maximum serialized size of the request body
/// (including the envelope keys), enforced before any transport.
const MAX_REQUEST_BYTES: usize = 32 * 1024;

/// A validated, strict endpoint: lowercase `http` scheme, no credentials,
/// no query or fragment, numeric loopback destination (127.0.0.0/8 or ::1;
/// `localhost` normalized to 127.0.0.1), explicit or default port 80, and a
/// validated absolute-path prefix that is preserved in the request path.
#[derive(Debug, Clone)]
struct EndpointParts {
    ip: std::net::IpAddr,
    port: u16,
    /// Validated absolute-path prefix ("/v1" or "" for the root). No `.`/`..`
    /// segments, no unsafe characters.
    base_path: String,
    /// Display form for the HTTP `Host` header.
    host_header: String,
}

/// Strict endpoint validation (fail closed). DNS names other than
/// `localhost` are refused; HTTPS endpoints are refused because TLS is not
/// implemented. Credentials, query strings and fragments are rejected.
fn parse_endpoint(endpoint: &str) -> std::result::Result<EndpointParts, LocalModelFailure> {
    let rest = endpoint.trim();
    if !rest.starts_with("http://") {
        let scheme = rest.split('/').next().unwrap_or("");
        if scheme.eq_ignore_ascii_case("https:") {
            return Err(LocalModelFailure::new(
                "unsupported-scheme",
                "local model endpoint uses plain http only; TLS endpoints are refused",
            ));
        }
        return Err(LocalModelFailure::new(
            "unsupported-scheme",
            "local model endpoint must use the lowercase 'http' scheme",
        ));
    }
    let rest = &rest["http://".len()..];
    if rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(LocalModelFailure::new(
            "invalid-endpoint",
            "local model endpoint must not contain credentials, a query string or a fragment",
        ));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(|| {
            LocalModelFailure::new(
                "invalid-endpoint",
                "local model endpoint has a malformed IPv6 host",
            )
        })?;
        let host = stripped[..end].to_string();
        let after = &stripped[end + 1..];
        let port: u16 = if let Some(p) = after.strip_prefix(':') {
            p.parse().map_err(|_| {
                LocalModelFailure::new("invalid-endpoint", "local model endpoint port is not valid")
            })?
        } else if after.is_empty() {
            80
        } else {
            return Err(LocalModelFailure::new(
                "invalid-endpoint",
                "local model endpoint is malformed after the IPv6 host",
            ));
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => {
                let port: u16 = p.parse().map_err(|_| {
                    LocalModelFailure::new(
                        "invalid-endpoint",
                        "local model endpoint port is not valid",
                    )
                })?;
                (h.to_string(), port)
            }
            None => (authority.to_string(), 80),
        }
    };
    if host.is_empty() {
        return Err(LocalModelFailure::new(
            "invalid-endpoint",
            "local model endpoint has an empty host",
        ));
    }
    let ip: std::net::IpAddr = if host.eq_ignore_ascii_case("localhost") {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        host.parse().map_err(|_| {
            LocalModelFailure::new(
                "nonlocal-endpoint",
                "local model endpoint host must be a numeric loopback address; DNS names are refused",
            )
        })?
    };
    let loopback = match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !loopback {
        return Err(LocalModelFailure::new(
            "nonlocal-endpoint",
            "local model endpoint is not loopback-only; refusing the network effect",
        ));
    }
    if port == 0 {
        return Err(LocalModelFailure::new(
            "invalid-endpoint",
            "local model endpoint port 0 is not valid",
        ));
    }
    if !path.starts_with('/') {
        return Err(LocalModelFailure::new(
            "invalid-endpoint",
            "local model endpoint path is not absolute",
        ));
    }
    let prefix = path.trim_end_matches('/').to_string();
    if !prefix.is_empty() {
        for seg in prefix.split('/').skip(1) {
            if seg.is_empty() || seg == "." || seg == ".." {
                return Err(LocalModelFailure::new(
                    "invalid-endpoint",
                    "local model endpoint path contains an empty or dot segment",
                ));
            }
            if !seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err(LocalModelFailure::new(
                    "invalid-endpoint",
                    "local model endpoint path contains unsafe characters",
                ));
            }
        }
    }
    Ok(EndpointParts {
        ip,
        port,
        base_path: prefix,
        host_header: format!("{ip}:{port}"),
    })
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

/// Send `data` with the invocation deadline and cooperative cancellation
/// honoured between socket write quanta.
fn write_with_deadline(
    stream: &mut TcpStream,
    data: &[u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<(), LocalModelFailure> {
    stream.set_write_timeout(Some(IO_POLL)).map_err(|error| {
        LocalModelFailure::new(
            "timeout",
            format!("local model write timeout could not be applied: {error}"),
        )
    })?;
    let mut sent = 0usize;
    while sent < data.len() {
        if Instant::now() >= deadline {
            return Err(LocalModelFailure::new(
                "timeout",
                "local model inference timed out",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(LocalModelFailure::new(
                "cancelled",
                "local model inference cancelled",
            ));
        }
        match stream.write(&data[sent..]) {
            Ok(0) => {
                return Err(LocalModelFailure::new(
                    "transport-error",
                    "local model inference request write made no progress",
                ));
            }
            Ok(n) => sent += n,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => return Err(local_model_io_failure(error)),
        }
    }
    Ok(())
}

/// Parsed response headers: `(head_end, content_length)` where `head_end` is
/// the byte offset of the body (end of the blank line) and `content_length`
/// is the declared body size.
fn try_parse_headers(
    response: &[u8],
    max_response_bytes: usize,
    total_cap: usize,
) -> std::result::Result<Option<(usize, usize)>, LocalModelFailure> {
    let Some(pos) = find_bytes(response, b"\r\n\r\n") else {
        return Ok(None);
    };
    let first_crlf = find_bytes(response, b"\r\n").ok_or_else(|| {
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
    let head_end = pos.checked_add(4).ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response declares an overflowing header",
        )
    })?;
    let head = String::from_utf8_lossy(&response[..pos]).into_owned();
    let mut declared: Option<usize> = None;
    for line in head.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case("content-length")
        {
            if declared.is_some() {
                return Err(LocalModelFailure::new(
                    "invalid-response",
                    "local model response declares content-length more than once",
                ));
            }
            let parsed: usize = value.trim().parse().map_err(|_| {
                LocalModelFailure::new(
                    "invalid-response",
                    "local model response declares a non-numeric content-length",
                )
            })?;
            declared = Some(parsed);
        }
    }
    let cl = declared.ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response is missing content-length",
        )
    })?;
    if cl > max_response_bytes {
        return Err(LocalModelFailure::new(
            "oversized-response",
            format!(
                "local model response declared a body of {cl} bytes, above the configured budget of {max_response_bytes} bytes"
            ),
        ));
    }
    let total = head_end.checked_add(cl).ok_or_else(|| {
        LocalModelFailure::new(
            "invalid-response",
            "local model response declares an overflowing content-length",
        )
    })?;
    if total > total_cap {
        return Err(LocalModelFailure::new(
            "oversized-response",
            "local model response exceeded the configured budget",
        ));
    }
    Ok(Some((head_end, cl)))
}

/// Read one bounded HTTP/1.1 response with the invocation deadline and
/// cooperative cancellation honoured between read quanta. Headers are
/// bounded by [`MAX_HEADER_BYTES`] and the body by `max_response_bytes`;
/// both limits are enforced BEFORE any byte is appended.
fn read_with_deadline(
    stream: &mut TcpStream,
    deadline: Instant,
    cancel: &AtomicBool,
    max_response_bytes: usize,
) -> std::result::Result<String, LocalModelFailure> {
    stream.set_read_timeout(Some(IO_POLL)).map_err(|error| {
        LocalModelFailure::new(
            "timeout",
            format!("local model read timeout could not be applied: {error}"),
        )
    })?;
    let total_cap = MAX_HEADER_BYTES.saturating_add(max_response_bytes);
    let mut response: Vec<u8> = Vec::new();
    let mut header_end: Option<usize> = None;
    let mut content_length: Option<usize> = None;
    let mut buf = [0u8; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(LocalModelFailure::new(
                "timeout",
                "local model inference timed out",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(LocalModelFailure::new(
                "cancelled",
                "local model inference cancelled",
            ));
        }
        if let (Some(he), Some(cl)) = (header_end, content_length)
            && response.len() >= he.saturating_add(cl)
        {
            break;
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                return Err(LocalModelFailure::new(
                    "truncated-response",
                    "local model response ended before the declared body arrived",
                ));
            }
            Ok(n) => {
                if header_end.is_none() {
                    if response.len().saturating_add(n) > MAX_HEADER_BYTES {
                        return Err(LocalModelFailure::new(
                            "oversized-header",
                            "local model response headers exceeded the configured bound",
                        ));
                    }
                } else if response.len().saturating_add(n) > total_cap {
                    return Err(LocalModelFailure::new(
                        "oversized-response",
                        "local model response exceeded the configured budget",
                    ));
                }
                response.extend_from_slice(&buf[..n]);
                if header_end.is_none()
                    && let Some((he, cl)) =
                        try_parse_headers(&response, max_response_bytes, total_cap)?
                {
                    header_end = Some(he);
                    content_length = Some(cl);
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => return Err(local_model_io_failure(error)),
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
    let body = &response[header_end..header_end.saturating_add(content_length)];
    Ok(String::from_utf8_lossy(body).into_owned())
}

/// One bounded OpenAI-compatible chat-completions exchange over the local
/// transport: strict loopback-only destination, ONE end-to-end deadline,
/// bounded response bytes enforced during reads, cooperative cancellation
/// honoured during connect/write/read.
fn local_http_infer(
    endpoint: &str,
    body: &str,
    timeout: Duration,
    max_response_bytes: usize,
    cancel: &AtomicBool,
) -> std::result::Result<String, LocalModelFailure> {
    let ep = parse_endpoint(endpoint)?;
    if cancel.load(Ordering::SeqCst) {
        return Err(LocalModelFailure::new(
            "cancelled",
            "local model inference cancelled before the network effect",
        ));
    }
    let deadline = Instant::now() + timeout;
    // Initiate the connection in polled quanta so both the deadline and
    // cancellation take effect during connect.
    let addr = std::net::SocketAddr::new(ep.ip, ep.port);
    let mut stream = loop {
        if Instant::now() >= deadline {
            return Err(LocalModelFailure::new(
                "timeout",
                "local model inference timed out",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(LocalModelFailure::new(
                "cancelled",
                "local model inference cancelled",
            ));
        }
        match std::net::TcpStream::connect_timeout(&addr, IO_POLL) {
            Ok(s) => break s,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => {
                let status = if error.kind() == std::io::ErrorKind::TimedOut {
                    "timeout"
                } else {
                    "connection-refused"
                };
                return Err(LocalModelFailure::new(
                    status,
                    format!("local model inference connect to {addr} failed: {error}"),
                ));
            }
        }
    };
    stream.set_nodelay(true).map_err(|error| {
        LocalModelFailure::new(
            "transport-error",
            format!("local model inference socket could not be configured: {error}"),
        )
    })?;
    let request_path = if ep.base_path.is_empty() {
        "/chat/completions".to_string()
    } else {
        format!("{}/chat/completions", ep.base_path)
    };
    let head = format!(
        "POST {request_path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccept: application/json\r\nConnection: close\r\nUser-Agent: prometheos-lite\r\n\r\n",
        ep.host_header,
        body.len()
    );
    write_with_deadline(&mut stream, head.as_bytes(), deadline, cancel)?;
    write_with_deadline(&mut stream, body.as_bytes(), deadline, cancel)?;
    stream.flush().map_err(local_model_io_failure)?;
    read_with_deadline(&mut stream, deadline, cancel, max_response_bytes)
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
            "local model response was not valid JSON".to_string(),
        )
    })?;
    if parsed.get("model").and_then(Value::as_str) != Some(model) {
        return Err(LocalModelFailure::new(
            "model-mismatch",
            "local model responded with a different model than the configured one".to_string(),
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
    let choice = choices.first().ok_or_else(|| {
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
                "local model structured output was not valid JSON".to_string(),
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
    if messages.len() > MAX_MESSAGES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "local model request exceeded the aggregate message budget of {MAX_MESSAGES} messages"
            ),
        ));
    }
    let aggregate_messages_bytes: usize = messages.iter().map(|m| m.to_string().len()).sum();
    if aggregate_messages_bytes > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "local model request exceeded the aggregate message budget of {MAX_REQUEST_BYTES} bytes"
            ),
        ));
    }
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
    if config.timeout_ms == 0 {
        return Ok(governed_failure(
            "invalid-config",
            "local model operator-configured timeout is zero".to_string(),
        ));
    }
    let timeout_ms = match obj.get("timeoutMs") {
        None => config.timeout_ms,
        Some(value) => {
            let Some(ms) = value.as_u64() else {
                return Ok(governed_failure(
                    "invalid-timeout",
                    "local model argument \"timeoutMs\" must be a positive integer".to_string(),
                ));
            };
            if ms == 0 {
                return Ok(governed_failure(
                    "invalid-timeout",
                    "local model argument \"timeoutMs\" must be at least 1".to_string(),
                ));
            }
            if ms > config.timeout_ms {
                return Ok(governed_failure(
                    "invalid-timeout",
                    format!(
                        "local model argument \"timeoutMs\" exceeds the configured timeout ceiling of {} ms",
                        config.timeout_ms
                    ),
                ));
            }
            ms
        }
    };
    let timeout = Duration::from_millis(timeout_ms);
    let mut secrets = config.known_secrets.clone();
    if let Some(arr) = obj.get("knownSecrets").and_then(Value::as_array) {
        for item in arr {
            if let Some(secret) = item.as_str()
                && !secret.is_empty()
                && !secrets.iter().any(|e| e == secret)
            {
                secrets.push(secret.to_string());
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
    if let Some(response_format) = obj.get("responseFormat")
        && let serde_json::Value::Object(map) = &mut body_value
    {
        map.insert("response_format".to_string(), response_format.clone());
    }
    let body_text = serde_json::to_string(&body_value).context("local model request serializes")?;
    if body_text.len() > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "local model request exceeded the aggregate request budget of {MAX_REQUEST_BYTES} bytes"
            ),
        ));
    }
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
    // Measure actual elapsed time after the inference completes; measuring
    // before the transport would record ~0 regardless of real latency.
    let latency_ms = started.elapsed().as_millis() as u64;
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
