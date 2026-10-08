//! SPEC 007 capability negotiation: `RuntimeCapabilitySet`,
//! `WorkRequirements`, and the deterministic `CompatibilityDecision`.
//!
//! A declaration is capability METADATA, never authority: possessing a
//! capability never grants permission to use it. Negotiation resolves
//! `work requirements × runtime capabilities × effective authority`
//! before effectful dispatch and may only preserve or reduce authority.
//!
//! Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/src/capability.rs), with
//! these adaptations: imports resolve to Lite's vendored modules
//! (`contracts::AuthorityProfile`/`Budgets`, `profile::Boundaries`,
//! `profile::authority_widened`, `canonical`); the capability
//! declaration families pin to the SPEC 007 v1.2.0 bundle
//! ([`SUPPORTED_CAPABILITY_SCHEMA_VERSION`]) while the v1.1 artifact
//! families keep `SUPPORTED_SCHEMA_VERSION`; and the reference's
//! `numeric::number_cmp` is implemented here (same semantics: compare
//! JSON numbers as f64, treating non-finite as incomparable).

use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::Diagnostic;
use super::canonical::try_canonical_digest;
use super::contracts::{AuthorityProfile, Budgets};
use super::profile::{Boundaries, Boundary, ProcessBoundary, authority_widened};
use super::types::SemVer;

/// The SPEC 007 (v1.2) bundle version these declaration families pin.
/// The v1.1 artifact families (WorkEvent, WorkflowDefinition, …) are
/// unchanged and keep `SUPPORTED_SCHEMA_VERSION`.
pub const SUPPORTED_CAPABILITY_SCHEMA_VERSION: &str = "1.2.0";

fn supported_capability_semver() -> SemVer {
    SemVer::parse(SUPPORTED_CAPABILITY_SCHEMA_VERSION).expect("constant is valid")
}

/// Three-way comparison of JSON numbers that is EXACT for every pair
/// the SOMA schemas can express. The invariant: an INTEGER operand
/// (u64/i64) is NEVER converted through f64 — integers beyond 2^53
/// would collapse there, so a hard limit one unit above its ceiling
/// must still compare Greater. A finite DECIMAL operand that is itself
/// integral converts to i128 exactly; a non-integral one (necessarily
/// of magnitude < 2^52) is compared against small integers through a
/// lossless f64 cast and against large ones by magnitude. When exact
/// ordering cannot be established (an integral decimal beyond i128's
/// exact range, or a non-finite value the parser should never emit),
/// the result is `Cmp::Incomparable` and callers fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    NotGreater,
    Greater,
    Incomparable,
}

/// One operand reduced to an exactly comparable form.
enum Exact {
    /// An exact integer: a u64/i64 lexeme, or a finite f64 lexeme that
    /// is integral and within i128's exact conversion range.
    Int(i128),
    /// A finite NON-integral f64 (magnitude < 2^52 by f64 spacing, so
    /// the value is exactly representable).
    Frac(f64),
}

fn exact_integer(n: &Number) -> Option<i128> {
    if let Some(u) = n.as_u64() {
        return Some(i128::from(u));
    }
    if let Some(i) = n.as_i64() {
        return Some(i128::from(i));
    }
    None
}

fn exact_operand(n: &Number) -> Option<Exact> {
    if let Some(x) = exact_integer(n) {
        return Some(Exact::Int(x));
    }
    let f = n.as_f64()?;
    if !f.is_finite() {
        return None;
    }
    if f.fract() == 0.0 {
        // Integral decimal lexeme: the cast is exact while |f| < 2^127
        // (at that magnitude f64 spacing is 2^74, so every integral
        // f64 below 2^127 is an integer i128 can hold). Beyond it the
        // cast would saturate — fail closed instead of comparing a
        // corrupted value.
        if f.abs() < 2.0f64.powi(127) {
            return Some(Exact::Int(f as i128));
        }
        return None;
    }
    Some(Exact::Frac(f))
}

/// An exact integer vs a non-integral decimal (magnitude < 2^52).
fn int_vs_frac(i: i128, f: f64) -> Cmp {
    if i > TWO_POW_53 {
        // i >= 2^53+1 > 2^52 > f
        Cmp::Greater
    } else if i < -TWO_POW_53 {
        // i <= -(2^53+1) < -2^52 <= f
        Cmp::NotGreater
    } else {
        // |i| <= 2^53: every such integer is exactly representable in
        // f64, so this cast is lossless and the comparison exact.
        if (i as f64) > f {
            Cmp::Greater
        } else {
            Cmp::NotGreater
        }
    }
}

const TWO_POW_53: i128 = 1 << 53;

fn number_cmp(a: &Number, b: &Number) -> Cmp {
    let (x, y) = match (exact_operand(a), exact_operand(b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return Cmp::Incomparable,
    };
    match (x, y) {
        (Exact::Int(i), Exact::Int(j)) => {
            if i > j {
                Cmp::Greater
            } else {
                Cmp::NotGreater
            }
        }
        (Exact::Frac(i), Exact::Frac(j)) => {
            if i > j {
                Cmp::Greater
            } else {
                Cmp::NotGreater
            }
        }
        (Exact::Int(i), Exact::Frac(f)) => int_vs_frac(i, f),
        (Exact::Frac(f), Exact::Int(i)) => match int_vs_frac(i, f) {
            Cmp::Greater => Cmp::NotGreater,
            Cmp::NotGreater => Cmp::Greater,
            Cmp::Incomparable => Cmp::Incomparable,
        },
    }
}

/// Execution locality of a runtime (closed variant set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locality {
    Local,
    Remote,
    Hybrid,
}

/// Implementation identity (name + revision), mirroring HarnessAdapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Identity {
    pub name: String,
    pub revision: String,
}

/// Adapter identity/revision pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterFamily {
    pub family: String,
    pub revision: String,
}

/// A versioned declaration of what an implementation CAN do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RuntimeCapabilitySet {
    pub id: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub identity: Identity,
    pub declared_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spec_versions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_families: Vec<AdapterFamily>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect_classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Boundaries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_import: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_export: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_support: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_projection: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locality: Option<Locality>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub privacy_classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_structured_input: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_structured_output: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_ceilings: Option<Budgets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_attachment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_escalation: Option<bool>,
}

impl RuntimeCapabilitySet {
    /// Structural audit: container-level version compatibility only.
    /// Capability metadata makes no cross-artifact claims by itself.
    pub fn audit(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        gate_version(&mut out, &self.schema_version, "capability set");
        gate_version(&mut out, &self.version, "capability set");
        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }

    /// Canonical digest of this declaration (the identity referenced by
    /// decisions and freshness policy).
    pub fn canonical_digest(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        try_canonical_digest(&value).map_err(|e| e.to_string())
    }
}

/// Minimum adapter family revision demanded by the work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterRequirement {
    pub family: String,
    pub min_revision: String,
}

/// Freshness/trust policy over capability declarations. Timestamps are
/// RFC 3339 UTC strings compared lexicographically (the established
/// injected-clock-domain pattern); no wall clock is consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Freshness {
    pub not_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_sources: Option<Vec<String>>,
}

/// Continuity obligations the work imposes on the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuity {
    #[serde(
        default,
        rename = "checkpointExport",
        skip_serializing_if = "Option::is_none"
    )]
    pub checkpoint_export: Option<bool>,
    #[serde(
        default,
        rename = "checkpointImport",
        skip_serializing_if = "Option::is_none"
    )]
    pub checkpoint_import: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceObligations {
    #[serde(
        default,
        rename = "requireEvidence",
        skip_serializing_if = "Option::is_none"
    )]
    pub require_evidence: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LocalityConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_execution: Option<Vec<Locality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<Vec<String>>,
}

/// Portable requirements derived from a work definition / compiled plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkRequirements {
    pub id: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub required_spec_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_requirements: Vec<AdapterRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional_capabilities: Vec<String>,
    pub degradation_permitted: bool,
    pub forbid_substitution: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Boundaries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity: Option<Continuity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_obligations: Option<EvidenceObligations>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_limits: Option<Budgets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<Freshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locality: Option<LocalityConstraints>,
}

impl WorkRequirements {
    /// Structural audit: container-level version compatibility only.
    pub fn audit(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        gate_version(&mut out, &self.schema_version, "requirements");
        gate_version(&mut out, &self.version, "requirements");
        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }

    /// Canonical digest of these requirements.
    pub fn canonical_digest(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        try_canonical_digest(&value).map_err(|e| e.to_string())
    }
}

/// Outcome status of a negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Compatible,
    CompatibleWithDegradation,
    Incompatible,
}

/// Provenance evidence for one declaration used by a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeclarationRef {
    pub digest: String,
    pub source: String,
    pub declared_at: String,
}

/// An explicit runtime/harness/workspace substitution record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Substitution {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Explicitly selected adapter (identity + revision).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdapterSelection {
    pub family: String,
    pub name: String,
    pub revision: String,
}

/// Declared/effective authority snapshot embedded for self-contained
/// fail-closed verification (negotiation may only preserve or reduce).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityPair {
    pub declared: AuthorityProfile,
    pub effective: AuthorityProfile,
}

/// Snapshot of the work's continuity/evidence/hard-limit/locality
/// constraints, embedded so re-verification is self-contained.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RequirementConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity: Option<Continuity>,
    #[serde(
        default,
        rename = "requireEvidence",
        skip_serializing_if = "Option::is_none"
    )]
    pub require_evidence: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_limits: Option<Budgets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_execution: Option<Vec<Locality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_requirements: Option<Vec<AdapterRequirement>>,
}

/// Fail-closed RFC 3339 UTC-Z validation (the schema-enforced pattern):
/// `YYYY-MM-DDTHH:MM:SSZ`. Lexicographic comparison is only well-defined
/// for this normalized form. The shape check below pins the lexeme; the
/// chrono parse then verifies it is a GENUINE calendar instant — a
/// punctuation-shaped impossibility like `9999-99-99T99:99:99Z` matches
/// the schema's regex but is not a real date-time and must fail closed.
fn is_rfc3339_z(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z'
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 10 | 13 | 16 | 19 => true,
            _ => c.is_ascii_digit(),
        })
        && chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").is_ok()
}

/// The vendored schemas pin version lexemes to `^\d+\.\d+\.\d+$`
/// (digits-only, exactly three components). Compatibility is judged
/// separately (SOMA-CMP-0001); this is pure lexeme validity.
fn is_semver_shaped(s: &str) -> bool {
    let mut parts = s.split('.');
    let all_digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), Some(c), None) => all_digits(a) && all_digits(b) && all_digits(c),
        _ => false,
    }
}

/// Budget value constraints from the vendored `Budgets` definitions:
/// every dimension is `minimum: 0`, and `retries`/`concurrency` are
/// `type: integer`. JSON Schema integer semantics: a mathematically
/// integral value is valid in ANY notation — `1.0` and `0.0` are valid
/// integers while `1.5` is not — so the integer check is
/// `fract() == 0.0` on the finite value (large integral decimals and
/// u64/i64 integers alike), not the parser's storage representation.
fn validate_budgets(label: &str, budgets: Option<&Budgets>) -> Result<(), String> {
    let Some(b) = budgets else {
        return Ok(());
    };
    for dim in Budgets::DIMENSIONS {
        let Some(n) = b.get(dim) else {
            continue;
        };
        let Some(f) = n.as_f64() else {
            return Err(format!("{label}.{dim} is not a finite number"));
        };
        if f < 0.0 {
            return Err(format!(
                "{label}.{dim} must be nonnegative (minimum 0), got {n}"
            ));
        }
        if matches!(dim, "retries" | "concurrency") && f.fract() != 0.0 {
            return Err(format!(
                "{label}.{dim} must be an integer (type integer), got {n}"
            ));
        }
    }
    Ok(())
}

/// Value constraints the vendored WorkRequirements schema imposes
/// beyond the Rust shapes: semver-shaped version lexemes, nonnegative
/// (and integer-only where pinned) hard limits, and a genuinely valid
/// RFC 3339 UTC-Z freshness instant.
pub fn validate_work_requirements(reqs: &WorkRequirements) -> Result<(), String> {
    for (label, v) in [
        ("requirements.schemaVersion", &reqs.schema_version),
        ("requirements.version", &reqs.version),
        (
            "requirements.requiredSpecVersion",
            &reqs.required_spec_version,
        ),
    ] {
        if !is_semver_shaped(v) {
            return Err(format!(
                "{label} {v:?} violates the ^\\d+\\.\\d+\\.\\d+$ version pattern"
            ));
        }
    }
    validate_budgets("requirements.hardLimits", reqs.hard_limits.as_ref())?;
    if let Some(fr) = &reqs.freshness
        && !is_rfc3339_z(&fr.not_after)
    {
        return Err(format!(
            "requirements.freshness.notAfter {:?} is not a genuine RFC 3339 UTC-Z timestamp",
            fr.not_after
        ));
    }
    Ok(())
}

/// Value constraints the vendored AuthorityProfile schema imposes on
/// the caller-supplied authority profiles (the Rust shape enforces the
/// closed enum sets and deny_unknown_fields; budgets still carry the
/// nonnegative/integer-only value constraints).
pub fn validate_authority_profile(label: &str, auth: &AuthorityProfile) -> Result<(), String> {
    validate_budgets(&format!("{label}.budgets"), auth.budgets.as_ref())
}

/// Value constraints the vendored RuntimeCapabilitySet schema imposes
/// beyond the Rust shapes (semver-shaped versions, a genuine declaredAt
/// instant, nonnegative integer-constrained ceilings, and nonnegative
/// structured-input/output maxima).
fn validate_capability_set(caps: &RuntimeCapabilitySet) -> Result<(), String> {
    for (label, v) in [
        ("capability set schemaVersion", &caps.schema_version),
        ("capability set version", &caps.version),
    ] {
        if !is_semver_shaped(v) {
            return Err(format!(
                "{label} {v:?} violates the ^\\d+\\.\\d+\\.\\d+$ version pattern"
            ));
        }
    }
    for v in &caps.spec_versions {
        if !is_semver_shaped(v) {
            return Err(format!(
                "capability set specVersions entry {v:?} violates the version pattern"
            ));
        }
    }
    if !is_rfc3339_z(&caps.declared_at) {
        return Err(format!(
            "capability set declaredAt {:?} is not a genuine RFC 3339 UTC-Z timestamp",
            caps.declared_at
        ));
    }
    validate_budgets(
        "capability set budgetCeilings",
        caps.budget_ceilings.as_ref(),
    )?;
    for (label, max) in [
        ("maxStructuredInput", &caps.max_structured_input),
        ("maxStructuredOutput", &caps.max_structured_output),
    ] {
        if let Some(n) = max
            && n.as_f64().is_some_and(|f| f < 0.0)
        {
            return Err(format!("{label} must be nonnegative (minimum 0), got {n}"));
        }
    }
    Ok(())
}

/// Snapshot of the runtime's support surface used by the resolution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CapabilitySupport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_support: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_import: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_export: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_attachment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locality: Option<Locality>,
    #[serde(
        default,
        rename = "privacyClasses",
        skip_serializing_if = "Option::is_none"
    )]
    pub privacy_classes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_ceilings: Option<Budgets>,
}

/// Deterministic result of resolving
/// `work requirements × runtime capabilities × effective authority`.
///
/// The decision embeds snapshots of everything it was computed from so any
/// consumer (including a resumed/handoff run) can re-verify it without
/// access to the original artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityDecision {
    pub id: String,
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    #[serde(rename = "requirementsDigest")]
    pub requirements_digest: String,
    #[serde(rename = "capabilitySetDigest")]
    pub capability_set_digest: String,
    #[serde(rename = "evaluatedAt")]
    pub evaluated_at: String,
    pub status: DecisionStatus,
    #[serde(
        default,
        rename = "selectedSpecVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub selected_spec_version: Option<String>,
    #[serde(
        default,
        rename = "declaredSpecVersions",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub declared_spec_versions: Vec<String>,
    #[serde(
        default,
        rename = "selectedAdapters",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub selected_adapters: Vec<AdapterSelection>,
    #[serde(
        default,
        rename = "requiredCapabilities",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub required_capabilities: Vec<String>,
    #[serde(
        default,
        rename = "capabilitiesCovered",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub capabilities_covered: Vec<String>,
    #[serde(
        default,
        rename = "optionalCapabilities",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub optional_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub omissions: Vec<String>,
    #[serde(
        default,
        rename = "workspaceRequired",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_required: Option<Boundaries>,
    #[serde(
        default,
        rename = "workspaceDeclared",
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_declared: Option<Boundaries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<Freshness>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<DeclarationRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub substitutions: Vec<Substitution>,
    #[serde(default, rename = "substitutionPermitted")]
    pub substitution_permitted: bool,
    #[serde(default, rename = "degradationPermitted")]
    pub degradation_permitted: bool,
    pub authority: AuthorityPair,
    #[serde(
        default,
        rename = "requirementConstraints",
        skip_serializing_if = "Option::is_none"
    )]
    pub requirement_constraints: Option<RequirementConstraints>,
    #[serde(
        default,
        rename = "capabilitySupport",
        skip_serializing_if = "Option::is_none"
    )]
    pub capability_support: Option<CapabilitySupport>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

fn rank_boundary(b: Option<Boundary>) -> u8 {
    match b {
        Some(Boundary::Read) => 1,
        Some(Boundary::Write) => 2,
        _ => 0,
    }
}

fn rank_process(p: Option<ProcessBoundary>) -> u8 {
    // SPEC 007 section 5 step 5: process/network boundaries rank
    // `none < allow < deny` (an absent dimension is `none`). Mapping
    // deny below allow would REVERSE compatibility decisions.
    match p {
        None | Some(ProcessBoundary::None) => 0,
        Some(ProcessBoundary::Allow) => 1,
        Some(ProcessBoundary::Deny) => 2,
    }
}

/// Fail-closed container-version gate: a version that is unparseable OR
/// newer than the supported bundle is SOMA-CMP-0001 — never skipped.
/// (SPEC 005: undecidable inputs fail closed; the strict entry point
/// additionally rejects them outright.)
fn gate_version(out: &mut Vec<Diagnostic>, v: &str, what: &str) {
    match SemVer::parse(v) {
        Ok(parsed) => {
            if !parsed.is_compatible_with(&supported_capability_semver()) {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!("{what} version {v} newer than bundle"),
                ));
            }
        }
        Err(_) => out.push(Diagnostic::new(
            "SOMA-CMP-0001",
            format!("{what} version {v:?} is unparseable"),
        )),
    }
}

impl CompatibilityDecision {
    /// Every normative rule except the status/consistency one. Used by
    /// [`Self::audit`] and by [`resolve`], which fixes the status AFTER
    /// computing the substantive result so consistency holds by construction.
    fn substantive_audit(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        // SOMA-CMP-0001: container version gate.
        gate_version(&mut out, &self.schema_version, "decision");
        gate_version(&mut out, &self.version, "decision");

        // SOMA-CAP-0002: selected contract version must be declared covered.
        if let Some(sel) = &self.selected_spec_version
            && !self.declared_spec_versions.iter().any(|v| v == sel)
        {
            out.push(Diagnostic::new(
                "SOMA-CAP-0002",
                format!("selected contract version {sel} is not declared by the runtime"),
            ));
        }

        // SOMA-CAP-0001: every required capability must be covered.
        for rc in &self.required_capabilities {
            if !self.capabilities_covered.iter().any(|c| c == rc) {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0001",
                    format!("required capability {rc} is not covered"),
                ));
            }
        }

        // SOMA-CAP-0005: required workspace boundary exceeds declared mode.
        for (dim, rank) in [
            ("workspace", rank_boundary),
            ("filesystem", rank_boundary),
            ("git", rank_boundary),
            ("secret", rank_boundary),
        ] {
            let pick = |w: Option<&Boundaries>| {
                w.and_then(|w| match dim {
                    "workspace" => w.workspace,
                    "filesystem" => w.filesystem,
                    "git" => w.git,
                    _ => w.secret,
                })
            };
            if rank(pick(self.workspace_required.as_ref()))
                > rank(pick(self.workspace_declared.as_ref()))
            {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0005",
                    format!("{dim} requirement exceeds the declared runtime mode"),
                ));
            }
        }
        for (dim, rank) in [("process", rank_process), ("network", rank_process)] {
            let pick = |w: Option<&Boundaries>| {
                w.and_then(|w| match dim {
                    "process" => w.process,
                    _ => w.network,
                })
            };
            if rank(pick(self.workspace_required.as_ref()))
                > rank(pick(self.workspace_declared.as_ref()))
            {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0005",
                    format!("{dim} requirement exceeds the declared runtime mode"),
                ));
            }
        }

        // SOMA-CAP-0003: declaration provenance must be present, fresh,
        // trusted. Timestamps entering lexicographic comparison must be
        // RFC 3339 UTC-Z normalized — non-normalized input fails closed
        // instead of producing arbitrary orderings.
        let mut ts_ok = true;
        if !is_rfc3339_z(&self.evaluated_at) {
            out.push(Diagnostic::new(
                "SOMA-CMP-0003",
                format!(
                    "evaluatedAt {:?} is not an RFC 3339 UTC-Z timestamp",
                    self.evaluated_at
                ),
            ));
            ts_ok = false;
        }
        if let Some(fr) = &self.freshness
            && !is_rfc3339_z(&fr.not_after)
        {
            out.push(Diagnostic::new(
                "SOMA-CMP-0003",
                format!(
                    "freshness.notAfter {:?} is not an RFC 3339 UTC-Z timestamp",
                    fr.not_after
                ),
            ));
            ts_ok = false;
        }
        for d in &self.declarations {
            if !is_rfc3339_z(&d.declared_at) {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0003",
                    format!(
                        "declaration declaredAt {:?} is not an RFC 3339 UTC-Z timestamp",
                        d.declared_at
                    ),
                ));
                ts_ok = false;
            }
        }
        // Only compare lexicographically when every participant is
        // well-formed; otherwise the verdicts above already fail closed.
        if ts_ok {
            if self.declarations.is_empty() {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0003",
                    "decision carries no capability-declaration provenance",
                ));
            } else {
                // A freshness window that already closed at evaluation time
                // means the metadata was stale when the decision was made.
                if let Some(fr) = &self.freshness
                    && self.evaluated_at.as_str() > fr.not_after.as_str()
                {
                    out.push(Diagnostic::new(
                        "SOMA-CAP-0003",
                        "freshness window closed before the evaluation instant",
                    ));
                }
                for d in &self.declarations {
                    if d.declared_at.as_str() > self.evaluated_at.as_str() {
                        out.push(Diagnostic::new(
                            "SOMA-CAP-0003",
                            format!(
                                "declaration {} is future-dated relative to evaluation",
                                d.digest
                            ),
                        ));
                    }
                    if let Some(fr) = &self.freshness {
                        if d.declared_at.as_str() > fr.not_after.as_str() {
                            out.push(Diagnostic::new(
                                "SOMA-CAP-0003",
                                format!(
                                    "declaration {} postdates the freshness window cutoff",
                                    d.digest
                                ),
                            ));
                        }
                        if let Some(trusted) = fr.trusted_sources.as_ref() {
                            // An EMPTY trust list normatively excludes every
                            // source (fail closed on degenerate input).
                            if !trusted.iter().any(|t| t == &d.source) {
                                out.push(Diagnostic::new(
                                    "SOMA-CAP-0003",
                                    format!(
                                        "declaration source {:?} is not a trusted source",
                                        d.source
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        // SOMA-CAP-0004: substitutions must be permitted and justified.
        if !self.substitutions.is_empty() {
            if !self.substitution_permitted {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0004",
                    "substitution is forbidden by the work requirements",
                ));
            }
            for s in &self.substitutions {
                if s.reason.as_deref().map(str::trim).unwrap_or("").is_empty() {
                    out.push(Diagnostic::new(
                        "SOMA-CAP-0004",
                        format!("substitution {} -> {} carries no reason", s.from, s.to),
                    ));
                }
            }
        }

        // SOMA-CAP-0007: degradation must be explicitly permitted.
        if !self.degradation_permitted
            && (!self.omissions.is_empty()
                || self.status == DecisionStatus::CompatibleWithDegradation)
        {
            out.push(Diagnostic::new(
                "SOMA-CAP-0007",
                "degradation is not permitted by the work requirements",
            ));
        }

        // SOMA-CAP-0006: negotiated authority may never widen.
        if authority_widened(&self.authority.effective, &self.authority.declared) {
            out.push(Diagnostic::new(
                "SOMA-CAP-0006",
                "negotiated effective authority widens the declared authority",
            ));
        }

        // Continuity / evidence / locality / hard-limit obligations, checked
        // against the embedded support snapshot (self-contained). A demanded
        // obligation with NO support snapshot fails closed: absence of
        // evidence of support is not support.
        if let Some(rc) = &self.requirement_constraints {
            let cs = self.capability_support.as_ref();
            if let Some(cont) = &rc.continuity {
                for (flag, supported, label) in [
                    (
                        cont.resume,
                        cs.and_then(|c| c.resume_support),
                        "resume support",
                    ),
                    (
                        cont.checkpoint_import,
                        cs.and_then(|c| c.checkpoint_import),
                        "checkpoint import support",
                    ),
                    (
                        cont.checkpoint_export,
                        cs.and_then(|c| c.checkpoint_export),
                        "checkpoint export support",
                    ),
                ] {
                    if flag == Some(true) && supported != Some(true) {
                        out.push(Diagnostic::new(
                            "SOMA-CAP-0001",
                            format!("required {label} is not declared by the runtime"),
                        ));
                    }
                }
            }
            if rc.require_evidence == Some(true)
                && cs.and_then(|c| c.evidence_attachment) != Some(true)
            {
                out.push(Diagnostic::new(
                    "SOMA-CAP-0001",
                    "required evidence attachment is not declared by the runtime",
                ));
            }
            match (&rc.allowed_execution, cs.and_then(|c| c.locality)) {
                (Some(allowed), Some(actual)) => {
                    if !allowed.is_empty() && !allowed.contains(&actual) {
                        out.push(Diagnostic::new(
                            "SOMA-CAP-0005",
                            format!(
                                "locality requirement excludes the declared runtime locality {:?}",
                                actual
                            ),
                        ));
                    }
                }
                (Some(allowed), None) if !allowed.is_empty() => {
                    out.push(Diagnostic::new(
                        "SOMA-CAP-0005",
                        "locality requirement with no declared runtime locality",
                    ));
                }
                _ => {}
            }
            // Demanded privacy classes must be declared by the runtime.
            if let Some(demanded) = &rc.privacy
                && !demanded.is_empty()
            {
                let provided = cs.and_then(|c| c.privacy_classes.as_ref());
                for p in demanded {
                    match provided {
                        None => out.push(Diagnostic::new(
                            "SOMA-CAP-0001",
                            format!("required privacy class {p} is not declared by the runtime"),
                        )),
                        Some(list) if !list.contains(p) => out.push(Diagnostic::new(
                            "SOMA-CAP-0001",
                            format!("required privacy class {p} is not declared by the runtime"),
                        )),
                        Some(_) => {}
                    }
                }
            }
            // Demanded adapter families/revision floors must be selected;
            // an unmatched demand is SOMA-CAP-0002 (never a silent pass).
            if let Some(demands) = &rc.adapter_requirements {
                for ar in demands {
                    let matched = self.selected_adapters.iter().any(|s| {
                        s.family == ar.family && s.revision.as_str() >= ar.min_revision.as_str()
                    });
                    if !matched {
                        out.push(Diagnostic::new(
                            "SOMA-CAP-0002",
                            format!(
                                "required adapter family {} at revision >= {} is not declared by the runtime",
                                ar.family, ar.min_revision
                            ),
                        ));
                    }
                }
            }
            if let Some(limits) = &rc.hard_limits {
                let ceilings = cs.and_then(|c| c.budget_ceilings.as_ref());
                for dim in Budgets::DIMENSIONS {
                    if let Some(limit) = limits.get(dim) {
                        match ceilings.and_then(|b| b.get(dim)) {
                            None => out.push(Diagnostic::new(
                                "SOMA-AUTH-0009",
                                format!("hard limit {dim} has no declared runtime ceiling"),
                            )),
                            Some(c) => match number_cmp(limit, c) {
                                Cmp::Greater | Cmp::Incomparable => out.push(Diagnostic::new(
                                    "SOMA-AUTH-0009",
                                    format!("hard limit {dim} exceeds the runtime budget ceiling"),
                                )),
                                Cmp::NotGreater => {}
                            },
                        }
                    }
                }
            }
        }

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }

    /// Self-contained fail-closed audit. Every normative rule checks only
    /// fields embedded in the decision, so re-verification after handoff or
    /// resume needs nothing but this document.
    pub fn audit(&self) -> Vec<Diagnostic> {
        let mut out = self.substantive_audit();

        // SOMA-CAP-0008: status must agree with BOTH the recorded diagnostics
        // and the recorded omissions.
        let has_diags = !self.diagnostics.is_empty();
        if (self.status == DecisionStatus::Incompatible) != has_diags {
            out.push(Diagnostic::new(
                "SOMA-CAP-0008",
                "status is inconsistent with the recorded diagnostics",
            ));
        }
        if self.status == DecisionStatus::Compatible && !self.omissions.is_empty() {
            out.push(Diagnostic::new(
                "SOMA-CAP-0008",
                "compatible status is inconsistent with recorded omissions",
            ));
        }
        if self.status == DecisionStatus::CompatibleWithDegradation
            && self.omissions.is_empty()
            && !has_diags
        {
            out.push(Diagnostic::new(
                "SOMA-CAP-0008",
                "degraded status with no omissions and no diagnostics",
            ));
        }

        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}

/// Inputs to the deterministic pre-dispatch negotiation.
pub struct NegotiationInputs<'a> {
    pub id: String,
    pub requirements: &'a WorkRequirements,
    pub capabilities: &'a RuntimeCapabilitySet,
    pub authority_declared: &'a AuthorityProfile,
    pub authority_effective: &'a AuthorityProfile,
    /// Injected clock instant (RFC 3339 UTC-Z); never a wall clock in the
    /// library — the transport injects its evaluation instant.
    pub evaluated_at: String,
    /// Substitutions proposed by the caller; each MUST carry a reason.
    pub substitutions: Vec<Substitution>,
}

/// Resolve the negotiation deterministically. Same inputs always produce the
/// same decision (no randomness, no ambient state, lexicographic timestamps).
pub fn resolve(inputs: &NegotiationInputs<'_>) -> Result<CompatibilityDecision, String> {
    let reqs = inputs.requirements;
    let caps = inputs.capabilities;

    // SCHEMA-CONTRACT GATE — the value constraints of the vendored SPEC
    // 007 schemas, not just the Rust shapes. A violation fails closed
    // HERE, before any decision exists: the CompatibilityDecision schema
    // snapshots freshness, the authority pair, the hard limits, and the
    // declarations verbatim, so resolving schema-invalid inputs would
    // EMIT a schema-invalid decision document (whether called from the
    // HTTP handler or any other caller). No decision, no digest, no
    // status — Err.
    validate_work_requirements(reqs)
        .and_then(|_| validate_capability_set(caps))
        .and_then(|_| validate_authority_profile("authority.declared", inputs.authority_declared))
        .and_then(|_| validate_authority_profile("authority.effective", inputs.authority_effective))
        .map_err(|e| format!("schema-invalid negotiation input: {e}"))?;
    if !is_rfc3339_z(&inputs.evaluated_at) {
        return Err(format!(
            "schema-invalid negotiation input: evaluatedAt {:?} is not a genuine RFC 3339 UTC-Z timestamp",
            inputs.evaluated_at
        ));
    }

    // SPEC 007 section 5 step 1 — INPUT version gate. An artifact whose
    // container versions exceed the supported bundle fails closed BEFORE any
    // negotiation outcome is produced (never fail open on newer inputs).
    let mut input_gate: Vec<Diagnostic> = Vec::new();
    input_gate.extend(reqs.audit());
    input_gate.extend(caps.audit());
    match SemVer::parse(&reqs.required_spec_version) {
        Ok(parsed) => {
            if !parsed.is_compatible_with(&supported_capability_semver()) {
                input_gate.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!(
                        "required spec version {} is beyond the supported bundle",
                        reqs.required_spec_version
                    ),
                ));
            }
        }
        Err(_) => input_gate.push(Diagnostic::new(
            "SOMA-CMP-0001",
            format!(
                "required spec version {:?} is unparseable",
                reqs.required_spec_version
            ),
        )),
    }

    let requirements_digest = reqs.canonical_digest()?;
    let capability_set_digest = caps.canonical_digest()?;

    // Timestamps entering the lexicographic clock domain are guaranteed
    // genuine RFC 3339 UTC-Z instants by the schema-contract gate above
    // (evaluatedAt, declaredAt, and freshness.notAfter all fail closed
    // there); the decision audit re-verifies the embedded copies.

    // Covered = demanded classes present in the runtime's declared
    // effect classes. Missing REQUIRED classes remain uncovered and
    // surface as SOMA-CAP-0001 through the audit below.
    let demand: Vec<String> = reqs
        .required_capabilities
        .iter()
        .chain(reqs.optional_capabilities.iter())
        .cloned()
        .collect();
    let capabilities_covered: Vec<String> = demand
        .iter()
        .filter(|c| caps.effect_classes.iter().any(|e| e == *c))
        .cloned()
        .collect();
    let omissions: Vec<String> = reqs
        .optional_capabilities
        .iter()
        .filter(|c| !caps.effect_classes.iter().any(|e| e == *c))
        .cloned()
        .collect();

    // Adapter selection is explicit by family + revision (byte-wise string
    // compare, documented normatively in SPEC 007 section 5 step 3).
    let selected_adapters: Vec<AdapterSelection> = reqs
        .adapter_requirements
        .iter()
        .filter_map(|r| {
            caps.adapter_families
                .iter()
                .find(|f| f.family == r.family && f.revision.as_str() >= r.min_revision.as_str())
                .map(|f| AdapterSelection {
                    family: f.family.clone(),
                    name: caps.identity.name.clone(),
                    revision: f.revision.clone(),
                })
        })
        .collect();

    let declarations = vec![DeclarationRef {
        digest: capability_set_digest.clone(),
        source: caps.identity.name.clone(),
        declared_at: caps.declared_at.clone(),
    }];

    let mut decision = CompatibilityDecision {
        id: inputs.id.clone(),
        schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        requirements_digest,
        capability_set_digest,
        evaluated_at: inputs.evaluated_at.clone(),
        status: DecisionStatus::Compatible,
        selected_spec_version: Some(reqs.required_spec_version.clone()),
        declared_spec_versions: caps.spec_versions.clone(),
        selected_adapters,
        required_capabilities: reqs.required_capabilities.clone(),
        capabilities_covered,
        optional_capabilities: reqs.optional_capabilities.clone(),
        omissions,
        workspace_required: reqs.workspace,
        workspace_declared: caps.workspace,
        freshness: reqs.freshness.clone(),
        declarations,
        substitutions: inputs.substitutions.clone(),
        substitution_permitted: !reqs.forbid_substitution,
        degradation_permitted: reqs.degradation_permitted,
        authority: AuthorityPair {
            declared: inputs.authority_declared.clone(),
            effective: inputs.authority_effective.clone(),
        },
        requirement_constraints: Some(RequirementConstraints {
            continuity: reqs.continuity.clone(),
            require_evidence: reqs
                .evidence_obligations
                .as_ref()
                .and_then(|e| e.require_evidence),
            hard_limits: reqs.hard_limits.clone(),
            allowed_execution: reqs
                .locality
                .as_ref()
                .and_then(|l| l.allowed_execution.clone()),
            privacy: reqs.locality.as_ref().and_then(|l| l.privacy.clone()),
            adapter_requirements: if reqs.adapter_requirements.is_empty() {
                None
            } else {
                Some(reqs.adapter_requirements.clone())
            },
        }),
        capability_support: Some(CapabilitySupport {
            resume_support: caps.resume_support,
            checkpoint_import: caps.checkpoint_import,
            checkpoint_export: caps.checkpoint_export,
            evidence_attachment: caps.evidence_attachment,
            locality: caps.locality,
            privacy_classes: if caps.privacy_classes.is_empty() {
                None
            } else {
                Some(caps.privacy_classes.clone())
            },
            budget_ceilings: caps.budget_ceilings.clone(),
        }),
        diagnostics: Vec::new(),
    };

    // Fail-closed: any substantive diagnostic makes the decision
    // incompatible; permitted degradation is the only compatible-with-
    // degradation path. Status is fixed AFTER the substantive audit so the
    // SOMA-CAP-0008 consistency invariant holds by construction.
    let mut substantive = decision.substantive_audit();
    substantive.extend(input_gate);
    substantive.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
    substantive.dedup_by(|a, b| a.code == b.code && a.message == b.message);
    decision.status = if !substantive.is_empty() {
        DecisionStatus::Incompatible
    } else if !decision.omissions.is_empty() {
        DecisionStatus::CompatibleWithDegradation
    } else {
        DecisionStatus::Compatible
    };
    decision.diagnostics = substantive;
    Ok(decision)
}
