//! #132 Slice 1B: the pure fail-closed SOMA `WorkEvent` projection over
//! the verified Slice 1A journal.
//!
//! A deterministic, read-only mapping from durable `work_context_events`
//! records (`ProvenanceState::Verified` only) to the published SOMA SPEC
//! 006 `WorkEvent` contracts (vendored v1.1.0 bundle). The projection:
//!
//! - **consumes verified records only** — legacy rows, mixed provenance
//!   states, and tampered/corrupt rows are refused, never fabricated;
//! - **fails closed on unsupported semantics** — unmapped journal event
//!   types, the `HumanDecision` execution class (SOMA v1.1 has no honest
//!   bucket), and stored legacy `system` producers are all refused with
//!   explicit errors;
//! - **invents nothing** — every SOMA field maps from a named journal
//!   source or is an honest, documented omission (`payload`, `evidence`,
//!   top-level `implementation`, `replay`/`conflict` stay absent/false);
//! - **emits `WorkEventBatch` only where its own audit can pass** — the
//!   per-run projection includes cross-run causal ancestors by closure so
//!   parent-closure holds by construction; stream pages are a distinct
//!   container type, never a masquerading batch.
//!
//! No endpoint, no transport, no client (Slice 2/3 boundaries).

use crate::db::repository::work_context_events::{JournalRecord, ProvenanceState};
use crate::work::provenance::{
    ExecutionClass as LiteExecutionClass, ProducerKind, RepoBinding, RunIdentity,
};
use crate::workflow::soma::Diagnostic;
use crate::workflow::soma::SUPPORTED_SCHEMA_VERSION;
use crate::workflow::soma::contracts::AuthorityProfile;
use crate::workflow::soma::event::{
    Actor, ActorKind, Compatibility, Implementation as SomaImplementation, WorkEvent,
};
use crate::workflow::soma::types::{ExecutionClass as SomaExecutionClass, Hex64, MutationMode};

/// Why a projection refused. Fail closed: no partial batches, no
/// fabricated envelopes, no silently skipped records.
#[derive(Debug)]
pub enum ProjectionError {
    /// Upstream journal failure — tamper detection, column drift, a
    /// mixed provenance state, a corrupt envelope, or IO. Surfaced from
    /// the verified read gate, never swallowed (the read gate always
    /// runs and always fails BEFORE any mapping work).
    Journal(anyhow::Error),
    /// The record cannot be honestly projected: a legacy/unverified row,
    /// an unmapped journal event type, the `HumanDecision` execution
    /// class, a stored legacy `system` producer, or (run batches) an
    /// unresolvable causal parent / ancestor cycle.
    Unsupported { event_id: String, reason: String },
    /// The projected batch failed the vendored `WorkEventBatch::audit`.
    /// A batch is never emitted dirty.
    Audit(Vec<Diagnostic>),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::Journal(e) => write!(f, "journal read failed: {e:#}"),
            ProjectionError::Unsupported { event_id, reason } => {
                write!(f, "event {event_id} cannot be projected: {reason}")
            }
            ProjectionError::Audit(diags) => {
                write!(f, "projected batch failed the SOMA audit: {diags:?}")
            }
        }
    }
}

impl std::error::Error for ProjectionError {}

/// The journal event type → SOMA SPEC 006 `eventType` mapping (approved
/// plan §1). The journal's `event_type` column is an open string; every
/// writer-known type has a pinned row and anything else fails closed.
fn map_event_type(journal_type: &str) -> Result<&'static str, String> {
    match journal_type {
        "context_created" => Ok("context"),
        "artifact_added" => Ok("evidence"),
        "decision_added" => Ok("decision"),
        "graph_decision" => Ok("decision"),
        "status_changed" => Ok("lifecycle"),
        "phase_transition" => Ok("lifecycle"),
        "context_blocked" => Ok("lifecycle"),
        "context_unblocked" => Ok("lifecycle"),
        "context_cancelled" => Ok("lifecycle"),
        "execution_interrupted" => Ok("lifecycle"),
        other => Err(format!(
            "journal event type {other:?} has no pinned SOMA SPEC 006 mapping \
             (the projection never guesses; extend the mapping table explicitly)"
        )),
    }
}

/// The Lite producer kind → SOMA `ActorKind` mapping. Exact and total for
/// every kind the write path still produces; the legacy stored `system`
/// producer has no honest SOMA representation and fails closed.
fn map_producer_kind(kind: &ProducerKind) -> Result<ActorKind, String> {
    match kind {
        ProducerKind::Human => Ok(ActorKind::Human),
        ProducerKind::Harness => Ok(ActorKind::Harness),
        ProducerKind::Tool => Ok(ActorKind::Tool),
        ProducerKind::Memory => Ok(ActorKind::Memory),
        ProducerKind::System => Err(
            "stored 'system' producer predates the write-time Harness fix and has \
             no honest SOMA ActorKind representation"
                .to_string(),
        ),
    }
}

/// The Lite execution class → SOMA v1.1 `executionClass` mapping
/// (approved plan §2.1). `HumanDecision` fails closed: SOMA v1.1 has no
/// human execution class (human decisions are represented by the actor
/// kind and decision/approval events), so mapping to any bucket would
/// change canonical semantic identity.
fn map_execution_class(class: &LiteExecutionClass) -> Result<SomaExecutionClass, String> {
    match class {
        LiteExecutionClass::Deterministic => Ok(SomaExecutionClass::Deterministic),
        LiteExecutionClass::ConstrainedModel => Ok(SomaExecutionClass::ModelAssisted),
        LiteExecutionClass::ScopedAgent => Ok(SomaExecutionClass::OpenEnded),
        LiteExecutionClass::HumanDecision => {
            Err("the HumanDecision execution class has no honest SOMA v1.1 \
             executionClass bucket — mapping it would change semantic identity"
                .to_string())
        }
    }
}

/// The repository binding → SOMA `repoRevision`. `Unbound` is the honest
/// empty string — never a fabricated revision.
fn map_repo_revision(binding: &RepoBinding) -> String {
    match binding {
        RepoBinding::Bound { revision } | RepoBinding::Dirty { revision, .. } => revision.clone(),
        RepoBinding::Unbound => String::new(),
    }
}

/// The SOMA `AuthorityProfile` projection of one Lite authority
/// descriptor: `executionClass` per §2.1, `mutation: "none"` (journal
/// event production exercises no SOMA mutation grant — the event
/// RECORDS effects, it does not perform them), and every other profile
/// field an honest omission (Lite autonomy/approval-policy semantics
/// have no SOMA v1.1 counterpart and are never invented).
fn map_authority(
    descriptor: &crate::work::provenance::AuthorityDescriptor,
) -> Result<AuthorityProfile, String> {
    Ok(AuthorityProfile {
        execution_class: map_execution_class(&descriptor.execution_class)?,
        mutation: MutationMode::None_,
        readable_scopes: None,
        writable_scopes: None,
        tools: None,
        network_policy: None,
        provider_policy: None,
        secrets: None,
        escalation: None,
        review: None,
        abstention: None,
        budgets: None,
        content_restrictions: None,
    })
}

/// Map ONE verified journal record to a SOMA `WorkEvent` (approved plan
/// §2 — every field's source or its documented omission). Pure: a
/// function of the record alone. Fail closed on anything that cannot be
/// honestly expressed.
pub fn map_record(record: &JournalRecord) -> Result<WorkEvent, ProjectionError> {
    let event_id = record.event.id.clone();
    let envelope = match &record.provenance {
        ProvenanceState::Verified(envelope) => envelope.as_ref(),
        ProvenanceState::LegacyUnverified => {
            return Err(ProjectionError::Unsupported {
                event_id,
                reason: "legacy row without provenance — the projection consumes \
                         verified Slice 1A records only and never fabricates an envelope"
                    .to_string(),
            });
        }
    };

    let unsupported = |reason: String| ProjectionError::Unsupported {
        event_id: event_id.clone(),
        reason,
    };

    let event_type = map_event_type(&record.event.event_type).map_err(unsupported)?;
    let actor_kind = map_producer_kind(&envelope.producer.kind).map_err(unsupported)?;
    let authority = map_authority(&envelope.authority.declared).map_err(unsupported)?;
    let effective_authority = map_authority(&envelope.authority.effective).map_err(unsupported)?;

    let mut work_event = WorkEvent {
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        version: SUPPORTED_SCHEMA_VERSION.to_string(),
        id: record.event.id.clone(),
        event_type: event_type.to_string(),
        actor: Actor {
            kind: actor_kind,
            identity: envelope.producer.identity.clone(),
            revision: None,
            implementation: envelope.producer.implementation.as_ref().map(|imp| {
                SomaImplementation {
                    name: imp.name.clone(),
                    revision: imp.revision.clone(),
                    build: imp.build.clone(),
                }
            }),
        },
        authority,
        effective_authority,
        // The durable seq IS the stable cursor; u64 by AUTOINCREMENT
        // construction (strictly positive, never reused).
        sequence: u64::try_from(record.seq)
            .map_err(|_| unsupported(format!("journal seq {} is not a u64", record.seq)))?,
        // The RAW stored timestamp text, byte-exact (review P1): the
        // projected event's timestamp IS the stored representation —
        // `Z` stays `Z`, `+00:00` stays `+00:00`. Never reformatted.
        timestamp: record.stored.created_at_text.clone(),
        idempotency_key: record.event.id.clone(),
        correlation_id: envelope.causation.correlation_id.clone(),
        repo_revision: map_repo_revision(&envelope.repo_binding),
        compatibility: Compatibility {
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            min_reader_version: None,
        },
        // Set below: the SPEC 006 semantic digest over the projected
        // content — never copied from the journal source digest.
        semantic_digest: Hex64::parse(&"0".repeat(64)).expect("64 zeros are valid hex"),
        // The REAL recorded causal parent — never sequence adjacency.
        parents: envelope.causation.parent_event_id.iter().cloned().collect(),
        // Honest omissions (approved plan §2): no journal row carries an
        // EvidenceReference, a distinct top-level implementation, a
        // SOMA-typed payload, or replay/conflict evidence.
        evidence: None,
        implementation: None,
        payload: None,
        replay: false,
        conflict: false,
    };

    let computed = work_event
        .computed_semantic_digest()
        .map_err(|e| unsupported(format!("semantic digest unavailable: {e}")))?;
    work_event.semantic_digest = Hex64::parse(&computed)
        .map_err(|e| unsupported(format!("computed digest invalid: {e}")))?;

    Ok(work_event)
}

/// A reconnectable stream segment — deliberately NOT a SOMA
/// `WorkEventBatch` (approved plan, correction 1): a page's causal
/// parents may live in earlier pages, so the batch type — whose audit
/// demands in-batch parent closure — would be a type-level lie. The
/// events inside are real `WorkEvent`s, each passing the full per-event
/// audit; only batch-level closure checks do not apply, because there
/// is no batch.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkEventStreamPage {
    /// Seq-ordered projected events (durable `seq` ASC).
    pub events: Vec<WorkEvent>,
}

/// One projected page: the versioned envelope wrapping a stream segment,
/// plus the durable cursor for reconnecting without gaps or duplication.
#[derive(Debug, Clone)]
pub struct ProjectionPage {
    pub envelope: crate::workflow::projection::VersionedProjectionEnvelope<WorkEventStreamPage>,
    /// The `after_seq` cursor for the next page (`None` = exhausted).
    /// `seq` is the journal's `INTEGER PRIMARY KEY AUTOINCREMENT`:
    /// monotonic, never reused, stable across VACUUM/rebuilds.
    pub next_after: Option<i64>,
}

/// The complete-record source-digest input for one verified record
/// (approved plan, correction 2; review P1): EVERY column the read path
/// returned, binding the RAW STORED BYTES — `data` as the raw SQLite
/// TEXT (never re-serialized from the parsed value: key order,
/// whitespace, and number lexemes are binding), `createdAt` as the raw
/// stored text (`Z` vs `+00:00` bind differently), the row's own
/// `sourceDigest` (chaining to the journal's row-level digest), and
/// the derived identity columns. Two journals differing in ANY stored
/// byte produce different projection source digests.
fn complete_record_source_value(record: &JournalRecord) -> serde_json::Value {
    serde_json::json!({
        "seq": record.seq,
        "id": record.event.id,
        "workContextId": record.event.work_context_id,
        "eventType": record.event.event_type,
        "data": record.stored.data_text,
        "createdAt": record.stored.created_at_text,
        "provenanceJson": record.stored.provenance_json,
        "sourceDigest": record.stored.source_digest,
        "runId": record.stored.run_id,
        "principalId": record.stored.principal_id,
        "correlationId": record.stored.correlation_id,
    })
}

/// Build the versioned envelope around a mapped projection payload:
/// `sourceDigest` binds the projection to the exact journal bytes it
/// was derived from; `projectionDigest` binds the emitted payload.
fn projection_envelope<V: serde::Serialize>(
    source: &serde_json::Value,
    payload: V,
) -> Result<crate::workflow::projection::VersionedProjectionEnvelope<V>, ProjectionError> {
    use crate::workflow::projection::{PROJECTION_VERSION_V1, VersionedProjectionEnvelope};
    let payload_value = serde_json::to_value(&payload).map_err(|e| {
        ProjectionError::Journal(anyhow::anyhow!("projection payload serialization: {e}"))
    })?;
    let source_digest =
        crate::workflow::soma::canonical::try_canonical_digest(source).map_err(|e| {
            ProjectionError::Journal(anyhow::anyhow!("projection source digest unavailable: {e}"))
        })?;
    let projection_digest = crate::workflow::soma::canonical::try_canonical_digest(&payload_value)
        .map_err(|e| {
            ProjectionError::Journal(anyhow::anyhow!("projection digest unavailable: {e}"))
        })?;
    Ok(VersionedProjectionEnvelope {
        projection_version: PROJECTION_VERSION_V1.to_string(),
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        source_digest,
        projection_digest,
        payload,
    })
}

/// The shared fail-closed core of every projection entry point
/// (binding requirement 2 — read-gate precedence): the verified journal
/// read ALWAYS runs first and its errors ALWAYS fail before any mapping
/// work; then each record maps and passes the full per-event audit.
fn map_verified_records<'a>(
    records: impl IntoIterator<Item = &'a JournalRecord>,
) -> Result<Vec<WorkEvent>, ProjectionError> {
    let supported = crate::workflow::soma::supported_version();
    let mut events = Vec::new();
    for record in records {
        let mapped = map_record(record)?;
        let diags = mapped.audit(&supported);
        if !diags.is_empty() {
            return Err(ProjectionError::Audit(diags));
        }
        events.push(mapped);
    }
    Ok(events)
}

/// Project up to `limit` (clamped 1..=500) verified journal events for
/// the context whose durable `seq` exceeds `after_seq` (0 = from the
/// beginning), ordered `seq` ASC. A reconnectable stream segment — see
/// [`WorkEventStreamPage`] for why this is not a `WorkEventBatch`.
///
/// Fail closed: tampered/mixed/corrupt rows fail in the verified read
/// gate BEFORE any projection work; legacy rows, unmapped event types,
/// the `HumanDecision` execution class, and stored `system` producers
/// fail with explicit `Unsupported` errors — never fabricated events,
/// never a partial page.
pub fn project_page(
    db: &crate::db::Db,
    context_id: &str,
    after_seq: i64,
    limit: usize,
) -> Result<ProjectionPage, ProjectionError> {
    use crate::db::repository::work_context_events::read_journal_records_conn;

    let clamped = limit.clamp(1, 500);
    // Read-gate precedence (binding requirement 2): the verified read
    // runs and fails FIRST; its errors are surfaced, never swallowed.
    let records = read_journal_records_conn(
        db.conn(),
        context_id,
        after_seq,
        i64::try_from(clamped + 1).unwrap_or(i64::MAX),
    )
    .map_err(ProjectionError::Journal)?;

    let exhausted = records.len() <= clamped;
    let page_records: &[JournalRecord] = if exhausted {
        &records
    } else {
        &records[..clamped]
    };

    let events = map_verified_records(page_records)?;
    let source = serde_json::Value::Array(
        page_records
            .iter()
            .map(complete_record_source_value)
            .collect(),
    );
    let next_after = page_records
        .last()
        .filter(|_| !exhausted)
        .map(|record| record.seq);
    let envelope = projection_envelope(&source, WorkEventStreamPage { events })?;

    Ok(ProjectionPage {
        envelope,
        next_after,
    })
}

// ---------------------------------------------------------------------------
// Typed run identity + per-run batches (correction 4 + binding
// requirement 1: differently typed run identities with equal strings
// must never silently merge).
// ---------------------------------------------------------------------------

/// The KIND of a recorded run identity — `work_run_id`, `graph_run_id`,
/// or `request_id` are three DIFFERENT identity types that happen to be
/// stored as strings. Grouping by the bare string would silently merge,
/// e.g., a work run and a graph run that share an id value; the typed
/// key keeps them distinct runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunKeyKind {
    WorkRun,
    GraphRun,
    Request,
}

/// A real recorded run identity: kind + id. Derives ONLY from the
/// envelope's `RunIdentity` — never from the work-context container.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunKey {
    pub kind: RunKeyKind,
    pub id: String,
}

impl RunKey {
    /// Derive the typed key from the envelope's real recorded identity
    /// (work run, else graph run, else request — the same precedence as
    /// the flat query column, but TYPED so equal strings of different
    /// kinds never merge).
    pub fn from_run_identity(run: &RunIdentity) -> Self {
        if let Some(id) = &run.work_run_id {
            Self {
                kind: RunKeyKind::WorkRun,
                id: id.clone(),
            }
        } else if let Some(id) = &run.graph_run_id {
            Self {
                kind: RunKeyKind::GraphRun,
                id: id.clone(),
            }
        } else {
            Self {
                kind: RunKeyKind::Request,
                id: run.request_id.clone(),
            }
        }
    }
}

impl JournalRecord {
    /// The record's typed run key, from its VERIFIED envelope only.
    fn verified_run_key(&self) -> Result<RunKey, ProjectionError> {
        match &self.provenance {
            ProvenanceState::Verified(envelope) => Ok(RunKey::from_run_identity(&envelope.run)),
            ProvenanceState::LegacyUnverified => Err(ProjectionError::Unsupported {
                event_id: self.event.id.clone(),
                reason: "legacy row without provenance — run identity is unavailable \
                         and the projection never fabricates one"
                    .to_string(),
            }),
        }
    }
}

/// The context's distinct recorded run keys, in first-appearance seq
/// order — the read-model rebuild enumeration. Fail closed on any
/// non-verified record (a context with legacy rows cannot be rebuilt;
/// silently listing runs that would then fail to project is a lie).
pub fn recorded_run_keys(
    db: &crate::db::Db,
    context_id: &str,
) -> Result<Vec<RunKey>, ProjectionError> {
    use crate::db::repository::work_context_events::read_journal_records_conn;
    let records = read_journal_records_conn(db.conn(), context_id, 0, i64::MAX)
        .map_err(ProjectionError::Journal)?;
    let mut keys: Vec<RunKey> = Vec::new();
    for record in &records {
        let key = record.verified_run_key()?;
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// Project the recorded run's event history as a causally-closed SOMA
/// `WorkEventBatch` with `runId` = the run's REAL recorded identity
/// (approved plan, correction 4 — never the work-context container).
///
/// The batch contains the run's own events PLUS the transitive causal
/// ancestors of those events that live outside the run (each ancestor
/// verbatim — its own correlation, actor, and run identity intact;
/// nothing rewritten, nothing invented, nothing erased). Parent
/// closure therefore holds BY CONSTRUCTION and the full vendored
/// `WorkEventBatch::audit` gate runs before the batch is returned; a
/// batch is never emitted dirty.
pub fn project_run_work_event_batch(
    db: &crate::db::Db,
    context_id: &str,
    run_key: &RunKey,
) -> Result<
    crate::workflow::projection::VersionedProjectionEnvelope<
        crate::workflow::soma::event::WorkEventBatch,
    >,
    ProjectionError,
> {
    use crate::db::repository::work_context_events::read_journal_records_conn;
    use crate::workflow::soma::event::WorkEventBatch;

    // Read-gate precedence: the verified read runs and fails FIRST.
    let records = read_journal_records_conn(db.conn(), context_id, 0, i64::MAX)
        .map_err(ProjectionError::Journal)?;

    // The run's own records (typed key from the parsed envelope — the
    // flat column is the index, the envelope is truth).
    let mut own: Vec<&JournalRecord> = Vec::new();
    for record in &records {
        if record.verified_run_key()? == *run_key {
            own.push(record);
        }
    }
    if own.is_empty() {
        return Err(ProjectionError::Unsupported {
            event_id: context_id.to_string(),
            reason: format!(
                "no recorded events for run key {run_key:?} — the projection never \
                 invents an empty run"
            ),
        });
    }

    // Causal-ancestor closure: pull in parents that live outside the
    // run, transitively, from the context's verified records. The walk
    // fails closed on unresolvable parents; cycles are caught by the
    // batch audit gate below (the vendored verifier is the authority).
    let by_id: std::collections::HashMap<&str, &JournalRecord> = records
        .iter()
        .map(|record| (record.event.id.as_str(), record))
        .collect();
    let mut selected: Vec<&JournalRecord> = own.clone();
    let mut seen_ids: std::collections::HashSet<String> =
        own.iter().map(|record| record.event.id.clone()).collect();
    for record in &own {
        let mut next = verified_parent_id(record)?;
        while let Some(parent_id) = next {
            if !seen_ids.insert(parent_id.clone()) {
                break; // already included (an own record or a visited ancestor)
            }
            let parent =
                by_id
                    .get(parent_id.as_str())
                    .ok_or_else(|| ProjectionError::Unsupported {
                        event_id: record.event.id.clone(),
                        reason: format!(
                            "causal parent {parent_id:?} does not resolve within the \
                         context's verified records"
                        ),
                    })?;
            selected.push(parent);
            next = verified_parent_id(parent)?;
        }
    }
    // Durable seq order for the batch.
    selected.sort_by_key(|record| record.seq);

    let events = map_verified_records(selected.iter().copied())?;
    let source = serde_json::Value::Array(
        selected
            .iter()
            .map(|record| complete_record_source_value(record))
            .collect(),
    );
    let batch = WorkEventBatch {
        schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
        version: SUPPORTED_SCHEMA_VERSION.to_string(),
        run_id: run_key.id.clone(),
        events,
        compatibility: Some(Compatibility {
            schema_version: SUPPORTED_SCHEMA_VERSION.to_string(),
            min_reader_version: None,
        }),
    };

    // The full audit gate (parent closure, cycles, per-event checks) —
    // a batch is never emitted dirty.
    let diags = batch.audit(&crate::workflow::soma::supported_version());
    if !diags.is_empty() {
        return Err(ProjectionError::Audit(diags));
    }

    projection_envelope(&source, batch)
}

/// The record's recorded causal parent id, from its VERIFIED envelope.
fn verified_parent_id(record: &JournalRecord) -> Result<Option<String>, ProjectionError> {
    match &record.provenance {
        ProvenanceState::Verified(envelope) => Ok(envelope.causation.parent_event_id.clone()),
        ProvenanceState::LegacyUnverified => Err(ProjectionError::Unsupported {
            event_id: record.event.id.clone(),
            reason: "legacy row without provenance — causation is unavailable".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repository::work_context_events::ProvenanceState;
    use crate::work::event::WorkContextEvent;
    use crate::work::provenance::{
        AuthorityDescriptor, AuthorityRecord, CausationRecord, JournalContext, PrincipalRef,
        Producer, ProducerKind, ProvenanceEnvelope, RepoBinding, RunIdentity,
    };
    use crate::work::types::{ApprovalPolicy, AutonomyLevel};
    use crate::workflow::soma::supported_version;

    fn record_with(event_type: &str, envelope: ProvenanceEnvelope) -> JournalRecord {
        JournalRecord {
            seq: 7,
            event: WorkContextEvent::new(
                "ev-1".to_string(),
                "ctx-1".to_string(),
                event_type.to_string(),
                serde_json::json!({ "from": "Draft", "to": "InProgress" }),
            ),
            provenance: ProvenanceState::Verified(Box::new(envelope)),
            stored: crate::db::repository::work_context_events::StoredColumns {
                data_text: r#"{"from":"Draft","to":"InProgress"}"#.to_string(),
                created_at_text: "2026-10-02T12:00:00+00:00".to_string(),
                ..Default::default()
            },
        }
    }

    fn harness_envelope() -> ProvenanceEnvelope {
        let ctx = JournalContext::internal_system(
            "req-1".to_string(),
            JournalContext::work_authority(AutonomyLevel::Chat, ApprovalPolicy::Auto),
        );
        ctx.event_envelope(None)
    }

    #[test]
    fn event_type_mapping_table_is_exact() {
        let expected = [
            ("context_created", "context"),
            ("artifact_added", "evidence"),
            ("decision_added", "decision"),
            ("graph_decision", "decision"),
            ("status_changed", "lifecycle"),
            ("phase_transition", "lifecycle"),
            ("context_blocked", "lifecycle"),
            ("context_unblocked", "lifecycle"),
            ("context_cancelled", "lifecycle"),
            ("execution_interrupted", "lifecycle"),
        ];
        for (journal_type, soma_type) in expected {
            let record = record_with(journal_type, harness_envelope());
            let mapped = map_record(&record).unwrap();
            assert_eq!(mapped.event_type, soma_type, "{journal_type}");
        }
    }

    #[test]
    fn unmapped_event_type_fails_closed() {
        let record = record_with("future_widget", harness_envelope());
        let err = map_record(&record).unwrap_err();
        match err {
            ProjectionError::Unsupported { event_id, reason } => {
                assert_eq!(event_id, "ev-1");
                assert!(
                    reason.contains("future_widget"),
                    "the error must name the unmapped type: {reason}"
                );
            }
            other => panic!("expected Unsupported, got {other}"),
        }
    }

    #[test]
    fn human_decision_execution_class_fails_closed() {
        let mut envelope = harness_envelope();
        let descriptor = AuthorityDescriptor {
            autonomy: AutonomyLevel::Chat,
            approval_policy: ApprovalPolicy::ManualAll,
            execution_class: LiteExecutionClass::HumanDecision,
        };
        envelope.authority = AuthorityRecord {
            declared: descriptor.clone(),
            effective: descriptor,
        };
        let record = record_with("status_changed", envelope);
        match map_record(&record) {
            Err(ProjectionError::Unsupported { reason, .. }) => {
                assert!(reason.contains("HumanDecision"), "{reason}")
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn stored_system_producer_fails_closed() {
        let mut envelope = harness_envelope();
        envelope.producer.kind = ProducerKind::System;
        let record = record_with("status_changed", envelope);
        match map_record(&record) {
            Err(ProjectionError::Unsupported { reason, .. }) => {
                assert!(reason.contains("system"), "{reason}")
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn legacy_unverified_record_fails_closed() {
        let record = JournalRecord {
            seq: 1,
            event: WorkContextEvent::new(
                "ev-legacy".to_string(),
                "ctx-1".to_string(),
                "status_changed".to_string(),
                serde_json::json!({}),
            ),
            provenance: ProvenanceState::LegacyUnverified,
            stored: crate::db::repository::work_context_events::StoredColumns::default(),
        };
        match map_record(&record) {
            Err(ProjectionError::Unsupported { event_id, reason }) => {
                assert_eq!(event_id, "ev-legacy");
                assert!(reason.contains("legacy"), "{reason}")
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn producer_maps_to_the_exact_actor() {
        let mut envelope = harness_envelope();
        envelope.producer = Producer {
            kind: ProducerKind::Human,
            identity: "user-1".to_string(),
            implementation: None,
        };
        let mapped = map_record(&record_with("status_changed", envelope)).unwrap();
        assert_eq!(mapped.actor.kind, ActorKind::Human);
        assert_eq!(mapped.actor.identity, "user-1");
        assert_eq!(mapped.actor.revision, None);
        assert_eq!(mapped.actor.implementation, None);

        // Harness producer carries the isomorphic implementation.
        let harness = map_record(&record_with("status_changed", harness_envelope())).unwrap();
        assert_eq!(harness.actor.kind, ActorKind::Harness);
        let imp = harness.actor.implementation.expect("harness has one");
        assert_eq!(imp.name, "prometheos-lite");
        assert_eq!(imp.revision, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn authority_profiles_map_execution_class_and_none_mutation() {
        let mapped = map_record(&record_with("status_changed", harness_envelope())).unwrap();
        // The work-path authority is ConstrainedModel -> model-assisted.
        assert_eq!(
            mapped.authority.execution_class,
            SomaExecutionClass::ModelAssisted
        );
        assert_eq!(mapped.authority.mutation, MutationMode::None_);
        assert_eq!(
            mapped.effective_authority.execution_class,
            SomaExecutionClass::ModelAssisted
        );
        assert_eq!(mapped.effective_authority.mutation, MutationMode::None_);
        // Every other profile field is an honest omission.
        assert_eq!(mapped.authority.readable_scopes, None);
        assert_eq!(mapped.authority.writable_scopes, None);
        assert_eq!(mapped.authority.tools, None);
        assert_eq!(mapped.authority.network_policy, None);
        assert_eq!(mapped.authority.provider_policy, None);
        assert_eq!(mapped.authority.secrets, None);
        assert_eq!(mapped.authority.escalation, None);
        assert_eq!(mapped.authority.review, None);
        assert_eq!(mapped.authority.abstention, None);
        assert_eq!(mapped.authority.budgets, None);
        assert_eq!(mapped.authority.content_restrictions, None);
    }

    #[test]
    fn unbound_repo_binding_projects_the_honest_empty_revision() {
        let mapped = map_record(&record_with("status_changed", harness_envelope())).unwrap();
        assert_eq!(mapped.repo_revision, "");

        let mut bound = harness_envelope();
        bound.repo_binding = RepoBinding::Bound {
            revision: "abc123".to_string(),
        };
        let mapped = map_record(&record_with("status_changed", bound)).unwrap();
        assert_eq!(mapped.repo_revision, "abc123");
    }

    #[test]
    fn causation_maps_the_real_recorded_parent() {
        let mut with_parent = harness_envelope();
        with_parent.causation = CausationRecord {
            correlation_id: "corr-1".to_string(),
            parent_event_id: Some("ev-parent".to_string()),
        };
        let mapped = map_record(&record_with("status_changed", with_parent)).unwrap();
        assert_eq!(mapped.parents, vec!["ev-parent".to_string()]);
        assert_eq!(mapped.correlation_id, "corr-1");

        let root = map_record(&record_with("status_changed", harness_envelope())).unwrap();
        assert_eq!(root.parents, Vec::<String>::new());
    }

    #[test]
    fn projected_event_audits_clean_per_event() {
        let mapped = map_record(&record_with("execution_interrupted", harness_envelope())).unwrap();
        assert!(mapped.audit(&supported_version()).is_empty());
    }

    #[test]
    fn sequence_timestamp_and_idempotency_carry_durable_identity() {
        let record = record_with("status_changed", harness_envelope());
        let mapped = map_record(&record).unwrap();
        assert_eq!(mapped.sequence, 7);
        assert_eq!(mapped.idempotency_key, "ev-1");
        // The RAW stored timestamp text, byte-exact (review P1) — the
        // projected event's timestamp IS the stored representation,
        // never a parsed/re-rendered normalization of it.
        assert_eq!(mapped.timestamp, record.stored.created_at_text);
        assert_eq!(mapped.timestamp, "2026-10-02T12:00:00+00:00");
        assert_eq!(mapped.schema_version, SUPPORTED_SCHEMA_VERSION);
        assert_eq!(mapped.version, SUPPORTED_SCHEMA_VERSION);
        assert_eq!(
            mapped.compatibility.schema_version,
            SUPPORTED_SCHEMA_VERSION
        );
        assert_eq!(mapped.compatibility.min_reader_version, None);
        assert_eq!(mapped.evidence, None);
        assert_eq!(mapped.implementation, None);
        assert_eq!(mapped.payload, None);
        assert!(!mapped.replay);
        assert!(!mapped.conflict);
    }

    #[test]
    fn semantic_digest_is_computed_over_the_projected_content() {
        let record = record_with("decision_added", harness_envelope());
        let mapped = map_record(&record).unwrap();
        let recomputed = mapped.computed_semantic_digest().unwrap();
        assert_eq!(mapped.semantic_digest.as_str(), recomputed);
        assert_ne!(
            mapped.semantic_digest.as_str(),
            crate::work::provenance::compute_event_source_digest(
                &record.event.id,
                &record.event.work_context_id,
                &record.event.event_type,
                &record.event.data,
                &record.event.created_at.to_rfc3339(),
                &harness_envelope().to_canonical_json_string().unwrap(),
            )
            .unwrap(),
            "the SPEC 006 semantic digest is NOT the journal source digest"
        );
    }

    #[test]
    fn run_identity_fields_are_never_invented() {
        // The envelope's run identities (work run / graph run / request)
        // have no WorkEvent field; they are preserved in the durable
        // journal and bind through the projection envelope digests —
        // never fabricated into a SOMA field.
        let envelope = ProvenanceEnvelope {
            schema_version: crate::work::provenance::PROVENANCE_SCHEMA_VERSION.to_string(),
            producer: Producer {
                kind: ProducerKind::Harness,
                identity: "prometheos-lite".to_string(),
                implementation: None,
            },
            principal: PrincipalRef::Absent,
            causation: CausationRecord {
                correlation_id: "corr-1".to_string(),
                parent_event_id: None,
            },
            authority: {
                let d = AuthorityDescriptor {
                    autonomy: AutonomyLevel::Chat,
                    approval_policy: ApprovalPolicy::Auto,
                    execution_class: LiteExecutionClass::ConstrainedModel,
                };
                AuthorityRecord {
                    declared: d.clone(),
                    effective: d,
                }
            },
            repo_binding: RepoBinding::Unbound,
            run: RunIdentity {
                work_run_id: Some("work-run-9".to_string()),
                graph_run_id: None,
                request_id: "req-9".to_string(),
            },
        };
        let mapped = map_record(&record_with("status_changed", envelope)).unwrap();
        // The correlation groups the logical operation; nothing else
        // about the run leaked into invented fields.
        assert_eq!(mapped.correlation_id, "corr-1");
        assert_eq!(mapped.evidence, None);
        assert_eq!(mapped.payload, None);
    }
}
