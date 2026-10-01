//! Deterministic, read-only projections of the canonical SOMA++ AST.
//!
//! Slice 1 ships the versioned projection envelope, the canonical JSON
//! project + verify path, the non-normative human plan project + verify
//! path, and the redaction/disclosure policy applied to human plans.

pub mod envelope;
pub mod human;
pub mod redaction;

pub use envelope::{
    ALLOWED_PROJECTION_VERSIONS, PROJECTION_VERSION_V1, VersionedProjectionEnvelope,
};
pub use human::project_human_plan;
pub use redaction::RedactionPolicy;

use crate::workflow::soma::canonical::try_canonical_digest;
use crate::workflow::soma::contracts::WorkflowDefinition;
use crate::workflow::soma::{Diagnostic, supported_version};

/// Fail-closed input gate: only an audit-clean, schema-compatible canonical
/// AST may be projected. Returns the audit's own diagnostics on refusal.
pub fn validated_source(wf: &WorkflowDefinition) -> Result<(), Vec<Diagnostic>> {
    let supported = supported_version();
    let diags = wf.audit(&supported);
    if !diags.is_empty() {
        return Err(diags);
    }
    match crate::workflow::soma::types::SemVer::parse(&wf.schema_version) {
        Ok(v) if v == supported => Ok(()),
        _ => Err(vec![Diagnostic::new(
            "PROJ-0001",
            format!(
                "workflow schemaVersion {} is not the supported SOMA schema",
                wf.schema_version
            ),
        )]),
    }
}

/// Canonical digest of a projection value under the pinned SOMA DecimalV2
/// policy; failures map to SOMA-CMP-0004.
fn digest_of(value: &serde_json::Value) -> Result<String, Vec<Diagnostic>> {
    try_canonical_digest(value).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("projection digest cannot be computed ({e})"),
        )]
    })
}

/// Byte-deterministic canonical JSON projection of the validated AST.
/// Never redacted: it is a semantic artifact.
pub fn project_canonical_json(
    wf: &WorkflowDefinition,
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>> {
    validated_source(wf)?;
    let payload = serde_json::to_value(wf).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("workflow cannot be serialized for projection ({e})"),
        )]
    })?;
    let mut source_value = payload.clone();
    if let Some(obj) = source_value.as_object_mut() {
        obj.remove("contentDigest");
    }
    // Same rule as governance_compiler::workflow_digest_of (minus
    // `contentDigest`); equality with that function is asserted by
    // `source_digest_matches_workflow_digest_of`.
    let source_digest = digest_of(&source_value)?;
    let projection_digest = digest_of(&payload)?;
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: wf.schema_version.clone(),
        source_digest,
        projection_digest,
        payload,
    })
}

/// Fail-closed read/verify path for canonical projection bytes.
pub fn verify_canonical_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<serde_json::Value>, Vec<Diagnostic>> {
    let env = envelope::parse_envelope_bytes::<serde_json::Value>(raw)?;
    let expected = digest_of(&env.payload)?;
    if expected != env.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "canonical projection digest does not verify",
        )]);
    }
    Ok(env)
}

/// Identity layer: the envelope must match the source AST it claims.
/// Self-contained: envelope metadata (`projectionVersion`, `schemaVersion`,
/// digest shapes) and the payload digest are validated here directly, so a
/// forged envelope fails closed without a prior byte-verification call.
pub fn verify_projection_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    wf: &WorkflowDefinition,
) -> Result<(), Vec<Diagnostic>> {
    validated_source(wf)?;
    let source_value = serde_json::to_value(wf).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("workflow cannot be serialized for verification ({e})"),
        )]
    })?;
    envelope::verify_envelope_metadata(envelope)?;
    let expected_payload = digest_of(&envelope.payload)?;
    if expected_payload != envelope.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "canonical projection digest does not verify",
        )]);
    }
    if envelope.schema_version != wf.schema_version {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "schemaVersion does not match the source AST",
        )]);
    }
    let mut digest_value = source_value.clone();
    if let Some(obj) = digest_value.as_object_mut() {
        obj.remove("contentDigest");
    }
    let expected_source = digest_of(&digest_value)?;
    if expected_source != envelope.source_digest {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "sourceDigest does not match the source AST",
        )]);
    }
    if source_value != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "projection payload does not match the source AST",
        )]);
    }
    Ok(())
}

/// Fail-closed read/verify path for human plan projection bytes.
pub fn verify_human_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<String>, Vec<Diagnostic>> {
    let env = envelope::parse_envelope_bytes::<String>(raw)?;
    let expected = crate::workflow::soma::canonical::sha256_hex(env.payload.as_bytes());
    if expected != env.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "human projection digest does not verify",
        )]);
    }
    Ok(env)
}

/// Identity layer for human plans: digest + payload must equal a fresh
/// deterministic render from the source AST under the same policy.
/// Self-contained: envelope metadata and the payload digest are validated
/// here directly, so a forged envelope fails closed without a prior
/// byte-verification call.
pub fn verify_human_against_source(
    envelope: &VersionedProjectionEnvelope<String>,
    wf: &WorkflowDefinition,
    policy: Option<&RedactionPolicy>,
) -> Result<(), Vec<Diagnostic>> {
    let fresh = human::project_human_plan(wf, policy.cloned())?;
    envelope::verify_envelope_metadata(envelope)?;
    let expected = crate::workflow::soma::canonical::sha256_hex(envelope.payload.as_bytes());
    if expected != envelope.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "human projection digest does not verify",
        )]);
    }
    if envelope.schema_version != wf.schema_version {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "schemaVersion does not match the source AST",
        )]);
    }
    if fresh.source_digest != envelope.source_digest || fresh.payload != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "human projection does not match a fresh render of the source AST",
        )]);
    }
    Ok(())
}
