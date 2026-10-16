//! E4/X05 (#162) Slice 1 — durable suspension + resume-token contracts.
//!
//! Typed `SuspensionPointV1` / `ResumeTokenV1` mapped to the vendored SOMA
//! v1.1 schemas (vendored `soma/**` stays read-only): a human-gate (or
//! operator) suspension persists the run WITHOUT holding a live worker,
//! journals a suspension evidence record in the run state, and a resume
//! token authorizes re-import through a fail-closed ladder — unsupported
//! version (`SOMA-RES-0004`), tampered/mismatched token or suspension
//! (`SOMA-RES-0003`), expiry against an injected RFC 3339 clock
//! (`SOMA-RES-0002`), and stale plan/state/authority digests
//! (`SOMA-RES-0001`) — all BEFORE delegating to
//! [`super::graph_resume::resume_run`], whose stale repo-revision and
//! replay-authorization gates still apply. A token never bypasses the run
//! law.
//!
//! Digest binding rule (state): the suspension records the sealed state
//! digest computed BEFORE the suspension journal entry is appended; the
//! resume gate recomputes it from the imported checkpoint with exactly
//! this suspension's journal entry removed (`artifact_kind ==
//! "suspension-point"`, `id == format!("suspension:{id}")`). Any forward
//! motion after suspension changes the filtered digest, so a state that
//! moved fails closed as `SOMA-RES-0001`.
//!
//! # Permit-only boundary (Slice B repair P1-A)
//!
//! The raw transition primitives (`suspend_run`, `resume_with_token`) and
//! every `AuthorityBinding` construction mechanism are crate-private, so
//! `GovernancePermit` is the only externally reachable governed
//! suspension/resume path. Stated honestly: NO API/CLI suspension endpoint
//! exists today — the library permit boundary IS the externally reachable
//! governed transition surface (nothing more is claimed). The standalone
//! X05 implementation is preserved in-crate and exercised through that
//! boundary.
//!
//! The compile-fail proofs below run as rustdoc tests under
//! `cargo test --doc --all-features`, which is registered in BOTH
//! authoritative evidence-runner specification points
//! (`SUITE_SPEC["core"]` and `rust_core()` in `scripts/local_ci.py`).
//!
//! An external caller cannot invoke raw suspension:
//!
//! ```compile_fail
//! use prometheos_lite::workflow::graph_suspend::suspend_run;
//! ```
//!
//! An external caller cannot invoke raw resume:
//!
//! ```compile_fail
//! use prometheos_lite::workflow::graph_suspend::resume_with_token;
//! ```
//!
//! An external caller cannot construct a caller-controlled authority
//! binding (the type itself is unnameable, so no caller-supplied plan or
//! authority digest can ever enter a transition):
//!
//! ```compile_fail
//! use prometheos_lite::workflow::graph_suspend::AuthorityBinding;
//! let _binding = AuthorityBinding {
//!     plan_digest: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
//!     authority_snapshot_digest: "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
//! };
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::graph_resume::{ReplayAuthorization, ResumedRun, resume_run};
use super::graph_state::GraphRunStateV1;
use super::soma::canonical::try_canonical_digest;
use super::soma::{Diagnostic, DiagnosticSource};
use crate::workflow::governance_compiler::remediation_for;

/// Published schema version of the Lite suspension contracts below.
pub const SUSPENSION_SCHEMA_VERSION: &str = "1.0.0";

/// `why` the run suspended (operator request, pending human review, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuspensionProvenanceV1 {
    #[serde(default)]
    pub trigger: String,
}

/// What a resume must re-arm: the operation and the digest of its inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumabilityV1 {
    pub operation_id: String,
    pub inputs_digest: String,
}

/// A durable suspension point (vendored `SuspensionPoint` v1.1 shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuspensionPointV1 {
    pub id: String,
    pub schema_version: String,
    pub workflow_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub provenance: SuspensionProvenanceV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration: Option<String>,
    /// Sealed digest of the run state at suspension time (see module docs).
    pub state_digest: String,
    /// Reviewed governance-plan identity this run was issued under.
    pub plan_digest: String,
    /// Digest of the authority snapshot the run was governed by.
    pub authority_snapshot_digest: String,
    pub resumability: ResumabilityV1,
}

impl SuspensionPointV1 {
    /// Canonical digest of this suspension document.
    pub fn digest(&self) -> Result<String> {
        let value = serde_json::to_value(self).context("serialize suspension point")?;
        try_canonical_digest(&value).context("digest suspension point")
    }
}

/// A resume token bound to one suspension point (vendored `ResumeToken`
/// v1.1 shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeTokenV1 {
    pub id: String,
    pub suspension_point_id: String,
    pub suspension_digest: String,
    pub workflow_version: String,
    pub schema_version: String,
    pub authority_snapshot_digest: String,
    pub plan_digest: String,
    /// Canonical digest over the binding tuple
    /// (`suspensionPointId`, `suspensionDigest`, `planDigest`,
    /// `authoritySnapshotDigest`, `workflowVersion`, `schemaVersion`,
    /// `revision`), self-excluded.
    pub binding_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration: Option<String>,
}

/// Digest binding digest-authority for suspension/resume transitions.
///
/// A binding packages the reviewed plan identity plus the compiled
/// authority snapshot digest **and can only be constructed inside this
/// crate by `GovernancePermit`** (via [`AuthorityBinding::new`]). The raw
/// suspend/resume primitives take `&AuthorityBinding`, so production code
/// cannot suspend or resume under caller-supplied digests without a
/// permit — transitions are permit-only (E4/X06 Slice B repair 1).
pub(crate) struct AuthorityBinding {
    plan_digest: String,
    authority_snapshot_digest: String,
}

impl AuthorityBinding {
    pub(crate) fn new(plan_digest: &str, authority_snapshot_digest: &str) -> Self {
        Self {
            plan_digest: plan_digest.to_string(),
            authority_snapshot_digest: authority_snapshot_digest.to_string(),
        }
    }

    /// Test-only constructor for in-crate isolation controls (Blocker 2):
    /// allows varying plan vs authority digests independently. Never
    /// reachable from production code paths.
    #[cfg(test)]
    pub(crate) fn for_test(plan_digest: &str, authority_snapshot_digest: &str) -> Self {
        Self::new(plan_digest, authority_snapshot_digest)
    }
}

/// Input bundle for [`suspend_run`] (plan/authority digests come from the
/// [`AuthorityBinding`] — never from the caller).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SuspendParams<'a> {
    pub id: &'a str,
    pub workflow_version: &'a str,
    pub trigger: &'a str,
    pub revision: Option<&'a str>,
    pub expiration: Option<&'a str>,
    pub operation_id: &'a str,
    pub inputs_digest: &'a str,
    pub recorded_at: &'a str,
}

/// Resume-time environment the token is checked against. The plan and
/// authority expectations come from the [`AuthorityBinding`] presented by
/// the resuming permit, not from free parameters.
#[derive(Debug, Clone)]
pub(crate) struct ResumeCheckParams<'a> {
    pub manifest: &'a super::graph_state::GraphManifestV1,
    pub portable_digest: &'a str,
    pub repo_revision_now: &'a str,
    pub now: &'a str,
    pub replay: Option<ReplayAuthorization>,
}

/// Fail-closed resume verdicts.
#[derive(Debug)]
pub enum ResumeWithTokenError {
    /// Catalogue-coded token/suspension refusal (`SOMA-RES-*`).
    Refusal(Vec<Diagnostic>),
    /// The underlying #123 run law refused (stale repo revision,
    /// unauthorized replay, terminated run, checkpoint integrity...).
    Underlying(anyhow::Error),
}

impl std::fmt::Display for ResumeWithTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refusal(d) => {
                write!(
                    f,
                    "{}",
                    d.iter()
                        .map(|d| d.code.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Self::Underlying(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ResumeWithTokenError {}

fn parse_rfc3339(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

fn refusal(code: &'static str, pointer: &str, message: String) -> ResumeWithTokenError {
    let mut d = Diagnostic::new(code, message);
    d.source = Some(DiagnosticSource {
        path: pointer.to_string(),
        subject: None,
    });
    d.remediation = Some(remediation_for(code));
    ResumeWithTokenError::Refusal(vec![d])
}

/// Lowercase 64-hex digest shape (`^[0-9a-f]{64}$`), as the vendored
/// `SuspensionPoint`/`ResumeToken` schemas pattern every digest member.
fn is_64hex_lower(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The exact binding tuple a resume token commits to. Absence of the
/// optional members is explicit (empty string), so removing or extending
/// `expiration` changes the binding digest.
fn token_binding_input(
    suspension_point_id: &str,
    suspension_digest: &str,
    plan_digest: &str,
    authority_snapshot_digest: &str,
    workflow_version: &str,
    schema_version: &str,
    revision: Option<&str>,
    expiration: Option<&str>,
) -> serde_json::Value {
    json!({
        "suspensionPointId": suspension_point_id,
        "suspensionDigest": suspension_digest,
        "planDigest": plan_digest,
        "authoritySnapshotDigest": authority_snapshot_digest,
        "workflowVersion": workflow_version,
        "schemaVersion": schema_version,
        "revision": revision.unwrap_or_default(),
        "expiration": expiration.unwrap_or_default(),
    })
}

/// Suspend a live run at a durable point and journal the event.
///
/// The `binding` carries the reviewed plan identity and authority
/// snapshot digests; only `GovernancePermit` can construct one, so a
/// suspension always binds real permit materials. Refuses a terminated
/// run: a cancelled run cannot be suspended or resumed (the #123 law).
/// Seals the state, records the suspension evidence entry, and reseals so
/// the exported checkpoint round-trips.
pub(crate) fn suspend_run(
    state: &mut GraphRunStateV1,
    params: SuspendParams<'_>,
    binding: &AuthorityBinding,
) -> Result<SuspensionPointV1> {
    if state.termination.is_some() {
        bail!("cannot suspend a terminated run");
    }
    // One consistent version policy: creation, issuance, and resume all
    // support exactly `SUSPENSION_SCHEMA_VERSION`. A suspension created
    // under any other version could never be resumed, so creation refuses.
    if params.workflow_version != SUSPENSION_SCHEMA_VERSION {
        bail!(
            "workflow_version {workflow_version:?} is not the supported version \
             {SUSPENSION_SCHEMA_VERSION:?}",
            workflow_version = params.workflow_version,
        );
    }
    // Write-boundary digest validation (defense in depth): the vendored
    // `^[0-9a-f]{64}$` shape is enforced even for the binding's digests —
    // `AuthorityBinding` is crate-private, but malformed material must
    // never produce a document anywhere.
    for (member, value) in &[
        ("planDigest", binding.plan_digest.clone()),
        (
            "authoritySnapshotDigest",
            binding.authority_snapshot_digest.clone(),
        ),
        ("inputsDigest", params.inputs_digest.to_string()),
    ] {
        if !is_64hex_lower(value) {
            bail!("{member} must be a 64-hex lowercase digest");
        }
    }
    if let Some(exp) = params.expiration
        && parse_rfc3339(exp).is_none()
    {
        bail!("expiration must be RFC 3339");
    }
    // Seal first: the suspension binds this exact state.
    state.content_digest = Some(state.compute_digest());
    let state_digest = state.content_digest.clone().expect("sealed");
    let suspension = SuspensionPointV1 {
        id: params.id.to_string(),
        schema_version: SUSPENSION_SCHEMA_VERSION.to_string(),
        workflow_version: params.workflow_version.to_string(),
        revision: params.revision.map(str::to_string),
        provenance: SuspensionProvenanceV1 {
            trigger: params.trigger.to_string(),
        },
        expiration: params.expiration.map(str::to_string),
        state_digest: state_digest.clone(),
        plan_digest: binding.plan_digest.clone(),
        authority_snapshot_digest: binding.authority_snapshot_digest.clone(),
        resumability: ResumabilityV1 {
            operation_id: params.operation_id.to_string(),
            inputs_digest: params.inputs_digest.to_string(),
        },
    };
    let event_digest = suspension.digest()?;
    state
        .evidence_refs
        .push(crate::workflow::memory_contracts::EvidenceReferenceV1 {
            id: format!("suspension:{}", params.id),
            event_digest,
            artifact_digest: state_digest,
            artifact_kind: "suspension-point".to_string(),
            produced_by: params.trigger.to_string(),
            produced_at: Some(params.recorded_at.to_string()),
        });
    // Reseal so a checkpoint exported after suspension round-trips.
    state.content_digest = Some(state.compute_digest());
    Ok(suspension)
}

/// Issue a resume token bound to one suspension point.
///
/// The one consistent version policy: issuance refuses a suspension whose
/// contract versions the resume ladder does not support.
pub fn issue_resume_token(
    suspension: &SuspensionPointV1,
    id: &str,
    expiration: Option<&str>,
) -> Result<ResumeTokenV1> {
    if suspension.schema_version != SUSPENSION_SCHEMA_VERSION
        || suspension.workflow_version != SUSPENSION_SCHEMA_VERSION
    {
        bail!("cannot issue a token for an unsupported suspension version");
    }
    if let Some(exp) = expiration
        && parse_rfc3339(exp).is_none()
    {
        bail!("expiration must be RFC 3339");
    }
    let suspension_digest = suspension.digest()?;
    let binding_digest = try_canonical_digest(&token_binding_input(
        &suspension.id,
        &suspension_digest,
        &suspension.plan_digest,
        &suspension.authority_snapshot_digest,
        &suspension.workflow_version,
        &suspension.schema_version,
        suspension.revision.as_deref(),
        expiration,
    ))
    .context("digest token binding")?;
    Ok(ResumeTokenV1 {
        id: id.to_string(),
        suspension_point_id: suspension.id.clone(),
        suspension_digest,
        workflow_version: suspension.workflow_version.clone(),
        schema_version: suspension.schema_version.clone(),
        authority_snapshot_digest: suspension.authority_snapshot_digest.clone(),
        plan_digest: suspension.plan_digest.clone(),
        binding_digest,
        revision: suspension.revision.clone(),
        expiration: expiration.map(str::to_string),
    })
}

/// Resume a suspended run with a token, enforcing the fail-closed ladder
/// before delegating to the #123 run law.
pub(crate) fn resume_with_token(
    checkpoint_text: &str,
    suspension: &SuspensionPointV1,
    token: &ResumeTokenV1,
    params: &ResumeCheckParams<'_>,
    binding: &AuthorityBinding,
) -> Result<ResumedRun, ResumeWithTokenError> {
    // 1. Version gate (runs first: an unrecognised shape is unsupported,
    //    not tampered).
    for (v, pointer) in [
        (suspension.schema_version.as_str(), "/schemaVersion"),
        (suspension.workflow_version.as_str(), "/workflowVersion"),
        (token.schema_version.as_str(), "/schemaVersion"),
        (token.workflow_version.as_str(), "/workflowVersion"),
    ] {
        if v != SUSPENSION_SCHEMA_VERSION {
            return Err(refusal(
                "SOMA-RES-0004",
                pointer,
                format!("unsupported suspension-contract version {v:?}"),
            ));
        }
    }

    // 2. Token ↔ suspension alignment.
    let suspension_digest = suspension
        .digest()
        .map_err(ResumeWithTokenError::Underlying)?;
    if token.suspension_point_id != suspension.id || token.suspension_digest != suspension_digest {
        return Err(refusal(
            "SOMA-RES-0003",
            "/suspensionDigest",
            "resume token does not bind to the supplied suspension point".to_string(),
        ));
    }
    if token.plan_digest != suspension.plan_digest
        || token.authority_snapshot_digest != suspension.authority_snapshot_digest
        || token.workflow_version != suspension.workflow_version
        || token.schema_version != suspension.schema_version
        || token.revision != suspension.revision
    {
        return Err(refusal(
            "SOMA-RES-0003",
            "",
            "resume token members drift from the suspension they bind".to_string(),
        ));
    }

    // 3. Token self-binding (tamper). Absence of the optionals is explicit
    //    (empty-string members) so removing or extending expiration changes
    //    the recomputed digest.
    let recomputed = try_canonical_digest(&token_binding_input(
        &token.suspension_point_id,
        &token.suspension_digest,
        &token.plan_digest,
        &token.authority_snapshot_digest,
        &token.workflow_version,
        &token.schema_version,
        token.revision.as_deref(),
        token.expiration.as_deref(),
    ))
    .map_err(|e| ResumeWithTokenError::Underlying(anyhow::Error::from(e)))?;
    if recomputed != token.binding_digest {
        return Err(refusal(
            "SOMA-RES-0003",
            "/bindingDigest",
            "resume token binding digest does not verify".to_string(),
        ));
    }

    // 4. Expiry: any declared expiration must lie strictly in the future.
    let now = parse_rfc3339(params.now).ok_or_else(|| {
        refusal(
            "SOMA-RES-0003",
            "",
            "resume clock is not RFC 3339".to_string(),
        )
    })?;
    for exp in [
        suspension.expiration.as_deref(),
        token.expiration.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let exp_at = parse_rfc3339(exp).ok_or_else(|| {
            refusal(
                "SOMA-RES-0003",
                "/expiration",
                "expiration is not RFC 3339".to_string(),
            )
        })?;
        if now >= exp_at {
            return Err(refusal(
                "SOMA-RES-0002",
                "/expiration",
                "resume token (or its suspension point) has expired".to_string(),
            ));
        }
    }

    // 5. Staleness: the resuming permit's plan identity and authority
    //    snapshot must equal the suspension's bindings; the run-state
    //    digest is reconstructed from the imported checkpoint.
    if binding.plan_digest != suspension.plan_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/planDigest",
            "reviewed plan identity moved since suspension".to_string(),
        ));
    }
    if binding.authority_snapshot_digest != suspension.authority_snapshot_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/authoritySnapshotDigest",
            "authority snapshot moved since suspension".to_string(),
        ));
    }
    let imported = GraphRunStateV1::import_checkpoint(
        checkpoint_text,
        params.manifest,
        params.portable_digest,
    )
    .map_err(ResumeWithTokenError::Underlying)?;
    // The imported checkpoint must contain EXACTLY ONE suspension evidence
    // entry for this suspension, and the entry itself must bind the
    // suspension document and the pre-suspension state digest. A
    // pre-suspension checkpoint (no entry), duplicated entries, or a
    // tampered entry are all stale bindings and fail closed.
    let suspension_evidence_id = format!("suspension:{}", suspension.id);
    let entries: Vec<_> = imported
        .evidence_refs
        .iter()
        .filter(|e| e.artifact_kind == "suspension-point" && e.id == suspension_evidence_id)
        .collect();
    if entries.len() != 1 {
        return Err(refusal(
            "SOMA-RES-0001",
            "/evidenceRefs",
            format!(
                "expected exactly one suspension journal entry {suspension_evidence_id:?}, found {}",
                entries.len()
            ),
        ));
    }
    let entry = entries[0];
    if entry.event_digest != suspension_digest || entry.artifact_digest != suspension.state_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/evidenceRefs",
            "suspension journal entry digest mismatch (tampered evidence)".to_string(),
        ));
    }
    let mut pre = imported;
    pre.evidence_refs
        .retain(|e| !(e.artifact_kind == "suspension-point" && e.id == suspension_evidence_id));
    let state_digest = pre.compute_digest();
    if state_digest != suspension.state_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/stateDigest",
            "run state moved since suspension (stale binding)".to_string(),
        ));
    }

    // 6. Delegated run law (stale repo revision, replay authorization,
    //    termination, ...). A token never bypasses it.
    resume_run(
        checkpoint_text,
        params.manifest,
        params.portable_digest,
        params.repo_revision_now,
        params.replay.clone(),
    )
    .map_err(ResumeWithTokenError::Underlying)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::graph_state::{GraphManifestV1, GraphNodeV1, GraphRunStateV1};

    const PLAN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const AUTH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const OTHER: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    fn manifest() -> GraphManifestV1 {
        GraphManifestV1 {
            schema_version: "1.0.0".into(),
            graph_id: "g".into(),
            version: "1.0.0".into(),
            nodes: vec![GraphNodeV1 {
                node_id: "a".into(),
                capability: "c.a".into(),
                purpose: None,
                resources: vec![],
                join: None,
            }],
            edges: vec![],
            entry_points: vec!["a".into()],
            terminal_exits: vec!["a".into()],
            shared_state_keys: vec![],
            policy_digest: None,
            content_digest: None,
        }
        .sealed()
    }

    fn fixture() -> (
        GraphRunStateV1,
        SuspensionPointV1,
        ResumeTokenV1,
        AuthorityBinding,
    ) {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let binding = AuthorityBinding::new(PLAN, AUTH);
        let suspension = suspend_run(
            &mut run,
            SuspendParams {
                id: "susp-1",
                workflow_version: SUSPENSION_SCHEMA_VERSION,
                trigger: "human-review",
                revision: None,
                expiration: None,
                operation_id: "a",
                inputs_digest: PLAN, // shape only: any exact 64-hex value
                recorded_at: "t1",
            },
            &binding,
        )
        .unwrap();
        let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
        (run, suspension, token, binding)
    }

    fn codes(err: &ResumeWithTokenError) -> Vec<String> {
        match err {
            ResumeWithTokenError::Refusal(d) => d.iter().map(|d| d.code.clone()).collect(),
            ResumeWithTokenError::Underlying(e) => vec![e.to_string()],
        }
    }

    /// First refusal's diagnostic source path, if any.
    fn first_source(err: &ResumeWithTokenError) -> Option<&str> {
        match err {
            ResumeWithTokenError::Refusal(d) => d
                .first()
                .and_then(|d| d.source.as_ref())
                .map(|s| s.path.as_str()),
            ResumeWithTokenError::Underlying(_) => None,
        }
    }

    /// Audit-clean workflow identical in shape to the shared integration
    /// fixture `permit_for(&["a"], &["echo"])` (integration-test helpers
    /// are not reachable from in-crate tests, so the known-good document
    /// is reproduced here verbatim in structure).
    fn reviewed_workflow_text() -> String {
        r#"{
  "schemaVersion": "1.1.0", "version": "1.1.0", "id": "lite-governed",
  "name": "Governed", "kind": "atomic",
  "inputPorts": [{"name": "goal", "direction": "input", "type": "string",
                   "cardinality": "single", "requiredness": "required"}],
  "outputPorts": [{"name": "result", "direction": "output", "type": "string",
                    "cardinality": "single", "requiredness": "required"}],
  "body": [{
    "schemaVersion": "1.1.0", "version": "1.1.0", "id": "a",
    "executionClass": "deterministic",
    "inputs": [{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}],
    "outputs": [{"name": "result", "type": "string", "emits": ["Produced"]}],
    "authority": ["readable:repo://evaluation", "readable:repo://x", "readable:repo://fixture",
                  "writable:work://evaluation", "writable:work://y", "writable:repo://fixture", "echo"],
    "effects": [], "uses": [], "secrets": [], "context": []
  }],
  "authority": {
    "executionClass": "deterministic", "mutation": "none", "tools": {"echo": []},
    "readableScopes": ["repo://evaluation", "repo://x", "repo://fixture"],
    "writableScopes": ["work://evaluation", "work://y", "repo://fixture"],
    "networkPolicy": {"default": "deny"}, "providerPolicy": {"allowlist": []},
    "secrets": []
  }
}"#
        .to_string()
    }

    /// A REAL reviewed permit for the fixture workflow — the exact
    /// artifact shape a reviewer approves (identity = canonical digest of
    /// the reviewed text; issue refuses any other identity).
    fn reviewed_permit() -> crate::workflow::governance_permit::GovernancePermit {
        let text = reviewed_workflow_text();
        let identity = crate::workflow::governance_compiler::compile_workflow_text(&text)
            .expect("audit-clean workflow compiles")
            .canonicalization
            .sha256;
        crate::workflow::governance_permit::GovernancePermit::issue(&text, &identity)
            .expect("permit issues for its reviewed identity")
    }

    /// P1-B isolated authority-staleness control (review repair): plan
    /// identity is held EXACTLY equal to a real reviewed permit's
    /// identity; only a different VALID authority snapshot digest is
    /// presented, and the refusal must be `SOMA-RES-0001` pointing at
    /// `/authoritySnapshotDigest`, never `/planDigest`.
    ///
    /// Coherence of the four artifacts is proven first: the permit-bound
    /// suspension binds the reviewed plan identity + authority snapshot;
    /// the token is byte-deterministic over that suspension; the evidence
    /// journal holds EXACTLY ONE entry binding the suspension document
    /// and its pre-suspension state seal; the checkpoint round-trips and
    /// reproduces that seal once the journal entry is removed; and the
    /// issuing permit itself resumes successfully (positive control).
    #[test]
    fn authority_snapshot_only_drift_refuses_on_reviewed_plan_held_equal() {
        let permit = reviewed_permit();
        let m = manifest();
        let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();

        // 1. Suspension binds EXACTLY the reviewed permit identity.
        let suspension = permit
            .suspend_run(
                &mut run,
                "susp-1",
                "human-review",
                "a",
                PLAN,
                "t1",
                None,
                None,
            )
            .expect("permit-bound suspension");
        assert_eq!(suspension.plan_digest, permit.plan_identity());
        assert_eq!(
            suspension.authority_snapshot_digest,
            permit.authority_snapshot_digest()
        );
        assert_ne!(
            permit.authority_snapshot_digest(),
            OTHER,
            "control digest must differ from the reviewed snapshot"
        );

        // 2. Token coherence: byte-deterministic over this suspension.
        let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
        let reissued = issue_resume_token(&suspension, "tok-1", None).unwrap();
        assert_eq!(token.binding_digest, reissued.binding_digest);
        assert_eq!(token.suspension_digest, suspension.digest().unwrap());

        // 3. Evidence journal coherence: exactly one entry binding the
        //    suspension document and the pre-suspension state seal.
        let entries: Vec<_> = run
            .evidence_refs
            .iter()
            .filter(|e| e.artifact_kind == "suspension-point" && e.id == "suspension:susp-1")
            .collect();
        assert_eq!(entries.len(), 1, "{:?}", run.evidence_refs);
        assert_eq!(entries[0].event_digest, suspension.digest().unwrap());
        assert_eq!(entries[0].artifact_digest, suspension.state_digest);

        // 4. Checkpoint coherence: export/import round-trips; dropping the
        //    journal entry reproduces the exact state seal the suspension
        //    binds (the resume ladder's own state rule).
        let ckpt = run.export_checkpoint().unwrap();
        let imported = GraphRunStateV1::import_checkpoint(&ckpt, &m, "pd").unwrap();
        let mut pre = imported;
        pre.evidence_refs
            .retain(|e| !(e.artifact_kind == "suspension-point" && e.id == "suspension:susp-1"));
        assert_eq!(pre.compute_digest(), suspension.state_digest);

        // 5. Positive control: the issuing permit resumes all four
        //    artifacts through the governed path.
        permit
            .resume_run(
                &ckpt,
                &suspension,
                &token,
                &m,
                "pd",
                "rev-1",
                "2026-10-13T00:00:00Z",
                None,
            )
            .expect("identical binding resumes");

        // 6. ISOLATION: plan identity held EXACTLY equal to the reviewed
        //    permit; only a different VALID authority snapshot digest.
        let drift = AuthorityBinding::for_test(permit.plan_identity(), OTHER);
        let err = resume_with_token(
            &ckpt,
            &suspension,
            &token,
            &ResumeCheckParams {
                manifest: &m,
                portable_digest: "pd",
                repo_revision_now: "rev-1",
                now: "2026-10-13T00:00:00Z",
                replay: None,
            },
            &drift,
        )
        .unwrap_err();
        assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()], "{err:?}");
        assert_eq!(
            first_source(&err),
            Some("/authoritySnapshotDigest"),
            "refusal must point at the authority snapshot member, not planDigest: {err:?}"
        );
        assert_ne!(first_source(&err), Some("/planDigest"));
    }

    /// Plan-only mismatch (authority snapshot held equal) → RES-0001.
    #[test]
    fn plan_only_mismatch_fails_stale_with_authority_held_equal() {
        let (run, suspension, token, _binding) = fixture();
        let ckpt = run.export_checkpoint().unwrap();
        let m = manifest();
        let drifting = AuthorityBinding::for_test(OTHER, AUTH);
        let err = resume_with_token(
            &ckpt,
            &suspension,
            &token,
            &ResumeCheckParams {
                manifest: &m,
                portable_digest: "pd",
                repo_revision_now: "rev-1",
                now: "2026-10-13T00:00:00Z",
                replay: None,
            },
            &drifting,
        )
        .unwrap_err();
        assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
        assert_eq!(
            first_source(&err),
            Some("/planDigest"),
            "plan drift must point at the plan member: {err:?}"
        );
    }

    /// Control: identical bindings resume successfully.
    #[test]
    fn identical_binding_resumes() {
        let (run, suspension, token, binding) = fixture();
        let ckpt = run.export_checkpoint().unwrap();
        let m = manifest();
        let ok = resume_with_token(
            &ckpt,
            &suspension,
            &token,
            &ResumeCheckParams {
                manifest: &m,
                portable_digest: "pd",
                repo_revision_now: "rev-1",
                now: "2026-10-13T00:00:00Z",
                replay: None,
            },
            &binding,
        );
        assert!(
            ok.is_ok(),
            "{}",
            ok.err().map(|e| codes(&e).join(",")).unwrap_or_default()
        );
    }

    /// Structural guarantee: the primitives cannot be exercised with
    /// caller-supplied digests — AuthorityBinding has no public
    /// constructor. This test's existence documents the invariant; the
    /// compiler enforces it (a pub fn would be needed and isn't).
    #[test]
    fn authority_binding_fields_stay_crate_private() {
        let b = AuthorityBinding::new(PLAN, AUTH);
        assert_eq!(b.plan_digest, PLAN);
        assert_eq!(b.authority_snapshot_digest, AUTH);
        assert_eq!(b.plan_digest.len(), 64);
    }
}
