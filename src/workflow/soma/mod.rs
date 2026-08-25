//! Canonical SOMA++ workflow contract families for the Lite production
//! compiler/runtime (issue #159).
//!
//! Ownership: SOMA/SOMA++ own the normative semantics, canonical
//! serialization, schemas, diagnostics, and compatibility rules. This module
//! IMPLEMENTS the published contracts (v1.1 bundle) in Lite and proves
//! conformance against the digest-pinned fixtures vendored under
//! `vendored/soma/v1.1/`. It defines no competing AST and adds no Lite-only
//! semantic fields.
//!
//! Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-canonical + soma-validate), with one
//! documented divergence: number lexemes are normalized from parsed values
//! rather than preserved verbatim (Lite does not enable
//! `serde_json/arbitrary_precision` globally); see `canonical.rs`.

pub mod adapters;
pub mod audit_workflow;
pub mod canonical;
pub mod contracts;
pub mod event;
pub mod profile;
pub mod types;

use types::SemVer;

pub type SupportedVersion = SemVer;

/// The bundle schema version these models normatively describe.
pub const SUPPORTED_SCHEMA_VERSION: &str = "1.1.0";

/// Parsed supported version (fail-fast constant).
pub fn supported_version() -> SemVer {
    SemVer::parse(SUPPORTED_SCHEMA_VERSION).expect("constant is valid")
}

/// A stable SOMA diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub category: String,
    pub message: String,
    #[serde(default)]
    pub related: Vec<String>,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "error".into(),
            category: category_for(code).to_string(),
            message: message.into(),
            related: Vec::new(),
        }
    }

    pub fn related(
        code: &'static str,
        message: impl Into<String>,
        related: impl Into<String>,
    ) -> Self {
        let mut d = Self::new(code, message);
        d.related.push(related.into());
        d
    }
}

fn category_for(code: &str) -> &'static str {
    if code.contains("AUTH") {
        "authority_expansion"
    } else if code.contains("GOV") {
        "governance"
    } else if code.contains("EXP") {
        "expansion"
    } else if code.contains("OUT") {
        "outcome"
    } else if code.contains("CMP") {
        "canonical_integrity"
    } else if code.contains("RES") {
        "resume"
    } else if code.contains("EVT") {
        "event"
    } else if code.contains("PROF") {
        "profile"
    } else if code.contains("ADAPT") {
        "adapter"
    } else {
        "general"
    }
}

/// Canonical digest of a parsed JSON value (normative decimal-v2 policy).
pub fn canonical_digest(value: &serde_json::Value) -> String {
    canonical::canonical_digest(value)
}

/// Normalized fixed-point lexeme for a JSON number (governance/budget use);
/// `None` when the lexeme violates the number policy (fail closed).
pub(crate) fn numeric_lexeme(n: &serde_json::Number) -> Option<String> {
    if n.is_i64() || n.is_u64() {
        return Some(n.to_string());
    }
    let f = n.as_f64()?;
    if !f.is_finite() {
        return None;
    }
    if f == 0.0 {
        return Some("0".into());
    }
    if f.fract() == 0.0 && f.abs() < 1e18 {
        return Some(format!("{}", f as i128));
    }
    let s = format!("{f}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
        if trimmed.is_empty() || trimmed == "-" {
            return Some("0".into());
        }
        return Some(trimmed.to_string());
    }
    Some(s)
}

/// Validate one artifact document by its manifest `artifact` kind against
/// the pinned v1.1 bundle. Lenient on VERSIONS (they surface as stable
/// CMP-0001-family diagnostics inside the audit); strict on structure:
/// duplicate keys and schema violations fail closed before any audit runs.
///
/// Errors (`Err`) are input refusals — malformed JSON or a document that
/// violates its declared schema; callers map them to SOMA-CMP-0003.
pub fn validate_artifact_text(artifact_kind: &str, text: &str) -> Result<Vec<Diagnostic>, String> {
    // Strict structural scan first so duplicate keys cannot pass via the DOM
    // path (serde silently keeps the last duplicate).
    match canonical::find_duplicate_key(text.as_bytes()) {
        Err(_) => return Err("malformed json".into()),
        Ok(Some(_)) => {
            return Ok(vec![Diagnostic::new(
                "SOMA-CMP-0007",
                "duplicate object key",
            )]);
        }
        Ok(None) => {}
    }
    let raw: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("schema violation: {e}"))?;
    let supported = supported_version();

    match artifact_kind {
        "WorkflowDefinition" => {
            let model: contracts::WorkflowDefinition =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit(&supported))
        }
        "WorkEvent" => {
            let model: event::WorkEvent =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit(&supported))
        }
        "WorkEventBatch" => {
            let model: event::WorkEventBatch =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit(&supported))
        }
        "ExecutionProfile" => {
            let model: profile::ExecutionProfile =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit(&supported))
        }
        "HarnessAdapter" => {
            let model: adapters::HarnessAdapter =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit_claims())
        }
        "AdapterConformance" => {
            let model: adapters::AdapterConformance =
                serde_json::from_value(raw).map_err(|e| format!("schema violation: {e}"))?;
            Ok(model.audit(&supported))
        }
        other => Err(format!("unsupported artifact kind {other:?}")),
    }
}
