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
pub mod capability;
pub mod contracts;
pub mod event;
pub mod profile;
pub mod types;

use std::collections::HashMap;
use std::sync::OnceLock;

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
    /// Optional source location (contract req3): where the offending
    /// input came from. Omitted from the wire when unset so previously
    /// serialized diagnostics remain byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DiagnosticSource>,
    /// Optional remediation (contract req3): how to fix the issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<DiagnosticRemediation>,
}

/// Source location attached to a [`Diagnostic`]. All members are optional
/// per the published Diagnostic schema (`source.path`/`source.subject`
/// are the only members Lite populates).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticSource {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

/// Remediation hint attached to a [`Diagnostic`]. `summary` is required
/// by the published Diagnostic schema; `action` is an optional stable
/// identifier for tooling.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticRemediation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub summary: String,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: "error".into(),
            category: category_for(code).to_string(),
            message: message.into(),
            related: Vec::new(),
            source: None,
            remediation: None,
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

    /// Attach a source location and optional subject.
    ///
    /// `path` is an RFC 6901 JSON pointer into the offending document
    /// (`""` = the whole document); e.g. `/body/0` for the first body
    /// unit or `/canonicalization/sha256` for a plan seal member. Per
    /// the published Diagnostic schema both members are plain strings —
    /// no runtime shape is enforced.
    ///
    /// `subject` carries a stable identifier of the offending element
    /// (operation id, workflow id, workflowDigest), usually the first
    /// `related` entry.
    pub fn with_source(mut self, path: impl Into<String>, subject: Option<String>) -> Self {
        self.source = Some(DiagnosticSource {
            path: path.into(),
            subject,
        });
        self
    }

    /// Attach a remediation action id and human-readable summary.
    pub fn with_remediation(
        mut self,
        action: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        self.remediation = Some(DiagnosticRemediation {
            action: Some(action.into()),
            summary: summary.into(),
        });
        self
    }
}

/// Borrowed view of the vendored catalogue — deserialized straight from
/// the static `include_str!` text so every entry is a `&'static str`.
#[derive(serde::Deserialize)]
struct CatalogueFile<'a> {
    #[serde(borrow)]
    codes: Vec<CatalogueEntry<'a>>,
}

#[derive(serde::Deserialize)]
struct CatalogueEntry<'a> {
    code: &'a str,
    category: &'a str,
}

/// The published per-code category table — vendored
/// `vendored/soma/v1.1/diagnostics.json`, the normative source of truth
/// (same rule as the oracle's exact-match `category_for`). Every stable
/// diagnostic code resolves to its exact pinned category, never a
/// family-generic stand-in derived from substring matching.
///
/// Fail-safe: codes absent from the catalogue fall back to `"general"`;
/// `tests/emitted_diagnostics_conformance.rs` proves every code Lite
/// emits is catalogue-registered, so production diagnostics never take
/// the fallback.
fn category_for(code: &str) -> &'static str {
    static CATALOGUE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    CATALOGUE
        .get_or_init(|| {
            let file: CatalogueFile<'static> =
                serde_json::from_str(include_str!("../../../vendored/soma/v1.1/diagnostics.json"))
                    .expect("vendored diagnostics.json parses");
            file.codes
                .into_iter()
                .map(|entry| (entry.code, entry.category))
                .collect()
        })
        .get(code)
        .copied()
        .unwrap_or("general")
}

/// Canonical digest of a parsed JSON value (normative decimal-v2 policy).
/// Fail closed: number-policy violations (non-finite / magnitude /
/// precision) are errors, never substituted values. Inputs that did not
/// pass [`canonical::validate_number_lexemes`] at the text boundary can
/// reach here only via programmatically constructed values.
pub fn try_canonical_digest(
    value: &serde_json::Value,
) -> Result<String, canonical::CanonicalError> {
    canonical::try_canonical_digest(value)
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
/// Exception: the `ExecutionPlan` branch returns policy refusals as
/// `Ok` diagnostics so the boundary's catalogue codes (CMP-0001, CMP-0007)
/// survive instead of collapsing into CMP-0003.
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
    // Number-policy scan before parsing: with `arbitrary_precision` off,
    // serde silently rewrites lexemes (`1.10` → `1.1`) and truncates past
    // ~17 significant digits, so this raw-text guard is the only layer that
    // can refuse such inputs instead of silently altering them.
    let raw_bytes = text.as_bytes();
    match canonical::validate_number_lexemes(raw_bytes) {
        Ok(()) => {}
        // Malformed number grammar is a malformed document, not a policy
        // decision; keep that classification distinct from policy refusals.
        Err(canonical::CanonicalError::MalformedLexeme(e)) => {
            return Err(format!("malformed json: {e}"));
        }
        Err(e) => return Err(format!("number policy violation: {e}")),
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
        // The plan boundary shares `governance_compiler`'s strict validator
        // (review blocker 4) so this branch and `verify_reviewed_plan`
        // refuse exactly the same texts. Policy refusals keep their
        // catalogue codes (CMP-0001/CMP-0003/CMP-0007) as diagnostics
        // instead of collapsing into the generic Err -> CMP-0003 mapping
        // of the other kinds; the raw dup/number/JSON scans above already
        // ran on this text, so anything reaching here was well-formed.
        "ExecutionPlan" => {
            match crate::workflow::governance_compiler::validate_execution_plan_text(text) {
                Err(d) => Ok(vec![d]),
                Ok(_) => Ok(vec![]),
            }
        }
        other => Err(format!("unsupported artifact kind {other:?}")),
    }
}
