//! `WorkEvent` and `WorkEventBatch`: governed-run events with evidence
//! binding, authority projection checks, and causality validation.
//! Ported from the published reference implementation
//! (`prometheosai/soma`, crates/soma-validate/src/event.rs).

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::contracts::{AuthorityProfile, EvidenceReference};
use super::profile::authority_widened;
use super::types::Hex64;
use super::{Diagnostic, SupportedVersion};

/// Who/what produced an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum ActorKind {
    Agent,
    Model,
    Provider,
    Harness,
    Human,
    Tool,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Actor {
    pub kind: ActorKind,
    pub identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<Implementation>,
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
pub struct Compatibility {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "minReaderVersion"
    )]
    pub min_reader_version: Option<String>,
}

/// Checkpoint envelope inside an event payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadCheckpointEnvelope {
    #[serde(rename = "stateDigest")]
    pub state_digest: Hex64,
    pub sequence: u64,
    #[serde(rename = "repoRevision")]
    pub repo_revision: String,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "boundEvidence"
    )]
    pub bound_evidence: Vec<String>,
}

/// Resume-token reference inside an event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResumeTokenRef {
    pub token: String,
    #[serde(rename = "checkpointDigest")]
    pub checkpoint_digest: Hex64,
    #[serde(rename = "issuedAt")]
    pub issued_at: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPayload {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomePayload {
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "acceptedBy")]
    pub accepted_by: Vec<Actor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ApprovalPayload {
    /// approved | rejected | abstained
    pub decision: String,
    pub approver: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reApprovalPath"
    )]
    pub re_approval_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionPayload {
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CapabilityPayload {
    pub capability: String,
    pub granted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HandoffPayload {
    pub target: Actor,
    #[serde(rename = "semanticIdentity")]
    pub semantic_identity: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "preservesCheckpoint"
    )]
    pub preserves_checkpoint: Option<bool>,
}

/// Typed event payloads; the variant matches `eventType`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EventPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<OutcomePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "checkpointEnvelope"
    )]
    pub checkpoint_envelope: Option<PayloadCheckpointEnvelope>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "resumeToken"
    )]
    pub resume_token: Option<ResumeTokenRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<DecisionPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<CapabilityPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<HandoffPayload>,
}

/// One governed-run event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkEvent {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    pub id: String,
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub actor: Actor,
    pub authority: AuthorityProfile,
    #[serde(rename = "effectiveAuthority")]
    pub effective_authority: AuthorityProfile,
    pub sequence: u64,
    pub timestamp: String,
    #[serde(rename = "idempotencyKey")]
    pub idempotency_key: String,
    #[serde(rename = "correlationId")]
    pub correlation_id: String,
    #[serde(rename = "repoRevision")]
    pub repo_revision: String,
    pub compatibility: Compatibility,
    #[serde(rename = "semanticDigest")]
    pub semantic_digest: Hex64,
    pub parents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<EvidenceReference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<Implementation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<EventPayload>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
}

impl WorkEvent {
    /// Canonical digest over the event's semantic payload (the content that
    /// must survive handoffs): eventType/version/repoRevision/implementation/
    /// payload. Mirrors the published bridge's `_event_semantic_digest`.
    pub fn computed_semantic_digest(&self) -> String {
        let mut content = serde_json::Map::new();
        content.insert("eventType".into(), self.event_type.clone().into());
        content.insert("version".into(), self.version.clone().into());
        content.insert("repoRevision".into(), self.repo_revision.clone().into());
        if let Some(imp) = &self.implementation
            && let Ok(v) = serde_json::to_value(imp)
        {
            content.insert("implementation".into(), v);
        }
        if let Some(p) = &self.payload
            && let Ok(v) = serde_json::to_value(p)
        {
            content.insert("payload".into(), v);
        }
        super::canonical::canonical_digest(&serde_json::Value::Object(content))
    }

    /// Audit against a supported bundle version.
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        // SOMA-EVT-0001 / SOMA-CMP-0001: unsupported versions.
        for (label, v) in [
            ("schemaVersion", &self.schema_version),
            ("version", &self.version),
        ] {
            match super::types::SemVer::parse(v) {
                Err(_) => out.push(Diagnostic::new(
                    "SOMA-EVT-0001",
                    format!("{label} {v:?} is not a valid semantic version"),
                )),
                Ok(parsed_v) => {
                    if !parsed_v.is_compatible_with(supported) {
                        out.push(Diagnostic::new(
                            "SOMA-CMP-0001",
                            format!("artifact {label} {v} newer than bundle"),
                        ));
                        out.push(Diagnostic::new(
                            "SOMA-EVT-0001",
                            format!("WorkEvent {label} {v} unsupported by bundle"),
                        ));
                    }
                }
            }
        }

        // SOMA-CMP-0004: asserted vs computed semantic digest.
        let computed = self.computed_semantic_digest();
        if computed != self.semantic_digest.as_str() {
            out.push(Diagnostic::related(
                "SOMA-CMP-0004",
                "semantic digest does not verify",
                self.id.clone(),
            ));
        }

        // SOMA-EVT-0004: result/outcome events need BOUND evidence.
        if self.event_type == "result" || self.event_type == "outcome" {
            let bound_to = self.computed_semantic_digest();
            let bound = self
                .evidence
                .iter()
                .flatten()
                .any(|e| bound_to == e.event_digest.as_str());
            if !bound {
                out.push(Diagnostic::related(
                    "SOMA-EVT-0004",
                    "attributable result/outcome missing bound evidence",
                    self.id.clone(),
                ));
            }
        }

        // SOMA-EVT-0002 (checkpoint shape) / SOMA-RES-0003 (resume token).
        if let Some(payload) = &self.payload {
            if self.event_type == "checkpoint" {
                match &payload.checkpoint_envelope {
                    None => out.push(Diagnostic::related(
                        "SOMA-EVT-0002",
                        "checkpoint without envelope",
                        self.id.clone(),
                    )),
                    Some(env) => {
                        if env.state_digest.as_str().is_empty() {
                            out.push(Diagnostic::new("SOMA-EVT-0002", "checkpoint without state"));
                        }
                    }
                }
            }
            if self.event_type == "resume" && payload.resume_token.is_none() {
                out.push(Diagnostic::new(
                    "SOMA-RES-0003",
                    "resume without resume token",
                ));
            }
        }

        // SOMA-EVT-0003: effective authority must not widen declared.
        if authority_widened(&self.effective_authority, &self.authority) {
            out.push(Diagnostic::related(
                "SOMA-EVT-0003",
                "effective authority exceeds declared authority",
                self.id.clone(),
            ));
        }

        // SOMA-EVT-0005: non-idempotent replay conflict.
        if self.replay && self.conflict {
            out.push(Diagnostic::related(
                "SOMA-EVT-0005",
                "replay conflicts with causal history and is not idempotent",
                self.id.clone(),
            ));
        }
        out
    }
}

/// Causally ordered batch of events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkEventBatch {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub version: String,
    #[serde(rename = "runId")]
    pub run_id: String,
    pub events: Vec<WorkEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<Compatibility>,
}

impl WorkEventBatch {
    pub fn audit(&self, supported: &SupportedVersion) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        for v in [&self.schema_version, &self.version] {
            if let Ok(parsed) = super::types::SemVer::parse(v)
                && !parsed.is_compatible_with(supported)
            {
                out.push(Diagnostic::new(
                    "SOMA-CMP-0001",
                    format!("batch version {v} newer than bundle"),
                ));
            }
        }

        let ids: BTreeSet<&str> = self.events.iter().map(|e| e.id.as_str()).collect();
        for e in &self.events {
            out.extend(e.audit(supported));
            // SOMA-EVT-0002: parents must exist within the batch.
            for parent in &e.parents {
                if !ids.contains(parent.as_str()) {
                    out.push(Diagnostic::related(
                        "SOMA-EVT-0002",
                        format!("references missing causal parent {parent:?}"),
                        e.id.clone(),
                    ));
                }
            }
        }
        if batch_has_cycle(&self.events) {
            out.push(Diagnostic::new("SOMA-EVT-0002", "cyclic causal parents"));
        }
        out.sort_by(|a, b| (&a.code, &a.message).cmp(&(&b.code, &b.message)));
        out.dedup_by(|a, b| a.code == b.code && a.message == b.message);
        out
    }
}

fn batch_has_cycle(events: &[WorkEvent]) -> bool {
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    for e in events {
        indegree.entry(e.id.as_str()).or_insert(0);
        for p in &e.parents {
            edges.entry(p.as_str()).or_default().push(e.id.as_str());
            *indegree.entry(e.id.as_str()).or_insert(0) += 1;
        }
    }
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(k, _)| *k)
        .collect();
    let mut visited = 0;
    while let Some(n) = queue.pop() {
        visited += 1;
        for nxt in edges.get(n).into_iter().flatten() {
            let d = indegree.get_mut(nxt).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push(nxt);
            }
        }
    }
    visited < indegree.len()
}
