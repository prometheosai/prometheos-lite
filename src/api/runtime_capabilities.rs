//! #132 Slice 3: runtime capability negotiation endpoints (SPEC 007).
//!
//! - `GET /runtime/capabilities` — Lite's own `RuntimeCapabilitySet`,
//!   built ONCE per process instance during `AppState` construction from
//!   the pinned runtime-truth table below, frozen as canonical bytes.
//!   A declaration is capability METADATA, never authority.
//! - `POST /runtime/compatibility/simulate` — an explicitly bounded
//!   ADVISORY SIMULATION: it answers "would these requirements be
//!   compatible with this runtime's capabilities IF executed under the
//!   caller-stated authority pair?" Nothing is persisted, dispatched,
//!   or granted; the decision embeds everything it was computed from.
//!   The handler owns the RAW request (no body extractors, no
//!   body-limit middleware) so every response it generates — 200, 400,
//!   413, 500 — carries `X-Advisory-Simulation: true`.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::state::AppState;
use crate::api::work_contexts::{ApiError, if_none_match_matches, representation_etag};
use crate::workflow::soma::canonical::{sha256_hex, try_canonical_bytes, try_canonical_digest};
use crate::workflow::soma::capability::{
    AuthorityPair, CompatibilityDecision, NegotiationInputs, RuntimeCapabilitySet, Substitution,
    WorkRequirements, resolve,
};

/// The maximum accepted simulation request body (the handler-enforced
/// bound; `DefaultBodyLimit` is DISABLED on the route so this is the
/// sole size gate and every size rejection is handler-generated).
pub const SIMULATION_BODY_LIMIT_BYTES: usize = 256 * 1024;

/// The per-collection bound for the unauthenticated compute surface.
pub const SIMULATION_COLLECTION_LIMIT: usize = 64;

/// The immutable, instance-scoped declaration: the typed set plus its
/// frozen canonical bytes, content digest, and complete-representation
/// ETag. Built during `AppState::new`; stable for the instance lifetime.
pub struct RuntimeCapabilityDeclaration {
    pub set: RuntimeCapabilitySet,
    pub canonical_bytes: Vec<u8>,
    pub digest: String,
    pub etag: String,
}

impl RuntimeCapabilityDeclaration {
    /// Fallible construction from a given set: the declaration must
    /// audit CLEAN (structural validity) and canonicalize — any failure
    /// is an error that prevents application startup (never a panic,
    /// never a partially initialized state).
    pub fn build_with(set: RuntimeCapabilitySet) -> Result<Self, String> {
        let diags = set.audit();
        if !diags.is_empty() {
            return Err(format!(
                "runtime capability declaration is structurally invalid: {:?}",
                diags
            ));
        }
        let value =
            serde_json::to_value(&set).map_err(|e| format!("declaration serialization: {e}"))?;
        let canonical_bytes = try_canonical_bytes(&value)
            .map_err(|e| format!("declaration cannot be canonicalized: {e:?}"))?;
        let digest = try_canonical_digest(&value)
            .map_err(|e| format!("declaration digest unavailable: {e:?}"))?;
        // The complete-representation validator (the Slice 2 formula,
        // non-paged: the canonical digest of {"body": <body sha>}).
        let binding = serde_json::json!({ "body": sha256_hex(&canonical_bytes) });
        let etag = format!(
            "\"{}\"",
            try_canonical_digest(&binding).map_err(|e| format!("etag unavailable: {e:?}"))?
        );
        Ok(Self {
            set,
            canonical_bytes,
            digest,
            etag,
        })
    }

    /// Build Lite's declaration from the pinned runtime-truth table.
    /// Every claimed field carries its SPEC 007 meaning plus a concrete
    /// production path (asserted by the honesty tests); everything else
    /// is an honest OMISSION — never invented:
    ///
    /// - `identity` — the crate identity the provenance implementation
    ///   already records (`prometheos-lite` + `CARGO_PKG_VERSION`).
    /// - `specVersions` — the bundles Lite actually consumes (vendored
    ///   v1.1 for the artifact families, v1.2 for the capability
    ///   families).
    /// - `checkpointImport`/`checkpointExport` — the digest-verified
    ///   `GraphRunStateV1::import_checkpoint`/`export_checkpoint`
    ///   production paths behind the registered graph-run endpoints.
    /// - `eventStreaming: false` — durable journaling + cursorable paged
    ///   observation is NOT streaming; nothing adjacent may launder the
    ///   claim.
    /// - `readProjection: true` — the Slice 1B/2 projection + observation
    ///   surface (`soma_projection::project_page`, the /work-events
    ///   route family).
    /// - `locality: local` — Lite's local execution paths.
    /// - `evidenceAttachment` — the harness evidence production paths.
    /// - `validation` — the node validation gates.
    ///
    /// Omitted (no demonstrated production path; the omission-first
    /// rule): `resumeSupport` (no checkpoint-restore-to-execution path),
    /// `adapterFamilies`, `effectClasses` (no closed enforced vocabulary
    /// is enumerated), `workspace` (per-dimension claims need named
    /// paths), `privacyClasses`, `maxStructuredInput/Output`,
    /// `budgetCeilings` (no runtime-wide config hard ceiling),
    /// `approvalEscalation`.
    pub fn build(declared_at: String) -> Result<Self, String> {
        let set = RuntimeCapabilitySet {
            id: "prometheos-lite".to_string(),
            schema_version: crate::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION
                .to_string(),
            version: crate::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION
                .to_string(),
            identity: crate::workflow::soma::capability::Identity {
                name: "prometheos-lite".to_string(),
                revision: env!("CARGO_PKG_VERSION").to_string(),
            },
            declared_at,
            spec_versions: vec![
                crate::workflow::soma::SUPPORTED_SCHEMA_VERSION.to_string(),
                crate::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            ],
            adapter_families: Vec::new(),
            effect_classes: Vec::new(),
            workspace: None,
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: None,
            event_streaming: Some(false),
            read_projection: Some(true),
            locality: Some(crate::workflow::soma::capability::Locality::Local),
            privacy_classes: Vec::new(),
            max_structured_input: None,
            max_structured_output: None,
            budget_ceilings: None,
            evidence_attachment: Some(true),
            validation: Some(true),
            approval_escalation: None,
        };
        Self::build_with(set)
    }
}

/// The advisory responder: EVERY response this route generates — 200
/// decisions, 400, 413, and 500 — flows through here, so
/// `X-Advisory-Simulation: true` is structural.
fn advisory_response(
    status: StatusCode,
    etag: Option<String>,
    body: Vec<u8>,
) -> Result<axum::response::Response, axum::http::Error> {
    let mut builder = axum::http::Response::builder()
        .status(status)
        .header("x-advisory-simulation", "true")
        .header(axum::http::header::CONTENT_TYPE, "application/json");
    if let Some(etag) = etag {
        builder = builder.header(axum::http::header::ETAG, etag);
    }
    builder.body(Body::from(body))
}

fn advisory_error(status: StatusCode, message: &str) -> axum::response::Response {
    advisory_response(
        status,
        None,
        serde_json::json!({ "error": message })
            .to_string()
            .into_bytes(),
    )
    .expect("static response builds")
}

/// `GET /runtime/capabilities` — Lite's frozen declaration. Runtime-level
/// metadata (the `/health` and `/runtime/stack` family): no per-user
/// data, no ownership scope.
pub async fn get_runtime_capabilities(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    let declaration = &state.runtime_capabilities;
    if let Some(header_value) = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        && if_none_match_matches(header_value, &declaration.etag)
    {
        return axum::http::Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(axum::http::header::ETAG, declaration.etag.clone())
            .body(Body::empty())
            .map_err(|e| ApiError::Internal(format!("response build failed: {e}")));
    }
    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .header(axum::http::header::ETAG, declaration.etag.clone())
        .body(Body::from(declaration.canonical_bytes.clone()))
        .map_err(|e| ApiError::Internal(format!("response build failed: {e}")))
}

/// The simulation request: caller-supplied (explicitly hypothetical)
/// requirements + authority pair, and optional proposed substitutions.
/// Structurally strict (`deny_unknown_fields` all the way down);
/// serializable so the recursive bound walker can inspect every
/// caller-controlled collection.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRequest {
    pub requirements: WorkRequirements,
    pub authority: AuthorityPair,
    #[serde(default)]
    pub substitutions: Vec<Substitution>,
}

fn check_collection_bounds(req: &SimulationRequest) -> Result<(), String> {
    // The bound is RECURSIVE: every caller-controlled collection in the
    // request — arrays AND object key sets, at any depth — is limited to
    // SIMULATION_COLLECTION_LIMIT entries. This covers the top-level
    // arrays AND the nested ones the caller controls inside freshness
    // (trustedSources), locality (allowedExecution, privacy), both
    // authority profiles (readableScopes, writableScopes, tools maps,
    // network/provider allowlists, secrets scopes, contentRestrictions)
    // — anything the strict parse accepts.
    let value = serde_json::to_value(req)
        .map_err(|e| format!("request cannot be inspected for bounds: {e}"))?;
    match find_oversized_collection(&value, "$") {
        Some(path) => Err(format!(
            "{path} exceeds the simulation collection bound of {SIMULATION_COLLECTION_LIMIT}"
        )),
        None => Ok(()),
    }
}

/// Recursively locate the first caller-controlled collection exceeding
/// the bound, reporting its path for the 400 response.
fn find_oversized_collection(value: &serde_json::Value, path: &str) -> Option<String> {
    match value {
        serde_json::Value::Array(items) => {
            if items.len() > SIMULATION_COLLECTION_LIMIT {
                return Some(format!("{path} ({}) entries", items.len()));
            }
            items
                .iter()
                .enumerate()
                .find_map(|(i, item)| find_oversized_collection(item, &format!("{path}[{i}]")))
        }
        serde_json::Value::Object(map) => {
            if map.len() > SIMULATION_COLLECTION_LIMIT {
                return Some(format!("{path} ({}) keys", map.len()));
            }
            map.iter()
                .find_map(|(key, child)| find_oversized_collection(child, &format!("{path}.{key}")))
        }
        _ => None,
    }
}

/// `POST /runtime/compatibility/simulate` — the bounded advisory
/// simulation. The handler owns the RAW request: no body extractors and
/// no body-limit middleware precede it (`DefaultBodyLimit::disable()` is
/// applied on the route), so the 256 KiB cap below is the sole size
/// gate and its 413 is handler-generated.
pub async fn simulate_compatibility(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
) -> axum::response::Response {
    // Split inside the handler; no extractors consumed the body first.
    let (_parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, SIMULATION_BODY_LIMIT_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return advisory_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("simulation body exceeds the {SIMULATION_BODY_LIMIT_BYTES}-byte bound"),
            );
        }
    };

    let parsed: SimulationRequest = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(e) => {
            return advisory_error(
                StatusCode::BAD_REQUEST,
                &format!("simulation request is structurally invalid: {e}"),
            );
        }
    };
    if let Err(message) = check_collection_bounds(&parsed) {
        return advisory_error(StatusCode::BAD_REQUEST, &message);
    }

    // The server injects the evaluation instant (RFC 3339 UTC-Z, the
    // schema-enforced lexicographic clock domain).
    let evaluated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let decision = resolve(&NegotiationInputs {
        id: uuid::Uuid::new_v4().to_string(),
        requirements: &parsed.requirements,
        capabilities: &state.runtime_capabilities.set,
        authority_declared: &parsed.authority.declared,
        authority_effective: &parsed.authority.effective,
        evaluated_at,
        substitutions: parsed.substitutions.clone(),
    });
    let decision: CompatibilityDecision = match decision {
        Ok(decision) => decision,
        Err(e) => {
            return advisory_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("negotiation resolution failed: {e}"),
            );
        }
    };

    let value = match serde_json::to_value(&decision) {
        Ok(value) => value,
        Err(e) => {
            return advisory_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("decision serialization failed: {e}"),
            );
        }
    };
    let canonical = match try_canonical_bytes(&value) {
        Ok(canonical) => canonical,
        Err(e) => {
            return advisory_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("decision cannot be canonicalized: {e:?}"),
            );
        }
    };
    let etag = match representation_etag(&canonical, None) {
        Ok(etag) => etag,
        Err(_) => {
            return advisory_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "decision etag unavailable",
            );
        }
    };
    match advisory_response(StatusCode::OK, Some(etag), canonical) {
        Ok(response) => response,
        Err(e) => advisory_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("response build failed: {e}"),
        ),
    }
}

/// The route wiring for the capability family (the router calls this so
/// the simulate route's `DefaultBodyLimit::disable()` stays colocated
/// with the handler that owns the size gate).
pub fn routes() -> axum::Router<Arc<AppState>> {
    use axum::routing::get;
    axum::Router::new()
        .route("/runtime/capabilities", get(get_runtime_capabilities))
        .route(
            "/runtime/compatibility/simulate",
            axum::routing::post(simulate_compatibility),
        )
        // The simulate handler enforces its own 256 KiB bound and
        // generates every rejection itself; no global body limit may
        // preempt it (axum's default is 2 MiB).
        .layer(DefaultBodyLimit::disable())
}
