//! Slice 1A (#132): the durable provenance envelope for journal events.
//!
//! Every `work_context_events` write carries a versioned, closed, typed
//! provenance envelope — recorded at write time, never inferred later:
//! who/what produced the event (the producer), which human initiated it
//! (the principal, with an honest typed absence when none did — never a
//! fabricated identity), correlation and causation (a real parent event
//! id or an explicit absence — never derived from sequence adjacency),
//! the declared AND effective authority at write time, an explicit
//! repository binding (`bound` / `dirty` with a deterministic workspace
//! digest / `unbound`), and the real run identities (work run, graph run,
//! request — preserved distinctly, never collapsed).
//!
//! The envelope serializes to canonical JSON bytes that are validated
//! against the SOMA number policy at the write boundary; the complete
//! source event (all writer-controlled fields) is digest-locked via
//! [`compute_event_source_digest`] and re-verified on read.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::work::types::{ApprovalPolicy, AutonomyLevel};
use crate::workflow::soma::canonical;

/// The envelope's own version identity — evolves independently of the
/// event payload schema (the flat columns stay stable; structure grows
/// inside this versioned envelope).
pub const PROVENANCE_SCHEMA_VERSION: &str = "1.0.0";

/// Who or what produced the event. `Human` is a valid producer: a human
/// acting directly (e.g. canceling a context by request) is recorded as
/// the producer, not laundered through a system identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum ProducerKind {
    Human,
    Harness,
    Tool,
    Memory,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Producer {
    pub kind: ProducerKind,
    pub identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<Implementation>,
}

/// The initiating human principal. Absence is an honest, typed state —
/// never a fabricated identity such as "system" or "unknown".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum PrincipalRef {
    Human { identity: String },
    Absent,
}

/// Correlation and causation, recorded at write time. `parent_event_id`
/// is a REAL recorded parent event id when the event is causally
/// downstream of another journal event, and an explicit absence
/// otherwise. Sequence numbers order the log; they never encode
/// causality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CausationRecord {
    pub correlation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
}

/// The closed execution-class vocabulary for authority descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum ExecutionClass {
    Deterministic,
    ConstrainedModel,
    ScopedAgent,
    HumanDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AuthorityDescriptor {
    pub autonomy: AutonomyLevel,
    pub approval_policy: ApprovalPolicy,
    pub execution_class: ExecutionClass,
}

/// Declared and effective authority, stored separately even when equal:
/// `declared` is the invocation's envelope; `effective` is what was
/// actually exercised at write time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRecord {
    pub declared: AuthorityDescriptor,
    pub effective: AuthorityDescriptor,
}

/// Explicit repository binding. `Dirty` records the HEAD revision plus a
/// deterministic digest of the dirty workspace (same workspace state →
/// same digest); `Unbound` is the recorded fact that no repository
/// binding exists — never a fabricated revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum RepoBinding {
    Bound {
        revision: String,
    },
    Dirty {
        revision: String,
        #[serde(rename = "workspaceDigest")]
        workspace_digest: String,
    },
    Unbound,
}

/// Real run identities, preserved distinctly — never collapsed into one
/// generic request id. `request_id` is required: every journal event
/// traces to some originating request (an HTTP request, a CLI command,
/// an internal invocation). `work_run_id` is a run-loop invocation;
/// `graph_run_id` is the graph-run identity (checkpoint registry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_run_id: Option<String>,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProvenanceEnvelope {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub producer: Producer,
    pub principal: PrincipalRef,
    pub causation: CausationRecord,
    pub authority: AuthorityRecord,
    #[serde(rename = "repoBinding")]
    pub repo_binding: RepoBinding,
    pub run: RunIdentity,
}

impl ProvenanceEnvelope {
    /// Serialize to canonical JSON bytes, validated against the SOMA
    /// number policy at the boundary. The typed structures contain only
    /// strings and enum names, but the byte validation still runs: an
    /// envelope that fails canonical byte rules can never be stored.
    pub fn to_canonical_json_string(&self) -> Result<String> {
        let text = serde_json::to_string(self).context("serializing provenance envelope")?;
        canonical::validate_number_lexemes(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("provenance envelope byte validation failed: {e}"))?;
        Ok(text)
    }

    /// Parse strictly: unknown fields, a wrong schema version, or a
    /// number-policy violation in the stored bytes is refused.
    pub fn parse_canonical(text: &str) -> Result<Self> {
        canonical::validate_number_lexemes(text.as_bytes())?;
        let envelope: ProvenanceEnvelope = serde_json::from_str(text)
            .with_context(|| format!("parsing provenance envelope: {text}"))?;
        if envelope.schema_version != PROVENANCE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported provenance schema version: {} (supported: {PROVENANCE_SCHEMA_VERSION})",
                envelope.schema_version
            );
        }
        Ok(envelope)
    }

    /// The flat `run_id` query column: the effective run identity for
    /// query purposes (work run, else graph run, else request) — a
    /// derived key; the real identities live in the envelope.
    pub fn run_query_key(&self) -> String {
        if let Some(id) = &self.run.work_run_id {
            return id.clone();
        }
        if let Some(id) = &self.run.graph_run_id {
            return id.clone();
        }
        self.run.request_id.clone()
    }

    /// The flat `principal_id` query column: `None` when the principal
    /// is honestly absent (the column matches the envelope exactly).
    pub fn principal_query_key(&self) -> Option<String> {
        match &self.principal {
            PrincipalRef::Human { identity } => Some(identity.clone()),
            PrincipalRef::Absent => None,
        }
    }

    pub fn correlation_query_key(&self) -> String {
        self.causation.correlation_id.clone()
    }
}

/// The canonical source-event digest input: the COMPLETE writer-controlled
/// journal record — `{id, workContextId, eventType, data, createdAt,
/// provenanceJson}` — every field the writer controls, including the full
/// provenance envelope. The store-assigned `seq` cursor and the digest
/// column itself are excluded by construction (placement and self,
/// not writer content).
pub fn compute_event_source_digest(
    id: &str,
    work_context_id: &str,
    event_type: &str,
    data: &serde_json::Value,
    created_at: &str,
    provenance_json: &str,
) -> Result<String> {
    let record = serde_json::json!({
        "id": id,
        "workContextId": work_context_id,
        "eventType": event_type,
        "data": data,
        "createdAt": created_at,
        "provenanceJson": provenance_json,
    });
    crate::workflow::soma::try_canonical_digest(&record)
        .map_err(|e| anyhow::anyhow!("source event digest unavailable: {e}"))
}

/// The per-invocation provenance base: one is built per API request, CLI
/// command, or run-loop invocation, and every journal event written
/// during that invocation derives its envelope from it — keeping
/// request-to-run correlation continuous.
#[derive(Debug, Clone)]
pub struct JournalContext {
    pub producer: Producer,
    pub principal: PrincipalRef,
    pub run: RunIdentity,
    pub authority: AuthorityRecord,
    pub repo_binding: RepoBinding,
}

impl JournalContext {
    /// An API request context: the requesting user is the principal (and
    /// the producer when the human acts directly, e.g. cancel).
    pub fn for_request(principal_id: &str, request_id: String, authority: AuthorityRecord) -> Self {
        Self {
            producer: Producer {
                kind: ProducerKind::Human,
                identity: principal_id.to_string(),
                implementation: None,
            },
            principal: PrincipalRef::Human {
                identity: principal_id.to_string(),
            },
            run: RunIdentity {
                work_run_id: None,
                graph_run_id: None,
                request_id,
            },
            authority,
            repo_binding: RepoBinding::Unbound,
        }
    }

    /// A run-loop context: the runtime harness produces the run's events;
    /// the initiating principal (from the run request) is preserved, and
    /// the work-run identity rides along with the originating request id.
    pub fn for_work_run(
        principal_id: &str,
        request_id: String,
        work_run_id: String,
        authority: AuthorityRecord,
    ) -> Self {
        Self {
            producer: Producer {
                kind: ProducerKind::Harness,
                identity: "prometheos-lite".to_string(),
                implementation: Some(Implementation {
                    name: "prometheos-lite".to_string(),
                    revision: env!("CARGO_PKG_VERSION").to_string(),
                    build: None,
                }),
            },
            principal: PrincipalRef::Human {
                identity: principal_id.to_string(),
            },
            run: RunIdentity {
                work_run_id: Some(work_run_id),
                graph_run_id: None,
                request_id,
            },
            authority,
            repo_binding: RepoBinding::Unbound,
        }
    }

    /// Internal-system context: for invocations with no initiating human
    /// (internal maintenance, test harnesses). The principal absence is
    /// the honest recorded state — never a fabricated identity.
    pub fn internal_system(request_id: String, authority: AuthorityRecord) -> Self {
        Self {
            producer: Producer {
                kind: ProducerKind::System,
                identity: "prometheos-lite".to_string(),
                implementation: Some(Implementation {
                    name: "prometheos-lite".to_string(),
                    revision: env!("CARGO_PKG_VERSION").to_string(),
                    build: None,
                }),
            },
            principal: PrincipalRef::Absent,
            run: RunIdentity {
                work_run_id: None,
                graph_run_id: None,
                request_id,
            },
            authority,
            repo_binding: RepoBinding::Unbound,
        }
    }

    /// Derive one event's envelope from this invocation context. The
    /// correlation id groups the logical operation (the work run, else
    /// the request); the parent event id is a REAL recorded causal
    /// parent or an explicit absence.
    pub fn event_envelope(&self, parent_event_id: Option<String>) -> ProvenanceEnvelope {
        let correlation_id = self
            .run
            .work_run_id
            .clone()
            .unwrap_or_else(|| self.run.request_id.clone());
        ProvenanceEnvelope {
            schema_version: PROVENANCE_SCHEMA_VERSION.to_string(),
            producer: self.producer.clone(),
            principal: self.principal.clone(),
            causation: CausationRecord {
                correlation_id,
                parent_event_id,
            },
            authority: self.authority.clone(),
            repo_binding: self.repo_binding.clone(),
            run: self.run.clone(),
        }
    }

    /// The envelope for an event bound to a graph run: the graph-run
    /// identity is recorded alongside the work-run/request identities.
    pub fn graph_run_envelope(
        &self,
        graph_run_id: String,
        parent_event_id: Option<String>,
    ) -> ProvenanceEnvelope {
        let mut envelope = self.event_envelope(parent_event_id);
        envelope.run.graph_run_id = Some(graph_run_id);
        envelope
    }

    /// Build the default authority record for the work path from the
    /// context's autonomy and approval policy.
    pub fn work_authority(
        autonomy: AutonomyLevel,
        approval_policy: ApprovalPolicy,
    ) -> AuthorityRecord {
        let descriptor = AuthorityDescriptor {
            autonomy,
            approval_policy,
            execution_class: ExecutionClass::ConstrainedModel,
        };
        AuthorityRecord {
            declared: descriptor.clone(),
            effective: descriptor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> ProvenanceEnvelope {
        let ctx = JournalContext::for_work_run(
            "user-1",
            "req-1".to_string(),
            "work-run-1".to_string(),
            JournalContext::work_authority(AutonomyLevel::Review, ApprovalPolicy::Auto),
        );
        ctx.event_envelope(Some("parent-event-1".to_string()))
    }

    #[test]
    fn envelope_round_trips_through_canonical_bytes() {
        let envelope = sample_envelope();
        let text = envelope.to_canonical_json_string().unwrap();
        assert!(text.contains("\"schemaVersion\":\"1.0.0\""));
        let parsed = ProvenanceEnvelope::parse_canonical(&text).unwrap();
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn parse_refuses_unknown_fields_and_wrong_versions() {
        let text = sample_envelope().to_canonical_json_string().unwrap();
        let with_extra = text.replace(
            "\"schemaVersion\":\"1.0.0\"",
            "\"schemaVersion\":\"1.0.0\",\"unexpected\":1",
        );
        assert!(ProvenanceEnvelope::parse_canonical(&with_extra).is_err());

        let wrong_version = text.replace("\"1.0.0\"", "\"2.0.0\"");
        assert!(ProvenanceEnvelope::parse_canonical(&wrong_version).is_err());
    }

    #[test]
    fn run_query_key_prefers_real_identities_in_order() {
        let envelope = sample_envelope();
        assert_eq!(envelope.run_query_key(), "work-run-1");

        let mut no_work = envelope.clone();
        no_work.run.work_run_id = None;
        assert_eq!(no_work.run_query_key(), "req-1");

        let ctx = JournalContext::internal_system(
            "req-2".to_string(),
            JournalContext::work_authority(AutonomyLevel::Chat, ApprovalPolicy::ManualAll),
        );
        let envelope = ctx.event_envelope(None);
        assert_eq!(envelope.run_query_key(), "req-2");
    }

    #[test]
    fn absent_principal_matches_a_null_query_column() {
        let ctx = JournalContext::internal_system(
            "req-3".to_string(),
            JournalContext::work_authority(AutonomyLevel::Chat, ApprovalPolicy::Auto),
        );
        let envelope = ctx.event_envelope(None);
        assert_eq!(envelope.principal_query_key(), None);
        assert!(matches!(envelope.principal, PrincipalRef::Absent));
    }

    #[test]
    fn source_digest_covers_the_complete_record_not_the_payload_alone() {
        let provenance = sample_envelope().to_canonical_json_string().unwrap();
        let base = compute_event_source_digest(
            "ev-1",
            "ctx-1",
            "status_changed",
            &serde_json::json!({ "from": "Draft" }),
            "2026-09-25T00:00:00Z",
            &provenance,
        )
        .unwrap();

        // Same payload, different provenance → different digest (the
        // digest is not payload-only).
        let other_provenance = JournalContext::internal_system(
            "req-x".to_string(),
            JournalContext::work_authority(AutonomyLevel::Chat, ApprovalPolicy::Auto),
        )
        .event_envelope(None)
        .to_canonical_json_string()
        .unwrap();
        let with_other = compute_event_source_digest(
            "ev-1",
            "ctx-1",
            "status_changed",
            &serde_json::json!({ "from": "Draft" }),
            "2026-09-25T00:00:00Z",
            &other_provenance,
        )
        .unwrap();
        assert_ne!(base, with_other);

        // Any writer-controlled field change changes the digest.
        let tampered_type = compute_event_source_digest(
            "ev-1",
            "ctx-1",
            "context_cancelled",
            &serde_json::json!({ "from": "Draft" }),
            "2026-09-25T00:00:00Z",
            &provenance,
        )
        .unwrap();
        assert_ne!(base, tampered_type);
    }

    #[test]
    fn number_policy_violations_fail_byte_validation() {
        // A programmatically built envelope cannot violate the policy,
        // but the byte validation gate itself must refuse one that does.
        let text = sample_envelope().to_canonical_json_string().unwrap();
        let bad = format!("{text} 1e999 ");
        assert!(ProvenanceEnvelope::parse_canonical(&bad).is_err());
    }
}
