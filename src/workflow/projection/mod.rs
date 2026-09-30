//! Deterministic, read-only projections of the canonical SOMA++ AST.
//!
//! Slice 1 ships the versioned envelope and the canonical JSON view. The
//! human plan, verify path, and disclosure policy are added by the tasks
//! that own them.

pub mod envelope;

pub use envelope::{
    ALLOWED_PROJECTION_VERSIONS, PROJECTION_VERSION_V1, VersionedProjectionEnvelope,
};

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
