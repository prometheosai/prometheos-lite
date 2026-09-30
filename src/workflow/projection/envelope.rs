//! Versioned projection envelope: the stable identity wrapper every Lite
//! projection is published inside. Nested payload — never flattened.

use serde::Serialize;

use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::canonical::try_canonical_bytes;

/// The only projection format version Slice 1 emits or accepts.
pub const PROJECTION_VERSION_V1: &str = "projection.v1";

/// Closed allow-list of projection versions. Fail closed on anything else.
pub const ALLOWED_PROJECTION_VERSIONS: [&str; 1] = [PROJECTION_VERSION_V1];

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionedProjectionEnvelope<V> {
    /// Projection format version (`projection.v1`).
    pub projection_version: String,
    /// SOMA schema version of the source workflow.
    pub schema_version: String,
    /// Canonical digest of the source AST (`contentDigest` stripped).
    pub source_digest: String,
    /// Canonical digest of the serialized projection payload.
    pub projection_digest: String,
    /// Projection payload — nested, never `#[serde(flatten)]`.
    pub payload: V,
}

impl<V: Serialize> VersionedProjectionEnvelope<V> {
    /// Deterministic wire bytes: the envelope serialized through the SOMA
    /// canonical renderer (lexicographic keys, no whitespace, decimal-v2).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let value = serde_json::to_value(self).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("projection envelope cannot be serialized ({e})"),
            )]
        })?;
        try_canonical_bytes(&value).map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0004",
                format!("projection envelope cannot be canonicalized ({e})"),
            )]
        })
    }
}
