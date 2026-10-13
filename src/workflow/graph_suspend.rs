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

/// Input bundle for [`suspend_run`].
#[derive(Debug, Clone, Copy)]
pub struct SuspendParams<'a> {
    pub id: &'a str,
    pub workflow_version: &'a str,
    pub trigger: &'a str,
    pub revision: Option<&'a str>,
    pub expiration: Option<&'a str>,
    pub operation_id: &'a str,
    pub inputs_digest: &'a str,
    pub plan_digest: &'a str,
    pub authority_snapshot_digest: &'a str,
    pub recorded_at: &'a str,
}

/// Resume-time environment the token is checked against.
#[derive(Debug, Clone)]
pub struct ResumeCheckParams<'a> {
    pub manifest: &'a super::graph_state::GraphManifestV1,
    pub portable_digest: &'a str,
    pub repo_revision_now: &'a str,
    pub plan_digest_now: &'a str,
    pub authority_snapshot_digest_now: &'a str,
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
/// Refuses a terminated run: a cancelled run cannot be suspended or
/// resumed (the #123 law). Seals the state, records the suspension
/// evidence entry, and reseals so the exported checkpoint round-trips.
pub fn suspend_run(
    state: &mut GraphRunStateV1,
    params: SuspendParams<'_>,
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
    // Write-boundary digest validation: every digest member must match the
    // vendored `^[0-9a-f]{64}$` shape before any document exists.
    for (member, value, label) in [
        ("planDigest", params.plan_digest, "plan_digest"),
        (
            "authoritySnapshotDigest",
            params.authority_snapshot_digest,
            "authority_snapshot_digest",
        ),
        ("inputsDigest", params.inputs_digest, "inputs_digest"),
    ] {
        if !is_64hex_lower(value) {
            bail!("{member} ({label}) must be a 64-hex lowercase digest");
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
        plan_digest: params.plan_digest.to_string(),
        authority_snapshot_digest: params.authority_snapshot_digest.to_string(),
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
pub fn resume_with_token(
    checkpoint_text: &str,
    suspension: &SuspensionPointV1,
    token: &ResumeTokenV1,
    params: &ResumeCheckParams<'_>,
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

    // 5. Staleness: plan / authority / state must match resume time.
    if params.plan_digest_now != suspension.plan_digest {
        return Err(refusal(
            "SOMA-RES-0001",
            "/planDigest",
            "reviewed plan identity moved since suspension".to_string(),
        ));
    }
    if params.authority_snapshot_digest_now != suspension.authority_snapshot_digest {
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
