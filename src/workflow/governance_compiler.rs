//! Versioned Lite → SOMA governance compiler (issue #163, Slice 1).
//!
//! SOMA is the semantic source of truth: this module validates a
//! `WorkflowDefinition` against the vendored v1.1 bundle BEFORE any plan
//! exists (compile-before-expose), maps Lite's runtime authority snapshot
//! into the SOMA authority vocabulary (disclosing anything that vocabulary
//! cannot express instead of inventing rules), and seals the resulting
//! execution plan so a later execution gate can refuse plans whose
//! identity does not match the reviewed record (digest binding).
//!
//! Out of scope for this slice (follow-up work, stated in the change
//! doc): wiring the compiled plan into `node_runner`, provider execution,
//! and UI.
//!
//! Ground truth: `vendored/soma/v1.1/` (schemas, diagnostics catalogue,
//! fixtures) — read-only.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::workflow::AuthorityLevel;
use crate::workflow::execution_graph::topological_order;
use crate::workflow::policy::EffectiveExecutionSnapshotV1;
use crate::workflow::soma::contracts::{AuthorityProfile, EscalationPolicy, WorkflowDefinition};
use crate::workflow::soma::types::{ExecutionClass, Hex64, MutationMode, SemVer};
use crate::workflow::soma::{
    Diagnostic, DiagnosticRemediation, DiagnosticSource, SUPPORTED_SCHEMA_VERSION, canonical,
    try_canonical_digest, validate_artifact_text,
};

/// Identity of the Lite → SOMA authority mapping (contract req1).
pub const MAPPING_VERSION: &str = "lite-to-soma-v1";

/// Version of the compiled governance plan format produced by
/// [`compile_workflow_text`].
pub const GOV_PLAN_VERSION: &str = "1.0.0";

/// Lite-enforced runtime restrictions that the vendored SOMA authority
/// vocabulary cannot express. They are disclosed on every mapping —
/// never silently dropped. See the change doc for why each one is
/// Lite-enforced only (e.g. `AuthorityProfile.budgets` uses a different
/// budget shape than Lite's snapshot, and `providerPolicy` has an
/// allowlist but no denylist).
pub const LITE_ENFORCED_ONLY: [&str; 4] = [
    "deniedProviders",
    "forbiddenPaths",
    "maxAttempts",
    "tokenBudget",
];

/// The versioned result of mapping a Lite authority snapshot into the
/// SOMA authority vocabulary (contract req1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiteToSomaMappingV1 {
    /// [`MAPPING_VERSION`] at the time of mapping.
    pub version: String,
    /// The SOMA-authoritative profile built from the snapshot.
    pub authority: AuthorityProfile,
    /// Lite-enforced restrictions with no SOMA counterpart.
    pub lite_enforced_only: Vec<String>,
}

/// Map one Lite authority level plus the immutable effective execution
/// snapshot into the SOMA authority vocabulary.
///
/// Rules (no invented semantics):
/// - `executionClass` is passed through from the caller (SOMA vocabulary;
///   Lite declares no default here).
/// - `mutation` is `explicit` only when the level may actually apply
///   (`Assist`/`Execute`), otherwise `none`.
/// - readable/writable scopes come from the effective snapshot.
/// - `escalation` maps only when the snapshot names a target.
/// - Restrictions SOMA cannot express are listed in `liteEnforcedOnly`.
pub fn map_lite_authority(
    level: AuthorityLevel,
    snapshot: &EffectiveExecutionSnapshotV1,
    execution_class: ExecutionClass,
) -> LiteToSomaMappingV1 {
    let mutation = if level.can_apply() {
        MutationMode::Explicit
    } else {
        MutationMode::None_
    };
    let escalation = if snapshot.escalation_target.is_empty() {
        None
    } else {
        Some(EscalationPolicy {
            to: snapshot.escalation_target.clone(),
        })
    };
    LiteToSomaMappingV1 {
        version: MAPPING_VERSION.to_string(),
        authority: AuthorityProfile {
            execution_class,
            mutation,
            readable_scopes: Some(snapshot.readable_scopes.clone()),
            writable_scopes: Some(snapshot.writable_scopes.clone()),
            tools: None,
            network_policy: None,
            provider_policy: None,
            secrets: None,
            escalation,
            review: None,
            abstention: None,
            budgets: None,
            content_restrictions: None,
        },
        lite_enforced_only: LITE_ENFORCED_ONLY
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    }
}

/// One step of a compiled governance plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStep {
    /// Published v1.1 step key: `s{index:04}:{operationId}` over the
    /// workflow's topological order — unique within the plan.
    pub key: String,
    /// The workflow body operation this step executes.
    pub operation_id: String,
}

/// Self-sealing canonicalization block of a compiled plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Canonicalization {
    pub version: String,
    pub sha256: String,
}

/// A compiled, sealed governance plan (SOMA `ExecutionPlan` shape).
///
/// The plan only exists after [`compile_workflow_text`] accepts the
/// workflow: there is no plan for a workflow that fails validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledGovernancePlanV1 {
    pub schema_version: String,
    pub plan_version: String,
    /// Canonical digest of the workflow with `contentDigest` removed —
    /// the same rule the audit uses for SOMA-CMP-0004.
    pub workflow_digest: String,
    pub steps: Vec<PlanStep>,
    pub canonicalization: Canonicalization,
}

/// Compile workflow text into a sealed governance plan (contract req2:
/// validate before any plan exists).
///
/// - Input refusals (malformed JSON, schema violations, number-policy
///   violations) surface as SOMA-CMP-0003 with the refusal reason.
/// - Audit diagnostics (any non-empty result) are returned as errors:
///   no plan is produced.
/// - On success the returned plan is deterministic for identical input.
pub fn compile_workflow_text(text: &str) -> Result<CompiledGovernancePlanV1, Vec<Diagnostic>> {
    match validate_artifact_text("WorkflowDefinition", text) {
        Err(reason) => {
            return Err(vec![enrich_one(
                input_refusal(reason),
                "",
                workflow_id_of(text).as_deref(),
            )]);
        }
        Ok(diags) if !diags.is_empty() => return Err(enrich_all(diags, text)),
        Ok(_) => {}
    }

    // Validation passed: re-parsing cannot fail, but stay fail-closed.
    let model: WorkflowDefinition = serde_json::from_str(text)
        .map_err(|e| vec![input_refusal(format!("schema violation: {e}"))])?;

    let workflow_digest = workflow_digest_of(&model)?;
    // Steps follow the published v1.1 topological ordering; a cycle here
    // means the audit gate was bypassed, so fail closed without a plan.
    let Some(order) = topological_order(&model) else {
        return Err(vec![Diagnostic::new(
            "SOMA-EXP-0002",
            "workflow body has an operation-edge cycle; the plan cannot be sealed".to_string(),
        )]);
    };
    let steps = order
        .iter()
        .enumerate()
        .map(|(i, &body_idx)| PlanStep {
            key: format!("s{i:04}:{}", model.body[body_idx].id),
            operation_id: model.body[body_idx].id.clone(),
        })
        .collect();
    seal_plan(workflow_digest, steps)
}

/// Canonical digest of the serialized workflow with `contentDigest`
/// removed (the audit's SOMA-CMP-0004 rule). `pub(crate)` so the Lite
/// execution graph binds to the same digest without re-deriving it.
pub(crate) fn workflow_digest_of(model: &WorkflowDefinition) -> Result<String, Vec<Diagnostic>> {
    let mut value = serde_json::to_value(model).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("workflow cannot be serialized for digest verification ({e})"),
        )]
    })?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("contentDigest");
    }
    try_canonical_digest(&value).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("workflow digest cannot be recomputed ({e})"),
        )]
    })
}

/// Build the plan and seal it: `canonicalization.sha256` is the
/// canonical digest of the plan with only `canonicalization.sha256`
/// removed — `canonicalization.version` stays inside the digest input
/// (the published v1.1 sealing rule; version equals the artifact's
/// `schemaVersion`).
fn seal_plan(
    workflow_digest: String,
    steps: Vec<PlanStep>,
) -> Result<CompiledGovernancePlanV1, Vec<Diagnostic>> {
    let mut map = serde_json::Map::new();
    map.insert("schemaVersion".into(), json!(SUPPORTED_SCHEMA_VERSION));
    map.insert("planVersion".into(), json!(GOV_PLAN_VERSION));
    map.insert("workflowDigest".into(), json!(workflow_digest));
    let steps_value = serde_json::to_value(&steps).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("plan steps cannot be serialized ({e})"),
        )]
    })?;
    map.insert("steps".into(), steps_value);

    // Digest input: the full plan including canonicalization.version but
    // without the sha256 member (self-reference).
    let mut canonicalization = serde_json::Map::new();
    canonicalization.insert("version".into(), json!(SUPPORTED_SCHEMA_VERSION));
    map.insert(
        "canonicalization".into(),
        Value::Object(canonicalization.clone()),
    );
    let seal = try_canonical_digest(&Value::Object(map.clone())).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("plan digest cannot be recomputed ({e})"),
        )]
    })?;
    canonicalization.insert("sha256".into(), json!(seal));
    map.insert(
        "canonicalization".into(),
        Value::Object(canonicalization),
    );

    serde_json::from_value(Value::Object(map)).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0003",
            format!("compiled plan violates its declared schema ({e})"),
        )]
    })
}

/// A parse/schema-level input refusal, mapped to the published code
/// (per `validate_artifact_text`'s contract: callers map `Err` to
/// SOMA-CMP-0003).
fn input_refusal(reason: String) -> Diagnostic {
    Diagnostic::new("SOMA-CMP-0003", reason)
}

/// The shared strict boundary for SOMA `ExecutionPlan` text.
///
/// Every production path that ingests plan text runs this function first
/// (review blocker 4): [`verify_reviewed_plan`] step 1 and the
/// `ExecutionPlan` branch of [`validate_artifact_text`]. Checks, in order:
///
/// 1. Duplicate object members on the raw text (the DOM path would
///    silently keep the last duplicate) — SOMA-CMP-0007
///    (`duplicate_key`, per the vendored catalogue).
/// 2. JSON parse plus typed parse against [`CompiledGovernancePlanV1`]
///    (`deny_unknown_fields` refuses unknown members anywhere) —
///    SOMA-CMP-0003 (`schema_violation`).
/// 3. Strict `MAJOR.MINOR.PATCH` versions for `schemaVersion`,
///    `planVersion` and `canonicalization.version`, schema/plan major
///    exactly 1, and `canonicalization.version == schemaVersion` —
///    SOMA-CMP-0001 (`unsupported_version`). The parse is the reference
///    `SemVer::parse` rule (no leading zeros, no partials, no
///    prerelease/build tags): stricter than the schema regex, matching
///    exactly what the oracle's typed deserialization accepts.
/// 4. `workflowDigest` and `canonicalization.sha256` as 64 lowercase hex
///    chars ([`Hex64`]) — SOMA-CMP-0003.
/// 5. Unique `steps[].key` values (the schema documents keys as unique
///    within the plan) — SOMA-CMP-0003.
///
/// Returns the parsed document so callers keep the exact input shape.
// Diagnostic carries optional source/remediation members, which pushes
// Result<Value, Diagnostic> past clippy's large-err threshold. The Err
// value is returned by value only on the refusal path; boxing would
// churn every caller for no size win (same rationale as
// governance.rs::selection_within_authority).
#[allow(clippy::result_large_err)]
pub fn validate_execution_plan_text(text: &str) -> Result<Value, Diagnostic> {
    match canonical::find_duplicate_key(text.as_bytes()) {
        Err(_) => {
            return Err(Diagnostic::new("SOMA-CMP-0003", "malformed json"));
        }
        Ok(Some(_)) => {
            return Err(Diagnostic::new("SOMA-CMP-0007", "duplicate object key"));
        }
        Ok(None) => {}
    }
    let raw: Value = serde_json::from_str(text)
        .map_err(|e| Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}")))?;
    let plan: CompiledGovernancePlanV1 = serde_json::from_value(raw.clone())
        .map_err(|e| Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}")))?;

    let version_error = || {
        Diagnostic::new(
            "SOMA-CMP-0001",
            format!(
                "unsupported governance plan version (schemaVersion={}, planVersion={}, canonicalizationVersion={})",
                plan.schema_version, plan.plan_version, plan.canonicalization.version
            ),
        )
    };
    let schema_parsed = SemVer::parse(&plan.schema_version);
    let plan_parsed = SemVer::parse(&plan.plan_version);
    let canon_parsed = SemVer::parse(&plan.canonicalization.version);
    if schema_parsed.is_err() || plan_parsed.is_err() || canon_parsed.is_err() {
        // First failing member, in schema order, names the pointer.
        let pointer = if schema_parsed.is_err() {
            "/schemaVersion"
        } else if plan_parsed.is_err() {
            "/planVersion"
        } else {
            "/canonicalization/version"
        };
        return Err(version_error().with_source(pointer, None));
    }
    let schema_major = schema_parsed.expect("schema version parsed");
    let plan_major = plan_parsed.expect("plan version parsed");
    if schema_major.major != 1 {
        return Err(version_error().with_source("/schemaVersion", None));
    }
    if plan_major.major != 1 {
        return Err(version_error().with_source("/planVersion", None));
    }
    if plan.canonicalization.version != plan.schema_version {
        return Err(version_error().with_source("/canonicalization/version", None));
    }
    if let Err(e) = Hex64::parse(&plan.workflow_digest) {
        return Err(
            Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}"))
                .with_source("/workflowDigest", None),
        );
    }
    if let Err(e) = Hex64::parse(&plan.canonicalization.sha256) {
        return Err(
            Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}"))
                .with_source("/canonicalization/sha256", None),
        );
    }
    let mut step_keys = std::collections::HashSet::new();
    for (i, step) in plan.steps.iter().enumerate() {
        if !step_keys.insert(step.key.as_str()) {
            return Err(Diagnostic::new(
                "SOMA-CMP-0003",
                "schema violation: duplicate step key",
            )
            .with_source(format!("/steps/{i}"), None));
        }
    }
    Ok(raw)
}

/// Digest-binding check (contract req5): refuse a plan whose compiled
/// identity differs from the reviewed plan.
///
/// Checks, in order (fail-closed, first failure wins):
/// 1. The text passes [`validate_execution_plan_text`] — the shared strict
///    boundary (parse, strict versions, lowercase-hex digests, unique step
///    keys; else SOMA-CMP-0007/SOMA-CMP-0003/SOMA-CMP-0001).
/// 2. The plan's self-seal verifies — digest input removes only
///    `canonicalization.sha256` (else SOMA-CMP-0004, integrity).
/// 3. The plan's canonical digest equals `reviewed_identity` — the
///    identity recorded by review — so execution can never run a plan
///    other than the one reviewed (else SOMA-CMP-0004, binding).
///
/// `reviewed_identity` is the plan's `canonicalization.sha256` as it was
/// captured at review time (it commits to every plan member).
pub fn verify_reviewed_plan(
    plan_text: &str,
    reviewed_identity: &str,
) -> Result<(), Vec<Diagnostic>> {
    let plan: CompiledGovernancePlanV1 = match validate_execution_plan_text(plan_text) {
        Err(d) => {
            return Err(vec![enrich_one(d, "", plan_digest_hint(plan_text).as_deref())]);
        }
        Ok(value) => match serde_json::from_value(value) {
            Ok(plan) => plan,
            Err(e) => {
                return Err(vec![enrich_one(
                    Diagnostic::new("SOMA-CMP-0003", format!("schema violation: {e}")),
                    "",
                    plan_digest_hint(plan_text).as_deref(),
                )]);
            }
        },
    };
    let ctx = Some(plan.workflow_digest.as_str());

    // 2. Self-seal integrity.
    let value = serde_json::to_value(&plan).map_err(|e| {
        vec![Diagnostic::new(
            "SOMA-CMP-0004",
            format!("plan cannot be re-serialized for seal verification ({e})"),
        )]
    })?;
    let seal_input = value.as_object().cloned().map(|mut obj| {
        if let Some(Value::Object(canonicalization)) = obj.get_mut("canonicalization") {
            canonicalization.remove("sha256");
        }
        Value::Object(obj)
    });
    let computed = seal_input
        .as_ref()
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "SOMA-CMP-0004",
                "plan is not an object".to_string(),
            )]
        })
        .and_then(|v| {
            try_canonical_digest(v).map_err(|e| {
                vec![Diagnostic::new(
                    "SOMA-CMP-0004",
                    format!("plan digest cannot be recomputed ({e})"),
                )]
            })
        })?;
    if computed != plan.canonicalization.sha256 {
        return Err(vec![enrich_one(
            Diagnostic::new(
                "SOMA-CMP-0004",
                "plan canonicalization does not verify against the reviewed plan digest",
            ),
            "/canonicalization/sha256",
            ctx,
        )]);
    }

    // 3. Digest binding: compiled identity must equal the reviewed one.
    if plan.canonicalization.sha256 != reviewed_identity {
        let mut d = Diagnostic::new(
            "SOMA-CMP-0004",
            "plan identity does not match the reviewed plan",
        );
        d.remediation = Some(DiagnosticRemediation {
            action: Some("restore-reviewed-plan".to_string()),
            summary: "Execution must use the exact plan that was reviewed; restore the reviewed \
                 plan or send the new plan through review."
                .to_string(),
        });
        return Err(vec![enrich_one(d, "/canonicalization/sha256", ctx)]);
    }

    Ok(())
}

/// Best-effort workflow digest from raw plan text (for diagnostic source).
fn plan_digest_hint(plan_text: &str) -> Option<String> {
    serde_json::from_str::<Value>(plan_text).ok().and_then(|v| {
        v.get("workflowDigest")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

/// Workflow id if the text still parses far enough to expose one.
fn workflow_id_of(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
}

fn enrich_all(diagnostics: Vec<Diagnostic>, text: &str) -> Vec<Diagnostic> {
    let id = workflow_id_of(text);
    diagnostics
        .into_iter()
        .map(|d| enrich_one(d, "", id.as_deref()))
        .collect()
}

/// Contract req3: attach source and remediation to one diagnostic.
///
/// `path` is an RFC 6901 JSON pointer into the offending document
/// (`""` = the whole document); `subject_hint` names the artifact the
/// document belongs to (workflow id / workflowDigest) when no stable
/// element id is known. A precise source already attached at the
/// refusal site is preserved — only a missing subject is filled in.
fn enrich_one(mut d: Diagnostic, path: &str, subject_hint: Option<&str>) -> Diagnostic {
    let subject = d
        .related
        .first()
        .cloned()
        .or_else(|| subject_hint.map(str::to_string));
    match d.source.as_mut() {
        Some(existing) => {
            if existing.subject.is_none() {
                existing.subject = subject;
            }
        }
        None => {
            d.source = Some(DiagnosticSource {
                path: path.to_string(),
                subject,
            });
        }
    }
    if d.remediation.is_none() {
        d.remediation = Some(remediation_for(&d.code));
    }
    d
}

/// Lite-authored advisory remediation for published SOMA codes.
///
/// Codes/severities/messages are never altered — only an optional
/// `remediation` hint is attached. Codes without a curated hint get a
/// fallback that points at the vendored catalogue (no invented guidance).
pub fn remediation_for(code: &str) -> DiagnosticRemediation {
    let (action, summary): (Option<&str>, &str) = match code {
        "SOMA-AUTH-0001" => (
            Some("grant-capability"),
            "Add the used capability to the operation's authority grants, or remove its use.",
        ),
        "SOMA-AUTH-0002" => (
            Some("narrow-composite"),
            "Reduce the composite's imported authority, or move the operation to a workflow within the declared imports.",
        ),
        "SOMA-AUTH-0003" => (
            Some("declare-scope"),
            "Declare the accessed scope in readableScopes/writableScopes, or stop the access.",
        ),
        "SOMA-AUTH-0004" => (
            Some("allow-provider"),
            "Add the provider to providerPolicy.allowlist, or route the content to an allowed provider.",
        ),
        "SOMA-AUTH-0005" => (
            Some("restrict-capability"),
            "Remove the capability or operation from the allowed set, or declare it.",
        ),
        "SOMA-AUTH-0006" => (
            Some("declare-secret"),
            "Declare the secret grant, or stop reading the secret.",
        ),
        "SOMA-AUTH-0007" => (
            Some("approve-effect"),
            "Attach an approved review decision to the effect before execution.",
        ),
        "SOMA-AUTH-0008" => (
            Some("add-recovery"),
            "Grant authority for the irreversible effect, or add an escalation (recovery) path.",
        ),
        "SOMA-EXP-0007" => (
            Some("declare-crossing"),
            "Declare the data/authority/effect/context crossing on the edge, or remove it.",
        ),
        "SOMA-CMP-0001" => (
            Some("upgrade-version"),
            "Use an artifact version compatible with the vendored bundle.",
        ),
        "SOMA-CMP-0003" => (
            Some("fix-schema"),
            "Correct the document so it matches its declared schema; check for unknown or malformed fields.",
        ),
        "SOMA-CMP-0004" => (
            Some("reseal-digest"),
            "Recompute the canonical digest with the tooling; never hand-edit digest fields.",
        ),
        "SOMA-CMP-0007" => (
            Some("dedupe-keys"),
            "Remove the duplicate object key so the document has one canonical form.",
        ),
        _ => (
            None,
            "Refer to the SOMA v1.1 diagnostics catalogue for this code's normative guidance.",
        ),
    };
    DiagnosticRemediation {
        action: action.map(str::to_string),
        summary: summary.to_string(),
    }
}
