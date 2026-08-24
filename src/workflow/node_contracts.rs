//! Lite-owned node runtime contracts (`lite.node.v1`) for #116.
//!
//! OWNERSHIP: no published SOMA++ family exists for node runtime
//! manifests/results. Everything here is explicitly **Lite-owned** and MUST
//! NOT be presented as canonical SOMA++. Alignment by semantics/name only:
//! - authority mirrors SOMA AuthorityProfile conventions (scopes + budgets);
//! - terminal outcomes stay COMPATIBLE with SOMA TypedOutcome categories
//!   (completed/failed/blocked/review-required/cancelled) without redefining
//!   upstream semantics;
//! - evidence references reuse [`EvidenceReferenceV1`] (the canonical SOMA v1
//!   EvidenceReference mirror from lite.memory.v1) verbatim;
//! - memory operations are the typed #152 MemoryQuery/MemoryWrite - never
//!   implementation-specific storage calls.
//!
//! Retry/route/runtime metadata is explicitly Lite-only. When #159 lands,
//! explicit mappings + conformance tests must prove no conflict with the
//! canonical SOMA++ AST.
//!
//! Fail-closed: unknown fields rejected; major versions above 1 rejected.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::{EvidenceReferenceV1, MemoryQuery, MemoryWrite};

/// Version of the lite.node contract family.
pub const NODE_CONTRACT_VERSION: &str = "1.0.0";
pub const NODE_CONTRACT_MAJOR: u64 = 1;

fn ensure_supported_node_version(v: &str) -> Result<()> {
    let sv = crate::workflow::schema::SchemaVersion::parse(v)
        .with_context(|| format!("invalid lite.node schema_version {v:?}"))?;
    let ceiling =
        crate::workflow::schema::SchemaVersion::new(NODE_CONTRACT_MAJOR as u32, u32::MAX, u32::MAX);
    if sv > ceiling {
        bail!(
            "unsupported lite.node contract version {v}: major above {NODE_CONTRACT_MAJOR} (fail closed)"
        );
    }
    Ok(())
}

/// Declared input or output of a node (portable, provider-neutral).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeIo {
    pub name: String,
    /// Portable type label (e.g. "lite.memory.v1.RetrievalResult").
    pub type_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Retryability policy: which failure classes may retry and how often.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    /// Failure classifications eligible for retry (Lite-owned labels).
    pub retryable_classes: Vec<String>,
}

/// Terminal outcome categories, compatible with SOMA typed-outcome
/// semantics without redefining them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Completed,
    Failed,
    Blocked,
    ReviewRequired,
    Cancelled,
}

impl OutcomeKind {
    /// True when a `reason` string is REQUIRED for this outcome.
    pub fn requires_reason(&self) -> bool {
        !matches!(self, OutcomeKind::Completed)
    }
}

/// The declared execution contract for one node (pre-execution).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeManifestV1 {
    pub schema_version: String,
    pub node_id: String,
    pub purpose: String,
    pub inputs: Vec<NodeIo>,
    pub outputs: Vec<NodeIo>,
    /// SOMA-AuthorityProfile-aligned authorization scope lists.
    pub readable_scopes: Vec<String>,
    pub writable_scopes: Vec<String>,
    /// SOMA budgets.token analog for this node's context+work.
    #[serde(default)]
    pub token_budget: Option<u64>,
    pub retry: RetryPolicy,
    /// Governed memory reads (typed #152 operations).
    #[serde(default)]
    pub memory_reads: Vec<MemoryQuery>,
    /// Governed memory writes (typed #152 operations).
    #[serde(default)]
    pub memory_writes: Vec<MemoryWrite>,
    /// Versioned reference to a PortableWorkState document (optional).
    #[serde(default)]
    pub work_state_ref: Option<String>,
    /// Lite-only routing hints (explicitly NOT canonical).
    #[serde(default)]
    pub next_route_hints: Vec<String>,
}

impl NodeManifestV1 {
    pub fn parse_json(json: &str) -> Result<Self> {
        let m: Self =
            serde_json::from_str(json).context("failed to parse lite.node NodeManifest")?;
        ensure_supported_node_version(&m.schema_version)?;
        if m.node_id.is_empty() {
            bail!("node_id must not be empty");
        }
        if m.readable_scopes.is_empty() && m.writable_scopes.is_empty() {
            bail!("manifest carries no scopes: nothing is authorized");
        }
        Ok(m)
    }
}

/// The recorded execution result for one node (post-execution).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeResultV1 {
    pub schema_version: String,
    pub node_id: String,
    pub outcome: OutcomeKind,
    /// Required for every outcome except Completed.
    pub reason: String,
    /// Portable outputs actually produced.
    pub outputs: Vec<NodeIo>,
    /// Evidence references preserving canonical identity where used.
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceReferenceV1>,
    /// Counts of governed memory operations executed (audit).
    pub memory_reads_executed: u32,
    pub memory_writes_executed: u32,
    /// Versioned PortableWorkState pointer produced/updated (optional).
    #[serde(default)]
    pub work_state_ref: Option<String>,
    /// Failure classification for resource/policy breaches (optional;
    /// uses the same taxonomy as evaluation records when present).
    #[serde(default)]
    pub failure_classification: Option<String>,
    pub started_at: String,
    pub completed_at: String,
    /// SHA-256 over the canonical form excluding this field (audit digest).
    pub result_digest: String,
}

impl NodeResultV1 {
    pub fn parse_json(json: &str) -> Result<Self> {
        let r: Self = serde_json::from_str(json).context("failed to parse lite.node NodeResult")?;
        ensure_supported_node_version(&r.schema_version)?;
        if r.node_id.is_empty() {
            bail!("node_id must not be empty");
        }
        if r.outcome.requires_reason() && r.reason.is_empty() {
            bail!(
                "outcome {} requires a non-empty reason",
                serde_json::to_string(&r.outcome)?
            );
        }
        Ok(r)
    }

    /// Deterministic audit digest over the canonical form of every field
    /// EXCEPT `result_digest` itself (recomputable by receivers).
    pub fn compute_digest(&self) -> Result<String> {
        let pre = serde_json::json!({
            "completedAt": self.completed_at,
            "evidenceRefs": self.evidence_refs,
            "failureClassification": self.failure_classification,
            "memoryReadsExecuted": self.memory_reads_executed,
            "memoryWritesExecuted": self.memory_writes_executed,
            "nodeId": self.node_id,
            "outcome": self.outcome,
            "outputs": self.outputs,
            "reason": self.reason,
            "schemaVersion": self.schema_version,
            "startedAt": self.started_at,
            "workStateRef": self.work_state_ref,
        });
        crate::workflow::memory_contracts::canonical_digest(&pre)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json() -> String {
        serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": "node-1",
            "purpose": "retrieve repo evidence",
            "inputs": [{"name": "task", "typeRef": "core.String"}],
            "outputs": [{"name": "bundle", "typeRef": "lite.memory.v1.ContextBundle"}],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "tokenBudget": 2048,
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]},
            "memoryReads": [{
                "schemaVersion": "1.0.0", "queryId": "qr-1",
                "readableScopes": ["project://demo"], "text": "decisions"
            }],
            "nextRouteHints": ["validate"]
        })
        .to_string()
    }

    #[test]
    fn manifest_roundtrip_and_gate() {
        let m = NodeManifestV1::parse_json(&manifest_json()).unwrap();
        assert_eq!(m.node_id, "node-1");
        assert_eq!(m.memory_reads.len(), 1);
        let parsed = NodeManifestV1::parse_json(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn manifest_future_major_fails_closed() {
        let mut v: serde_json::Value = serde_json::from_str(&manifest_json()).unwrap();
        v["schemaVersion"] = "3.0.0".into();
        let err = NodeManifestV1::parse_json(&v.to_string()).unwrap_err();
        assert!(err.to_string().contains("fail closed"), "{err}");
    }

    #[test]
    fn manifest_unknown_field_fails_closed() {
        let mut v: serde_json::Value = serde_json::from_str(&manifest_json()).unwrap();
        v["sneakyExtra"] = 1.into();
        assert!(NodeManifestV1::parse_json(&v.to_string()).is_err());
    }

    #[test]
    fn manifest_without_any_scope_is_rejected() {
        let mut v: serde_json::Value = serde_json::from_str(&manifest_json()).unwrap();
        v["readableScopes"] = serde_json::json!([]);
        v["writableScopes"] = serde_json::json!([]);
        let err = NodeManifestV1::parse_json(&v.to_string()).unwrap_err();
        assert!(err.to_string().contains("nothing is authorized"), "{err}");
    }

    fn sample_result(outcome: OutcomeKind, reason: &str) -> NodeResultV1 {
        NodeResultV1 {
            schema_version: NODE_CONTRACT_VERSION.into(),
            node_id: "node-1".into(),
            outcome,
            reason: reason.into(),
            outputs: vec![],
            evidence_refs: vec![],
            memory_reads_executed: 1,
            memory_writes_executed: 0,
            work_state_ref: Some("pws:w-1".into()),
            failure_classification: None,
            started_at: "2026-08-24T00:00:00Z".into(),
            completed_at: "2026-08-24T00:00:01Z".into(),
            result_digest: String::new(),
        }
    }

    #[test]
    fn result_digest_is_recomputable_and_content_sensitive() {
        let mut r = sample_result(OutcomeKind::Completed, "");
        r.result_digest = r.compute_digest().unwrap();
        let again = r.compute_digest().unwrap();
        assert_eq!(r.result_digest, again);
        let mut other = r.clone();
        other.memory_reads_executed += 1;
        assert_ne!(other.compute_digest().unwrap(), r.result_digest);
    }

    #[test]
    fn non_completed_outcomes_require_reason() {
        for (o, ok) in [
            (OutcomeKind::Completed, true),
            (OutcomeKind::Failed, false),
            (OutcomeKind::Blocked, false),
            (OutcomeKind::ReviewRequired, false),
            (OutcomeKind::Cancelled, false),
        ] {
            let r = sample_result(o, if ok { "" } else { "because" });
            let parsed = NodeResultV1::parse_json(&serde_json::to_string(&r).unwrap());
            if ok {
                assert!(parsed.is_ok(), "{o:?}");
            } else {
                assert!(parsed.is_ok(), "with reason must parse: {o:?}");
            }
        }
        // Missing reason on Failed must fail closed.
        let bad = sample_result(OutcomeKind::Failed, "");
        let err = NodeResultV1::parse_json(&serde_json::to_string(&bad).unwrap()).unwrap_err();
        assert!(err.to_string().contains("reason"), "{err}");
    }

    // Example documenting the SOMA boundary (acceptance: examples document
    // the mapping edge). Completed vs blocked carry different obligations.
    #[test]
    fn example_soma_boundary_outcomes_and_evidence() {
        let mut done = sample_result(OutcomeKind::Completed, "");
        done.evidence_refs.push(EvidenceReferenceV1 {
            id: "ev-1".into(),
            event_digest: "a".repeat(64),
            artifact_digest: "b".repeat(64),
            artifact_kind: "repository-symbol".into(),
            produced_by: "repo-evidence".into(),
            produced_at: None,
        });
        done.result_digest = done.compute_digest().unwrap();
        let parsed = NodeResultV1::parse_json(&serde_json::to_string(&done).unwrap()).unwrap();
        // Canonical SOMA EvidenceReference mirror survives the roundtrip.
        assert_eq!(parsed.evidence_refs[0].artifact_kind, "repository-symbol");
        assert_eq!(parsed.outcome, OutcomeKind::Completed);
    }
}
