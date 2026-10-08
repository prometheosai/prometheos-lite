//! Slice 3 §5 — evidence-timeline projection.
//!
//! Render is pure over `(TimelineProjectionSource, policy)`;
//! against-source verification fresh-renders from THE SAME source.

use serde::{Deserialize, Serialize};

use crate::db::repository::work_context_events::ProvenanceState;
use crate::work::soma_projection::RunKey;
use crate::workflow::projection::envelope;
use crate::workflow::projection::graph::GraphDisclosureView;
use crate::workflow::projection::review::{OmissionView, RunKeyView};
use crate::workflow::projection::{
    PROJECTION_VERSION_V1, VersionedProjectionEnvelope, digest_of, validated_source,
};
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::contracts::WorkflowDefinition;
use crate::workflow::soma::event::{WorkEvent, WorkEventBatch};
use crate::workflow::soma::types::{Hex64, OutcomeVariant};

const TIMELINE_DISCLOSURE_DOMAIN: &str = "projection.evidence-timeline.policy.v1";

pub const TIMELINE_SCHEMA_VERSION: &str = "lite.evidence-timeline.v1";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CompletenessView {
    pub more_available: bool,
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
    pub semantic_digest: Hex64,
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
    pub scope: TimelineScope,
    pub events: Vec<TimelineEventView>,
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
            .cmp(&b.produced_at)
            .then(a.artifact_kind.cmp(&b.artifact_kind))
            .then(a.id.cmp(&b.id))
    });
    out
}

/// Render one event into its projected view with the disclosure-driven
/// encryption marker (withheld nodes carry a stable `withheld: true` +
/// `hidden_*` counts = 0, matching GraphDisclosureView semantics).
fn render_event(
    event: &WorkEvent,
    provenance: Option<&[(String, ProvenanceState)]>,
) -> Result<TimelineEventView, Vec<Diagnostic>> {
    let disclosure = GraphDisclosureView {
        withheld: false,
        reason: "scope-visible".to_string(),
        category: "visible".to_string(),
        hidden_nodes: None,
        hidden_edges: None,
    };
    Ok(TimelineEventView {
        event_id: event.id.clone(),
        event_type: event.event_type.clone(),
        actor_kind: actor_kind_label(&event.actor.kind).to_string(),
        actor_identity: event.actor.identity.clone(),
        authority_class: authority_class_label(&event.effective_authority),
        sequence: event.sequence,
        timestamp: Some(event.timestamp.clone()),
        correlation_id: event.correlation_id.clone(),
        repo_revision: event.repo_revision.clone(),
        semantic_digest: event.semantic_digest.clone(),
        evidence_references: evidence_view(event),
        provenance: provenance_for_event(provenance, &event.id),
        outcomes: event_outcomes(event),
        status: event_status_label(event),
        disclosure,
    })
}

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

fn policy_digest(
    wf: &WorkflowDefinition,
    normalized: &TimelineDisclosurePolicy,
) -> Result<String, Vec<Diagnostic>> {
    let value = serde_json::json!({
        "domain": TIMELINE_DISCLOSURE_DOMAIN,
        "schemaVersion": TIMELINE_SCHEMA_VERSION,
        "rootSourceDigest": crate::workflow::projection::source_digest_of(wf)?,
        "policy": serde_json::to_value(normalized).map_err(|e| {
            vec![Diagnostic::new(
                "PROJ-0001",
                format!("disclosure policy cannot be serialized ({e})"),
            )]
        })?,
    });
    digest_of(&value)
}

/// §5: deterministic timeline renderer.
pub fn render_timeline_projection(
    source: &TimelineProjectionSource<'_>,
    policy: Option<&TimelineDisclosurePolicy>,
    wf: &WorkflowDefinition,
) -> Result<VersionedProjectionEnvelope<TimelineProjectionPayload>, Vec<Diagnostic>> {
    validated_source(wf)?;
    let events = total_order_event_list(source.batch)?;
    let normalized = policy.cloned().unwrap_or_default().normalize();
    let mut rendered_events: Vec<TimelineEventView> = Vec::new();
    for event in events {
        let mut view = render_event(event, source.provenance)?;
        if !normalized
            .authorized_targets
            .iter()
            .any(|t| t == "actorIdentity")
        {
            view.actor_identity = "".to_string();
            view.disclosure = GraphDisclosureView {
                withheld: true,
                reason: "actor-identity-withheld".to_string(),
                category: "policy-withheld".to_string(),
                hidden_nodes: None,
                hidden_edges: None,
            };
        }
        rendered_events.push(view);
    }
    let mut omissions = Vec::new();
    if source.page.is_none() {
        omissions.push(OmissionView {
            section: "completeness".to_string(),
            category: "unavailable".to_string(),
            reason: "noAuthoritativeSource".to_string(),
        });
    }
    if source.provenance.is_none() {
        omissions.push(OmissionView {
            section: "provenance".to_string(),
            category: "unavailable".to_string(),
            reason: "noAuthoritativeSource".to_string(),
        });
    }
    omissions.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(a.category.cmp(&b.category))
            .then(a.reason.cmp(&b.reason))
    });
    let payload = TimelineProjectionPayload {
        timeline_schema_version: TIMELINE_SCHEMA_VERSION.to_string(),
        disclosure_policy_digest: policy_digest(wf, &normalized)?,
        scope: TimelineScope {
            run_key: Some(RunKeyView::from(source.scope)),
            request_correlation: source
                .batch
                .events
                .first()
                .map(|e| e.correlation_id.clone()),
            projected: source.page.copied(),
        },
        events: rendered_events,
        omissions,
        completeness: match source.page {
            Some(p) => CompletenessView {
                more_available: p.more_available,
                next_after: Some(p.next_after),
            },
            None => CompletenessView {
                more_available: true,
                next_after: None,
            },
        },
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
        source_digest: crate::workflow::projection::source_digest_of(wf)?,
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

/// §7: against-source verifier — fresh-renders from the SAME input source.
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
    let payload: TimelineProjectionPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|e| {
            vec![Diagnostic::new(
                "SOMA-CMP-0003",
                format!("timeline payload fails structural validation ({e})"),
            )]
        })?;
    let normalized = policy.cloned().unwrap_or_default().normalize();
    let expected = policy_digest(wf, &normalized)?;
    if expected != payload.disclosure_policy_digest {
        return Err(vec![Diagnostic::new(
            "SOMA-CMP-0004",
            "disclosure policy digest does not verify".to_string(),
        )]);
    }
    let fresh = render_timeline_projection(source, Some(&normalized), wf)?;
    let fresh_value = serde_json::to_value(&fresh.payload).map_err(|e| {
        vec![Diagnostic::new(
            "PROJ-0001",
            format!("timeline payload cannot be serialized ({e})"),
        )]
    })?;
    if fresh_value != envelope.payload {
        return Err(vec![Diagnostic::new(
            "PROJ-0002",
            "timeline projection does not match a fresh render of the authoritative inputs"
                .to_string(),
        )]);
    }
    Ok(())
}
