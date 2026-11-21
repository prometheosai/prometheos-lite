//! Domain-owned persistence contract for the durable event journal.
//!
//! The `db` layer may depend on this module and on `work::event`; it must
//! not depend on any other `work` module. The contract exists so the
//! persistence layer depends on one small, named domain surface instead
//! of the provenance implementation (see the architecture-hardening
//! dispatch, finding A-001).
//!
//! The single verb of the contract is [`seal_journal_entry`]: everything
//! the store must write for one event - the canonical envelope bytes,
//! the whole-record source digest, and the flat identity columns - is
//! computed once, at the domain boundary, and handed to `db` as a plain
//! data structure. `db` performs persistence; it does not decide how
//! envelope semantics work.

use anyhow::Result;

use crate::work::event::WorkContextEvent;
pub use crate::work::provenance::{
    JournalContext, ProvenanceEnvelope, compute_event_source_digest,
};

/// The exact values the persistence layer must store for one journal row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedJournalEntry {
    /// Canonical, write-validated envelope bytes (stored verbatim).
    pub provenance_json: String,
    /// Whole-record source digest (re-verified on read).
    pub source_digest: String,
    /// Flat identity columns derived from the envelope (must match it).
    pub run_id: String,
    pub principal_id: Option<String>,
    pub correlation_id: String,
}

/// Seal a journal write at the domain boundary: validate the envelope's
/// write invariants, serialize it canonically, digest-lock the entire
/// writer-controlled record (id, context, type, data, timestamp and the
/// envelope itself) and derive the flat identity columns. This is the
/// only place `db` obtains these values.
pub fn seal_journal_entry(
    event: &WorkContextEvent,
    envelope: &ProvenanceEnvelope,
) -> Result<SealedJournalEntry> {
    envelope.validate_write_invariants()?;
    let provenance_json = envelope.to_canonical_json_string()?;
    let source_digest = compute_event_source_digest(
        &event.id,
        &event.work_context_id,
        &event.event_type,
        &event.data,
        &event.created_at.to_rfc3339(),
        &provenance_json,
    )?;
    Ok(SealedJournalEntry {
        provenance_json,
        source_digest,
        run_id: envelope.run_query_key(),
        principal_id: envelope.principal_query_key(),
        correlation_id: envelope.correlation_query_key(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::types::{ApprovalPolicy, AutonomyLevel};

    fn envelope() -> ProvenanceEnvelope {
        JournalContext::internal_system(
            format!("test-{}", uuid::Uuid::new_v4()),
            JournalContext::work_authority(AutonomyLevel::Review, ApprovalPolicy::Auto),
        )
        .event_envelope(None)
    }

    fn event() -> WorkContextEvent {
        WorkContextEvent {
            id: format!("evt-{}", uuid::Uuid::new_v4()),
            work_context_id: "ctx-1".to_string(),
            event_type: "context_created".to_string(),
            data: serde_json::json!({"k": "v"}),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn sealed_entry_is_consistent_with_the_envelope() {
        let event = event();
        let envelope = envelope();
        let sealed = seal_journal_entry(&event, &envelope).expect("valid envelope seals");
        assert_eq!(sealed.source_digest.len(), 64);
        assert!(sealed.source_digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(sealed.run_id, envelope.run_query_key());
        assert_eq!(sealed.principal_id, envelope.principal_query_key());
        assert_eq!(sealed.correlation_id, envelope.correlation_query_key());
        let parsed: serde_json::Value =
            serde_json::from_str(&sealed.provenance_json).expect("canonical bytes parse");
        assert_eq!(
            parsed.get("schemaVersion").and_then(|v| v.as_str()),
            Some("1.0.0")
        );
    }

    #[test]
    fn digest_binds_every_writer_controlled_field() {
        let envelope = envelope();
        let base_event = event();
        let base = seal_journal_entry(&base_event, &envelope).expect("seals");
        let fields = [
            ("id", {
                let mut e = base_event.clone();
                e.id = format!("evt-{}", uuid::Uuid::new_v4());
                e
            }),
            ("work_context_id", {
                let mut e = base_event.clone();
                e.work_context_id = "ctx-2".to_string();
                e
            }),
            ("event_type", {
                let mut e = base_event.clone();
                e.event_type = "status_changed".to_string();
                e
            }),
            ("data", {
                let mut e = base_event.clone();
                e.data = serde_json::json!({"k": "w"});
                e
            }),
            ("created_at", {
                let mut e = base_event.clone();
                e.created_at = base_event.created_at + chrono::Duration::seconds(1);
                e
            }),
        ];
        for (name, mutated) in fields {
            let sealed = seal_journal_entry(&mutated, &envelope).expect("seals");
            assert_ne!(
                sealed.source_digest, base.source_digest,
                "digest must bind {name}"
            );
        }
    }
}
