//! Governed remote-service inference harness (issue #154, Slice I).
//!
//! Delivers the **remote-service** production harness family
//! ([`HarnessFamily::RemoteService`]) through the single sanctioned effect path:
//! [`HarnessDispatch::call`] → [`crate::workflow::node_runner::NodeRunner`] →
//! governed capability. The adapter speaks an OpenAI-compatible chat
//! completions protocol to an operator-configured HTTPS endpoint and never
//! invokes the declared capability outside its declaration.
//!
//! Network boundary (Slice I): HTTPS with ordinary certificate verification
//! (system roots plus any operator-configured extra trust anchors; TLS is
//! never disabled), an explicit destination allowlist, refusal of loopback /
//! private / link-local / CGNAT / reserved / documentation / multicast /
//! unspecified / metadata-service destinations unless the operator has
//! explicitly set `allow_private_destinations` (and even then metadata
//! services are always refused), no redirect following, no proxy
//! inheritance, no DNS-rebinding escape (resolution is validated once, pinned
//! for the connection, and a mixed public/private answer set is refused),
//! no credentials in URLs, no endpoint override from task input, strictly
//! bounded request and response sizes, one end-to-end invocation deadline
//! with active cooperative cancellation, and redaction before persistence.
//! Capability declarations never grant authority; unsupported combinations
//! refuse before any effect.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
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

pub const REMOTE_SERVICE_HARNESS_ID: &str = "lite.harness.remote_service";
pub const REMOTE_SERVICE_INFER_CAPABILITY: &str = "harness.remote_service.infer";
pub const REMOTE_SERVICE_IDENTITY_NAME: &str = "lite-remote-service";
pub const REMOTE_SERVICE_REVISION: &str = "v1";
pub const REMOTE_SERVICE_SCHEMA: &str = "lite.remote.infer";
pub const REMOTE_SERVICE_VERSION: &str = "1.0.0";
pub const REMOTE_SERVICE_CAPS_ID: &str = "lite.harness.remote_service.caps";

/// Poll quantum for cooperative cancellation while the request is in flight.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// Aggregate request budget: maximum number of messages accepted in one
/// inference request.
const MAX_MESSAGES: usize = 32;
/// Aggregate request budget: maximum serialized size of the request body
/// (including the envelope keys), enforced before any transport.
const MAX_REQUEST_BYTES: usize = 32 * 1024;
/// Default operator ceiling on generated output tokens. Bounded on purpose;
/// operators may raise or lower it via `max_output_tokens`.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 4096;

/// Configuration for the governed remote-service handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode. Credentials come from this operator-owned configuration,
/// never from task arguments, and are redacted from every artefact this
/// adapter produces.
#[derive(Debug, Clone)]
pub struct RemoteServiceConfig {
    /// Operator-configured HTTPS origin
    /// (e.g. `https://api.example.com/v1`). Must be `https`, carry no
    /// credentials, query string or fragment, and its host must appear in
    /// `allowed_hosts`. There is no discovery and no fallback endpoint.
    pub endpoint: String,
    /// Explicit destination allowlist. Entries are exact lowercase host
    /// (`api.example.com`, matched port 443) or host-port
    /// (`api.example.com:8443`, matched exactly) values; no wildcards.
    /// The endpoint must match exactly one entry.
    pub allowed_hosts: Vec<String>,
    /// Model identifier the endpoint must serve (provenance evidence). The
    /// wire response must attest exactly this model or the inference is
    /// refused; a silent model substitution is impossible.
    pub model: String,
    /// Provider credential, supplied by the operator through the approved
    /// secret boundary at runner construction time. Never accepted from task
    /// arguments, never logged, never persisted. May be empty for an
    /// operator-configured unauthenticated internal endpoint.
    pub api_key: String,
    /// Default invocation deadline when the step omits `timeoutMs`.
    pub timeout_ms: u64,
    /// Connect-phase deadline, capped below `timeout_ms`.
    pub connect_timeout_ms: u64,
    /// Operator ceiling on the `max_tokens` the provider may generate per
    /// invocation. Requests above the ceiling are refused before any network
    /// effect. Defaults to a bounded value; invalid (zero) fails closed.
    pub max_output_tokens: u64,
    /// Hard budget, in bytes, of the remote response body accepted per
    /// invocation.
    pub max_output_bytes: usize,
    /// Additional trust anchors (PEM-encoded certificates), additive to the
    /// platform system roots — e.g. a private enterprise CA. Verification is
    /// never weakened; an empty set means system roots only.
    pub extra_root_certs_pem: Vec<Vec<u8>>,
    /// Explicit operator policy: when false (default), loopback, private,
    /// link-local, CGNAT, documentation, reserved, multicast and unspecified
    /// destinations are refused. When true, those classes are allowed for
    /// governed internal services. Metadata services are refused regardless.
    pub allow_private_destinations: bool,
    /// Known secret values, redacted verbatim from the verified result.
    pub known_secrets: Vec<String>,
    /// Cooperative cancellation source; setting it aborts the invocation.
    pub cancel: Arc<AtomicBool>,
}

impl Default for RemoteServiceConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            allowed_hosts: Vec::new(),
            model: "remote-model-default".to_string(),
            api_key: String::new(),
            timeout_ms: 30_000,
            connect_timeout_ms: 10_000,
            max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
            max_output_bytes: 256 * 1024,
            extra_root_certs_pem: Vec::new(),
            allow_private_destinations: false,
            known_secrets: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl RemoteServiceConfig {
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            ..Self::default()
        }
    }

    pub fn allowed_hosts(mut self, allowed_hosts: Vec<String>) -> Self {
        self.allowed_hosts = allowed_hosts;
        self
    }

    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = api_key.into();
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn max_output_tokens(mut self, max_output_tokens: u64) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    pub fn connect_timeout_ms(mut self, connect_timeout_ms: u64) -> Self {
        self.connect_timeout_ms = connect_timeout_ms;
        self
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn extra_root_certs_pem(mut self, extra_root_certs_pem: Vec<Vec<u8>>) -> Self {
        self.extra_root_certs_pem = extra_root_certs_pem;
        self
    }

    pub fn allow_private_destinations(mut self, allow: bool) -> Self {
        self.allow_private_destinations = allow;
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

/// Typed failure classification for one governed remote-service invocation.
///
/// The classification is the typed failure surface the harness outcome
/// contract requires; it never widens authority, it only names the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteServiceFailure {
    status: String,
    message: String,
}

impl RemoteServiceFailure {
    fn new(status: &str, message: impl Into<String>) -> Self {
        Self {
            status: status.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RemoteServiceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for RemoteServiceFailure {}

fn governed_failure(status: &str, error: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "success": false,
        "status": status,
        "output": { "status": status, "error": error.into() },
    }))
    .context("remote service failure serializes")
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
            "harnessId": REMOTE_SERVICE_HARNESS_ID,
            "capability": REMOTE_SERVICE_INFER_CAPABILITY,
            "content": output,
            "latencyMs": latency_ms,
            "requestId": request_id,
            "correlationId": correlation_id,
        },
    }))
    .context("remote service success serializes")
    .expect("governed success document serializes")
}

/// Versioned remote-service declaration owned by this implementation.
pub fn remote_service_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
        identity: Identity {
            name: REMOTE_SERVICE_IDENTITY_NAME.to_string(),
            revision: REMOTE_SERVICE_REVISION.to_string(),
        },
        family: HarnessFamily::RemoteService,
        declared_capabilities: vec![REMOTE_SERVICE_INFER_CAPABILITY.to_string()],
        claimed_capabilities: vec![REMOTE_SERVICE_INFER_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: REMOTE_SERVICE_CAPS_ID.to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: REMOTE_SERVICE_IDENTITY_NAME.to_string(),
                revision: REMOTE_SERVICE_REVISION.to_string(),
            },
            declared_at: "2026-11-12T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: Vec::new(),
            effect_classes: vec!["network-io".to_string()],
            workspace: None,
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
            event_streaming: Some(false),
            read_projection: Some(false),
            locality: Some(Locality::Remote),
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

/// The replaceable governed remote-service adapter.
pub struct RemoteServiceHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for RemoteServiceHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteServiceHarness {
    pub fn new() -> Self {
        Self {
            declaration: remote_service_declaration(),
        }
    }
}

#[async_trait]
impl ExecutionHarness for RemoteServiceHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The step identifier flows into the governed request, the persisted
        // evidence and the correlation ids; bound it before it reaches any
        // of those surfaces.
        if step.step_id.chars().count() > 512 {
            anyhow::bail!(
                "remote service step identifier exceeds the 512 character bound; refusing before dispatch"
            );
        }
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
            serde_json::from_str(&output).context("remote service result must be a JSON result")?;
        if parsed.get("success").and_then(Value::as_bool) != Some(true)
            || parsed.get("status").and_then(Value::as_str) != Some("ok")
        {
            // Never reflect the raw output (it may carry attacker-influenced
            // or secret-adjacent text); the typed status is sufficient.
            let status = parsed
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            anyhow::bail!("remote service inference did not succeed (status {status})");
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
            harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
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
            harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
            harness_revision: REMOTE_SERVICE_REVISION.to_string(),
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
        if checkpoint.harness_id != REMOTE_SERVICE_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {REMOTE_SERVICE_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != REMOTE_SERVICE_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({REMOTE_SERVICE_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: REMOTE_SERVICE_HARNESS_ID.to_string(),
            harness_revision: REMOTE_SERVICE_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.remote_service.infer` capability for a
/// runner's registry, bound to the operator-owned configuration and an
/// acquired workspace root (unused beyond the governed contract).
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently bounded and validated. `workspace_root` must be the
/// governed workspace root for the work; the handler never touches the
/// filesystem, this parameter only preserves the shared capability contract.
pub fn remote_service_capability(
    config: RemoteServiceConfig,
    workspace_root: PathBuf,
) -> Capability {
    Capability::deterministic(
        &["model", "endpoint", "messages", "maxTokens"],
        move |args| {
            let config = config.clone();
            let workspace_root = workspace_root.clone();
            let _ = workspace_root;
            run_remote_service_guarded(&config, args)
        },
    )
}

// ---------------------------------------------------------------------------
// Address classification and destination policy
// ---------------------------------------------------------------------------

/// Hard-denied destination classes, refused regardless of the
/// `allow_private_destinations` operator policy. These are cloud / instance
/// metadata services; a governed remote-service adapter has no legitimate
/// reason to reach them.
fn is_metadata_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 169.254.169.0/24 (AWS/GCP/Azure IMDS), 169.254.170.0/24
            // (ECS task credentials), 100.100.100.200 (Aliyun).
            (o[0] == 169 && o[1] == 254 && (o[2] == 169 || o[2] == 170))
                || (o[0] == 100 && o[1] == 100 && o[2] == 100 && o[3] == 200)
        }
        IpAddr::V6(v6) => {
            // fd00:ec2::254 — AWS EC2 IPv6 metadata service.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_metadata_address(&IpAddr::V4(v4));
            }
            let s = v6.segments();
            s[0] == 0xfd00
                && s[1] == 0x0ec2
                && s[2] == 0
                && s[3] == 0
                && s[4] == 0
                && s[5] == 0
                && s[6] == 0
                && s[7] == 0x0254
        }
    }
}

/// True when the address is not globally routable under the strict default
/// policy (loopback, private, link-local, CGNAT, unspecified, multicast,
/// documentation, reserved).
pub(crate) fn is_non_public_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                // 100.64.0.0/10 CGNAT
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                // 0.0.0.0/8
                || o[0] == 0
                // 192.0.0.0/24 (IETF protocol assignments)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 documentation
                || (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
                // 198.18.0.0/15 benchmarking
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                // 240.0.0.0/4 reserved (incl. 255.255.255.255)
                || (o[0] & 0xf0) == 240
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return true;
            }
            // IPv4-mapped ::ffff:a.b.c.d — classify the embedded address.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_non_public_address(&IpAddr::V4(v4));
            }
            let s = v6.segments();
            // 64:ff9b::/32 NAT64 (well-known and local-use): transition
            // mechanism capable of tunnelling to otherwise unreachable
            // space; refuse the whole range (fail closed).
            if s[0] == 0x0064 && s[1] == 0xff9b {
                return true;
            }
            // fc00::/7 unique-local
            if (s[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // fe80::/10 link-local
            if (s[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // ff00::/8 multicast
            if (s[0] & 0xff00) == 0xff00 {
                return true;
            }
            // 2001:db8::/32 documentation
            if s[0] == 0x2001 && s[1] == 0x0db8 {
                return true;
            }
            // 2001::/32 Teredo and 2002::/16 6to4: transition mechanisms
            // capable of tunnelling to otherwise unreachable space; refuse.
            if s[0] == 0x2001 && s[1] == 0x0000 {
                return true;
            }
            if s[0] == 0x2002 {
                return true;
            }
            // 100::/64 discard-only
            if s[0] == 0x0100 && s[1] == 0x0000 {
                return true;
            }
            false
        }
    }
}

/// Split a `host` or `[v6]` / `host:port` / `[v6]:port` spelling into a
/// normalized (lowercase host, port) pair, or `None` when malformed. A bare
/// IPv6 address is not accepted (it would collide with the port separator).
fn split_host_port(s: &str, default_port: u16) -> Option<(String, u16)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(stripped) = s.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].trim();
        let after = &stripped[end + 1..];
        let port: u16 = if let Some(p) = after.strip_prefix(':') {
            if p.is_empty() || p.bytes().any(|b| !b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()?
        } else if after.is_empty() {
            default_port
        } else {
            return None;
        };
        return Some((host.to_ascii_lowercase(), port));
    }
    if s.matches(':').count() > 1 {
        // Bare IPv6 without brackets: ambiguous with the port separator.
        return None;
    }
    match s.rsplit_once(':') {
        Some((h, p)) => {
            if h.is_empty() || p.is_empty() || p.bytes().any(|b| !b.is_ascii_digit()) {
                return None;
            }
            let port: u16 = p.parse().ok()?;
            Some((h.to_ascii_lowercase(), port))
        }
        None => Some((s.to_ascii_lowercase(), default_port)),
    }
}

/// A validated, strict HTTPS endpoint: explicit `https` scheme, no
/// credentials, no query string or fragment, a validated absolute-path
/// prefix, an explicit or default port 443, and a destination whose every
/// resolved address passes the destination policy.
#[derive(Debug, Clone)]
pub(crate) struct ValidatedEndpoint {
    /// Lowercase host (DNS name or IP literal).
    host: String,
    port: u16,
    /// Validated absolute-path prefix ("/v1" or "" for the root).
    base_path: String,
}

impl ValidatedEndpoint {
    /// The normalized `host` (or `host:port` when not the default 443) form
    /// used for allowlist matching.
    fn allowlist_key(&self) -> String {
        if self.port == 443 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Strict HTTPS endpoint validation (fail closed).
fn parse_endpoint(
    endpoint: &str,
    allow_private_destinations: bool,
) -> std::result::Result<ValidatedEndpoint, RemoteServiceFailure> {
    let rest = endpoint.trim();
    if !rest.starts_with("https://") {
        let scheme = rest.split('/').next().unwrap_or("");
        if scheme.eq_ignore_ascii_case("http:") {
            return Err(RemoteServiceFailure::new(
                "unsupported-scheme",
                "remote service endpoint must use https; plain http is refused",
            ));
        }
        return Err(RemoteServiceFailure::new(
            "unsupported-scheme",
            "remote service endpoint must use the lowercase 'https' scheme",
        ));
    }
    let rest = &rest["https://".len()..];
    if rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(RemoteServiceFailure::new(
            "invalid-endpoint",
            "remote service endpoint must not contain credentials, a query string or a fragment",
        ));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = split_host_port(authority, 443).ok_or_else(|| {
        RemoteServiceFailure::new(
            "invalid-endpoint",
            "remote service endpoint authority is not a valid host[:port]",
        )
    })?;
    if port == 0 {
        return Err(RemoteServiceFailure::new(
            "invalid-endpoint",
            "remote service endpoint port 0 is not valid",
        ));
    }
    if host.is_empty()
        || host.len() > 253
        || host.starts_with('-')
        || host.ends_with('-')
        || host.ends_with('.')
        || host.contains("..")
        || host.contains('%')
    {
        return Err(RemoteServiceFailure::new(
            "invalid-endpoint",
            "remote service endpoint has an invalid host",
        ));
    }
    // A host is either a valid DNS name or a valid IP literal.
    match host.parse::<IpAddr>() {
        Ok(ip) => {
            if is_metadata_address(&ip) {
                return Err(RemoteServiceFailure::new(
                    "destination-refused",
                    "remote service endpoint is a metadata destination; refusing the network effect",
                ));
            }
            if !allow_private_destinations && is_non_public_address(&ip) {
                return Err(RemoteServiceFailure::new(
                    "destination-refused",
                    "remote service endpoint is not a public destination; refusing the network effect",
                ));
            }
        }
        Err(_) => {
            for label in host.split('.') {
                if label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(RemoteServiceFailure::new(
                        "invalid-endpoint",
                        "remote service endpoint host is not a valid DNS name",
                    ));
                }
            }
        }
    }
    if !path.starts_with('/') {
        return Err(RemoteServiceFailure::new(
            "invalid-endpoint",
            "remote service endpoint path is not absolute",
        ));
    }
    let prefix = path.trim_end_matches('/').to_string();
    if !prefix.is_empty() {
        for seg in prefix.split('/').skip(1) {
            if seg.is_empty() || seg == "." || seg == ".." {
                return Err(RemoteServiceFailure::new(
                    "invalid-endpoint",
                    "remote service endpoint path contains an empty or dot segment",
                ));
            }
            if !seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err(RemoteServiceFailure::new(
                    "invalid-endpoint",
                    "remote service endpoint path contains unsafe characters",
                ));
            }
        }
    }
    Ok(ValidatedEndpoint {
        host,
        port,
        base_path: prefix,
    })
}

/// Validate the allowlist: the endpoint's normalized host(:port) must match
/// exactly one configured entry. No wildcards; entries are parsed as
/// `host` (matching only the default 443), `host:port`, `[v6]` or
/// `[v6]:port` and compared exactly.
fn validate_allowlist(
    endpoint: &ValidatedEndpoint,
    allowed_hosts: &[String],
) -> std::result::Result<(), RemoteServiceFailure> {
    if allowed_hosts.is_empty() {
        return Err(RemoteServiceFailure::new(
            "allowlist-refused",
            "remote service requires an explicit destination allowlist; refusing the network effect",
        ));
    }
    let key = endpoint.allowlist_key();
    let matched = allowed_hosts.iter().any(|entry| {
        split_host_port(entry, 443)
            .map(|(h, p)| (h, p) == (endpoint.host.clone(), endpoint.port))
            .unwrap_or(false)
    });
    if matched {
        Ok(())
    } else {
        Err(RemoteServiceFailure::new(
            "allowlist-refused",
            format!("remote service endpoint is not in the configured allowlist ({key})"),
        ))
    }
}

/// Resolver abstraction so the resolution lifecycle is injectable and
/// deterministically testable. The production resolver uses the platform
/// resolver (`getaddrinfo` via `ToSocketAddrs`).
pub(crate) trait AddressResolver: Send {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>>;
}

pub(crate) struct SystemResolver;

impl AddressResolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        (host, port).to_socket_addrs().map(|a| a.collect())
    }
}

/// Validate a full candidate address set (fail closed on ANY refused
/// address; mixed authorized/unauthorized sets are rejected outright).
fn validate_addresses(
    mut addrs: Vec<SocketAddr>,
    allow_private_destinations: bool,
) -> std::result::Result<Vec<SocketAddr>, RemoteServiceFailure> {
    if addrs.is_empty() {
        return Err(RemoteServiceFailure::new(
            "dns-refused",
            "remote service host resolved to no addresses; refusing the network effect",
        ));
    }
    addrs.sort_by_key(|a| (a.ip().to_string(), a.port()));
    addrs.dedup();
    for addr in &addrs {
        if is_metadata_address(&addr.ip()) {
            return Err(RemoteServiceFailure::new(
                "destination-refused",
                "remote service resolved to a metadata destination; refusing the network effect",
            ));
        }
        if !allow_private_destinations && is_non_public_address(&addr.ip()) {
            return Err(RemoteServiceFailure::new(
                "destination-refused",
                "remote service resolved to a non-public address; refusing the network effect",
            ));
        }
    }
    Ok(addrs)
}

/// Resolve the endpoint host with a hard budget, inside the invocation
/// deadline. The blocking OS resolver runs on ONE dedicated thread with a
/// capacity-1 channel; the calling worker wakes at most every 50ms to check
/// the budget and the cooperative cancellation flag, and validates the FULL
/// answer set before any connection is pinned. A resolution exceeding the
/// budget is abandoned and refused — the underlying `getaddrinfo(3)` call may
/// keep running in that single detached thread until the OS returns (one
/// thread per abandoned resolution, never retried, never looping); a
/// synchronous interruption of an in-flight resolver syscall is NOT claimed.
fn resolve_with_budget(
    resolver: Box<dyn AddressResolver>,
    host: &str,
    port: u16,
    budget: Duration,
    cancel: &AtomicBool,
    allow_private_destinations: bool,
) -> std::result::Result<Vec<SocketAddr>, RemoteServiceFailure> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<SocketAddr>>>(1);
    let owned_host = host.to_string();
    let _resolver_thread = std::thread::Builder::new()
        .name("prometheos-resolver".to_string())
        .spawn(move || {
            let outcome = resolver.resolve(&owned_host, port);
            // Capacity is 1: this can never block, even if the waiter was
            // abandoned after a timeout.
            let _ = tx.try_send(outcome);
        })
        .map_err(|_| {
            RemoteServiceFailure::new(
                "dns-refused",
                "remote service resolver thread could not be spawned; refusing the network effect",
            )
        })?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(RemoteServiceFailure::new(
                "cancelled",
                "remote service inference cancelled during resolution",
            ));
        }
        let elapsed = started.elapsed();
        if elapsed >= budget {
            return Err(RemoteServiceFailure::new(
                "timeout",
                "remote service host resolution exceeded the invocation deadline",
            ));
        }
        let wait = Duration::from_millis(50).min(budget - elapsed);
        match rx.recv_timeout(wait) {
            Ok(Ok(addrs)) => return validate_addresses(addrs, allow_private_destinations),
            Ok(Err(_)) => {
                return Err(RemoteServiceFailure::new(
                    "dns-refused",
                    "remote service host could not be resolved; refusing the network effect",
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(RemoteServiceFailure::new(
                    "dns-refused",
                    "remote service resolver thread terminated abnormally; refusing the network effect",
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP transport (reqwest; the repository's existing vetted client — no new
// HTTP parser is written here)
// ---------------------------------------------------------------------------

fn map_reqwest_error(error: &reqwest::Error) -> RemoteServiceFailure {
    if error.is_timeout() {
        return RemoteServiceFailure::new(
            "timeout",
            "remote service request timed out before completion",
        );
    }
    if error.is_connect() || error.is_request() {
        return RemoteServiceFailure::new(
            "transport-error",
            "remote service connection failed or its certificate could not be verified",
        );
    }
    if error.is_body() {
        return RemoteServiceFailure::new(
            "transport-error",
            "remote service response body could not be read within the deadline",
        );
    }
    RemoteServiceFailure::new(
        "transport-error",
        "remote service request failed at the transport layer",
    )
}

fn build_client(
    endpoint: &ValidatedEndpoint,
    pinned: &[SocketAddr],
    config: &RemoteServiceConfig,
) -> std::result::Result<reqwest::Client, RemoteServiceFailure> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .http1_only()
        .connect_timeout(Duration::from_millis(
            config.connect_timeout_ms.min(config.timeout_ms),
        ))
        .timeout(Duration::from_millis(config.timeout_ms))
        .resolve_to_addrs(&endpoint.host, pinned);
    for pem in &config.extra_root_certs_pem {
        let cert = reqwest::Certificate::from_pem(pem).map_err(|_| {
            RemoteServiceFailure::new(
                "invalid-config",
                "an operator-configured extra root certificate is not a valid PEM certificate",
            )
        })?;
        builder = builder.add_root_certificate(cert);
    }
    builder.build().map_err(|_| {
        RemoteServiceFailure::new(
            "transport-error",
            "remote service client could not be constructed for the validated destination",
        )
    })
}

/// Bounded POST with the deadline and cancellation observed across
/// connect/write/read. The response body is read by streaming with a hard
/// byte cap; the raw status code is all that leaves this function.
async fn send_and_read(
    client: &reqwest::Client,
    url: String,
    body: Vec<u8>,
    api_key: &str,
    max_output_bytes: usize,
) -> std::result::Result<(u16, Vec<u8>), RemoteServiceFailure> {
    let mut request = client
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(body);
    if !api_key.is_empty() {
        request = request.header("authorization", format!("Bearer {api_key}"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| map_reqwest_error(&error))?;
    let status = response.status().as_u16();
    // Refuse a declared body above the budget before reading a single byte.
    if let Some(declared) = response.content_length()
        && declared > max_output_bytes as u64
    {
        return Err(RemoteServiceFailure::new(
            "oversized-response",
            format!(
                "remote service declared a body above the configured budget of {max_output_bytes} bytes"
            ),
        ));
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut response = response;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > max_output_bytes {
                    return Err(RemoteServiceFailure::new(
                        "oversized-response",
                        format!(
                            "remote service response exceeded the configured budget of {max_output_bytes} bytes"
                        ),
                    ));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => {
                return Err(RemoteServiceFailure::new(
                    "transport-error",
                    "remote service response body could not be read within the deadline",
                ));
            }
        }
    }
    Ok((status, buf))
}

/// Outcome of the blocking transport round trip on the isolated worker
/// thread.
enum TransportOutcome {
    Cancelled,
    TimedOut,
    Result(Result<(u16, Vec<u8>), RemoteServiceFailure>),
}

/// Run one bounded HTTPS round trip on a fresh OS thread with its own
/// current-thread tokio runtime. This works from ANY caller context —
/// synchronous dispatch inside a governed capability cannot block_on an
/// existing runtime, and a nested runtime panics; the isolated thread never
/// does. The whole lifecycle (connect, write, read) is bounded by `timeout`
/// and abortable via `cancel` (observed every [`CANCEL_POLL`]).
/// Invocation lifecycle executed on ONE dedicated OS thread: DNS resolution
/// (budget-bounded, cancellation-observable), client construction with the
/// validated addresses pinned, then one bounded connect/write/read round
/// trip under the same deadline. `deadline` is measured by the caller from
/// the start of the governed invocation; resolution consumes from the same
/// budget, so an unresponsive resolver can never stall past it.
fn blocking_roundtrip(
    client_builder_endpoint: ValidatedEndpoint,
    config: RemoteServiceConfig,
    url: String,
    body_bytes: Vec<u8>,
    api_key: String,
    max_output_bytes: usize,
    deadline: std::time::Instant,
    cancel: Arc<AtomicBool>,
) -> TransportOutcome {
    let allow_private_destinations = config.allow_private_destinations;
    let handle = std::thread::spawn(move || {
        // ---- DNS, budget-bounded and cancellable ----------------------
        let dns_budget = deadline.saturating_duration_since(std::time::Instant::now());
        if dns_budget.is_zero() {
            return TransportOutcome::TimedOut;
        }
        let pinned = match resolve_with_budget(
            Box::new(SystemResolver),
            &client_builder_endpoint.host,
            client_builder_endpoint.port,
            dns_budget,
            &cancel,
            allow_private_destinations,
        ) {
            Ok(pinned) => pinned,
            Err(failure) => return TransportOutcome::Result(Err(failure)),
        };
        // ---- Client construction, bound to the pinned addresses -------
        let client = match build_client(&client_builder_endpoint, &pinned, &config) {
            Ok(client) => client,
            Err(failure) => return TransportOutcome::Result(Err(failure)),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let runtime = match runtime {
            Ok(runtime) => runtime,
            Err(_) => {
                return TransportOutcome::Result(Err(RemoteServiceFailure::new(
                    "transport-error",
                    "remote service runtime could not be constructed",
                )));
            }
        };
        runtime.block_on(async move {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return TransportOutcome::TimedOut;
            }
            let work = async move {
                tokio::time::timeout(
                    remaining,
                    send_and_read(&client, url, body_bytes, &api_key, max_output_bytes),
                )
                .await
            };
            let watcher = async move {
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    tokio::time::sleep(CANCEL_POLL).await;
                }
            };
            tokio::select! {
                res = work => match res {
                    Ok(inner) => TransportOutcome::Result(inner),
                    Err(_elapsed) => TransportOutcome::TimedOut,
                },
                _ = watcher => TransportOutcome::Cancelled,
            }
        })
    });
    match handle.join() {
        Ok(outcome) => outcome,
        Err(_) => TransportOutcome::Result(Err(RemoteServiceFailure::new(
            "transport-error",
            "remote service worker thread panicked",
        ))),
    }
}

/// Recursively redact every string leaf of a JSON value with `redactor`.
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

/// Build the governed success document from one verified remote response.
fn build_remote_service_success(
    response_body: &[u8],
    model: &str,
    json_mode: bool,
    latency_ms: u64,
    request_id: &str,
    correlation_id: &str,
    redactor: &Redactor,
) -> std::result::Result<Value, RemoteServiceFailure> {
    let parsed: Value = serde_json::from_slice(response_body).map_err(|_| {
        RemoteServiceFailure::new("invalid-json", "remote service response was not valid JSON")
    })?;
    if parsed.get("model").and_then(Value::as_str) != Some(model) {
        return Err(RemoteServiceFailure::new(
            "model-mismatch",
            "remote service responded with a different model than the configured one",
        ));
    }
    let choices = parsed
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            RemoteServiceFailure::new(
                "invalid-response",
                "remote service response is missing the choices array",
            )
        })?;
    let choice = choices.first().ok_or_else(|| {
        RemoteServiceFailure::new(
            "invalid-response",
            "remote service response is missing choices[0]",
        )
    })?;
    let message = choice.get("message").ok_or_else(|| {
        RemoteServiceFailure::new(
            "invalid-response",
            "remote service response choice is missing message",
        )
    })?;
    let content_str = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RemoteServiceFailure::new(
                "invalid-response",
                "remote service response is missing message content",
            )
        })?;
    let content = if json_mode {
        let mut structured: Value = serde_json::from_str(content_str).map_err(|_| {
            RemoteServiceFailure::new(
                "invalid-structured-output",
                "remote service structured output was not valid JSON",
            )
        })?;
        redact_json_value(&mut structured, redactor);
        structured
    } else {
        json!(redactor.redact(content_str))
    };
    let usage_raw = parsed.get("usage").cloned().unwrap_or(Value::Null);
    let usage = json!({
        "promptTokens": usage_raw.get("prompt_tokens").and_then(Value::as_u64),
        "completionTokens": usage_raw.get("completion_tokens").and_then(Value::as_u64),
        "totalTokens": usage_raw.get("total_tokens").and_then(Value::as_u64),
    });
    Ok(json!({
        "harnessId": REMOTE_SERVICE_HARNESS_ID,
        "capability": REMOTE_SERVICE_INFER_CAPABILITY,
        "model": model,
        "content": content,
        "usage": usage,
        "latencyMs": latency_ms,
        "requestId": request_id,
        "correlationId": correlation_id,
    }))
}

/// Governed remote-service inference: HTTPS OpenAI-compatible chat
/// completions against the operator-configured, allowlisted destination —
/// bounded by deadline, bytes, cancellation, redaction and fail-closed
/// failure classification.
fn run_remote_service_guarded(config: &RemoteServiceConfig, args: &Value) -> Result<String> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("remote service arguments must be an object"))?;
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
                format!("remote service argument {key:?} is not supported"),
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
            anyhow::anyhow!("remote service argument \"model\" must be a non-empty string")
        })?;
    if model.chars().count() > 256
        || !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Ok(governed_failure(
            "invalid-model",
            format!("remote service model identifier {model:?} is not supported"),
        ));
    }
    if model != config.model {
        return Ok(governed_failure(
            "model-mismatch",
            format!(
                "remote service model {model:?} does not match the configured model {:?}",
                config.model
            ),
        ));
    }
    // Task input can never override the operator-configured endpoint.
    if let Some(endpoint_arg) = obj.get("endpoint").and_then(Value::as_str)
        && endpoint_arg != config.endpoint
    {
        return Ok(governed_failure(
            "endpoint-mismatch",
            "remote service endpoint in the request is not the operator-configured endpoint; refusing",
        ));
    }
    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("remote service argument \"messages\" must be an array"))?;
    if messages.len() > MAX_MESSAGES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "remote service request exceeded the aggregate message budget of {MAX_MESSAGES} messages"
            ),
        ));
    }
    let aggregate_messages_bytes: usize = messages.iter().map(|m| m.to_string().len()).sum();
    if aggregate_messages_bytes > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "remote service request exceeded the aggregate message budget of {MAX_REQUEST_BYTES} bytes"
            ),
        ));
    }
    for message in messages {
        let message_object = message
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("remote service messages must be objects"))?;
        let role = message_object
            .get("role")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("remote service message role must be a non-empty string")
            })?;
        if !matches!(role, "system" | "user" | "assistant") {
            return Ok(governed_failure(
                "invalid-message",
                format!("remote service message role {role:?} is not supported"),
            ));
        }
        let content = message_object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("remote service message content must be a string"))?;
        if content.len() > MAX_REQUEST_BYTES {
            return Ok(governed_failure(
                "request-budget-exceeded",
                format!(
                    "remote service message content exceeded the aggregate budget of {MAX_REQUEST_BYTES} bytes"
                ),
            ));
        }
    }
    // ---- Operator configuration must be valid before ANY other work.
    // An invalid operator ceiling fails closed here — before argument
    // interpretation, before any preflight, before any network effect.
    if config.timeout_ms == 0 {
        return Ok(governed_failure(
            "invalid-config",
            "operator timeout ceiling is not a positive integer; refusing",
        ));
    }
    if config.max_output_tokens == 0 {
        return Ok(governed_failure(
            "invalid-config",
            "operator token ceiling is not a positive integer; refusing",
        ));
    }
    // ---- Governed preflight: no network effect before this point. ------
    // Everything below is pure validation; the only network effect is the
    // bounded, cancellable resolution+request lifecycle that follows it.
    let timeout_ms = match obj.get("timeoutMs") {
        None => config.timeout_ms,
        Some(value) => match value.as_u64() {
            Some(t) if (1..=config.timeout_ms).contains(&t) => t,
            _ => {
                return Ok(governed_failure(
                    "invalid-timeout",
                    "timeoutMs must be a positive integer no greater than the operator-configured ceiling",
                ));
            }
        },
    };
    // ---- Governed token budget: exactly the operator ceiling, never more.
    let max_tokens = match obj.get("maxTokens") {
        None => None,
        Some(value) => match value.as_u64() {
            Some(t) if (1..=config.max_output_tokens).contains(&t) => Some(t),
            _ => {
                return Ok(governed_failure(
                    "invalid-token-budget",
                    "maxTokens must be a positive integer no greater than the operator-configured ceiling",
                ));
            }
        },
    };
    // ---- Structured-output schema: exactly one accepted value, fail closed.
    let json_mode = match obj.get("responseFormat") {
        None => false,
        Some(Value::String(format)) if format == "json_object" => true,
        Some(_) => {
            return Ok(governed_failure(
                "invalid-response-format",
                "responseFormat must be exactly \"json_object\" when supplied",
            ));
        }
    };
    // ---- Bounded correlation/request identifiers. ----------------------
    for (key, value) in obj {
        if matches!(key.as_str(), "requestId" | "correlationId" | "stepId") {
            match value {
                Value::String(s) if !s.is_empty() && s.chars().count() <= 512 => {}
                _ => {
                    return Ok(governed_failure(
                        "invalid-identifier",
                        format!("{key} must be a non-empty string of at most 512 characters"),
                    ));
                }
            }
        }
    }
    // ---- Bounded known secrets. ----------------------------------------
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
    let mut redactor_secrets = known_secrets;
    redactor_secrets.extend(config.known_secrets.clone());
    if !config.api_key.is_empty() {
        redactor_secrets.push(config.api_key.clone());
    }
    let redactor = Redactor::new().with_known_secrets(&redactor_secrets);

    // ---- Deadline starts here; DNS is inside it -----------------------
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);

    // ---- Governed preflight (still no network effect) -----------------
    let endpoint = match parse_endpoint(&config.endpoint, config.allow_private_destinations) {
        Ok(endpoint) => endpoint,
        Err(failure) => return Ok(governed_failure(&failure.status, failure.message)),
    };
    if let Err(failure) = validate_allowlist(&endpoint, &config.allowed_hosts) {
        return Ok(governed_failure(&failure.status, failure.message));
    }

    // ---- Wire request (OpenAI-compatible chat completions) --------------
    let mut request_body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });
    if let Some(mt) = max_tokens {
        request_body["max_tokens"] = json!(mt);
    }
    if json_mode {
        request_body["response_format"] = json!({"type": "json_object"});
    }
    let body_bytes =
        serde_json::to_vec(&request_body).context("remote service request body serializes")?;
    if body_bytes.len() > MAX_REQUEST_BYTES {
        return Ok(governed_failure(
            "request-budget-exceeded",
            format!(
                "remote service request body exceeded the aggregate budget of {MAX_REQUEST_BYTES} bytes"
            ),
        ));
    }

    let url = {
        let authority = match endpoint.host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(_)) => format!("[{}]", endpoint.host),
            _ => endpoint.host.clone(),
        };
        let port_suffix = if endpoint.port == 443 {
            String::new()
        } else {
            format!(":{}", endpoint.port)
        };
        // base_path was validated by parse_endpoint: absolute, no dot
        // segments, no unsafe characters. The request path is
        // base_path + /chat/completions; no caller input reaches it.
        format!(
            "https://{authority}{port_suffix}{}/chat/completions",
            endpoint.base_path
        )
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

    // The effective timeout (task may only narrow the operator ceiling)
    // governs the whole lifecycle, including DNS.
    let mut effective_config = config.clone();
    effective_config.timeout_ms = timeout_ms;
    let cancel = config.cancel.clone();
    let api_key = config.api_key.clone();
    let max_output_bytes = config.max_output_bytes;
    let latency_ms;
    let select_outcome = blocking_roundtrip(
        endpoint,
        effective_config,
        url,
        body_bytes,
        api_key,
        max_output_bytes,
        deadline,
        cancel,
    );
    match select_outcome {
        TransportOutcome::Cancelled => Ok(governed_failure(
            "cancelled",
            "remote service inference cancelled",
        )),
        TransportOutcome::TimedOut => Ok(governed_failure(
            "timeout",
            "remote service request timed out before completion",
        )),
        TransportOutcome::Result(Err(failure)) => {
            Ok(governed_failure(&failure.status, failure.message))
        }
        TransportOutcome::Result(Ok((status, body))) => {
            latency_ms = started.elapsed().as_millis() as u64;
            if !(200..300).contains(&status) {
                return Ok(governed_failure(
                    "provider-error",
                    format!("remote service provider returned HTTP {status}"),
                ));
            }
            match build_remote_service_success(
                &body,
                &model,
                json_mode,
                latency_ms,
                &request_id,
                &correlation_id,
                &redactor,
            ) {
                Ok(output) => {
                    let mut output = output;
                    redact_json_value(&mut output, &redactor);
                    let text = governed_success(output, latency_ms, &request_id, &correlation_id);
                    if redactor.redact(&text) != text {
                        return Ok(governed_failure(
                            "secret-bearing-response",
                            "remote service result still contains a known secret after redaction; refusing to persist",
                        ));
                    }
                    Ok(text)
                }
                Err(failure) => Ok(governed_failure(&failure.status, failure.message)),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the pure destination-policy functions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod destination_policy_tests {
    use super::*;

    fn public_ok(ip: &str) -> bool {
        !is_non_public_address(&ip.parse().expect("ip parses"))
    }

    #[test]
    fn destination_policy_table() {
        // Globally routable, non-special.
        assert!(public_ok("1.1.1.1"));
        assert!(public_ok("8.8.8.8"));
        assert!(public_ok("93.184.216.34"));
        assert!(public_ok("2606:4700:4700::1111"));
        // Loopback / unspecified / broadcast.
        assert!(is_non_public_address(&"127.0.0.1".parse().unwrap()));
        assert!(is_non_public_address(&"127.1.2.3".parse().unwrap()));
        assert!(is_non_public_address(&"0.0.0.0".parse().unwrap()));
        assert!(is_non_public_address(&"255.255.255.255".parse().unwrap()));
        assert!(is_non_public_address(&"::1".parse().unwrap()));
        assert!(is_non_public_address(&"::".parse().unwrap()));
        // Private / CGNAT / link-local.
        assert!(is_non_public_address(&"10.1.2.3".parse().unwrap()));
        assert!(is_non_public_address(&"172.16.0.1".parse().unwrap()));
        assert!(is_non_public_address(&"172.31.255.255".parse().unwrap()));
        assert!(is_non_public_address(&"192.168.1.1".parse().unwrap()));
        assert!(is_non_public_address(&"100.64.0.1".parse().unwrap()));
        assert!(is_non_public_address(&"100.127.255.255".parse().unwrap()));
        assert!(is_non_public_address(&"169.254.0.1".parse().unwrap()));
        // Documentation / benchmarking / reserved.
        assert!(is_non_public_address(&"192.0.2.1".parse().unwrap()));
        assert!(is_non_public_address(&"198.51.100.1".parse().unwrap()));
        assert!(is_non_public_address(&"203.0.113.1".parse().unwrap()));
        assert!(is_non_public_address(&"198.18.0.1".parse().unwrap()));
        assert!(is_non_public_address(&"198.19.255.255".parse().unwrap()));
        assert!(is_non_public_address(&"240.1.2.3".parse().unwrap()));
        // IPv4-mapped IPv6 classifies the embedded address.
        assert!(is_non_public_address(&"::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_non_public_address(&"::ffff:10.0.0.1".parse().unwrap()));
        assert!(public_ok("::ffff:8.8.8.8"));
        // NAT64 / 6to4 / Teredo / discard — refused outright.
        assert!(is_non_public_address(&"64:ff9b::1.2.3.4".parse().unwrap()));
        assert!(is_non_public_address(&"2002:0808:0808::".parse().unwrap()));
        assert!(is_non_public_address(&"2001:0::1".parse().unwrap()));
        assert!(is_non_public_address(&"100::1".parse().unwrap()));
        // ULA / link-local / multicast / documentation.
        assert!(is_non_public_address(&"fc00::1".parse().unwrap()));
        assert!(is_non_public_address(&"fd12::1".parse().unwrap()));
        assert!(is_non_public_address(&"fe80::1".parse().unwrap()));
        assert!(is_non_public_address(&"febf::1".parse().unwrap()));
        assert!(is_non_public_address(&"ff02::1".parse().unwrap()));
        assert!(is_non_public_address(&"2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn metadata_destinations_are_always_refused() {
        assert!(is_metadata_address(&"169.254.169.254".parse().unwrap()));
        assert!(is_metadata_address(&"169.254.170.2".parse().unwrap()));
        assert!(is_metadata_address(&"100.100.100.200".parse().unwrap()));
        assert!(is_metadata_address(&"fd00:ec2::254".parse().unwrap()));
        assert!(!is_metadata_address(&"169.254.0.5".parse().unwrap()));
        assert!(!is_metadata_address(&"1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn host_port_split_table() {
        assert_eq!(
            split_host_port("api.example.com", 443),
            Some(("api.example.com".to_string(), 443))
        );
        assert_eq!(
            split_host_port("API.Example.COM", 443),
            Some(("api.example.com".to_string(), 443))
        );
        assert_eq!(
            split_host_port("api.example.com:8443", 443),
            Some(("api.example.com".to_string(), 8443))
        );
        assert_eq!(
            split_host_port("127.0.0.1", 443),
            Some(("127.0.0.1".to_string(), 443))
        );
        assert_eq!(
            split_host_port("[::1]", 443),
            Some(("::1".to_string(), 443))
        );
        assert_eq!(
            split_host_port("[::1]:8443", 443),
            Some(("::1".to_string(), 8443))
        );
        // Bare IPv6 is ambiguous with the port separator — rejected.
        assert_eq!(split_host_port("::1", 443), None);
        // Garbage.
        assert_eq!(split_host_port("", 443), None);
        assert_eq!(split_host_port(":443", 443), None);
        assert_eq!(split_host_port("api.example.com:", 443), None);
        assert_eq!(split_host_port("api.example.com:http", 443), None);
        assert_eq!(split_host_port("[::1]x", 443), None);
        assert_eq!(split_host_port("[::1", 443), None);
    }

    #[test]
    fn endpoint_validation_table() {
        // Valid HTTPS origins.
        assert!(parse_endpoint("https://api.example.com", false).is_ok());
        assert!(parse_endpoint("https://api.example.com/v1", false).is_ok());
        assert!(parse_endpoint("https://api.example.com/v1/", false).is_ok());
        assert!(parse_endpoint("https://api.example.com:8443/v1", false).is_ok());
        assert!(parse_endpoint("https://[2606:4700:4700::1111]/v1", false).is_ok());
        // Scheme refusal.
        assert_eq!(
            parse_endpoint("http://api.example.com/v1", false)
                .unwrap_err()
                .status,
            "unsupported-scheme"
        );
        assert_eq!(
            parse_endpoint("ws://api.example.com/v1", false)
                .unwrap_err()
                .status,
            "unsupported-scheme"
        );
        // Credentials / query / fragment.
        assert_eq!(
            parse_endpoint("https://user:pass@api.example.com/v1", false)
                .unwrap_err()
                .status,
            "invalid-endpoint"
        );
        assert_eq!(
            parse_endpoint("https://api.example.com/v1?a=b", false)
                .unwrap_err()
                .status,
            "invalid-endpoint"
        );
        assert_eq!(
            parse_endpoint("https://api.example.com/v1#frag", false)
                .unwrap_err()
                .status,
            "invalid-endpoint"
        );
        // IP-literal destinations in the strict policy.
        assert_eq!(
            parse_endpoint("https://127.0.0.1/v1", false)
                .unwrap_err()
                .status,
            "destination-refused"
        );
        assert_eq!(
            parse_endpoint("https://10.0.0.1/v1", false)
                .unwrap_err()
                .status,
            "destination-refused"
        );
        assert_eq!(
            parse_endpoint("https://169.254.169.254/latest", false)
                .unwrap_err()
                .status,
            "destination-refused"
        );
        assert_eq!(
            parse_endpoint("https://[::1]/v1", false)
                .unwrap_err()
                .status,
            "destination-refused"
        );
        // DNS names pass syntactically; resolution is a separate step.
        assert!(parse_endpoint("https://api.example.com", false).is_ok());
        // Trailing dot, percent, empty label, leading/trailing hyphen.
        assert!(parse_endpoint("https://api.example.com./v1", false).is_err());
        assert!(parse_endpoint("https://a%20b.example.com/v1", false).is_err());
        assert!(parse_endpoint("https://-api.example.com/v1", false).is_err());
        assert!(parse_endpoint("https://api-.example.com/v1", false).is_err());
        assert!(parse_endpoint("https://api..example.com/v1", false).is_err());
        // Path traversal and unsafe segments.
        assert!(parse_endpoint("https://api.example.com/../admin", false).is_err());
        assert!(parse_endpoint("https://api.example.com/a/./b", false).is_err());
        assert!(parse_endpoint("https://api.example.com/%2e%2e/b", false).is_err());
        // Port edge cases.
        assert!(parse_endpoint("https://api.example.com:0/v1", false).is_err());
        assert!(parse_endpoint("https://api.example.com:99999/v1", false).is_err());
    }

    #[test]
    fn allowlist_exact_match_only() {
        let endpoint = ValidatedEndpoint {
            host: "api.example.com".to_string(),
            port: 443,
            base_path: "/v1".to_string(),
        };
        assert!(validate_allowlist(&endpoint, &["api.example.com".to_string()]).is_ok());
        assert!(validate_allowlist(&endpoint, &["API.EXAMPLE.COM".to_string()]).is_ok());
        assert!(validate_allowlist(&endpoint, &["*".to_string()]).is_err());
        assert!(validate_allowlist(&endpoint, &["api.example.com.evil.com".to_string()]).is_err());
        assert!(validate_allowlist(&endpoint, &["example.com".to_string()]).is_err());
        assert!(validate_allowlist(&endpoint, &[]).is_err());

        let endpoint8443 = ValidatedEndpoint {
            host: "api.example.com".to_string(),
            port: 8443,
            base_path: "/v1".to_string(),
        };
        // A host entry without a port only matches the default 443.
        assert!(validate_allowlist(&endpoint8443, &["api.example.com".to_string()]).is_err());
        assert!(validate_allowlist(&endpoint8443, &["api.example.com:8443".to_string()]).is_ok());
        assert!(validate_allowlist(&endpoint8443, &["api.example.com:443".to_string()]).is_err());
    }
}

#[cfg(test)]
mod resolution_lifecycle_tests {
    use super::*;

    struct FakeResolver {
        addrs: Option<Vec<SocketAddr>>,
        delay: std::time::Duration,
    }

    impl AddressResolver for FakeResolver {
        fn resolve(&self, _host: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
            if !self.delay.is_zero() {
                std::thread::sleep(self.delay);
            }
            self.addrs
                .clone()
                .ok_or_else(|| std::io::Error::other("resolver error"))
        }
    }

    fn resolve_with(
        addrs: Option<Vec<SocketAddr>>,
        delay: std::time::Duration,
        budget: std::time::Duration,
        allow_private: bool,
    ) -> std::result::Result<Vec<SocketAddr>, RemoteServiceFailure> {
        let cancel = AtomicBool::new(false);
        resolve_with_budget(
            Box::new(FakeResolver { addrs, delay }),
            "host.internal",
            443,
            budget,
            &cancel,
            allow_private,
        )
    }

    #[test]
    fn resolution_budget_covers_full_answer_set() {
        let public = || SocketAddr::new("1.1.1.1".parse().unwrap(), 443);
        let ok = resolve_with(
            Some(vec![public(), public()]),
            Duration::ZERO,
            Duration::from_secs(1),
            false,
        );
        assert!(ok.is_ok());
        // Mixed public/private: refused outright.
        let mixed = resolve_with(
            Some(vec![
                public(),
                SocketAddr::new("10.0.0.8".parse().unwrap(), 443),
            ]),
            Duration::ZERO,
            Duration::from_secs(1),
            false,
        );
        assert_eq!(mixed.unwrap_err().status, "destination-refused");
        // Metadata answer under the private-destination policy: still refused.
        let metadata = resolve_with(
            Some(vec![SocketAddr::new(
                "169.254.169.254".parse().unwrap(),
                80,
            )]),
            Duration::ZERO,
            Duration::from_secs(1),
            true,
        );
        assert_eq!(metadata.unwrap_err().status, "destination-refused");
        // Resolver error / empty answer: refused.
        let err = resolve_with(None, Duration::ZERO, Duration::from_secs(1), false);
        assert_eq!(err.unwrap_err().status, "dns-refused");
        let empty = resolve_with(Some(vec![]), Duration::ZERO, Duration::from_secs(1), false);
        assert_eq!(empty.unwrap_err().status, "dns-refused");
        // Private answer accepted only under the explicit operator policy.
        let private_ok = resolve_with(
            Some(vec![SocketAddr::new("192.168.1.9".parse().unwrap(), 8080)]),
            Duration::ZERO,
            Duration::from_secs(1),
            true,
        );
        assert!(private_ok.is_ok());
    }

    #[test]
    fn resolution_timeout_and_cancellation() {
        // A resolver stuck beyond the budget is abandoned and refused, not
        // waited on indefinitely.
        let started = std::time::Instant::now();
        let timed = resolve_with(
            Some(vec![SocketAddr::new("1.1.1.1".parse().unwrap(), 443)]),
            Duration::from_secs(30),
            Duration::from_millis(150),
            false,
        );
        let elapsed = started.elapsed();
        assert_eq!(timed.unwrap_err().status, "timeout");
        assert!(
            elapsed < Duration::from_secs(5),
            "budget bounds the wait ({elapsed:?})"
        );

        // Cancellation interrupts the wait without touching the resolver.
        let cancel = AtomicBool::new(true);
        let cancelled = resolve_with_budget(
            Box::new(FakeResolver {
                addrs: Some(vec![SocketAddr::new("1.1.1.1".parse().unwrap(), 443)]),
                delay: Duration::from_secs(30),
            }),
            "host.internal",
            443,
            Duration::from_secs(30),
            &cancel,
            false,
        );
        assert_eq!(cancelled.unwrap_err().status, "cancelled");
    }
}
