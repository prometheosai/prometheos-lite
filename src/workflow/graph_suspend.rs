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
//! # Permit-only boundary (Slice B repair P1-A, second round)
//!
//! The raw transitions accept NO digest material: `suspend_run` and
//! `resume_with_token` require a real `&GovernancePermit` and read the
//! reviewed plan identity and the compiled authority snapshot from it
//! (the permit's fields are private and its only constructor is reviewed
//! issuance). The former digest-carrying binding type and the free-form
//! input bundles were REMOVED, so no module — inside the crate or out —
//! can hand a transition forged plan or authority digests: the only
//! digests a transition ever sees are the ones the supplied permit was
//! issued with. The digest-parameterized cores below are private to this
//! module and exist so the in-crate staleness-isolation controls can
//! vary plan and authority digests independently; production transitions
//! always enter through the permit-gated functions.
//!
//! An in-crate policy regression
//! (`raw_transition_surface_stays_permit_gated_in_crate`) fails if a
//! forgeable binding constructor or a raw digest-bearing transition
//! surface becomes production-visible again.
//!
//! Stated honestly: NO API/CLI suspension endpoint exists today — the
//! library permit boundary IS the externally reachable governed
//! transition surface (nothing more is claimed).
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
//! The forgeable binding type is gone entirely — no caller, external or
//! in-crate, can name it, so no caller-supplied plan or authority digest
//! can ever enter a transition:
//!
//! ```compile_fail
//! use prometheos_lite::workflow::graph_suspend::AuthorityBinding;
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::execution_graph::CompiledExecutionGraphV1;
use super::governance_permit::GovernancePermit;
use super::graph_resume::{ReplayAuthorization, ResumedRun, resume_run};
use super::graph_state::{GraphManifestV1, GraphRunStateV1};
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

/// Input record for the suspension core (module-private: the
/// permit-gated wrapper builds it from its own arguments, so no
/// production code outside this module can construct one).
#[derive(Debug, Clone, Copy)]
struct SuspendRecord<'a> {
    id: &'a str,
    trigger: &'a str,
    revision: Option<&'a str>,
    expiration: Option<&'a str>,
    operation_id: &'a str,
    inputs_digest: &'a str,
    recorded_at: &'a str,
}

/// Resume-time environment the token is checked against
/// (module-private; the permit-gated wrapper builds it). The plan and
/// authority expectations always come from the resuming permit, never
/// from free parameters.
#[derive(Debug, Clone)]
struct ResumeCheck<'a> {
    manifest: &'a GraphManifestV1,
    portable_digest: &'a str,
    repo_revision_now: &'a str,
    now: &'a str,
    replay: Option<ReplayAuthorization>,
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
/// Permit-gated transition (P1-A repair): the reviewed plan identity and
/// authority snapshot digests are read from `permit` — a real
/// `GovernancePermit` whose only constructor is reviewed issuance — so no
/// caller, in-crate or external, can supply forged digest material.
///
/// Operation binding (P1-B repair): fails closed BEFORE any state
/// mutation unless the suspension operation is (1) authorized by the
/// permit (a step of the reviewed plan), (2) a node of `manifest`, (3)
/// on the run's live frontier, and (4) `manifest` is the exact graph
/// revision the run pinned at open. A refused suspension journals
/// nothing and leaves the run state byte-identical.
///
/// Refuses a terminated run: a cancelled run cannot be suspended or
/// resumed (the #123 law). Seals the state, records the suspension
/// evidence entry, and reseals so the exported checkpoint round-trips.
#[allow(clippy::too_many_arguments)]
pub(crate) fn suspend_run(
    state: &mut GraphRunStateV1,
    manifest: &GraphManifestV1,
    suspension_id: &str,
    trigger: &str,
    operation_id: &str,
    inputs_digest: &str,
    now: &str,
    revision: Option<&str>,
    expiration: Option<&str>,
    permit: &GovernancePermit,
) -> Result<SuspensionPointV1> {
    // The #123 law first: a terminated run can never transition.
    if state.termination.is_some() {
        bail!("cannot suspend a terminated run");
    }
    // P1-B binding gate 1 — the supplied manifest must be the exact
    // graph revision this run pinned at open.
    if state.graph_manifest_digest != manifest.compute_digest() {
        bail!("supplied manifest is not the graph manifest revision this run pinned");
    }
    // P1-B binding gate 2 — the operation must be authorized by the
    // permit: a step of the reviewed plan this permit was issued for.
    if !permit
        .execution_graph()
        .steps
        .iter()
        .any(|step| step.operation_id == operation_id)
    {
        bail!(
            "operation {operation_id:?} is not governed by the reviewed plan; \
             the permit does not authorize suspending it"
        );
    }
    // P1-B binding gate 3 — the operation must be a node of the supplied
    // graph manifest (the run/manifest contract the caller presents).
    if !manifest.nodes.iter().any(|n| n.node_id == operation_id) {
        bail!("operation {operation_id:?} is not a node of the supplied graph manifest");
    }
    // P1-B binding gate 4 — the operation must be live: a node currently
    // on the run's frontier (eligible to execute now, neither completed
    // nor future-locked).
    if !state.frontier.iter().any(|n| n == operation_id) {
        bail!("operation {operation_id:?} is not on the live frontier of the supplied run");
    }
    suspend_run_bound(
        state,
        SuspendRecord {
            id: suspension_id,
            trigger,
            revision,
            expiration,
            operation_id,
            inputs_digest,
            recorded_at: now,
        },
        permit.plan_identity(),
        &permit.authority_snapshot_digest(),
    )
}

/// Digest-parameterized suspension core (module-private). Production
/// transitions always enter through [`suspend_run`], which supplies the
/// digests of a real permit; this core exists separately so the in-crate
/// staleness-isolation controls can vary plan and authority digests
/// independently. It enforces no permit authorization — that is the
/// wrapper's gate — only the inherited write-boundary law.
fn suspend_run_bound(
    state: &mut GraphRunStateV1,
    record: SuspendRecord<'_>,
    plan_digest: &str,
    authority_snapshot_digest: &str,
) -> Result<SuspensionPointV1> {
    if state.termination.is_some() {
        bail!("cannot suspend a terminated run");
    }
    // One consistent version policy is structural at creation: there is
    // no version input to forge — the document always carries exactly
    // `SUSPENSION_SCHEMA_VERSION`, and issuance/resume refuse anything
    // else.
    // Write-boundary digest validation (defense in depth): the vendored
    // `^[0-9a-f]{64}$` shape is enforced even for the permit-supplied
    // digests — malformed material must never produce a document anywhere.
    for (member, value) in &[
        ("planDigest", plan_digest.to_string()),
        (
            "authoritySnapshotDigest",
            authority_snapshot_digest.to_string(),
        ),
        ("inputsDigest", record.inputs_digest.to_string()),
    ] {
        if !is_64hex_lower(value) {
            bail!("{member} must be a 64-hex lowercase digest");
        }
    }
    if let Some(exp) = record.expiration
        && parse_rfc3339(exp).is_none()
    {
        bail!("expiration must be RFC 3339");
    }
    // Seal first: the suspension binds this exact state.
    state.content_digest = Some(state.compute_digest());
    let state_digest = state.content_digest.clone().expect("sealed");
    let suspension = SuspensionPointV1 {
        id: record.id.to_string(),
        schema_version: SUSPENSION_SCHEMA_VERSION.to_string(),
        workflow_version: SUSPENSION_SCHEMA_VERSION.to_string(),
        revision: record.revision.map(str::to_string),
        provenance: SuspensionProvenanceV1 {
            trigger: record.trigger.to_string(),
        },
        expiration: record.expiration.map(str::to_string),
        state_digest: state_digest.clone(),
        plan_digest: plan_digest.to_string(),
        authority_snapshot_digest: authority_snapshot_digest.to_string(),
        resumability: ResumabilityV1 {
            operation_id: record.operation_id.to_string(),
            inputs_digest: record.inputs_digest.to_string(),
        },
    };
    let event_digest = suspension.digest()?;
    state
        .evidence_refs
        .push(crate::workflow::memory_contracts::EvidenceReferenceV1 {
            id: format!("suspension:{}", record.id),
            event_digest,
            artifact_digest: state_digest,
            artifact_kind: "suspension-point".to_string(),
            produced_by: record.trigger.to_string(),
            produced_at: Some(record.recorded_at.to_string()),
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
///
/// Permit-gated transition (P1-A repair): the staleness expectations are
/// read from `permit` — a real `GovernancePermit` — so resume can never
/// be driven by forged digest material.
///
/// Operation binding (P1-B repair): the ladder revalidates the
/// suspension's resumability operation against the resuming permit (a
/// step of its reviewed plan), the supplied manifest, and after import
/// the live frontier of the resumed run — the same binding the creation
/// gate enforced, re-proven at resume (`SOMA-RES-0001` pointing at
/// `/resumability/operationId`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resume_with_token(
    checkpoint_text: &str,
    suspension: &SuspensionPointV1,
    token: &ResumeTokenV1,
    manifest: &GraphManifestV1,
    portable_digest: &str,
    repo_revision_now: &str,
    now: &str,
    replay: Option<ReplayAuthorization>,
    permit: &GovernancePermit,
) -> Result<ResumedRun, ResumeWithTokenError> {
    resume_with_token_bound(
        checkpoint_text,
        suspension,
        token,
        &ResumeCheck {
            manifest,
            portable_digest,
            repo_revision_now,
            now,
            replay,
        },
        permit.plan_identity(),
        &permit.authority_snapshot_digest(),
        permit.execution_graph(),
    )
}

/// Digest-parameterized resume core (module-private): the same
/// fail-closed ladder, with the plan/authority expectations and the
/// authorized operation set supplied explicitly so the in-crate
/// isolation controls can vary them independently. Production resumes
/// always enter through [`resume_with_token`], which supplies the digests
/// and authorized steps of a real permit.
fn resume_with_token_bound(
    checkpoint_text: &str,
    suspension: &SuspensionPointV1,
    token: &ResumeTokenV1,
    params: &ResumeCheck<'_>,
    plan_digest: &str,
    authority_snapshot_digest: &str,
    authorized: &CompiledExecutionGraphV1,
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
    if plan_digest != suspension.plan_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/planDigest",
            "reviewed plan identity moved since suspension".to_string(),
        ));
    }
    if authority_snapshot_digest != suspension.authority_snapshot_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/authoritySnapshotDigest",
            "authority snapshot moved since suspension".to_string(),
        ));
    }
    // 5b. Operation binding (P1-B revalidation): the resumability
    //     operation must still be authorized by the resuming permit and
    //     be a node of the supplied manifest contract. A suspension
    //     document whose operation is foreign to the reviewed plan —
    //     e.g. a legacy artifact created before the creation gate
    //     existed, or a document with a rewritten resumability member —
    //     fails closed here as a stale binding.
    let operation = suspension.resumability.operation_id.as_str();
    if !authorized
        .steps
        .iter()
        .any(|step| step.operation_id == operation)
    {
        return Err(refusal(
            "SOMA-RES-0001",
            "/resumability/operationId",
            format!(
                "suspension operation {operation:?} is not governed by the \
                 reviewed plan (unknown to the resuming permit)"
            ),
        ));
    }
    if !params.manifest.nodes.iter().any(|n| n.node_id == operation) {
        return Err(refusal(
            "SOMA-RES-0001",
            "/resumability/operationId",
            format!(
                "suspension operation {operation:?} is not a node of the \
                 supplied graph manifest"
            ),
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
    // 5c. Operation binding (P1-B revalidation, live node): the imported
    //     state is now proven unmoved, so the resumability operation must
    //     sit on its live frontier — the exact node the creation gate saw
    //     as eligible to execute.
    if !pre.frontier.iter().any(|n| n == operation) {
        return Err(refusal(
            "SOMA-RES-0001",
            "/resumability/operationId",
            format!(
                "suspension operation {operation:?} is not on the live \
                 frontier of the resumed run"
            ),
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

    fn fixture() -> (GraphRunStateV1, SuspensionPointV1, ResumeTokenV1) {
        let m = manifest();
        let mut run = GraphRunStateV1::open("r1", &m, "rev-1", "pws", "pd", "t0").unwrap();
        let suspension = suspend_run_bound(
            &mut run,
            SuspendRecord {
                id: "susp-1",
                trigger: "human-review",
                revision: None,
                expiration: None,
                operation_id: "a",
                inputs_digest: PLAN, // shape only: any exact 64-hex value
                recorded_at: "t1",
            },
            PLAN,
            AUTH,
        )
        .unwrap();
        let token = issue_resume_token(&suspension, "tok-1", None).unwrap();
        (run, suspension, token)
    }

    /// Resume-time environment for the private-core controls below.
    fn check(m: &GraphManifestV1) -> ResumeCheck<'_> {
        ResumeCheck {
            manifest: m,
            portable_digest: "pd",
            repo_revision_now: "rev-1",
            now: "2026-10-13T00:00:00Z",
            replay: None,
        }
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

        // 1. Suspension binds EXACTLY the reviewed permit identity (the
        //    permit path also proves the operation binding here: "a" is
        //    a reviewed step, a manifest node, and on the live frontier).
        let suspension = permit
            .suspend_run(
                &mut run,
                &m,
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

        // 6. ISOLATION via the module-private core: plan identity held
        //    EXACTLY equal to the reviewed permit (presented together
        //    with the permit's own authorized steps); only a different
        //    VALID authority snapshot digest.
        let err = resume_with_token_bound(
            &ckpt,
            &suspension,
            &token,
            &check(&m),
            permit.plan_identity(),
            OTHER,
            permit.execution_graph(),
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
        let permit = reviewed_permit();
        let (run, suspension, token) = fixture();
        let ckpt = run.export_checkpoint().unwrap();
        let m = manifest();
        let err = resume_with_token_bound(
            &ckpt,
            &suspension,
            &token,
            &check(&m),
            OTHER,
            AUTH,
            permit.execution_graph(),
        )
        .unwrap_err();
        assert_eq!(codes(&err), vec!["SOMA-RES-0001".to_string()]);
        assert_eq!(
            first_source(&err),
            Some("/planDigest"),
            "plan drift must point at the plan member: {err:?}"
        );
    }

    /// Control: identical bindings resume successfully (the reviewed
    /// permit's own authorized steps presented with the fixture's
    /// synthetic digests).
    #[test]
    fn identical_binding_resumes() {
        let permit = reviewed_permit();
        let (run, suspension, token) = fixture();
        let ckpt = run.export_checkpoint().unwrap();
        let m = manifest();
        let ok = resume_with_token_bound(
            &ckpt,
            &suspension,
            &token,
            &check(&m),
            PLAN,
            AUTH,
            permit.execution_graph(),
        );
        assert!(
            ok.is_ok(),
            "{}",
            ok.err().map(|e| codes(&e).join(",")).unwrap_or_default()
        );
    }

    /// P1-A in-crate policy regression: the raw transition surface must
    /// stay permit-gated. External compile-fail doctests cannot observe
    /// crate-internal visibility, so this control reads the production
    /// source of both modules and fails if a forgeable binding
    /// constructor or a raw digest-bearing transition surface becomes
    /// production-visible again.
    #[test]
    fn raw_transition_surface_stays_permit_gated_in_crate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let graph_suspend_src = std::fs::read_to_string(root.join("src/workflow/graph_suspend.rs"))
            .expect("graph_suspend source readable");
        let permit_src = std::fs::read_to_string(root.join("src/workflow/governance_permit.rs"))
            .expect("governance_permit source readable");
        // Code lines only: doc comments (//! and ///) can legitimately
        // mention removed names in prose and compile-fail proofs.
        let code_of = |src: &str| {
            let production = src
                .split_once("#[cfg(test)]")
                .map(|(p, _)| p)
                .unwrap_or(src);
            production
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with("//!") && !t.starts_with("///")
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let code = code_of(&graph_suspend_src);
        let permit_code = code_of(&permit_src);

        // The forgeable binding type and the free-form digest-bearing
        // input bundles are gone from production code entirely.
        for forbidden in ["AuthorityBinding", "SuspendParams", "ResumeCheckParams"] {
            assert!(
                !code.contains(forbidden),
                "forbidden production surface {forbidden:?} reappeared in graph_suspend.rs"
            );
            assert!(
                !permit_code.contains(forbidden),
                "forbidden production surface {forbidden:?} reappeared in governance_permit.rs"
            );
        }
        // No fully-public raw transitions.
        assert!(!code.contains("pub fn suspend_run"));
        assert!(!code.contains("pub fn resume_with_token"));
        // The crate-visible transitions exist and REQUIRE a real permit.
        for name in ["suspend_run", "resume_with_token"] {
            let decl = format!("pub(crate) fn {name}(");
            let at = code.find(&decl).unwrap_or_else(|| {
                panic!("permit-gated transition {name} is not declared crate-visible")
            });
            let signature = &code[at..at + 600];
            assert!(
                signature.contains("permit: &GovernancePermit"),
                "{name} must take a real GovernancePermit"
            );
            assert!(
                !signature.contains("plan_digest: &str")
                    && !signature.contains("authority_snapshot_digest: &str"),
                "{name} must not accept raw digest material"
            );
        }
        // The digest-parameterized cores stay module-private.
        for core in ["suspend_run_bound", "resume_with_token_bound"] {
            let declared_private = code.lines().any(|l| l.trim() == format!("fn {core}("));
            assert!(
                declared_private,
                "digest core {core} must be declared private: `fn {core}(`"
            );
        }
        // The public governed surface on the permit stays public.
        assert!(permit_code.contains("pub fn suspend_run("));
        assert!(permit_code.contains("pub fn resume_run("));
    }
}
