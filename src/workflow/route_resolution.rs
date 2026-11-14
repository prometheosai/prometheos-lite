//! Governed route resolution and the immutable effective execution snapshot
//! (issue #133, Slice A: route decision / policy profile / preflight).
//!
//! Resolves an operator-owned, explicit route candidate set against the SPEC
//! 007 negotiation and the SPEC 006 `ExecutionProfile` contract *before any
//! effect*, and emits a versioned, digest-sealed effective execution snapshot
//! plus sealed evidence of the selected and rejected routes.
//!
//! Design rules (fail-closed, never a second authorization framework):
//!
//! * **Candidates are requirements, never grants.** A profile is an ordered
//!   constraint: `Conservative ⊂ Standard ⊂ Experimental`. No profile grants
//!   authority the work or the operator has not already been granted, and
//!   experimental candidates are only reachable under the experimental
//!   profile. Effective authority never widens declared authority; the
//!   `ExecutionProfile::audit` invariants re-check it at seal time.
//! * **No substitution, no fallback.** A pinned candidate that is ineligible
//!   refuses the whole resolution (`ROUTE-0004`); eligible replacements are
//!   never swapped in. Without a pin, two eligible candidates tied at the
//!   best priority refuse as ambiguous (`ROUTE-0005`) rather than letting
//!   declaration or registration order decide.
//! * **Selection never invokes the adapter.** This layer is preflight only;
//!   governed dispatch integration is Slice B. No network or process effect
//!   occurs here.
//! * **Evidence carries identity and digests, never raw envelopes.** The
//!   required inputs (authority profiles, requirements) are embedded as
//!   canonical digests in the evidence and the snapshot; raw authority
//!   contents (execution class, mutation mode, scopes, secrets) are never
//!   serialized into `RouteEvidenceV1` or `EffectiveExecutionSnapshotV1`.
//!
//! Refusal codes (stable):
//!
//! * `ROUTE-0001` — the route candidate set is empty.
//! * `ROUTE-0002` — duplicate candidate id in the set.
//! * `ROUTE-0003` — no candidate is eligible (per-candidate reasons).
//! * `ROUTE-0004` — pinned candidate unknown or ineligible (no fallback).
//! * `ROUTE-0005` — ambiguity: more than one eligible candidate tied at the
//!   best priority without a pin.
//! * `ROUTE-0006` — request-precondition failure (empty route set id, invalid
//!   route set version/evaluatedAt, requirements demand a workspace that was
//!   not provided, workspace manifest integrity/mode refusal, or ceilings
//!   whose per-dimension strict intersection is unrepresentable).
//!
//! Budget rule (no sampling, no source precedence by accident): the effective
//! ceiling is the per-dimension strict intersection across declared authority
//! budgets, effective authority budgets, work hard limits, runtime capability
//! ceilings, and the candidate budget. An omitted dimension in one source
//! never erases another source's ceiling; the effective ceiling never exceeds
//! any applicable limit.

use anyhow::{Context, bail};
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::workflow::execution_harness::{
    HarnessRegistry, WorkspaceEvidenceV1, WorkspaceManifestDefect, validate_workspace_manifest,
    workspace_adapter_family, workspace_mode_satisfies,
};
use crate::workflow::soma::capability::{
    DecisionStatus, NegotiationInputs, WorkRequirements, is_rfc3339_z, is_semver_shaped, resolve,
    validate_budgets,
};
use crate::workflow::soma::contracts::{AuthorityProfile, Budgets};
use crate::workflow::soma::profile::{ExecutionProfile, ProfileSide, ValidationPolicy};
use crate::workflow::soma::{self, SupportedVersion};
use crate::workflow::workspace::{AdapterKind, WorkspaceManifestV1, WorkspaceMode};

pub const ROUTE_CANDIDATE_SCHEMA_VERSION: &str = "1.0.0";
pub const ROUTE_EVIDENCE_SCHEMA_VERSION: &str = "1.0.0";
pub const EXECUTION_SNAPSHOT_SCHEMA_VERSION: &str = "1.0.0";

/// Operator-requested policy profile. Profiles order constraint strictness;
/// they never widen authority.
///
/// * `conservative` — local providers only; no experimental candidates.
/// * `standard` — any locality the work contract permits; no experimental
///   candidates.
/// * `experimental` — also admits operator-flagged experimental candidates.
///
/// Requirement-level locality constraints (`WorkRequirements::locality`) and
/// the authority envelope apply to every profile; a profile may only narrow
/// them further, never widen them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteProfile {
    Conservative,
    Standard,
    Experimental,
}

/// Trust/freshness metadata for the capability declaration a route would use.
/// Timestamps are RFC 3339 UTC-Z strings compared against the injected
/// evaluation instant; no wall clock is consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteTrust {
    /// Where the declaration came from (operator-audited source handle).
    pub source: String,
    /// RFC 3339 UTC-Z instant the metadata was verified by the operator.
    pub verified_at: String,
    /// How long the verification remains valid, in seconds (>= 1).
    pub ttl_seconds: u64,
}

/// Candidate-declared workspace adapter binding. When set, the actual
/// workspace manifest provided by the caller must match this binding exactly
/// (adapter kind and adapter revision), or the candidate is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteWorkspaceBinding {
    pub adapter: AdapterKind,
    pub revision: String,
}

/// One operator-owned route candidate. Every field is explicit; there are
/// no defaults and no inferred values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RouteCandidateV1 {
    pub schema_version: String,
    /// Operator-chosen stable identifier, unique within the route set.
    pub candidate_id: String,
    /// Explicit provider identity (e.g. `lite-native`, `local-openai`,
    /// `remote-mock`). Never a secret.
    pub provider_identity: String,
    /// Locality of the provider endpoint.
    pub provider_locality: crate::workflow::soma::capability::Locality,
    /// Explicit model identifier, when the route carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Registered harness this route dispatches through.
    pub harness_id: String,
    /// Published harness revision. Must equal the registered declaration's
    /// published identity revision; mismatch refuses the candidate.
    pub harness_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<RouteWorkspaceBinding>,
    /// Trust/freshness metadata for the capability declaration.
    pub trust: RouteTrust,
    /// Route-level resource budget ceiling. May only narrow; must fit the
    /// requirements hard limit when one is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budgets>,
    /// Explicit operator ordering; lower is preferred. Ties at the best
    /// priority are ambiguous and refuse (`ROUTE-0005`) unless pinned.
    pub priority: u32,
    /// Operator marks the candidate experimental. Eligible only under the
    /// experimental profile; authority and capability ceilings still apply.
    #[serde(default)]
    pub experimental: bool,
}

impl RouteCandidateV1 {
    fn validate(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.schema_version != ROUTE_CANDIDATE_SCHEMA_VERSION {
            reasons.push(format!(
                "candidate schemaVersion {:?} is not {ROUTE_CANDIDATE_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.candidate_id.trim().is_empty() {
            reasons.push("candidateId must be non-empty".to_string());
        }
        if self.provider_identity.trim().is_empty() {
            reasons.push("providerIdentity must be non-empty".to_string());
        }
        if self.harness_id.trim().is_empty() {
            reasons.push("harnessId must be non-empty".to_string());
        }
        if self.harness_revision.trim().is_empty() {
            reasons.push("harnessRevision must be non-empty".to_string());
        }
        if let Some(binding) = &self.workspace
            && binding.revision.trim().is_empty()
        {
            reasons.push("workspace binding revision must be non-empty".to_string());
        }
        if self.trust.source.trim().is_empty() {
            reasons.push("trust.source must be non-empty".to_string());
        }
        if !is_rfc3339_z(&self.trust.verified_at) {
            reasons.push(format!(
                "trust.verifiedAt {:?} is not a genuine RFC 3339 UTC-Z timestamp",
                self.trust.verified_at
            ));
        }
        if self.trust.ttl_seconds == 0 {
            reasons.push("trust.ttlSeconds must be at least 1".to_string());
        }
        if let Err(error) = validate_budgets("candidate.budget", self.budget.as_ref()) {
            reasons.push(error);
        }
        reasons
    }
}

/// One rejected candidate with the deterministic reasons it was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RejectedRouteV1 {
    pub candidate_id: String,
    pub provider_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub reasons: Vec<String>,
}

/// The selected route, recorded verbatim in evidence and snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedRouteV1 {
    pub candidate_id: String,
    pub provider_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub harness_id: String,
    pub harness_revision: String,
    pub family: String,
    pub priority: u32,
    pub experimental: bool,
}

/// Request to resolve exactly ONE route for one work item. The pin is a
/// requirement, not a hint.
pub struct RouteResolutionRequest<'a> {
    pub route_set_id: String,
    pub route_set_version: String,
    pub candidates: Vec<RouteCandidateV1>,
    pub pinned_candidate_id: Option<&'a str>,
    pub requirements: &'a WorkRequirements,
    pub authority_declared: &'a AuthorityProfile,
    pub authority_effective: &'a AuthorityProfile,
    pub profile: RouteProfile,
    /// Injected instant (RFC 3339 UTC-Z); the seam never reads a clock.
    pub evaluated_at: String,
    /// The actual execution workspace (#171), when the work or a candidate
    /// binding demands one.
    pub workspace: Option<&'a WorkspaceManifestV1>,
}

/// Sealed evidence of one route resolution. Carries identity and digest
/// material only — never raw authority envelopes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteEvidenceV1 {
    pub schema_version: String,
    pub route_set_id: String,
    pub route_set_version: String,
    pub evaluated_at: String,
    pub profile: RouteProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_candidate_id: Option<String>,
    pub selected: SelectedRouteV1,
    pub rejected: Vec<RejectedRouteV1>,
    pub requirements_digest: String,
    pub declaration_digest: String,
    pub capability_set_digest: String,
    pub decision_id: String,
    pub decision_digest: String,
    pub authority_declared_digest: String,
    pub authority_effective_digest: String,
    pub snapshot_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl RouteEvidenceV1 {
    fn try_sealed(mut self) -> anyhow::Result<Self> {
        self.content_digest = None;
        self.content_digest = Some(self.try_compute_digest()?);
        Ok(self)
    }

    fn try_compute_digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self).context("route evidence serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    /// Fail-closed re-verification of durable evidence.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::{bail, ensure};
        ensure!(
            self.schema_version == ROUTE_EVIDENCE_SCHEMA_VERSION,
            "unsupported route evidence schema version {} (supported {ROUTE_EVIDENCE_SCHEMA_VERSION})",
            self.schema_version
        );
        for (label, value) in [
            ("requirementsDigest", &self.requirements_digest),
            ("declarationDigest", &self.declaration_digest),
            ("capabilitySetDigest", &self.capability_set_digest),
            ("decisionDigest", &self.decision_digest),
            ("authorityDeclaredDigest", &self.authority_declared_digest),
            ("authorityEffectiveDigest", &self.authority_effective_digest),
            ("snapshotDigest", &self.snapshot_digest),
        ] {
            ensure!(is_hex64(value), "{label} is not a lower hex sha256 digest");
        }
        let expected = self.try_compute_digest()?;
        match &self.content_digest {
            Some(actual) if *actual == expected => Ok(()),
            _ => bail!("route evidence contentDigest does not verify"),
        }
    }
}

/// Immutable, versioned effective execution snapshot mapped to the SOMA
/// SPEC 006 `ExecutionProfile` contract. Produced only on a resolved route;
/// never produced on refusal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectiveExecutionSnapshotV1 {
    pub schema_version: String,
    pub snapshot_id: String,
    pub route_set_id: String,
    pub route_set_version: String,
    pub profile: RouteProfile,
    pub evaluated_at: String,
    pub selected: SelectedRouteV1,
    /// The SPEC 006 mapping; verified against the resolver's own invariants.
    pub effective_profile: ExecutionProfile,
    pub requirements_digest: String,
    pub declaration_digest: String,
    pub capability_set_digest: String,
    pub decision_digest: String,
    pub authority_declared_digest: String,
    pub authority_effective_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceEvidenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl EffectiveExecutionSnapshotV1 {
    fn try_sealed(mut self) -> anyhow::Result<Self> {
        self.content_digest = None;
        self.content_digest = Some(self.try_compute_digest()?);
        Ok(self)
    }

    fn try_compute_digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self).context("execution snapshot serializes")?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("contentDigest");
        }
        soma::try_canonical_digest(&value)
            .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
    }

    /// Fail-closed re-verification: digest match plus the SPEC 006 profile
    /// audit against the supported bundle.
    pub fn verify(&self) -> anyhow::Result<()> {
        use anyhow::{bail, ensure};
        ensure!(
            self.schema_version == EXECUTION_SNAPSHOT_SCHEMA_VERSION,
            "unsupported snapshot schema version {} (supported {EXECUTION_SNAPSHOT_SCHEMA_VERSION})",
            self.schema_version
        );
        let supported: SupportedVersion = soma::supported_version();
        let profile_findings = self.effective_profile.audit(&supported);
        ensure!(
            profile_findings.is_empty(),
            "effective profile does not audit clean: {}",
            profile_findings
                .iter()
                .map(|d| format!("{} {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
        let expected = self.try_compute_digest()?;
        match &self.content_digest {
            Some(actual) if *actual == expected => Ok(()),
            _ => bail!("execution snapshot contentDigest does not verify"),
        }
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn canonical_value_digest<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    let value = serde_json::to_value(value).context("canonical digest input serializes")?;
    soma::try_canonical_digest(&value)
        .map_err(|error| anyhow::anyhow!("canonical digest failed: {error}"))
}

/// A resolved route: the immutable effective execution snapshot plus the
/// sealed route evidence. No adapter is invoked; this is preflight only.
#[derive(Debug)]
pub struct RouteResolution {
    pub snapshot: EffectiveExecutionSnapshotV1,
    pub evidence: RouteEvidenceV1,
}

struct EvaluatedCandidate {
    candidate: RouteCandidateV1,
    declaration: crate::workflow::execution_harness::HarnessDeclarationV1,
    declared_capabilities: Vec<String>,
    effective_capabilities: Vec<String>,
    decision: crate::workflow::soma::capability::CompatibilityDecision,
    declaration_digest: String,
    capability_set_digest: String,
}

fn validate_trust(trust: &RouteTrust, evaluated_at: &str) -> Result<(), String> {
    let verified = chrono::NaiveDateTime::parse_from_str(&trust.verified_at, "%Y-%m-%dT%H:%M:%SZ")
        .map_err(|_| format!("trust.verifiedAt {:?} is not parseable", trust.verified_at))?;
    let evaluated = chrono::NaiveDateTime::parse_from_str(evaluated_at, "%Y-%m-%dT%H:%M:%SZ")
        .map_err(|_| format!("evaluatedAt {evaluated_at:?} is not parseable"))?;
    // Fail closed on future-dated verification: metadata "verified" after the
    // evaluation instant could not have been observed yet.
    if verified > evaluated {
        return Err(format!(
            "trust.verifiedAt {} is later than evaluatedAt {}; refusing future-dated verification",
            trust.verified_at, evaluated_at
        ));
    }
    // Checked conversion: an arbitrary u64 TTL is never silently wrapped into
    // a signed duration.
    let ttl_seconds = i64::try_from(trust.ttl_seconds).map_err(|_| {
        format!(
            "trust.ttlSeconds {} exceeds the representable duration",
            trust.ttl_seconds
        )
    })?;
    let duration = chrono::Duration::try_seconds(ttl_seconds).ok_or_else(|| {
        format!(
            "trust.ttlSeconds {} exceeds the representable chrono duration",
            trust.ttl_seconds
        )
    })?;
    let Some(expiry) = verified.checked_add_signed(duration) else {
        return Err(format!(
            "trust.verifiedAt {} + trust.ttlSeconds {} overflows the representable instant",
            trust.verified_at, trust.ttl_seconds
        ));
    };
    if expiry <= evaluated {
        return Err(format!(
            "capability metadata stale: trust.verifiedAt {} + ttlSeconds {} expires at or before evaluatedAt {}",
            trust.verified_at, trust.ttl_seconds, evaluated_at
        ));
    }
    Ok(())
}

/// Compute the per-dimension strict intersection (minimum) across all
/// applicable budget ceilings. An omitted dimension in one source does NOT
/// erase a ceiling supplied by another; incomparable lexemes fail closed.
fn intersect_budgets(label: &str, sources: &[&Budgets]) -> Result<Option<Budgets>, String> {
    let mut tokens = None;
    let mut cost = None;
    let mut duration = None;
    let mut retries = None;
    let mut concurrency = None;
    for dim in Budgets::DIMENSIONS {
        let mut min: Option<serde_json::Number> = None;
        for source in sources {
            let Some(value) = source.get(dim) else {
                continue;
            };
            match &min {
                None => min = Some(value.clone()),
                Some(current) => match crate::workflow::soma::types::number_cmp(value, current) {
                    crate::workflow::soma::types::Cmp::Greater => {}
                    crate::workflow::soma::types::Cmp::NotGreater => min = Some(value.clone()),
                    crate::workflow::soma::types::Cmp::Incomparable => {
                        return Err(format!(
                            "{label} {dim} ceiling is outside the number policy"
                        ));
                    }
                },
            }
        }
        match dim {
            "tokens" => tokens = min,
            "cost" => cost = min,
            "duration" => duration = min,
            "retries" => retries = min,
            "concurrency" => concurrency = min,
            _ => unreachable!("Budgets::DIMENSIONS is exhaustive"),
        }
    }
    if [&tokens, &cost, &duration, &retries, &concurrency]
        .iter()
        .any(|dim| dim.is_some())
    {
        Ok(Some(Budgets {
            tokens,
            cost,
            duration,
            retries,
            concurrency,
        }))
    } else {
        Ok(None)
    }
}

fn check_budget_ceiling(label: &str, effective: &Budgets, ceiling: &Budgets) -> Vec<String> {
    let mut reasons = Vec::new();
    for dim in Budgets::DIMENSIONS {
        let Some(eff) = effective.get(dim) else {
            continue;
        };
        let Some(limit) = ceiling.get(dim) else {
            continue;
        };
        match crate::workflow::soma::types::number_cmp(eff, limit) {
            crate::workflow::soma::types::Cmp::Greater => {
                reasons.push(format!(
                    "{label} {dim} exceeds the ceiling ({eff} > {limit})"
                ));
            }
            crate::workflow::soma::types::Cmp::Incomparable => {
                reasons.push(format!("{label} {dim} is outside the number policy"));
            }
            crate::workflow::soma::types::Cmp::NotGreater => {}
        }
    }
    reasons
}

#[allow(clippy::too_many_lines)]
fn evaluate_candidate(
    candidate: &RouteCandidateV1,
    request: &RouteResolutionRequest<'_>,
    registry: &HarnessRegistry,
) -> Result<EvaluatedCandidate, Vec<String>> {
    use crate::workflow::soma::capability::Locality;
    let mut reasons = candidate.validate();

    // --- Registered harness lookup (the declaration is the source of truth).
    let harness = match registry.get(&candidate.harness_id) {
        Ok(harness) => harness,
        Err(_) => {
            reasons.push(format!(
                "harness {:?} is not registered in the route set's registry",
                candidate.harness_id
            ));
            return Err(reasons);
        }
    };
    let declaration = harness.declaration();
    if declaration.identity.revision != candidate.harness_revision {
        reasons.push(format!(
            "harness {:?} published revision {:?} does not match candidate revision {:?}",
            declaration.harness_id, declaration.identity.revision, candidate.harness_revision
        ));
    }

    // --- Profile gating: ordered constraints, never grants.
    if candidate.experimental && request.profile != RouteProfile::Experimental {
        reasons.push(format!(
            "candidate {:?} is operator-flagged experimental; requires the experimental profile",
            candidate.candidate_id
        ));
    }
    if request.profile == RouteProfile::Conservative
        && candidate.provider_locality != Locality::Local
    {
        reasons.push(format!(
            "profile conservative restricts routes to local providers; candidate locality {:?}",
            candidate.provider_locality
        ));
    }

    // --- Work-contract locality + privacy (intersection, never union).
    if let Some(locality) = &request.requirements.locality {
        if let Some(allowed) = &locality.allowed_execution {
            if !allowed.contains(&candidate.provider_locality) {
                reasons.push(format!(
                    "candidate provider locality {:?} is not permitted by work locality constraints",
                    candidate.provider_locality
                ));
            }
            if let Some(declaration_locality) = declaration.capabilities.locality
                && !allowed.contains(&declaration_locality)
            {
                reasons.push(format!(
                    "harness {:?} declares locality {:?} which the work locality constraints refuse",
                    declaration.harness_id, declaration_locality
                ));
            }
        }
        if let Some(privacy) = &locality.privacy {
            for required in privacy {
                if !declaration.capabilities.privacy_classes.contains(required) {
                    reasons.push(format!(
                        "harness {:?} does not claim required privacy class {:?}",
                        declaration.harness_id, required
                    ));
                }
            }
        }
    }

    // --- Trust/freshness (injected instant, lexicographic-safe UTC-Z).
    if let Err(reason) = validate_trust(&candidate.trust, &request.evaluated_at) {
        reasons.push(reason);
    }

    // --- Budget ceilings: candidate may only narrow; every dimension it sets
    // must be verifiable against at least one declared ceiling (authority,
    // work hardLimits, or runtime capability ceilings).
    if let Some(candidate_budget) = &candidate.budget {
        if let Some(hard_limits) = &request.requirements.hard_limits {
            reasons.extend(check_budget_ceiling(
                "candidate budget",
                candidate_budget,
                hard_limits,
            ));
        }
        for dim in Budgets::DIMENSIONS {
            if candidate_budget.get(dim).is_none() {
                continue;
            }
            let bounded = request
                .authority_declared
                .budgets
                .as_ref()
                .and_then(|b| b.get(dim))
                .is_some()
                || request
                    .requirements
                    .hard_limits
                    .as_ref()
                    .and_then(|b| b.get(dim))
                    .is_some()
                || declaration
                    .capabilities
                    .budget_ceilings
                    .as_ref()
                    .and_then(|b| b.get(dim))
                    .is_some();
            if !bounded {
                reasons.push(format!(
                    "candidate budget dimension {dim} is unverifiable: no declared ceiling (authority budget, hardLimits, or runtime) bounds it"
                ));
            }
        }
    }

    // --- Workspace binding (exact adapter kind + revision match).
    if let Some(binding) = &candidate.workspace {
        match request.workspace {
            None => reasons.push(format!(
                "candidate binds workspace adapter {:?} / revision {:?} but no workspace manifest was provided",
                binding.adapter, binding.revision
            )),
            Some(manifest) => {
                if manifest.adapter != binding.adapter || manifest.adapter_revision != binding.revision {
                    reasons.push(format!(
                        "workspace adapter {:?} / revision {:?} does not match candidate binding {:?} / {:?}",
                        manifest.adapter, manifest.adapter_revision, binding.adapter, binding.revision
                    ));
                }
            }
        }
    }

    // --- SPEC 007 negotiation over the published declaration. Fail-closed:
    // anything other than `Compatible` refuses the candidate with diagnostics.
    let decision = resolve(&NegotiationInputs {
        id: format!("{}/{}", request.route_set_id, candidate.candidate_id),
        requirements: request.requirements,
        capabilities: &declaration.capabilities,
        authority_declared: request.authority_declared,
        authority_effective: request.authority_effective,
        evaluated_at: request.evaluated_at.clone(),
        substitutions: Vec::new(),
    });
    let decision = match decision {
        Ok(decision) if decision.status == DecisionStatus::Compatible => decision,
        Ok(decision) => {
            let detail = decision
                .diagnostics
                .iter()
                .map(|d| format!("{} {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; ");
            reasons.push(format!(
                "SPEC 007 negotiation status {:?}: {detail}",
                decision.status
            ));
            return Err(reasons);
        }
        Err(error) => {
            reasons.push(format!("SPEC 007 negotiation failed closed: {error}"));
            return Err(reasons);
        }
    };

    if !reasons.is_empty() {
        return Err(reasons);
    }

    let declaration_digest = declaration
        .try_compute_digest()
        .map_err(|error| vec![format!("declaration digest failed: {error}")])?;
    let capability_set_digest = declaration
        .capabilities
        .canonical_digest()
        .map_err(|error| vec![format!("capability set digest failed: {error}")])?;

    Ok(EvaluatedCandidate {
        candidate: candidate.clone(),
        declaration: declaration.clone(),
        declared_capabilities: declaration.declared_capabilities.clone(),
        effective_capabilities: declaration.claimed_capabilities.clone(),
        decision,
        declaration_digest,
        capability_set_digest,
    })
}

/// Resolve exactly one route for one work item. Hard refusals return an
/// error carrying the deterministic per-candidate reasons; a refusal never
/// emits a snapshot and never falls back to a replacement candidate.
pub fn resolve_route(
    request: &RouteResolutionRequest<'_>,
    registry: &HarnessRegistry,
) -> anyhow::Result<RouteResolution> {
    // ---- Request-level preconditions (caller bugs, not candidate faults).
    if request.route_set_id.trim().is_empty() {
        bail!("ROUTE-0006: route set id must be non-empty");
    }
    if !is_semver_shaped(&request.route_set_version) {
        bail!(
            "ROUTE-0006: route set version {:?} violates the ^\\d+\\.\\d+\\.\\d+$ version pattern",
            request.route_set_version
        );
    }
    if !is_rfc3339_z(&request.evaluated_at) {
        bail!(
            "ROUTE-0006: evaluatedAt {:?} is not a genuine RFC 3339 UTC-Z timestamp",
            request.evaluated_at
        );
    }
    if request.candidates.is_empty() {
        bail!("ROUTE-0001: the route candidate set is empty");
    }
    let mut seen = HashSet::with_capacity(request.candidates.len());
    for candidate in &request.candidates {
        if !seen.insert(candidate.candidate_id.as_str()) {
            bail!(
                "ROUTE-0002: duplicate candidate id {:?} in the route candidate set",
                candidate.candidate_id
            );
        }
    }
    let requirements_digest = request
        .requirements
        .canonical_digest()
        .map_err(|error| anyhow::anyhow!("ROUTE-0006: requirements digest failed: {error}"))?;

    // ---- Workspace precondition: demands a manifest, or refuses.
    let requires_workspace = request.requirements.workspace.is_some();
    let workspace_manifest = match request.workspace {
        Some(manifest) => {
            match validate_workspace_manifest(manifest) {
                Ok(()) => {}
                Err(WorkspaceManifestDefect::UnsupportedSchemaVersion(version)) => {
                    bail!("ROUTE-0006: unsupported workspace schema version {version}");
                }
                Err(WorkspaceManifestDefect::DigestMismatch) => {
                    bail!("ROUTE-0006: workspace manifest contentDigest does not verify");
                }
                Err(WorkspaceManifestDefect::IntegrityCheckFailed(error)) => {
                    bail!("ROUTE-0006: workspace manifest integrity check failed: {error}");
                }
            }
            if !workspace_mode_satisfies(manifest.mode, request.requirements) {
                bail!(
                    "ROUTE-0006: workspace mode ({:?}) cannot satisfy the required workspace boundary",
                    manifest.mode
                );
            }
            Some(manifest)
        }
        None if requires_workspace => {
            bail!("ROUTE-0006: work requirements demand a workspace but none was provided")
        }
        None => None,
    };

    // ---- Per-candidate evaluation.
    let mut eligible: Vec<EvaluatedCandidate> = Vec::new();
    let mut rejected: Vec<RejectedRouteV1> = Vec::new();
    for candidate in &request.candidates {
        let reject = |reasons: Vec<String>| RejectedRouteV1 {
            candidate_id: candidate.candidate_id.clone(),
            provider_identity: candidate.provider_identity.clone(),
            model: candidate.model.clone(),
            reasons,
        };
        match evaluate_candidate(candidate, request, registry) {
            Ok(evaluated) => eligible.push(evaluated),
            Err(reasons) => rejected.push(reject(reasons)),
        }
    }

    // ---- Deterministic selection: stable sort by explicit priority.
    eligible.sort_by_key(|entry| entry.candidate.priority);
    let reasons_of = |rejected: &[RejectedRouteV1]| {
        rejected
            .iter()
            .map(|entry| format!("{}: {}", entry.candidate_id, entry.reasons.join(" | ")))
            .collect::<Vec<_>>()
            .join(" ; ")
    };
    let selection_index: usize = match request.pinned_candidate_id {
        Some(pin) => match eligible
            .iter()
            .position(|entry| entry.candidate.candidate_id == pin)
        {
            Some(index) => index,
            None => {
                let detail = reasons_of(&rejected);
                bail!(
                    "ROUTE-0004: pinned candidate {pin:?} is unknown or ineligible; refusing without fallback ({detail})"
                );
            }
        },
        None => match eligible.first() {
            None => {
                let detail = reasons_of(&rejected);
                bail!(
                    "ROUTE-0003: no candidate is eligible for route set {:?}; refusing without fallback ({detail})",
                    request.route_set_id
                );
            }
            Some(best) => {
                let best_priority = best.candidate.priority;
                let tied: Vec<&str> = eligible
                    .iter()
                    .take_while(|entry| entry.candidate.priority == best_priority)
                    .map(|entry| entry.candidate.candidate_id.as_str())
                    .collect();
                if tied.len() > 1 {
                    bail!(
                        "ROUTE-0005: route resolution is ambiguous between {}; pin exactly one candidate",
                        tied.join(", ")
                    );
                }
                0
            }
        },
    };
    let selected = eligible.remove(selection_index);
    let best_priority = selected.candidate.priority;

    // Eligible candidates that were not selected are evidence too: record
    // them with the deterministic reason they lost, never silently drop them.
    for entry in eligible {
        let reason = match request.pinned_candidate_id {
            Some(pin) => format!("eligible but not selected: route resolved by pin to {pin}"),
            None => format!(
                "eligible but not selected: priority {} > best eligible priority {}",
                entry.candidate.priority, best_priority
            ),
        };
        rejected.push(RejectedRouteV1 {
            candidate_id: entry.candidate.candidate_id.clone(),
            provider_identity: entry.candidate.provider_identity.clone(),
            model: entry.candidate.model.clone(),
            reasons: vec![reason],
        });
    }

    // ---- Build the immutable effective execution snapshot (SPEC 006).
    //
    // Per-dimension strict intersection across ALL applicable ceilings. The
    // effective ceiling never exceeds any of them, and an omitted dimension
    // in one source never erases a ceiling supplied by another.
    //
    // Declared side: ceilings the calling contracts publish — declared
    // authority envelope, work hard limits, and the runtime capability
    // ceilings claimed by the harness (the SPEC 006 declared contract).
    let snapshot_id = format!(
        "{}/{}",
        request.route_set_id, selected.candidate.candidate_id
    );
    let authority_declared_digest = canonical_value_digest(request.authority_declared)?;
    let authority_effective_digest = canonical_value_digest(request.authority_effective)?;
    let decision_digest = canonical_value_digest(&selected.decision)?;
    let hard_limits = request.requirements.hard_limits.clone();
    let runtime_ceilings = selected.declaration.capabilities.budget_ceilings.clone();
    let declared_sources: Vec<&Budgets> = [
        &request.authority_declared.budgets,
        &hard_limits,
        &runtime_ceilings,
    ]
    .into_iter()
    .flatten()
    .collect();
    let declared_budgets = intersect_budgets("declared budget", &declared_sources)
        .map_err(|error| anyhow::anyhow!("ROUTE-0006: {error}"))?;
    // Effective side: never above the declared intersection, the effective
    // authority grant, or the candidate ceiling (the actual narrowing).
    let effective_sources: Vec<&Budgets> = [
        &declared_budgets,
        &request.authority_effective.budgets,
        &selected.candidate.budget,
    ]
    .into_iter()
    .flatten()
    .collect();
    let effective_budgets = intersect_budgets("effective budget", &effective_sources)
        .map_err(|error| anyhow::anyhow!("ROUTE-0006: {error}"))?;

    let effective_profile = ExecutionProfile {
        id: snapshot_id.clone(),
        schema_version: soma::SUPPORTED_SCHEMA_VERSION.to_string(),
        version: soma::SUPPORTED_SCHEMA_VERSION.to_string(),
        harness: selected.declaration.harness_id.clone(),
        model: selected.candidate.model.clone(),
        provider: Some(selected.candidate.provider_identity.clone()),
        declared: ProfileSide {
            authority: request.authority_declared.clone(),
            boundaries: request.requirements.workspace,
            budgets: declared_budgets,
            capabilities: selected.declared_capabilities.clone(),
        },
        effective: ProfileSide {
            authority: request.authority_effective.clone(),
            boundaries: request.requirements.workspace,
            budgets: effective_budgets,
            capabilities: selected.effective_capabilities.clone(),
        },
        units: Vec::new(),
        limits: request.requirements.hard_limits.clone(),
        approval_rules: Vec::new(),
        evidence: None,
        redaction: None,
        retention: None,
        validation: Some(ValidationPolicy {
            require_evidence: request
                .requirements
                .evidence_obligations
                .as_ref()
                .and_then(|obligations| obligations.require_evidence),
            fail_closed: Some(true),
        }),
    };
    let supported: SupportedVersion = soma::supported_version();
    let profile_findings = effective_profile.audit(&supported);
    if let Some(first) = profile_findings.first() {
        bail!(
            "ROUTE-0006: effective execution profile does not audit clean: {} {}",
            first.code,
            first.message
        );
    }

    let selected_route = SelectedRouteV1 {
        candidate_id: selected.candidate.candidate_id.clone(),
        provider_identity: selected.candidate.provider_identity.clone(),
        model: selected.candidate.model.clone(),
        harness_id: selected.declaration.harness_id.clone(),
        harness_revision: selected.declaration.identity.revision.clone(),
        family: selected.declaration.family.as_str().to_string(),
        priority: selected.candidate.priority,
        experimental: selected.candidate.experimental,
    };
    let workspace_evidence = workspace_manifest.map(|manifest| WorkspaceEvidenceV1 {
        workspace_id: manifest.workspace_id.clone(),
        adapter: workspace_adapter_family(manifest.adapter).to_string(),
        adapter_revision: manifest.adapter_revision.clone(),
        mode: match manifest.mode {
            WorkspaceMode::ReadOnly => "read",
            WorkspaceMode::Writable => "write",
        }
        .to_string(),
        manifest_digest: manifest.compute_digest(),
    });

    let snapshot = EffectiveExecutionSnapshotV1 {
        schema_version: EXECUTION_SNAPSHOT_SCHEMA_VERSION.to_string(),
        snapshot_id,
        route_set_id: request.route_set_id.clone(),
        route_set_version: request.route_set_version.clone(),
        profile: request.profile,
        evaluated_at: request.evaluated_at.clone(),
        selected: selected_route.clone(),
        effective_profile,
        requirements_digest: requirements_digest.clone(),
        declaration_digest: selected.declaration_digest.clone(),
        capability_set_digest: selected.capability_set_digest.clone(),
        decision_digest: decision_digest.clone(),
        authority_declared_digest: authority_declared_digest.clone(),
        authority_effective_digest: authority_effective_digest.clone(),
        workspace: workspace_evidence,
        content_digest: None,
    }
    .try_sealed()?;

    let evidence = RouteEvidenceV1 {
        schema_version: ROUTE_EVIDENCE_SCHEMA_VERSION.to_string(),
        route_set_id: request.route_set_id.clone(),
        route_set_version: request.route_set_version.clone(),
        evaluated_at: request.evaluated_at.clone(),
        profile: request.profile,
        pinned_candidate_id: request.pinned_candidate_id.map(str::to_string),
        selected: selected_route,
        rejected,
        requirements_digest,
        declaration_digest: selected.declaration_digest,
        capability_set_digest: selected.capability_set_digest,
        decision_id: selected.decision.id.clone(),
        decision_digest,
        authority_declared_digest,
        authority_effective_digest,
        snapshot_digest: snapshot.try_compute_digest()?,
        content_digest: None,
    }
    .try_sealed()?;

    Ok(RouteResolution { snapshot, evidence })
}
