//! Slice 3 §5 — evidence-timeline projection.
//!
//! Render is pure over `(TimelineProjectionSource, policy)`;
//! against-source verification fresh-renders from THE SAME source.
//!
//! Fail-closed: the authoritative `WorkEventBatch::audit` runs before any
//! projection, the typed `RunKey` is bound to `batch.runId`, completeness is
//! copied from recorded page state (never inferred), and disclosure is a
//! validated allow-list.

use serde::{Deserialize, Serialize};

use crate::db::repository::work_context_events::ProvenanceState;
use crate::work::soma_projection::RunKey;
use crate::workflow::projection::envelope;
use crate::workflow::projection::graph::GraphDisclosureView;
use crate::workflow::projection::review::{OmissionView, RunKeyView};
use crate::workflow::projection::{
    PROJECTION_VERSION_V1, VersionedProjectionEnvelope, digest_of, source_digest_of,
    validated_source,
};
use crate::workflow::soma::contracts::WorkflowDefinition;
use crate::workflow::soma::event::{WorkEvent, WorkEventBatch};
use crate::workflow::soma::types::{Hex64, OutcomeVariant};
use crate::workflow::soma::{Diagnostic, supported_version};

pub const TIMELINE_SCHEMA_VERSION: &str = "lite.evidence-timeline.v1";

/// Domain label for the timeline policy digest (§3.4.1).
pub const TIMELINE_POLICY_DOMAIN: &str = "projection.evidence-timeline.policy.v1";
/// Domain label for the timeline event-stream digest (§3.4.2).
pub const TIMELINE_EVENTS_DOMAIN: &str = "projection.evidence-timeline.events.v1";

/// Closed vocabulary of timeline disclosure targets (§6.1).
pub const TIMELINE_DISCLOSURE_TARGETS: &[&str] =
    &["payloadDetails", "actorIdentity", "referenceDigest"];
/// Closed vocabulary of timeline count-authorization targets (§6.2).
pub const TIMELINE_COUNT_TARGETS: &[&str] = &["events"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineDisclosurePolicy {
    #[serde(default)]
    pub authorized_targets: Vec<String>,
    #[serde(default)]
    pub count_authorization: Vec<String>,
}

impl TimelineDisclosurePolicy {
    pub fn normalize(&self) -> Self {
        let mut a = self.authorized_targets.clone();
        let mut c = self.count_authorization.clone();
        a.sort();
        a.dedup();
        c.sort();
        c.dedup();
        Self {
            authorized_targets: a,
            count_authorization: c,
        }
    }

    fn has_target(&self, target: &str) -> bool {
        self.authorized_targets.iter().any(|t| t == target)
    }
    fn has_count(&self, target: &str) -> bool {
        self.count_authorization.iter().any(|t| t == target)
    }
}

fn hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// §6.1: validate both target lists, then normalize. Unknown ⇒ `PROJ-0003`.
pub fn normalize_timeline_policy(
    policy: Option<&TimelineDisclosurePolicy>,
) -> Result<TimelineDisclosurePolicy, Vec<Diagnostic>> {
    let raw = policy.cloned().unwrap_or_default();
    let mut diags: Vec<Diagnostic> = Vec::new();
    for t in &raw.authorized_targets {
        if !TIMELINE_DISCLOSURE_TARGETS.contains(&t.as_str()) {
            diags.push(Diagnostic::new(
                "PROJ-0003",
                format!("unknown timeline disclosure target: {t}"),
            ));
        }
    }
    for t in &raw.count_authorization {
        if !TIMELINE_COUNT_TARGETS.contains(&t.as_str()) {
            diags.push(Diagnostic::new(
                "PROJ-0003",
                format!("unknown timeline count-authorization target: {t}"),
            ));
        }
    }
    if !diags.is_empty() {
        diags.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.message.cmp(&b.message)));
        diags.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        return Err(diags);
    }
    Ok(raw.normalize())
}

/// ProjectionPageMeta borrows the authoritative page result fields;
/// NEVER inferred from event count.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectionPageMeta {
    pub next_after: i64,
    pub more_available: bool,
}

/// §5.0 — Authoritative timeline input: borrows of existing data.
pub struct TimelineProjectionSource<'a> {
    pub batch: &'a WorkEventBatch,
    pub provenance: Option<&'a [(String, ProvenanceState)]>,
    pub page: Option<&'a ProjectionPageMeta>,
    pub scope: &'a RunKey,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_key: Option<RunKeyView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_correlation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected: Option<ProjectionPageMeta>,
}

/// §4.1/§5.5 — honest completeness. `available` is `false` when no
/// authoritative page state was recorded; `moreAvailable`/`nextAfter` are
/// then `None` — never an invented paging fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompletenessView {
    pub available: bool,
    pub more_available: Option<bool>,
    pub next_after: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineProvenanceView {
    pub producer_kind: Option<String>,
    pub repo_binding: Option<RepoBindingLabel>,
    pub execution_class: Option<String>,
    pub refresh_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RepoBindingLabel {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineEventView {
    pub event_id: String,
    pub event_type: String,
    pub actor_kind: String,
    pub actor_identity: String,
    pub authority_class: String,
    pub sequence: u64,
    pub timestamp: Option<String>,
    pub correlation_id: String,
    pub repo_revision: String,
    pub semantic_digest: Option<Hex64>,
    pub evidence_references: Vec<crate::workflow::projection::review::EvidenceReferenceView>,
    pub provenance: TimelineProvenanceView,
    pub outcomes: Option<Vec<OutcomeVariant>>,
    pub status: String,
    pub disclosure: GraphDisclosureView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TimelineProjectionPayload {
    pub timeline_schema_version: String,
    pub disclosure_policy_digest: String,
    pub event_stream_digest: String,
    pub scope: TimelineScope,
    pub events: Vec<TimelineEventView>,
    /// §6.2: independently gated event count; `None` when `countAuthorization`
    /// does not include `"events"`, paired with a withheld omission.
    pub event_count: Option<u64>,
    pub omissions: Vec<OmissionView>,
    pub completeness: CompletenessView,
}

fn producer_kind_name(k: &crate::work::provenance::ProducerKind) -> &'static str {
    use crate::work::provenance::ProducerKind;
    match k {
        ProducerKind::Human => "human",
        ProducerKind::Harness => "harness",
        ProducerKind::Tool => "tool",
        ProducerKind::Memory => "memory",
        ProducerKind::System => "system",
    }
}

fn provenance_for_event(
    provenance: Option<&[(String, ProvenanceState)]>,
    event_id: &str,
) -> TimelineProvenanceView {
    match provenance {
        Some(rows) => rows
            .iter()
            .find(|(id, _)| id == event_id)
            .map(|(_, s)| match s {
                ProvenanceState::Verified(env) => TimelineProvenanceView {
                    producer_kind: Some(producer_kind_name(&env.producer.kind).to_string()),
                    repo_binding: None,
                    execution_class: Some(
                        format!("{:?}", env.authority.declared.execution_class).to_lowercase(),
                    ),
                    refresh_state: "verified".to_string(),
                },
                ProvenanceState::LegacyUnverified => TimelineProvenanceView {
                    producer_kind: None,
                    repo_binding: None,
                    execution_class: None,
                    refresh_state: "legacy-unverified".to_string(),
                },
            })
            .unwrap_or(TimelineProvenanceView {
                producer_kind: None,
                repo_binding: None,
                execution_class: None,
                refresh_state: "unavailable".to_string(),
            }),
        None => TimelineProvenanceView {
            producer_kind: None,
            repo_binding: None,
            execution_class: None,
            refresh_state: "unavailable".to_string(),
        },
    }
}

fn event_status_label(event: &WorkEvent) -> String {
    let Some(payload) = event.payload.as_ref() else {
        return "unavailable".to_string();
    };
    if let Some(outcome) = payload.outcome.as_ref() {
        return outcome.status.clone();
    }
    "unavailable".to_string()
}

fn event_outcomes(event: &WorkEvent) -> Option<Vec<OutcomeVariant>> {
    let payload = event.payload.as_ref()?;
    let outcome = payload.outcome.as_ref()?;
    match outcome.status.as_str() {
        "Produced" => Some(vec![OutcomeVariant::Produced]),
        "Skipped" => Some(vec![OutcomeVariant::Skipped]),
        "Blocked" => Some(vec![OutcomeVariant::Blocked]),
        "Failed" => Some(vec![OutcomeVariant::Failed]),
        "Cancelled" => Some(vec![OutcomeVariant::Cancelled]),
        "ReviewRequired" => Some(vec![OutcomeVariant::ReviewRequired]),
        _ => None,
    }
}

fn actor_kind_label(kind: &crate::workflow::soma::event::ActorKind) -> &'static str {
    use crate::workflow::soma::event::ActorKind;
    match kind {
        ActorKind::Agent => "agent",
        ActorKind::Model => "model",
        ActorKind::Provider => "provider",
        ActorKind::Harness => "harness",
        ActorKind::Human => "human",
        ActorKind::Tool => "tool",
        ActorKind::Memory => "memory",
    }
}

fn authority_class_label(a: &crate::workflow::soma::contracts::AuthorityProfile) -> String {
    use crate::workflow::soma::types::ExecutionClass;
    match a.execution_class {
        ExecutionClass::Deterministic => "deterministic".to_string(),
        ExecutionClass::ModelAssisted => "model-assisted".to_string(),
        ExecutionClass::OpenEnded => "open-ended".to_string(),
    }
}

fn evidence_view(
    event: &WorkEvent,
) -> Vec<crate::workflow::projection::review::EvidenceReferenceView> {
    let Some(refs) = event.evidence.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<_> = refs
        .iter()
        .map(
            |r| crate::workflow::projection::review::EvidenceReferenceView {
                id: r.id.clone(),
                event_digest: r.event_digest.clone(),
                artifact_digest: r.artifact_digest.clone(),
                artifact_kind: r.artifact_kind.clone(),
                produced_by: r.produced_by.clone(),
                produced_at: r.produced_at.clone(),
            },
        )
        .collect();
    out.sort_by(|a, b| {
        a.produced_at
            .as_deref()
            .unwrap_or("")
            .cmp(b.produced_at.as_deref().unwrap_or(""))
            .then(a.artifact_digest.as_str().cmp(b.artifact_digest.as_str()))
            .then(a.id.cmp(&b.id))
    });
    out
}

/// Render one event into its projected view (raw, pre-disclosure).
fn render_event(
    event: &WorkEvent,
    provenance: Option<&[(String, ProvenanceState)]>,
) -> TimelineEventView {
    TimelineEventView {
        event_id: event.id.clone(),
        event_type: event.event_type.clone(),
        actor_kind: actor_kind_label(&event.actor.kind).to_string(),
        actor_identity: event.actor.identity.clone(),
        authority_class: authority_class_label(&event.effective_authority),
        sequence: event.sequence,
        timestamp: Some(event.timestamp.clone()),
        correlation_id: event.correlation_id.clone(),
        repo_revision: event.repo_revision.clone(),
        semantic_digest: Some(event.semantic_digest.clone()),
        evidence_references: evidence_view(event),
        provenance: provenance_for_event(provenance, &event.id),
        outcomes: event_outcomes(event),
        status: event_status_label(event),
        disclosure: GraphDisclosureView {
            withheld: false,
            reason: "scope-visible".to_string(),
            category: "visible".to_string(),
            hidden_nodes: None,
            hidden_edges: None,
        },
    }
}

/// §5.4: deterministic total order by `(sequence, semanticDigest)`; exact
/// duplicate identity fails closed (`SOMA-CMP-0011`).
fn total_order_event_list(batch: &WorkEventBatch) -> Result<Vec<&WorkEvent>, Vec<Diagnostic>> {
    let mut events: Vec<&WorkEvent> = batch.events.iter().collect();
    events.sort_by(|a, b| {
        a.sequence
            .cmp(&b.sequence)
            .then(a.semantic_digest.as_str().cmp(b.semantic_digest.as_str()))
    });
    for pair in events.windows(2) {
        if pair[0].sequence == pair[1].sequence
            && pair[0].semantic_digest == pair[1].semantic_digest
        {
            return Err(vec![Diagnostic::new(
                "SOMA-CMP-0011",
                format!(
                    "duplicate event identity within run {} at sequence {}",
                    batch.run_id, pair[0].sequence
                ),
            )]);
        }
    }
    Ok(events)
}

/// §5.5: a missing `sequence` between the observed min and max is a source
/// gap, surfaced honestly; never silently dropped.
fn has_sequence_gap(events: &[&WorkEvent]) -> bool {
    let mut seqs: Vec<u64> = events.iter().map(|e| e.sequence).collect();
    seqs.sort_unstable();
    seqs.dedup();
    seqs.windows(2).any(|w| w[1].saturating_sub(w[0]) > 1)
}

fn distinct_correlations(events: &[&WorkEvent]) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> =
        events.iter().map(|e| e.correlation_id.clone()).collect();
    std::mem::take(&mut set).into_iter().collect()
}

/// §3.4.2: `eventStreamDigest` preimage over the authoritative event stream,
/// in rendering order, using the SOURCE semantic digests (never the
/// possibly-disclosed payload copies).
fn event_stream_digest(
    wf: &WorkflowDefinition,
    source: &TimelineProjectionSource<'_>,
) -> Result<String, Vec<Diagnostic>> {
    let events = total_order_event_list(source.batch)?;
    let digests: Vec<&str> = events.iter().map(|e| e.semantic_digest.as_str()).collect();
    let preimage = serde_json::json!({
        "domain": TIMELINE_EVENTS_DOMAIN,
        "rootSourceDigest": source_digest_of(wf)?,
        "timelineSchemaVersion": TIMELINE_SCHEMA_VERSION,
        "runKey": RunKeyView::from(source.scope),
        "events": digests,
    });
    digest_of(&preimage)
}

fn policy_digest(
    wf: &WorkflowDefinition,
    normalized: &TimelineDisclosurePolicy,
) -> Result<String, Vec<Diagnostic>> {
    let preimage = serde_json::json!({
        "domain": TIMELINE_POLICY_DOMAIN,
        "schemaVersion": TIMELINE_SCHEMA_VERSION,
        "rootSourceDigest": source_digest_of(wf)?,
        "policy": serde_json::to_value(normalized).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("disclosure policy cannot be serialized ({e})"),
            )]
        })?,
    });
    digest_of(&preimage)
}

fn push_omission(acc: &mut Vec<OmissionView>, section: &str, category: &str, reason: &str) {
    acc.push(OmissionView {
        section: section.to_string(),
        category: category.to_string(),
        reason: reason.to_string(),
    });
}

fn sort_omissions(omissions: &mut Vec<OmissionView>) {
    omissions.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(a.category.cmp(&b.category))
            .then(a.reason.cmp(&b.reason))
    });
    omissions.dedup();
}

/// Apply the normalized disclosure policy to the rendered events.
fn apply_disclosure(
    events: &mut [TimelineEventView],
    policy: &TimelineDisclosurePolicy,
    omissions: &mut Vec<OmissionView>,
) {
    let allow_actor = policy.has_target("actorIdentity");
    let allow_details = policy.has_target("payloadDetails");
    let allow_refs = policy.has_target("referenceDigest");
    let mut withheld_actor = false;
    let mut withheld_details = false;
    let mut withheld_refs = false;
    for view in events.iter_mut() {
        if !allow_actor && !view.actor_identity.is_empty() {
            withheld_actor = true;
            view.actor_identity = String::new();
            view.disclosure = GraphDisclosureView {
                withheld: true,
                reason: "actor-identity-withheld".to_string(),
                category: "policy-withheld".to_string(),
                hidden_nodes: None,
                hidden_edges: None,
            };
        }
        if !allow_details {
            if !view.event_type.is_empty()
                || !view.correlation_id.is_empty()
                || !view.repo_revision.is_empty()
                || view.timestamp.is_some()
                || view.outcomes.is_some()
                || view.status != "unavailable"
            {
                withheld_details = true;
            }
            view.event_type = String::new();
            view.correlation_id = String::new();
            view.repo_revision = String::new();
            view.timestamp = None;
            view.outcomes = None;
            view.status = "unavailable".to_string();
        }
        if !allow_refs {
            if view.semantic_digest.is_some() || !view.evidence_references.is_empty() {
                withheld_refs = true;
            }
            view.semantic_digest = None;
            view.evidence_references.clear();
        }
    }
    if withheld_actor {
        push_omission(omissions, "actorIdentity", "withheld", "disclosurePolicy");
    }
    if withheld_details {
        push_omission(omissions, "payloadDetails", "withheld", "disclosurePolicy");
    }
    if withheld_refs {
        push_omission(omissions, "referenceDigest", "withheld", "disclosurePolicy");
    }
}

/// §5: deterministic timeline renderer.
pub fn render_timeline_projection(
    source: &TimelineProjectionSource<'_>,
    policy: Option<&TimelineDisclosurePolicy>,
    wf: &WorkflowDefinition,
) -> Result<VersionedProjectionEnvelope<TimelineProjectionPayload>, Vec<Diagnostic>> {
    validated_source(wf)?;
    let normalized = normalize_timeline_policy(policy)?;

    // Item 2: the authoritative event audit is mandatory and fail-closed.
    let audit = source.batch.audit(&supported_version());
    if !audit.is_empty() {
        return Err(audit);
    }
    // Item 2: bind the typed RunKey to the authoritative batch run id.
    if source.scope.id != source.batch.run_id {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0002",
            format!(
                "runKey id {:?} does not match batch runId {:?}",
                source.scope.id, source.batch.run_id
            ),
        )]);
    }

    let ordered = total_order_event_list(source.batch)?;
    let comparable: Vec<&WorkEvent> = ordered.clone();
    let mut rendered_events: Vec<TimelineEventView> = ordered
        .iter()
        .map(|event| render_event(event, source.provenance))
        .collect();

    let mut omissions: Vec<OmissionView> = Vec::new();
    if has_sequence_gap(&comparable) {
        push_omission(&mut omissions, "events", "unavailable", "sourceGap");
    }
    if source.page.is_none() {
        push_omission(
            &mut omissions,
            "completeness",
            "unavailable",
            "noAuthoritativeSource",
        );
    }
    if source.provenance.is_none() {
        push_omission(
            &mut omissions,
            "provenance",
            "unavailable",
            "noAuthoritativeSource",
        );
    }

    apply_disclosure(&mut rendered_events, &normalized, &mut omissions);

    let correlations = distinct_correlations(&comparable);
    // Scope correlation is payload detail: expose it only when
    // `payloadDetails` is authorized; a mixed set is out of scope.
    let request_correlation = if correlations.len() == 1 {
        if normalized.has_target("payloadDetails") {
            correlations.first().cloned()
        } else {
            None
        }
    } else {
        push_omission(&mut omissions, "scope", "outOfScope", "scopeExcluded");
        None
    };

    sort_omissions(&mut omissions);

    let completeness = match source.page {
        Some(p) => CompletenessView {
            available: true,
            more_available: Some(p.more_available),
            next_after: Some(p.next_after),
        },
        // No recorded page state: availability is unknown; never invent
        // `moreAvailable: true`.
        None => CompletenessView {
            available: false,
            more_available: None,
            next_after: None,
        },
    };

    // §6.2: independently gated event count; `None` when unauthorized,
    // paired with an explicit withheld omission so the absence is visible.
    let event_count: Option<u64> = if normalized.has_count("events") {
        Some(rendered_events.len() as u64)
    } else {
        push_omission(&mut omissions, "events", "withheld", "disclosurePolicy");
        None
    };

    let payload = TimelineProjectionPayload {
        timeline_schema_version: TIMELINE_SCHEMA_VERSION.to_string(),
        disclosure_policy_digest: policy_digest(wf, &normalized)?,
        event_stream_digest: event_stream_digest(wf, source)?,
        scope: TimelineScope {
            run_key: Some(RunKeyView::from(source.scope)),
            request_correlation,
            projected: source.page.copied(),
        },
        events: rendered_events,
        event_count,
        omissions,
        completeness,
    };
    let payload_value = serde_json::to_value(&payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("timeline payload cannot be serialized ({e})"),
        )]
    })?;
    let projection_digest = digest_of(&payload_value)?;
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: wf.schema_version.clone(),
        source_digest: source_digest_of(wf)?,
        projection_digest,
        payload,
    })
}

/// §7: structural byte verifier — source-independent.
pub fn verify_timeline_projection_bytes(
    raw: &[u8],
) -> Result<VersionedProjectionEnvelope<TimelineProjectionPayload>, Vec<Diagnostic>> {
    let env = envelope::parse_envelope_bytes::<TimelineProjectionPayload>(raw)?;
    if env.payload.timeline_schema_version != TIMELINE_SCHEMA_VERSION {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0001",
            format!(
                "unsupported timelineSchemaVersion {}",
                env.payload.timeline_schema_version
            ),
        )]);
    }
    for (label, digest) in [
        (
            "disclosurePolicyDigest",
            &env.payload.disclosure_policy_digest,
        ),
        ("eventStreamDigest", &env.payload.event_stream_digest),
    ] {
        if !hex64(digest) {
            return Err(vec![Diagnostic::new(
                "PROJ-0002",
                format!("{label} is not a lowercase 64-hex digest"),
            )]);
        }
    }
    let payload_value = serde_json::to_value(&env.payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("timeline payload cannot be serialized ({e})"),
        )]
    })?;
    let expected = digest_of(&payload_value)?;
    if expected != env.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "timeline projection digest does not verify".to_string(),
        )]);
    }
    Ok(env)
}

/// §7: against-source verifier. Independently verifies envelope metadata,
/// `sourceDigest`, `projectionDigest == digest(payload)`, the normalized
/// policy digest, `eventStreamDigest`, and fresh-render byte identity.
pub fn verify_timeline_against_source(
    envelope: &VersionedProjectionEnvelope<serde_json::Value>,
    source: &TimelineProjectionSource<'_>,
    policy: Option<&TimelineDisclosurePolicy>,
    wf: &WorkflowDefinition,
) -> Result<(), Vec<Diagnostic>> {
    envelope::verify_envelope_metadata(envelope)?;
    if envelope.schema_version != wf.schema_version {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "schemaVersion does not match the source AST".to_string(),
        )]);
    }
    let expected_source = source_digest_of(wf)?;
    if expected_source != envelope.source_digest {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "sourceDigest does not match the source AST".to_string(),
        )]);
    }
    let expected_payload = digest_of(&envelope.payload)?;
    if expected_payload != envelope.projection_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "timeline projection digest does not verify".to_string(),
        )]);
    }
    let payload: TimelineProjectionPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0003",
                format!("timeline payload fails structural validation ({e})"),
            )]
        })?;
    let normalized = normalize_timeline_policy(policy)?;
    let expected_policy = policy_digest(wf, &normalized)?;
    if expected_policy != payload.disclosure_policy_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "disclosure policy digest does not verify".to_string(),
        )]);
    }
    let expected_stream = event_stream_digest(wf, source)?;
    if expected_stream != payload.event_stream_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "event stream digest does not verify".to_string(),
        )]);
    }
    let fresh = render_timeline_projection(source, Some(&normalized), wf)?;
    let fresh_value = serde_json::to_value(&fresh.payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("timeline payload cannot be serialized ({e})"),
        )]
    })?;
    if fresh.source_digest != envelope.source_digest || fresh_value != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "timeline projection does not match a fresh render of the authoritative inputs"
                .to_string(),
        )]);
    }
    Ok(())
}
